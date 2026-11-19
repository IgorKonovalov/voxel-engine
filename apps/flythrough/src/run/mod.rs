//! TODO

use crate::app::{diag, ms_since};
use crate::audio::{Audio, RoomProbe};
use crate::autopilot::Autopilot;
use crate::castle;
use crate::cli::Args;
use crate::edits::{Coarse, Failure, Net};
use crate::fly::{Fly, Walk};
use crate::hud::{self};
use crate::now;
use crate::path::PathPlayer;
use crate::report::Recorder;
use crate::run::fade::FadeSample;
use crate::run::hud_feed::{HUD_SCALE, Hud, Samples};
use crate::run::queue::QueueRun;
use crate::setup::{CrispRows, Gfx};
use crate::streaming::{STREAM_POOL_BYTES, Stream};
use crate::{ScrollSteps, chunk_ram, level_mirror_budget, level_pool_units, mips_ram};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use vxe_app::{Flow, Gpu};
use vxe_render::{
    Camera, FRAME_PASSES, Frame, GpuWorld, HudOverlay, LightParams, Lighting, LightingResolution,
    LoadingScreen, LodParams, MarchTimer, MirrorConfig, PassTimer, Presenter, UpscalerKind, Window,
    wgpu,
};
use vxe_world::{MaterialTable, VoxelCoord, World};

pub(crate) mod edit;
pub(crate) mod fade;
pub(crate) mod frame;
pub(crate) mod hud_feed;
pub(crate) mod hud_lines;
pub(crate) mod keys;
pub(crate) mod loading;
pub(crate) mod pilot;
pub(crate) mod profile;
pub(crate) mod queue;
pub(crate) mod switch;
pub(crate) mod tour;

/// The running fly-through, once the world exists.
pub(crate) struct Run {
    pub(crate) start: Instant,
    /// Set by every error path that ends the session; see [`Failure`].
    pub(crate) failure: Failure,
    /// The server's world, which the shared client, the mirror and the queries read.
    pub(crate) world: Arc<World>,
    /// `None` only while dropping.
    pub(crate) net: Option<Net>,
    /// The voxel the camera starts in; the mirror uploads nearest it first.
    pub(crate) spawn: VoxelCoord,
    pub(crate) args: Args,
    pub(crate) fly: Fly,
    pub(crate) light: LightParams,
    /// Every mode a key toggles (`toggles::TOGGLES`) and the banner naming them; the frame
    /// takes the lighting, the look, the cutaway and the upscaler from them at the start of
    /// each frame (`F6`'s resolution and `F10`'s upscaler need the device).
    pub(crate) modes: hud::Modes,
    /// `F10` changed the upscaler: applied at the next frame.
    pub(crate) upscaler_changed: bool,
    /// `Z` changed the sharpening: applied at the next frame.
    pub(crate) sharpen_changed: bool,
    /// The scene's banner row (`castle gate`) and when it was last shown, by the frame loop's
    /// clock; `None` without one.
    pub(crate) scene: Option<(String, f64)>,
    /// `--castle`'s current shot, which `I` advances; `None` without the castle.
    pub(crate) castle_shot: Option<castle::CastleShot>,
    /// `--pose`'s banner row (`pose vein`) and the camera it starts at: the row shows until
    /// the camera first moves.
    pub(crate) pose: Option<(String, Camera)>,
    /// Scroll banked toward the next dig-radius step.
    pub(crate) scroll: ScrollSteps,
    /// `L2` and `R2` held, for their press edges.
    pub(crate) triggers: [bool; 2],
    pub(crate) bench_runs: u64,
    pub(crate) frame: u32,
    pub(crate) gfx: Option<Gfx>,
    pub(crate) samples: Option<Samples>,
    /// Under `--lamp-toggle`, the probes plus gather GPU time of each frame outside and
    /// inside a light-change window.
    pub(crate) window_ms: [Vec<f64>; 2],
    /// Under `--fade-reading`, a sample a frame from the end of the digs on.
    pub(crate) fade: Vec<FadeSample>,
    pub(crate) stream: Option<Stream>,
    pub(crate) pilot: Option<Autopilot>,
    /// Walk mode's body, `None` in fly mode.
    pub(crate) walk: Option<Walk>,
    pub(crate) coarse: Option<Coarse>,
    pub(crate) hud: Hud,
    pub(crate) audio: Option<Audio>,
    /// `--room-line` without an output device.
    pub(crate) room_probe: Option<RoomProbe>,
    /// When the exposure line was last checked for printing (ms since the start) and its EV.
    pub(crate) exposure_print: Option<(f64, f32)>,
    /// The live queue's readings.
    pub(crate) queue: QueueRun,
    /// `--path`'s player and the frame times its report reads.
    pub(crate) player: Option<PathPlayer>,
    pub(crate) recorder: Option<Recorder>,
    /// The adapter the device was created on, for the report.
    pub(crate) adapter: Option<wgpu::AdapterInfo>,
    /// The surface's format and colour space, for the report.
    pub(crate) surface: Option<(wgpu::TextureFormat, Option<wgpu::SurfaceColorSpace>)>,
    /// `--tester`'s help panel (`F1`) is shown.
    pub(crate) help: bool,
    /// The material file, read again once a second (`crate::materials`); `None` for the
    /// built-in copy.
    pub(crate) materials: Option<crate::materials::Watch>,
    /// `--vox-op`'s ops, until they are submitted.
    pub(crate) vox_ops: Option<crate::structures::VoxOps>,
    /// A structure placed from the belt and its op count, until the tick that applies its
    /// ops says the notice.
    pub(crate) structure_placed: Option<(String, usize)>,
    /// The menu (`Tab`, `F1`); never opened under `--tester`.
    pub(crate) menu: crate::hud::menu::Menu,
    /// The last line an action said (`Run::say`) and when (the run's clock, ms): the menu's
    /// notice row, for `hud::BANNER_MS`.
    pub(crate) notice: Option<(String, f64)>,
    /// When (the run's clock, ms) a first `Esc` with the menu closed and placement off asked
    /// before quitting: a second within [`QUIT_ASK_MS`] ends the run, any other key or the
    /// timeout drops the question (`Run::key_step`).
    pub(crate) quit_asked: Option<f64>,
    /// The menu's `source` entry or `--switch-at` asked for the next world source: the app rebuilds the run
    /// after this frame (`run::switch`).
    pub(crate) switch: bool,
    /// The menu's `trees` entry asked for the outdoor hills' next `--trees`: the app rebuilds the run after
    /// this frame through the switch's path (`run::switch`).
    pub(crate) retree: bool,
    /// Start-up still stepping behind loading frames; `None` once the first world frame may
    /// draw.
    pub(crate) loading: Option<loading::Loading>,
}

/// The features and limits the fly-through asks of `adapter` for a brick pool of
/// `pool_bytes`: the timestamp queries where offered, and [`vxe_render::limits_for`]'s limits.
pub(crate) fn device_request_for(
    adapter: &wgpu::Adapter,
    pool_bytes: u64,
) -> (wgpu::Features, wgpu::Limits) {
    // An adapter below the shaders' floor: name the limit, then request the floor, which the
    // device request refuses on the same limit and the shell reports as a setup error.
    let limits = vxe_render::limits_for(adapter, pool_bytes).unwrap_or_else(|e| {
        eprintln!("flythrough: {e}");
        *e.requested
    });
    (adapter.features() & wgpu::Features::TIMESTAMP_QUERY, limits)
}

/// What [`build_gfx`] builds the run's GPU objects from: read from the run on the main thread,
/// so the build itself needs only the device and the queue.
pub(crate) struct GfxPlan {
    format: wgpu::TextureFormat,
    output: [u32; 2],
    internal: [u32; 2],
    kind: UpscalerKind,
    jitter: bool,
    probe_spacing: u32,
    sharpen: f32,
    lighting: Lighting,
    resolution: LightingResolution,
    /// The streamed world's LOD rule; `None` on a level.
    lod: Option<LodParams>,
    mirror: MirrorConfig,
    /// The mirror takes the field's band slots.
    field: bool,
    /// The level's mirror focus (the spawn); `None` on the streamed world.
    focus: Option<VoxelCoord>,
}

/// How long (ms) the first `Esc` waits for the second before the question drops.
pub(crate) const QUIT_ASK_MS: f64 = 2000.0;
/// The banner row while a first `Esc` waits for the second.
pub(crate) const QUIT_ASK_WORDS: &str = "Esc again to quit";

/// Units of [`build_gfx`]'s progress, from the stages' times read on the RTX 3080 Laptop with
/// the driver compiling (`docs/plans/logs/0075.md`): the frame's creation about 6 s (its
/// compute pipelines), its settings about 0.6 s, the entity pipelines about 6 s.
pub(crate) const BUILD_UNITS: u64 = 21;
const FRAME_UNITS: u64 = 10;
const SETTINGS_UNITS: u64 = 1;

/// The run's GPU objects from `plan`: the frame and its settings, the mirror, the presenter,
/// the overlay and the timers, then, when `entities` (the loading screen's worker), the
/// entity pipelines ([`Frame::prepare_entities`]) and the light bake's
/// ([`Frame::prepare_bake`]). Adds each part's units to `built` as it ends (of
/// [`BUILD_UNITS`]); safe to run off the main thread (it touches nothing but the device and
/// the queue).
pub(crate) fn build_gfx(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    plan: GfxPlan,
    entities: bool,
    built: &AtomicU64,
) -> Gfx {
    let timer = PassTimer::with_passes(device, queue, FRAME_PASSES.len() as u32);
    let field_timer = MarchTimer::new(device, queue);
    let mut frame = Frame::with_sizes(device, plan.internal, plan.output);
    diag(format_args!(
        "frame created, {} B of targets",
        frame.frame_bytes()
    ));
    built.fetch_add(FRAME_UNITS, Ordering::Relaxed);
    set_plan(&mut frame, device, queue, &plan);
    diag(format_args!(
        "lighting set, {} B of targets",
        frame.frame_bytes()
    ));
    let mut mirror = reserve_mirror(device, plan.mirror, plan.field);
    if let Some(focus) = plan.focus {
        mirror.set_focus(focus);
    }
    let mut gfx = Gfx {
        mirror,
        frame,
        presenter: Presenter::new(device, plan.format),
        timer,
        field_timer,
        hud: HudOverlay::new(device, plan.format, HUD_SCALE),
        hud_timer: PassTimer::new(device, queue),
    };
    built.fetch_add(SETTINGS_UNITS, Ordering::Relaxed);
    if entities {
        // A walker may be placed on any frame, so the entity pipelines are built before the
        // first world frame, and the frame that draws the first part builds none.
        gfx.frame.prepare_entities(device);
        diag(format_args!("entity pipelines built"));
        gfx.frame.prepare_bake(device);
        diag(format_args!("bake pipeline built"));
    }
    built.store(BUILD_UNITS, Ordering::Relaxed);
    gfx
}

/// `plan`'s settings on `frame`: the probe spacing, the jitter, the upscaler and its
/// sharpening, the lighting and its resolution, and the LOD rule (the default on a level).
fn set_plan(frame: &mut Frame, device: &wgpu::Device, queue: &wgpu::Queue, plan: &GfxPlan) {
    frame.set_probe_spacing(device, plan.probe_spacing);
    frame.set_jitter(device, plan.jitter);
    frame.set_upscaler_kind(device, plan.kind);
    frame.set_sharpness(queue, plan.sharpen);
    frame.set_lighting(plan.lighting);
    frame.set_resolution(device, plan.resolution);
    frame.set_lod(plan.lod.unwrap_or_default());
}

/// A mirror under `cfg`, with the field's band slots when `field`. Prints what it reserved
/// and how long creating its buffers took.
pub(crate) fn reserve_mirror(device: &wgpu::Device, cfg: MirrorConfig, field: bool) -> GpuWorld {
    let t = now();
    let mirror = if field {
        GpuWorld::with_field_surface(device, cfg)
    } else {
        GpuWorld::with_config(device, cfg)
    };
    let ms = ms_since(t);
    diag(format_args!("mirror reserved"));
    let l = mirror.shard_layout();
    println!(
        "mirror: reserved {} B of a {} B budget ({} units over {} shards, {} records) in {ms:.1} ms",
        mirror.allocated_bytes(),
        cfg.budget_bytes,
        l.reserved_units,
        l.reserved_units.div_ceil(l.units_per_shard),
        l.reserved_records
    );
    mirror
}

/// Logs a teardown stage before and after it runs, like the shell's, so a hang at exit
/// names its stage.
pub(crate) fn teardown_stage(what: &str, f: impl FnOnce()) {
    log::info!("teardown: dropping the {what}");
    let t = now();
    f();
    log::info!("teardown: {what} dropped in {:.1} ms", ms_since(t));
}

impl Run {
    /// RAM held by resident chunks: full chunks' voxels and LOD-only chunks' mips.
    pub(crate) fn resident_ram(&self) -> u64 {
        let full: u64 = self
            .world
            .store()
            .coords()
            .into_iter()
            .filter_map(|c| self.world.store().get(c))
            .map(|c| chunk_ram(&c.read().unwrap_or_else(std::sync::PoisonError::into_inner)))
            .sum();
        let lod: u64 = self
            .world
            .lod_coords()
            .into_iter()
            .filter_map(|c| self.world.lod(c))
            .map(|m| mips_ram(&m))
            .sum();
        full + lod
    }

    pub(crate) fn device_request(
        &mut self,
        adapter: &wgpu::Adapter,
    ) -> (wgpu::Features, wgpu::Limits) {
        let pool = if self.stream.is_some() {
            STREAM_POOL_BYTES
        } else {
            level_pool_units(&self.world, self.args.field_surface())
                * vxe_render::internals::UNIT_BYTES
        };
        device_request_for(adapter, pool)
    }

    /// Reads the device, then builds the frame and the mirror and starts loading. With the
    /// loading screen on, the build ([`build_gfx`], the entity pipelines included) runs on a
    /// worker thread and the frame loop draws loading frames with `splash` (the screen the
    /// first frame was drawn with, else a new one) until it hands the objects back; with it
    /// off, the build and the whole load run here.
    pub(crate) fn init(&mut self, gpu: &Gpu, splash: Option<LoadingScreen>) {
        self.adapter = Some(gpu.adapter.get_info());
        self.surface = Some((gpu.surface_format, gpu.surface_color_space));
        println!("device ready after {:.0} ms", ms_since(self.start));
        if let Some(rec) = self.recorder.as_mut() {
            rec.device_ms = Some(ms_since(self.start));
        }
        let plan = self.gfx_plan(gpu);
        println!("{}", self.modes.upscaler.line());
        println!("{}", self.modes.line());
        println!("{}", CrispRows::line(self.world.materials()));
        let screen = self.args.loading.screen_on(self.args.headless);
        if screen {
            let build = loading::Build::spawn(gpu, plan);
            self.loading = Some(self.start_loading(gpu, splash, build));
            return;
        }
        self.gfx = Some(build_gfx(
            &gpu.device,
            &gpu.queue,
            plan,
            false,
            &AtomicU64::new(0),
        ));
        let loading = self.start_loading_unscreened();
        self.load_all(gpu, loading);
    }

    /// What [`build_gfx`] builds the run's GPU objects from, read from the run and the device.
    fn gfx_plan(&self, gpu: &Gpu) -> GfxPlan {
        let output = self.args.render.unwrap_or(gpu.size);
        let (kind, scale, jitter) = self.args.upscaler_settings(self.modes.upscaler.mode);
        let streamed = self.stream.is_some();
        let mirror = if streamed {
            MirrorConfig {
                sync_budget_bytes: self.args.sync_budget,
                sync_budget_time: self.args.sync_time,
                record_chunks: Some(crate::stream_record_chunks(self.args.residency)),
                ..MirrorConfig::new(Window::Follow {
                    side: crate::window_side(self.args.residency),
                })
            }
        } else {
            MirrorConfig::new(Window::Fixed(self.world.extent()))
        };
        GfxPlan {
            format: gpu.surface_format,
            output,
            internal: scale.internal(output),
            kind,
            jitter,
            probe_spacing: self.args.light.probe_spacing,
            sharpen: self.args.look.sharpen,
            lighting: self.modes.lighting,
            resolution: self.modes.resolution,
            lod: streamed.then_some(LodParams {
                r_full: self.args.residency.r_full,
                forced_level: None,
            }),
            mirror: self.mirror_config(mirror),
            field: self.args.field_surface(),
            focus: (!streamed).then_some(self.spawn),
        }
    }

    /// The mirror's VRAM budget: `--mirror-budget`, else the level's from its content and
    /// extent, else the streamed default.
    pub(crate) fn mirror_budget(&self) -> u64 {
        self.args.mirror_budget.unwrap_or_else(|| {
            if self.stream.is_some() {
                self.args.stream_mirror_budget()
            } else {
                let chunks = self
                    .world
                    .extent()
                    .size()
                    .iter()
                    .map(|&v| v as u64)
                    .product();
                level_mirror_budget(&self.world, chunks, self.args.field_surface())
            }
        })
    }

    /// `cfg` reserving [`Run::mirror_budget`] with the run's light radius.
    pub(crate) fn mirror_config(&self, cfg: MirrorConfig) -> MirrorConfig {
        MirrorConfig {
            budget_bytes: self.mirror_budget(),
            light_radius: self.args.light.radius(),
            ..cfg
        }
    }

    /// Sizes the frame to the window (`gpu.size`) unless `--render` fixes it. While the
    /// loading screen's worker holds the frame there is none to size: the hand-back sizes it.
    pub(crate) fn resized(&mut self, gpu: &Gpu) {
        if let (Some(gfx), None) = (self.gfx.as_mut(), self.args.render)
            && gfx.frame.output_size() != gpu.size
        {
            let (_, scale, _) = self.args.upscaler_settings(self.modes.upscaler.mode);
            gfx.frame
                .set_sizes(&gpu.device, scale.internal(gpu.size), gpu.size);
        }
    }

    /// Ticks the server, then streams, syncs, marches and presents one frame begun at `t0`;
    /// `dt` is the frame interval the exposure adapts over in a window.
    /// Records a failed mirror sync as the session's failure and ends it.
    pub(crate) fn mirror_failed(&mut self, e: impl std::fmt::Display) -> Flow {
        self.failure.set(format!("mirror sync failed: {e}"));
        Flow::Exit
    }
}

impl Drop for Run {
    /// Writes the `--save` file, drops the server and client, stops and joins the streamer's
    /// workers, then drops the GPU mirror and passes, then the world (swapped for an empty
    /// one, whose drop is free; the server and client held the other references).
    fn drop(&mut self) {
        // The loading screen's worker, when the run ends before it handed the GPU objects
        // back (the window closed during the build): joined, so its objects drop in the
        // GPU stage below and the device outlives no build.
        if let Some(gfx) = self.loading.as_mut().and_then(loading::Loading::take_built) {
            self.gfx = Some(gfx);
        }
        self.save();
        self.hud.run.blocking_reads = self.gfx.as_ref().map(Gfx::blocking_reads);
        for line in self.queue.tally.summary() {
            println!("frames: {line}");
        }
        if let Some(line) = self.hud.run.line() {
            println!("{line}");
        }
        self.write_report();
        if self.args.spawn_hash || !self.args.switch_at.is_empty() {
            println!("{}", switch::spawn_hash_line(&self.world, self.spawn));
        }
        if self.args.census {
            println!("{}", self.world.brick_census());
            println!("packed: {:?}", self.world.packed_stats());
        }
        // The server and client hold the world's other references and their logs; their
        // drop is cheap and is not a traced stage.
        drop(self.net.take());
        if let Some(stream) = self.stream.take() {
            teardown_stage("streamer", || drop(stream));
        }
        if let Some(gfx) = self.gfx.take() {
            teardown_stage("GPU mirror and passes", || drop(gfx));
        }
        let world = std::mem::replace(&mut self.world, Arc::new(World::new(&MaterialTable::new())));
        teardown_stage("world", || drop(world));
    }
}

#[cfg(test)]
mod menu_tests;
#[cfg(test)]
mod picker_tests;
#[cfg(test)]
mod structure_limit_tests;
#[cfg(test)]
mod structure_tests;
#[cfg(test)]
mod tests;
