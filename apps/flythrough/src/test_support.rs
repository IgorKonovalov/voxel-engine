//! What the lib's unit tests share: an allocator that counts each thread's bytes, the
//! argument vector, a native server and client, and a bare run over a carved world.

use crate::cave::ROCK;
use crate::cli::parse_args_from;
use crate::edits::{DIG_RADIUS, Failure, Net, voxel_of};
use crate::fly::{CutawayMode, Fly};
use crate::hud::{self};
use crate::logic::DigInput;
use crate::now;
use crate::run::Run;
use crate::run::hud_feed::Hud;
use crate::run::queue::QueueRun;
use crate::{ScrollSteps, cave_materials};
use std::sync::Arc;
use vxe_net::Event;
use vxe_render::glam::Vec3;
use vxe_render::{Camera, LightParams};
use vxe_world::{ChunkExtent, EditOp, FIXED_ONE, Fixed3, Mode, OpResult, World};

/// Bytes allocated on each thread, counted for the HUD's no-allocation test.
pub(crate) mod counting {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    thread_local! {
        static BYTES: Cell<u64> = const { Cell::new(0) };
    }

    /// The system allocator, adding each allocation's size to the calling thread's count.
    pub struct Counting;

    // SAFETY: every call forwards to `System` unchanged; the count is a side effect that
    // never allocates (a const-initialised thread-local `Cell`).
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let _ = BYTES.try_with(|b| b.set(b.get() + layout.size() as u64));
            // SAFETY: the caller upholds `alloc`'s contract, which `System` shares.
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: `ptr` came from `System` through `alloc` or `realloc` with `layout`.
            unsafe { System.dealloc(ptr, layout) }
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            let _ = BYTES.try_with(|b| b.set(b.get() + new_size as u64));
            // SAFETY: as `dealloc`, and the caller upholds `realloc`'s size contract.
            unsafe { System.realloc(ptr, layout, new_size) }
        }
    }

    /// Bytes this thread has allocated so far.
    pub fn bytes() -> u64 {
        BYTES.with(Cell::get)
    }
}

#[global_allocator]
static ALLOC: counting::Counting = counting::Counting;

pub(crate) fn argv(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| (*s).to_owned()).collect()
}

/// One server tick of `net` on `inputs`, sent by its client: the logic's broadcast lines
/// and every event the client polled.
pub(crate) fn net_tick(net: &mut Net, inputs: &[DigInput]) -> (Vec<String>, Vec<Event>) {
    for i in inputs {
        net.client.send_input(i.encode());
    }
    let report = net.server.tick();
    assert_eq!(report.logic_error, None);
    let events = net.client.poll();
    let lines = events
        .iter()
        .filter_map(|e| match e {
            Event::Logic { bytes, .. } => Some(String::from_utf8(bytes.clone()).unwrap()),
            _ => None,
        })
        .flat_map(|t| t.lines().map(str::to_owned).collect::<Vec<_>>())
        .collect();
    (lines, events)
}

/// The results of the applied ops among `events`, in order.
pub(crate) fn applied(events: &[Event]) -> Vec<OpResult> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Applied { result, .. } => Some(result.clone()),
            _ => None,
        })
        .collect()
}

pub(crate) fn native_net(world: &Arc<World>) -> Net {
    Net::new(world, Box::new(crate::logic::DigLogic::default()), 1)
}

/// A fly-mode run on `world` with no window, GPU, stream or audio, its camera at the
/// origin voxel's centre looking along +x, the native logic on its server.
pub(crate) fn bare_run(world: World) -> Run {
    let world = Arc::new(world);
    let net = native_net(&world);
    let args = parse_args_from(argv(&["--level"])).expect("--level parses");
    let modes = args.start_modes(0.0);
    let camera = Camera {
        position: Vec3::splat(0.5),
        yaw: 0.0,
        pitch: 0.0,
        vfov: 1.0,
    };
    Run {
        start: now(),
        failure: Failure::default(),
        world,
        net: Some(net),
        spawn: voxel_of(camera.position),
        look: args.start_look(),
        preset: args.look.preset,
        args,
        fly: Fly::new(camera),
        light: LightParams::headlamp(),
        modes,
        upscaler: hud::UpscalerBanner::new(hud::UpscalerMode::Off, "1"),
        upscaler_changed: false,
        sharpen_changed: false,
        pan: None,
        scene: None,
        castle_shot: None,
        pose: None,
        dig_radius: DIG_RADIUS,
        scroll: ScrollSteps::default(),
        triggers: [false; 2],
        bench_runs: 0,
        frame: 0,
        gfx: None,
        samples: None,
        window_ms: [Vec::new(), Vec::new()],
        fade: Vec::new(),
        stream: None,
        pilot: None,
        walk: None,
        coarse: None,
        hud: Hud::new(false),
        audio: None,
        room_probe: None,
        exposure_print: None,
        queue: QueueRun::default(),
        player: None,
        recorder: None,
        adapter: None,
        help: false,
        cutaway: CutawayMode::Auto,
    }
}

/// Rock over chunks `[-1, 1)³` with a 16-voxel box carved around the origin.
pub(crate) fn carved_rock() -> World {
    let world = World::generate(
        ChunkExtent::new([-1, -1, -1], [1, 1, 1]),
        &vxe_world::UniformSource(ROCK),
        &cave_materials(),
    );
    world
        .apply(&EditOp::box_(
            Fixed3([-8 * FIXED_ONE; 3]),
            Fixed3([8 * FIXED_ONE; 3]),
            Mode::Carve,
        ))
        .unwrap();
    world
}
