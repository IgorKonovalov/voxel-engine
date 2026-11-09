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
/// pane reflects no light with the lamp off (the store holds no translucent face), so each pixel
/// is `T(L)` of what it showed before the pane: `L` the exact path through the glass boxes along
/// the pixel's ray, `T = tint^L` per channel, `tint = 0.8 (0.5 + 0.5 albedo / max(albedo))` from
/// the row's albedo. Within 2 levels, head on (the halves at `L` near 1 and 2: `T` and `T^2`) and
/// at 30 degrees of yaw (`L` near 2.3 through the 2-voxel half). A second pane over a black wall
/// lies out of both views. The per-layer composite fails it.
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
            let sd =
                (floor.iter().map(|f| (f - mean).powi(2)).sum::<f64>() / floor.len() as f64).sqrt();
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
