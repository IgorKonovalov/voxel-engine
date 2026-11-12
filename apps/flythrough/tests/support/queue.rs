//! The live queue's shared rigs: the hand-built box worlds, the stepped frame and the store
//! read-backs both `gpu_queue` binaries take.

use super::SyncNow;
use super::cave_table;
use flythrough::cave::{ORE, ROCK};
use std::collections::BTreeMap;
use vxe_render::LightingResolution;
use vxe_render::{BakeSettings, QueueStats};
use vxe_render::{Camera, Exposure, Frame, LightParams, Lighting, QueueView, wgpu};
use vxe_render::{GpuWorld, MirrorConfig, Window};
use vxe_world::{
    AIR, ChunkCoord, ChunkExtent, EditOp, FIXED_ONE, Fixed3, Mode, UniformSource, VoxelCoord, World,
};

pub fn box_op(min: [i64; 3], max: [i64; 3], mode: Mode) -> EditOp {
    EditOp::box_(
        Fixed3(min.map(|v| v * FIXED_ONE)),
        Fixed3(max.map(|v| v * FIXED_ONE)),
        mode,
    )
}

pub fn fill(world: &World, min: [i64; 3], max: [i64; 3], m: vxe_world::MaterialId) {
    world
        .apply(&box_op(min, max, Mode::Fill(m)))
        .expect("the box is resident");
}

pub fn world_of(size: [i32; 3], m: vxe_world::MaterialId) -> World {
    World::generate(
        ChunkExtent::new([0, 0, 0], size),
        &UniformSource(m),
        &cave_table(),
    )
}

/// The brick level at 64 samples a brick direction, rounds of 64.
pub fn counts() -> BakeSettings {
    BakeSettings {
        round: 64,
        brick_samples: 64,
        ..BakeSettings::default()
    }
}

/// The coarse pass's count a brick direction.
pub const COARSE: u32 = 16;

/// Steps before a test gives up on an empty queue.
pub const MAX_STEPS: usize = 600;

pub struct Q {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub gpu: GpuWorld,
    pub world: World,
    pub frame: Frame,
    pub cfg: MirrorConfig,
    pub camera: Camera,
    pub counts: BakeSettings,
    /// The steps' sample count: `None` is a frame's sliced step, `Some` the pre-roll's.
    pub samples: Option<u64>,
    /// The last step's `bake_queue_step` call, timed from outside, in ms.
    pub last_step_ms: f64,
    /// Whether a chunk is resident full, as a streamer's would say; `None` is a bounded world.
    pub resident: Option<fn(ChunkCoord) -> bool>,
    /// The streamer's full radius in chunks the view hands the queue; `None` a bounded world.
    pub full_radius: Option<i32>,
}

/// How a rig's queue and mirror start.
pub struct Setup {
    pub window: Option<Window>,
    pub eye: [f32; 3],
    pub pool_bytes: u64,
    /// Bytes a mirror sync writes before it stops taking chunks (at least one).
    pub sync_budget: u64,
    pub coarse: u32,
    pub near: i32,
    pub slice_ms: f64,
    /// The queue's order; `None` keeps its default.
    pub order: Option<vxe_render::QueueOrder>,
    /// The queue's counts.
    pub counts: BakeSettings,
    /// The mirror's light radius in chunks.
    pub light_radius: i32,
}

impl Setup {
    pub fn at(eye: [f32; 3]) -> Self {
        Self {
            window: None,
            eye,
            pool_bytes: 16 << 20,
            sync_budget: u64::MAX,
            coarse: 0,
            near: vxe_render::DEFAULT_NEAR,
            slice_ms: 1.5,
            order: None,
            counts: counts(),
            light_radius: 2,
        }
    }
}

/// A camera at `eye` looking along +x.
pub fn camera_at(eye: [f32; 3]) -> Camera {
    Camera {
        position: eye.into(),
        yaw: 0.0,
        pitch: 0.0,
        vfov: 70f32.to_radians(),
    }
}

pub fn new_frame(device: &wgpu::Device, pool_bytes: u64) -> Frame {
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
pub fn rig(world: World, setup: &Setup) -> Option<Q> {
    let (device, queue) = super::software_device(64 << 20)?;
    let cfg = MirrorConfig {
        budget_bytes: 64 << 20,
        light_radius: setup.light_radius,
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
        full_radius: None,
    })
}

impl Q {
    /// One frame's queue step as the window's frame takes it: the mirror synced (`sync`), then
    /// the step with no sample count; then the device waited on, so the next step reads this
    /// one's submit back.
    pub fn step_with(&mut self, sync: bool) -> QueueStats {
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
            // A fixed 60 Hz frame: a per-second budget sizes the slice repeatably.
            interval_s: 1.0 / 60.0,
            hold: None,
            half_angle: {
                let [w, h] = self.frame.output_size();
                vxe_render::internals::view_half_angle(self.camera.vfov, w as f32 / h as f32)
            },
            frustum: None,
            full_radius: self.full_radius,
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

    pub fn step(&mut self) -> QueueStats {
        self.step_with(true)
    }

    pub fn q(&self) -> &vxe_render::BakeQueue {
        self.frame.bake_queue().expect("the queue started")
    }

    /// Steps until the queue is empty: the steps taken.
    pub fn settle(&mut self) -> usize {
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
    pub fn shown(&self, c: [i32; 3]) -> Option<Vec<u32>> {
        self.frame
            .read_store_block(&self.device, &self.queue, ChunkCoord(c))
    }

    /// The store's own copy of `c`'s block: its target.
    pub fn target(&self, c: [i32; 3]) -> Option<Vec<u32>> {
        self.frame
            .store()
            .block_words(ChunkCoord(c))
            .map(<[u32]>::to_vec)
    }

    /// Every block of the world's extent, from the pool.
    pub fn blocks(&self) -> BTreeMap<[i32; 3], Vec<u32>> {
        blocks_of(&self.device, &self.queue, &self.frame, self.world.extent())
    }

    /// A fresh mirror and frame of the world as it is now, its extent baked in one go at the
    /// rig's counts: every block.
    pub fn fresh(&self) -> BTreeMap<[i32; 3], Vec<u32>> {
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
    pub fn edit(&mut self, op: &EditOp) {
        let r = self.world.apply(op).expect("the edit is resident");
        let (min, max) = r.aabb.expect("the edit changed voxels");
        self.gpu
            .sync_now(&self.world, &self.device, &self.queue)
            .expect("the mirror fits");
        self.frame.invalidate(min.0, max.0);
    }

    pub fn render(&mut self) -> Vec<u8> {
        self.frame.step(
            &self.device,
            &self.queue,
            &self.gpu,
            &self.camera,
            &LightParams::default(),
        );
        vxe_render::debug::read_rgba8(&self.device, &self.queue, self.frame.target())
    }

    /// No two blocks of `chunks` share a pool word.
    pub fn assert_places_apart(&self, chunks: &[[i32; 3]]) {
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
pub fn blocks_of(
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

/// Two chunks along x: a rock block across their border under an ore plate, the camera in
/// chunk 0 looking at it along +x.
pub fn border_box() -> (World, [f32; 3]) {
    let world = world_of([2, 1, 1], AIR);
    fill(&world, [40, 20, 20], [88, 40, 40], ROCK);
    fill(&world, [44, 24, 40], [84, 36, 42], ORE);
    (world, [20.5, 30.5, 50.5])
}

/// Six chunks in a row along x, a rock block in each chunk of `rocks` (16 voxels from its
/// borders) and an ore plate in chunk 2.
pub fn row(rocks: &[i64]) -> World {
    let world = world_of([6, 1, 1], AIR);
    for &x in rocks {
        fill(&world, [x * 64 + 16, 16, 16], [x * 64 + 48, 48, 48], ROCK);
    }
    fill(&world, [140, 28, 48], [150, 36, 50], ORE);
    world
}

/// The row's camera, in chunk 2.
pub const ROW_EYE: [f32; 3] = [150.5, 32.5, 60.5];

/// The score at angle weight `angle` and refine weight 2.
pub fn score(angle: f32) -> vxe_render::QueueOrder {
    vxe_render::QueueOrder::Score {
        angle,
        refine: vxe_render::Refine::Weight(2.0),
    }
}

impl Q {
    /// Every pool word is the header's, a held block's, a job's place or free.
    pub fn assert_pool_accounted(&self) {
        let store = self.frame.store();
        let used = store.stats(0, 0, 0).bytes / 4;
        assert_eq!(
            used + store.free_words() + self.q().job_words(),
            store.pool().size() / 4,
            "every pool word is the header's, a block's, a job's or free"
        );
    }

    /// Steps until `jobs()` holds chunk `x` of the row: the steps taken.
    pub fn step_until_job(&mut self, x: i32) -> usize {
        for n in 1..=MAX_STEPS {
            self.step();
            if self.q().jobs().iter().any(|c| c.0[0] == x) {
                return n;
            }
        }
        panic!("chunk {x} never opened");
    }
}
