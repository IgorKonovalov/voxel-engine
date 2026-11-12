//! The loaded light (ADR-0046, Plan 0059): a bake's brick records and the frames that read them, headless on
//! the software adapter, and the ignored readings on a named adapter (lane rule 9c): the live
//! queue's digs, the resolution cut and the budgets at rest.
//!
//! Every world here is a hand-built box of under 2000 faces at test counts: nothing bakes a whole
//! world in a test.

#![allow(
    clippy::disallowed_methods,
    reason = "the readings time their bakes on the wall clock"
)]

mod support;

use std::collections::{BTreeMap, HashMap};

use flythrough::cave::{ORE, ROCK};
use support::rigs::tone_bytes;
use support::{SyncNow, reading_device};
use vxe_render::bake::{self, BakeSettings};
use vxe_render::gpu_world::{GpuWorld, MirrorConfig, UNIT_BYTES, Window};
use vxe_render::marcher::LightingResolution;
use vxe_render::{Camera, Exposure, Frame, LightParams, Lighting, readback, wgpu};
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
/// probes off, exposure 8).
fn rig_on(world: World, size: [u32; 2], light_radius: Option<i32>) -> Option<Rig> {
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
    let frame = new_frame(&device, size);
    Some(Rig {
        device,
        queue,
        gpu,
        world,
        frame,
        cfg,
    })
}

fn new_frame(device: &wgpu::Device, size: [u32; 2]) -> Frame {
    let mut frame = Frame::new(device, size[0], size[1]);
    frame.set_jitter(device, false);
    frame.set_resolution(device, LightingResolution::Full);
    frame.set_lighting(Lighting {
        lamp: false,
        lamp_bounce: false,
        exposure: Exposure::Manual(8.0),
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

    /// Every held direction of the store's brick records by brick and face ([`cells_of`]).
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

    /// A fresh mirror and frame of the world as it is now, `piece` baked to `settings`: its records'
    /// directions ([`cells_of`]).
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
        let mut frame = new_frame(&self.device, [16, 16]);
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

/// The brick records of `blocks` (cells of 8 voxels, one a brick) by world brick and face, each
/// direction the record holds: its `rgb9e5` word and the record's tail word (`material | mask <<
/// 16`).
fn cells_of(blocks: &BTreeMap<[i32; 3], Vec<u32>>) -> HashMap<([i32; 3], u8), [u32; 2]> {
    let mut out = HashMap::new();
    for (&c, block) in blocks {
        for slot in 0..512usize {
            let dir = block[slot];
            if dir == 0 {
                continue;
            }
            let rec = &block[(dir >> 11) as usize - bake::BRICK_WORDS..(dir >> 11) as usize];
            let b = vxe_world::brick_of_slot(slot);
            let g = [0, 1, 2].map(|k| c[k] * 8 + b[k]);
            for f in 0..6u8 {
                if rec[6] >> (16 + f) & 1 != 0 {
                    out.insert((g, f), [rec[usize::from(f)], rec[6]]);
                }
            }
        }
    }
    out
}

/// The brick level's value of face `face` of voxel `v` (temporal.wgsl's `store_brick_value` at
/// cells of 8 voxels, emission scale 1): the 2 x 2 bricks around the voxel's centre in the face's
/// plane, bilinear over those that hold the direction; its own brick's alone where none has weight.
fn brick_value(
    cells: &HashMap<([i32; 3], u8), [u32; 2]>,
    v: [i32; 3],
    face: u8,
) -> Option<[f32; 3]> {
    let axis = usize::from(face / 2);
    let (ta, tb) = ((axis + 1) % 3, (axis + 2) % 3);
    let own = v.map(|x| x.div_euclid(8));
    let g = v.map(|x| (x as f32 + 0.5) / 8.0 - 0.5);
    let lo = g.map(|x| x.floor() as i32);
    let f = [0, 1, 2].map(|k| g[k] - g[k].floor());
    let mut sum = [0f32; 3];
    let mut weight = 0f32;
    for i in 0..4 {
        let (oa, ob) = (i & 1, i >> 1);
        let mut b = own;
        b[ta] = lo[ta] + oa;
        b[tb] = lo[tb] + ob;
        let wa = if oa == 1 { f[ta] } else { 1.0 - f[ta] };
        let wb = if ob == 1 { f[tb] } else { 1.0 - f[tb] };
        if let Some(c) = cells.get(&(b, face)) {
            let x = bake::rgb9e5_unpack(c[0]);
            for k in 0..3 {
                sum[k] += x[k] * (wa * wb);
            }
            weight += wa * wb;
        }
    }
    if weight > 0.0 {
        return Some(sum.map(|x| x / weight));
    }
    cells.get(&(own, face)).map(|c| bake::rgb9e5_unpack(c[0]))
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

/// The brick level at `samples` a brick direction, rounds of 64.
fn test_counts(samples: u32) -> BakeSettings {
    BakeSettings {
        round: 64,
        brick_samples: samples,
        ..BakeSettings::default()
    }
}

/// The records depend on each face's own samples alone: one batch, batches of 7 faces and the
/// jobs reversed read bit-identical; and [`four_ways`].
#[test]
#[ignore = "soak"]
fn a_bake_does_not_depend_on_its_slicing_or_order() {
    let (world, _) = small_box();
    let Some(mut rig) = rig_on(world, [16, 16], None) else {
        return;
    };
    let one = BakeSettings {
        batch_faces: 65_536,
        ..test_counts(1024)
    };
    let report = rig.bake(&one);
    assert!(report.faces > 1000, "{} faces", report.faces);
    assert!(report.faces < 2000, "{} faces", report.faces);
    let first = rig.blocks();
    let held = cells_of(&first);
    let lit = held.values().filter(|c| c[0] != 0).count();
    println!("{} held directions, {lit} lit", held.len());
    // The rock's four top bricks face the ore plate: their +z directions are lit.
    for (x, y) in [(5, 5), (5, 6), (6, 5), (6, 6)] {
        let up = held.get(&([x, y, 6], 5)).map_or(0, |c| c[0]);
        assert_ne!(up, 0, "brick ({x}, {y}, 6) +z unlit");
    }
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
    four_ways(&mut rig);
}

/// Bakes `rig` at 4096 samples a brick direction, rounds of 64, four ways: several rounds a
/// submit with every job in one submit, one round a submit, each job alone, and each job alone at
/// one round a submit; asserts bit-identical records and samples a face.
fn four_ways(rig: &mut Rig) {
    let stop = BakeSettings {
        keep_faces: true,
        ..test_counts(4096)
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
    assert!(counts.len() > 2, "faces of different shares: {counts:?}");
    assert!(
        several.max_rounds_in_submit > 1,
        "several rounds a submit: {}",
        several.max_rounds_in_submit
    );
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
    assert!(alone.max_rounds_in_submit > 1);
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

/// After the bake the first frame is the final one: frame 1 and frame 64 are bit-identical, its
/// emissive part is the brick level's value ([`brick_value`]) times the exposure within one half
/// step, the light pass drew nothing, and no face misses the store.
#[test]
fn a_baked_view_is_final_on_its_first_frame() {
    let (world, camera) = small_box();
    let Some(mut rig) = rig_on(world, [64, 36], None) else {
        return;
    };
    rig.bake(&test_counts(1024));
    let cells = rig.cells();
    rig.frame.set_count_store(true);
    rig.step(&camera);
    let first = rig.display();
    let accum = readback::read_float(&rig.device, &rig.queue, rig.frame.accumulated());
    let pos = readback::read_float(&rig.device, &rig.queue, rig.frame.hit_positions());
    let info = readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    let light = readback::read_float(&rig.device, &rig.queue, rig.frame.irradiance());
    let stats = rig.frame.read_store_stats(&rig.device, &rig.queue);
    assert_eq!(stats.missing, 0, "store_missing");
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
        let value =
            brick_value(&cells, key.0, key.1).expect("the face's brick holds its direction");
        let want = value.map(|v| v * 8.0);
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
        lit += usize::from(value.iter().any(|&v| v > 0.0));
    }
    assert!(checked > 200, "{checked} pixels with a face");
    assert!(lit > 100, "{lit} lit pixels");
    for _ in 1..64 {
        rig.step(&camera);
    }
    assert_eq!(rig.display(), first, "frame 64 against frame 1");
}

/// The store's read counters on a fixed frame: a box of two chunks whose emitter lies in the
/// one-chunk piece that is baked, seen from above, so its cube-face hits split between brick reads
/// (the piece) and no value (the chunk outside, no block). With the counting pipeline
/// (`Frame::set_count_store`) the three counts equal the ones the temporal pass wrote into the
/// store's own words before the counters had a buffer of their own (read at 486a53e through the
/// same `read_store_stats`); with it off the temporal pass writes nothing, so the counter buffer,
/// cleared at the frame's start, still reads zero after it.
#[test]
fn the_store_counters_count_with_the_override_on() {
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
    let Some(mut rig) = rig_on(world, [64, 36], None) else {
        return;
    };
    rig.frame
        .bake_until(
            &rig.device,
            &rig.queue,
            &mut rig.gpu,
            &rig.world,
            ChunkExtent::new([1, 0, 0], [2, 1, 1]),
            &dig_counts(),
            &mut |_| {},
        )
        .expect("the piece fits");
    let camera = Camera {
        position: [64.0, 32.0, 40.0].into(),
        yaw: 0.0,
        pitch: (-80f32).to_radians(),
        vfov: 90f32.to_radians(),
    };
    let counts = |s: vxe_render::StoreStats| [s.missing, s.brick_reads, s.coarse_reads];
    rig.frame.set_count_store(true);
    rig.step(&camera);
    let on = counts(rig.frame.read_store_stats(&rig.device, &rig.queue));
    println!("counting: missing, brick reads, coarse reads {on:?}");
    assert_eq!(on, [366, 658, 0], "the counts against the start commit's");
    rig.frame.set_count_store(false);
    rig.step(&camera);
    let off = counts(rig.frame.read_store_stats(&rig.device, &rig.queue));
    assert_eq!(off, [0; 3], "the counters with the override off");
}

/// ADR-0053's read chain on the GPU. Chunks A1 (0,0,0) and A2 (0,0,1) are baked, each with its
/// own ore emitter over a rock floor (A2's larger), inside level-1 cell (0,0,0); B (1,0,0) is
/// face-adjacent to A1 alone and C (1,1,0) to neither, both blockless floors in the same cell; D
/// (4,0,0) is a blockless floor in a level-2 cell with no block. Seen from above, lamp off, each
/// floor's top face (+z) reads: under `chain` B's stored term A1's summary within one `rgb9e5`
/// step, C's the mean of A1's and A2's (their level-1 record), D's 0; under `parent` B reads that
/// mean; under `neighbour` C reads 0; under `black` B, C and D read 0. Every pixel with no brick
/// value reads the CPU's chain (`LightSummaries::chain`) for its chunk and face; `store_missing`
/// is the same in every mode, and the fallback and still-black counters split it as the chain
/// does (under `chain` B's and C's lit faces are fallback, D's still black). The pool's summary
/// words are the CPU's.
#[test]
fn a_missing_brick_reads_its_neighbour_then_its_parent() {
    use vxe_render::bake::MissingLight;
    // z is up. Four chunks tall, so the eye above the floors lies inside the mirrored world.
    let world = air_world([5, 2, 4], &table());
    // Floors 4 voxels thick, their top faces at z 8 (layer 0) or 72 (layer 1), inset from the
    // chunk's sides so each floor is its own chunk's.
    let floor = |c: [i64; 3]| {
        let o = c.map(|v| v * 64);
        fill(
            &world,
            [o[0] + 4, o[1] + 4, o[2] + 4],
            [o[0] + 60, o[1] + 60, o[2] + 8],
            ROCK,
        );
    };
    for c in [[0, 0, 0], [0, 0, 1], [1, 0, 0], [1, 1, 0], [4, 0, 0]] {
        floor(c);
    }
    fill(&world, [28, 28, 20], [36, 36, 22], ORE);
    fill(&world, [20, 20, 84], [44, 44, 88], ORE);
    let Some(mut rig) = rig_on(world, [128, 72], None) else {
        return;
    };
    rig.frame
        .bake_until(
            &rig.device,
            &rig.queue,
            &mut rig.gpu,
            &rig.world,
            ChunkExtent::new([0, 0, 0], [1, 1, 2]),
            &dig_counts(),
            &mut |_| {},
        )
        .expect("the piece fits");
    let light = rig.frame.store().light().clone();
    let (a1, a2) = (
        *light.summary([0, 0, 0]).expect("A1 is lit"),
        *light.summary([0, 0, 1]).expect("A2 is lit"),
    );
    // The floors' top faces, +z.
    let top = 5;
    assert_ne!(
        a1[top], a2[top],
        "the two emitters light their floors apart"
    );
    let p1 = *light
        .parent(1, [0, 0, 0])
        .expect("A1 and A2's level-1 cell");
    for (c, s) in [([0, 0, 0], a1), ([0, 0, 1], a2)] {
        let block = rig
            .frame
            .read_store_block(&rig.device, &rig.queue, ChunkCoord(c))
            .expect("a baked block");
        let at = bake::SUMMARY_AT as usize;
        assert_eq!(block[at..at + bake::BRICK_WORDS], s, "{c:?}'s pool summary");
    }
    for c in [[1, 0, 0], [1, 1, 0], [4, 0, 0]] {
        assert!(
            rig.frame.store().block(ChunkCoord(c)).is_none(),
            "{c:?} is blockless"
        );
    }
    let cells = rig.cells();
    let camera = Camera {
        position: [160.0, 64.0, 220.0].into(),
        yaw: 0.0,
        pitch: (-85f32).to_radians(),
        vfov: 90f32.to_radians(),
    };
    let eye = camera.position.to_array();
    rig.frame.set_count_store(true);
    let mut missing = None;
    for mode in MissingLight::ALL {
        rig.frame.set_missing_light(&rig.queue, mode);
        rig.step(&camera);
        let accum = rig.accum();
        let pos = readback::read_float(&rig.device, &rig.queue, rig.frame.hit_positions());
        let info = readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
        let stats = rig.frame.read_store_stats(&rig.device, &rig.queue);
        assert_eq!(
            *missing.get_or_insert(stats.missing),
            stats.missing,
            "{mode:?}: store_missing"
        );
        let (mut fallback, mut black) = (0u32, 0u32);
        // Lit +y pixels of B, C and D, and the value each read.
        let mut tops: HashMap<[i32; 3], Vec<[f32; 3]>> = HashMap::new();
        for i in 0..pos.len() {
            let Some((v, f)) = hit_face(eye, pos[i], info[i]) else {
                continue;
            };
            if brick_value(&cells, v, f).is_some() {
                continue;
            }
            let c = v.map(|x| x.div_euclid(64));
            let want = light
                .chain(c, mode)
                .filter(|r| r[6] >> (16 + u32::from(f)) & 1 != 0)
                .map_or([0.0; 3], |r| bake::rgb9e5_unpack(r[usize::from(f)]));
            if want.iter().any(|&x| x > 0.0) {
                fallback += 1;
            } else {
                black += 1;
            }
            let got = [accum[i][0], accum[i][1], accum[i][2]];
            for k in 0..3 {
                let w = want[k] * 8.0;
                let step = (w.abs() / 1024.0).max(1.0 / 16384.0);
                assert!(
                    (got[k] - w).abs() <= step,
                    "{mode:?}: pixel {i} of chunk {c:?} face {f}, channel {k}: {} against {w}",
                    got[k]
                );
            }
            if usize::from(f) == top {
                tops.entry(c).or_default().push(got.map(|x| x / 8.0));
            }
        }
        assert_eq!(
            [stats.fallback, stats.still_black],
            [fallback, black],
            "{mode:?}: the fallback and still-black counters"
        );
        assert_eq!(
            fallback + black,
            stats.missing,
            "{mode:?}: they split the missing"
        );
        let reads = |c: [i32; 3]| -> &Vec<[f32; 3]> {
            let t = tops.get(&c).unwrap_or_else(|| {
                let seen: BTreeMap<_, _> = tops.iter().map(|(k, v)| (*k, v.len())).collect();
                panic!("{mode:?}: no top pixel of {c:?}; seen {seen:?}")
            });
            assert!(t.len() > 20, "{mode:?}: {} top pixels of {c:?}", t.len());
            t
        };
        let all_near = |c: [i32; 3], w: u32| {
            let want = bake::rgb9e5_unpack(w);
            let step = 2f32.powi((w >> 27) as i32 - 24);
            reads(c)
                .iter()
                .all(|g| (0..3).all(|k| (g[k] - want[k]).abs() <= step * 1.01 + want[k] / 1024.0))
        };
        let all_zero = |c: [i32; 3]| reads(c).iter().all(|g| *g == [0.0; 3]);
        match mode {
            MissingLight::Chain => {
                assert!(all_near([1, 0, 0], a1[top]), "chain: B reads A1's summary");
                assert!(
                    all_near([1, 1, 0], p1[top]),
                    "chain: C reads A1's and A2's mean"
                );
                assert!(all_zero([4, 0, 0]), "chain: D reads 0");
            }
            MissingLight::Parent => {
                assert!(all_near([1, 0, 0], p1[top]), "parent: B reads the mean");
                assert!(all_zero([4, 0, 0]), "parent: D reads 0");
            }
            MissingLight::Neighbour => {
                assert!(
                    all_near([1, 0, 0], a1[top]),
                    "neighbour: B reads A1's summary"
                );
                assert!(all_zero([1, 1, 0]), "neighbour: C reads 0");
            }
            MissingLight::Black => {
                for c in [[1, 0, 0], [1, 1, 0], [4, 0, 0]] {
                    assert!(all_zero(c), "black: {c:?} reads 0");
                }
                assert_eq!(stats.fallback, 0, "black: no fallback");
            }
        }
        println!(
            "{mode:?}: missing {}, fallback {}, still black {}",
            stats.missing, stats.fallback, stats.still_black
        );
    }
}

/// The baked light with the filter off (`Lighting::filter`, the fly-through's `--filter off`): on
/// the small box after its bake, lamp off, the mean display luma over the pixels whose cube face
/// holds a lit brick value is within 2 % of the same frame's with the filter on.
#[test]
fn the_baked_light_shows_with_the_filter_off() {
    let (world, camera) = small_box();
    let Some(mut rig) = rig_on(world, [64, 36], None) else {
        return;
    };
    rig.bake(&test_counts(1024));
    let cells = rig.cells();
    let frames = |rig: &mut Rig, filter: bool| {
        rig.frame.set_lighting(Lighting {
            filter,
            ..rig.frame.lighting()
        });
        rig.frame.reset_history();
        for _ in 0..4 {
            rig.step(&camera);
        }
        rig.display()
    };
    let on = frames(&mut rig, true);
    let pos = readback::read_float(&rig.device, &rig.queue, rig.frame.hit_positions());
    let info = readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    let off = frames(&mut rig, false);
    let eye = camera.position.to_array();
    let lit: Vec<usize> = (0..pos.len())
        .filter(|&i| {
            hit_face(eye, pos[i], info[i])
                .and_then(|(v, f)| brick_value(&cells, v, f))
                .is_some_and(|value| value.iter().any(|&c| c > 0.0))
        })
        .collect();
    let mean = |img: &[u8]| {
        let l = luma(img);
        lit.iter().map(|&i| f64::from(l[i])).sum::<f64>() / lit.len() as f64
    };
    let (m_on, m_off) = (mean(&on), mean(&off));
    println!(
        "{} lit pixels: mean luma {m_on:.3} filter on, {m_off:.3} off",
        lit.len()
    );
    assert!(lit.len() > 100, "{} lit pixels", lit.len());
    assert!(m_on > 10.0, "the lit face's luma {m_on}");
    assert!(
        (m_off - m_on).abs() <= 0.02 * m_on,
        "filter off {m_off} against on {m_on}"
    );
}

/// The owner's "moving fast" as a property: a frame reached by a 90 degree yaw in 2 frames reads
/// what a fresh frame at that pose reads, on every pixel with a face.
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
    let Some(mut rig) = rig_on(world, [64, 36], None) else {
        return;
    };
    rig.bake(&test_counts(1024));
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

    let mut fresh = new_frame(&rig.device, [64, 36]);
    let extent = rig.world.extent();
    fresh
        .bake_until(
            &rig.device,
            &rig.queue,
            &mut rig.gpu,
            &rig.world,
            extent,
            &test_counts(1024),
            &mut |_| {},
        )
        .expect("the box fits the pool");
    std::mem::swap(&mut rig.frame, &mut fresh);
    rig.step(&turned);
    let still = rig.display();
    let still = faced(&rig, &turned, &still);
    assert_eq!(moved, still, "the moved frame against the fresh one");
}

// ---- The dig, the scale, the lamp, the file and the piece (Phase 2) ----

/// A dig's settings beside a bake's: the test counts, the dig's `n_max` the same.
fn dig_counts() -> BakeSettings {
    test_counts(1024)
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

/// The directions of `cells` ([`cells_of`]) whose brick is one of `ring`.
fn in_ring(
    cells: &HashMap<([i32; 3], u8), [u32; 2]>,
    ring: &BTreeMap<[i32; 3], Vec<u16>>,
) -> BTreeMap<([i32; 3], u8), [u32; 2]> {
    cells
        .iter()
        .filter(|((g, _), _)| {
            let c = g.map(|x| x.div_euclid(8));
            let b = g.map(|x| x.rem_euclid(8));
            ring.get(&c)
                .is_some_and(|s| s.contains(&(vxe_world::brick_slot(b) as u16)))
        })
        .map(|(k, v)| (*k, *v))
        .collect()
}

/// A lit wall, a radius 3 carve: on the first frame the hole is on screen, every record of the
/// ring is bit-identical to a fresh bake of the edited world (the same counts), every record
/// beyond it is as before, and that frame and frame +64 are bit-identical.
#[test]
fn a_dug_hole_is_final_on_its_first_frame() {
    let (world, camera) = small_box();
    let Some(mut rig) = rig_on(world, [64, 36], None) else {
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
    println!("{} directions in the ring of {}", a.len(), after.len());
    // The ring reaches every brick of the box: its 12 bricks hold over 40 directions.
    assert!(a.len() > 40, "{} directions in the ring", a.len());
    assert_eq!(a, f, "the ring against a fresh bake");
    for (k, v) in &after {
        if !a.contains_key(k) {
            assert_eq!(before.get(k), Some(v), "{k:?} beyond the ring moved");
        }
    }
    // The hole's new faces: directions of bricks within the carve's reach that held none before.
    let new: Vec<_> = after.keys().filter(|k| !before.contains_key(k)).collect();
    assert!(!new.is_empty(), "the carve exposed no face");
    rig.frame.set_count_store(true);
    rig.step(&camera);
    let first = rig.display();
    let pos = readback::read_float(&rig.device, &rig.queue, rig.frame.hit_positions());
    let info = readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    let eye = camera.position.to_array();
    let on_screen = (0..pos.len())
        .filter_map(|i| hit_face(eye, pos[i], info[i]))
        .filter(|(v, f)| !before.contains_key(&(v.map(|x| x.div_euclid(8)), *f)))
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

/// Carving the box's one emitter brings every record of the ring to exactly 0 on that frame,
/// bit-identical to a fresh bake.
#[test]
fn a_dug_emitter_leaves_the_near_ring_at_once() {
    let world = air_world([1, 1, 1], &table());
    fill(&world, [8, 8, 20], [40, 40, 21], ROCK);
    fill(&world, [22, 22, 30], [24, 24, 32], ORE);
    let Some(mut rig) = rig_on(world, [16, 16], None) else {
        return;
    };
    rig.bake(&dig_counts());
    let lit = rig.cells().values().filter(|c| c[0] != 0).count();
    println!("{lit} lit directions before");
    // The slab's 16 bricks face the ore with their +z.
    assert!(lit >= 16, "{lit} lit directions before");
    let (_, aabb) = rig.dig(
        &box_op([22, 22, 30], [24, 24, 32], Mode::Carve),
        &dig_counts(),
    );
    let piece = rig.world.extent();
    let ring = bake::dig_ring(&[aabb], piece);
    let after = in_ring(&rig.cells(), &ring);
    println!("{} directions in the ring", after.len());
    // The slab's 16 bricks, each with its +z and -z.
    assert!(after.len() >= 32, "{} directions in the ring", after.len());
    let lit: Vec<_> = after.iter().filter(|(_, c)| c[0] != 0).collect();
    assert!(
        lit.is_empty(),
        "{} directions still lit, first {:?}",
        lit.len(),
        lit.first()
    );
    let fresh = in_ring(&rig.fresh_cells(piece, &dig_counts()), &ring);
    assert_eq!(after, fresh, "the ring against a fresh bake");
}

/// `Frame::set_emission_scales` at 0.5 halves a lit wall's displayed irradiance on that frame,
/// within one `rgb9e5` step, the records unchanged; a material at 0 when baked reads its light
/// when raised.
#[test]
fn a_scale_change_moves_no_bake() {
    let (world, camera) = small_box();
    let Some(mut rig) = rig_on(world, [64, 36], None) else {
        return;
    };
    rig.frame.set_emission_scale(&rig.queue, ORE, 0.0);
    rig.bake(&test_counts(1024));
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
    assert_eq!(rig.cells(), cells, "the records after the scale changes");
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

/// With the lamp on a frame's emissive part equals the lamp-off frame's and the records are
/// unchanged; off again, the frame is bit-identical to the first lamp-off frame once the lamp's
/// history has emptied, [`LAMP_EMPTY_FRAMES`] frames after the lamp goes off.
#[test]
fn the_lamp_adds_over_the_bake() {
    let (world, camera) = small_box();
    let Some(mut rig) = rig_on(world, [64, 36], None) else {
        return;
    };
    rig.bake(&test_counts(1024));
    let cells = rig.cells();
    rig.step(&camera);
    let off = rig.display();
    let off_accum = rig.accum();
    let lighting = |lamp: bool| Lighting {
        lamp,
        lamp_bounce: false,
        exposure: Exposure::Manual(8.0),
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
    assert_eq!(rig.cells(), cells, "the records under the lamp");
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
    let Some(mut rig) = rig_on(world, [64, 36], None) else {
        return;
    };
    let settings = test_counts(1024);
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
/// emitter sees the pane's camera-side faces, so their records are 0 and each pixel is `T(L)` of
/// what it showed before the pane: `L` the exact path through the glass boxes along the pixel's
/// ray, `T = tint^L` per channel, `tint = 0.8 (0.5 + 0.5 albedo / max(albedo))` from the row's
/// albedo. Within 2 levels, head on (the halves at `L` near 1 and 2: `T` and `T^2`) and at 30
/// degrees of yaw (`L` near 2.3 through the 2-voxel half). Then a pane lit from the camera's side
/// over a black wall: each pixel's reflected light before exposure (the layer target's rgb) is
/// Schlick's `R` at F0 0.04 at its angle times `E / PI`, `E` the brick level's value of its entry
/// face ([`brick_value`] of the store's records, times the ore's emission scale), above 0 and
/// within 10 %, and the pixel shows it. With the ore strip carved away (its ring re-baked), the
/// same pixels reflect 0. The per-layer composite fails the first.
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
    let Some(mut rig) = rig_on(world, size, None) else {
        return;
    };
    let settings = test_counts(1024);
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
    // The lit pane over the black wall: its reflected share from its +x face's brick record.
    settle(&mut rig, &lit_view);
    let shown = luma(&rig.display());
    let info = readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    let pos = readback::read_float(&rig.device, &rig.queue, rig.frame.hit_positions());
    let layer = readback::read_float(&rig.device, &rig.queue, rig.frame.layers());
    let cells = rig.cells();
    let scale = rig.frame.emission_scale(ORE);
    let eye = lit_view.position.to_array().map(f64::from);
    let lum = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let (mut lit, mut dark, mut worst, mut worst_shown) = (Vec::new(), 0usize, 0f32, 0f32);
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
        let Some(e) = brick_value(&cells, v, 1) else {
            panic!("no record at the pane's face {v:?}");
        };
        let c = (1.0 - d[0].abs()) as f32;
        let r = 0.04 + 0.96 * c.powi(5);
        let want = e.map(|x| r * x * scale / std::f32::consts::PI);
        let got = [layer[i][0], layer[i][1], layer[i][2]];
        if lum(want) <= 0.0 {
            dark += 1;
            continue;
        }
        lit.push(i);
        worst = worst.max((lum(got) / lum(want) - 1.0).abs());
        worst_shown = worst_shown.max((shown[i] - level(got.map(|x| x * ex))).abs());
    }
    println!(
        "lit pane: {} pixels over the black wall, {dark} with E 0; worst |reflected / (R E / PI) - 1| {worst:.4}; worst |pixel - display(reflected)| {worst_shown:.2} levels",
        lit.len()
    );
    assert!(lit.len() > 1000, "{} lit pane pixels", lit.len());
    assert_eq!(dark, 0, "pane pixels whose entry face's record reads 0");
    assert!(worst <= 0.10, "reflected light {worst:.4} off R E / PI");
    assert!(
        worst_shown <= 2.0,
        "a pixel {worst_shown:.2} levels off its reflected light"
    );
    // The control: no emitter sees the pane, so the same pixels reflect nothing.
    rig.dig(
        &EditOp::box_(
            Fixed3([100, 56, 114].map(|v| v * FIXED_ONE)),
            Fixed3([104, 84, 116].map(|v| v * FIXED_ONE)),
            Mode::Carve,
        ),
        &settings,
    );
    settle(&mut rig, &lit_view);
    let layer = readback::read_float(&rig.device, &rig.queue, rig.frame.layers());
    let unlit = lit
        .iter()
        .filter(|&&i| layer[i][0] != 0.0 || layer[i][1] != 0.0 || layer[i][2] != 0.0)
        .count();
    println!("no emitter: {unlit} of {} pane pixels reflect", lit.len());
    assert_eq!(unlit, 0, "pane pixels reflecting with no emitter");
}

/// A box of two chunks whose emitter lies outside a one-chunk piece: the piece's records are a
/// whole bake's, the chunk outside has no block and its hits count in `store_missing`; a carve
/// that straddles the piece's edge re-bakes only records inside it, bit-identical to a fresh piece bake
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
    let Some(mut rig) = rig_on(world, [64, 36], None) else {
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
    let inside: BTreeMap<_, _> = whole.iter().filter(|((g, _), _)| g[0] >= 8).collect();
    let lit = cells.values().filter(|c| c[0] != 0).count();
    println!("{lit} lit directions in the piece");
    // The piece's 12 slab bricks face the ore with their +z.
    assert!(lit >= 12, "{lit} lit directions in the piece");
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
    rig.frame.set_count_store(true);
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

// ---- The readings' views ----

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

// ---- The readings on a named adapter ----

/// The window's native size (the owner's 2530 x 1378).
const NATIVE: [u32; 2] = [2530, 1378];

/// A view's piece: its name, the piece's centre eye and its radius (`None`: the whole world).
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
/// lamp off, exposure 8, an empty store: the rig and the piece.
fn view_rig(view: &ReadView, size: [u32; 2]) -> (ReadRig, ChunkExtent) {
    let world = reference_world(view.scene);
    let pool = flythrough::level_pool_units(&world, false) * UNIT_BYTES;
    let (device, queue, adapter) = reading_device(pool).expect("a device");
    let cfg = MirrorConfig {
        budget_bytes: 512 << 20,
        light_radius: if view.scene == Scene::Showcase { 19 } else { 4 },
        ..MirrorConfig::new(Window::Fixed(world.extent()))
    };
    let mut gpu = GpuWorld::with_config(&device, cfg);
    let (_, centre_eye, radius) = file_of(view);
    gpu.set_focus(vxe_world::VoxelCoord(view.eye.map(|v| v as i32)));
    for _ in 0..8 {
        gpu.sync_now(&world, &device, &queue)
            .expect("the mirror fits the adapter");
    }
    let mut frame = Frame::new(&device, size[0], size[1]);
    frame.set_jitter(&device, false);
    frame.set_resolution(&device, LightingResolution::Full);
    frame.set_lighting(reference_lighting(false));
    frame.set_dt(1.0 / 60.0);
    let piece = piece_of(world.extent(), centre_eye, radius);
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

fn reference_lighting(lamp: bool) -> Lighting {
    Lighting {
        lamp,
        lamp_bounce: false,
        exposure: Exposure::Manual(8.0),
        ..Lighting::default()
    }
}

fn inside(piece: ChunkExtent, v: [i32; 3]) -> bool {
    let s = piece.size();
    (0..3).all(|k| {
        let c = v[k].div_euclid(64);
        c >= piece.min.0[k] && c < piece.min.0[k] + s[k]
    })
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

// Plan 0059 Phase 4: the samples cut, the shared seeds and the filter, read against the
// reference's files.

/// Bakes the brick store within 12 chunks of a view's centre (`file_of`'s eye), the default
/// brick level (8/4096), 512 a face's round: the milliseconds it took.
fn brick_bake(rig: &mut ReadRig, view: &ReadView) -> f64 {
    let (_, centre_eye, _) = file_of(view);
    let centre = centre_eye.map(|v| (v / 64.0).floor() as i32);
    let settings = BakeSettings {
        round: 512,
        ..BakeSettings::default()
    };
    let world = rig.world.extent();
    let size = world.size();
    let lo = [0, 1, 2].map(|k| (centre[k] - 12).max(world.min.0[k]));
    let hi = [0, 1, 2].map(|k| (centre[k] + 13).min(world.min.0[k] + size[k]));
    let extent = ChunkExtent::new(lo, [0, 1, 2].map(|k| hi[k].max(lo[k])));
    let t = std::time::Instant::now();
    rig.frame
        .bake_until(
            &rig.device,
            &rig.queue,
            &mut rig.gpu,
            &rig.world,
            extent,
            &settings,
            &mut |_| {},
        )
        .expect("the extent fits the pool");
    t.elapsed().as_secs_f64() * 1e3
}

/// Frames of a yaw's read, as the settle rig's fast yaw.
const YAW_DOT_FRAMES: usize = 60;

// ---- ADR-0049: the store read by the far normal and the filters weighed by face ----

/// A 45-degree staircase of 1-voxel steps rising along +x over chunks `[0, 5) x [0, 1) x
/// [0, 5)`: column `x` in 64..320 holds rock up to `z = x - 64`, its tread facing +z and its
/// riser -x, and an ore plate at z 300 over it lights the treads more than the risers. The
/// camera: [`stair_cameras`].
fn staircase() -> World {
    let world = air_world([5, 1, 5], &table());
    for x in 64..320i64 {
        fill(&world, [x, 0, 0], [x + 1, 64, x - 63], ROCK);
    }
    fill(&world, [64, 0, 300], [320, 64, 304], ORE);
    world
}

/// The staircase's two cameras at `size` (70 degrees horizontal): far, 194 voxels above the
/// slope at x 70 looking down 10 degrees along +x, whose rays descend so they see treads and
/// risers, from about 125 to 245 voxels away; near, 24 voxels above it at x 100 looking up
/// 25 degrees, whose hits start about 25 voxels away (risers only past the slope's angle).
fn stair_cameras(size: [u32; 2]) -> [Camera; 2] {
    let vfov = vfov_of(size);
    [
        Camera {
            position: [70.5, 32.5, 200.5].into(),
            yaw: 0.0,
            pitch: (-10f32).to_radians(),
            vfov,
        },
        Camera {
            position: [100.5, 32.5, 60.5].into(),
            yaw: 0.0,
            pitch: 25f32.to_radians(),
            vfov,
        },
    ]
}

/// The vertical field of view of a horizontal 70 degrees at `size`.
fn vfov_of(size: [u32; 2]) -> f32 {
    2.0 * (35f32.to_radians().tan() * size[1] as f32 / size[0] as f32).atan()
}

/// One baked frame of the staircase per arm: (store read, far normal, look, weights,
/// resolution) and its read-backs.
struct StairFrame {
    accum: Vec<[f32; 4]>,
    pos: Vec<[f32; 4]>,
    info: Vec<[u16; 4]>,
    display: Vec<u8>,
}

struct StairArm {
    read: vxe_render::StoreRead,
    weights: vxe_render::LightWeights,
    far_normal: [f32; 2],
    mode: vxe_render::NormalMode,
    resolution: LightingResolution,
}

fn stair_frame(rig: &mut Rig, camera: &Camera, arm: &StairArm) -> StairFrame {
    rig.frame.set_resolution(&rig.device, arm.resolution);
    rig.frame.set_lighting(Lighting {
        lamp: false,
        lamp_bounce: false,
        exposure: Exposure::Manual(8.0),
        bake_pool_bytes: 64 << 20,
        store_read: arm.read,
        light_weights: arm.weights,
        ..Lighting::default()
    });
    rig.frame.set_look(vxe_render::LookParams {
        normal_mode: arm.mode,
        far_normal: arm.far_normal,
        ..vxe_render::LookParams::default()
    });
    // Two frames: the first after a setting change carries no history to differ by.
    rig.step(camera);
    rig.step(camera);
    StairFrame {
        accum: readback::read_float(&rig.device, &rig.queue, rig.frame.accumulated()),
        pos: readback::read_float(&rig.device, &rig.queue, rig.frame.hit_positions()),
        info: readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records()),
        display: rig.display(),
    }
}

/// The stored part ADR-0049's read gives a hit of normal `n` on face `face` of voxel `v`:
/// the brick level's direction values the normal leans toward, mixed by `n_a^2` over those
/// held, renormalised; the face's own where none is.
fn by_normal(
    cells: &HashMap<([i32; 3], u8), [u32; 2]>,
    v: [i32; 3],
    face: u8,
    n: [f32; 3],
) -> Option<[f32; 3]> {
    let own = brick_value(cells, v, face);
    let mut sum = [0f32; 3];
    let mut weight = 0f32;
    for (a, &na) in n.iter().enumerate() {
        let w = na * na;
        if w < 1e-4 {
            continue;
        }
        let dir = 2 * a as u8 + u8::from(na > 0.0);
        if let Some(r) = if dir == face {
            own
        } else {
            brick_value(cells, v, dir)
        } {
            for k in 0..3 {
                sum[k] += r[k] * w;
            }
            weight += w;
        }
    }
    if weight > 0.0 {
        Some(sum.map(|x| x / weight))
    } else {
        own
    }
}

fn within(a: [f32; 3], b: [f32; 3], rel: f32) -> bool {
    (0..3).all(|k| (a[k] - b[k]).abs() <= rel * b[k].abs().max(1e-3))
}

fn rgb(p: [f32; 4]) -> [f32; 3] {
    [p[0], p[1], p[2]].map(|x| x / 8.0)
}

/// The staircase at 96 x 54 (Plan 0061 Phase 2 done-when 1b, at Phase 5's defaults): the
/// fly-through's far normal (`LookPreset::DEFAULT`, `[2, 6]`) gives a voxel the far normal's
/// full weight where it covers at most 2 pixels, `t >= 96 / (2 tan 35 deg) / 2 = 34.3` voxels;
/// every rock riser or tread pixel the far camera hits past `t = 36` (all of them: its hits start
/// about 125 voxels away) reads, with the fly-through's store read and weights at full light
/// resolution, ADR-0049's mix within 1 % (the CPU's `by_normal` over the bake's records), and
/// with the library's (`Face`, `Normal`) its own face's value within 1 %, risers and treads
/// both occurring. The frame at the fly-through's defaults (half light resolution) matches
/// `far_staircase_store_normal.png` at the harness's floor (at most 0.1 % of pixels over
/// 2/255; lavapipe).
#[test]
fn a_small_far_staircase_reads_the_mix_and_matches_its_golden() {
    use vxe_render::{LightWeights, NormalMode, StoreRead};
    const SIZE: [u32; 2] = [96, 54];
    const LINE: f32 = 36.0;
    let [camera, _] = stair_cameras(SIZE);
    let Some(mut rig) = rig_on(staircase(), SIZE, None) else {
        return;
    };
    rig.frame.set_lighting(Lighting {
        lamp: false,
        lamp_bounce: false,
        exposure: Exposure::Manual(8.0),
        bake_pool_bytes: 64 << 20,
        ..Lighting::default()
    });
    rig.bake(&test_counts(256));
    let cells = rig.cells();
    let eye = camera.position.to_array();
    let far_normal = flythrough::LookPreset::DEFAULT.far_normal;
    let arm = |read, weights, resolution| StairArm {
        read,
        weights,
        far_normal,
        mode: NormalMode::Face,
        resolution,
    };
    let on = stair_frame(
        &mut rig,
        &camera,
        &arm(
            flythrough::DEFAULT_STORE_READ,
            flythrough::DEFAULT_LIGHT_WEIGHTS,
            LightingResolution::Full,
        ),
    );
    let off = stair_frame(
        &mut rig,
        &camera,
        &arm(
            StoreRead::Face,
            LightWeights::Normal,
            LightingResolution::Full,
        ),
    );
    let (mut checked, mut bad_on, mut bad_off, mut risers, mut treads) = (0, 0, 0, 0, 0);
    for i in 0..on.pos.len() {
        let Some((v, face)) = hit_face(eye, on.pos[i], on.info[i]) else {
            continue;
        };
        if on.info[i][2] != ROCK || on.pos[i][3] < LINE || !(face == 0 || face == 5) {
            continue;
        }
        let n = support::rigs::octa16_normal(on.info[i][0], on.info[i][1]);
        let Some(want) = by_normal(&cells, v, face, n) else {
            continue;
        };
        checked += 1;
        if !within(rgb(on.accum[i]), want, 0.01) {
            bad_on += 1;
        }
        let own = brick_value(&cells, v, face).expect("a mixed face has its own value");
        if !within(rgb(off.accum[i]), own, 0.01) {
            bad_off += 1;
        }
        if face == 0 {
            risers += 1;
        } else {
            treads += 1;
        }
    }
    println!(
        "{checked} rock pixels past t {LINE}: {risers} risers, {treads} treads; {bad_on} off the mix, {bad_off} off their face's value"
    );
    assert!(risers > 0 && treads > 0, "{risers} risers, {treads} treads");
    assert_eq!((bad_on, bad_off), (0, 0), "of {checked}");
    let defaults = stair_frame(
        &mut rig,
        &camera,
        &arm(
            flythrough::DEFAULT_STORE_READ,
            flythrough::DEFAULT_LIGHT_WEIGHTS,
            LightingResolution::default(),
        ),
    );
    let path = support::golden_path("far_staircase_store_normal.png");
    if std::env::var("VXE_BLESS").as_deref() == Ok("1") {
        support::write_png(&path, &defaults.display, SIZE[0], SIZE[1]);
        println!("blessed {path:?}");
        return;
    }
    let (golden, w, h) = support::read_png(&path);
    assert_eq!([w, h], SIZE);
    let f = support::fraction_differing(&defaults.display, &golden, 2);
    println!(
        "far_staircase_store_normal.png: {:.4} % of pixels differ by more than 2/255",
        f * 100.0
    );
    assert!(f <= 0.001, "{:.4} % of pixels differ", f * 100.0);
}

/// The staircase baked for the near-face tests at `size`, lit as the staircase tests are.
fn baked_staircase(size: [u32; 2]) -> Option<Rig> {
    let mut rig = rig_on(staircase(), size, None)?;
    rig.frame.set_lighting(Lighting {
        lamp: false,
        lamp_bounce: false,
        exposure: Exposure::Manual(8.0),
        bake_pool_bytes: 64 << 20,
        ..Lighting::default()
    });
    rig.bake(&test_counts(256));
    Some(rig)
}

/// The pixels of two staircase frames whose read-backs differ: (display bytes, accumulation
/// pixels).
fn frame_diff(a: &StairFrame, b: &StairFrame) -> (usize, usize) {
    (
        a.display
            .iter()
            .zip(&b.display)
            .filter(|(x, y)| x != y)
            .count(),
        a.accum.iter().zip(&b.accum).filter(|(x, y)| x != y).count(),
    )
}

/// ADR-0049's read leaves a near face to the bit (Plan 0061 Phase 2 done-when 2). At 192 x 108
/// under `--look face` with far normal `[1, 2]` the far normal's weight is 0 where a voxel spans
/// 2 pixels or more, `t <= 192 / (2 tan 35 deg) / 2 = 68.6` voxels, so the hit's normal is its
/// face's there and the read takes the face's own value: every hit pixel of the near camera at
/// `t <= 64` holds the same accumulated (stored) value bit for bit with the store read `Normal`
/// and `Face` (full light resolution, weights `Normal`), over at least 10 000 such pixels. With
/// the far normal off the far camera's display and accumulation are bit-identical, read
/// `Normal` against `Face`. Under `gradient27` and `trilinear8` (the normal leaves the face at
/// every distance) with the read on, every stored part of the far camera's frame is finite.
#[test]
fn the_store_read_by_normal_leaves_near_faces_alone() {
    use vxe_render::{LightWeights, NormalMode, StoreRead};
    const SIZE: [u32; 2] = [192, 108];
    const NEAR: f32 = 64.0;
    let [camera, near_camera] = stair_cameras(SIZE);
    let Some(mut rig) = baked_staircase(SIZE) else {
        return;
    };
    let arm = |read, far_normal, mode| StairArm {
        read,
        weights: LightWeights::Normal,
        far_normal,
        mode,
        resolution: LightingResolution::Full,
    };
    let face = stair_frame(
        &mut rig,
        &near_camera,
        &arm(StoreRead::Face, [1.0, 2.0], NormalMode::Face),
    );
    let normal = stair_frame(
        &mut rig,
        &near_camera,
        &arm(StoreRead::Normal, [1.0, 2.0], NormalMode::Face),
    );
    let near: Vec<usize> = (0..normal.pos.len())
        .filter(|&i| normal.info[i][3] & 1 != 0 && normal.pos[i][3] <= NEAR)
        .collect();
    let moved = near
        .iter()
        .filter(|&&i| normal.accum[i] != face.accum[i])
        .count();
    println!(
        "near camera, far normal [1, 2]: {moved} of {} hit pixels at t <= {NEAR} differ read Normal against Face",
        near.len()
    );
    assert!(near.len() >= 10_000, "{} near hit pixels", near.len());
    assert_eq!(moved, 0, "of {}", near.len());

    let face = stair_frame(
        &mut rig,
        &camera,
        &arm(StoreRead::Face, [0.0, 0.0], NormalMode::Face),
    );
    let normal = stair_frame(
        &mut rig,
        &camera,
        &arm(StoreRead::Normal, [0.0, 0.0], NormalMode::Face),
    );
    let diff = frame_diff(&normal, &face);
    println!(
        "far camera, far normal off: read Normal against Face differs in {} display bytes, {} accumulated pixels",
        diff.0, diff.1
    );
    assert_eq!(diff, (0, 0));

    for mode in [NormalMode::Gradient27, NormalMode::Trilinear8] {
        let f = stair_frame(&mut rig, &camera, &arm(StoreRead::Normal, [1.0, 2.0], mode));
        let nonfinite = f
            .accum
            .iter()
            .filter(|p| p.iter().any(|x| !x.is_finite()))
            .count();
        println!("{mode:?}, read Normal: {nonfinite} non-finite stored parts");
        assert_eq!(nonfinite, 0, "{mode:?}");
    }
}

/// The face weights leave a frame with the far normal off to the bit (Plan 0061 Phase 2
/// done-when 2b): there every hit's normal is its face's, so the 2 x 2 upsample's and the
/// a-trous pass's weights read the same dot either way. At 96 x 54 under `--look face`, for the
/// far and the near camera, the display is bit-identical with weights `Face` against `Normal`
/// (store read `Face`) at half and at full light resolution, and with the fly-through's store
/// read and weights against the library's (`Face`, `Normal`) at half.
#[test]
fn the_face_weights_leave_near_faces_alone() {
    use vxe_render::{LightWeights, NormalMode, StoreRead};
    const SIZE: [u32; 2] = [96, 54];
    let Some(mut rig) = baked_staircase(SIZE) else {
        return;
    };
    let arm = |read, weights, resolution| StairArm {
        read,
        weights,
        far_normal: [0.0, 0.0],
        mode: NormalMode::Face,
        resolution,
    };
    let half = LightingResolution::Half;
    let pairs = [
        (
            "weights Face against Normal, half",
            arm(StoreRead::Face, LightWeights::Face, half),
            arm(StoreRead::Face, LightWeights::Normal, half),
        ),
        (
            "weights Face against Normal, full",
            arm(
                StoreRead::Face,
                LightWeights::Face,
                LightingResolution::Full,
            ),
            arm(
                StoreRead::Face,
                LightWeights::Normal,
                LightingResolution::Full,
            ),
        ),
        (
            "the fly-through's read and weights against the library's, half",
            arm(
                flythrough::DEFAULT_STORE_READ,
                flythrough::DEFAULT_LIGHT_WEIGHTS,
                half,
            ),
            arm(StoreRead::Face, LightWeights::Normal, half),
        ),
    ];
    for (c, camera) in stair_cameras(SIZE).iter().enumerate() {
        for (name, a, b) in &pairs {
            let fa = stair_frame(&mut rig, camera, a);
            let fb = stair_frame(&mut rig, camera, b);
            let lit = fa.display.chunks(4).filter(|p| p[..3] != [0, 0, 0]).count();
            let (bytes, _) = frame_diff(&fa, &fb);
            println!(
                "camera {c}, far normal off, {name}: {bytes} display bytes differ ({lit} lit pixels)"
            );
            assert!(lit > 0, "camera {c}, {name}: a black frame");
            assert_eq!(bytes, 0, "camera {c}, {name}");
        }
    }
}

/// Readings: measurement tests that print numbers and assert nothing a tier runs. Each is
/// `#[ignore]`d and run by hand, `cargo nextest run -p <package> --run-ignored only -E
/// 'test(=reading::<name>)'`; the soak and full tiers leave this module out by its name.
mod reading {
    use super::*;

    /// The vein piece (chunks within 2 of the vein pose's) baked at the queue's settings on the
    /// adapter `VXE_TEST_ADAPTER` names; its blocks saved as `live/bake-count/adapter-<name>.bin`.
    /// When the RTX's and the Cezanne's files are both there, every direction value either holds is
    /// compared: the equal words' share, and the luminance's tone-mapped level difference, mean and
    /// p99.
    #[test]
    #[ignore = "a reading on a named adapter: VXE_TEST_ADAPTER, under the perf lock"]
    fn adapter_records() {
        let name = std::env::var("VXE_TEST_ADAPTER").unwrap_or_else(|_| "fallback".to_owned());
        let mut log = Log::open("adapter.txt");
        let vein = flythrough::VEIN_POSE.position;
        let mut rig = read_rig(Scene::Level, vein);
        let piece = piece_of(rig.world.extent(), vein, Some(2));
        let t = std::time::Instant::now();
        let r = rig
            .frame
            .bake_until(
                &rig.device,
                &rig.queue,
                &mut rig.gpu,
                &rig.world,
                piece,
                &bake::queue_settings(),
                &mut |_| {},
            )
            .expect("the piece fits the pool");
        log.line(&format!(
        "adapter records, {}: the vein piece {:?}..{:?} at 8/4096, {} chunks, {} faces, {:.3e} samples in {:.0} ms",
        rig.adapter,
        piece.min.0,
        [0, 1, 2].map(|k| piece.min.0[k] + piece.size()[k]),
        r.chunks,
        r.faces,
        r.samples as f64,
        t.elapsed().as_secs_f64() * 1e3
    ));
        let blocks = frame_blocks(&rig.device, &rig.queue, &rig.frame, piece);
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../live/bake-count");
        let file = |n: &str| format!("{dir}/adapter-{}.bin", n.to_lowercase());
        let mut bytes = Vec::new();
        for (c, b) in &blocks {
            for v in c
                .iter()
                .map(|&x| x as u32)
                .chain([b.len() as u32])
                .chain(b.iter().copied())
            {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
        }
        std::fs::write(file(&name), &bytes).expect("the records file");
        let read = |n: &str| -> Option<BTreeMap<[i32; 3], Vec<u32>>> {
            let raw = std::fs::read(file(n)).ok()?;
            let w: Vec<u32> = raw
                .chunks(4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();
            let mut out = BTreeMap::new();
            let mut i = 0;
            while i < w.len() {
                let c = [w[i] as i32, w[i + 1] as i32, w[i + 2] as i32];
                let n = w[i + 3] as usize;
                out.insert(c, w[i + 4..i + 4 + n].to_vec());
                i += 4 + n;
            }
            Some(out)
        };
        let (Some(a), Some(b)) = (read("rtx"), read("radeon")) else {
            log.line("adapter records: the other adapter's file is not there yet");
            return;
        };
        let (ca, cb) = (cells_of(&a), cells_of(&b));
        let level = |w: u32| {
            let v = bake::rgb9e5_unpack(w);
            bake::tone_level(
                0.2126 * f64::from(v[0]) + 0.7152 * f64::from(v[1]) + 0.0722 * f64::from(v[2]),
            )
        };
        let mut diffs = Vec::new();
        let (mut equal, mut only) = (0usize, 0usize);
        for (k, x) in &ca {
            let Some(y) = cb.get(k) else {
                only += 1;
                continue;
            };
            equal += usize::from(x[0] == y[0]);
            diffs.push((level(x[0]) - level(y[0])).abs());
        }
        only += cb.keys().filter(|k| !ca.contains_key(k)).count();
        diffs.sort_by(f64::total_cmp);
        let n = diffs.len().max(1);
        log.line(&format!(
        "adapter records, RTX against Cezanne: {} directions, {only} held by one alone; equal words {:.4}; |levels| mean {:.3}, p50 {:.3}, p99 {:.3}, max {:.3}",
        diffs.len(),
        equal as f64 / n as f64,
        diffs.iter().sum::<f64>() / n as f64,
        diffs[(n as f64 * 0.5) as usize % n],
        diffs[((n as f64 * 0.99) as usize).min(n - 1)],
        diffs.last().copied().unwrap_or(0.0)
    ));
    }

    /// The hall's pane with the lamp off: at the hall view (native, `VXE_READ_SIZE`), a 40 x 40 x 2
    /// box of the reference's glass row 6 voxels ahead along the horizontal axis the view faces most,
    /// placed before the brick bake (`brick_bake`), then 64 still frames. Over the pixels with the
    /// pane in front: the reflected term before exposure (the layer target's rgb), its luminance p50
    /// and p90 and the pixels where it is 0; its display level alone at exposure 8 against the
    /// pixel's shown luma, p50 and p90 of each and of their ratio (the reflected share). Logs to
    /// `live/bake-count/glass.txt`.
    #[test]
    #[ignore = "a reading on a named adapter: VXE_TEST_ADAPTER, under the perf lock"]
    fn glass_readings() {
        let mut log = Log::open("glass.txt");
        let size = std::env::var("VXE_READ_SIZE")
            .ok()
            .and_then(|v| {
                let (w, h) = v.split_once('x')?;
                Some([w.parse().ok()?, h.parse().ok()?])
            })
            .unwrap_or(NATIVE);
        let view = read_views()
            .into_iter()
            .find(|v| v.name == "hall")
            .expect("the hall view");
        let (mut rig, _) = view_rig(&view, size);
        let glass = flythrough::showcase_materials().len() as vxe_world::MaterialId;
        let camera = view_camera(&view);
        let (fwd, _, _) = camera.basis();
        let a = if fwd.x.abs() >= fwd.y.abs() { 0 } else { 1 };
        let eye = camera.position.to_array().map(|v| v.floor() as i64);
        let ahead = if fwd.to_array()[a] > 0.0 { 6 } else { -7 };
        let mut min = eye.map(|v| v - 20);
        let mut max = eye.map(|v| v + 20);
        min[a] = eye[a] + ahead;
        max[a] = min[a] + 2;
        let r = rig
            .world
            .apply(&EditOp::box_(
                Fixed3(min.map(|v| v * FIXED_ONE)),
                Fixed3(max.map(|v| v * FIXED_ONE)),
                Mode::Fill(glass),
            ))
            .expect("the pane is resident");
        rig.gpu
            .sync_now(&rig.world, &rig.device, &rig.queue)
            .expect("the mirror fits");
        let placed: u32 = r.changed_from.iter().map(|(_, n)| n).sum();
        let bake_ms = brick_bake(&mut rig, &view);
        for _ in 0..64 {
            rig.frame.step(
                &rig.device,
                &rig.queue,
                &rig.gpu,
                &camera,
                &LightParams::default(),
            );
        }
        let lighting = rig.frame.lighting();
        let level = |c: [f32; 3]| {
            let b = tone_bytes([c[0], c[1], c[2], 1.0], &lighting);
            0.2126 * f32::from(b[0]) + 0.7152 * f32::from(b[1]) + 0.0722 * f32::from(b[2])
        };
        let info = readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
        let layer = readback::read_float(&rig.device, &rig.queue, rig.frame.layers());
        let shown = luma(&readback::read_rgba8(
            &rig.device,
            &rig.queue,
            rig.frame.target(),
        ));
        let lum = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let (mut refl, mut levels, mut shares, mut lumas) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut zero = 0usize;
        for i in 0..info.len() {
            if info[i][3] & 4 == 0 {
                continue;
            }
            let c = [layer[i][0], layer[i][1], layer[i][2]];
            if c == [0.0; 3] {
                zero += 1;
            }
            let l = level(c.map(|x| x * 8.0));
            refl.push(lum(c));
            levels.push(l);
            lumas.push(shown[i]);
            shares.push(if shown[i] > 0.0 { l / shown[i] } else { 0.0 });
        }
        let q = |v: &mut Vec<f32>, p: f32| {
            v.sort_by(f32::total_cmp);
            v.get(((v.len() as f32 - 1.0) * p) as usize)
                .copied()
                .unwrap_or(0.0)
        };
        log.line(&format!(
        "glass hall adapter {} {}x{} lamp off exposure 8: pane {min:?}..{max:?}, {placed} voxels; bake {bake_ms:.0} ms; {} pixels behind the pane, {zero} reflecting 0; reflected luminance before exposure p50 {:.5} p90 {:.5}; its level alone p50 {:.1} p90 {:.1}; shown luma p50 {:.1} p90 {:.1}; share (level / shown) p50 {:.3} p90 {:.3}",
        rig.adapter,
        size[0],
        size[1],
        refl.len(),
        q(&mut refl, 0.5),
        q(&mut refl, 0.9),
        q(&mut levels, 0.5),
        q(&mut levels, 0.9),
        q(&mut lumas, 0.5),
        q(&mut lumas, 0.9),
        q(&mut shares, 0.5),
        q(&mut shares, 0.9),
    ));
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
        // The hit in the view's central half nearest 30 voxels whose carve (radius 5, plus the
        // ring's voxel) lies inside the piece: the view's centre can hit past the level's edge.
        let pos = readback::read_float(&rig.device, &rig.queue, rig.frame.hit_positions());
        let info = readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
        let dist = |p: &[f32; 3]| (0..3).map(|k| (p[k] - eye[k]).powi(2)).sum::<f32>().sqrt();
        let hit = (size[1] / 4..3 * size[1] / 4)
            .flat_map(|y| (size[0] / 4..3 * size[0] / 4).map(move |x| (y * size[0] + x) as usize))
            .filter(|&i| info[i][3] & 1 != 0)
            .map(|i| [pos[i][0], pos[i][1], pos[i][2]])
            .filter(|p| {
                let v = p.map(|v| v.floor() as i32);
                inside(piece, v.map(|c| c - 6)) && inside(piece, v.map(|c| c + 6))
            })
            .min_by(|a, b| (dist(a) - 30.0).abs().total_cmp(&(dist(b) - 30.0).abs()))
            .expect("a hit inside the piece in the view's central half");
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
        dist(&hit),
        dug[0],
        dug[1],
        ring.values().map(Vec::len).sum::<usize>(),
        ring.len()
    ));
        let arms: Vec<(&str, BakeSettings)> = vec![
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

    /// The resolution cut under the brick store (Plan 0059 Phase 10 as re-read at Phase 7b): at each
    /// view of `VXE_BAKE_VIEWS` (default vein and hall), the view's brick store baked (`brick_bake`), and
    /// three settings at `NATIVE`: native with no upscaler (the reference), FSR 2 at 1.5 with far
    /// normal 1..2, and FSR 2 at 1.5 with far normal 2..6 (the fly-through's default image). Per
    /// setting, after 128 still frames: the share of the pixels within 1, 2 and 4 display levels of
    /// the reference's, the passes' milliseconds over 120 timed frames at rest, and the fast yaw's
    /// excess dots (64 frames at -90 degrees, ten 9-degree steps back, 60 frames
    /// counted against the still floor). Logs to `live/bake-count/resolution.txt`.
    #[test]
    #[ignore = "a reading on the RTX: VXE_TEST_ADAPTER=rtx, under the perf lock"]
    fn resolution_readings() {
        let mut log = Log::open("resolution.txt");
        let names = std::env::var("VXE_BAKE_VIEWS").unwrap_or_else(|_| "vein,hall".to_owned());
        for view in read_views() {
            if !names.split(',').any(|n| n == view.name) {
                continue;
            }
            let (mut rig, _) = view_rig(&view, NATIVE);
            let load_ms = brick_bake(&mut rig, &view);
            let timer = vxe_render::marcher::PassTimer::with_passes(
                &rig.device,
                &rig.queue,
                vxe_render::marcher::FRAME_PASSES.len() as u32,
            )
            .expect("timestamps on the RTX");
            let still = view_camera(&view);
            let yawed = |deg: f32| Camera {
                yaw: still.yaw + deg.to_radians(),
                ..still
            };
            let light = LightParams::default();
            let fsr = || {
                vxe_render::UpscalerKind::Fsr2(vxe_render::Fsr2Settings {
                    sharpness: 0.5,
                    ..vxe_render::Fsr2Settings::default()
                })
            };
            let settings: [(&str, Option<vxe_render::UpscalerKind>, [f32; 2]); 3] = [
                ("native, no upscaler, far normal 1..2", None, [1.0, 2.0]),
                ("fsr2 at 1.5, far normal 1..2", Some(fsr()), [1.0, 2.0]),
                ("fsr2 at 1.5, far normal 2..6", Some(fsr()), [2.0, 6.0]),
            ];
            let mut reference: Option<Vec<f32>> = None;
            for (setting, kind, far) in settings {
                match kind {
                    None => {
                        rig.frame
                            .set_upscaler_kind(&rig.device, vxe_render::UpscalerKind::Off);
                        rig.frame.set_jitter(&rig.device, false);
                        rig.frame.set_sizes(&rig.device, NATIVE, NATIVE);
                    }
                    Some(k) => {
                        rig.frame.set_upscaler_kind(&rig.device, k);
                        rig.frame.set_sizes(
                            &rig.device,
                            vxe_render::marcher::RenderScale::OneHalf.internal(NATIVE),
                            NATIVE,
                        );
                    }
                }
                rig.frame.set_look(vxe_render::marcher::LookParams {
                    far_normal: far,
                    ..rig.frame.look()
                });
                rig.frame.reset_history();
                for _ in 0..128 {
                    rig.frame
                        .step(&rig.device, &rig.queue, &rig.gpu, &still, &light);
                }
                let img = luma(&readback::read_rgba8(
                    &rig.device,
                    &rig.queue,
                    rig.frame.target(),
                ));
                let shares = match &reference {
                    None => {
                        reference = Some(img.clone());
                        "the reference".to_owned()
                    }
                    Some(r) => {
                        let n = r.len() as f64;
                        let within = |lv: f32| {
                            r.iter()
                                .zip(&img)
                                .filter(|(a, b)| (*a - *b).abs() <= lv)
                                .count() as f64
                                / n
                        };
                        format!(
                            "within 1/2/4 levels of the reference {:.4} / {:.4} / {:.4}",
                            within(1.0),
                            within(2.0),
                            within(4.0)
                        )
                    }
                };
                let mut sum = vec![0.0; vxe_render::marcher::FRAME_PASSES.len()];
                for _ in 0..120 {
                    for (s, ms) in sum.iter_mut().zip(timed_step(&mut rig, &timer, &still)) {
                        *s += ms;
                    }
                }
                let passes = pass_line(&sum, 120);
                let dots = |rig: &ReadRig| -> f64 {
                    let px = readback::read_rgba8(&rig.device, &rig.queue, rig.frame.target());
                    support::aliasing::dots(&px, NATIVE[0], NATIVE[1])
                        .iter()
                        .filter(|d| **d)
                        .count() as f64
                };
                let mut floor = Vec::new();
                for _ in 0..64 {
                    rig.frame
                        .step(&rig.device, &rig.queue, &rig.gpu, &still, &light);
                    floor.push(dots(&rig));
                }
                for _ in 0..64 {
                    rig.frame
                        .step(&rig.device, &rig.queue, &rig.gpu, &yawed(-90.0), &light);
                }
                for i in 1..=10 {
                    rig.frame.step(
                        &rig.device,
                        &rig.queue,
                        &rig.gpu,
                        &yawed(-90.0 + 9.0 * i as f32),
                        &light,
                    );
                }
                let mean = floor.iter().sum::<f64>() / floor.len() as f64;
                let sd = (floor.iter().map(|f| (f - mean).powi(2)).sum::<f64>()
                    / floor.len() as f64)
                    .sqrt();
                let mut excess = 0.0;
                for _ in 0..YAW_DOT_FRAMES {
                    rig.frame
                        .step(&rig.device, &rig.queue, &rig.gpu, &still, &light);
                    excess += dots(&rig) - mean;
                }
                log.line(&format!(
                "resolution {} adapter {} {}x{} (bricks 8/4096 loaded in {load_ms:.0} ms), {setting}: {shares}; at rest {passes}; fast yaw excess dots 1-60 {excess:+.1} (floor {mean:.1}, sd {sd:.1}, line {:.1})",
                view.name,
                rig.adapter,
                NATIVE[0],
                NATIVE[1],
                YAW_DOT_FRAMES as f64 * sd
            ));
            }
        }
    }

    /// The light's budget at rest (Plan 0059 Phase 12): at each view of `VXE_BAKE_VIEWS` (default
    /// vein and hall) at `VXE_READ_SIZE` (default `NATIVE`), the brick store (`brick_bake`), native
    /// with no upscaler and FSR 2 at 1.5, far normal 1..2, lamp off. Per arm, after 1024 still frames
    /// from no history: the passes over 120 timed frames, the lighting line (gradient, light,
    /// temporal, probes, gather), and the display's change from frame 1024 to 1025 (the one frame)
    /// and from 512 to 1024 (the drift): the mean of the channels' level differences, the largest,
    /// and whether the two frames are bit-identical. Logs to `live/bake-count/rest.txt`.
    #[test]
    #[ignore = "a reading on a named adapter: VXE_TEST_ADAPTER, under the perf lock"]
    fn rest_readings() {
        let mut log = Log::open("rest.txt");
        let names = std::env::var("VXE_BAKE_VIEWS").unwrap_or_else(|_| "vein,hall".to_owned());
        let size = std::env::var("VXE_READ_SIZE")
            .ok()
            .and_then(|v| {
                let (w, h) = v.split_once('x')?;
                Some([w.parse().ok()?, h.parse().ok()?])
            })
            .unwrap_or(NATIVE);
        const LINE: [&str; 5] = ["gradient", "light", "temporal", "probes", "gather"];
        for view in read_views() {
            if !names.split(',').any(|n| n == view.name) {
                continue;
            }
            let (mut rig, _) = view_rig(&view, size);
            brick_bake(&mut rig, &view);
            let timer = vxe_render::marcher::PassTimer::with_passes(
                &rig.device,
                &rig.queue,
                vxe_render::marcher::FRAME_PASSES.len() as u32,
            )
            .expect("timestamps on the adapter");
            let still = view_camera(&view);
            let light = LightParams::default();
            let light_name = "baked";
            {
                for fsr in [false, true] {
                    rig.frame.set_lighting(reference_lighting(false));
                    if fsr {
                        rig.frame.set_upscaler_kind(
                            &rig.device,
                            vxe_render::UpscalerKind::Fsr2(vxe_render::Fsr2Settings {
                                sharpness: 0.5,
                                ..vxe_render::Fsr2Settings::default()
                            }),
                        );
                        rig.frame.set_sizes(
                            &rig.device,
                            vxe_render::marcher::RenderScale::OneHalf.internal(size),
                            size,
                        );
                    } else {
                        rig.frame
                            .set_upscaler_kind(&rig.device, vxe_render::UpscalerKind::Off);
                        rig.frame.set_jitter(&rig.device, false);
                        rig.frame.set_sizes(&rig.device, size, size);
                    }
                    rig.frame.set_look(vxe_render::marcher::LookParams {
                        far_normal: [1.0, 2.0],
                        ..rig.frame.look()
                    });
                    rig.frame.reset_history();
                    let read = |rig: &ReadRig| {
                        readback::read_rgba8(&rig.device, &rig.queue, rig.frame.target())
                    };
                    let mut at512 = Vec::new();
                    for f in 1..=1024 {
                        rig.frame
                            .step(&rig.device, &rig.queue, &rig.gpu, &still, &light);
                        if f == 512 {
                            at512 = read(&rig);
                        }
                    }
                    let at1024 = read(&rig);
                    rig.frame
                        .step(&rig.device, &rig.queue, &rig.gpu, &still, &light);
                    let at1025 = read(&rig);
                    let change = |a: &[u8], b: &[u8]| {
                        let mut sum = 0u64;
                        let mut max = 0u8;
                        for (p, q) in a.chunks(4).zip(b.chunks(4)) {
                            for k in 0..3 {
                                let d = p[k].abs_diff(q[k]);
                                sum += u64::from(d);
                                max = max.max(d);
                            }
                        }
                        format!(
                            "mean {:.4} max {max}{}",
                            sum as f64 / (a.len() / 4 * 3) as f64,
                            if a == b { ", bit-identical" } else { "" }
                        )
                    };
                    let one = change(&at1024, &at1025);
                    let drift = change(&at512, &at1024);
                    let mut sum = vec![0.0; vxe_render::marcher::FRAME_PASSES.len()];
                    for _ in 0..120 {
                        for (s, ms) in sum.iter_mut().zip(timed_step(&mut rig, &timer, &still)) {
                            *s += ms;
                        }
                    }
                    let of = |n: &str| {
                        vxe_render::marcher::FRAME_PASSES
                            .iter()
                            .position(|p| *p == n)
                            .map_or(0.0, |i| sum[i] / 120.0)
                    };
                    let line: f64 = LINE.iter().map(|n| of(n)).sum();
                    log.line(&format!(
                    "rest {} adapter {} {}x{} {light_name} {}: lighting line {line:.2} ms (temporal {:.2}); one frame 1024/1025 {one}; drift 512/1024 {drift}; {}",
                    view.name,
                    rig.adapter,
                    size[0],
                    size[1],
                    if fsr { "fsr2 at 1.5" } else { "native" },
                    of("temporal"),
                    pass_line(&sum, 120)
                ));
                }
            }
        }
    }

    /// The per-pass table (Plan 0065 Phase 1): at each view of `VXE_BAKE_VIEWS` (default vein and
    /// hall), the brick store (`brick_bake`), FSR 2 at 1.5 to `NATIVE` (internal 1687 x 919),
    /// sharpness 0.5, the far normal at `VXE_FAR_NORMAL` (`lo,hi`, default the fly-through's
    /// `2,6`). Per arm (lighting tier half and full, lamp off and on, at rest and walking), 128
    /// still frames from no history, then 120 timed frames: still, or walking (the eye moves at 14
    /// voxels a second, 60 frames out and 60 back, its view held: at the vein back along the
    /// steady flight that ends there, at the hall along the level heading). Each pass's
    /// mean milliseconds, every pass printed to 3 decimals. Logs to `live/bake-count/passes.txt`.
    #[test]
    #[ignore = "a reading on a named adapter: VXE_TEST_ADAPTER, under the perf lock"]
    fn pass_readings() {
        let mut log = Log::open("passes.txt");
        let names = std::env::var("VXE_BAKE_VIEWS").unwrap_or_else(|_| "vein,hall".to_owned());
        let far = std::env::var("VXE_FAR_NORMAL")
            .ok()
            .and_then(|v| {
                let (lo, hi) = v.split_once(',')?;
                Some([lo.parse().ok()?, hi.parse().ok()?])
            })
            .unwrap_or([2.0, 6.0]);
        const FRAMES: usize = 120;
        for view in read_views() {
            if !names.split(',').any(|n| n == view.name) {
                continue;
            }
            let (mut rig, _) = view_rig(&view, NATIVE);
            brick_bake(&mut rig, &view);
            let timer = vxe_render::marcher::PassTimer::with_passes(
                &rig.device,
                &rig.queue,
                vxe_render::marcher::FRAME_PASSES.len() as u32,
            )
            .expect("timestamps on the adapter");
            let internal = vxe_render::marcher::RenderScale::OneHalf.internal(NATIVE);
            rig.frame.set_upscaler_kind(
                &rig.device,
                vxe_render::UpscalerKind::Fsr2(vxe_render::Fsr2Settings {
                    sharpness: 0.5,
                    ..vxe_render::Fsr2Settings::default()
                }),
            );
            rig.frame.set_sizes(&rig.device, internal, NATIVE);
            rig.frame.set_look(vxe_render::marcher::LookParams {
                far_normal: far,
                ..rig.frame.look()
            });
            let still = view_camera(&view);
            // The walk's direction: at the vein back along the steady flight that ends at its
            // pose (a cube of 5 voxels of air around every step), elsewhere the view's level
            // heading.
            let ahead = if view.name == "vein" {
                let (start, end) = support::settle::flight_path(&flythrough::VEIN_POSE);
                (vxe_render::glam::Vec3::from(start) - vxe_render::glam::Vec3::from(end))
                    .normalize()
            } else {
                vxe_render::glam::Vec3::new(still.yaw.cos(), still.yaw.sin(), 0.0)
            };
            let walk = |f: usize| {
                let t = (f % FRAMES) as f32;
                let out = if t < 60.0 { t } else { FRAMES as f32 - t };
                Camera {
                    position: still.position + ahead * (14.0 / 60.0 * out),
                    ..still
                }
            };
            let light = LightParams::default();
            for tier in [LightingResolution::Half, LightingResolution::Full] {
                rig.frame.set_resolution(&rig.device, tier);
                for lamp in [false, true] {
                    rig.frame.set_lighting(reference_lighting(lamp));
                    for walking in [false, true] {
                        rig.frame.reset_history();
                        for _ in 0..128 {
                            rig.frame
                                .step(&rig.device, &rig.queue, &rig.gpu, &still, &light);
                        }
                        let mut sum = vec![0.0; vxe_render::marcher::FRAME_PASSES.len()];
                        for f in 0..FRAMES {
                            let c = if walking { walk(f) } else { still };
                            for (s, ms) in sum.iter_mut().zip(timed_step(&mut rig, &timer, &c)) {
                                *s += ms;
                            }
                        }
                        let total: f64 = sum.iter().sum::<f64>() / FRAMES as f64;
                        let passes: Vec<String> = vxe_render::marcher::FRAME_PASSES
                            .iter()
                            .zip(&sum)
                            .filter(|(_, ms)| **ms > 0.0)
                            .map(|(n, ms)| format!("{n} {:.3}", ms / FRAMES as f64))
                            .collect();
                        log.line(&format!(
                            "passes {} adapter {} {}x{} to {}x{} fsr2 at 1.5, far normal {}..{}, tier {}, lamp {}, {}: total {total:.3} ms, {}",
                            view.name,
                            rig.adapter,
                            internal[0],
                            internal[1],
                            NATIVE[0],
                            NATIVE[1],
                            far[0],
                            far[1],
                            tier.name(),
                            if lamp { "on" } else { "off" },
                            if walking { "walking" } else { "at rest" },
                            passes.join(", ")
                        ));
                    }
                }
            }
        }
    }

    /// Plan 0061 Phase 2's claims on the staircase, as a reading (the phase is exploration):
    /// done-when 1 at 192 x 108 (`t >= 140`) and 1b at 96 x 54 (`t >= 72`), far normal `[1, 2]`:
    /// with the read on every rock riser or tread pixel past the line reads ADR-0049's mix within
    /// 1 % (the CPU's `by_normal` over the bake's records), with it off its face's value within
    /// 1 % and both directions occur; done-when 2: `t <= 64` reads the same with the read on and
    /// off under `face`, the frame is bit-identical with the far normal off, and under
    /// `gradient27` and `trilinear8` every stored part is finite; done-when 2b: with the far
    /// normal off the face weights leave the frame bit-identical at half and full resolution.
    /// Counts and failures go to `live/bake-count/staircase.txt`.
    #[test]
    #[ignore = "a reading: Plan 0061 Phase 2, run by hand"]
    fn staircase_read() {
        use vxe_render::{LightWeights, NormalMode, StoreRead};
        let mut log = Log::open("staircase.txt");
        log.line(&format!(
            "staircase reading, {}",
            std::env::var("VXE_TEST_ADAPTER").unwrap_or_else(|_| "lavapipe".to_owned())
        ));
        for (size, line) in [([192u32, 108u32], 140f32), ([96, 54], 72.0)] {
            let world = staircase();
            let [camera, near_camera] = stair_cameras(size);
            let Some(mut rig) = rig_on(world, size, None) else {
                return;
            };
            rig.frame.set_lighting(Lighting {
                lamp: false,
                lamp_bounce: false,
                exposure: Exposure::Manual(8.0),
                bake_pool_bytes: 64 << 20,
                ..Lighting::default()
            });
            let t0 = std::time::Instant::now();
            let report = rig.bake(&test_counts(256));
            log.line(&format!(
                "{}x{}: baked {} faces in {:.1} s",
                size[0],
                size[1],
                report.faces,
                t0.elapsed().as_secs_f64()
            ));
            let cells = rig.cells();
            let eye = camera.position.to_array();
            let arm = |read, far_normal, mode, weights, resolution| StairArm {
                read,
                weights,
                far_normal,
                mode,
                resolution,
            };
            let full = LightingResolution::Full;
            let off = rig_frame(
                &mut rig,
                &camera,
                arm(
                    StoreRead::Face,
                    [1.0, 2.0],
                    NormalMode::Face,
                    LightWeights::Normal,
                    full,
                ),
            );
            let on = rig_frame(
                &mut rig,
                &camera,
                arm(
                    StoreRead::Normal,
                    [1.0, 2.0],
                    NormalMode::Face,
                    LightWeights::Normal,
                    full,
                ),
            );
            // Done-when 1 / 1b.
            let (mut checked, mut bad_on, mut bad_off) = (0, 0, 0);
            let (mut risers, mut treads) = (0, 0);
            let (mut vx, mut vz) = (0f64, 0f64);
            let mut worst = 0f32;
            let mut faces = [0u32; 6];
            let mut moved = 0;
            let mut n_sum = [0f64; 3];
            for i in 0..on.pos.len() {
                if let Some((_, f)) = hit_face(eye, on.pos[i], on.info[i])
                    && on.pos[i][3] >= line
                {
                    faces[usize::from(f)] += 1;
                }
            }
            log.line(&format!(
                "{}x{}: hit faces past the line by direction {faces:?}",
                size[0], size[1]
            ));
            for i in 0..on.pos.len() {
                let Some((v, face)) = hit_face(eye, on.pos[i], on.info[i]) else {
                    continue;
                };
                if on.info[i][2] != ROCK || on.pos[i][3] < line || !(face == 0 || face == 5) {
                    continue;
                }
                let n = support::rigs::octa16_normal(on.info[i][0], on.info[i][1]);
                let Some(want) = by_normal(&cells, v, face, n) else {
                    continue;
                };
                checked += 1;
                let got = rgb(on.accum[i]);
                let e = (0..3)
                    .map(|k| (got[k] - want[k]).abs() / want[k].abs().max(1e-3))
                    .fold(0f32, f32::max);
                worst = worst.max(e);
                if !within(got, want, 0.01) {
                    bad_on += 1;
                }
                let own = brick_value(&cells, v, face).unwrap_or([0.0; 3]);
                if !within(rgb(off.accum[i]), own, 0.01) {
                    bad_off += 1;
                }
                if !within(got, own, 0.01) {
                    moved += 1;
                }
                n_sum = [0, 1, 2].map(|k| n_sum[k] + f64::from(n[k]));
                let l = f64::from(own[0] + own[1] + own[2]) / 3.0;
                if face == 0 {
                    risers += 1;
                    vx += l;
                } else {
                    treads += 1;
                    vz += l;
                }
            }
            log.line(&format!(
                "{}x{}: done-when 1: {checked} rock pixels at t >= {line}: read on, {bad_on} off the mix by over 1 % (worst {:.4}); read off, {bad_off} off their face's value; {risers} risers mean {:.4}, {treads} treads mean {:.4} (ratio {:.2})",
                size[0],
                size[1],
                worst,
                vx / f64::from(risers.max(1)),
                vz / f64::from(treads.max(1)),
                (vz / f64::from(treads.max(1))) / (vx / f64::from(risers.max(1))).max(1e-9),
            ));
            log.line(&format!(
                "{}x{}: the read moves {moved} of {checked} past the line by over 1 % from their face's value; mean hit normal [{:.3}, {:.3}, {:.3}]",
                size[0],
                size[1],
                n_sum[0] / f64::from(checked.max(1)),
                n_sum[1] / f64::from(checked.max(1)),
                n_sum[2] / f64::from(checked.max(1)),
            ));
            // Done-when 2: near pixels and the far normal off.
            let off = rig_frame(
                &mut rig,
                &near_camera,
                arm(
                    StoreRead::Face,
                    [1.0, 2.0],
                    NormalMode::Face,
                    LightWeights::Normal,
                    full,
                ),
            );
            let on = rig_frame(
                &mut rig,
                &near_camera,
                arm(
                    StoreRead::Normal,
                    [1.0, 2.0],
                    NormalMode::Face,
                    LightWeights::Normal,
                    full,
                ),
            );
            let near_diff = (0..on.pos.len())
                .filter(|&i| {
                    on.info[i][3] & 1 != 0 && on.pos[i][3] <= 64.0 && on.accum[i] != off.accum[i]
                })
                .count();
            let near = (0..on.pos.len())
                .filter(|&i| on.info[i][3] & 1 != 0 && on.pos[i][3] <= 64.0)
                .count();
            let none_off = rig_frame(
                &mut rig,
                &camera,
                arm(
                    StoreRead::Face,
                    [0.0, 0.0],
                    NormalMode::Face,
                    LightWeights::Normal,
                    full,
                ),
            );
            let none_on = rig_frame(
                &mut rig,
                &camera,
                arm(
                    StoreRead::Normal,
                    [0.0, 0.0],
                    NormalMode::Face,
                    LightWeights::Normal,
                    full,
                ),
            );
            log.line(&format!(
                "{}x{}: done-when 2: {near_diff} of {near} hit pixels at t <= 64 differ read on against off; far normal off, display differs in {} bytes, accumulation in {} pixels",
                size[0],
                size[1],
                none_on.display.iter().zip(&none_off.display).filter(|(a, b)| a != b).count(),
                none_on.accum.iter().zip(&none_off.accum).filter(|(a, b)| a != b).count(),
            ));
            for mode in [NormalMode::Gradient27, NormalMode::Trilinear8] {
                let f = rig_frame(
                    &mut rig,
                    &camera,
                    arm(
                        StoreRead::Normal,
                        [1.0, 2.0],
                        mode,
                        LightWeights::Normal,
                        full,
                    ),
                );
                let nonfinite = f
                    .accum
                    .iter()
                    .filter(|p| p.iter().any(|x| !x.is_finite()))
                    .count();
                let face_off = rig_frame(
                    &mut rig,
                    &camera,
                    arm(
                        StoreRead::Face,
                        [1.0, 2.0],
                        mode,
                        LightWeights::Normal,
                        full,
                    ),
                );
                let moved = f
                    .accum
                    .iter()
                    .zip(&face_off.accum)
                    .filter(|(a, b)| a != b)
                    .count();
                log.line(&format!(
                    "{}x{}: done-when 2, {mode:?}: {nonfinite} non-finite stored parts, read on moves {moved} pixels",
                    size[0], size[1]
                ));
            }
            // Done-when 2b.
            for resolution in [LightingResolution::Half, LightingResolution::Full] {
                let a = rig_frame(
                    &mut rig,
                    &camera,
                    arm(
                        StoreRead::Face,
                        [0.0, 0.0],
                        NormalMode::Face,
                        LightWeights::Normal,
                        resolution,
                    ),
                );
                let b = rig_frame(
                    &mut rig,
                    &camera,
                    arm(
                        StoreRead::Face,
                        [0.0, 0.0],
                        NormalMode::Face,
                        LightWeights::Face,
                        resolution,
                    ),
                );
                log.line(&format!(
                    "{}x{}: done-when 2b, {resolution:?}: face weights against normal, far normal off: display differs in {} bytes",
                    size[0],
                    size[1],
                    a.display.iter().zip(&b.display).filter(|(x, y)| x != y).count()
                ));
            }
        }
    }

    fn rig_frame(rig: &mut Rig, camera: &Camera, arm: StairArm) -> StairFrame {
        stair_frame(rig, camera, &arm)
    }
}
