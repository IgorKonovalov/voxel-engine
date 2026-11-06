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
    let mut f = new_frame(&rig.device, [16, 16], Light::Baked);
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
        let e = f
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
        let opaque = bake::opaque_table(world.materials());
        let e = world.extent();
        let s = e.size();
        let (mut faces, mut words) = (Vec::new(), 0u64);
        for z in 0..s[2] {
            for y in 0..s[1] {
                for x in 0..s[0] {
                    let c = ChunkCoord([x, y, z]);
                    if let Some(f) = bake::list_world_chunk(&world, c, &opaque)
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
        let opaque = bake::opaque_table(world.materials());
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
                    if let Some(f) = bake::list_world_chunk(&world, ChunkCoord([x, y, z]), &opaque)
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
