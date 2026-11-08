//! The loaded light (ADR-0046, Plan 0059): a bake's cells and the frames that read them, headless on
//! the software adapter, and the ignored `bake_count_readings` on the RTX (lane rule 9c), which sets
//! the reference's sample count.
//!
//! Every world here is a hand-built box of under 2000 faces at test counts: nothing bakes a whole
//! world in a test.

#![allow(
    clippy::disallowed_methods,
    reason = "the count reading times its runs on the wall clock"
)]

mod support;

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::sync::Arc;

use flythrough::cave::{ORE, ROCK};
use support::rigs::tone_bytes;
use support::{SyncNow, reading_device};
use vxe_render::bake::{self, BakeSettings};
use vxe_render::gpu_world::{GpuWorld, MirrorConfig, UNIT_BYTES, Window};
use vxe_render::marcher::LightingResolution;
use vxe_render::{Camera, Exposure, Frame, Light, LightParams, Lighting, readback, wgpu};
use vxe_world::{
    AIR, ChunkCoord, ChunkExtent, EditOp, FIXED_ONE, Fixed3, MaterialTable, Mode, UniformSource,
    World,
};

struct Rig {
    device: wgpu::Device,
    queue: wgpu::Queue,
    gpu: GpuWorld,
    world: World,
    frame: Frame,
    cfg: MirrorConfig,
}

fn fill(world: &World, min: [i64; 3], max: [i64; 3], m: vxe_world::MaterialId) {
    world
        .apply(&EditOp::box_(
            Fixed3(min.map(|v| v * FIXED_ONE)),
            Fixed3(max.map(|v| v * FIXED_ONE)),
            Mode::Fill(m),
        ))
        .expect("the box is resident");
}

fn air_world(size: [i32; 3], table: &MaterialTable) -> World {
    World::generate(
        ChunkExtent::new([0, 0, 0], size),
        &UniformSource(AIR),
        table,
    )
}

fn table() -> MaterialTable {
    flythrough::with_emissive_ore(&flythrough::cave_materials())
}

/// A rock block under an ore plate over chunks `[0, 2)^3`: about 1700 faces, three of the block's
/// sides in the camera's view, with a 64 x 36 frame looking at it.
fn small_box() -> (World, Camera) {
    let world = air_world([2, 2, 2], &table());
    fill(&world, [40, 40, 40], [56, 56, 56], ROCK);
    fill(&world, [42, 42, 72], [54, 54, 74], ORE);
    let camera = Camera {
        position: [30.5, 28.5, 70.5].into(),
        yaw: 45f32.to_radians(),
        pitch: (-35f32).to_radians(),
        vfov: 70f32.to_radians(),
    };
    (world, camera)
}

/// `world` mirrored whole under a frame at `size` (full-resolution lighting, no jitter, lamp and
/// probes off, exposure 8), its light `light`.
fn rig_on(world: World, size: [u32; 2], light: Light, light_radius: Option<i32>) -> Option<Rig> {
    let (device, queue) = support::software_device(64 << 20)?;
    let base = MirrorConfig::new(Window::Fixed(world.extent()));
    let cfg = MirrorConfig {
        budget_bytes: 64 << 20,
        light_radius: light_radius.unwrap_or(base.light_radius),
        ..base
    };
    let mut gpu = GpuWorld::with_config(&device, cfg);
    for _ in 0..2 {
        gpu.sync_now(&world, &device, &queue)
            .expect("the mirror fits");
    }
    let frame = new_frame(&device, size, light);
    Some(Rig {
        device,
        queue,
        gpu,
        world,
        frame,
        cfg,
    })
}

fn new_frame(device: &wgpu::Device, size: [u32; 2], light: Light) -> Frame {
    let mut frame = Frame::new(device, size[0], size[1]);
    frame.set_jitter(device, false);
    frame.set_resolution(device, LightingResolution::Full);
    frame.set_lighting(Lighting {
        lamp: false,
        lamp_bounce: false,
        exposure: Exposure::Manual(8.0),
        light,
        bake_pool_bytes: 16 << 20,
        ..Lighting::default()
    });
    frame.set_dt(1.0 / 60.0);
    frame
}

impl Rig {
    fn bake(&mut self, settings: &BakeSettings) -> bake::BakeReport {
        let extent = self.world.extent();
        self.frame
            .bake_until(
                &self.device,
                &self.queue,
                &mut self.gpu,
                &self.world,
                extent,
                settings,
                &mut |_| {},
            )
            .expect("the box fits the pool")
    }

    /// Every chunk's block, for a comparison of the cells.
    fn blocks(&self) -> BTreeMap<[i32; 3], Vec<u32>> {
        frame_blocks(&self.device, &self.queue, &self.frame, self.world.extent())
    }

    /// Every cell of the store by voxel and face: the `rgb9e5` word and the tail word.
    fn cells(&self) -> HashMap<([i32; 3], u8), [u32; 2]> {
        cells_of(&self.blocks())
    }

    /// Applies `op`, mirrors it, and re-bakes its ring to `settings` as the window's frame does:
    /// the report and the edit's box.
    fn dig(&mut self, op: &EditOp, settings: &BakeSettings) -> (bake::DigReport, [[i32; 3]; 2]) {
        let r = self.world.apply(op).expect("the dig is resident");
        let (min, max) = r.aabb.expect("the dig changed voxels");
        self.gpu
            .sync_now(&self.world, &self.device, &self.queue)
            .expect("the mirror fits");
        self.frame.invalidate(min.0, max.0);
        let report = self
            .frame
            .rebake_digs(
                &self.device,
                &self.queue,
                &mut self.gpu,
                &self.world,
                settings,
                &mut |_| {},
            )
            .expect("a box waits")
            .expect("the ring fits");
        (report, [min.0, max.0])
    }

    /// A fresh mirror and frame of the world as it is now, `piece` baked to `settings`: its cells.
    fn fresh_cells(
        &self,
        piece: ChunkExtent,
        settings: &BakeSettings,
    ) -> HashMap<([i32; 3], u8), [u32; 2]> {
        let (_, frame) = self.fresh_bake(piece, settings);
        cells_of(&frame_blocks(
            &self.device,
            &self.queue,
            &frame,
            self.world.extent(),
        ))
    }

    fn fresh_bake(&self, piece: ChunkExtent, settings: &BakeSettings) -> (GpuWorld, Frame) {
        let mut gpu = GpuWorld::with_config(&self.device, self.cfg);
        for _ in 0..2 {
            gpu.sync_now(&self.world, &self.device, &self.queue)
                .expect("the mirror fits");
        }
        let mut frame = new_frame(&self.device, [16, 16], Light::Baked);
        frame
            .bake_until(
                &self.device,
                &self.queue,
                &mut gpu,
                &self.world,
                piece,
                settings,
                &mut |_| {},
            )
            .expect("the box fits the pool");
        (gpu, frame)
    }

    fn accum(&self) -> Vec<[f32; 4]> {
        readback::read_float(&self.device, &self.queue, self.frame.accumulated())
    }
}

/// `frame`'s block of every chunk of `extent` that has one.
fn frame_blocks(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    frame: &Frame,
    e: ChunkExtent,
) -> BTreeMap<[i32; 3], Vec<u32>> {
    let s = e.size();
    let mut out = BTreeMap::new();
    for z in 0..s[2] {
        for y in 0..s[1] {
            for x in 0..s[0] {
                let c = [e.min.0[0] + x, e.min.0[1] + y, e.min.0[2] + z];
                if let Some(b) = frame.read_store_block(device, queue, ChunkCoord(c)) {
                    out.insert(c, b);
                }
            }
        }
    }
    out
}

/// The cells of `blocks` by voxel and face: the `rgb9e5` word and the tail word.
fn cells_of(blocks: &BTreeMap<[i32; 3], Vec<u32>>) -> HashMap<([i32; 3], u8), [u32; 2]> {
    let mut out = HashMap::new();
    for (&c, block) in blocks {
        for slot in 0..512usize {
            let dir = block[slot];
            let (run, n) = ((dir >> 11) as usize, (dir & 0x7FF) as usize);
            let b = vxe_world::brick_of_slot(slot);
            for i in 0..n {
                let [word, tail] = [block[run + 2 * i], block[run + 2 * i + 1]];
                let key = (tail & 0xFFF) as i32;
                let l = key >> 3;
                let inner = [l % 8, l / 8 % 8, l / 64];
                let v = [0, 1, 2].map(|k| c[k] * 64 + b[k] * 8 + inner[k]);
                out.insert((v, (key & 7) as u8), [word, tail]);
            }
        }
    }
    out
}

impl Rig {
    fn step(&mut self, camera: &Camera) {
        self.frame.step(
            &self.device,
            &self.queue,
            &self.gpu,
            camera,
            &LightParams::default(),
        );
    }

    fn display(&self) -> Vec<u8> {
        readback::read_rgba8(&self.device, &self.queue, self.frame.target())
    }
}

/// A hit's cube face, from its position and record: its voxel and entry face.
fn hit_face(eye: [f32; 3], pos: [f32; 4], info: [u16; 4]) -> Option<([i32; 3], u8)> {
    let flags = info[3];
    if flags & 1 == 0 || flags & 6 != 0 {
        return None;
    }
    let axis = i32::from((flags >> 3) & 3) - 1;
    if axis < 0 {
        return None;
    }
    let axis = axis as usize;
    let positive = eye[axis] > pos[axis];
    let mut p = [pos[0], pos[1], pos[2]];
    p[axis] += if positive { -0.5 } else { 0.5 };
    Some((
        p.map(|v| v.floor() as i32),
        (2 * axis) as u8 + u8::from(positive),
    ))
}

fn test_counts(n_min: u32, n_max: u32) -> BakeSettings {
    BakeSettings {
        round: 64,
        n_min,
        n_max,
        ..BakeSettings::default()
    }
}

/// The cells depend on a face's own samples alone: one batch, batches of 7 faces and the jobs
/// reversed read bit-identical, with the stop in play (64 to 256 samples a face); and baked to a
/// test stop, [`four_ways`] on the box and on a slab whose faces end at different counts.
#[test]
fn a_bake_does_not_depend_on_its_slicing_or_order() {
    let (world, _) = small_box();
    let Some(mut rig) = rig_on(world, [16, 16], Light::Baked, None) else {
        return;
    };
    let one = BakeSettings {
        batch_faces: 65_536,
        ..test_counts(64, 256)
    };
    let report = rig.bake(&one);
    assert!(report.faces > 1000, "{} faces", report.faces);
    assert!(report.faces < 2000, "{} faces", report.faces);
    let first = rig.blocks();
    let lit = first
        .values()
        .flat_map(|b| b.iter().skip(512).step_by(2))
        .filter(|w| **w != 0)
        .count();
    assert!(lit > 100, "{lit} lit cells");
    rig.bake(&BakeSettings {
        batch_faces: 7,
        ..one.clone()
    });
    assert_eq!(rig.blocks(), first, "batches of 7 faces");
    rig.bake(&BakeSettings {
        reversed: true,
        ..one.clone()
    });
    assert_eq!(rig.blocks(), first, "the jobs reversed");

    // To a test stop, four ways: cache_box, where every face ends at `N_MIN`, then a two-chunk
    // slab under a small emitter at the chunk border, where the faces end at different counts.
    four_ways(&mut rig, false);
    let mut t = MaterialTable::new();
    for (id, row) in flythrough::cave_materials()
        .rows()
        .iter()
        .enumerate()
        .skip(1)
    {
        let mut m = row.clone();
        if id == usize::from(ORE) {
            m.emission = [0.05, 0.05, 0.05];
        }
        assert_eq!(usize::from(t.push(m)), id);
    }
    let world = air_world([2, 1, 1], &t);
    fill(&world, [40, 16, 20], [88, 48, 21], ROCK);
    fill(&world, [63, 30, 21], [64, 31, 22], ORE);
    let Some(mut rig) = rig_on(world, [16, 16], Light::Baked, None) else {
        return;
    };
    four_ways(&mut rig, true);
}

/// Bakes `rig` to `the_stop_spends_where_the_noise_is`'s constants (64 to 4096, rounds of 64)
/// four ways, one round a submit, several rounds with every job in one submit, each job alone,
/// each job alone at one round a submit, and asserts bit-identical cells and samples a face. A
/// face can take several rounds in a submit only where the stop is in play.
fn four_ways(rig: &mut Rig, stop_in_play: bool) {
    let stop = BakeSettings {
        keep_faces: true,
        ..test_counts(64, 4096)
    };
    let samples_of = |r: &bake::BakeReport| -> HashMap<([i32; 3], u8), u32> {
        r.outcomes
            .iter()
            .map(|o| ((o.voxel, o.face), o.samples))
            .collect()
    };
    let several = rig.bake(&stop);
    let spent = samples_of(&several);
    let counts: std::collections::BTreeSet<u32> = spent.values().copied().collect();
    println!(
        "{} jobs, {} faces, {} samples, {} wasted, {} submits, most rounds a submit {}, ends at {} counts from {:?} to {:?}",
        several.chunks,
        several.faces,
        several.samples,
        several.wasted,
        several.submits,
        several.max_rounds_in_submit,
        counts.len(),
        counts.first(),
        counts.last()
    );
    assert!(several.chunks > 1, "{} jobs", several.chunks);
    assert_eq!(
        several.max_jobs_in_submit, several.chunks,
        "every job in one submit"
    );
    assert_eq!(spent.len() as u64, several.faces);
    if stop_in_play {
        assert!(counts.len() > 2, "the stop is in play: {counts:?}");
        assert!(
            several.max_rounds_in_submit > 1,
            "several rounds a submit: {}",
            several.max_rounds_in_submit
        );
    }
    let stopped = rig.blocks();
    let one_round = rig.bake(&BakeSettings {
        submit_rounds: 1,
        ..stop.clone()
    });
    assert_eq!(one_round.max_rounds_in_submit, 1);
    assert_eq!(
        one_round.wasted, 0,
        "one round a submit throws nothing away"
    );
    assert_eq!(rig.blocks(), stopped, "one round a submit");
    assert_eq!(samples_of(&one_round), spent, "one round a submit");
    let alone = rig.bake(&BakeSettings {
        jobs_alone: true,
        ..stop.clone()
    });
    assert_eq!(alone.max_jobs_in_submit, 1);
    assert!(!stop_in_play || alone.max_rounds_in_submit > 1);
    assert_eq!(rig.blocks(), stopped, "each job alone");
    assert_eq!(samples_of(&alone), spent, "each job alone");
    let alone_one = rig.bake(&BakeSettings {
        jobs_alone: true,
        submit_rounds: 1,
        ..stop
    });
    assert_eq!(alone_one.max_jobs_in_submit, 1);
    assert_eq!(alone_one.max_rounds_in_submit, 1);
    assert_eq!(rig.blocks(), stopped, "each job alone, one round a submit");
    assert_eq!(
        samples_of(&alone_one),
        spent,
        "each job alone, one round a submit"
    );
    for r in [&one_round, &alone, &alone_one] {
        assert_eq!(r.samples, several.samples);
    }
}

/// The irradiance at `p` (normal +z) from a unit-radiance polygon: Lambert's formula.
fn polygon_irradiance(p: [f64; 3], polygon: &[[f64; 3]]) -> f64 {
    let n = polygon.len();
    let unit = |v: [f64; 3]| {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        v.map(|c| c / l)
    };
    let mut sum = 0.0;
    for i in 0..n {
        let a = unit([0, 1, 2].map(|k| polygon[i][k] - p[k]));
        let b = unit([0, 1, 2].map(|k| polygon[(i + 1) % n][k] - p[k]));
        let cross = [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ];
        let theta = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2])
            .clamp(-1.0, 1.0)
            .acos();
        sum += theta * unit(cross)[2];
    }
    0.5 * sum.abs()
}

/// The irradiance at `p` (normal +z, below the voxel) from the faces of emitter voxel `e` that
/// face `p`: the bottom always, a side where `p` lies beyond its plane.
fn voxel_irradiance(p: [f64; 3], e: [f64; 3]) -> f64 {
    let [x0, y0, z0] = e;
    let [x1, y1, z1] = [x0 + 1.0, y0 + 1.0, z0 + 1.0];
    let mut total =
        polygon_irradiance(p, &[[x0, y0, z0], [x1, y0, z0], [x1, y1, z0], [x0, y1, z0]]);
    if p[0] < x0 {
        total += polygon_irradiance(p, &[[x0, y0, z0], [x0, y1, z0], [x0, y1, z1], [x0, y0, z1]]);
    }
    if p[0] > x1 {
        total += polygon_irradiance(p, &[[x1, y0, z0], [x1, y1, z0], [x1, y1, z1], [x1, y0, z1]]);
    }
    if p[1] < y0 {
        total += polygon_irradiance(p, &[[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]]);
    }
    if p[1] > y1 {
        total += polygon_irradiance(p, &[[x0, y1, z0], [x1, y1, z0], [x1, y1, z1], [x0, y1, z1]]);
    }
    total
}

/// An unoccluded wall under one emitter voxel, nearer than `L / 4`, reads the closed form times the
/// window within 3 % on every face (a fixed 4096 samples, the clamp off); a wall at twice the light
/// radius from the emitter reads exactly 0.
#[test]
fn a_wall_reads_its_analytic_light() {
    let world = air_world([4, 1, 1], &table());
    // The light radius is 1 chunk: L is 64 voxels. The wall's top at z 20, the emitter's bottom at
    // z 32: at most about 13.6 voxels from a face's centre, under L / 4.
    fill(&world, [10, 10, 19], [19, 19, 20], ROCK);
    fill(&world, [14, 14, 32], [15, 15, 33], ORE);
    // The far wall: over 128 voxels from the emitter.
    fill(&world, [160, 10, 19], [169, 19, 20], ROCK);
    let Some(mut rig) = rig_on(world, [16, 16], Light::Baked, Some(1)) else {
        return;
    };
    let report = rig.bake(&BakeSettings {
        round: 1024,
        n_min: 4096,
        n_max: 4096,
        clamp: false,
        ..BakeSettings::default()
    });
    assert_eq!(report.faces_at_max, report.faces);
    let cells = rig.cells();
    let le = flythrough::ORE_EMISSION;
    let (mut worst, mut read) = (0f64, 0);
    for x in 10..19 {
        for y in 10..19 {
            let word = cells[&([x, y, 19], 5)][0];
            let got = bake::rgb9e5_unpack(word);
            // The closed form over the face, on a 4 x 4 grid.
            let mut e = 0.0;
            for i in 0..4 {
                for j in 0..4 {
                    let p = [
                        f64::from(x) + (f64::from(i) + 0.5) / 4.0,
                        f64::from(y) + (f64::from(j) + 0.5) / 4.0,
                        20.0,
                    ];
                    let d = (0..3)
                        .map(|k| (p[k] - [14.5, 14.5, 32.5][k]).powi(2))
                        .sum::<f64>()
                        .sqrt();
                    let q = (d / 64.0).powi(4);
                    e += voxel_irradiance(p, [14.0, 14.0, 32.0]) * (1.0 - q).powi(2) / 16.0;
                }
            }
            for k in 0..3 {
                let want = e * f64::from(le[k]);
                let err = (f64::from(got[k]) - want).abs() / want;
                worst = worst.max(err);
                read += 1;
            }
        }
    }
    println!("worst of {read} channels: {:.4} of the closed form", worst);
    assert!(worst <= 0.03, "worst {worst:.4} of the closed form");
    for x in 160..169 {
        for y in 10..19 {
            assert_eq!(cells[&([x, y, 19], 5)][0], 0, "far face {x} {y}");
        }
    }
}

/// `G_local` over the coplanar neighbour pairs `(f, n)` of `keys` in `a` and `b` (two seed sets'
/// display values): `|(D_f - D_n)_A - (D_f - D_n)_B| / sqrt 2`, each pair once (`n` one voxel
/// along a tangent's positive direction), the pair's two faces in one chunk (`seam` false) or in
/// two (`seam` true).
fn g_local(
    a: &HashMap<([i32; 3], u8), f32>,
    b: &HashMap<([i32; 3], u8), f32>,
    seam: bool,
) -> Vec<f32> {
    let mut out = Vec::new();
    for (&(v, side), &da) in a {
        let Some(&db) = b.get(&(v, side)) else {
            continue;
        };
        let axis = usize::from(side / 2);
        for t in [(axis + 1) % 3, (axis + 2) % 3] {
            let mut n = v;
            n[t] += 1;
            let (Some(&na), Some(&nb)) = (a.get(&(n, side)), b.get(&(n, side))) else {
                continue;
            };
            let crosses = v[t].div_euclid(64) != n[t].div_euclid(64);
            if crosses == seam {
                out.push(((da - na) - (db - nb)).abs() / 2f32.sqrt());
            }
        }
    }
    out
}

/// A cut's settings: `n` samples a face in one round, the seed form and the filter.
fn cut_counts(n: u32, form: bake::SeedForm, tent: bool, seed: u32) -> BakeSettings {
    BakeSettings {
        round: n,
        n_min: n,
        n_max: n,
        form,
        tent,
        seed,
        ..BakeSettings::default()
    }
}

/// The shared seeds do what their name says: on a wall lit through a slit at 16 samples a face,
/// two coplanar faces of one chunk read closer to each other under chunk-side seeds than under
/// independent ones, by `G_local` over two seed sets each; the independent arm is the failure
/// case.
#[test]
fn the_seed_forms_differ_as_named() {
    let world = air_world([1, 1, 1], &table());
    // The wall's top at z 20; a plate at z 30 with a slit at x 30..34; the emitter at z 45.
    fill(&world, [8, 8, 19], [56, 56, 20], ROCK);
    fill(&world, [0, 0, 30], [30, 64, 31], ROCK);
    fill(&world, [34, 0, 30], [64, 64, 31], ROCK);
    fill(&world, [16, 16, 45], [48, 48, 46], ORE);
    let Some(mut rig) = rig_on(world, [16, 16], Light::Baked, Some(1)) else {
        return;
    };
    let wall: bake::FaceFilter = Arc::new(|v, f| f == 5 && v[2] == 19);
    let l = Lighting::default();
    let mut runs = Vec::new();
    for form in [bake::SeedForm::Independent, bake::SeedForm::ChunkSide] {
        let mut seeds = Vec::new();
        for seed in [0u32, 0x5EED_0001] {
            rig.bake(&BakeSettings {
                filter: Some(wall.clone()),
                ..cut_counts(16, form, false, seed)
            });
            let d: HashMap<([i32; 3], u8), f32> = rig
                .cells()
                .into_iter()
                .filter(|((v, f), _)| wall(*v, *f))
                .map(|(k, c)| (k, d_of(bake::rgb9e5_unpack(c[0]), &l)))
                .collect();
            seeds.push(d);
        }
        runs.push(seeds);
    }
    // The pairs any run reads lit.
    let lit = |k: &([i32; 3], u8)| runs.iter().flatten().any(|d| d[k] > 0.0);
    let keep = |d: &HashMap<([i32; 3], u8), f32>| -> HashMap<([i32; 3], u8), f32> {
        d.iter()
            .filter(|(k, _)| lit(k))
            .map(|(k, v)| (*k, *v))
            .collect()
    };
    let mut g = Vec::new();
    for (form, seeds) in ["independent", "chunk-side"].iter().zip(&runs) {
        let mut v = g_local(&keep(&seeds[0]), &keep(&seeds[1]), false);
        let mean = v.iter().sum::<f32>() / v.len() as f32;
        let (p50, p90) = (q(&mut v, 0.5), q(&mut v, 0.9));
        println!(
            "{form}: G_local p50 {p50:.2} p90 {p90:.2} mean {mean:.2} over {} pairs",
            v.len()
        );
        assert!(v.len() > 200, "{form}: {} lit pairs", v.len());
        g.push((p50, p90, mean));
    }
    let (independent, chunk_side) = (g[0], g[1]);
    assert!(
        chunk_side.0 < independent.0
            && chunk_side.1 < independent.1
            && chunk_side.2 < independent.2,
        "chunk-side {chunk_side:?} against independent {independent:?}"
    );
}

/// The filter (`bake::tent`, the table's blend kernel) keeps light off the far side of a crease
/// and softens a shadow edge by one face (Plan 0057's arithmetic): an unlit wall's row along an
/// L crease reads exactly 0 beside a lit floor; on a floor half under a slab, the face two from the
/// edge on the dark side reads exactly 0 and the face beside the edge reads between 0.15 and 0.35
/// of the lit face beside it (the tent's 4 of 16).
#[test]
fn the_filter_stops_at_a_crease_and_softens_a_shadow_by_one_face() {
    let world = air_world([3, 1, 1], &table());
    // Chunk 0, the shadow: a floor's top at z 10, a slab at z 11 over x 32..64, an emitter row
    // above the slab's edge at x 32, so every point of a face from x 32 on is in its shadow.
    fill(&world, [0, 0, 0], [64, 64, 10], ROCK);
    fill(&world, [32, 0, 11], [64, 64, 12], ROCK);
    fill(&world, [32, 16, 40], [33, 48, 41], ORE);
    // Chunk 2, the crease: a floor's top at z 10, a wall one voxel high at x 168, its -x face
    // turned away from an emitter row at x 178 and z 40 that lights the floor before it.
    fill(&world, [128, 0, 0], [192, 64, 10], ROCK);
    fill(&world, [168, 0, 10], [169, 64, 11], ROCK);
    fill(&world, [178, 20, 40], [180, 44, 41], ORE);
    let Some(mut rig) = rig_on(world, [16, 16], Light::Baked, Some(1)) else {
        return;
    };
    // The faces read and their kernels' neighbours: the floor's rows y 28..37 by the shadow's
    // edge and the crease, and the wall's -x faces.
    let near: bake::FaceFilter = Arc::new(|v, f| {
        let rows = (28..37).contains(&v[1]);
        let floor = f == 5 && v[2] == 9 && ((26..40).contains(&v[0]) || (160..169).contains(&v[0]));
        let wall = f == 0 && v[0] == 168 && v[2] == 10;
        rows && (floor || wall)
    });
    let at = |rig: &Rig| -> HashMap<([i32; 3], u8), f32> {
        rig.cells()
            .into_iter()
            .map(|(k, c)| (k, bake::rgb9e5_unpack(c[0])[0]))
            .collect()
    };
    rig.bake(&BakeSettings {
        filter: Some(near.clone()),
        ..cut_counts(1024, bake::SeedForm::Independent, false, 0)
    });
    let raw = at(&rig);
    rig.bake(&BakeSettings {
        filter: Some(near.clone()),
        ..cut_counts(1024, bake::SeedForm::Independent, true, 0)
    });
    let tent = at(&rig);
    for y in 30..35 {
        let row = |m: &HashMap<([i32; 3], u8), f32>| -> Vec<String> {
            (29..36)
                .map(|x| format!("{:.4}", m[&([x, y, 9], 5)]))
                .collect()
        };
        println!(
            "shadow row y {y}, x 29..35: raw {:?}, filtered {:?}",
            row(&raw),
            row(&tent)
        );
        // The bake's own edge: x 31 lit, x 32 on in shadow.
        assert!(raw[&([31, y, 9], 5)] > 0.0, "the lit face beside the edge");
        for x in 32..36 {
            assert_eq!(raw[&([x, y, 9], 5)], 0.0, "unfiltered dark face {x} {y}");
        }
        assert_eq!(
            tent[&([33, y, 9], 5)],
            0.0,
            "two from the edge, filtered, {y}"
        );
        let beside = tent[&([32, y, 9], 5)] / raw[&([31, y, 9], 5)];
        println!("  beside the edge, filtered, over the lit face: {beside:.3}");
        assert!(
            (0.15..=0.35).contains(&beside),
            "beside the edge {beside} at {y}"
        );
        // The crease: the floor before the wall is lit; the wall's -x face reads exactly 0.
        assert!(
            tent[&([167, y, 9], 5)] > 0.0,
            "the floor beside the crease at {y}"
        );
        assert_eq!(raw[&([168, y, 10], 0)], 0.0, "the wall, unfiltered, {y}");
        assert_eq!(
            tent[&([168, y, 10], 0)],
            0.0,
            "the wall's row, filtered, {y}"
        );
    }
}

/// After the bake the first frame is the final one: frame 1 and frame 64 are bit-identical, its
/// emissive part is the cell's value times the exposure within one half step, the light pass drew
/// nothing and the table took nothing, and no face misses the store.
#[test]
fn a_baked_view_is_final_on_its_first_frame() {
    let (world, camera) = small_box();
    let Some(mut rig) = rig_on(world, [64, 36], Light::Baked, None) else {
        return;
    };
    rig.bake(&test_counts(128, 128));
    let cells = rig.cells();
    rig.step(&camera);
    let first = rig.display();
    let accum = readback::read_float(&rig.device, &rig.queue, rig.frame.accumulated());
    let pos = readback::read_float(&rig.device, &rig.queue, rig.frame.hit_positions());
    let info = readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    let light = readback::read_float(&rig.device, &rig.queue, rig.frame.irradiance());
    let stats = rig.frame.read_store_stats(&rig.device, &rig.queue);
    let table = rig.frame.read_table_stats(&rig.device, &rig.queue);
    assert_eq!(stats.missing, 0, "store_missing");
    assert_eq!(
        (table.occupied, table.misses),
        (0, 0),
        "the table took nothing"
    );
    let eye = camera.position.to_array();
    let (mut checked, mut lit) = (0, 0);
    for i in 0..pos.len() {
        let Some(key) = hit_face(eye, pos[i], info[i]) else {
            continue;
        };
        assert_eq!(
            [light[i][0], light[i][1], light[i][2]],
            [0.0; 3],
            "the light pass's emissive sample at pixel {i}"
        );
        let cell = cells[&key][0];
        let want = bake::rgb9e5_unpack(cell).map(|v| v * 8.0);
        for k in 0..3 {
            let step = (want[k].abs() / 1024.0).max(1.0 / 16384.0);
            assert!(
                (accum[i][k] - want[k]).abs() <= step,
                "pixel {i} channel {k}: {} against {}",
                accum[i][k],
                want[k]
            );
        }
        checked += 1;
        lit += usize::from(cell != 0);
    }
    assert!(checked > 200, "{checked} pixels with a face");
    assert!(lit > 100, "{lit} lit pixels");
    for _ in 1..64 {
        rig.step(&camera);
    }
    assert_eq!(rig.display(), first, "frame 64 against frame 1");
}

/// A world for the stop: a floor with one ore voxel on it. The face beside the emitter, a floor face
/// far from it and the floor's underside, which no emitter reaches. A dim ore keeps the faces
/// beside it off the tone map's shoulder, where a level says nothing of the spread.
#[test]
fn the_stop_spends_where_the_noise_is() {
    let mut t = MaterialTable::new();
    let base = flythrough::cave_materials();
    for (id, row) in base.rows().iter().enumerate().skip(1) {
        let mut m = row.clone();
        if id == usize::from(ORE) {
            m.emission = [0.05, 0.05, 0.05];
        }
        assert_eq!(usize::from(t.push(m)), id);
    }
    let world = air_world([1, 1, 1], &t);
    fill(&world, [8, 8, 20], [48, 48, 21], ROCK);
    fill(&world, [10, 10, 21], [11, 11, 22], ORE);
    let Some(mut rig) = rig_on(world, [16, 16], Light::Baked, None) else {
        return;
    };
    let report = rig.bake(&BakeSettings {
        round: 64,
        n_min: 64,
        n_max: 4096,
        keep_faces: true,
        ..BakeSettings::default()
    });
    let at = |v: [i32; 3], f: u8| {
        report
            .outcomes
            .iter()
            .find(|o| o.voxel == v && o.face == f)
            .unwrap_or_else(|| panic!("face {v:?} {f}"))
    };
    let beside = at([11, 10, 20], 5);
    let far = at([47, 47, 20], 5);
    let unlit = at([30, 30, 20], 4);
    println!(
        "samples: beside {} ({:?} se {:.3}), far {} ({:?} se {:.3}), unlit {}",
        beside.samples, beside.mean[0], beside.se, far.samples, far.mean[0], far.se, unlit.samples
    );
    assert_eq!(unlit.samples, 64, "the unlit face");
    assert_eq!(unlit.mean[0], [0.0; 3]);
    assert!(beside.mean[0][0] > 0.0 && far.mean[0][0] > 0.0);
    assert!(
        beside.samples > far.samples,
        "beside {} far {}",
        beside.samples,
        far.samples
    );
    for o in &report.outcomes {
        assert!(
            o.se <= bake::BAKE_SE || o.samples >= 4096,
            "{:?} {}: {} samples, se {}",
            o.voxel,
            o.face,
            o.samples,
            o.se
        );
    }
}

/// The owner's "moving fast" as a property: a frame reached by a 90 degree yaw in 2 frames reads
/// what a fresh frame at that pose reads, on every pixel with a face. The table in the same arm
/// fails it.
#[test]
fn a_moving_camera_reads_what_a_still_one_reads() {
    let (world, turned) = small_box();
    let camera = Camera {
        yaw: turned.yaw - 90f32.to_radians(),
        ..turned
    };
    let half = Camera {
        yaw: turned.yaw - 45f32.to_radians(),
        ..turned
    };
    let Some(mut rig) = rig_on(world, [64, 36], Light::Baked, None) else {
        return;
    };
    rig.bake(&test_counts(128, 128));
    let faced = |rig: &Rig, cam: &Camera, img: &[u8]| -> Vec<(usize, [u8; 4])> {
        let pos = readback::read_float(&rig.device, &rig.queue, rig.frame.hit_positions());
        let info = readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
        (0..pos.len())
            .filter(|&i| hit_face(cam.position.to_array(), pos[i], info[i]).is_some())
            .map(|i| {
                (
                    i,
                    [img[4 * i], img[4 * i + 1], img[4 * i + 2], img[4 * i + 3]],
                )
            })
            .collect()
    };
    rig.step(&camera);
    rig.step(&half);
    rig.step(&turned);
    let moved = rig.display();
    let moved = faced(&rig, &turned, &moved);
    assert!(moved.len() > 100, "{} pixels with a face", moved.len());

    let mut fresh = new_frame(&rig.device, [64, 36], Light::Baked);
    let extent = rig.world.extent();
    fresh
        .bake_until(
            &rig.device,
            &rig.queue,
            &mut rig.gpu,
            &rig.world,
            extent,
            &test_counts(128, 128),
            &mut |_| {},
        )
        .expect("the box fits the pool");
    std::mem::swap(&mut rig.frame, &mut fresh);
    rig.step(&turned);
    let still = rig.display();
    let still = faced(&rig, &turned, &still);
    assert_eq!(moved, still, "the moved frame against the fresh one");

    // The table in the same arm, on one frame with a small table and no preload: the moved
    // frame's young faces differ from a fresh frame's, each from an empty table.
    let table_light = |cells: u32| Lighting {
        lamp: false,
        lamp_bounce: false,
        exposure: Exposure::Manual(8.0),
        table_cells: cells,
        preload_radius: 0,
        ..Lighting::default()
    };
    rig.frame.set_lighting(table_light(1 << 14));
    rig.frame.reset_history();
    rig.step(&camera);
    rig.step(&half);
    rig.step(&turned);
    let a = rig.display();
    let a = faced(&rig, &turned, &a);
    rig.frame.set_lighting(table_light(1 << 13));
    rig.frame.reset_history();
    rig.step(&turned);
    let b = rig.display();
    let b = faced(&rig, &turned, &b);
    assert_ne!(a, b, "the table's moved frame against its fresh one");
}

// ---- The dig, the scale, the lamp, the file and the piece (Phase 2) ----

/// A dig's settings beside a bake's: the test counts, the dig's `n_max` the same.
fn dig_counts() -> BakeSettings {
    test_counts(64, 256)
}

fn sphere(c: [i64; 3], r: i64, mode: Mode) -> EditOp {
    EditOp::sphere(Fixed3(c.map(|v| v * FIXED_ONE)), r * FIXED_ONE, mode)
}

fn box_op(min: [i64; 3], max: [i64; 3], mode: Mode) -> EditOp {
    EditOp::box_(
        Fixed3(min.map(|v| v * FIXED_ONE)),
        Fixed3(max.map(|v| v * FIXED_ONE)),
        mode,
    )
}

/// The cells of `cells` whose voxel lies in a brick of `ring`.
fn in_ring(
    cells: &HashMap<([i32; 3], u8), [u32; 2]>,
    ring: &BTreeMap<[i32; 3], Vec<u16>>,
) -> BTreeMap<([i32; 3], u8), [u32; 2]> {
    cells
        .iter()
        .filter(|((v, _), _)| {
            let c = v.map(|x| x.div_euclid(64));
            let b = v.map(|x| x.rem_euclid(64) / 8);
            ring.get(&c)
                .is_some_and(|s| s.contains(&(vxe_world::brick_slot(b) as u16)))
        })
        .map(|(k, v)| (*k, *v))
        .collect()
}

/// A lit wall, a radius 3 carve: on the first frame the hole is on screen, every cell of the ring
/// is bit-identical to a fresh bake of the edited world (the same `n_max`), every cell beyond it
/// is as before, and that frame and frame +64 are bit-identical.
#[test]
fn a_dug_hole_is_final_on_its_first_frame() {
    let (world, camera) = small_box();
    let Some(mut rig) = rig_on(world, [64, 36], Light::Baked, None) else {
        return;
    };
    rig.bake(&dig_counts());
    let before = rig.cells();
    let (dig, aabb) = rig.dig(&sphere([48, 48, 56], 3, Mode::Carve), &dig_counts());
    assert!(dig.bake.faces > 0, "the dig baked no face");
    let piece = rig.world.extent();
    let ring = bake::dig_ring(&[aabb], piece);
    let after = rig.cells();
    let fresh = rig.fresh_cells(piece, &dig_counts());
    let (a, f) = (in_ring(&after, &ring), in_ring(&fresh, &ring));
    assert!(a.len() > 100, "{} cells in the ring", a.len());
    assert_eq!(a, f, "the ring against a fresh bake");
    for (k, v) in &after {
        if !a.contains_key(k) {
            assert_eq!(before.get(k), Some(v), "{k:?} beyond the ring moved");
        }
    }
    // The hole's new faces: voxels within the carve's reach that had no cell before.
    let new: Vec<_> = after.keys().filter(|k| !before.contains_key(k)).collect();
    assert!(!new.is_empty(), "the carve exposed no face");
    rig.step(&camera);
    let first = rig.display();
    let pos = readback::read_float(&rig.device, &rig.queue, rig.frame.hit_positions());
    let info = readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    let eye = camera.position.to_array();
    let on_screen = (0..pos.len())
        .filter_map(|i| hit_face(eye, pos[i], info[i]))
        .filter(|k| !before.contains_key(k))
        .count();
    println!(
        "dig: {} chunks, {} faces, {} samples; {} new faces, {} hole pixels",
        dig.bake.chunks,
        dig.bake.faces,
        dig.bake.samples,
        new.len(),
        on_screen
    );
    assert!(on_screen > 0, "the hole is on screen");
    let stats = rig.frame.read_store_stats(&rig.device, &rig.queue);
    assert_eq!(stats.missing, 0, "store_missing");
    for _ in 0..64 {
        rig.step(&camera);
    }
    assert_eq!(rig.display(), first, "frame +64 against the dig's frame");
}

/// Carving the box's one emitter brings every cell of the ring to exactly 0 on that frame,
/// bit-identical to a fresh bake.
#[test]
fn a_dug_emitter_leaves_the_near_ring_at_once() {
    let world = air_world([1, 1, 1], &table());
    fill(&world, [8, 8, 20], [40, 40, 21], ROCK);
    fill(&world, [22, 22, 30], [24, 24, 32], ORE);
    let Some(mut rig) = rig_on(world, [16, 16], Light::Baked, None) else {
        return;
    };
    rig.bake(&dig_counts());
    let lit = rig.cells().values().filter(|c| c[0] != 0).count();
    assert!(lit > 100, "{lit} lit cells before");
    let (_, aabb) = rig.dig(
        &box_op([22, 22, 30], [24, 24, 32], Mode::Carve),
        &dig_counts(),
    );
    let piece = rig.world.extent();
    let ring = bake::dig_ring(&[aabb], piece);
    let after = in_ring(&rig.cells(), &ring);
    assert!(after.len() > 1000, "{} cells in the ring", after.len());
    let lit: Vec<_> = after.iter().filter(|(_, c)| c[0] != 0).collect();
    assert!(
        lit.is_empty(),
        "{} cells still lit, first {:?}",
        lit.len(),
        lit.first()
    );
    let fresh = in_ring(&rig.fresh_cells(piece, &dig_counts()), &ring);
    assert_eq!(after, fresh, "the ring against a fresh bake");
}

/// A tunnel: an emitter, a plug, and the tunnel's far end beyond [`bake::DIG_REACH`] of the plug.
/// After a window is carved in the plug, the far end's cells are bit-identical to before (where
/// a fresh bake lights them), and the HUD names the limit.
#[test]
fn the_far_light_stays_as_the_reference_states() {
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [2, 1, 1]),
        &UniformSource(ROCK),
        &table(),
    );
    world
        .apply(&box_op([8, 28, 28], [110, 34, 34], Mode::Carve))
        .expect("the tunnel");
    fill(&world, [10, 30, 30], [12, 32, 32], ORE);
    fill(&world, [30, 28, 28], [32, 34, 34], ROCK);
    let Some(mut rig) = rig_on(world, [16, 16], Light::Baked, Some(2)) else {
        return;
    };
    rig.bake(&dig_counts());
    let far = |cells: &HashMap<([i32; 3], u8), [u32; 2]>| -> BTreeMap<([i32; 3], u8), [u32; 2]> {
        cells
            .iter()
            .filter(|((v, _), _)| v[0] >= 72)
            .map(|(k, v)| (*k, *v))
            .collect()
    };
    let before = far(&rig.cells());
    assert!(before.len() > 500, "{} far cells", before.len());
    let (dig, _) = rig.dig(
        &box_op([30, 30, 30], [32, 32, 32], Mode::Carve),
        &dig_counts(),
    );
    let after = far(&rig.cells());
    assert_eq!(after, before, "the far end beyond the ring");
    let fresh = far(&rig.fresh_cells(rig.world.extent(), &dig_counts()));
    let lit = fresh.values().filter(|c| c[0] != 0).count();
    println!("far cells {}, lit in a fresh bake {lit}", before.len());
    assert!(
        lit > 0,
        "a fresh bake lights the far end through the window"
    );
    let note = flythrough::hud::store_note(
        None,
        Some(flythrough::hud::DigHitch {
            faces: dig.bake.faces,
            samples: dig.bake.samples,
            ms: 1.0,
            outside: false,
        }),
    );
    assert!(note.contains("far light stale after a dig"), "{note}");
}

/// `Frame::set_emission_scales` at 0.5 halves a lit wall's displayed irradiance on that frame,
/// within one `rgb9e5` step, the cells unchanged; a material at 0 when baked reads its light when
/// raised.
#[test]
fn a_scale_change_moves_no_bake() {
    let (world, camera) = small_box();
    let Some(mut rig) = rig_on(world, [64, 36], Light::Baked, None) else {
        return;
    };
    rig.frame.set_emission_scale(&rig.queue, ORE, 0.0);
    rig.bake(&test_counts(128, 128));
    let cells = rig.cells();
    rig.step(&camera);
    let dark = rig.accum();
    assert!(
        dark.iter()
            .all(|p| p[0] == 0.0 && p[1] == 0.0 && p[2] == 0.0),
        "the ore at 0 lights nothing"
    );
    rig.frame.set_emission_scale(&rig.queue, ORE, 1.0);
    rig.step(&camera);
    let full = rig.accum();
    rig.frame.set_emission_scale(&rig.queue, ORE, 0.5);
    rig.step(&camera);
    let half = rig.accum();
    assert_eq!(rig.cells(), cells, "the cells after the scale changes");
    let mut lit = 0;
    for i in 0..full.len() {
        for k in 0..3 {
            let step = (full[i][k].abs() / 512.0).max(1.0 / 16384.0);
            assert!(
                (half[i][k] - 0.5 * full[i][k]).abs() <= step,
                "pixel {i} channel {k}: {} against half of {}",
                half[i][k],
                full[i][k]
            );
        }
        lit += usize::from(full[i][0] > 0.0);
    }
    assert!(lit > 100, "{lit} lit pixels at scale 1");
}

/// With the lamp on a frame's emissive part equals the lamp-off frame's and the cells are
/// unchanged; off again, the frame is bit-identical to the first lamp-off frame once the lamp's
/// history has emptied, [`LAMP_EMPTY_FRAMES`] frames after the lamp goes off.
#[test]
fn the_lamp_adds_over_the_bake() {
    let (world, camera) = small_box();
    let Some(mut rig) = rig_on(world, [64, 36], Light::Baked, None) else {
        return;
    };
    rig.bake(&test_counts(128, 128));
    let cells = rig.cells();
    rig.step(&camera);
    let off = rig.display();
    let off_accum = rig.accum();
    let lighting = |lamp: bool| Lighting {
        lamp,
        lamp_bounce: false,
        exposure: Exposure::Manual(8.0),
        light: Light::Baked,
        bake_pool_bytes: 16 << 20,
        ..Lighting::default()
    };
    rig.frame.set_lighting(lighting(true));
    for _ in 0..4 {
        rig.step(&camera);
    }
    assert_ne!(rig.display(), off, "the lamp shows");
    let rgb = |a: &[[f32; 4]]| -> Vec<[f32; 3]> { a.iter().map(|p| [p[0], p[1], p[2]]).collect() };
    let differ = rgb(&rig.accum())
        .iter()
        .zip(rgb(&off_accum))
        .filter(|(a, b)| **a != *b)
        .count();
    assert_eq!(differ, 0, "pixels whose emissive part moved under the lamp");
    assert_eq!(rig.cells(), cells, "the cells under the lamp");
    rig.frame.set_lighting(lighting(false));
    let mut at = None;
    for n in 1..=256 {
        rig.step(&camera);
        if rig.display() == off {
            at = Some(n);
            break;
        }
    }
    println!("lamp off: the first lamp-off frame again at +{at:?}");
    assert_eq!(
        at,
        Some(LAMP_EMPTY_FRAMES),
        "frames after the lamp goes off"
    );
}

/// Frames after the lamp goes off at which the frame is the lamp-off frame again.
const LAMP_EMPTY_FRAMES: u32 = 1;

/// A pane of opacity 0.6 and a dark albedo, one voxel thick, 2.5 voxels ahead of
/// [`small_box`]'s eye, over the block lit by its ore plate: with the lamp off (the headlamp's
/// parameters set, as the window's frames pass them), every pixel and channel reads no brighter
/// than the larger of the block alone and the pane drawn opaque (its own lit face). One rig: the
/// block alone, then the pane dug in as translucent glass, then the same box's glass replaced by its
/// opaque twin, each dig re-baking its ring as the window's frame does.
#[test]
fn a_glass_pane_does_not_brighten_what_it_covers() {
    let mut t = table();
    let rock = t.get(ROCK).expect("rock").clone();
    for translucent in [true, false] {
        let flags = if translucent {
            rock.flags | vxe_world::MaterialFlags::TRANSLUCENT
        } else {
            rock.flags
        };
        t.push(vxe_world::Material {
            name: format!("pane {}", if translucent { "glass" } else { "opaque" }),
            albedo: [0.10, 0.10, 0.14],
            emission: [0.0; 3],
            opacity: 0.6,
            flags,
            ..rock.clone()
        });
    }
    let (glass_id, opaque_id) = (
        (t.len() - 2) as vxe_world::MaterialId,
        (t.len() - 1) as vxe_world::MaterialId,
    );
    let (_, camera) = small_box();
    let world = air_world([2, 2, 2], &t);
    fill(&world, [40, 40, 40], [56, 56, 56], ROCK);
    fill(&world, [42, 42, 72], [54, 54, 74], ORE);
    let Some(mut rig) = rig_on(world, [64, 36], Light::Baked, None) else {
        return;
    };
    let settings = test_counts(128, 128);
    rig.bake(&settings);
    let frame_of = |rig: &mut Rig, name: &str| -> (Vec<u8>, usize) {
        for _ in 0..16 {
            rig.step(&camera);
        }
        let info = readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
        let covered = info.iter().filter(|r| r[3] & 4 != 0).count();
        println!("{name}: {covered} pixels behind a translucent voxel");
        (rig.display(), covered)
    };
    let pane = |mode: Mode| {
        EditOp::box_(
            Fixed3([33, 16, 40].map(|v: i64| v * FIXED_ONE)),
            Fixed3([34, 72, 80].map(|v: i64| v * FIXED_ONE)),
            mode,
        )
    };
    let (wall, _) = frame_of(&mut rig, "the block alone");
    rig.dig(&pane(Mode::Fill(glass_id)), &settings);
    let (glass, covered) = frame_of(&mut rig, "the glass pane");
    rig.dig(
        &pane(Mode::Replace {
            from: glass_id,
            to: opaque_id,
        }),
        &settings,
    );
    let (opaque, _) = frame_of(&mut rig, "the opaque pane");
    assert!(covered > 1000, "{covered} pixels behind the pane");
    let lit = wall.chunks(4).filter(|p| p[..3] != [0, 0, 0]).count();
    assert!(lit > 100, "{lit} lit pixels of the block alone");
    let mut over = 0;
    let mut first = None;
    for (i, ((g, w), o)) in glass
        .chunks(4)
        .zip(wall.chunks(4))
        .zip(opaque.chunks(4))
        .enumerate()
    {
        for k in 0..3 {
            if g[k] > w[k].max(o[k]) {
                over += 1;
                first.get_or_insert((i, k, g[k], w[k], o[k]));
            }
        }
    }
    assert_eq!(
        over, 0,
        "channels brighter behind the pane than the block alone and the pane's own face; the first (pixel, channel, pane, block, own face): {first:?}"
    );
}

/// The composite's radiance of each pixel's hit before the display and before any translucent
/// solid in front of it (pre-exposed): `albedo * E / PI` plus the emission at exposure `ex`, `E`
/// the irradiance the composite reads (the filter's rgb, or with the filter off the light pass's
/// rgb plus its lamp's white). 0 on a miss.
fn hit_radiance(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    frame: &Frame,
    mats: &MaterialTable,
    ex: f32,
) -> Vec<[f32; 3]> {
    let filter = frame.lighting().filter;
    let info = readback::read_rgba16u(device, queue, frame.hit_records());
    let lit_tex = if filter {
        frame.filtered()
    } else {
        frame.irradiance()
    };
    let lit = readback::read_float(device, queue, lit_tex);
    info.iter()
        .zip(&lit)
        .map(|(r, l)| {
            if r[3] & 1 == 0 {
                return [0.0; 3];
            }
            let m = mats.get(r[2]).expect("a material");
            let s = frame.emission_scale(r[2]);
            let w = if filter { 0.0 } else { l[3] };
            [0, 1, 2]
                .map(|k| m.albedo[k] * (l[k] + w) / std::f32::consts::PI + m.emission[k] * s * ex)
        })
        .collect()
}

/// The length of `o + d s`, `0 <= s < s_end`, inside the boxes `[min, max)` (disjoint).
fn path_in_boxes(o: [f64; 3], d: [f64; 3], s_end: f64, boxes: &[([i32; 3], [i32; 3])]) -> f64 {
    let mut sum = 0.0;
    for (min, max) in boxes {
        let (mut a, mut b) = (0.0f64, s_end);
        for k in 0..3 {
            let (lo, hi) = (f64::from(min[k]), f64::from(max[k]));
            if d[k] == 0.0 {
                if o[k] < lo || o[k] >= hi {
                    b = a;
                }
                continue;
            }
            let (p, q) = ((lo - o[k]) / d[k], (hi - o[k]) / d[k]);
            a = a.max(p.min(q));
            b = b.min(p.max(q));
        }
        sum += (b - a).max(0.0);
    }
    sum
}

/// ADR-0047's filter (lavapipe, a 64 x 36 frame, lamp off, exposure 8). A wall lit by an ore
/// strip on its own side, behind a pane 1 voxel thick on one half and 2 on the other; no
/// emitter sees the pane's camera-side faces, so their store cells are 0 and each pixel is
/// `T(L)` of what it showed before the pane: `L` the exact path through the glass boxes along the
/// pixel's ray, `T = tint^L` per channel, `tint = 0.8 (0.5 + 0.5 albedo / max(albedo))` from the
/// row's albedo. Within 2 levels, head on (the halves at `L` near 1 and 2: `T` and `T^2`) and at
/// 30 degrees of yaw (`L` near 2.3 through the 2-voxel half). Then a pane lit from the camera's
/// side over a black wall: each pixel's reflected light before exposure (the layer target's rgb)
/// is Schlick's `R` at F0 0.04 at its angle times its entry face's store cell `E / PI`, within
/// 10 %, and the pixel shows it. The per-layer composite fails the first.
#[test]
fn a_glass_pane_filters_by_path_length() {
    let mut t = table();
    let rock = t.get(ROCK).expect("rock").clone();
    let albedo = [0.25, 0.35, 0.85];
    t.push(vxe_world::Material {
        name: "glass".to_owned(),
        albedo,
        emission: [0.0; 3],
        opacity: 0.6,
        flags: rock.flags | vxe_world::MaterialFlags::TRANSLUCENT,
        ..rock.clone()
    });
    let glass_id = (t.len() - 1) as vxe_world::MaterialId;
    t.push(vxe_world::Material {
        name: "black".to_owned(),
        albedo: [0.0; 3],
        emission: [0.0; 3],
        ..rock.clone()
    });
    let black_id = (t.len() - 1) as vxe_world::MaterialId;
    let mats = t.clone();
    let world = air_world([2, 2, 2], &t);
    // The wall and its ore strip above, out of the views.
    fill(&world, [40, 52, 22], [41, 100, 40], ROCK);
    fill(&world, [34, 52, 42], [36, 100, 44], ORE);
    // The lit pane's black wall and its ore strip, on the camera's side of the pane.
    fill(&world, [84, 54, 92], [85, 88, 118], black_id);
    fill(&world, [100, 56, 114], [104, 84, 116], ORE);
    let halves: [([i32; 3], [i32; 3]); 2] =
        [([30, 56, 20], [31, 70, 42]), ([30, 70, 20], [32, 86, 42])];
    let lit_pane = ([96, 56, 96], [98, 84, 112]);
    let size = [64u32, 36];
    let Some(mut rig) = rig_on(world, size, Light::Baked, None) else {
        return;
    };
    let settings = test_counts(64, 64);
    rig.bake(&settings);
    let ex = 8.0f32;
    let head_on = Camera {
        position: [20.0, 70.0, 31.0].into(),
        yaw: 0.0,
        pitch: 0.0,
        vfov: 30f32.to_radians(),
    };
    let oblique = Camera {
        position: [31.0 - 8.66, 72.0, 31.0].into(),
        yaw: 30f32.to_radians(),
        ..head_on
    };
    let lit_view = Camera {
        position: [108.0, 70.0, 104.0].into(),
        yaw: 180f32.to_radians(),
        ..head_on
    };
    let lighting = rig.frame.lighting();
    let level = |c: [f32; 3]| {
        let b = tone_bytes([c[0], c[1], c[2], 1.0], &lighting);
        0.2126 * f32::from(b[0]) + 0.7152 * f32::from(b[1]) + 0.0722 * f32::from(b[2])
    };
    let settle = |rig: &mut Rig, camera: &Camera| {
        for _ in 0..8 {
            rig.step(camera);
        }
    };
    let mut before = Vec::new();
    for camera in [&head_on, &oblique] {
        settle(&mut rig, camera);
        before.push(hit_radiance(&rig.device, &rig.queue, &rig.frame, &mats, ex));
    }
    for (min, max) in halves.iter().chain([&lit_pane]) {
        rig.dig(
            &EditOp::box_(
                Fixed3(min.map(|v| i64::from(v) * FIXED_ONE)),
                Fixed3(max.map(|v| i64::from(v) * FIXED_ONE)),
                Mode::Fill(glass_id),
            ),
            &settings,
        );
    }
    let top = albedo.iter().fold(0f32, |a, b| a.max(*b));
    let tint = albedo.map(|a| f64::from(0.8 * (0.5 + 0.5 * a / top)));
    for (camera, c0, name) in [
        (&head_on, &before[0], "head on"),
        (&oblique, &before[1], "30 degrees"),
    ] {
        settle(&mut rig, camera);
        let shown = luma(&rig.display());
        let info = readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
        let pos = readback::read_float(&rig.device, &rig.queue, rig.frame.hit_positions());
        let eye = camera.position.to_array().map(f64::from);
        let (mut worst, mut n) = (0f32, 0usize);
        let mut by_half: [Vec<f64>; 2] = [Vec::new(), Vec::new()];
        for i in 0..info.len() {
            if info[i][3] & 1 == 0 || info[i][3] & 4 == 0 {
                continue;
            }
            let to = [0, 1, 2].map(|k| f64::from(pos[i][k]) - eye[k]);
            let s_end = (to[0] * to[0] + to[1] * to[1] + to[2] * to[2]).sqrt();
            let d = to.map(|v| v / s_end);
            let lens = halves.map(|b| path_in_boxes(eye, d, s_end, &[b]));
            let l = lens[0] + lens[1];
            let tr = tint.map(|x| x.powf(l) as f32);
            let want = level([0, 1, 2].map(|k| tr[k] * c0[i][k]));
            let off = (shown[i] - want).abs();
            if off > worst {
                worst = off;
            }
            n += 1;
            if lens[1] == 0.0 {
                by_half[0].push(l);
            } else if lens[0] == 0.0 {
                by_half[1].push(l);
            }
        }
        let median = |v: &mut Vec<f64>| {
            v.sort_by(f64::total_cmp);
            v.get(v.len() / 2).copied().unwrap_or(0.0)
        };
        let (n1, n2) = (by_half[0].len(), by_half[1].len());
        let (l1, l2) = (median(&mut by_half[0]), median(&mut by_half[1]));
        println!(
            "{name}: {n} pane pixels; L median {l1:.3} over {n1} (1 voxel), {l2:.3} over {n2} (2 voxels); worst |pixel - T(L) behind| {worst:.2} levels"
        );
        assert!(n > 1000, "{name}: {n} pixels behind the pane");
        assert!(
            worst <= 2.0,
            "{name}: a pixel {worst:.2} levels from T(L) of what it showed before the pane"
        );
        if camera.yaw == 0.0 {
            assert!(n1 > 300 && n2 > 300, "{name}: halves {n1} and {n2}");
            assert!(
                (1.0..1.1).contains(&l1) && (2.0..2.2).contains(&l2),
                "{l1} {l2}"
            );
        } else {
            assert!(
                n2 > 300 && (2.25..2.4).contains(&l2),
                "{name}: {n2} at {l2}"
            );
        }
    }
    // The lit pane over the black wall: its reflected share from its +x face's cell.
    settle(&mut rig, &lit_view);
    let shown = luma(&rig.display());
    let info = readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    let pos = readback::read_float(&rig.device, &rig.queue, rig.frame.hit_positions());
    let layer = readback::read_float(&rig.device, &rig.queue, rig.frame.layers());
    let cells = rig.cells();
    let eye = lit_view.position.to_array().map(f64::from);
    let lum = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let (mut n, mut worst, mut worst_shown) = (0usize, 0f32, 0f32);
    for i in 0..info.len() {
        if info[i][3] & 1 == 0 || info[i][3] & 4 == 0 || info[i][2] != black_id {
            continue;
        }
        let to = [0, 1, 2].map(|k| f64::from(pos[i][k]) - eye[k]);
        let len = (to[0] * to[0] + to[1] * to[1] + to[2] * to[2]).sqrt();
        let d = to.map(|v| v / len);
        // The entry: the plane x = 98, the pane's +x face; its voxel is x = 97.
        let s = (98.0 - eye[0]) / d[0];
        let p = [0, 1, 2].map(|k| eye[k] + d[k] * s);
        let v = [97, p[1].floor() as i32, p[2].floor() as i32];
        let Some(cell) = cells.get(&(v, 1)) else {
            panic!("no cell at the pane's face {v:?}");
        };
        assert!(
            cell[1] & bake::CELL_TRANSLUCENT != 0,
            "the face's tail is flagged"
        );
        let e = bake::rgb9e5_unpack(cell[0])
            .map(|x| x * rig.frame.emission_scale((cell[1] >> 16) as u16));
        let c = (1.0 - d[0].abs()) as f32;
        let r = 0.04 + 0.96 * c.powi(5);
        let want = e.map(|x| r * x / std::f32::consts::PI);
        let got = [layer[i][0], layer[i][1], layer[i][2]];
        if lum(want) <= 0.0 {
            continue;
        }
        n += 1;
        worst = worst.max((lum(got) / lum(want) - 1.0).abs());
        worst_shown = worst_shown.max((shown[i] - level(got.map(|x| x * ex))).abs());
    }
    println!(
        "lit pane: {n} pixels over the black wall; worst |reflected / (R E / PI) - 1| {worst:.4}; worst |pixel - display(reflected)| {worst_shown:.2} levels"
    );
    assert!(n > 1000, "{n} lit pane pixels");
    assert!(worst <= 0.10, "reflected light {worst:.4} off R E / PI");
    assert!(
        worst_shown <= 2.0,
        "a pixel {worst_shown:.2} levels off its reflected light"
    );
}

/// A written store file loads into a new frame with every cell and a frame bit-identical; a file
/// with another seed, piece, light radius or stop is refused with the field's name.
#[test]
fn a_store_file_loads_the_same_light() {
    let (world, camera) = small_box();
    let Some(mut rig) = rig_on(world, [64, 36], Light::Baked, None) else {
        return;
    };
    let settings = test_counts(64, 256);
    rig.bake(&settings);
    let cells = rig.cells();
    rig.step(&camera);
    let frame = rig.display();
    let extent = rig.world.extent();
    let key = |seed: u64, piece: ChunkExtent, settings: &BakeSettings, gpu: &GpuWorld| {
        Frame::store_key(gpu, extent, seed, piece, ChunkCoord([0, 0, 0]), 2, settings)
    };
    let k = key(1, extent, &settings, &rig.gpu);
    let dir = std::env::temp_dir().join(format!("vxe-store-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a temp dir");
    let path = dir.join("store.bin");
    let bytes = rig.frame.write_store_file(&path, &k).expect("written");
    assert!(bytes > 0);
    let mut loaded = new_frame(&rig.device, [64, 36], Light::Baked);
    loaded
        .load_store_file(&rig.device, &rig.queue, &mut rig.gpu, &rig.world, &path, &k)
        .expect("the file loads");
    std::mem::swap(&mut rig.frame, &mut loaded);
    assert_eq!(rig.cells(), cells, "the loaded cells");
    rig.step(&camera);
    assert_eq!(rig.display(), frame, "a frame from the loaded store");
    let one = ChunkExtent::new([0, 0, 0], [1, 1, 1]);
    let other_light = MirrorConfig {
        light_radius: rig.cfg.light_radius + 1,
        ..rig.cfg
    };
    let other_gpu = GpuWorld::with_config(&rig.device, other_light);
    // A refused file leaves the store untouched: the key is read before anything is loaded.
    for (bad, field) in [
        (key(2, extent, &settings, &rig.gpu), "world seed"),
        (key(1, one, &settings, &rig.gpu), "piece"),
        (key(1, extent, &settings, &other_gpu), "light radius"),
        (
            key(
                1,
                extent,
                &BakeSettings {
                    n_max: 512,
                    ..settings.clone()
                },
                &rig.gpu,
            ),
            "n_max",
        ),
        (
            key(
                1,
                extent,
                &BakeSettings {
                    se: 0.25,
                    ..settings.clone()
                },
                &rig.gpu,
            ),
            "stop's standard error",
        ),
    ] {
        let e = rig
            .frame
            .load_store_file(
                &rig.device,
                &rig.queue,
                &mut rig.gpu,
                &rig.world,
                &path,
                &bad,
            )
            .expect_err("refused");
        assert!(e.to_string().contains(field), "{e} names {field}");
    }
    assert_eq!(rig.cells(), cells, "the cells after the refusals");
    std::fs::remove_dir_all(&dir).ok();
}

/// A box of two chunks whose emitter lies outside a one-chunk piece: the piece's cells are a whole
/// bake's, the chunk outside has no block and its hits count in `store_missing`; a carve that
/// straddles the piece's edge re-bakes only cells inside it, bit-identical to a fresh piece bake
/// of the edited world, and makes no block outside.
#[test]
fn a_piece_reads_what_the_whole_world_reads() {
    let mut t = MaterialTable::new();
    for (id, row) in flythrough::cave_materials()
        .rows()
        .iter()
        .enumerate()
        .skip(1)
    {
        let mut m = row.clone();
        if id == usize::from(ORE) {
            m.emission = [0.05, 0.05, 0.05];
        }
        assert_eq!(usize::from(t.push(m)), id);
    }
    let world = air_world([2, 1, 1], &t);
    fill(&world, [40, 16, 20], [88, 48, 21], ROCK);
    fill(&world, [60, 30, 21], [61, 31, 22], ORE);
    let Some(mut rig) = rig_on(world, [64, 36], Light::Baked, None) else {
        return;
    };
    let settings = dig_counts();
    rig.bake(&settings);
    let whole = rig.cells();
    let piece = ChunkExtent::new([1, 0, 0], [2, 1, 1]);
    rig.frame
        .bake_until(
            &rig.device,
            &rig.queue,
            &mut rig.gpu,
            &rig.world,
            piece,
            &settings,
            &mut |_| {},
        )
        .expect("the piece fits");
    let blocks = rig.blocks();
    assert!(
        !blocks.contains_key(&[0, 0, 0]),
        "a block outside the piece"
    );
    let cells = cells_of(&blocks);
    let inside: BTreeMap<_, _> = whole.iter().filter(|((v, _), _)| v[0] >= 64).collect();
    let lit = cells.values().filter(|c| c[0] != 0).count();
    assert!(lit > 50, "{lit} lit cells in the piece");
    assert_eq!(
        cells.iter().collect::<BTreeMap<_, _>>(),
        inside,
        "the piece against the whole bake"
    );
    let camera = Camera {
        position: [64.0, 32.0, 40.0].into(),
        yaw: 0.0,
        pitch: (-80f32).to_radians(),
        vfov: 90f32.to_radians(),
    };
    rig.step(&camera);
    let stats = rig.frame.read_store_stats(&rig.device, &rig.queue);
    println!("store_missing {} over the chunk outside", stats.missing);
    assert!(stats.missing > 100, "store_missing {}", stats.missing);
    let (dig, _) = rig.dig(&sphere([64, 30, 20], 3, Mode::Carve), &settings);
    assert!(dig.bake.faces > 0);
    let blocks = rig.blocks();
    assert!(
        !blocks.contains_key(&[0, 0, 0]),
        "a block outside after the dig"
    );
    let fresh = rig.fresh_cells(piece, &settings);
    assert_eq!(
        cells_of(&blocks),
        fresh,
        "the dug piece against a fresh piece bake"
    );
}

// ---- The count reading ----

/// Luma of the display value of irradiance `c` at exposure 8, through an `rgb9e5` cell: `D_f`.
fn d_of(c: [f32; 3], lighting: &Lighting) -> f32 {
    let v = bake::rgb9e5_unpack(bake::rgb9e5_pack(c)).map(|v| v * 8.0);
    let b = tone_bytes([v[0], v[1], v[2], 1.0], lighting);
    0.2126 * f32::from(b[0]) + 0.7152 * f32::from(b[1]) + 0.0722 * f32::from(b[2])
}

fn q(v: &mut [f32], f: f64) -> f32 {
    v.sort_by(f32::total_cmp);
    if v.is_empty() {
        return f32::NAN;
    }
    v[((v.len() - 1) as f64 * f).round() as usize]
}

/// A view of the reading: the name, the world and the eye.
#[derive(Clone, Copy, PartialEq)]
enum Scene {
    Level,
    Showcase,
}

struct ReadView {
    name: &'static str,
    scene: Scene,
    eye: [f32; 3],
}

fn read_views() -> Vec<ReadView> {
    use flythrough::{PATCH_POSE, VEIN_POSE};
    let hall = flythrough::showcase::SHOTS[0].eye.0;
    vec![
        ReadView {
            name: "vein",
            scene: Scene::Level,
            eye: VEIN_POSE.position,
        },
        ReadView {
            name: "back",
            scene: Scene::Level,
            eye: support::settle::VEIN_FLY_IN_START,
        },
        ReadView {
            name: "patch",
            scene: Scene::Level,
            eye: PATCH_POSE.position,
        },
        ReadView {
            name: "hall",
            scene: Scene::Showcase,
            eye: hall.map(|v| v as f32),
        },
    ]
}

fn scene_world(scene: Scene) -> World {
    match scene {
        Scene::Level => {
            let source = flythrough::cave::CaveSource::new(
                1,
                ChunkExtent::new([0, 0, 0], flythrough::cave::LEVEL_CHUNKS),
            );
            World::generate(source.extent, &source, &table())
        }
        Scene::Showcase => {
            let source = flythrough::showcase::ShowcaseSource::new(1);
            World::generate(source.extent(), &source, &flythrough::showcase_materials())
        }
    }
}

struct ReadRig {
    device: wgpu::Device,
    queue: wgpu::Queue,
    gpu: GpuWorld,
    world: World,
    frame: Frame,
    adapter: String,
}

fn read_rig(scene: Scene, eye: [f32; 3]) -> ReadRig {
    let world = scene_world(scene);
    let pool = flythrough::level_pool_units(&world, false) * UNIT_BYTES;
    let (device, queue, adapter) = reading_device(pool).expect("a device");
    let cfg = MirrorConfig {
        budget_bytes: 512 << 20,
        light_radius: if scene == Scene::Showcase { 19 } else { 4 },
        ..MirrorConfig::new(Window::Fixed(world.extent()))
    };
    let mut gpu = GpuWorld::with_config(&device, cfg);
    gpu.set_focus(vxe_world::VoxelCoord(eye.map(|v| v as i32)));
    for _ in 0..8 {
        gpu.sync_now(&world, &device, &queue)
            .expect("the mirror fits the adapter");
    }
    let mut frame = Frame::new(&device, 16, 16);
    frame.set_lighting(Lighting {
        lamp: false,
        lamp_bounce: false,
        exposure: Exposure::Manual(8.0),
        light: Light::Baked,
        ..Lighting::default()
    });
    ReadRig {
        device,
        queue,
        gpu,
        world,
        frame,
        adapter,
    }
}

/// The chunks that hold any voxel within `r` voxels of `eye`.
fn extent_within(world: ChunkExtent, eye: [f32; 3], r: f32) -> ChunkExtent {
    let size = world.size();
    let lo = [0, 1, 2].map(|k| (((eye[k] - r) / 64.0).floor() as i32).max(world.min.0[k]));
    let hi =
        [0, 1, 2].map(|k| (((eye[k] + r) / 64.0).floor() as i32 + 1).min(world.min.0[k] + size[k]));
    ChunkExtent::new(lo, hi)
}

/// Faces within 96 voxels of the eye, by voxel centre.
fn within_96(eye: [f32; 3]) -> bake::FaceFilter {
    Arc::new(move |v, _| {
        (0..3)
            .map(|k| (v[k] as f32 + 0.5 - eye[k]).powi(2))
            .sum::<f32>()
            <= 96.0 * 96.0
    })
}

/// A log under `live/`, written line by line so it outlives a lost device.
struct Log(std::fs::File);

impl Log {
    fn open(name: &str) -> Self {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../live/bake-count");
        std::fs::create_dir_all(dir).expect("live/bake-count");
        Self(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(format!("{dir}/{name}"))
                .expect("the log"),
        )
    }

    fn line(&mut self, s: &str) {
        use std::io::Write as _;
        println!("{s}");
        writeln!(self.0, "{s}").expect("the log");
        self.0.flush().expect("the log");
    }
}

/// `VXE_BAKE_PART`: `census` (CPU, the whole worlds and each view's piece at `VXE_BAKE_RADIUS`), `grid` (fixed counts, candidates, the clamp, two seed sets),
/// `stop` (the stop at `VXE_BAKE_CANDIDATES` and `VXE_BAKE_CLAMP`), each to `live/bake-count/`.
/// `VXE_BAKE_VIEWS` names the views (default all), `VXE_BAKE_N` the deepest count (default 262144)
/// and `VXE_BAKE_CANDIDATE_ARMS` the candidates of the grid (default `8,32`). The stop takes
/// `VXE_BAKE_SEEDS` (default `0,0x5EED0001`) and `VXE_BAKE_FIXED`, a fixed count run first for
/// the stop's rate.
#[test]
#[ignore = "a reading on the RTX, minutes an arm: VXE_TEST_ADAPTER=rtx, under the perf lock"]
fn bake_count_readings() {
    let part = std::env::var("VXE_BAKE_PART").unwrap_or_else(|_| "grid".to_owned());
    if part == "census" {
        census();
        return;
    }
    let names =
        std::env::var("VXE_BAKE_VIEWS").unwrap_or_else(|_| "vein,back,patch,hall".to_owned());
    for view in read_views() {
        if !names.split(',').any(|n| n == view.name) {
            continue;
        }
        match part.as_str() {
            "grid" => grid_view(&view),
            "stop" => stop_view(&view),
            _ => panic!("VXE_BAKE_PART takes census, grid or stop"),
        }
    }
}

fn census() {
    let mut log = Log::open("census.txt");
    for (name, scene) in [("level", Scene::Level), ("showcase", Scene::Showcase)] {
        let world = scene_world(scene);
        let classes = bake::face_classes(world.materials());
        let e = world.extent();
        let s = e.size();
        let (mut faces, mut words) = (Vec::new(), 0u64);
        for z in 0..s[2] {
            for y in 0..s[1] {
                for x in 0..s[0] {
                    let c = ChunkCoord([x, y, z]);
                    if let Some(f) = bake::list_world_chunk(&world, c, &classes)
                        && f.faces() > 0
                    {
                        faces.push(f.faces() as u32);
                        words += f.block_words();
                    }
                }
            }
        }
        faces.sort_unstable();
        let total: u64 = faces.iter().map(|&f| u64::from(f)).sum();
        let chunks = (s[0] * s[1] * s[2]) as usize;
        let at = |f: f64| faces[((faces.len() - 1) as f64 * f).round() as usize];
        let table = (s[0] * s[1] * s[2]) as u64 + bake::STORE_TABLE;
        log.line(&format!(
            "census {name}: {} chunks, {} surface ({:.3}), faces {total}, per surface chunk p50 {} p99 {} max {}, store {} B",
            chunks,
            faces.len(),
            faces.len() as f64 / chunks as f64,
            at(0.5),
            at(0.99),
            at(1.0),
            (words + table) * 4
        ));
    }
    // Each view's piece at `VXE_BAKE_RADIUS` (default 1): the chunks within it of the eye's
    // chunk on each axis, inside the world, as the fly-through's `--bake-radius` takes them.
    let r: i32 = std::env::var("VXE_BAKE_RADIUS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    for view in read_views() {
        let world = scene_world(view.scene);
        let classes = bake::face_classes(world.materials());
        let e = world.extent();
        let size = e.size();
        let c = view.eye.map(|v| (v / 64.0).floor() as i32);
        let lo = [0, 1, 2].map(|k| (c[k] - r).max(e.min.0[k]));
        let hi = [0, 1, 2].map(|k| (c[k] + r + 1).min(e.min.0[k] + size[k]));
        let (mut chunks, mut surface, mut faces) = (0u32, 0u32, 0u64);
        for z in lo[2]..hi[2] {
            for y in lo[1]..hi[1] {
                for x in lo[0]..hi[0] {
                    chunks += 1;
                    if let Some(f) = bake::list_world_chunk(&world, ChunkCoord([x, y, z]), &classes)
                        && f.faces() > 0
                    {
                        surface += 1;
                        faces += f.faces() as u64;
                    }
                }
            }
        }
        log.line(&format!(
            "piece {} r{r} at chunk {c:?}: {chunks} chunks, {surface} surface, faces {faces}",
            view.name
        ));
    }
}

/// One run of a view's grid.
struct GridRun {
    candidates: u32,
    seed: u32,
    faces: Vec<bake::FaceOutcome>,
}

/// `(voxel, face)` of an outcome.
fn key_of(o: &bake::FaceOutcome) -> ([i32; 3], u8) {
    (o.voxel, o.face)
}

/// Emitter voxels of `world` near `extent`, by chunk.
fn emitters_by_chunk(world: &World, extent: ChunkExtent) -> HashMap<[i32; 3], Vec<[i32; 3]>> {
    let mut out: HashMap<[i32; 3], Vec<[i32; 3]>> = HashMap::new();
    let emits: Vec<bool> = world
        .materials()
        .rows()
        .iter()
        .map(|m| m.emission.iter().any(|&e| e > 0.0))
        .collect();
    let s = extent.size();
    for z in -1..=s[2] {
        for y in -1..=s[1] {
            for x in -1..=s[0] {
                let c = [
                    extent.min.0[0] + x,
                    extent.min.0[1] + y,
                    extent.min.0[2] + z,
                ];
                let Some(chunk) = world.store().get(ChunkCoord(c)) else {
                    continue;
                };
                let chunk = chunk.read().expect("the chunk");
                let list = out.entry(c).or_default();
                for slot in 0..512 {
                    let b = vxe_world::brick_of_slot(slot);
                    let brick = chunk.brick(slot);
                    for i in 0..512usize {
                        if emits
                            .get(usize::from(brick.get(i)))
                            .copied()
                            .unwrap_or(false)
                        {
                            let i = i as i32;
                            let inner = [i % 8, i / 8 % 8, i / 64];
                            list.push([0, 1, 2].map(|k| c[k] * 64 + b[k] * 8 + inner[k]));
                        }
                    }
                }
            }
        }
    }
    out
}

/// The distance band to a face's nearest emitter: under 4, 4 to 16, 16 to 64, over 64 voxels.
fn band_of(emitters: &HashMap<[i32; 3], Vec<[i32; 3]>>, v: [i32; 3]) -> usize {
    let c = v.map(|a| a.div_euclid(64));
    let mut best = i64::MAX;
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                if let Some(list) = emitters.get(&[c[0] + dx, c[1] + dy, c[2] + dz]) {
                    for e in list {
                        let d2 = (0..3).map(|k| i64::from(e[k] - v[k]).pow(2)).sum();
                        best = best.min(d2);
                    }
                }
            }
        }
    }
    match best {
        d if d < 16 => 0,
        d if d < 256 => 1,
        d if d < 4096 => 2,
        _ => 3,
    }
}

/// `G_plane` over `faces`: `|D_f - mean(D_n)|` over the faces with at least two coplanar edge
/// neighbours, a neighbour the coplanar face, then a voxel toward the air, then one into the rock.
fn g_plane(faces: &HashMap<([i32; 3], u8), f32>) -> Vec<f32> {
    let mut out = Vec::new();
    for (&(v, side), &d) in faces {
        let axis = usize::from(side / 2);
        let sign = if side % 2 == 1 { 1 } else { -1 };
        let (u, w) = ((axis + 1) % 3, (axis + 2) % 3);
        let (mut sum, mut n, mut edges) = (0f32, 0u32, 0u32);
        for du in -1..=1i32 {
            for dw in -1..=1i32 {
                if du == 0 && dw == 0 {
                    continue;
                }
                for (step, off) in [0, sign, -sign].into_iter().enumerate() {
                    let mut c = v;
                    c[u] += du;
                    c[w] += dw;
                    c[axis] += off;
                    if let Some(&dn) = faces.get(&(c, side)) {
                        sum += dn;
                        n += 1;
                        edges += u32::from(step == 0 && du.abs() + dw.abs() == 1);
                        break;
                    }
                }
            }
        }
        if edges >= 2 {
            out.push((d - sum / n as f32).abs());
        }
    }
    out
}

/// The display value of each non-emitting face of `run` at `n` samples, `arm` 0 unclamped, 1 clamped.
fn displayed(
    run: &[bake::FaceOutcome],
    n: u32,
    arm: usize,
    emitter: &dyn Fn([i32; 3]) -> bool,
) -> HashMap<([i32; 3], u8), f32> {
    let l = Lighting::default();
    run.iter()
        .filter(|o| !emitter(o.voxel))
        .filter_map(|o| {
            let s = o.snapshots.iter().find(|s| s.0 == n)?;
            let c = if arm == 0 { s.1 } else { s.2 };
            Some((key_of(o), d_of(c, &l)))
        })
        .collect()
}

fn grid_view(view: &ReadView) {
    let n_deep: u32 = std::env::var("VXE_BAKE_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(262_144);
    let cand_arms: Vec<u32> = std::env::var("VXE_BAKE_CANDIDATE_ARMS")
        .unwrap_or_else(|_| "8,32".to_owned())
        .split(',')
        .map(|v| v.parse().expect("candidates"))
        .collect();
    let counts: Vec<u32> = [4096u32, 16_384, 65_536, 262_144]
        .into_iter()
        .filter(|&n| n <= n_deep)
        .collect();
    let mut log = Log::open(&format!("grid-{}.txt", view.name));
    let mut rig = read_rig(view.scene, view.eye);
    let extent = extent_within(rig.world.extent(), view.eye, 96.0);
    log.line(&format!(
        "view {} eye {:?} scene light radius {} adapter {} extent {:?} {:?} n {counts:?} candidates {cand_arms:?}",
        view.name,
        view.eye,
        rig.gpu.light_radius(),
        rig.adapter,
        extent.min.0,
        extent.size(),
    ));
    let mut runs: Vec<GridRun> = Vec::new();
    for &candidates in &cand_arms {
        for seed in [0u32, 0x5EED_0001] {
            let settings = BakeSettings {
                candidates,
                clamp: false,
                seed,
                round: bake::BAKE_ROUND,
                n_min: n_deep,
                n_max: n_deep,
                keep_faces: true,
                snapshots: counts.clone(),
                filter: Some(within_96(view.eye)),
                ..BakeSettings::default()
            };
            let start = std::time::Instant::now();
            let mut printed = 0.0;
            let label = format!("c{candidates} seed {seed:#x}");
            let report = rig
                .frame
                .bake_until(
                    &rig.device,
                    &rig.queue,
                    &mut rig.gpu,
                    &rig.world,
                    extent,
                    &settings,
                    &mut |p| {
                        let s = start.elapsed().as_secs_f64();
                        if s - printed >= 30.0 {
                            printed = s;
                            println!(
                                "  {label}: {} of {} chunks, {:.3e} samples, {:.0} s",
                                p.chunks_done, p.chunks, p.samples as f64, s
                            );
                        }
                    },
                )
                .expect("the bake runs");
            log.line(&format!(
                "run {label}: faces {} samples {:.4e} wall {:.1} s, ns a sample {:.2}, mean shadow ray {:.1} voxels, rays per sample {:.3}, light samples {}",
                report.faces,
                report.samples as f64,
                start.elapsed().as_secs_f64(),
                report.gpu_seconds * 1e9 / report.samples as f64,
                report.ray_len / report.rays.max(1) as f64,
                report.rays as f64 / report.samples as f64,
                rig.gpu.light_samples(),
            ));
            runs.push(GridRun {
                candidates,
                seed,
                faces: report.outcomes,
            });
        }
    }
    let world = &rig.world;
    let emits: Vec<bool> = world
        .materials()
        .rows()
        .iter()
        .map(|m| m.emission.iter().any(|&e| e > 0.0))
        .collect();
    let emitter = |v: [i32; 3]| {
        world
            .material_at(v)
            .and_then(|m| emits.get(usize::from(m)).copied())
            .unwrap_or(false)
    };
    let emitters = emitters_by_chunk(world, extent);
    // The reference: the mean, over every run, of the deepest count unclamped.
    let mut reference_sum: HashMap<([i32; 3], u8), ([f64; 3], u32)> = HashMap::new();
    for r in &runs {
        for o in &r.faces {
            if let Some(s) = o.snapshots.iter().find(|s| s.0 == n_deep) {
                let e = reference_sum.entry(key_of(o)).or_insert(([0.0; 3], 0));
                for k in 0..3 {
                    e.0[k] += f64::from(s.1[k]);
                }
                e.1 += 1;
            }
        }
    }
    let l = Lighting::default();
    let reference: HashMap<([i32; 3], u8), f32> = reference_sum
        .iter()
        .filter(|(k, _)| !emitter(k.0))
        .map(|(k, (sum, n))| (*k, d_of(sum.map(|v| (v / f64::from(*n)) as f32), &l)))
        .collect();
    let zero = reference.values().filter(|d| **d == 0.0).count();
    log.line(&format!(
        "reference: {} faces (emitters out), {} read 0 ({:.3})",
        reference.len(),
        zero,
        zero as f64 / reference.len().max(1) as f64
    ));
    // Deciles of the reference's brightness.
    let mut sorted: Vec<f32> = reference.values().copied().collect();
    sorted.sort_by(f32::total_cmp);
    let decile_of = |d: f32| -> usize {
        (sorted.partition_point(|v| *v < d) * 10 / sorted.len().max(1)).min(9)
    };
    let bands: HashMap<([i32; 3], u8), usize> = reference
        .keys()
        .map(|k| (*k, band_of(&emitters, k.0)))
        .collect();
    for &candidates in &cand_arms {
        let pair: Vec<&GridRun> = runs.iter().filter(|r| r.candidates == candidates).collect();
        let [a, b] = [pair[0], pair[1]];
        debug_assert_ne!(a.seed, b.seed);
        for &n in &counts {
            for (arm, arm_name) in ["off", "on"].iter().enumerate() {
                let da = displayed(&a.faces, n, arm, &emitter);
                let db = displayed(&b.faces, n, arm, &emitter);
                let mut all = Vec::new();
                let mut by_band: [Vec<f32>; 4] = Default::default();
                let mut by_decile: [Vec<f32>; 10] = Default::default();
                let mut bias: [(f64, u32); 10] = [(0.0, 0); 10];
                for (k, &x) in &da {
                    let Some(&y) = db.get(k) else { continue };
                    let g = (x - y).abs() / 2f32.sqrt();
                    all.push(g);
                    by_band[bands[k]].push(g);
                    let r = reference[k];
                    let d = decile_of(r);
                    by_decile[d].push(g);
                    bias[d].0 += f64::from((x + y) * 0.5 - r);
                    bias[d].1 += 1;
                }
                let mut plane = g_plane(&da);
                let mut row = format!(
                    "{} c{candidates} n{n} clamp {arm_name}: G_noise {:.2} / {:.2} ({}) G_plane {:.2} / {:.2}",
                    view.name,
                    q(&mut all, 0.5),
                    q(&mut all, 0.9),
                    all.len(),
                    q(&mut plane, 0.5),
                    q(&mut plane, 0.9),
                );
                row.push_str(" | bands");
                for b in &mut by_band {
                    let _ = write!(row, " {:.2}/{:.2}({})", q(b, 0.5), q(b, 0.9), b.len());
                }
                row.push_str(" | deciles");
                for b in &mut by_decile {
                    let _ = write!(row, " {:.2}/{:.2}", q(b, 0.5), q(b, 0.9));
                }
                row.push_str(" | bias");
                for (s, c) in &bias {
                    let _ = write!(row, " {:+.2}", s / f64::from((*c).max(1)));
                }
                log.line(&row);
            }
        }
    }
}

fn stop_view(view: &ReadView) {
    let candidates: u32 = std::env::var("VXE_BAKE_CANDIDATES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(bake::KEPT_CANDIDATES);
    let clamp = std::env::var("VXE_BAKE_CLAMP").is_ok_and(|v| v == "on");
    let mut log = Log::open(&format!("stop-{}.txt", view.name));
    let mut rig = read_rig(view.scene, view.eye);
    let extent = extent_within(rig.world.extent(), view.eye, 96.0);
    log.line(&format!(
        "stop {} candidates {candidates} clamp {clamp} adapter {} extent {:?} {:?}",
        view.name,
        rig.adapter,
        extent.min.0,
        extent.size()
    ));
    // `VXE_BAKE_FIXED`: a fixed count's run of the same faces first, seed 0, for the stop's rate.
    if let Some(n) = std::env::var("VXE_BAKE_FIXED")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
    {
        let settings = BakeSettings {
            candidates,
            clamp,
            n_min: n,
            n_max: n,
            filter: Some(within_96(view.eye)),
            ..BakeSettings::default()
        };
        let start = std::time::Instant::now();
        let report = rig
            .frame
            .bake_until(
                &rig.device,
                &rig.queue,
                &mut rig.gpu,
                &rig.world,
                extent,
                &settings,
                &mut |_| {},
            )
            .expect("the bake runs");
        let wall = start.elapsed().as_secs_f64();
        log.line(&format!(
            "fixed n{n} seed 0x0: faces {} samples {} wasted {} submits {}, wall {:.1} s, wall ns a sample kept {:.2}, submit ns a sample {:.2}",
            report.faces,
            report.samples,
            report.wasted,
            report.submits,
            wall,
            wall * 1e9 / report.samples.max(1) as f64,
            report.gpu_seconds * 1e9 / report.samples.max(1) as f64,
        ));
    }
    // `VXE_BAKE_SEEDS`: the seeds (default both); `G_noise` needs two.
    let seeds: Vec<u32> = std::env::var("VXE_BAKE_SEEDS")
        .unwrap_or_else(|_| "0,0x5EED0001".to_owned())
        .split(',')
        .map(|v| {
            let v = v.trim();
            v.strip_prefix("0x")
                .map_or_else(|| v.parse(), |h| u32::from_str_radix(h, 16))
                .expect("a seed")
        })
        .collect();
    let mut seeded = Vec::new();
    for seed in seeds {
        let settings = BakeSettings {
            candidates,
            clamp,
            seed,
            keep_faces: true,
            filter: Some(within_96(view.eye)),
            ..BakeSettings::default()
        };
        let start = std::time::Instant::now();
        let mut printed = 0.0;
        let report = rig
            .frame
            .bake_until(
                &rig.device,
                &rig.queue,
                &mut rig.gpu,
                &rig.world,
                extent,
                &settings,
                &mut |p| {
                    let s = start.elapsed().as_secs_f64();
                    if s - printed >= 30.0 {
                        printed = s;
                        println!(
                            "  seed {seed:#x}: {} of {} chunks, {} of {} faces, {:.3e} samples, {:.0} s",
                            p.chunks_done, p.chunks, p.faces_done, p.faces, p.samples as f64, s
                        );
                    }
                },
            )
            .expect("the bake runs");
        let wall = start.elapsed().as_secs_f64();
        log.line(&format!(
            "seed {seed:#x}: faces {} samples {:.4e} ({:.0} a face), at N_MIN {} at N_MAX {}, wall {:.1} s, ns a sample {:.2}, mean shadow ray {:.1}",
            report.faces,
            report.samples as f64,
            report.samples as f64 / report.faces.max(1) as f64,
            report.faces_at_min,
            report.faces_at_max,
            wall,
            report.gpu_seconds * 1e9 / report.samples.max(1) as f64,
            report.ray_len / report.rays.max(1) as f64,
        ));
        log.line(&format!(
            "seed {seed:#x} submits: samples kept {} wasted {} (share {:.4}), submits {}, most jobs a submit {}, most rounds a face a submit {}, wall ns a sample kept {:.2}",
            report.samples,
            report.wasted,
            report.wasted as f64 / report.samples.max(1) as f64,
            report.submits,
            report.max_jobs_in_submit,
            report.max_rounds_in_submit,
            wall * 1e9 / report.samples.max(1) as f64,
        ));
        seeded.push(report.outcomes);
    }
    if seeded.len() < 2 {
        return;
    }
    let world = &rig.world;
    let emits: Vec<bool> = world
        .materials()
        .rows()
        .iter()
        .map(|m| m.emission.iter().any(|&e| e > 0.0))
        .collect();
    let emitter = |v: [i32; 3]| {
        world
            .material_at(v)
            .and_then(|m| emits.get(usize::from(m)).copied())
            .unwrap_or(false)
    };
    let l = Lighting::default();
    let arm = usize::from(clamp);
    let shown = |run: &[bake::FaceOutcome]| -> HashMap<([i32; 3], u8), f32> {
        run.iter()
            .filter(|o| !emitter(o.voxel))
            .map(|o| (key_of(o), d_of(o.mean[arm], &l)))
            .collect()
    };
    let (da, db) = (shown(&seeded[0]), shown(&seeded[1]));
    let mut all: Vec<f32> = da
        .iter()
        .filter_map(|(k, x)| db.get(k).map(|y| (x - y).abs() / 2f32.sqrt()))
        .collect();
    let mut plane = g_plane(&da);
    log.line(&format!(
        "stop {}: G_noise {:.2} / {:.2} ({}) G_plane {:.2} / {:.2}",
        view.name,
        q(&mut all, 0.5),
        q(&mut all, 0.9),
        all.len(),
        q(&mut plane, 0.5),
        q(&mut plane, 0.9),
    ));
}

// ---- The reference's readings (Phase 2) ----

/// The window's native size (the owner's 2530 x 1378).
const NATIVE: [u32; 2] = [2530, 1378];

/// A view's store file: the file's name, the piece's centre eye and its radius (`None`: the whole
/// world), as `live/session3.sh` bakes them.
fn file_of(view: &ReadView) -> (&'static str, [f32; 3], Option<i32>) {
    let vein = flythrough::VEIN_POSE.position;
    match view.name {
        "vein" | "back" => ("vein", vein, Some(2)),
        "patch" => ("patch", view.eye, Some(2)),
        _ => (
            if std::env::var("VXE_REF_HALL").is_ok_and(|v| v == "r2") {
                "hall-r2"
            } else {
                "hall"
            },
            view.eye,
            std::env::var("VXE_REF_HALL")
                .is_ok_and(|v| v == "r2")
                .then_some(2),
        ),
    }
}

/// The fly-through's `bake_extent`: the chunks within `radius` of the eye's chunk, in `world`.
fn piece_of(world: ChunkExtent, eye: [f32; 3], radius: Option<i32>) -> ChunkExtent {
    let Some(r) = radius else {
        return world;
    };
    let size = world.size();
    let c = eye.map(|v| (v / 64.0).floor() as i32);
    let lo = [0, 1, 2].map(|k| (c[k] - r).max(world.min.0[k]));
    let hi = [0, 1, 2].map(|k| (c[k] + r + 1).min(world.min.0[k] + size[k]));
    ChunkExtent::new(lo, hi)
}

/// The reading's world: the scene's, with the reference's glass row appended as the fly-through
/// does under `--reference`.
fn reference_world(scene: Scene) -> World {
    let glass = flythrough::castle_materials(&flythrough::cave_materials())
        .get(flythrough::castle::material(flythrough::castle::GLASS))
        .expect("the castle's glass")
        .clone();
    let mut table = match scene {
        Scene::Level => table(),
        Scene::Showcase => flythrough::showcase_materials(),
    };
    table.push(glass);
    match scene {
        Scene::Level => {
            let source = flythrough::cave::CaveSource::new(
                1,
                ChunkExtent::new([0, 0, 0], flythrough::cave::LEVEL_CHUNKS),
            );
            World::generate(source.extent, &source, &table)
        }
        Scene::Showcase => {
            let source = flythrough::showcase::ShowcaseSource::new(1);
            World::generate(source.extent(), &source, &table)
        }
    }
}

/// A rig on the RTX over `view`'s world at `size`, native and full-resolution lighting, no jitter,
/// lamp off, exposure 8, its store loaded from `view`'s file: the rig and the piece.
fn reference_rig(view: &ReadView, size: [u32; 2]) -> (ReadRig, ChunkExtent) {
    let world = reference_world(view.scene);
    let pool = flythrough::level_pool_units(&world, false) * UNIT_BYTES;
    let (device, queue, adapter) = reading_device(pool).expect("a device");
    let cfg = MirrorConfig {
        budget_bytes: 512 << 20,
        light_radius: if view.scene == Scene::Showcase { 19 } else { 4 },
        ..MirrorConfig::new(Window::Fixed(world.extent()))
    };
    let mut gpu = GpuWorld::with_config(&device, cfg);
    let (name, centre_eye, radius) = file_of(view);
    gpu.set_focus(vxe_world::VoxelCoord(view.eye.map(|v| v as i32)));
    for _ in 0..8 {
        gpu.sync_now(&world, &device, &queue)
            .expect("the mirror fits the adapter");
    }
    let mut frame = Frame::new(&device, size[0], size[1]);
    frame.set_jitter(&device, false);
    frame.set_resolution(&device, LightingResolution::Full);
    frame.set_lighting(reference_lighting(false, Light::Baked));
    frame.set_dt(1.0 / 60.0);
    let piece = piece_of(world.extent(), centre_eye, radius);
    let key = Frame::store_key(
        &gpu,
        world.extent(),
        1,
        piece,
        ChunkCoord(centre_eye.map(|v| (v / 64.0).floor() as i32)),
        radius.unwrap_or(-1),
        &BakeSettings::default(),
    );
    let path = format!("{}/../../live/bake/{name}.bin", env!("CARGO_MANIFEST_DIR"));
    let t = std::time::Instant::now();
    frame
        .load_store_file(
            &device,
            &queue,
            &mut gpu,
            &world,
            std::path::Path::new(&path),
            &key,
        )
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    println!("loaded {path} in {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
    (
        ReadRig {
            device,
            queue,
            gpu,
            world,
            frame,
            adapter,
        },
        piece,
    )
}

fn reference_lighting(lamp: bool, light: Light) -> Lighting {
    Lighting {
        lamp,
        lamp_bounce: false,
        exposure: Exposure::Manual(8.0),
        light,
        keep_store: true,
        ..Lighting::default()
    }
}

/// The cells of `frame`'s store by voxel and face, from its own copy.
fn store_cells(frame: &Frame, piece: ChunkExtent) -> HashMap<([i32; 3], u8), [u32; 2]> {
    let mut blocks = BTreeMap::new();
    let s = piece.size();
    for z in 0..s[2] {
        for y in 0..s[1] {
            for x in 0..s[0] {
                let c = [piece.min.0[0] + x, piece.min.0[1] + y, piece.min.0[2] + z];
                if let Some(b) = frame.store().block_words(ChunkCoord(c)) {
                    blocks.insert(c, b.to_vec());
                }
            }
        }
    }
    cells_of(&blocks)
}

fn inside(piece: ChunkExtent, v: [i32; 3]) -> bool {
    let s = piece.size();
    (0..3).all(|k| {
        let c = v[k].div_euclid(64);
        c >= piece.min.0[k] && c < piece.min.0[k] + s[k]
    })
}

/// `VXE_REF_PART`: `noise` (the bar: `G_noise` of each view's file against a second seed's stop
/// bake of the same faces), `strip` (the reference's frame against the table's frame 4096),
/// `yaw` (the rig's fast yaw), `flight` (30 and 120 voxels/s), `dig` (a 5-voxel dig, a dig at 100
/// voxels and four `B` presses at the vein), `glass` (a pane at the hall, 600 still frames lamp
/// off then on). `VXE_BAKE_VIEWS` names the views. Each part logs to `live/bake-count/`.
#[test]
#[ignore = "a reading on the RTX from live/bake/'s files: VXE_TEST_ADAPTER=rtx, under the perf lock"]
fn reference_readings() {
    let parts = std::env::var("VXE_REF_PART").unwrap_or_else(|_| "noise".to_owned());
    let names =
        std::env::var("VXE_BAKE_VIEWS").unwrap_or_else(|_| "vein,back,patch,hall".to_owned());
    for part in parts.split(',') {
        for view in read_views() {
            if !names.split(',').any(|n| n == view.name) {
                continue;
            }
            match part {
                "noise" => ref_noise(&view),
                "strip" => ref_strip(&view),
                "yaw" if view.name == "vein" => ref_yaw(&view),
                "flight" if view.name != "back" => ref_flight(&view),
                "dig" if view.name == "vein" => ref_dig(&view),
                "glass" if view.name == "hall" => ref_glass(&view),
                "yaw" | "flight" | "dig" | "glass" => {}
                _ => panic!("VXE_REF_PART takes noise, strip, yaw, flight, dig or glass"),
            }
        }
    }
}

fn emitter_test(world: &World) -> impl Fn([i32; 3]) -> bool + '_ {
    let emits: Vec<bool> = world
        .materials()
        .rows()
        .iter()
        .map(|m| m.emission.iter().any(|&e| e > 0.0))
        .collect();
    move |v| {
        world
            .material_at(v)
            .and_then(|m| emits.get(usize::from(m)).copied())
            .unwrap_or(false)
    }
}

fn ref_noise(view: &ReadView) {
    let mut log = Log::open("reference-noise.txt");
    let (mut rig, piece) = reference_rig(view, [16, 16]);
    let file = store_cells(&rig.frame, piece);
    let eye = view.eye;
    let near = within_96(eye);
    let filter: bake::FaceFilter = Arc::new(move |v, f| near(v, f) && inside(piece, v));
    let extent = extent_within(rig.world.extent(), eye, 96.0);
    let settings = BakeSettings {
        seed: 0x5EED_0001,
        keep_faces: true,
        filter: Some(filter),
        ..BakeSettings::default()
    };
    let mut second = Frame::new(&rig.device, 16, 16);
    second.set_lighting(reference_lighting(false, Light::Baked));
    let t = std::time::Instant::now();
    let report = second
        .bake_until(
            &rig.device,
            &rig.queue,
            &mut rig.gpu,
            &rig.world,
            extent,
            &settings,
            &mut |_| {},
        )
        .expect("the second seed bakes");
    let wall = t.elapsed().as_secs_f64();
    let emitter = emitter_test(&rig.world);
    let l = Lighting::default();
    let mut da = HashMap::new();
    let mut all = Vec::new();
    for o in report.outcomes.iter().filter(|o| !emitter(o.voxel)) {
        let Some(cell) = file.get(&key_of(o)) else {
            continue;
        };
        let a = d_of(bake::rgb9e5_unpack(cell[0]), &l);
        let b = d_of(o.mean[0], &l);
        da.insert(key_of(o), a);
        all.push((a - b).abs() / 2f32.sqrt());
    }
    let mut plane = g_plane(&da);
    let n = all.len();
    let (p50, p90) = (q(&mut all, 0.5), q(&mut all, 0.9));
    log.line(&format!(
        "noise {} (file {}) adapter {}: G_noise {p50:.2} / {p90:.2} ({n} faces) bar p50 0.5 p90 1: {}; G_plane {:.2} / {:.2}; second seed {} faces {:.4e} samples, at N_MIN {} at N_MAX {}, {wall:.1} s, ns a sample {:.2}",
        view.name,
        file_of(view).0,
        rig.adapter,
        if p50 <= 0.5 && p90 <= 1.0 { "met" } else { "MISSED" },
        q(&mut plane, 0.5),
        q(&mut plane, 0.9),
        report.faces,
        report.samples as f64,
        report.faces_at_min,
        report.faces_at_max,
        wall * 1e9 / report.samples.max(1) as f64
    ));
}

/// The display luma of every pixel of `rgba` and, by pixel, its hit's distance from `eye` and
/// whether the hit lies inside `piece` (`None` for no hit).
fn hits_of(rig: &ReadRig, eye: [f32; 3], piece: ChunkExtent) -> Vec<Option<(f32, bool)>> {
    let pos = readback::read_float(&rig.device, &rig.queue, rig.frame.hit_positions());
    let info = readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    (0..pos.len())
        .map(|i| {
            (info[i][3] & 1 != 0).then(|| {
                let p = [pos[i][0], pos[i][1], pos[i][2]];
                let d = (0..3).map(|k| (p[k] - eye[k]).powi(2)).sum::<f32>().sqrt();
                (d, inside(piece, p.map(|v| v.floor() as i32)))
            })
        })
        .collect()
}

fn luma(rgba: &[u8]) -> Vec<f32> {
    rgba.chunks(4)
        .map(|p| 0.2126 * f32::from(p[0]) + 0.7152 * f32::from(p[1]) + 0.0722 * f32::from(p[2]))
        .collect()
}

fn view_camera(view: &ReadView) -> Camera {
    match view.name {
        "vein" => flythrough::VEIN_POSE.camera(),
        "patch" => flythrough::PATCH_POSE.camera(),
        "back" => support::settle::KRun::Path.camera(&flythrough::VEIN_POSE, 0),
        _ => {
            let shot = &flythrough::showcase::SHOTS[0];
            Camera {
                position: shot.eye.0.map(|v| v as f32 + 0.5).into(),
                yaw: (shot.yaw_deg as f32).to_radians(),
                pitch: (shot.pitch_deg as f32).to_radians(),
                vfov: 70f32.to_radians(),
            }
        }
    }
}

fn ref_strip(view: &ReadView) {
    let mut log = Log::open("reference-strip.txt");
    let (mut rig, piece) = reference_rig(view, NATIVE);
    let camera = view_camera(view);
    let eye = camera.position.to_array();
    let light = LightParams::default();
    rig.frame
        .step(&rig.device, &rig.queue, &rig.gpu, &camera, &light);
    let reference = luma(&readback::read_rgba8(
        &rig.device,
        &rig.queue,
        rig.frame.target(),
    ));
    let missing = rig.frame.read_store_stats(&rig.device, &rig.queue).missing;
    let hits = hits_of(&rig, eye, piece);
    rig.frame
        .set_lighting(reference_lighting(false, Light::Table));
    rig.frame.reset_history();
    let t = std::time::Instant::now();
    for _ in 0..4096 {
        rig.frame
            .step(&rig.device, &rig.queue, &rig.gpu, &camera, &light);
    }
    let table = luma(&readback::read_rgba8(
        &rig.device,
        &rig.queue,
        rig.frame.target(),
    ));
    let (mut n, mut within, mut far, mut dark, mut outside) = (0usize, [0usize; 3], 0, 0, 0);
    for i in 0..reference.len() {
        let Some((d, ins)) = hits[i] else { continue };
        if !ins {
            outside += 1;
            continue;
        }
        n += 1;
        let diff = (reference[i] - table[i]).abs();
        for (k, lv) in [1.0, 2.0, 4.0].iter().enumerate() {
            within[k] += usize::from(diff <= *lv);
        }
        if d > 64.0 {
            far += 1;
            dark += usize::from(table[i] - reference[i] > 8.0);
        }
    }
    let share = |v: usize| v as f64 / n.max(1) as f64;
    log.line(&format!(
        "strip {} adapter {}: hits in the piece {n}, outside {outside} ({:.4} of hits), store_missing {missing}; reference against the table's frame 4096 ({:.0} s): within 1/2/4 {:.4} / {:.4} / {:.4}; beyond 64 voxels {far} hits, reference over 8 levels darker than the table {:.4}",
        view.name,
        rig.adapter,
        outside as f64 / (n + outside).max(1) as f64,
        t.elapsed().as_secs_f64(),
        share(within[0]),
        share(within[1]),
        share(within[2]),
        dark as f64 / far.max(1) as f64,
    ));
}

/// Frames of a yaw's read, as the settle rig's fast yaw.
const YAW_DOT_FRAMES: usize = 60;

fn ref_yaw(view: &ReadView) {
    let mut log = Log::open("reference-yaw.txt");
    let size = [192, 108];
    let (mut rig, _) = reference_rig(view, size);
    let pose = flythrough::VEIN_POSE;
    let still = pose.camera();
    let yawed = |deg: f32| Camera {
        yaw: still.yaw + deg.to_radians(),
        ..still
    };
    for (name, lamp, light) in [
        ("reference lamp off", false, Light::Baked),
        ("reference lamp on", true, Light::Baked),
        ("table native lamp on", true, Light::Table),
    ] {
        rig.frame.set_lighting(reference_lighting(lamp, light));
        rig.frame.reset_history();
        let lp = if lamp {
            LightParams::headlamp()
        } else {
            LightParams::default()
        };
        let step = |rig: &mut ReadRig, c: &Camera| {
            rig.frame.step(&rig.device, &rig.queue, &rig.gpu, c, &lp);
        };
        let dots = |rig: &ReadRig| -> f64 {
            let px = readback::read_rgba8(&rig.device, &rig.queue, rig.frame.target());
            support::aliasing::dots(&px, size[0], size[1])
                .iter()
                .filter(|d| **d)
                .count() as f64
        };
        for _ in 0..192 {
            step(&mut rig, &still);
        }
        let mut floor = Vec::new();
        for _ in 0..64 {
            step(&mut rig, &still);
            floor.push(dots(&rig));
        }
        for _ in 0..64 {
            step(&mut rig, &yawed(-90.0));
        }
        for i in 1..=10 {
            step(&mut rig, &yawed(-90.0 + 9.0 * i as f32));
        }
        let mean = floor.iter().sum::<f64>() / floor.len() as f64;
        let sd =
            (floor.iter().map(|f| (f - mean).powi(2)).sum::<f64>() / floor.len() as f64).sqrt();
        let mut excess = 0.0;
        for _ in 0..YAW_DOT_FRAMES {
            step(&mut rig, &still);
            excess += dots(&rig) - mean;
        }
        log.line(&format!(
            "yaw vein {name} adapter {} 192x108 scale 1 no upscaler: excess dots 1-60 {excess:+.1} (floor {mean:.1}, sd {sd:.1}, line {:.1})",
            rig.adapter,
            YAW_DOT_FRAMES as f64 * sd
        ));
    }
}

/// One timed frame: the passes' milliseconds by name.
fn timed_step(rig: &mut ReadRig, timer: &vxe_render::marcher::PassTimer, c: &Camera) -> Vec<f64> {
    rig.frame.advance();
    let params = rig.frame.params(&rig.gpu, c, &LightParams::default());
    let mut enc = rig
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    rig.frame.encode(
        &rig.device,
        &rig.queue,
        &mut enc,
        &rig.gpu,
        &params,
        Some(timer),
    );
    timer.resolve(&mut enc);
    rig.queue.submit([enc.finish()]);
    timer.read_all_ms(&rig.device)
}

fn pass_line(sum: &[f64], frames: usize) -> String {
    let total: f64 = sum.iter().sum::<f64>() / frames as f64;
    let mut parts: Vec<String> = vxe_render::marcher::FRAME_PASSES
        .iter()
        .zip(sum)
        .filter(|(_, ms)| **ms / frames as f64 >= 0.05)
        .map(|(n, ms)| format!("{n} {:.2}", ms / frames as f64))
        .collect();
    parts.insert(0, format!("total {total:.2} ms"));
    parts.join(", ")
}

fn ref_flight(view: &ReadView) {
    let mut log = Log::open("reference-flight.txt");
    let (mut rig, piece) = reference_rig(view, NATIVE);
    let timer = vxe_render::marcher::PassTimer::with_passes(
        &rig.device,
        &rig.queue,
        vxe_render::marcher::FRAME_PASSES.len() as u32,
    )
    .expect("timestamps on the RTX");
    let still = view_camera(view);
    let mut sum = vec![0.0; vxe_render::marcher::FRAME_PASSES.len()];
    for _ in 0..32 {
        timed_step(&mut rig, &timer, &still);
    }
    for _ in 0..120 {
        for (s, ms) in sum.iter_mut().zip(timed_step(&mut rig, &timer, &still)) {
            *s += ms;
        }
    }
    log.line(&format!(
        "frame {} at rest adapter {} {}x{}: {}",
        view.name,
        rig.adapter,
        NATIVE[0],
        NATIVE[1],
        pass_line(&sum, 120)
    ));
    // The rig's steady flight (`settle::flight_path`, 180 clear voxels facing the travel) at
    // the level's poses; the hall has no rig path, so it flies along its view.
    let (start, dir, len) = match view.name {
        "vein" | "patch" => {
            let pose = if view.name == "vein" {
                &flythrough::VEIN_POSE
            } else {
                &flythrough::PATCH_POSE
            };
            let (a, b) = support::settle::flight_path(pose);
            let d = [0, 1, 2].map(|k| b[k] - a[k]);
            let len = d.iter().map(|v| v * v).sum::<f32>().sqrt();
            (a, d.map(|v| v / len), len)
        }
        _ => (
            still.position.to_array(),
            [
                still.yaw.cos() * still.pitch.cos(),
                still.yaw.sin() * still.pitch.cos(),
                still.pitch.sin(),
            ],
            f32::INFINITY,
        ),
    };
    let cam0 = Camera {
        position: start.into(),
        yaw: dir[1].atan2(dir[0]),
        pitch: dir[2].asin(),
        ..still
    };
    for speed in [30.0f32, 120.0] {
        let mut sum = vec![0.0; vxe_render::marcher::FRAME_PASSES.len()];
        let (mut frames, mut missing, mut most) = (0usize, 0u64, 0u32);
        let mut cam = cam0;
        rig.frame.reset_history();
        for f in 0..600 {
            let s = speed * f as f32 / 60.0;
            if s > len {
                break;
            }
            let p = [0, 1, 2].map(|k| start[k] + dir[k] * s);
            let v = p.map(|v| v.floor() as i32);
            if !inside(piece, v) {
                // Frames before the path enters the piece are skipped; leaving it ends the run.
                if frames > 0 {
                    break;
                }
                continue;
            }
            if rig
                .world
                .material_at(vxe_world::VoxelCoord(v))
                .is_some_and(|m| m != AIR)
            {
                break;
            }
            cam.position = p.into();
            for (s, ms) in sum.iter_mut().zip(timed_step(&mut rig, &timer, &cam)) {
                *s += ms;
            }
            let m = rig.frame.read_store_stats(&rig.device, &rig.queue).missing;
            missing += u64::from(m);
            most = most.max(m);
            frames += 1;
        }
        log.line(&format!(
            "flight {} {speed} voxels/s adapter {}: {frames} frames inside the piece before the edge or rock ({:.1} voxels); store_missing a frame mean {:.0} max {most} of {} pixels; {}",
            view.name,
            rig.adapter,
            frames as f32 * speed / 60.0,
            missing as f64 / frames.max(1) as f64,
            NATIVE[0] * NATIVE[1],
            pass_line(&sum, frames.max(1))
        ));
    }
}

/// The hit under pixel `(x, y)` of the last frame.
fn hit_at(rig: &ReadRig, size: [u32; 2], x: u32, y: u32) -> Option<[f32; 3]> {
    let pos = readback::read_float(&rig.device, &rig.queue, rig.frame.hit_positions());
    let info = readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    let i = (y * size[0] + x) as usize;
    (info[i][3] & 1 != 0).then(|| [pos[i][0], pos[i][1], pos[i][2]])
}

/// Applies `ops`, mirrors them and times the re-bake of their boxes: the line.
fn timed_dig(rig: &mut ReadRig, ops: &[EditOp], what: &str) -> String {
    let settings = BakeSettings {
        n_max: bake::DIG_N_MAX,
        ..BakeSettings::default()
    };
    timed_dig_with(rig, ops, what, &settings)
}

/// [`timed_dig`] re-baking to `settings`.
fn timed_dig_with(
    rig: &mut ReadRig,
    ops: &[EditOp],
    what: &str,
    settings: &BakeSettings,
) -> String {
    let mut changed = 0u64;
    let mut boxes = 0;
    for op in ops {
        let r = rig.world.apply(op).expect("the carve applies");
        changed += r.changed_from.iter().map(|c| u64::from(c.1)).sum::<u64>();
        if let Some((min, max)) = r.aabb {
            rig.frame.invalidate(min.0, max.0);
            boxes += 1;
        }
    }
    rig.gpu
        .sync_now(&rig.world, &rig.device, &rig.queue)
        .expect("the mirror fits");
    let t = std::time::Instant::now();
    let r = rig
        .frame
        .rebake_digs(
            &rig.device,
            &rig.queue,
            &mut rig.gpu,
            &rig.world,
            settings,
            &mut |_| {},
        )
        .map(|r| r.expect("the ring fits"))
        .unwrap_or_default();
    let ms = t.elapsed().as_secs_f64() * 1e3;
    format!(
        "{what}: {} ops, {boxes} boxes, {changed} voxels; ring {} bricks, {} chunks, {} faces, {:.4e} samples, at N_MIN {} at DIG_N_MAX {}, {ms:.0} ms ({:.2} min), ns a sample {:.2}",
        ops.len(),
        r.bricks,
        r.bake.chunks,
        r.bake.faces,
        r.bake.samples as f64,
        r.bake.faces_at_min,
        r.bake.faces_at_max,
        ms / 60e3,
        ms * 1e6 / r.bake.samples.max(1) as f64
    )
}

fn ref_dig(view: &ReadView) {
    let mut log = Log::open("reference-dig.txt");
    let size = [192, 108];
    let (mut rig, _) = reference_rig(view, size);
    let camera = flythrough::VEIN_POSE.camera();
    rig.frame.step(
        &rig.device,
        &rig.queue,
        &rig.gpu,
        &camera,
        &LightParams::default(),
    );
    let eye = camera.position.to_array();
    let dist = |e: [f32; 3], p: [f32; 3]| (0..3).map(|k| (p[k] - e[k]).powi(2)).sum::<f32>().sqrt();
    let carve = |p: [f32; 3]| {
        let c = p.map(|v| i64::from(v.floor() as i32) * FIXED_ONE + FIXED_ONE / 2);
        EditOp::sphere(Fixed3(c), 5 * FIXED_ONE, Mode::Carve)
    };
    let near = hit_at(&rig, size, size[0] / 2, size[1] / 2).expect("the view's centre hits rock");
    let line = timed_dig(
        &mut rig,
        &[carve(near)],
        &format!(
            "5-voxel dig at arm's length ({:.1} voxels)",
            dist(eye, near)
        ),
    );
    log.line(&format!("dig vein adapter {}: {line}", rig.adapter));
    // The settle rig's far dig (`settle::dig_camera` for `DigFar`): from the steady flight's
    // start facing its travel, the hit in the view's central half nearest 100 voxels.
    let down = support::settle::KRun::Path.camera(&flythrough::VEIN_POSE, 0);
    rig.frame.step(
        &rig.device,
        &rig.queue,
        &rig.gpu,
        &down,
        &LightParams::default(),
    );
    let from = down.position.to_array();
    let pos = readback::read_float(&rig.device, &rig.queue, rig.frame.hit_positions());
    let info = readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    let far = (size[1] / 4..3 * size[1] / 4)
        .flat_map(|y| (size[0] / 4..3 * size[0] / 4).map(move |x| (y * size[0] + x) as usize))
        .filter(|&i| info[i][3] & 1 != 0)
        .map(|i| [pos[i][0], pos[i][1], pos[i][2]])
        .min_by(|a, b| {
            let d = |p: &[f32; 3]| (dist(from, *p) - 100.0).abs();
            d(a).total_cmp(&d(b))
        })
        .expect("a hit near 100 voxels down the passage");
    let line = timed_dig(
        &mut rig,
        &[carve(far)],
        &format!(
            "5-voxel dig at 100 voxels, from the flight's start ({:.1} voxels)",
            dist(from, far)
        ),
    );
    log.line(&format!("dig vein adapter {}: {line}", rig.adapter));
    let origin = eye.map(|v| v.floor() as i64);
    for k in 0..4u64 {
        let seed = 1u64 ^ k.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let ops = flythrough_logic::bench_ops(origin, seed);
        let line = timed_dig(&mut rig, &ops, &format!("B press {}", k + 1));
        log.line(&format!("dig vein adapter {}: {line}", rig.adapter));
    }
}

fn ref_glass(view: &ReadView) {
    let mut log = Log::open("reference-glass.txt");
    let size = NATIVE;
    let (mut rig, piece) = reference_rig(view, size);
    let camera = view_camera(view);
    let glass = (rig.world.materials().len() - 1) as vxe_world::MaterialId;
    // The fly-through's pane: 40 x 40, 2 thick, 6 voxels ahead along the axis faced most.
    let e = camera.position.to_array().map(|v| v.floor() as i32);
    let (fwd, _, _) = camera.basis();
    let k = if fwd.x.abs() >= fwd.y.abs() { 0 } else { 1 };
    let ahead = if fwd.to_array()[k] >= 0.0 { 1 } else { -1 };
    let near = e[k] + ahead * 6;
    let mut min = [0i32; 3];
    let mut max = [0i32; 3];
    (min[k], max[k]) = if ahead > 0 {
        (near, near + 2)
    } else {
        (near - 1, near + 1)
    };
    for a in [1 - k, 2] {
        min[a] = e[a] - 20;
        max[a] = e[a] + 20;
    }
    assert!(inside(piece, min) && inside(piece, max.map(|v| v - 1)));
    let light = LightParams::default();
    rig.frame
        .step(&rig.device, &rig.queue, &rig.gpu, &camera, &light);
    let before = readback::read_rgba8(&rig.device, &rig.queue, rig.frame.target());
    let op = EditOp::box_(
        Fixed3(min.map(|v| i64::from(v) * FIXED_ONE)),
        Fixed3(max.map(|v| i64::from(v) * FIXED_ONE)),
        Mode::Fill(glass),
    );
    let dig = timed_dig(&mut rig, &[op], "pane");
    log.line(&format!("glass hall adapter {}: {dig}", rig.adapter));
    for (name, lamp, lp) in [
        ("lamp off", false, LightParams::default()),
        ("lamp on", true, LightParams::headlamp()),
    ] {
        rig.frame
            .set_lighting(reference_lighting(lamp, Light::Baked));
        let mut frames = Vec::new();
        let mut first_accum = None;
        for f in 0..600 {
            rig.frame
                .step(&rig.device, &rig.queue, &rig.gpu, &camera, &lp);
            if f % 30 == 0 || f == 599 {
                frames.push(readback::read_rgba8(
                    &rig.device,
                    &rig.queue,
                    rig.frame.target(),
                ));
            }
            if f == 0 {
                first_accum = Some(readback::read_float(
                    &rig.device,
                    &rig.queue,
                    rig.frame.accumulated(),
                ));
            }
        }
        let last_accum = readback::read_float(&rig.device, &rig.queue, rig.frame.accumulated());
        let first_accum = first_accum.expect("frame 0");
        // The pane's pixels: those the pane changed in the first frame after it.
        let mask: Vec<bool> = before
            .chunks(4)
            .zip(frames[0].chunks(4))
            .map(|(a, b)| a != b)
            .collect();
        let changed = first_accum
            .iter()
            .zip(&last_accum)
            .zip(&mask)
            .filter(|((a, b), m)| **m && a != b)
            .count();
        // Patches a step: 8 x 8 tiles over the pane holding a pixel that moved between two
        // reads 30 frames apart, the mean over the steps.
        let tiles = |a: &[u8], b: &[u8]| -> usize {
            let mut moved = std::collections::HashSet::new();
            for (i, m) in mask.iter().enumerate() {
                if *m && a[4 * i..4 * i + 3] != b[4 * i..4 * i + 3] {
                    let (x, y) = (i as u32 % size[0], i as u32 / size[0]);
                    moved.insert((x / 8, y / 8));
                }
            }
            moved.len()
        };
        let steps: Vec<usize> = frames
            .windows(2)
            .skip(1)
            .map(|w| tiles(&w[0], &w[1]))
            .collect();
        let mean = steps.iter().sum::<usize>() as f64 / steps.len().max(1) as f64;
        log.line(&format!(
            "glass hall {name} adapter {}: pane pixels {}, accumulation pixels over the pane changed from the first to the last of 600 frames {changed}; composite patches a step (8x8 tiles, 30 frames apart, frames 30 to 599) mean {mean:.2} max {}",
            rig.adapter,
            mask.iter().filter(|m| **m).count(),
            steps.iter().max().copied().unwrap_or(0)
        ));
    }
}

// Plan 0059 Phase 4: the samples cut, the shared seeds and the filter, read against the
// reference's files.

/// A face: its voxel and side.
type FaceKey = ([i32; 3], u8);

/// The cut's grain counts and the deepest of them, every run's length.
const CUT_COUNTS: [u32; 5] = [32, 128, 512, 1024, 2048];

/// `VXE_CUT_PART`: `grain` (every seed form at `CUT_COUNTS`, two seed sets, the filter on and off,
/// against the view's file; the vein's back view, the patch and the hall), `cost` (ns a sample in
/// one dispatch of 1 000 to 1 000 000 samples at the vein and the hall, on the adapter
/// `VXE_TEST_ADAPTER` names), `cpu` (a sampler over `World::raycast` on one thread at the vein),
/// `glass` (the pane's bright pixels at the hall), `parts` (the composite's parts at the pane,
/// at the hall's shot and the owner's eye, under the per-layer composite: historical),
/// `filter` (ADR-0047's filter at the hall's shot, [`cut_glass_filter`]), `rings` (the reach
/// cut's stores against the reference, [`cut_rings`]). `VXE_BAKE_VIEWS` names the views. Each part
/// logs to `live/bake-count/cut-<part>.txt`.
#[test]
#[ignore = "a reading on the RTX (and the Radeon for cost) from live/bake/'s files, under the perf lock"]
fn cut_readings() {
    let parts = std::env::var("VXE_CUT_PART").unwrap_or_else(|_| "grain".to_owned());
    let names =
        std::env::var("VXE_BAKE_VIEWS").unwrap_or_else(|_| "vein,back,patch,hall".to_owned());
    for part in parts.split(',') {
        for view in read_views() {
            if !names.split(',').any(|n| n == view.name) {
                continue;
            }
            match part {
                "grain" if view.name != "vein" => cut_grain(&view),
                "cost" if view.name == "vein" || view.name == "hall" => cut_cost(&view),
                "cpu" if view.name == "vein" => cut_cpu(&view),
                "glass" if view.name == "hall" => cut_glass(&view),
                "parts" if view.name == "hall" => cut_glass_parts(&view),
                "filter" if view.name == "hall" => cut_glass_filter(&view),
                "rings" if view.name != "back" => cut_rings(&view),
                "grain" | "cost" | "cpu" | "glass" | "parts" | "filter" | "rings" => {}
                _ => panic!("VXE_CUT_PART takes grain, cost, cpu, glass, parts, filter or rings"),
            }
        }
    }
}

/// The fly-through's `--cut` arms Phases 6 and 7a offer: `independent:512`, `rings:12/4/2/1` and
/// `bricks:12/CELL/SAMPLES`, by their file names and settings around `centre`.
fn ring_cuts(centre: [i32; 3]) -> Vec<(&'static str, BakeSettings)> {
    let cut = |rings: Option<bake::Rings>| BakeSettings {
        round: 512,
        n_min: 512,
        n_max: 512,
        tent: true,
        rings,
        ..BakeSettings::default()
    };
    let rings = |faces: Vec<(i32, u32)>, cell: u32, brick_samples: u32| {
        Some(bake::Rings {
            centre,
            brick: 12,
            faces,
            cell,
            brick_samples,
        })
    };
    vec![
        ("reference again", BakeSettings::default()),
        ("independent-512", cut(None)),
        (
            "rings-12-4-2-1",
            cut(rings(vec![(1, 512), (2, 128), (4, 32)], 8, 64)),
        ),
        ("bricks-8-64", cut(rings(Vec::new(), 8, 64))),
        ("bricks-8-512", cut(rings(Vec::new(), 8, 512))),
        ("bricks-8-4096", cut(rings(Vec::new(), 8, 4096))),
        ("bricks-4-64", cut(rings(Vec::new(), 4, 64))),
        ("bricks-4-512", cut(rings(Vec::new(), 4, 512))),
        ("bricks-4-4096", cut(rings(Vec::new(), 4, 4096))),
    ]
}

/// The reach cut against the reference at `view` (Plan 0059 Phase 6's strip): each cut's file
/// loaded in turn and one frame read, against the reference's frame, over the hits inside the
/// reference's piece: the share within 1, 2 and 4 levels by the hit chunk's ring (Chebyshev
/// distance from the piece's centre chunk), the share over 8 levels darker, the store's
/// pixels at the brick level and with no value, and the store's bytes. Each frame is read with
/// no history; the reference read again is the floor.
fn cut_rings(view: &ReadView) {
    let mut log = Log::open("cut-rings.txt");
    let (mut rig, piece) = reference_rig(view, NATIVE);
    let (name, centre_eye, radius) = file_of(view);
    let centre = centre_eye.map(|v| (v / 64.0).floor() as i32);
    let camera = view_camera(view);
    let eye = camera.position.to_array();
    let light = LightParams::default();
    rig.frame.reset_history();
    rig.frame
        .step(&rig.device, &rig.queue, &rig.gpu, &camera, &light);
    let reference = luma(&readback::read_rgba8(
        &rig.device,
        &rig.queue,
        rig.frame.target(),
    ));
    let pos = readback::read_float(&rig.device, &rig.queue, rig.frame.hit_positions());
    let hits = hits_of(&rig, eye, piece);
    let ring_of = |i: usize| {
        let c = [0, 1, 2].map(|k| (pos[i][k].floor() as i32).div_euclid(64));
        (0..3).map(|k| (c[k] - centre[k]).abs()).max().unwrap_or(0)
    };
    let ref_stats = rig.frame.read_store_stats(&rig.device, &rig.queue);
    log.line(&format!(
        "rings {} adapter {}: reference {} chunks {} B, store_missing {}",
        view.name, rig.adapter, ref_stats.chunks, ref_stats.bytes, ref_stats.missing
    ));
    let world = rig.world.extent();
    for (cut, settings) in ring_cuts(centre) {
        let extent = settings.rings.as_ref().map_or(piece, |r| r.extent(world));
        let key = Frame::store_key(
            &rig.gpu,
            world,
            1,
            extent,
            ChunkCoord(centre),
            radius.unwrap_or(-1),
            &settings,
        );
        let file = if cut == "reference again" {
            format!("{name}.bin")
        } else {
            format!("{name}.{cut}.bin")
        };
        let load_ms = load_cut(&mut rig, &file, &key);
        rig.frame.reset_history();
        rig.frame
            .step(&rig.device, &rig.queue, &rig.gpu, &camera, &light);
        let frame = luma(&readback::read_rgba8(
            &rig.device,
            &rig.queue,
            rig.frame.target(),
        ));
        let stats = rig.frame.read_store_stats(&rig.device, &rig.queue);
        // By ring 0, 1, 2 and beyond, and all: hits, within 1/2/4, over 8 darker.
        let mut rows = [[0usize; 5]; 5];
        for i in 0..frame.len() {
            let Some((_, true)) = hits[i] else { continue };
            let diff = frame[i] - reference[i];
            let ring = ring_of(i).min(3) as usize;
            for row in [ring, 4] {
                rows[row][0] += 1;
                for (k, lv) in [1.0, 2.0, 4.0].iter().enumerate() {
                    rows[row][1 + k] += usize::from(diff.abs() <= *lv);
                }
                rows[row][4] += usize::from(diff < -8.0);
            }
        }
        let row = |r: &[usize; 5]| {
            let n = r[0].max(1) as f64;
            format!(
                "{} hits, within 1/2/4 {:.4} / {:.4} / {:.4}, darker over 8 {:.4}",
                r[0],
                r[1] as f64 / n,
                r[2] as f64 / n,
                r[3] as f64 / n,
                r[4] as f64 / n
            )
        };
        let pixels = frame.len() as f64;
        log.line(&format!(
            "  {cut}: loaded in {load_ms:.0} ms, {} chunks {} B ({:.2} of the reference's); pixels at the brick level {:.4}, with no value {:.4}; in the piece: all {}; ring 0 {}; ring 1 {}; ring 2 {}; ring 3 on {}",
            stats.chunks,
            stats.bytes,
            stats.bytes as f64 / ref_stats.bytes.max(1) as f64,
            f64::from(stats.brick_reads) / pixels,
            f64::from(stats.missing) / pixels,
            row(&rows[4]),
            row(&rows[0]),
            row(&rows[1]),
            row(&rows[2]),
            row(&rows[3]),
        ));
    }
    // A dig on each ring store, re-baked to its own settings: a 5-voxel carve at the view's
    // centre (a face ring) and one 6 chunks along x from the centre chunk (the brick level). The
    // world keeps the first store's digs when the second loads.
    let carve = |v: [i32; 3]| {
        let c = v.map(|x| i64::from(x) * FIXED_ONE + FIXED_ONE / 2);
        EditOp::sphere(Fixed3(c), 5 * FIXED_ONE, Mode::Carve)
    };
    for (cut, settings) in ring_cuts(centre) {
        let Some(rings) = &settings.rings else {
            continue;
        };
        let extent = rings.extent(world);
        let key = Frame::store_key(
            &rig.gpu,
            world,
            1,
            extent,
            ChunkCoord(centre),
            radius.unwrap_or(-1),
            &settings,
        );
        load_cut(&mut rig, &format!("{name}.{cut}.bin"), &key);
        rig.frame
            .step(&rig.device, &rig.queue, &rig.gpu, &camera, &light);
        let near = hit_at(&rig, NATIVE, NATIVE[0] / 2, NATIVE[1] / 2)
            .expect("the view's centre hits rock")
            .map(|v| v.floor() as i32);
        let far = [0, 1, 2].map(|k| (centre[k] + [6, 0, 0][k]) * 64 + 32);
        let line = timed_dig_with(
            &mut rig,
            &[carve(near), carve(far)],
            &format!("{cut}: digs at {near:?} and {far:?}"),
            &settings,
        );
        rig.frame.reset_history();
        rig.frame
            .step(&rig.device, &rig.queue, &rig.gpu, &camera, &light);
        let stats = rig.frame.read_store_stats(&rig.device, &rig.queue);
        let pixels = f64::from(NATIVE[0] * NATIVE[1]);
        log.line(&format!(
            "  {line}; then {} chunks {} B, pixels at the brick level {:.4}, with no value {:.4}",
            stats.chunks,
            stats.bytes,
            f64::from(stats.brick_reads) / pixels,
            f64::from(stats.missing) / pixels,
        ));
    }
}

/// Loads `live/bake/<file>` into `rig`'s store under `key`; its milliseconds.
fn load_cut(rig: &mut ReadRig, file: &str, key: &bake::StoreKey) -> f64 {
    let path = format!("{}/../../live/bake/{file}", env!("CARGO_MANIFEST_DIR"));
    let t = std::time::Instant::now();
    rig.frame
        .load_store_file(
            &rig.device,
            &rig.queue,
            &mut rig.gpu,
            &rig.world,
            std::path::Path::new(&path),
            key,
        )
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    t.elapsed().as_secs_f64() * 1e3
}

/// Faces within `r` voxels of `eye`, by voxel centre.
fn within(eye: [f32; 3], r: f32) -> impl Fn([i32; 3]) -> bool + Send + Sync + Clone {
    move |v: [i32; 3]| {
        (0..3)
            .map(|k| (v[k] as f32 + 0.5 - eye[k]).powi(2))
            .sum::<f32>()
            <= r * r
    }
}

/// The value of `f`'s face at `n` samples (the unclamped mean), by key.
fn means_at(faces: &[bake::FaceOutcome], n: u32) -> HashMap<([i32; 3], u8), [f32; 3]> {
    faces
        .iter()
        .filter_map(|o| {
            let s = o.snapshots.iter().find(|s| s.0 == n)?;
            Some((key_of(o), s.1))
        })
        .collect()
}

/// `m` through the bake's filter, over `m`'s own faces.
fn tented(m: &HashMap<([i32; 3], u8), [f32; 3]>) -> HashMap<([i32; 3], u8), [f32; 3]> {
    let get = |v: [i32; 3], f: u8| m.get(&(v, f)).copied();
    m.keys()
        .filter_map(|&(v, f)| Some(((v, f), bake::tent(&get, v, f)?)))
        .collect()
}

/// `G_patch`: over 8 x 8 coplanar patches (same side and plane, the tangents' coordinates over
/// 8), holding at least 32 faces in both sets, `|mean_A - mean_B| / sqrt 2`.
fn g_patch(a: &HashMap<([i32; 3], u8), f32>, b: &HashMap<([i32; 3], u8), f32>) -> Vec<f32> {
    type Patch = (u8, i32, i32, i32);
    let mut patches: HashMap<Patch, (f64, f64, u32)> = HashMap::new();
    for (&(v, side), &da) in a {
        let Some(&db) = b.get(&(v, side)) else {
            continue;
        };
        let axis = usize::from(side / 2);
        let (u, w) = ((axis + 1) % 3, (axis + 2) % 3);
        let e = patches
            .entry((side, v[axis], v[u].div_euclid(8), v[w].div_euclid(8)))
            .or_default();
        e.0 += f64::from(da);
        e.1 += f64::from(db);
        e.2 += 1;
    }
    patches
        .values()
        .filter(|p| p.2 >= 32)
        .map(|p| ((p.0 - p.1).abs() / f64::from(p.2) / 2f64.sqrt()) as f32)
        .collect()
}

/// A row of 7 coplanar faces across the reference's sharpest flat shadow edge in `r` (a lit face
/// over 20 levels beside one at most 1, three coplanar faces on each side, each lit face at least
/// 0.8 of the edge's, each dark face at most 1 level), and the crease pair (a face and the
/// concave face across from it) whose reference differs most, as rows of 3 faces each side.
#[allow(clippy::type_complexity)]
fn harm_rows(
    r: &HashMap<([i32; 3], u8), f32>,
) -> (Option<Vec<([i32; 3], u8)>>, Option<Vec<([i32; 3], u8)>>) {
    let mut edge: Option<(f32, Vec<([i32; 3], u8)>)> = None;
    let mut crease: Option<(f32, Vec<([i32; 3], u8)>)> = None;
    let mut keys: Vec<_> = r.keys().copied().collect();
    keys.sort_unstable();
    for &(v, side) in &keys {
        let axis = usize::from(side / 2);
        let air = if side % 2 == 1 { 1 } else { -1 };
        for t in [(axis + 1) % 3, (axis + 2) % 3] {
            for dir in [1, -1] {
                let at = |k: i32| {
                    let mut c = v;
                    c[t] += dir * k;
                    (c, side)
                };
                let row: Vec<_> = (-2..=4).map(at).collect();
                if row.iter().all(|k| r.contains_key(k)) {
                    let lit = r[&at(0)];
                    let ok = lit > 20.0
                        && (-2..=0).all(|k| r[&at(k)] >= 0.8 * lit)
                        && (1..=4).all(|k| r[&at(k)] <= 1.0);
                    if ok && edge.as_ref().is_none_or(|e| lit > e.0) {
                        edge = Some((lit, row));
                    }
                }
                // The concave face across: the voxel a step along the tangent and a step toward
                // the air, its face turned back along the tangent.
                let mut w = v;
                w[t] += dir;
                w[axis] += air;
                let wside = (2 * t) as u8 + u8::from(dir < 0);
                let floor: Vec<_> = (0..3)
                    .rev()
                    .map(|k| {
                        let mut c = v;
                        c[t] -= dir * k;
                        (c, side)
                    })
                    .collect();
                let wall: Vec<_> = (0..3)
                    .map(|k| {
                        let mut c = w;
                        c[axis] += air * k;
                        (c, wside)
                    })
                    .collect();
                if floor.iter().chain(&wall).all(|k| r.contains_key(k)) {
                    let d = (r[&(v, side)] - r[&(w, wside)]).abs();
                    if crease.as_ref().is_none_or(|c| d > c.0) {
                        crease = Some((d, floor.into_iter().chain(wall).collect()));
                    }
                }
            }
        }
    }
    (edge.map(|e| e.1), crease.map(|c| c.1))
}

fn cut_grain(view: &ReadView) {
    let mut log = Log::open("cut-grain.txt");
    let (mut rig, piece) = reference_rig(view, [16, 16]);
    let file = store_cells(&rig.frame, piece);
    let eye = view.eye;
    let near98 = within(eye, 98.0);
    let near96 = within(eye, 96.0);
    let filter: bake::FaceFilter = Arc::new(move |v, _| near98(v) && inside(piece, v));
    let extent = extent_within(rig.world.extent(), eye, 98.0);
    let emitter = emitter_test(&rig.world);
    let l = Lighting::default();
    let reference: HashMap<([i32; 3], u8), f32> = file
        .iter()
        .filter(|((v, _), _)| near96(*v) && inside(piece, *v) && !emitter(*v))
        .map(|(k, c)| (*k, d_of(bake::rgb9e5_unpack(c[0]), &l)))
        .collect();
    let (edge, crease) = harm_rows(&reference);
    let mut cut = Frame::new(&rig.device, 16, 16);
    cut.set_lighting(reference_lighting(false, Light::Baked));
    for form in bake::SeedForm::ALL {
        let mut runs = Vec::new();
        for seed in [0u32, 0x5EED_0001] {
            let settings = BakeSettings {
                round: 32,
                n_min: 2048,
                n_max: 2048,
                form,
                seed,
                keep_faces: true,
                snapshots: CUT_COUNTS.to_vec(),
                filter: Some(filter.clone()),
                ..BakeSettings::default()
            };
            let report = cut
                .bake_until(
                    &rig.device,
                    &rig.queue,
                    &mut rig.gpu,
                    &rig.world,
                    extent,
                    &settings,
                    &mut |_| {},
                )
                .expect("the cut bakes");
            log.line(&format!(
                "grain {} {} seed {seed:#x} adapter {}: {} faces, {:.3e} samples, submit ns a sample {:.2}",
                view.name,
                form.name(),
                rig.adapter,
                report.faces,
                report.samples as f64,
                report.gpu_seconds * 1e9 / report.samples.max(1) as f64
            ));
            runs.push(report.outcomes);
        }
        for n in CUT_COUNTS {
            let raw = [means_at(&runs[0], n), means_at(&runs[1], n)];
            for tent in [false, true] {
                let m: Vec<HashMap<FaceKey, [f32; 3]>> = if tent {
                    raw.iter().map(tented).collect()
                } else {
                    raw.to_vec()
                };
                let d: Vec<HashMap<([i32; 3], u8), f32>> = m
                    .iter()
                    .map(|m| {
                        m.iter()
                            .filter(|(k, _)| reference.contains_key(k))
                            .map(|(k, c)| (*k, d_of(*c, &l)))
                            .collect()
                    })
                    .collect();
                let mut local = g_local(&d[0], &d[1], false);
                let mut seam = g_local(&d[0], &d[1], true);
                let mut patch = g_patch(&d[0], &d[1]);
                let mut noise: Vec<f32> = d[0]
                    .iter()
                    .filter_map(|(k, a)| Some((a - d[1].get(k)?).abs() / 2f32.sqrt()))
                    .collect();
                // The dark share and the bias by decile against the reference, seed set A.
                let lit: Vec<_> = reference.iter().filter(|(_, r)| **r > 1.0).collect();
                let dark = lit
                    .iter()
                    .filter(|(k, _)| m[0].get(k).is_some_and(|c| *c == [0.0; 3]))
                    .count();
                let mut by: Vec<(f32, f32)> = reference
                    .iter()
                    .filter_map(|(k, r)| Some((*r, d[0].get(k)? - r)))
                    .collect();
                by.sort_by(|a, b| a.0.total_cmp(&b.0));
                let deciles: Vec<String> = (0..10)
                    .map(|i| {
                        let s = &by[i * by.len() / 10..(i + 1) * by.len() / 10];
                        let mean = s.iter().map(|x| x.1).sum::<f32>() / s.len().max(1) as f32;
                        let sd = (s.iter().map(|x| (x.1 - mean).powi(2)).sum::<f32>()
                            / s.len().max(1) as f32)
                            .sqrt();
                        format!("{mean:+.2}/{sd:.2}")
                    })
                    .collect();
                log.line(&format!(
                    "grain {} {} N {n} filter {}: G_local {:.2} / {:.2} ({} pairs); G_seam {:.2} / {:.2} ({} pairs); G_patch p90 {:.2} ({} patches); G_noise {:.2} / {:.2}; dark {:.4} ({dark} of {}); bias by decile mean/sd {}",
                    view.name,
                    form.name(),
                    if tent { "on" } else { "off" },
                    q(&mut local, 0.5),
                    q(&mut local, 0.9),
                    local.len(),
                    q(&mut seam, 0.5),
                    q(&mut seam, 0.9),
                    seam.len(),
                    q(&mut patch, 0.9),
                    patch.len(),
                    q(&mut noise, 0.5),
                    q(&mut noise, 0.9),
                    dark as f64 / lit.len().max(1) as f64,
                    lit.len(),
                    deciles.join(" ")
                ));
                if n == 512 && !tent {
                    let f = tented(&raw[0]);
                    for (name, row) in [("shadow edge", &edge), ("crease", &crease)] {
                        let Some(row) = row else {
                            log.line(&format!("harm {} {name}: none found", view.name));
                            continue;
                        };
                        let vals = |m: &dyn Fn(&FaceKey) -> Option<f32>| -> String {
                            row.iter()
                                .map(|k| m(k).map_or("-".to_owned(), |x| format!("{x:.1}")))
                                .collect::<Vec<_>>()
                                .join(" ")
                        };
                        log.line(&format!(
                            "harm {} {name} {} N 512 at {:?}: reference [{}] cut [{}] filtered [{}]",
                            view.name,
                            form.name(),
                            row[0],
                            vals(&|k| reference.get(k).copied()),
                            vals(&|k| raw[0].get(k).map(|c| d_of(*c, &l))),
                            vals(&|k| f.get(k).map(|c| d_of(*c, &l))),
                        ));
                    }
                }
            }
        }
    }
}

/// The faces within 96 voxels of `eye`, in a hashed order: a ring's faces without its order.
fn shuffled_faces(world: &World, eye: [f32; 3]) -> Vec<([i32; 3], u8)> {
    let classes = bake::face_classes(world.materials());
    let near = within(eye, 96.0);
    let e = extent_within(world.extent(), eye, 96.0);
    let s = e.size();
    let mut out = Vec::new();
    for z in 0..s[2] {
        for y in 0..s[1] {
            for x in 0..s[0] {
                let c = [e.min.0[0] + x, e.min.0[1] + y, e.min.0[2] + z];
                let Some(f) = bake::list_world_chunk(world, ChunkCoord(c), &classes) else {
                    continue;
                };
                for (slot, keys) in &f.bricks {
                    let b = vxe_world::brick_of_slot(usize::from(*slot));
                    for &key in keys {
                        let i = i32::from(key >> 3);
                        let l = [i % 8, i / 8 % 8, i / 64];
                        let v = [0, 1, 2].map(|k| c[k] * 64 + b[k] * 8 + l[k]);
                        if near(v) {
                            out.push((v, (key & 7) as u8));
                        }
                    }
                }
            }
        }
    }
    let h = |k: &([i32; 3], u8)| {
        let mut x = (k.0[0] as u64) ^ ((k.0[1] as u64) << 21) ^ ((k.0[2] as u64) << 42);
        x ^= u64::from(k.1) << 61;
        x = x.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        x ^ (x >> 29)
    };
    out.sort_unstable_by_key(h);
    out
}

fn cut_cost(view: &ReadView) {
    let mut log = Log::open("cut-cost.txt");
    let (mut rig, _) = reference_rig(view, [16, 16]);
    let timer = vxe_render::marcher::PassTimer::new(&rig.device, &rig.queue).map(Arc::new);
    let faces = shuffled_faces(&rig.world, view.eye);
    let extent = extent_within(rig.world.extent(), view.eye, 96.0);
    let mut cut = Frame::new(&rig.device, 16, 16);
    cut.set_lighting(reference_lighting(false, Light::Baked));
    let arms = [
        (bake::SeedForm::Independent, 32u32),
        (bake::SeedForm::ChunkSide, 32),
        (bake::SeedForm::GlobalSide, 32),
        (bake::SeedForm::Lattice, 32),
        (bake::SeedForm::Independent, 512),
    ];
    for (form, entry) in arms {
        for size in [1_000u64, 10_000, 100_000, 300_000, 1_000_000] {
            let k = (size.div_ceil(u64::from(entry)) as usize).min(faces.len());
            let set: std::collections::HashSet<([i32; 3], u8)> =
                faces[..k].iter().copied().collect();
            let set = Arc::new(set);
            let filter: bake::FaceFilter = Arc::new(move |v, f| set.contains(&(v, f)));
            let settings = BakeSettings {
                round: entry,
                n_min: entry,
                n_max: entry,
                form,
                filter: Some(filter),
                timer: timer.clone(),
                ..BakeSettings::default()
            };
            // `VXE_CUT_WARM`: before each timed run, a bake of every face at 32 samples (about
            // 2 M samples) on the same faces' chunks, so the GPU is not idle into the run.
            let warm = std::env::var("VXE_CUT_WARM").is_ok_and(|v| v == "1");
            let warm_settings = BakeSettings {
                round: 32,
                n_min: 32,
                n_max: 32,
                form,
                ..BakeSettings::default()
            };
            let (mut wall, mut gpu, mut len) = (Vec::new(), Vec::new(), 0.0);
            let mut samples = 0;
            for _ in 0..6 {
                if warm {
                    cut.bake_until(
                        &rig.device,
                        &rig.queue,
                        &mut rig.gpu,
                        &rig.world,
                        extent,
                        &warm_settings,
                        &mut |_| {},
                    )
                    .expect("the warm-up bakes");
                }
                let r = cut
                    .bake_until(
                        &rig.device,
                        &rig.queue,
                        &mut rig.gpu,
                        &rig.world,
                        extent,
                        &settings,
                        &mut |_| {},
                    )
                    .expect("the cost run bakes");
                assert_eq!(r.submits, 1, "one dispatch");
                samples = r.samples;
                wall.push((r.gpu_seconds * 1e9 / r.samples as f64) as f32);
                gpu.push((r.dispatch_seconds * 1e9 / r.samples as f64) as f32);
                len = r.ray_len / r.rays.max(1) as f64;
            }
            // The first run warms the pipeline; the median of the other five.
            let each: Vec<String> = gpu.iter().map(|g| format!("{g:.1}")).collect();
            let (mut wall, mut gpu) = (wall[1..].to_vec(), gpu[1..].to_vec());
            log.line(&format!(
                "cost {} {} entry {entry} warm {warm} adapter {}: {samples} samples ({k} faces), ns a sample wall {:.2} gpu {:.2} (median of 5; gpu each {}), mean shadow ray {len:.1}",
                view.name,
                form.name(),
                rig.adapter,
                q(&mut wall, 0.5),
                q(&mut gpu, 0.5),
                each.join(" "),
            ));
        }
    }
}

/// A CPU sampler over `World::raycast` on this thread: per sample a uniform point of the face,
/// 32 candidates drawn uniformly from the emitter voxels within the light radius' box around
/// the face's chunk, resampled by `cos / d^2`, and one ray to the kept emitter's centre.
fn cut_cpu(view: &ReadView) {
    let mut log = Log::open("cut-cpu.txt");
    let world = reference_world(view.scene);
    let faces = shuffled_faces(&world, view.eye);
    let e = extent_within(world.extent(), view.eye, 96.0);
    let emitters: Vec<[i32; 3]> = emitters_by_chunk(&world, e)
        .into_values()
        .flatten()
        .collect();
    assert!(!emitters.is_empty());
    let mut rng = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    let (n_faces, n) = (2000usize, 64u32);
    let (mut visible, mut rays) = (0u64, 0u64);
    let t = std::time::Instant::now();
    for &(v, face) in faces.iter().take(n_faces) {
        let axis = usize::from(face / 2);
        let sign = if face % 2 == 1 { 1.0 } else { -1.0 };
        for _ in 0..n {
            let mut p = v.map(|x| f64::from(x) + 0.5);
            p[axis] += 0.5 * sign + 1e-3 * sign;
            for k in [(axis + 1) % 3, (axis + 2) % 3] {
                p[k] += (next() % 1024) as f64 / 1024.0 - 0.5;
            }
            let (mut w_sum, mut kept) = (0f64, None);
            for _ in 0..32 {
                let c = emitters[(next() % emitters.len() as u64) as usize];
                let d = [0, 1, 2].map(|k| f64::from(c[k]) + 0.5 - p[k]);
                let d2 = d.iter().map(|x| x * x).sum::<f64>();
                let cos = (d[axis] * sign / d2.sqrt()).max(0.0);
                let w = cos / d2.max(1.0);
                w_sum += w;
                if w > 0.0 && (next() % 1_000_000) as f64 / 1e6 * w_sum < w {
                    kept = Some(c);
                }
            }
            let Some(c) = kept else { continue };
            let o = Fixed3(p.map(|x| (x * FIXED_ONE as f64) as i64));
            let dir = [0, 1, 2].map(|k| (i64::from(c[k]) * FIXED_ONE + FIXED_ONE / 2) - o.0[k]);
            rays += 1;
            if let Ok(Some(hit)) = world.raycast(o, dir, 255)
                && hit.voxel.0 == c
            {
                visible += 1;
            }
        }
    }
    let secs = t.elapsed().as_secs_f64();
    let samples = n_faces as u64 * u64::from(n);
    log.line(&format!(
        "cpu {}: {samples} samples on one thread ({n_faces} faces x {n}), {rays} rays, {visible} reached the emitter, {:.1} s, ns a sample {:.0}",
        view.name,
        secs,
        secs * 1e9 / samples as f64
    ));
}

/// The pane's display pixels over 250 (luma) at the hall, at exposure 8 and 16 (the HUD's +4 EV),
/// lamp off and on, with their count and centroid against the frame's centre (the lamp's cone
/// axis along the view); and the same with the store swapped for the table, and with the lamp's
/// bounce on as the window runs it.
fn cut_glass(view: &ReadView) {
    let mut log = Log::open("cut-glass.txt");
    let size = NATIVE;
    let (mut rig, piece) = reference_rig(view, size);
    let camera = view_camera(view);
    let op = pane_op(&rig, &camera, piece);
    let lp_off = LightParams::default();
    let frame_at = |rig: &mut ReadRig, lighting: Lighting, lp: &LightParams| -> Vec<u8> {
        rig.frame.set_lighting(lighting);
        for _ in 0..64 {
            rig.frame
                .step(&rig.device, &rig.queue, &rig.gpu, &camera, lp);
        }
        readback::read_rgba8(&rig.device, &rig.queue, rig.frame.target())
    };
    let before = frame_at(&mut rig, reference_lighting(false, Light::Baked), &lp_off);
    let accum_before = readback::read_float(&rig.device, &rig.queue, rig.frame.accumulated());
    let dig = timed_dig(&mut rig, &[op], "pane");
    log.line(&format!("glass hall adapter {}: {dig}", rig.adapter));
    let after = frame_at(&mut rig, reference_lighting(false, Light::Baked), &lp_off);
    let mask: Vec<bool> = before
        .chunks(4)
        .zip(after.chunks(4))
        .map(|(a, b)| a != b)
        .collect();
    let pane = mask.iter().filter(|m| **m).count();
    // What the pane's pixels showed before it, and the accumulated lighting (before the display)
    // there before and after it, lamp off, exposure 8.
    let accum_after = readback::read_float(&rig.device, &rig.queue, rig.frame.accumulated());
    let lum = |p: &[f32; 4]| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
    let y0 = luma(&before);
    let bright_before = mask
        .iter()
        .enumerate()
        .filter(|(i, m)| **m && y0[*i] > 250.0)
        .count();
    let mut a0: Vec<f32> = accum_before
        .iter()
        .zip(&mask)
        .filter(|(_, m)| **m)
        .map(|(p, _)| lum(p))
        .collect();
    let mut a1: Vec<f32> = accum_after
        .iter()
        .zip(&mask)
        .filter(|(_, m)| **m)
        .map(|(p, _)| lum(p))
        .collect();
    let mut y_before: Vec<f32> = y0
        .iter()
        .zip(&mask)
        .filter(|(_, m)| **m)
        .map(|(v, _)| *v)
        .collect();
    log.line(&format!(
        "glass hall before the pane, same pixels: over 250 {bright_before} ({:.4}), luma p50 {:.0} p90 {:.0}; accumulated lighting luminance p50 / p90 / max before {:.4} / {:.4} / {:.4}, with the pane {:.4} / {:.4} / {:.4}",
        bright_before as f64 / pane.max(1) as f64,
        q(&mut y_before, 0.5),
        q(&mut y_before, 0.9),
        q(&mut a0, 0.5),
        q(&mut a0, 0.9),
        q(&mut a0, 1.0),
        q(&mut a1, 0.5),
        q(&mut a1, 0.9),
        q(&mut a1, 1.0),
    ));
    for (light, lname) in [(Light::Baked, "store"), (Light::Table, "table")] {
        for bounce in [false, true] {
            if light == Light::Table && bounce {
                continue;
            }
            for exposure in [8.0f32, 16.0] {
                for (lamp, lp) in [
                    (false, LightParams::default()),
                    (true, LightParams::headlamp()),
                ] {
                    let lighting = Lighting {
                        exposure: Exposure::Manual(exposure),
                        lamp_bounce: bounce,
                        ..reference_lighting(lamp, light)
                    };
                    let img = frame_at(&mut rig, lighting, &lp);
                    let y = luma(&img);
                    let (mut n, mut sx, mut sy) = (0u64, 0f64, 0f64);
                    for (i, m) in mask.iter().enumerate() {
                        if *m && y[i] > 250.0 {
                            n += 1;
                            sx += f64::from(i as u32 % size[0]);
                            sy += f64::from(i as u32 / size[0]);
                        }
                    }
                    let mut pane_y: Vec<f32> = y
                        .iter()
                        .zip(&mask)
                        .filter(|(_, m)| **m)
                        .map(|(v, _)| *v)
                        .collect();
                    let centroid = if n > 0 {
                        format!(
                            "({:.0}, {:.0}), off centre by ({:+.0}, {:+.0}) px",
                            sx / n as f64,
                            sy / n as f64,
                            sx / n as f64 - f64::from(size[0]) / 2.0,
                            sy / n as f64 - f64::from(size[1]) / 2.0
                        )
                    } else {
                        "-".to_owned()
                    };
                    log.line(&format!(
                        "glass hall {lname} bounce {} exposure {exposure} lamp {}: pane pixels {pane}, over 250 {n} ({:.4}), centroid {centroid}; pane luma p50 {:.0} p90 {:.0} max {:.0}",
                        if bounce { "on" } else { "off" },
                        if lamp { "on" } else { "off" },
                        n as f64 / pane.max(1) as f64,
                        q(&mut pane_y, 0.5),
                        q(&mut pane_y, 0.9),
                        q(&mut pane_y, 1.0),
                    ));
                }
            }
        }
    }
}

/// The glass pane of the readings: `PANE_EDGE` square, 2 voxels thick, 6 voxels ahead of
/// `camera`'s eye along the larger of its forward's x and y, as the fly-through's `1` places it.
fn pane_op(rig: &ReadRig, camera: &Camera, piece: ChunkExtent) -> EditOp {
    let glass = (rig.world.materials().len() - 1) as vxe_world::MaterialId;
    let (min, max) = pane_box(camera, piece);
    EditOp::box_(
        Fixed3(min.map(|v| i64::from(v) * FIXED_ONE)),
        Fixed3(max.map(|v| i64::from(v) * FIXED_ONE)),
        Mode::Fill(glass),
    )
}

/// [`pane_op`]'s box in voxels, `[min, max)`.
fn pane_box(camera: &Camera, piece: ChunkExtent) -> ([i32; 3], [i32; 3]) {
    let e = camera.position.to_array().map(|v| v.floor() as i32);
    let (fwd, _, _) = camera.basis();
    let k = if fwd.x.abs() >= fwd.y.abs() { 0 } else { 1 };
    let ahead = if fwd.to_array()[k] >= 0.0 { 1 } else { -1 };
    let near = e[k] + ahead * 6;
    let mut min = [0i32; 3];
    let mut max = [0i32; 3];
    (min[k], max[k]) = if ahead > 0 {
        (near, near + 2)
    } else {
        (near - 1, near + 1)
    };
    for a in [1 - k, 2] {
        min[a] = e[a] - 20;
        max[a] = e[a] + 20;
    }
    assert!(inside(piece, min) && inside(piece, max.map(|v| v - 1)));
    (min, max)
}

/// The translucent voxels of `glass` a ray from `eye` through `hit` enters inside `[min, max)`
/// before `hit`, up to `cap`: the layers the composite's walk counts, by a CPU voxel walk.
fn layers_on_ray(
    world: &World,
    glass: vxe_world::MaterialId,
    (min, max): ([i32; 3], [i32; 3]),
    eye: [f32; 3],
    hit: [f32; 3],
    cap: u32,
) -> u32 {
    let to = [0, 1, 2].map(|k| f64::from(hit[k] - eye[k]));
    let t_hit = (to[0] * to[0] + to[1] * to[1] + to[2] * to[2]).sqrt();
    let d = to.map(|v| v / t_hit);
    let o = eye.map(f64::from);
    let (mut t0, mut t1) = (0.0f64, t_hit);
    for k in 0..3 {
        let (lo, hi) = (f64::from(min[k]), f64::from(max[k]));
        if d[k] == 0.0 {
            if o[k] < lo || o[k] >= hi {
                return 0;
            }
            continue;
        }
        let (a, b) = ((lo - o[k]) / d[k], (hi - o[k]) / d[k]);
        t0 = t0.max(a.min(b));
        t1 = t1.min(a.max(b));
    }
    if t1 <= t0 {
        return 0;
    }
    let p = [0, 1, 2].map(|k| o[k] + d[k] * (t0 + 1e-6));
    let mut v = [0, 1, 2].map(|k| (p[k].floor() as i32).clamp(min[k], max[k] - 1));
    let step = d.map(|x| if x > 0.0 { 1 } else { -1 });
    let mut next = [0, 1, 2].map(|k| {
        if d[k] == 0.0 {
            f64::INFINITY
        } else {
            let edge = f64::from(v[k] + i32::from(d[k] > 0.0));
            (edge - o[k]) / d[k]
        }
    });
    let delta = d.map(|x| {
        if x == 0.0 {
            f64::INFINITY
        } else {
            1.0 / x.abs()
        }
    });
    let mut n = 0;
    loop {
        if world.material_at(vxe_world::VoxelCoord(v)) == Some(glass) {
            n += 1;
            if n >= cap {
                return n;
            }
        }
        let k = if next[0] <= next[1] && next[0] <= next[2] {
            0
        } else if next[1] <= next[2] {
            1
        } else {
            2
        };
        if next[k] >= t1 {
            return n;
        }
        v[k] += step[k];
        next[k] += delta[k];
        if v[k] < min[k] || v[k] >= max[k] {
            return n;
        }
    }
}

/// The owner's eye in the glass screenshot (its HUD's `fly at`), chunk [2, 5, 3].
const OWNER_GLASS_EYE: [f32; 3] = [169.0, 359.7, 222.7];

/// The parts the composite sums at a pane pixel under a per-layer composite model: it reads the
/// layer target's `w` as `(1 - a)^k`, while the filter of ADR-0047 writes `luminance(T)` there,
/// so its model figures do not describe the filter. Read from the frame's own textures before the display: the layer walk's premultiplied glass term (times the exposure, as the composite
/// adds it) and its transmittance; the hit behind (its material, the irradiance the composite
/// reads, its Lambertian term and its emission); the composite rebuilt from them against the
/// displayed pixel (any term outside both). At the hall's shot and at the owner's eye with the
/// shot's heading, exposure 8, the store: the lamp off, the lamp off with the headlamp's
/// intensity at 0, and the lamp on. Then done-when 2's figures: the pane's pixels over 250
/// against the share before the pane, and each pixel against `(1 - a) * behind + a * albedo *
/// light_at_glass` with `a` 0.6, `light_at_glass` the layer's own light per voxel; and against the
/// k-layer model `(1 - a)^k * behind` plus the layers' own terms (the glass term read), `k` counted
/// by a CPU walk of the pane's voxels along the pixel's ray up to the composite's layer cap.
/// `VXE_PARTS_POSES` (`shot`, `owner`) and `VXE_PARTS_ROWS` (`off`, `dark`, `on`) narrow it.
fn cut_glass_parts(view: &ReadView) {
    let mut log = Log::open("cut-glass-parts.txt");
    let size = NATIVE;
    let lum = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let poses = std::env::var("VXE_PARTS_POSES").unwrap_or_else(|_| "shot,owner".to_owned());
    let rows = std::env::var("VXE_PARTS_ROWS").unwrap_or_else(|_| "off,dark,on".to_owned());
    for (key, pose, eye) in [
        ("shot", "hall shot", None),
        ("owner", "owner eye", Some(OWNER_GLASS_EYE)),
    ] {
        if !poses.split(',').any(|p| p == key) {
            continue;
        }
        let (mut rig, piece) = reference_rig(view, size);
        let mut camera = view_camera(view);
        if let Some(e) = eye {
            camera.position = e.into();
        }
        let op = pane_op(&rig, &camera, piece);
        let pbox = pane_box(&camera, piece);
        let cap = vxe_render::marcher::LookParams::default().translucent_layers;
        let eye = camera.position.to_array();
        let mats = rig.world.materials().clone();
        let glass = (mats.len() - 1) as vxe_world::MaterialId;
        let g = mats.get(glass).expect("the glass row");
        let (ga, galb) = (g.opacity, g.albedo);
        let frame_at = |rig: &mut ReadRig, lighting: Lighting, lp: &LightParams| -> Vec<u8> {
            rig.frame.set_lighting(lighting);
            for _ in 0..64 {
                rig.frame
                    .step(&rig.device, &rig.queue, &rig.gpu, &camera, lp);
            }
            readback::read_rgba8(&rig.device, &rig.queue, rig.frame.target())
        };
        let base = reference_lighting(false, Light::Baked);
        let before = frame_at(&mut rig, base, &LightParams::default());
        let y0 = luma(&before);
        let dig = timed_dig(&mut rig, &[op], "pane");
        log.line(&format!(
            "parts {pose} adapter {}: eye {:?}, glass opacity {ga} albedo {galb:?}; {dig}",
            rig.adapter,
            camera.position.to_array()
        ));
        let dark = LightParams {
            intensity: 0.0,
            ..LightParams::default()
        };
        for (row, lname, lamp, lp) in [
            ("off", "lamp off", false, LightParams::default()),
            ("dark", "lamp off, intensity 0", false, dark),
            ("on", "lamp on", true, LightParams::default()),
        ] {
            if !rows.split(',').any(|r| r == row) {
                continue;
            }
            let lighting = reference_lighting(lamp, Light::Baked);
            let ex = 8.0f32;
            let img = frame_at(&mut rig, lighting, &lp);
            let y = luma(&img);
            let info = readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
            let layer = readback::read_float(&rig.device, &rig.queue, rig.frame.layers());
            let lit_tex = if lighting.filter {
                rig.frame.filtered()
            } else {
                rig.frame.irradiance()
            };
            let lit = readback::read_float(&rig.device, &rig.queue, lit_tex);
            let pos = readback::read_float(&rig.device, &rig.queue, rig.frame.hit_positions());
            let (mut dk, mut k_walk, mut k_off) = (vec![], BTreeMap::<u32, u64>::new(), 0u64);
            let (mut n, mut over, mut over_before) = (0u64, 0u64, 0u64);
            let (mut g_l, mut w, mut b_l, mut e_l, mut res, mut dw2) =
                (vec![], vec![], vec![], vec![], vec![], vec![]);
            let (mut d_glass, mut d_trans, mut d_behind, mut d_act) =
                (vec![], vec![], vec![], vec![]);
            let mut light_glass = vec![];
            let (mut emitters, mut layer_counts) = (0u64, BTreeMap::<u32, u64>::new());
            for i in 0..info.len() {
                let f = info[i][3];
                if f & 1 == 0 || f & 4 == 0 {
                    continue;
                }
                n += 1;
                let m = mats.get(info[i][2]).expect("a material");
                let l = layer[i];
                let gt = [l[0] * ex, l[1] * ex, l[2] * ex];
                let mut e = [lit[i][0], lit[i][1], lit[i][2]];
                if !lighting.filter {
                    e = e.map(|v| v + lit[i][3]);
                }
                let s = rig.frame.emission_scale(info[i][2]);
                let em = [0, 1, 2].map(|k| m.emission[k] * s * ex);
                if em.iter().any(|v| *v > 0.0) {
                    emitters += 1;
                }
                let behind = [0, 1, 2].map(|k| m.albedo[k] * e[k] / std::f32::consts::PI + em[k]);
                let model = [0, 1, 2].map(|k| gt[k] + l[3] * behind[k]);
                let dl = |c: [f32; 3]| {
                    let b = tone_bytes([c[0], c[1], c[2], 1.0], &lighting);
                    0.2126 * f32::from(b[0]) + 0.7152 * f32::from(b[1]) + 0.0722 * f32::from(b[2])
                };
                // Layers entered: transmittance (1 - a)^k.
                let k = ((l[3].max(1e-6)).ln() / (1.0 - ga).ln()).round() as u32;
                *layer_counts.entry(k).or_default() += 1;
                // The layer's light per voxel, taken equal across the k voxels entered:
                // gt = a * albedo * L * sum_{j<k} (1 - a)^j.
                let sum: f32 = (0..k).map(|j| (1.0 - ga).powi(j as i32)).sum();
                let lg = [0, 1, 2].map(|c| {
                    if sum > 0.0 && galb[c] > 0.0 {
                        gt[c] / (ga * galb[c] * sum)
                    } else {
                        0.0
                    }
                });
                let target = [0, 1, 2].map(|c| (1.0 - ga) * behind[c] + ga * galb[c] * lg[c]);
                g_l.push(lum(gt));
                w.push(l[3]);
                b_l.push(lum(behind));
                e_l.push(lum(e));
                light_glass.push(lum(lg));
                d_glass.push(dl(gt));
                d_trans.push(dl(behind.map(|v| v * l[3])));
                d_behind.push(dl(behind));
                d_act.push(y[i]);
                res.push((y[i] - dl(model)).abs());
                // The k-layer model: k from a CPU walk of the pane's voxels along the pixel's
                // ray, the layers' own terms the glass term read (their sum, by the walk's
                // definition: a * (1 - a)^(i - 1) * albedo * light at layer i).
                let kw = layers_on_ray(
                    &rig.world,
                    glass,
                    pbox,
                    eye,
                    [pos[i][0], pos[i][1], pos[i][2]],
                    cap,
                );
                *k_walk.entry(kw).or_default() += 1;
                let tk = (1.0 - ga).powi(kw as i32);
                k_off += u64::from((l[3] - tk).abs() > 1e-3);
                let model_k = [0, 1, 2].map(|c| gt[c] + tk * behind[c]);
                dk.push((y[i] - dl(model_k)).abs());
                dw2.push((y[i] - dl(target)).abs());
                over += u64::from(y[i] > 250.0);
                over_before += u64::from(y0[i] > 250.0);
            }
            let p = |v: &mut Vec<f32>| {
                format!("{:.4} / {:.4} / {:.4}", q(v, 0.5), q(v, 0.9), q(v, 1.0))
            };
            log.line(&format!(
                "parts {pose} {lname}: pane pixels {n} (emitting hits behind {emitters}, layers entered {layer_counts:?}); glass term (layer x exposure) luminance p50/p90/max {}; transmittance {}; light at the glass per voxel {}; behind: irradiance {}, radiance {}",
                p(&mut g_l), p(&mut w), p(&mut light_glass), p(&mut e_l), p(&mut b_l)
            ));
            log.line(&format!(
                "parts {pose} {lname}: display luma p50/p90/max: glass term alone {}, transmitted behind {}, behind alone {}, the pixel {}; |pixel - rebuilt composite| {}",
                p(&mut d_glass), p(&mut d_trans), p(&mut d_behind), p(&mut d_act), p(&mut res)
            ));
            log.line(&format!(
                "parts {pose} {lname}: over 250 with the pane {over} ({:.4}), before it {over_before} ({:.4}); |pixel - ((1 - a) behind + a albedo light_at_glass)| p50/p90/max {}",
                over as f64 / n.max(1) as f64,
                over_before as f64 / n.max(1) as f64,
                p(&mut dw2)
            ));
            log.line(&format!(
                "parts {pose} {lname}: k-layer model, k by a CPU walk of the pane (cap {cap}): layers {k_walk:?}, transmittance off (1 - a)^k by over 1e-3 at {k_off} pixels; |pixel - ((1 - a)^k behind + glass term)| p50/p90/max {}",
                p(&mut dk)
            ));
        }
    }
}

/// The texels of an `r32uint` texture, rows top to bottom (`read_rgba8` copies any 4-byte texel).
fn read_r32u(device: &wgpu::Device, queue: &wgpu::Queue, t: &wgpu::Texture) -> Vec<u32> {
    assert_eq!(t.format(), wgpu::TextureFormat::R32Uint);
    readback::read_rgba8(device, queue, t)
        .chunks(4)
        .map(|b| u32::from_le_bytes(b.try_into().expect("4 bytes")))
        .collect()
}

/// The angle under which a pane pixel's ray counts as near the pane's normal, for the filter's
/// "not dark" reading.
const NEAR_NORMAL_DEG: f64 = 10.0;

/// ADR-0047's filter at the hall's shot with [`pane_op`]'s pane, exposure 8, the store, the
/// headlamp's parameters as the window passes them, 64 frames a row, on the adapter
/// `VXE_TEST_ADAPTER` names (unset: lavapipe). Rows: before the pane lamp off and lamp on, then
/// the pane dug in and the same two. The pane's pixels are those whose hit has a translucent
/// voxel in front (the hit record's bit 4) after the dig.
/// - **No voxel grid:** over pairs of horizontally or vertically adjacent pane pixels, both flat
///   before the pane (each within 1 level of its four neighbours), `|y_a - y_b|` with the pane
///   minus the same before it, lamp off; p50, p90, max.
/// - **No white-out:** lamp on, the pane's pixels over 250 with the pane and the same pixels
///   over 250 without it.
/// - **Not dark:** lamp off, the pane's radiance before the display (`layer.rgb * ex + T *
///   behind`) against its behind's (`behind` = [`hit_radiance`] with the pane), mean luminance
///   over the pixels whose ray is within [`NEAR_NORMAL_DEG`] of the pane's normal, and over the
///   whole pane; the path length `L` through the pane by [`path_in_boxes`] beside it.
/// - **What is behind reads its cell:** lamp off, each pane pixel's behind with the pane against
///   the same pixel's radiance without it, in display levels; p50, p90, max.
/// - The rebuilt composite against the displayed pixel, lamp off and on (any term outside both).
fn cut_glass_filter(view: &ReadView) {
    let mut log = Log::open("cut-glass-filter.txt");
    let size = NATIVE;
    let w = size[0] as usize;
    let (mut rig, piece) = reference_rig(view, size);
    let camera = view_camera(view);
    let op = pane_op(&rig, &camera, piece);
    let pbox = pane_box(&camera, piece);
    let axis = (0..3)
        .find(|k| pbox.1[*k] - pbox.0[*k] == 2)
        .expect("the pane's thin axis");
    let mats = rig.world.materials().clone();
    let ex = 8.0f32;
    let lp = LightParams::headlamp();
    let lum = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let row = |rig: &mut ReadRig, lamp: bool| {
        let lighting = reference_lighting(lamp, Light::Baked);
        rig.frame.set_lighting(lighting);
        for _ in 0..64 {
            rig.frame
                .step(&rig.device, &rig.queue, &rig.gpu, &camera, &lp);
        }
        let img = readback::read_rgba8(&rig.device, &rig.queue, rig.frame.target());
        let rad = hit_radiance(&rig.device, &rig.queue, &rig.frame, &mats, ex);
        (luma(&img), rad, lighting)
    };
    let (y0_off, r0_off, lighting_off) = row(&mut rig, false);
    let (y0_on, _, _) = row(&mut rig, true);
    let dig = timed_dig(&mut rig, &[op], "pane");
    log.line(&format!(
        "filter hall adapter {}: eye {:?}, pane {pbox:?} (thin axis {axis}); {dig}",
        rig.adapter,
        camera.position.to_array()
    ));
    let (y1_off, r1_off, _) = row(&mut rig, false);
    let info = readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    let pos = readback::read_float(&rig.device, &rig.queue, rig.frame.hit_positions());
    let layer_off = readback::read_float(&rig.device, &rig.queue, rig.frame.layers());
    let t_off: Vec<[f32; 3]> = read_r32u(&rig.device, &rig.queue, rig.frame.transmittance())
        .into_iter()
        .map(bake::rgb9e5_unpack)
        .collect();
    let (y1_on, r1_on, lighting_on) = row(&mut rig, true);
    let layer_on = readback::read_float(&rig.device, &rig.queue, rig.frame.layers());
    let t_on: Vec<[f32; 3]> = read_r32u(&rig.device, &rig.queue, rig.frame.transmittance())
        .into_iter()
        .map(bake::rgb9e5_unpack)
        .collect();
    let pane: Vec<bool> = info
        .iter()
        .map(|r| r[3] & 1 != 0 && r[3] & 4 != 0)
        .collect();
    let n = pane.iter().filter(|p| **p).count();
    let dl = |c: [f32; 3], lighting: &Lighting| {
        let b = tone_bytes([c[0], c[1], c[2], 1.0], lighting);
        0.2126 * f32::from(b[0]) + 0.7152 * f32::from(b[1]) + 0.0722 * f32::from(b[2])
    };
    let p3 = |v: &mut Vec<f32>| format!("{:.2} / {:.2} / {:.2}", q(v, 0.5), q(v, 0.9), q(v, 1.0));

    // No voxel grid.
    let h = y0_off.len() / w;
    let flat = |i: usize| -> bool {
        let (x, y) = (i % w, i / w);
        let mut ok = true;
        for (dx, dy) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
            let (nx, ny) = (x as i64 + dx, y as i64 + dy);
            if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                return false;
            }
            let j = ny as usize * w + nx as usize;
            ok &= (y0_off[i] - y0_off[j]).abs() <= 1.0;
        }
        ok
    };
    let mut grid = vec![];
    let mut grid_after = vec![];
    for i in 0..pane.len() {
        if !pane[i] || !flat(i) {
            continue;
        }
        let (x, y) = (i % w, i / w);
        for j in [(x + 1 < w).then_some(i + 1), (y + 1 < h).then_some(i + w)]
            .into_iter()
            .flatten()
        {
            if pane[j] && flat(j) {
                let after = (y1_off[i] - y1_off[j]).abs();
                grid.push(after - (y0_off[i] - y0_off[j]).abs());
                grid_after.push(after);
            }
        }
    }
    let pairs = grid.len();
    log.line(&format!(
        "filter hall lamp off: pane pixels {n}; flat pairs {pairs}; |neighbour difference| with the pane p50/p90/max {}; with the pane minus before it p50/p90/max {}",
        p3(&mut grid_after),
        p3(&mut grid)
    ));

    // No white-out.
    let over_on = (0..pane.len())
        .filter(|i| pane[*i] && y1_on[*i] > 250.0)
        .count();
    let over_on_before = (0..pane.len())
        .filter(|i| pane[*i] && y0_on[*i] > 250.0)
        .count();
    let over_off = (0..pane.len())
        .filter(|i| pane[*i] && y1_off[*i] > 250.0)
        .count();
    let mut on_y: Vec<f32> = (0..pane.len())
        .filter(|i| pane[*i])
        .map(|i| y1_on[i])
        .collect();
    log.line(&format!(
        "filter hall lamp on: pane pixels over 250 with the pane {over_on} ({:.4}), the same pixels without it {over_on_before} ({:.4}); pane luma p50/p90/max {}; lamp off over 250 {over_off}",
        over_on as f64 / n.max(1) as f64,
        over_on_before as f64 / n.max(1) as f64,
        p3(&mut on_y)
    ));

    // Not dark, and the rebuilt composite.
    let eye = camera.position.to_array().map(f64::from);
    let cos_near = NEAR_NORMAL_DEG.to_radians().cos();
    let boxes = [pbox];
    let (mut near_pane, mut near_behind, mut near_n) = (0f64, 0f64, 0usize);
    let (mut all_pane, mut all_behind) = (0f64, 0f64);
    let (mut l_near, mut l_all, mut t_lum) = (vec![], vec![], vec![]);
    let (mut rebuilt_off, mut rebuilt_on) = (vec![], vec![]);
    for i in 0..pane.len() {
        if !pane[i] {
            continue;
        }
        let to = [0, 1, 2].map(|k| f64::from(pos[i][k]) - eye[k]);
        let len = (to[0] * to[0] + to[1] * to[1] + to[2] * to[2]).sqrt();
        let d = to.map(|v| v / len);
        let l = path_in_boxes(eye, d, len, &boxes) as f32;
        let behind = r1_off[i];
        let px = [0, 1, 2].map(|k| layer_off[i][k] * ex + t_off[i][k] * behind[k]);
        all_pane += f64::from(lum(px));
        all_behind += f64::from(lum(behind));
        l_all.push(l);
        t_lum.push(lum(t_off[i]));
        if d[axis].abs() >= cos_near {
            near_pane += f64::from(lum(px));
            near_behind += f64::from(lum(behind));
            near_n += 1;
            l_near.push(l);
        }
        rebuilt_off.push((y1_off[i] - dl(px, &lighting_off)).abs());
        let px_on = [0, 1, 2].map(|k| layer_on[i][k] * ex + t_on[i][k] * r1_on[i][k]);
        rebuilt_on.push((y1_on[i] - dl(px_on, &lighting_on)).abs());
    }
    log.line(&format!(
        "filter hall lamp off: not dark: within {NEAR_NORMAL_DEG} degrees of the normal {near_n} pixels, mean luminance pane {:.5} behind {:.5}, ratio {:.4} (L p50/p90/max {}); the whole pane mean pane {:.5} behind {:.5}, ratio {:.4} (L {}; luminance of T {})",
        near_pane / near_n.max(1) as f64,
        near_behind / near_n.max(1) as f64,
        near_pane / near_behind.max(1e-12),
        p3(&mut l_near),
        all_pane / n.max(1) as f64,
        all_behind / n.max(1) as f64,
        all_pane / all_behind.max(1e-12),
        p3(&mut l_all),
        p3(&mut t_lum)
    ));
    log.line(&format!(
        "filter hall: |pixel - display(layer ex + T behind)| p50/p90/max lamp off {}, lamp on {}",
        p3(&mut rebuilt_off),
        p3(&mut rebuilt_on)
    ));

    // What is behind reads its cell.
    let mut behind_off = vec![];
    for i in 0..pane.len() {
        if pane[i] {
            behind_off.push((dl(r1_off[i], &lighting_off) - dl(r0_off[i], &lighting_off)).abs());
        }
    }
    log.line(&format!(
        "filter hall lamp off: behind with the pane against the same pixels without it, |levels| p50/p90/max {}",
        p3(&mut behind_off)
    ));
}

/// The subdirectory of `live/bake/` holding the store files baked before ADR-0047's source.
const OLD_FILES: &str = "pre-5a";

/// A store file's blocks by chunk, and the cut files' blocks before the filter: the file's key
/// fields are skipped (26 for the reference's magic, 28 for a cut's), so a file with another
/// source hash parses too.
type FileBlocks = (BTreeMap<[i32; 3], Vec<u32>>, BTreeMap<[i32; 3], Vec<u32>>);

fn read_store_blocks(path: &std::path::Path) -> FileBlocks {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    let mut at = 0usize;
    let mut take = |n: usize| -> &[u8] {
        let s = &bytes[at..at + n];
        at += n;
        s
    };
    let fields = match take(8) {
        b"VXEBAKE1" => 26,
        b"VXEBAKE2" => 28,
        m => panic!("{path:?}: magic {m:?}"),
    };
    let _ = take(8 * fields);
    let u64_of = |b: &[u8]| u64::from_le_bytes(b.try_into().expect("8 bytes"));
    let lists = if fields == 28 { 2 } else { 1 };
    let mut out = [BTreeMap::new(), BTreeMap::new()];
    for map in out.iter_mut().take(lists) {
        let count = u64_of(take(8));
        for _ in 0..count {
            let c = [0, 1, 2].map(|_| i32::from_le_bytes(take(4).try_into().expect("4 bytes")));
            let words = u64_of(take(8)) as usize;
            let block: Vec<u32> = take(words * 4)
                .chunks(4)
                .map(|b| u32::from_le_bytes(b.try_into().expect("4 bytes")))
                .collect();
            map.insert(c, block);
        }
    }
    let [blocks, raw] = out;
    (blocks, raw)
}

/// A store file against an older one with the same key fields but the source hash (the files
/// in `live/bake/` against the copies in its subdirectory [`OLD_FILES`], ADR-0047): the reference and
/// independent 512 at the vein, the patch and the hall. A chunk whose block is unchanged counts
/// as identical; any other is re-listed from the reading's world (`bake::list_world_chunk`):
/// every opaque face's cell, found by its key in the new block's directory and in the old
/// block's (the new keys less the translucent faces), reads bit-identical, both words, in the
/// filtered blocks and the cut's blocks before the filter. Logs to
/// `live/bake-count/opaque-cells.txt`.
#[test]
#[ignore = "a reading of live/bake/'s files against live/bake/pre-5a/"]
fn store_files_keep_opaque_cells() {
    let mut log = Log::open("opaque-cells.txt");
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../live/bake");
    let mut bad_total = 0u64;
    for (pose, scene) in [
        ("vein", Scene::Level),
        ("patch", Scene::Level),
        ("hall", Scene::Showcase),
    ] {
        let world = reference_world(scene);
        let classes = bake::face_classes(world.materials());
        for file in [format!("{pose}.bin"), format!("{pose}.independent-512.bin")] {
            let new_path = std::path::Path::new(dir).join(&file);
            let old_path = std::path::Path::new(dir).join(OLD_FILES).join(&file);
            let (new, new_raw) = read_store_blocks(&new_path);
            let (old, old_raw) = read_store_blocks(&old_path);
            let (mut same, mut keyed, mut opaque, mut glass, mut bad) =
                (0u64, 0u64, 0u64, 0u64, 0u64);
            let mut chunks_differ = 0u64;
            for (kind, n_map, o_map) in [("filtered", &new, &old), ("raw", &new_raw, &old_raw)] {
                if n_map.keys().ne(o_map.keys()) {
                    chunks_differ += 1;
                    log.line(&format!("{file} {kind}: the chunk lists differ"));
                }
                for (c, nb) in n_map {
                    let Some(ob) = o_map.get(c) else { continue };
                    if nb == ob {
                        same += 1;
                        continue;
                    }
                    keyed += 1;
                    let faces = bake::list_world_chunk(&world, ChunkCoord(*c), &classes)
                        .expect("the chunk");
                    let d = bake::DIRECTORY_WORDS as usize;
                    assert_eq!(
                        &nb[..d],
                        &faces.directory()[..],
                        "{file} {c:?}: new directory"
                    );
                    let mut at_old = bake::DIRECTORY_WORDS as u32;
                    for (slot, keys) in &faces.bricks {
                        let (at_new, n_new) =
                            (nb[usize::from(*slot)] >> 11, nb[usize::from(*slot)] & 2047);
                        let opaque_keys: Vec<u16> = keys
                            .iter()
                            .copied()
                            .filter(|k| !faces.is_glass(*slot, *k))
                            .collect();
                        let (o_at, o_n) =
                            (ob[usize::from(*slot)] >> 11, ob[usize::from(*slot)] & 2047);
                        assert_eq!(n_new as usize, keys.len(), "{file} {c:?} slot {slot}");
                        assert_eq!(
                            o_n as usize,
                            opaque_keys.len(),
                            "{file} {c:?} slot {slot}: old run"
                        );
                        if !opaque_keys.is_empty() {
                            assert_eq!(o_at, at_old, "{file} {c:?} slot {slot}: old offset");
                            at_old += 2 * o_n;
                        }
                        let mut j_old = 0usize;
                        for (j, k) in keys.iter().enumerate() {
                            if faces.is_glass(*slot, *k) {
                                glass += 1;
                                continue;
                            }
                            opaque += 1;
                            let n_cell =
                                &nb[(at_new as usize + 2 * j)..(at_new as usize + 2 * j + 2)];
                            let o_cell =
                                &ob[(o_at as usize + 2 * j_old)..(o_at as usize + 2 * j_old + 2)];
                            j_old += 1;
                            if n_cell != o_cell {
                                bad += 1;
                            }
                        }
                    }
                }
            }
            log.line(&format!(
                "{file}: blocks bit-identical {same}; compared by key {keyed} (opaque cells {opaque}, translucent cells {glass}, opaque cells that differ {bad}); chunk lists differing {chunks_differ}"
            ));
            bad_total += bad + chunks_differ;
        }
    }
    assert_eq!(bad_total, 0, "opaque cells or chunk lists differ");
}

/// The live queue's dig against a fresh bake (Plan 0059 Phase 8e): at the far pose, a radius-1
/// piece baked at the queue's settings ([`bake::queue_settings`], 8/4096), one 5-voxel carve at
/// the view's centre, its ring re-baked to each arm's settings in turn (the same box each time,
/// so each arm re-bakes the same bricks from the same edited world), then a fresh bake of the
/// edited world at the queue's settings. For each arm, every ring brick's six direction values
/// against the fresh bake's: equal words, within one `rgb9e5` step a channel, and display
/// levels; split into the bricks the box dilated by one voxel reaches ("touched") and the rest
/// ("untouched"). The pre-dig store against the fresh bake rides beside them, which is the edit's
/// own change. Logs to `live/bake-count/queue-dig.txt`.
#[test]
#[ignore = "a reading on the RTX: VXE_TEST_ADAPTER=rtx, under the perf lock"]
fn queue_dig_readings() {
    let mut log = Log::open("queue-dig.txt");
    let pose = flythrough::FAR_POSE;
    let eye = pose.position;
    let size = [192, 108];
    let mut rig = read_rig(Scene::Level, eye);
    rig.frame = Frame::new(&rig.device, size[0], size[1]);
    rig.frame.set_jitter(&rig.device, false);
    rig.frame
        .set_resolution(&rig.device, LightingResolution::Full);
    rig.frame.set_lighting(Lighting {
        lamp: false,
        lamp_bounce: false,
        exposure: Exposure::Manual(8.0),
        light: Light::Baked,
        ..Lighting::default()
    });
    let world_extent = rig.world.extent();
    let centre = eye.map(|v| (v / 64.0).floor() as i32);
    let piece = ChunkExtent::new(centre.map(|v| v - 1), centre.map(|v| v + 2));
    let piece = {
        let s = world_extent.size();
        let lo = [0, 1, 2].map(|k| piece.min.0[k].max(world_extent.min.0[k]));
        let hi = [0, 1, 2].map(|k| (piece.min.0[k] + 3).min(world_extent.min.0[k] + s[k]));
        ChunkExtent::new(lo, hi)
    };
    let queue_settings = bake::queue_settings();
    let t = std::time::Instant::now();
    let r = rig
        .frame
        .bake_until(
            &rig.device,
            &rig.queue,
            &mut rig.gpu,
            &rig.world,
            piece,
            &queue_settings,
            &mut |_| {},
        )
        .expect("the piece fits the pool");
    log.line(&format!(
        "queue dig, adapter {}, far pose chunk {centre:?}, piece {:?}..{:?}: the piece at 8/4096 in {:.0} ms, {} chunks, {} faces, {:.3e} samples",
        rig.adapter,
        piece.min.0,
        [0, 1, 2].map(|k| piece.min.0[k] + piece.size()[k]),
        t.elapsed().as_secs_f64() * 1e3,
        r.chunks,
        r.faces,
        r.samples as f64
    ));
    let before = frame_blocks(&rig.device, &rig.queue, &rig.frame, piece);
    let camera = pose.camera();
    rig.frame.step(
        &rig.device,
        &rig.queue,
        &rig.gpu,
        &camera,
        &LightParams::default(),
    );
    let hit = hit_at(&rig, size, size[0] / 2, size[1] / 2).expect("the view's centre hits rock");
    let c = hit.map(|v| i64::from(v.floor() as i32) * FIXED_ONE + FIXED_ONE / 2);
    let op = EditOp::sphere(Fixed3(c), 5 * FIXED_ONE, Mode::Carve);
    let applied = rig.world.apply(&op).expect("the carve applies");
    let (min, max) = applied.aabb.expect("the carve changed voxels");
    let dug = [min.0, max.0];
    rig.gpu
        .sync_now(&rig.world, &rig.device, &rig.queue)
        .expect("the mirror fits");
    let ring = bake::dig_ring(&[dug], piece);
    log.line(&format!(
        "queue dig: carve at {:?} ({:.1} voxels from the eye), box {:?}..{:?}, ring {} bricks in {} chunks",
        hit.map(|v| v.floor() as i32),
        (0..3).map(|k| (hit[k] - eye[k]).powi(2)).sum::<f32>().sqrt(),
        dug[0],
        dug[1],
        ring.values().map(Vec::len).sum::<usize>(),
        ring.len()
    ));
    let arms: Vec<(&str, BakeSettings)> = vec![
        (
            "the window's dig settings (the reference's default, n_max DIG_N_MAX)",
            BakeSettings {
                n_max: bake::DIG_N_MAX,
                ..BakeSettings::default()
            },
        ),
        ("the queue's settings (round 64)", queue_settings.clone()),
        (
            "the queue's settings, round BAKE_ROUND",
            BakeSettings {
                round: bake::BAKE_ROUND,
                ..queue_settings.clone()
            },
        ),
    ];
    let mut after = Vec::new();
    for (name, settings) in &arms {
        rig.frame.invalidate(dug[0], dug[1]);
        let t = std::time::Instant::now();
        let r = rig
            .frame
            .rebake_digs(
                &rig.device,
                &rig.queue,
                &mut rig.gpu,
                &rig.world,
                settings,
                &mut |_| {},
            )
            .expect("a box waits")
            .expect("the ring fits");
        let ms = t.elapsed().as_secs_f64() * 1e3;
        log.line(&format!(
            "queue dig, {name}: {} bricks, {} chunks, {} faces, {:.3e} samples, {ms:.0} ms",
            r.bricks, r.bake.chunks, r.bake.faces, r.bake.samples as f64
        ));
        after.push(frame_blocks(&rig.device, &rig.queue, &rig.frame, piece));
    }
    // A fresh mirror and frame of the edited world, the piece at the queue's settings.
    let cfg = MirrorConfig {
        budget_bytes: 512 << 20,
        light_radius: 4,
        ..MirrorConfig::new(Window::Fixed(world_extent))
    };
    let mut gpu = GpuWorld::with_config(&rig.device, cfg);
    gpu.set_focus(vxe_world::VoxelCoord(eye.map(|v| v as i32)));
    for _ in 0..8 {
        gpu.sync_now(&rig.world, &rig.device, &rig.queue)
            .expect("the mirror fits");
    }
    let mut fresh_frame = Frame::new(&rig.device, 16, 16);
    fresh_frame.set_lighting(Lighting {
        lamp: false,
        lamp_bounce: false,
        exposure: Exposure::Manual(8.0),
        light: Light::Baked,
        ..Lighting::default()
    });
    fresh_frame
        .bake_until(
            &rig.device,
            &rig.queue,
            &mut gpu,
            &rig.world,
            piece,
            &queue_settings,
            &mut |_| {},
        )
        .expect("the piece fits the pool");
    let fresh = frame_blocks(&rig.device, &rig.queue, &fresh_frame, piece);
    let touched = |c: [i32; 3], slot: u16| {
        let b = [
            i32::from(slot) % 8,
            i32::from(slot) / 8 % 8,
            i32::from(slot) / 64,
        ];
        (0..3).all(|k| {
            let lo = c[k] * 64 + b[k] * 8;
            lo + 7 >= dug[0][k] - 1 && lo <= dug[1][k] + 1
        })
    };
    let mut compare = |name: &str, got: &BTreeMap<[i32; 3], Vec<u32>>| {
        for part in ["touched", "untouched"] {
            let mut diffs = Vec::new();
            let (mut equal, mut one_step, mut dir_differ) = (0usize, 0usize, 0usize);
            for (c, slots) in &ring {
                let (Some(a), Some(f)) = (got.get(c), fresh.get(c)) else {
                    continue;
                };
                for &slot in slots {
                    if touched(*c, slot) != (part == "touched") {
                        continue;
                    }
                    let (da, df) = (a[usize::from(slot)], f[usize::from(slot)]);
                    if da == 0 || df == 0 {
                        continue;
                    }
                    if (da & 0x7ff) != (df & 0x7ff) {
                        dir_differ += 1;
                        continue;
                    }
                    let (ra, rf) = ((da >> 11) as usize, (df >> 11) as usize);
                    for d in 0..6 {
                        let (wa, wf) =
                            (a[ra - bake::BRICK_WORDS + d], f[rf - bake::BRICK_WORDS + d]);
                        if wa == wf {
                            equal += 1;
                        }
                        let (va, vf) = (bake::rgb9e5_unpack(wa), bake::rgb9e5_unpack(wf));
                        let step = |w: u32| 2f32.powi((w >> 27) as i32 - 24);
                        let ulp = step(wa).max(step(wf));
                        if (0..3).all(|k| (va[k] - vf[k]).abs() <= ulp * 1.0001) {
                            one_step += 1;
                        }
                        let l = |v: [f32; 3]| {
                            bake::tone_level(
                                0.2126 * f64::from(v[0])
                                    + 0.7152 * f64::from(v[1])
                                    + 0.0722 * f64::from(v[2]),
                            )
                        };
                        diffs.push((l(va) - l(vf)).abs());
                    }
                }
            }
            diffs.sort_by(f64::total_cmp);
            let n = diffs.len();
            if n == 0 {
                log.line(&format!("queue dig, {name}, {part}: nothing compared"));
                continue;
            }
            let at = |q: f64| diffs[((n as f64 * q) as usize).min(n - 1)];
            log.line(&format!(
                "queue dig, {name}, {part} bricks: {n} direction values ({dir_differ} bricks whose cell counts differ, skipped); equal words {:.4}, within one rgb9e5 step {:.4}, within 1 level {:.4}; |levels| p50 {:.3} p90 {:.3} p99 {:.3} max {:.3}",
                equal as f64 / n as f64,
                one_step as f64 / n as f64,
                diffs.iter().filter(|d| **d <= 1.0).count() as f64 / n as f64,
                at(0.5),
                at(0.9),
                at(0.99),
                diffs[n - 1]
            ));
        }
    };
    compare("before the dig", &before);
    for ((name, _), got) in arms.iter().zip(&after) {
        compare(name, got);
    }
}
