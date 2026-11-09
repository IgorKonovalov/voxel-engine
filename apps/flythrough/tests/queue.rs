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
    frame.start_bake_queue(&queue, &gpu, counts(), setup.slice_ms);
    frame.set_bake_coarse(setup.coarse, setup.near);
    Some(Q {
        device,
        queue,
        gpu,
        world,
        frame,
        cfg,
        camera: camera_at(setup.eye),
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
        let view = QueueView {
            eye,
            forward: [1.0, 0.0, 0.0],
            resident: None,
            samples: None,
            hold: None,
        };
        let st = self
            .frame
            .bake_queue_step(&self.device, &self.queue, &mut self.gpu, &self.world, &view)
            .expect("the queue runs")
            .expect("the queue's step");
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

    /// A fresh mirror and frame of the world as it is now, its extent baked in one go at
    /// [`counts`]: every block.
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
                &counts(),
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

/// Six chunks in a row, a rock block in each and the camera in chunk 2 looking along +x, coarse
/// off: the jobs open by squared chunk distance from the camera's chunk, ties by the view (the
/// chunk ahead first), so 2, 3, 1, 4, 0, 5. At the smallest slice no step's submit plans more
/// than the step's cap but for the one entry (a round, 64 samples) that crosses it.
#[test]
fn the_queue_runs_near_first_and_within_its_samples() {
    let world = world_of([6, 1, 1], AIR);
    for x in 0..6 {
        fill(&world, [x * 64 + 16, 16, 16], [x * 64 + 48, 48, 48], ROCK);
    }
    fill(&world, [140, 28, 48], [150, 36, 50], ORE);
    let Some(mut q) = rig(
        world,
        &Setup {
            slice_ms: 1e-6,
            ..Setup::at([150.5, 32.5, 60.5])
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
