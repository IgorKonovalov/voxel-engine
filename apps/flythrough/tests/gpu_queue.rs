//! The live queue (ADR-0046's scheduler), the fly-through's light: stepped headless on the
//! software adapter as the window's frame steps it (the mirror synced, then
//! `Frame::bake_queue_step` with no sample count, so each step is sized by its slice and waits
//! on nothing), its store read back against one-shot bakes of the same world.
//!
//! Every world here is a few hand-built boxes at test counts (64 samples a brick direction,
//! a coarse pass of 16).

mod support;

use std::collections::{BTreeMap, BTreeSet};

use flythrough::cave::{ORE, ROCK};
use support::SyncNow;
use vxe_render::bake::{self, BakeSettings, QueueStats};
use vxe_render::gpu_world::{GpuWorld, MirrorConfig, Window};
use vxe_render::marcher::LightingResolution;
use vxe_render::{Camera, Exposure, Frame, LightParams, Lighting, QueueView, readback, wgpu};
use vxe_world::{
    AIR, ChunkCoord, ChunkExtent, EditOp, FIXED_ONE, Fixed3, MaterialTable, Mode, UniformSource,
    VoxelCoord, World,
};

fn table() -> MaterialTable {
    flythrough::with_emissive_ore(&flythrough::cave_materials())
}

fn box_op(min: [i64; 3], max: [i64; 3], mode: Mode) -> EditOp {
    EditOp::box_(
        Fixed3(min.map(|v| v * FIXED_ONE)),
        Fixed3(max.map(|v| v * FIXED_ONE)),
        mode,
    )
}

fn fill(world: &World, min: [i64; 3], max: [i64; 3], m: vxe_world::MaterialId) {
    world
        .apply(&box_op(min, max, Mode::Fill(m)))
        .expect("the box is resident");
}

fn world_of(size: [i32; 3], m: vxe_world::MaterialId) -> World {
    World::generate(
        ChunkExtent::new([0, 0, 0], size),
        &UniformSource(m),
        &table(),
    )
}

/// The brick level at 64 samples a brick direction, rounds of 64.
fn counts() -> BakeSettings {
    BakeSettings {
        round: 64,
        brick_samples: 64,
        ..BakeSettings::default()
    }
}

/// The coarse pass's count a brick direction.
const COARSE: u32 = 16;

/// Steps before a test gives up on an empty queue.
const MAX_STEPS: usize = 600;

struct Q {
    device: wgpu::Device,
    queue: wgpu::Queue,
    gpu: GpuWorld,
    world: World,
    frame: Frame,
    cfg: MirrorConfig,
    camera: Camera,
    counts: BakeSettings,
    /// The steps' sample count: `None` is a frame's sliced step, `Some` the pre-roll's.
    samples: Option<u64>,
    /// The last step's `bake_queue_step` call, timed from outside, in ms.
    last_step_ms: f64,
    /// Whether a chunk is resident full, as a streamer's would say; `None` is a bounded world.
    resident: Option<fn(ChunkCoord) -> bool>,
}

/// How a rig's queue and mirror start.
struct Setup {
    window: Option<Window>,
    eye: [f32; 3],
    pool_bytes: u64,
    /// Bytes a mirror sync writes before it stops taking chunks (at least one).
    sync_budget: u64,
    coarse: u32,
    near: i32,
    slice_ms: f64,
    /// The queue's order; `None` keeps its default.
    order: Option<bake::QueueOrder>,
    /// The queue's counts.
    counts: BakeSettings,
}

impl Setup {
    fn at(eye: [f32; 3]) -> Self {
        Self {
            window: None,
            eye,
            pool_bytes: 16 << 20,
            sync_budget: u64::MAX,
            coarse: 0,
            near: bake::DEFAULT_NEAR,
            slice_ms: 1.5,
            order: None,
            counts: counts(),
        }
    }
}

/// A camera at `eye` looking along +x.
fn camera_at(eye: [f32; 3]) -> Camera {
    Camera {
        position: eye.into(),
        yaw: 0.0,
        pitch: 0.0,
        vfov: 70f32.to_radians(),
    }
}

fn new_frame(device: &wgpu::Device, pool_bytes: u64) -> Frame {
    let mut frame = Frame::new(device, 32, 18);
    frame.set_jitter(device, false);
    frame.set_resolution(device, LightingResolution::Full);
    frame.set_lighting(Lighting {
        lamp: false,
        lamp_bounce: false,
        exposure: Exposure::Manual(8.0),
        bake_pool_bytes: pool_bytes,
        ..Lighting::default()
    });
    frame.set_dt(1.0 / 60.0);
    frame
}

/// `world` under a mirror of `setup`'s window (the world's extent by default, light radius 2)
/// synced once, its focus and camera at `setup.eye`, and the live queue started at
/// [`counts`] with `setup`'s coarse pass, near ring and slice.
fn rig(world: World, setup: &Setup) -> Option<Q> {
    let (device, queue) = support::software_device(64 << 20)?;
    let cfg = MirrorConfig {
        budget_bytes: 64 << 20,
        light_radius: 2,
        sync_budget_bytes: setup.sync_budget,
        ..MirrorConfig::new(setup.window.unwrap_or(Window::Fixed(world.extent())))
    };
    let mut gpu = GpuWorld::with_config(&device, cfg);
    gpu.set_focus(VoxelCoord(setup.eye.map(|v| v.floor() as i32)));
    gpu.sync_now(&world, &device, &queue)
        .expect("the mirror fits");
    let mut frame = new_frame(&device, setup.pool_bytes);
    frame.start_bake_queue(&queue, &gpu, setup.counts.clone(), setup.slice_ms);
    frame.set_bake_coarse(setup.coarse, setup.near);
    if let Some(order) = setup.order {
        frame.bake_queue_mut().expect("the queue started").order = order;
    }
    Some(Q {
        device,
        queue,
        gpu,
        world,
        frame,
        cfg,
        camera: camera_at(setup.eye),
        counts: setup.counts.clone(),
        samples: None,
        last_step_ms: 0.0,
        resident: None,
    })
}

impl Q {
    /// One frame's queue step as the window's frame takes it: the mirror synced (`sync`), then
    /// the step with no sample count; then the device waited on, so the next step reads this
    /// one's submit back.
    fn step_with(&mut self, sync: bool) -> QueueStats {
        let eye = self.camera.position.to_array();
        if sync {
            self.gpu
                .set_focus(VoxelCoord(eye.map(|v| v.floor() as i32)));
            self.gpu
                .sync_now(&self.world, &self.device, &self.queue)
                .expect("the mirror fits");
        }
        let resident = self.resident;
        let resident_fn = move |c: ChunkCoord| resident.is_some_and(|f| f(c));
        let view = QueueView {
            eye,
            forward: self.camera.basis().0.to_array(),
            resident: resident
                .is_some()
                .then_some(&resident_fn as &dyn Fn(ChunkCoord) -> bool),
            samples: self.samples,
            hold: None,
            half_angle: {
                let [w, h] = self.frame.output_size();
                bake::view_half_angle(self.camera.vfov, w as f32 / h as f32)
            },
            frustum: None,
            full_radius: None,
        };
        #[allow(
            clippy::disallowed_methods,
            reason = "the stage test times the step from outside"
        )]
        let t = std::time::Instant::now();
        let st = self
            .frame
            .bake_queue_step(&self.device, &self.queue, &mut self.gpu, &self.world, &view)
            .expect("the queue runs")
            .expect("the queue's step");
        #[allow(
            clippy::disallowed_methods,
            reason = "the stage test times the step from outside"
        )]
        let ms = t.elapsed().as_secs_f64() * 1e3;
        self.last_step_ms = ms;
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("device poll");
        st
    }

    fn step(&mut self) -> QueueStats {
        self.step_with(true)
    }

    fn q(&self) -> &bake::BakeQueue {
        self.frame.bake_queue().expect("the queue started")
    }

    /// Steps until the queue is empty: the steps taken.
    fn settle(&mut self) -> usize {
        for n in 1..=MAX_STEPS {
            self.step();
            if self.q().empty() {
                return n;
            }
        }
        panic!(
            "the queue is not empty after {MAX_STEPS} steps: {:?}",
            self.q().stats()
        );
    }

    /// The pool's copy of `c`'s block (what the frame reads), if it has one.
    fn shown(&self, c: [i32; 3]) -> Option<Vec<u32>> {
        self.frame
            .read_store_block(&self.device, &self.queue, ChunkCoord(c))
    }

    /// The store's own copy of `c`'s block: its target.
    fn target(&self, c: [i32; 3]) -> Option<Vec<u32>> {
        self.frame
            .store()
            .block_words(ChunkCoord(c))
            .map(<[u32]>::to_vec)
    }

    /// Every block of the world's extent, from the pool.
    fn blocks(&self) -> BTreeMap<[i32; 3], Vec<u32>> {
        blocks_of(&self.device, &self.queue, &self.frame, self.world.extent())
    }

    /// A fresh mirror and frame of the world as it is now, its extent baked in one go at the
    /// rig's counts: every block.
    fn fresh(&self) -> BTreeMap<[i32; 3], Vec<u32>> {
        let cfg = MirrorConfig {
            sync_budget_bytes: u64::MAX,
            window: Window::Fixed(self.world.extent()),
            ..self.cfg
        };
        let mut gpu = GpuWorld::with_config(&self.device, cfg);
        gpu.set_focus(VoxelCoord(
            self.camera.position.to_array().map(|v| v.floor() as i32),
        ));
        for _ in 0..2 {
            gpu.sync_now(&self.world, &self.device, &self.queue)
                .expect("the mirror fits");
        }
        let mut frame = new_frame(&self.device, 16 << 20);
        let extent = self.world.extent();
        frame
            .bake_until(
                &self.device,
                &self.queue,
                &mut gpu,
                &self.world,
                extent,
                &self.counts,
                &mut |_| {},
            )
            .expect("the world fits the pool");
        blocks_of(&self.device, &self.queue, &frame, extent)
    }

    /// Applies `op`, mirrors it and tells the queue its box.
    fn edit(&mut self, op: &EditOp) {
        let r = self.world.apply(op).expect("the edit is resident");
        let (min, max) = r.aabb.expect("the edit changed voxels");
        self.gpu
            .sync_now(&self.world, &self.device, &self.queue)
            .expect("the mirror fits");
        self.frame.invalidate(min.0, max.0);
    }

    fn render(&mut self) -> Vec<u8> {
        self.frame.step(
            &self.device,
            &self.queue,
            &self.gpu,
            &self.camera,
            &LightParams::default(),
        );
        readback::read_rgba8(&self.device, &self.queue, self.frame.target())
    }

    /// No two blocks of `chunks` share a pool word.
    fn assert_places_apart(&self, chunks: &[[i32; 3]]) {
        let mut places: Vec<(u64, u64, [i32; 3])> = chunks
            .iter()
            .filter_map(|&c| {
                self.frame
                    .store()
                    .block(ChunkCoord(c))
                    .map(|(f, w)| (f, w, c))
            })
            .collect();
        places.sort_unstable();
        for p in places.windows(2) {
            assert!(
                p[0].0 + p[0].1 <= p[1].0,
                "{:?} at {}..{} and {:?} at {}..{} overlap",
                p[0].2,
                p[0].0,
                p[0].0 + p[0].1,
                p[1].2,
                p[1].0,
                p[1].0 + p[1].1
            );
        }
    }
}

/// `frame`'s block of every chunk of `e` that has one.
fn blocks_of(
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

/// The record words of `block` that hold a direction's value: each surface brick's six
/// direction words.
fn value_words(block: &[u32]) -> impl Iterator<Item = usize> + '_ {
    block[..512].iter().filter(|&&d| d != 0).flat_map(|&d| {
        let end = (d >> 11) as usize;
        end - bake::BRICK_WORDS..end - bake::BRICK_WORDS + 6
    })
}

/// The `rgb9e5` step of word `w`'s exponent.
fn step_of(w: u32) -> f32 {
    2f32.powi((w >> 27) as i32 - 24)
}

/// Two chunks along x: a rock block across their border under an ore plate, the camera in
/// chunk 0 looking at it along +x.
fn border_box() -> (World, [f32; 3]) {
    let world = world_of([2, 1, 1], AIR);
    fill(&world, [40, 20, 20], [88, 40, 40], ROCK);
    fill(&world, [44, 24, 40], [84, 36, 42], ORE);
    (world, [20.5, 30.5, 50.5])
}

/// The two-chunk box through the live queue, coarse pass on (chunk 1 lies beyond the near ring
/// of 0, so it is lit coarse first and refined), until the queue is empty: every block is
/// bit-identical to a one-shot bake at the same counts. Then 32 more frames: the queue submits
/// nothing and reads nothing back, and the last two frames are bit-identical.
#[test]
fn the_queue_settles_to_a_direct_bake() {
    let (world, eye) = border_box();
    let mut q = match rig(
        world,
        &Setup {
            coarse: COARSE,
            near: 0,
            ..Setup::at(eye)
        },
    ) {
        Some(q) => q,
        None => return,
    };
    let mut coarse_seen = 0;
    for n in 1..=MAX_STEPS {
        let st = q.step();
        coarse_seen = coarse_seen.max(st.coarse);
        if q.q().empty() {
            println!("empty after {n} steps: {st:?}");
            break;
        }
    }
    assert!(q.q().empty(), "the queue settles");
    assert_eq!(coarse_seen, 1, "chunk 1 is lit coarse first");
    let got = q.blocks();
    assert_eq!(got.len(), 2, "both chunks hold a block");
    let fresh = q.fresh();
    for (c, b) in &got {
        assert_eq!(Some(b), fresh.get(c), "{c:?} against a one-shot bake");
        assert_eq!(
            q.target(*c).as_ref(),
            Some(b),
            "{c:?}: the pool shows the store's copy"
        );
    }
    let mut frames = Vec::new();
    for _ in 0..32 {
        let st = q.step();
        assert_eq!(
            (st.submitted, st.samples, st.jobs, st.fading),
            (0, 0, 0, 0),
            "an empty queue dispatches nothing"
        );
        frames.push(q.render());
    }
    assert_eq!(frames[30], frames[31], "two frames at rest");
}

/// [`the_queue_settles_to_a_direct_bake`] with the camera turned to look -x half-way: once
/// chunk 1's coarse block lands. Every block is still bit-identical to a one-shot bake.
#[test]
fn the_queue_settles_to_a_direct_bake_after_a_turn() {
    let (world, eye) = border_box();
    let mut q = match rig(
        world,
        &Setup {
            coarse: COARSE,
            near: 0,
            ..Setup::at(eye)
        },
    ) {
        Some(q) => q,
        None => return,
    };
    let mut turned = false;
    for n in 1..=MAX_STEPS {
        let st = q.step();
        if !turned && st.coarse > 0 {
            q.camera.yaw = std::f32::consts::PI;
            turned = true;
            println!("turned after {n} steps");
        }
        if q.q().empty() {
            println!("empty after {n} steps: {st:?}");
            break;
        }
    }
    assert!(turned, "chunk 1 was lit coarse");
    assert!(q.q().empty(), "the queue settles");
    let got = q.blocks();
    assert_eq!(got.len(), 2, "both chunks hold a block");
    let fresh = q.fresh();
    for (c, b) in &got {
        assert_eq!(Some(b), fresh.get(c), "{c:?} against a one-shot bake");
        assert_eq!(
            q.target(*c).as_ref(),
            Some(b),
            "{c:?}: the pool shows the store's copy"
        );
    }
}

/// Six chunks in a row, a rock block in each and the camera in chunk 2 looking along +x, coarse
/// off: the jobs open by squared chunk distance from the camera's chunk, ties by the view (the
/// chunk ahead first), so 2, 3, 1, 4, 0, 5. At the smallest slice no step's submit plans more
/// than the step's cap but for the one entry (a round, 64 samples) that crosses it.
#[test]
fn the_queue_runs_near_first_and_within_its_samples() {
    let Some(mut q) = rig(
        row(&[0, 1, 2, 3, 4, 5]),
        &Setup {
            slice_ms: 1e-6,
            ..Setup::at(ROW_EYE)
        },
    ) else {
        return;
    };
    let mut order: Vec<i32> = Vec::new();
    let mut steps = 0;
    loop {
        let st = q.step();
        steps += 1;
        assert!(steps <= MAX_STEPS, "the queue settles");
        assert!(
            st.submitted < st.cap + 64,
            "step {steps}: {} samples planned against the cap {}",
            st.submitted,
            st.cap
        );
        for c in q.q().jobs() {
            if !order.contains(&c.0[0]) {
                order.push(c.0[0]);
            }
        }
        if q.q().empty() {
            break;
        }
    }
    println!("{steps} steps, opened {order:?}");
    assert!(steps > 3, "the cap spreads the work over steps");
    assert_eq!(order, [2, 3, 1, 4, 0, 5]);
}

/// Six chunks in a row along x, a rock block in each chunk of `rocks` (16 voxels from its
/// borders) and an ore plate in chunk 2.
fn row(rocks: &[i64]) -> World {
    let world = world_of([6, 1, 1], AIR);
    for &x in rocks {
        fill(&world, [x * 64 + 16, 16, 16], [x * 64 + 48, 48, 48], ROCK);
    }
    fill(&world, [140, 28, 48], [150, 36, 50], ORE);
    world
}

/// The row's camera, in chunk 2.
const ROW_EYE: [f32; 3] = [150.5, 32.5, 60.5];

/// The score at angle weight `angle` and refine weight 2.
fn score(angle: f32) -> bake::QueueOrder {
    bake::QueueOrder::Score {
        angle,
        refine: bake::Refine::Weight(2.0),
    }
}

/// Steps until the queue is empty: the chunks' x in the order they first appear in `jobs()`.
fn open_order(q: &mut Q) -> Vec<i32> {
    let mut order = Vec::new();
    for _ in 0..MAX_STEPS {
        q.step();
        for c in q.q().jobs() {
            if !order.contains(&c.0[0]) {
                order.push(c.0[0]);
            }
        }
        if q.q().empty() {
            return order;
        }
    }
    panic!("the queue is not empty after {MAX_STEPS} steps");
}

/// ADR-0048's score on the row, camera in chunk 2 looking +x, `near` 0, coarse pass on, refine
/// weight 2: the chunks first open by `|o| x (1 + W x a)`, behind the camera `a` 0.632. At
/// angle weight 4: ahead 1, 2, 3, behind 1 x 3.53 and 2 x 3.53, so 2, 3, 4, 5, 1, 0; at 1:
/// behind 1.63 and 3.26, so 2, 3, 1, 4, 5, 0; at 0 the distance, ties ahead first, so 2, 3, 1,
/// 4, 0, 5, as under the distance order.
#[test]
fn the_score_blends_distance_and_the_view() {
    for (order, want) in [
        (score(4.0), [2, 3, 4, 5, 1, 0]),
        (score(1.0), [2, 3, 1, 4, 5, 0]),
        (score(0.0), [2, 3, 1, 4, 0, 5]),
        (bake::QueueOrder::Distance, [2, 3, 1, 4, 0, 5]),
    ] {
        let Some(mut q) = rig(
            row(&[0, 1, 2, 3, 4, 5]),
            &Setup {
                coarse: COARSE,
                near: 0,
                slice_ms: 1e-6,
                order: Some(order),
                ..Setup::at(ROW_EYE)
            },
        ) else {
            return;
        };
        let got = open_order(&mut q);
        println!("{order:?}: {got:?}");
        assert_eq!(got, want, "{order:?}");
    }
}

/// The row with chunk 3 holding a coarse block and chunk 5 none, the rest full, camera in chunk
/// 2 looking +x, `near` 0, refine weight 2: under the score chunk 3's refinement (1 x 2 = 2)
/// opens before chunk 5's first pass (3); with the class as a tier, and under the distance
/// order, chunk 5 opens first. Chunks 3 and 5 start empty and are filled by edits: 3 first,
/// stepped until its coarse block lands, then 5.
#[test]
fn a_blank_chunk_waits_behind_a_nearer_refinement_only_under_the_score() {
    for (order, first) in [
        (score(4.0), 3),
        (
            bake::QueueOrder::Score {
                angle: 4.0,
                refine: bake::Refine::Tier,
            },
            5,
        ),
        (bake::QueueOrder::Distance, 5),
    ] {
        let Some(mut q) = rig(
            row(&[0, 1, 2, 4]),
            &Setup {
                coarse: COARSE,
                near: 0,
                slice_ms: 1e-6,
                order: Some(order),
                ..Setup::at(ROW_EYE)
            },
        ) else {
            return;
        };
        q.settle();
        q.edit(&box_op([208, 16, 16], [240, 48, 48], Mode::Fill(ROCK)));
        let three = ChunkCoord([3, 0, 0]);
        let mut steps = 0;
        while !(q.frame.store().is_coarse(three) && !q.q().jobs().contains(&three)) {
            q.step();
            steps += 1;
            assert!(steps < MAX_STEPS, "chunk 3's coarse block lands");
        }
        q.edit(&box_op([336, 16, 16], [368, 48, 48], Mode::Fill(ROCK)));
        let mut seen: Vec<i32> = Vec::new();
        for _ in 0..MAX_STEPS {
            q.step();
            for c in q.q().jobs() {
                if (c.0[0] == 3 || c.0[0] == 5) && !seen.contains(&c.0[0]) {
                    seen.push(c.0[0]);
                }
            }
            if seen.len() == 2 {
                break;
            }
        }
        println!("{order:?}: {seen:?}");
        assert_eq!(seen.first(), Some(&first), "{order:?}: {seen:?}");
    }
}

/// The row at `near` 0 and angle weight 4, the camera turned to look -x once its own chunk
/// opens: of the chunks not yet opened at the turn, 1 and 0 (now ahead) open before 4 and 5.
/// Coarse off and 4096 samples a brick direction, so a chunk's job (about 3.9 x 10^5 samples)
/// fills the loop's two slices of 16 384 alone and the chunks open one at a time.
#[test]
fn a_turn_resorts_the_queue() {
    let Some(mut q) = rig(
        row(&[0, 1, 2, 3, 4, 5]),
        &Setup {
            near: 0,
            slice_ms: 1e-6,
            order: Some(score(4.0)),
            counts: BakeSettings {
                brick_samples: 4096,
                ..counts()
            },
            ..Setup::at(ROW_EYE)
        },
    ) else {
        return;
    };
    let mut before: Vec<i32> = Vec::new();
    while !before.contains(&2) {
        q.step();
        before.extend(q.q().jobs().iter().map(|c| c.0[0]));
        assert!(before.len() < 64, "chunk 2 opens");
    }
    q.camera.yaw = std::f32::consts::PI;
    let mut after: Vec<i32> = Vec::new();
    for _ in 0..MAX_STEPS {
        q.step();
        for c in q.q().jobs() {
            let x = c.0[0];
            if !before.contains(&x) && !after.contains(&x) {
                after.push(x);
            }
        }
        if q.q().empty() {
            break;
        }
    }
    println!("opened before the turn {before:?}, after {after:?}");
    let at = |x: i32| after.iter().position(|&v| v == x);
    for behind in [4, 5] {
        for ahead in [1, 0] {
            if let Some(b) = at(behind) {
                let a = at(ahead).expect("a chunk ahead opens");
                assert!(a < b, "{ahead} before {behind}: {after:?}");
            }
        }
    }
    assert!(at(1).is_some() && at(0).is_some(), "{before:?} {after:?}");
}

/// Eight chunks in a row, an 8^3 rock cube on one brick of each, its top layer ore (6 brick
/// directions, 24 576 samples a chunk at 4096 a brick direction), and a rig on it at `near` 0,
/// coarse off, the smallest slice (a step draws the cap's floor, 16 384 samples, and an opening
/// admits jobs while under two caps are left), the camera in chunk 0 looking +x.
fn cubes_rig() -> Option<Q> {
    let world = world_of([8, 1, 1], AIR);
    for x in 0..8 {
        fill(&world, [x * 64 + 24, 24, 24], [x * 64 + 32, 32, 31], ROCK);
        fill(&world, [x * 64 + 24, 24, 31], [x * 64 + 32, 32, 32], ORE);
    }
    rig(
        world,
        &Setup {
            near: 0,
            slice_ms: 1e-6,
            counts: BakeSettings {
                brick_samples: 4096,
                ..counts()
            },
            ..Setup::at([32.5, 32.5, 50.5])
        },
    )
}

impl Q {
    /// Every pool word is the header's, a held block's, a job's place or free.
    fn assert_pool_accounted(&self) {
        let store = self.frame.store();
        let used = store.stats(0, 0, 0).bytes / 4;
        assert_eq!(
            used + store.free_words() + self.q().job_words(),
            store.pool().size() / 4,
            "every pool word is the header's, a block's, a job's or free"
        );
    }

    /// Steps until `jobs()` holds chunk `x` of the row: the steps taken.
    fn step_until_job(&mut self, x: i32) -> usize {
        for n in 1..=MAX_STEPS {
            self.step();
            if self.q().jobs().iter().any(|c| c.0[0] == x) {
                return n;
            }
        }
        panic!("chunk {x} never opened");
    }
}

/// The cubes' row: the camera steps in chunk 0 until chunk 2's job is in the loop (with chunk
/// 1's, open and partly drawn), then moves to chunk 5 looking +x. At the next step the loop
/// holds no job of chunks 0 to 4, the withdrawn count and samples are above 0, each withdrawn
/// chunk holds no block it did not hold before and nothing of it is left but its listing, and
/// every pool word is accounted for. Run to empty, every block is bit-identical to a one-shot
/// bake.
#[test]
fn a_passed_chunk_is_withdrawn() {
    let Some(mut q) = cubes_rig() else {
        return;
    };
    let steps = q.step_until_job(2);
    let held: Vec<[i32; 3]> = q.q().jobs().iter().map(|c| c.0).collect();
    let had: Vec<bool> = (0..5)
        .map(|x| q.frame.store().block(ChunkCoord([x, 0, 0])).is_some())
        .collect();
    println!("chunk 2 opened after {steps} steps; jobs {held:?}, blocks {had:?}");
    assert!(
        held.contains(&[1, 0, 0]) && held.contains(&[2, 0, 0]),
        "{held:?}"
    );
    q.camera.position.x = 5.0 * 64.0 + 32.5;
    let st = q.step();
    let jobs: Vec<i32> = q.q().jobs().iter().map(|c| c.0[0]).collect();
    println!("after the move: jobs {jobs:?}, {st:?}");
    assert!(jobs.iter().all(|x| !(0..5).contains(x)), "{jobs:?}");
    assert!(st.withdrawn > 0, "{st:?}");
    assert!(st.withdrawn_samples > 0, "{st:?}");
    for c in &held {
        if (0..5).contains(&c[0]) {
            let block = q.frame.store().block(ChunkCoord(*c));
            assert!(
                block.is_none() || had[c[0] as usize],
                "{c:?} holds a block it did not hold"
            );
            let tracks = q.q().tracks(ChunkCoord(*c));
            assert!(
                tracks.iter().all(|t| *t == "listed"),
                "{c:?} left behind {tracks:?}"
            );
        }
    }
    q.assert_pool_accounted();
    q.settle();
    let got = q.blocks();
    assert_eq!(got.len(), 8, "every chunk holds a block");
    let fresh = q.fresh();
    for (c, b) in &got {
        assert_eq!(Some(b), fresh.get(c), "{c:?} against a one-shot bake");
    }
    q.assert_pool_accounted();
}

/// The cubes' row as in [`a_passed_chunk_is_withdrawn`], but the step after the move has a
/// sample count, as the pre-roll's steps do: it withdraws nothing, and chunk 1's and 2's jobs
/// that did not end in it are still in the loop.
#[test]
fn a_pre_roll_step_withdraws_nothing() {
    let Some(mut q) = cubes_rig() else {
        return;
    };
    q.step_until_job(2);
    let held: Vec<[i32; 3]> = q.q().jobs().iter().map(|c| c.0).collect();
    assert!(
        held.contains(&[1, 0, 0]) && held.contains(&[2, 0, 0]),
        "{held:?}"
    );
    q.camera.position.x = 5.0 * 64.0 + 32.5;
    q.samples = Some(16_384);
    let st = q.step();
    let jobs: Vec<[i32; 3]> = q.q().jobs().iter().map(|c| c.0).collect();
    println!("after the move: jobs {jobs:?}, {st:?}");
    assert_eq!(st.withdrawn, 0, "{st:?}");
    for c in &held {
        assert!(
            jobs.contains(c) || q.frame.store().block(ChunkCoord(*c)).is_some(),
            "{c:?} left the loop with no block"
        );
    }
    q.assert_pool_accounted();
}

/// The cubes' row with no extra slice for digs and a carve in chunk 1 once its block lands,
/// before the move to chunk 5: the dig's job stays in the loop after the move, and the dig's
/// chunk settles to a one-shot bake of the edited world.
#[test]
fn a_dig_is_never_withdrawn() {
    let Some(mut q) = cubes_rig() else {
        return;
    };
    // No extra slice for digs: the dig's job draws the cap's floor a step, so it is still in
    // the loop at the step after the move.
    q.frame.bake_queue_mut().expect("the queue started").edit_ms = 0.0;
    let one = ChunkCoord([1, 0, 0]);
    let mut n = 0;
    while q.frame.store().block(one).is_none() {
        q.step();
        n += 1;
        assert!(n < MAX_STEPS, "chunk 1 is baked");
    }
    q.edit(&box_op([88, 24, 24], [92, 28, 32], Mode::Carve));
    q.step();
    assert!(
        q.q().tracks(one).contains(&"dig_open"),
        "{:?}",
        q.q().tracks(one)
    );
    q.camera.position.x = 5.0 * 64.0 + 32.5;
    let st = q.step();
    println!("after the move: jobs {:?}, {st:?}", q.q().jobs());
    assert!(q.q().jobs().contains(&one), "the dig's job stays");
    assert!(q.q().tracks(one).contains(&"dig_open"));
    q.settle();
    assert_eq!(
        q.blocks().get(&one.0),
        q.fresh().get(&one.0),
        "the dig's chunk against a one-shot bake of the edited world"
    );
}

/// A 4 x 4 chunk floor of 32^3 rock blocks at 4096 samples a brick direction (a job of about
/// 3.9 x 10^5 samples, many steps), the camera in chunk [0, 0, 0] yawing by plus and minus 5
/// degrees a step about the view cone's edge on +x: the chunks along +x leave the view cone
/// and come back, and stay in the keep cone. Over 64 steps nothing is withdrawn, while a job's
/// chunk lies out of the view cone at some step.
#[test]
fn the_edge_of_the_view_does_not_churn() {
    let world = world_of([4, 4, 1], AIR);
    for x in 0..4 {
        for y in 0..4 {
            fill(
                &world,
                [x * 64 + 16, y * 64 + 16, 16],
                [x * 64 + 48, y * 64 + 48, 48],
                ROCK,
            );
        }
    }
    fill(&world, [20, 20, 48], [30, 30, 50], ORE);
    let Some(mut q) = rig(
        world,
        &Setup {
            near: 0,
            slice_ms: 1e-6,
            counts: BakeSettings {
                brick_samples: 4096,
                ..counts()
            },
            ..Setup::at([32.5, 32.5, 32.5])
        },
    ) else {
        return;
    };
    let edge = bake::view_half_angle(70f32.to_radians(), 16.0 / 9.0)
        + bake::QUEUE_VIEW_MARGIN_DEG.to_radians();
    let mut out_of_view = 0;
    for n in 0..64 {
        let sign = if n % 2 == 0 { 1.0 } else { -1.0 };
        q.camera.yaw = edge + sign * 5f32.to_radians();
        let st = q.step();
        assert_eq!(st.withdrawn, 0, "step {n}: {st:?}");
        let forward = q.camera.basis().0.to_array();
        out_of_view += q
            .q()
            .jobs()
            .iter()
            .filter(|c| {
                bake::angle_term(
                    c.0,
                    forward,
                    edge - bake::QUEUE_VIEW_MARGIN_DEG.to_radians(),
                    0,
                ) > 0.0
            })
            .count();
    }
    println!("job-steps out of the view cone: {out_of_view}");
    assert!(out_of_view > 0, "a job's chunk left the view cone");
}

/// A record fading from `from` to `to` over `frames` steps, shown each step: every value word
/// moves by at most `|to - from| / frames` a step plus one `rgb9e5` step, and the last shows `to`.
fn assert_fade(what: &str, from: &[u32], shown: &[Vec<u32>], to: &[u32], frames: u32) {
    assert!(
        shown.len() <= frames as usize,
        "{what}: {} steps to reach the target, FADE {frames}",
        shown.len()
    );
    assert_eq!(
        shown.last().map(Vec::as_slice),
        Some(to),
        "{what}: ends at the target"
    );
    let mut prev = from;
    for (k, now) in shown.iter().enumerate() {
        for w in value_words(to) {
            let (a, b) = (bake::rgb9e5_unpack(prev[w]), bake::rgb9e5_unpack(now[w]));
            let (x, y) = (bake::rgb9e5_unpack(from[w]), bake::rgb9e5_unpack(to[w]));
            let ulp = step_of(prev[w]).max(step_of(now[w]));
            for ch in 0..3 {
                let share = (y[ch] - x[ch]).abs() / frames as f32;
                assert!(
                    (b[ch] - a[ch]).abs() <= share + ulp * 1.0001,
                    "{what}, step {k}, word {w} channel {ch}: moved {} for a share {share} and a step {ulp}",
                    (b[ch] - a[ch]).abs()
                );
            }
        }
        prev = now;
    }
}

/// A fade as [`record_fades`] saw it: the block shown before it, each step's, and the target.
type Fade = (Vec<u32>, Vec<Vec<u32>>, Vec<u32>);

/// Steps the queue until it is empty, recording `c`'s pool block each step from the step its
/// target changes: each fade's `from` (the block shown the step before), the blocks shown and
/// its target.
fn record_fades(q: &mut Q, c: [i32; 3]) -> Vec<Fade> {
    let mut fades = Vec::new();
    let mut shown_before = q.shown(c);
    let mut target_before = q.target(c);
    let mut open: Option<Fade> = None;
    for _ in 0..MAX_STEPS {
        q.step();
        let (shown, target) = (q.shown(c), q.target(c));
        if target != target_before {
            if let Some(f) = open.take() {
                fades.push(f);
            }
            if let (Some(from), Some(s), Some(t)) = (&shown_before, &shown, &target)
                && from.len() == t.len()
                && from[..512] == t[..512]
                && s != t
            {
                open = Some((from.clone(), Vec::new(), t.clone()));
            }
        }
        if let (Some(f), Some(s)) = (open.as_mut(), &shown) {
            f.1.push(s.clone());
            if Some(s) == target.as_ref() {
                fades.push(open.take().expect("open"));
            }
        }
        shown_before = shown;
        target_before = target;
        if q.q().empty() && open.is_none() {
            break;
        }
    }
    assert!(open.is_none(), "a fade still runs when the queue is empty");
    fades
}

/// The coarse block's refinement fades over `FADE` steps, and a dig's job (from the dig's
/// provisional block) over `DIG_FADE`: each record moves at most its share a step plus one
/// `rgb9e5` step, and ends at the new block.
#[test]
fn a_fade_moves_a_record_by_its_share_a_frame() {
    let (world, eye) = border_box();
    let Some(mut q) = rig(
        world,
        &Setup {
            coarse: COARSE,
            near: 0,
            ..Setup::at(eye)
        },
    ) else {
        return;
    };
    let refined = record_fades(&mut q, [1, 0, 0]);
    assert_eq!(refined.len(), 1, "chunk 1's coarse block is refined once");
    let (from, shown, to) = &refined[0];
    assert!(
        value_words(to).any(|w| from[w] != to[w]),
        "the refinement changes a record"
    );
    assert_fade("the refinement", from, shown, to, bake::FADE);
    // A carve in the lit top of the box, inside chunk 1.
    q.edit(&box_op([70, 26, 37], [74, 30, 40], Mode::Carve));
    let dug = record_fades(&mut q, [1, 0, 0]);
    assert!(!dug.is_empty(), "the dig's job fades in");
    for (k, (from, shown, to)) in dug.iter().enumerate() {
        assert_fade(&format!("dig fade {k}"), from, shown, to, bake::DIG_FADE);
    }
}

/// Chunk 0's wall faces an ore block in chunk 1 across their border. With the mirror taking one
/// chunk a sync, nearest the focus first, chunk 0 is mirrored and chunk 1 waits: chunk 0 is not
/// baked (its reach holds a chunk waiting to upload) and is held as blocked. Once chunk 1 is
/// mirrored, both bake, bit-identical to a one-shot bake with both present from the start, and
/// chunk 0's wall is lit.
#[test]
fn a_chunk_waits_for_its_reach() {
    let world = world_of([2, 1, 1], AIR);
    fill(&world, [40, 8, 8], [48, 56, 56], ROCK);
    fill(&world, [70, 28, 28], [74, 36, 36], ORE);
    let Some(mut q) = rig(
        world,
        &Setup {
            sync_budget: 1,
            ..Setup::at([20.5, 32.5, 32.5])
        },
    ) else {
        return;
    };
    let (c0, c1) = (ChunkCoord([0, 0, 0]), ChunkCoord([1, 0, 0]));
    assert!(
        q.gpu.is_mirrored(c0) && q.gpu.is_pending(c1),
        "one chunk a sync"
    );
    for _ in 0..40 {
        q.step_with(false);
    }
    assert!(q.gpu.is_pending(c1));
    assert_eq!(
        q.frame.store().block(c0),
        None,
        "chunk 0 waits for its reach"
    );
    assert_eq!(q.q().tracks(c0), ["blocked"]);
    let n = q.settle();
    println!("settled {n} steps after chunk 1 arrived");
    let got = q.blocks();
    assert_eq!(got.len(), 2);
    assert_eq!(got, q.fresh(), "against a one-shot bake with both present");
    let lit = value_words(&got[&c0.0])
        .filter(|&w| got[&c0.0][w] != 0)
        .count();
    assert!(lit > 0, "chunk 0's wall is lit by chunk 1's ore");
}

/// A window of 3 chunks following the focus over four chunks in a row (chunk 3 air): with the
/// focus at chunk 1 the queue bakes chunks 0 to 2. Moving the focus to chunk 2 drops chunk 0
/// from the mirror: its block is gone, no per-chunk set of the queue holds it, chunk 3 (air)
/// is known faceless, and once the queue is empty every pool word is a block's, the header's or
/// free, so chunk 0's place was not leaked. Moving back to chunk 1 drops chunk 3 the same way.
#[test]
fn an_evicted_chunk_frees_its_block_and_its_bookkeeping() {
    let world = world_of([4, 1, 1], AIR);
    for x in 0..3 {
        fill(&world, [x * 64 + 16, 16, 16], [x * 64 + 48, 48, 48], ROCK);
    }
    fill(&world, [80, 28, 48], [90, 36, 50], ORE);
    let Some(mut q) = rig(
        world,
        &Setup {
            window: Some(Window::Follow { side: 3 }),
            ..Setup::at([96.5, 32.5, 32.5])
        },
    ) else {
        return;
    };
    let all: Vec<[i32; 3]> = (-1..5).map(|x| [x, 0, 0]).collect();
    let accounted = |q: &Q| {
        let store = q.frame.store();
        let used = store.stats(0, 0, 0).bytes / 4;
        assert_eq!(
            used + store.free_words(),
            store.pool().size() / 4,
            "every pool word is the header's, a block's or free"
        );
    };
    q.settle();
    assert!(q.frame.store().block(ChunkCoord([0, 0, 0])).is_some());
    accounted(&q);
    q.camera.position.x = 160.5;
    q.step();
    let gone = ChunkCoord([0, 0, 0]);
    assert!(!q.gpu.is_mirrored(gone));
    assert_eq!(
        q.frame.store().block(gone),
        None,
        "the evicted block is dropped"
    );
    q.settle();
    assert_eq!(
        q.q().tracks(gone),
        Vec::<&str>::new(),
        "chunk 0 left behind"
    );
    assert_eq!(q.q().tracks(ChunkCoord([3, 0, 0])), ["faceless"]);
    q.assert_places_apart(&all);
    accounted(&q);
    q.camera.position.x = 96.5;
    q.settle();
    let gone = ChunkCoord([3, 0, 0]);
    assert!(!q.gpu.is_mirrored(gone));
    assert_eq!(
        q.q().tracks(gone),
        Vec::<&str>::new(),
        "chunk 3 left behind"
    );
    assert!(
        q.frame.store().block(ChunkCoord([0, 0, 0])).is_some(),
        "chunk 0 baked again"
    );
    q.assert_places_apart(&all);
    accounted(&q);
}

// ---- Digs on the queue: rings 1 to 3 ----

/// A rock bar of two chunks with a tunnel along x: an ore block at its west end, a plug at
/// x 30..32, and the tunnel's far end past x 72. The camera sits in the tunnel west of the plug.
fn tunnel() -> (World, [f32; 3]) {
    let world = world_of([2, 1, 1], ROCK);
    world
        .apply(&box_op([8, 28, 28], [110, 34, 34], Mode::Carve))
        .expect("the tunnel");
    fill(&world, [10, 30, 30], [12, 32, 32], ORE);
    fill(&world, [30, 28, 28], [32, 34, 34], ROCK);
    (world, [20.5, 31.5, 31.5])
}

/// The records of bricks at world brick x 9 and beyond (voxel x from 72): the tunnel's far end.
fn far_end(blocks: &BTreeMap<[i32; 3], Vec<u32>>) -> BTreeMap<([i32; 3], usize), Vec<u32>> {
    let mut out = BTreeMap::new();
    for (&c, block) in blocks {
        for (slot, &d) in block[..512].iter().enumerate() {
            let b = vxe_world::brick_of_slot(slot);
            if d == 0 || c[0] * 8 + b[0] < 9 {
                continue;
            }
            let end = (d >> 11) as usize;
            out.insert((c, slot), block[end - bake::BRICK_WORDS..end].to_vec());
        }
    }
    out
}

/// The tunnel's far end, dark behind the plug. A window carved in the plug (a lit brick
/// touched, so ring 3 re-bakes the chunks within the light radius): once the queue is empty
/// the far end's records equal a one-shot bake's of the opened world, and some are lit. The
/// plug put back: once the queue is empty they equal the first bake's again.
#[test]
fn a_break_through_reaches_the_far_wall() {
    let (world, eye) = tunnel();
    let Some(mut q) = rig(world, &Setup::at(eye)) else {
        return;
    };
    q.settle();
    let dark = far_end(&q.blocks());
    assert!(dark.len() >= 20, "{} far bricks", dark.len());
    assert_eq!(
        dark,
        far_end(&q.fresh()),
        "the first bake against a one-shot bake"
    );
    q.edit(&box_op([30, 30, 30], [32, 32, 32], Mode::Carve));
    let mut reach = 0;
    while !q.q().empty() {
        reach = reach.max(q.step().reach);
    }
    assert!(reach > 0, "the window opens ring 3");
    let open = far_end(&q.blocks());
    assert_eq!(
        open,
        far_end(&q.fresh()),
        "the far end against a one-shot bake"
    );
    let lit = open
        .values()
        .filter(|r| r[..6].iter().any(|&w| w != 0))
        .count();
    let lit_before = dark
        .values()
        .filter(|r| r[..6].iter().any(|&w| w != 0))
        .count();
    println!(
        "far bricks {}, lit {lit_before} before, {lit} through the window",
        open.len()
    );
    assert!(lit > lit_before, "the window lights the far end");
    q.edit(&box_op([30, 30, 30], [32, 32, 32], Mode::Fill(ROCK)));
    q.settle();
    assert_eq!(far_end(&q.blocks()), dark, "the plug put back");
}

/// A carve in the tunnel's dark far end, which no emitter reaches past the plug: the queue
/// re-bakes rings 1 and 2 and opens no ring-3 chunk, and its blocks equal a one-shot bake's.
#[test]
fn a_dark_carve_opens_no_reach() {
    let (world, eye) = tunnel();
    let Some(mut q) = rig(world, &Setup::at(eye)) else {
        return;
    };
    q.settle();
    let before = q.frame.store().block(ChunkCoord([1, 0, 0]));
    q.edit(&box_op([100, 30, 30], [104, 34, 37], Mode::Carve));
    let (mut digs, mut reach, mut baked) = (0, 0, BTreeSet::new());
    while !q.q().empty() {
        let st = q.step();
        digs = digs.max(st.digs);
        reach = reach.max(st.reach);
        baked.extend(q.q().jobs().into_iter().map(|c| c.0));
        assert!(baked.len() < 1000);
    }
    println!("digs {digs}, reach {reach}, jobs {baked:?}, chunk 1 at {before:?}");
    assert!(digs > 0, "the carve's rings re-bake");
    assert_eq!(reach, 0, "a dark carve opens no ring 3");
    assert_eq!(q.blocks(), q.fresh(), "against a one-shot bake");
}

/// A rock slab under one ore block, in one chunk. Once the queue is empty the slab's top is
/// lit; the ore carved away, once the queue is empty every record of the chunk reads 0.
#[test]
fn an_emitter_dug_away_darkens_what_it_lit() {
    let world = world_of([1, 1, 1], AIR);
    fill(&world, [8, 8, 20], [40, 40, 21], ROCK);
    fill(&world, [22, 22, 30], [24, 24, 32], ORE);
    let Some(mut q) = rig(world, &Setup::at([30.5, 30.5, 50.5])) else {
        return;
    };
    q.settle();
    let block = &q.blocks()[&[0, 0, 0]];
    let lit = value_words(block).filter(|&w| block[w] != 0).count();
    // The slab's 16 bricks face the ore with their +z.
    assert!(lit >= 16, "{lit} lit directions");
    q.edit(&box_op([22, 22, 30], [24, 24, 32], Mode::Carve));
    q.settle();
    let block = &q.blocks()[&[0, 0, 0]];
    let still: Vec<usize> = value_words(block).filter(|&w| block[w] != 0).collect();
    assert!(still.is_empty(), "{} words still lit", still.len());
}

/// Four chunks in a row: rock blocks of 2 x 2 x 3 bricks in chunk 1, 3 x 3 x 3 in chunk 2 and
/// 2 x 2 x 2 in chunk 3, so chunk 1's block is a little larger than chunk 3's, and chunk 2's
/// grows when a carve opens its centre brick.
fn pool_world() -> World {
    let world = world_of([4, 1, 1], AIR);
    fill(&world, [80, 16, 16], [96, 32, 40], ROCK);
    fill(&world, [136, 16, 16], [160, 40, 40], ROCK);
    fill(&world, [208, 16, 16], [224, 32, 32], ROCK);
    world
}

/// [`pool_world`] under a window of 3 chunks following the focus, the pool sized so that, with
/// the camera at chunk 2, chunks 2 (full count) and 3 (coarse) take places and chunk 1's block
/// does not fit. While the pool refuses chunk 1 the queue goes on: it refines chunk 3 (a place
/// of chunk 3's size fits), reads it back and fades it, and no step fails. A carve into chunk
/// 2's centre brick asks for a larger provisional block, which the pool refuses: chunk 2 keeps
/// its block where the chunk table points and its dig waits. No two chunks ever hold the same
/// pool word. The camera moved to chunk 1 drops chunk 3, whose freed place merges with the room
/// left: the dig re-bakes and chunk 1 bakes.
#[test]
fn a_full_pool_keeps_the_queue_alive() {
    let eye = [168.5, 28.5, 56.5];
    let setup = Setup {
        window: Some(Window::Follow { side: 3 }),
        coarse: COARSE,
        near: 0,
        ..Setup::at(eye)
    };
    // The blocks' sizes, from a queue with room.
    let Some(mut probe) = rig(pool_world(), &setup) else {
        return;
    };
    let size = |q: &Q, x: i32| {
        q.frame
            .store()
            .block(ChunkCoord([x, 0, 0]))
            .expect("a block")
            .1
    };
    probe.settle();
    let (w2, w3) = (size(&probe, 2), size(&probe, 3));
    probe.camera.position.x = 96.5;
    probe.settle();
    let w1 = size(&probe, 1);
    let head = probe.frame.store().stats(0, 0, 0).bytes / 4 - w1 - w2;
    println!("words: header {head}, chunk 1 {w1}, 2 {w2}, 3 {w3}");
    drop(probe);
    let spare = w3 + 4;
    assert!(
        w1 > spare && w1 <= spare + w3,
        "chunk 1's block fits only with chunk 3's place"
    );
    assert!(w2 >= w1, "chunk 2's place would hold chunk 1's block");
    let Some(mut q) = rig(
        pool_world(),
        &Setup {
            pool_bytes: (head + w2 + w3 + spare) * 4,
            ..setup
        },
    ) else {
        return;
    };
    let all: Vec<[i32; 3]> = (0..5).map(|x| [x, 0, 0]).collect();
    let (c1, c2, c3) = (
        ChunkCoord([1, 0, 0]),
        ChunkCoord([2, 0, 0]),
        ChunkCoord([3, 0, 0]),
    );
    let (mut faded, mut st) = (0, QueueStats::default());
    for _ in 0..200 {
        st = q.step();
        q.assert_places_apart(&all);
        faded = faded.max(st.fading);
        if st.jobs == 0 && st.fading == 0 && !q.frame.store().is_coarse(c3) {
            break;
        }
    }
    println!(
        "refused {}, faded {faded}, baked {}",
        st.pool_full, st.baked
    );
    assert!(st.pool_full > 0, "the pool refuses chunk 1");
    assert_eq!(q.frame.store().block(c1), None);
    assert!(faded > 0, "chunk 3's refinement fades while chunk 1 waits");
    assert!(!q.frame.store().is_coarse(c3), "chunk 3 refined");
    assert_eq!(
        st.baked, 3,
        "chunk 2, chunk 3's coarse pass and its refinement"
    );
    let place = q.frame.store().block(c2);
    let refused = st.pool_full;
    q.edit(&box_op([146, 26, 30], [150, 30, 41], Mode::Carve));
    for _ in 0..20 {
        st = q.step();
        q.assert_places_apart(&all);
    }
    assert!(st.pool_full > refused, "the dig's larger block is refused");
    assert_eq!(st.digs, 1, "chunk 2's dig waits");
    assert_eq!(q.frame.store().block(c2), place, "chunk 2 keeps its block");
    q.camera.position.x = 96.5;
    for _ in 0..200 {
        st = q.step();
        q.assert_places_apart(&all);
        if q.frame.store().block(c1).is_some() && st.jobs == 0 && st.digs == 0 {
            break;
        }
    }
    assert_eq!(q.frame.store().block(c3), None, "chunk 3 evicted");
    assert_eq!(st.digs, 0, "the dig re-baked once chunk 3's place was free");
    assert!(
        q.frame.store().block(c1).is_some(),
        "chunk 1 bakes once the dig's old place is free"
    );
}

/// 20 steps of the row with the coarse pass on from the start, timed from outside each
/// `bake_queue_step` call: each step's stages and opening flag, its own time, and the queue's
/// openings before and after.
fn staged_steps() -> Option<(Vec<(bake::StepStages, f64)>, u64)> {
    let mut q = rig(
        row(&[0, 1, 2, 3, 4, 5]),
        &Setup {
            coarse: COARSE,
            ..Setup::at(ROW_EYE)
        },
    )?;
    let before = q.q().stats().openings;
    let mut steps = Vec::new();
    for _ in 0..20 {
        let st = q.step();
        steps.push((st.stages, q.last_step_ms));
    }
    let openings = q.q().stats().openings - before;
    Some((steps, openings))
}

/// Over 20 steps the stages' sum is within 5 % of the steps' own time (each call timed from
/// outside), and no stage reads negative.
#[test]
fn the_step_s_stages_sum_to_its_time() {
    let Some((steps, _)) = staged_steps() else {
        return;
    };
    let sum: f64 = steps.iter().map(|(s, _)| s.sum()).sum();
    let own: f64 = steps.iter().map(|(_, ms)| ms).sum();
    println!("stages {sum:.4} ms, own {own:.4} ms over 20 steps");
    for (s, _) in &steps {
        for (k, ms) in s.ms.iter().enumerate() {
            assert!(*ms >= 0.0, "{} reads {ms}", bake::STEP_STAGES[k]);
        }
    }
    assert!(
        (sum - own).abs() <= 0.05 * own,
        "stages {sum} ms against the steps' {own} ms"
    );
}

/// Over 20 steps of which some open and some do not, the opening and other counts sum to 20
/// and match the steps whose opening ran (the queue's count of openings), and each side's
/// stage sum is within 5 % of its steps' own time.
#[test]
fn the_stages_split_by_opening() {
    let Some((steps, openings)) = staged_steps() else {
        return;
    };
    let (open, not): (Vec<_>, Vec<_>) = steps.iter().partition(|(s, _)| s.opened);
    println!(
        "{} opened, {} not, {openings} openings",
        open.len(),
        not.len()
    );
    assert!(
        !open.is_empty() && !not.is_empty(),
        "the rig has both kinds of step"
    );
    assert_eq!(open.len() + not.len(), 20);
    assert_eq!(open.len() as u64, openings);
    for (what, side) in [("opening", &open), ("other", &not)] {
        let sum: f64 = side.iter().map(|(s, _)| s.sum()).sum();
        let own: f64 = side.iter().map(|(_, ms)| ms).sum();
        println!("{what}: stages {sum:.4} ms, own {own:.4} ms");
        assert!(
            (sum - own).abs() <= 0.05 * own,
            "{what}: stages {sum} ms against the steps' {own} ms"
        );
        assert!(
            side.iter().all(|(s, _)| s.opened == (what == "opening")),
            "{what}: the flag"
        );
    }
}

/// `n` chunks in a row along x, a rock block in each and an ore plate in chunk 2, mirrored one
/// chunk a sync nearest the camera (in chunk 2) first, `syncs` times: 7 syncs of 8, chunk 7
/// waits to upload and the chunks within the light radius of it (5 and 6) wait for their
/// reach.
fn row_n(n: i64, syncs: usize, setup: Setup) -> Option<Q> {
    let world = world_of([n as i32, 1, 1], AIR);
    for x in 0..n {
        fill(&world, [x * 64 + 16, 16, 16], [x * 64 + 48, 48, 48], ROCK);
    }
    fill(&world, [140, 28, 48], [150, 36, 50], ORE);
    let mut q = rig(
        world,
        &Setup {
            sync_budget: 1,
            ..setup
        },
    )?;
    for _ in 1..syncs {
        q.gpu
            .sync_now(&q.world, &q.device, &q.queue)
            .expect("the mirror fits");
    }
    Some(q)
}

/// Mirrors one chunk: one sync at the rig's budget.
fn sync_one(q: &mut Q) {
    q.gpu
        .sync_now(&q.world, &q.device, &q.queue)
        .expect("the mirror fits");
}

/// Nine chunks in a row, chunks 7 and 8 waiting to upload: once the queue has baked chunks 0
/// to 4 and checked 5 and 6, 100 further steps on a still mirror scan nothing, walk no reach
/// and score nothing. Evicting chunk 3 makes the next step scan exactly that chunk and free
/// its block, no other. Mirroring chunk 7 re-checks exactly the candidates within the light
/// radius and one of it, chunks 5, 6 and 7, each once: 5 bakes, 6 and 7 wait for chunk 8.
#[test]
fn a_still_step_scans_nothing() {
    let Some(mut q) = row_n(9, 7, Setup::at(ROW_EYE)) else {
        return;
    };
    let blocks = |q: &Q| -> Vec<i32> {
        (0..9)
            .filter(|&x| q.frame.store().block(ChunkCoord([x, 0, 0])).is_some())
            .collect()
    };
    for _ in 0..300 {
        q.step_with(false);
    }
    let st = q.q().stats();
    assert_eq!((st.jobs, st.fading), (0, 0), "drained: {st:?}");
    assert_eq!(
        blocks(&q),
        [0, 1, 2, 3, 4],
        "chunks 5 and 6 wait for their reach"
    );
    for n in 0..100 {
        let st = q.step_with(false);
        assert_eq!(
            (st.scanned, st.reach_checks, st.rebuilt),
            (0, 0, 0),
            "still step {n}: {st:?}"
        );
    }
    q.gpu.apply_events(&vxe_world::StreamEvents {
        evicted: vec![ChunkCoord([3, 0, 0])],
        ..Default::default()
    });
    let st = q.step_with(false);
    assert_eq!(st.scanned, 1, "the eviction's step scans its chunk");
    assert_eq!(
        blocks(&q),
        [0, 1, 2, 4],
        "chunk 3's block is freed, no other"
    );
    sync_one(&mut q);
    assert!(
        q.gpu.is_mirrored(ChunkCoord([7, 0, 0])),
        "chunk 7 is mirrored"
    );
    assert!(
        q.gpu.is_pending(ChunkCoord([8, 0, 0])),
        "chunk 8 still waits"
    );
    let mut checks = 0;
    for _ in 0..40 {
        checks += q.step_with(false).reach_checks;
    }
    assert_eq!(checks, 3, "chunks 5, 6 and 7 are checked again, once each");
    assert_eq!(blocks(&q), [0, 1, 2, 4, 5], "chunk 5 bakes");
}

/// The eight-chunk row with chunk 7 waiting to upload, at 1024 samples a brick direction and
/// the smallest slice: one job opens at a time, nearest first with the view's tie (2, 3, 1,
/// 4, 0), as the start commit opens them. Every opening after the first, the mirror still and
/// jobs ending between them, scores nothing again, and each candidate's reach is walked once
/// over the run (0 to 6).
/// Mirroring chunk 7 makes the next opening score once; 7 opens, then 5 and 6 once their
/// last check is old enough to be read again.
#[test]
fn an_opening_without_a_mirror_event_rebuilds_nothing() {
    let Some(mut q) = row_n(
        8,
        7,
        Setup {
            slice_ms: 1e-6,
            counts: BakeSettings {
                brick_samples: 1024,
                ..counts()
            },
            ..Setup::at(ROW_EYE)
        },
    ) else {
        return;
    };
    let mut order: Vec<i32> = Vec::new();
    let (mut checks, mut openings, mut rebuilt) = (0, 0, 0);
    let mut steps = 0;
    while !(q.q().jobs().is_empty() && order.len() == 5) {
        steps += 1;
        assert!(steps <= MAX_STEPS, "the five ready chunks bake");
        let before = q.q().stats().openings;
        let st = q.step_with(false);
        checks += st.reach_checks;
        if st.openings > before {
            openings += 1;
            rebuilt += st.rebuilt;
            assert!(
                openings == 1 || st.rebuilt == 0,
                "opening {openings} scores again: {st:?}"
            );
        }
        for c in q.q().jobs() {
            if !order.contains(&c.0[0]) {
                order.push(c.0[0]);
            }
        }
    }
    for _ in 0..40 {
        checks += q.step_with(false).reach_checks;
    }
    println!("order {order:?}, {openings} openings, {rebuilt} scored, {checks} reach walks");
    assert_eq!(order, [2, 3, 1, 4, 0]);
    assert!(openings >= 5, "one opening a job at least");
    assert_eq!(rebuilt, 1, "only the first opening scores");
    assert_eq!(checks, 7, "chunks 0 to 6 each walked once");
    sync_one(&mut q);
    let mut rebuilt = None;
    for _ in 0..20 {
        let before = q.q().stats().openings;
        let st = q.step_with(false);
        if st.openings > before {
            rebuilt = Some(st.rebuilt);
            break;
        }
    }
    assert_eq!(rebuilt, Some(1), "chunk 7's arrival scores the order once");
    for _ in 0..MAX_STEPS {
        q.step_with(false);
        for c in q.q().jobs() {
            if !order.contains(&c.0[0]) {
                order.push(c.0[0]);
            }
        }
        if q.q().empty() {
            break;
        }
    }
    // 5 and 6 were found blocked at the last openings and wait out the recheck: 7 first.
    assert_eq!(order, [2, 3, 1, 4, 0, 7, 5, 6]);
}

/// The order of a flight: the six-chunk row, the coarse pass on, the near shell 1 and the
/// smallest slice, the camera flown along +x from chunk 0 six voxels a step for 60 steps,
/// then held until the queue is empty. Each job's chunk is recorded as it first appears in
/// the jobs (a refinement appears again), against the start commit's recording.
#[test]
fn a_flight_opens_in_the_start_commit_s_order() {
    let Some(mut q) = rig(
        row(&[0, 1, 2, 3, 4, 5]),
        &Setup {
            coarse: COARSE,
            near: 1,
            slice_ms: 1e-6,
            ..Setup::at([20.5, 32.5, 60.5])
        },
    ) else {
        return;
    };
    let mut seen: Vec<i32> = Vec::new();
    let mut prev: Vec<ChunkCoord> = Vec::new();
    for n in 0..MAX_STEPS {
        if n < 60 {
            q.camera.position.x += 6.0;
        }
        q.step();
        let jobs = q.q().jobs();
        for c in &jobs {
            if !prev.contains(c) {
                seen.push(c.0[0]);
            }
        }
        prev = jobs;
        if n >= 60 && q.q().empty() {
            break;
        }
    }
    println!("flight order {seen:?}");
    assert_eq!(seen, FLIGHT_ORDER);
}

/// [`a_flight_opens_in_the_start_commit_s_order`]'s recording at the start commit (4bc8100).
const FLIGHT_ORDER: [i32; 10] = [0, 1, 2, 3, 4, 5, 2, 3, 4, 5];
