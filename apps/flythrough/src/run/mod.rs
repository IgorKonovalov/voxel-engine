//! TODO

use crate::app::{diag, ms_since};
use crate::audio::{Audio, RoomProbe};
use crate::autopilot::Autopilot;
use crate::castle;
use crate::cli::Args;
use crate::edits::{Coarse, Failure, Net, ball};
use crate::fly::{Fly, Walk};
use crate::hud::{self};
use crate::now;
use crate::path::PathPlayer;
use crate::report::Recorder;
use crate::run::fade::FadeSample;
use crate::run::hud_feed::{
    HUD_SCALE, Hud, Samples, band_census_line, field_line, field_pass_line,
};
use crate::run::queue::QueueRun;
use crate::setup::{CrispRows, Gfx};
use crate::streaming::{NEAR_D2, STREAM_POOL_BYTES, Stream};
use crate::{
    STREAM_MIRROR_BUDGET, ScrollSteps, chunk_ram, level_mirror_budget, level_pool_units, mips_ram,
};
use std::sync::Arc;
use std::time::Instant;
use vxe_app::{Flow, Gpu};
use vxe_render::{
    Camera, FRAME_PASSES, Frame, GpuWorld, HudOverlay, LightParams, LodParams, MarchTimer,
    MirrorConfig, PassTimer, Presenter, SyncStats, Window, wgpu,
};
use vxe_world::{ChunkCoord, MaterialTable, VoxelCoord, World};

pub(crate) mod edit;
pub(crate) mod fade;
pub(crate) mod frame;
pub(crate) mod hud_feed;
pub(crate) mod keys;
pub(crate) mod pilot;
pub(crate) mod queue;
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
    /// `--tester`'s help panel (`F1`) is shown.
    pub(crate) help: bool,
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
        // An adapter below the shaders' floor: name the limit, then request the floor, which
        // the device request refuses on the same limit and the shell reports as a setup error.
        let limits = vxe_render::limits_for(adapter, pool).unwrap_or_else(|e| {
            eprintln!("flythrough: {e}");
            *e.requested
        });
        (adapter.features() & wgpu::Features::TIMESTAMP_QUERY, limits)
    }

    pub(crate) fn init(&mut self, gpu: &Gpu) {
        self.adapter = Some(gpu.adapter.get_info());
        let output = self.args.render.unwrap_or(gpu.size);
        let timer = PassTimer::with_passes(&gpu.device, &gpu.queue, FRAME_PASSES.len() as u32);
        println!("device ready after {:.0} ms", ms_since(self.start));
        let field_timer = MarchTimer::new(&gpu.device, &gpu.queue);
        let (kind, scale, jitter) = self.args.upscaler_settings(self.modes.upscaler.mode);
        let mut frame = Frame::with_sizes(&gpu.device, scale.internal(output), output);
        diag(format_args!(
            "frame created, {} B of targets",
            frame.frame_bytes()
        ));
        frame.set_probe_spacing(&gpu.device, self.args.light.probe_spacing);
        frame.set_jitter(&gpu.device, jitter);
        frame.set_upscaler_kind(&gpu.device, kind);
        frame.set_sharpness(&gpu.queue, self.args.look.sharpen);
        println!("{}", self.modes.upscaler.line());
        frame.set_lighting(self.modes.lighting);
        frame.set_resolution(&gpu.device, self.modes.resolution);
        diag(format_args!(
            "lighting set, {} B of targets",
            frame.frame_bytes()
        ));
        println!("{}", self.modes.line());
        println!("{}", CrispRows::line(self.world.materials()));
        let mirror = if self.stream.is_some() {
            frame.set_lod(LodParams {
                r_full: self.args.residency.r_full,
                forced_level: None,
            });
            self.init_stream(gpu)
        } else {
            let mut mirror =
                self.new_mirror(gpu, MirrorConfig::new(Window::Fixed(self.world.extent())));
            mirror.set_focus(self.spawn);
            let t = now();
            let mut enc = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            let synced = mirror.sync(&self.world, &gpu.queue, &mut enc);
            let sync_ms = ms_since(t);
            gpu.queue.submit([enc.finish()]);
            match synced {
                Ok(s) => println!(
                    "mirror: uploaded {} B ({} bricks, {} chunks, {} writes) in {:.0} ms; mirror {} B live, {} B allocated; lights: {} samples from {} bricks in {:.3} ms",
                    s.bytes,
                    s.bricks,
                    s.chunks,
                    s.writes,
                    sync_ms,
                    mirror.mirror_bytes(),
                    mirror.allocated_bytes(),
                    s.light_samples,
                    s.light_bricks,
                    s.light_us as f64 / 1e3
                ),
                Err(e) => self.failure.set(format!("mirror sync failed: {e}")),
            }
            mirror
        };
        let mut mirror = mirror;
        self.queue_pre_roll(gpu, &mut frame, &mut mirror);
        let mut gfx = Gfx {
            mirror,
            frame,
            presenter: Presenter::new(&gpu.device, gpu.surface_format),
            timer,
            field_timer,
            hud: HudOverlay::new(&gpu.device, gpu.surface_format, HUD_SCALE),
            hud_timer: PassTimer::new(&gpu.device, &gpu.queue),
        };
        if gfx.mirror.field_surface() {
            println!("{}", field_pass_line("initial fill", gfx.timed_field(gpu)));
            println!("{}", field_line(&gfx.mirror));
        }
        if self.args.look.band_census && self.stream.is_none() {
            println!("{}", band_census_line(&self.world, &gfx.mirror));
        }
        self.gfx = Some(gfx);
        diag(format_args!("init done"));
    }

    /// The mirror's VRAM budget: `--mirror-budget`, else the level's from its content and
    /// extent, else the streamed default.
    pub(crate) fn mirror_budget(&self) -> u64 {
        self.args.mirror_budget.unwrap_or_else(|| {
            if self.stream.is_some() {
                STREAM_MIRROR_BUDGET
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

    /// A mirror under `cfg` reserving [`Run::mirror_budget`], with the field's band slots when
    /// the run starts in the field look ([`Args::field_surface`]). Prints what it reserved and how long creating its buffers took.
    pub(crate) fn new_mirror(&self, gpu: &Gpu, cfg: MirrorConfig) -> GpuWorld {
        let cfg = MirrorConfig {
            budget_bytes: self.mirror_budget(),
            light_radius: self.args.light.radius(),
            ..cfg
        };
        let t = now();
        let mirror = if self.args.field_surface() {
            GpuWorld::with_field_surface(&gpu.device, cfg)
        } else {
            GpuWorld::with_config(&gpu.device, cfg)
        };
        let ms = ms_since(t);
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

    /// The streaming mirror, returned once the chunks within squared distance `NEAR_D2` of
    /// the spawn chunk are resident and mirrored full.
    pub(crate) fn init_stream(&mut self, gpu: &Gpu) -> GpuWorld {
        let mut mirror = self.new_mirror(
            gpu,
            MirrorConfig {
                sync_budget_bytes: self.args.sync_budget,
                sync_budget_time: self.args.sync_time,
                record_chunks: Some(crate::stream_record_chunks(self.args.residency)),
                ..MirrorConfig::new(Window::Follow {
                    side: crate::window_side(self.args.residency),
                })
            },
        );
        let Some(stream) = self.stream.as_mut() else {
            return mirror;
        };
        let spawn = self.spawn.chunk();
        let near: Vec<ChunkCoord> = ball(NEAR_D2)
            .into_iter()
            .map(|o| ChunkCoord([0, 1, 2].map(|k| spawn.0[k] + o[k])))
            .collect();
        mirror.set_focus(self.spawn);
        // The light extraction of the first sync that scanned a brick, and of all of them.
        let mut first_light: Option<SyncStats> = None;
        let (mut light_us, mut syncs) = (0u64, 0u32);
        diag(format_args!(
            "mirror created, streaming {} near chunks",
            near.len()
        ));
        let mut diag_next = now();
        loop {
            if now() >= diag_next {
                let mirrored = near
                    .iter()
                    .filter(|&&c| mirror.is_mirrored(c) && !mirror.is_lod(c))
                    .count();
                diag(format_args!(
                    "streaming, {syncs} syncs, {mirrored} of {} near chunks mirrored",
                    near.len()
                ));
                diag_next = now() + std::time::Duration::from_secs(2);
            }
            stream.streamer.update(&self.world, &[self.spawn]);
            stream.tick_ms.push(ms_since(self.start));
            let ev = stream.streamer.poll(&self.world);
            mirror.apply_events(&ev);
            self.queue.prepared.extend(ev.prepared.iter().cloned());
            let mut enc = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            let synced = mirror.sync(&self.world, &gpu.queue, &mut enc);
            gpu.queue.submit([enc.finish()]);
            match synced {
                Ok(s) => {
                    syncs += 1;
                    light_us += s.light_us;
                    if s.light_bricks > 0 {
                        first_light.get_or_insert(s);
                    }
                }
                Err(e) => {
                    self.failure.set(format!("mirror sync failed: {e}"));
                    break;
                }
            }
            stream.track(&ev, &mirror);
            if near
                .iter()
                .all(|&c| mirror.is_mirrored(c) && !mirror.is_lod(c))
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        if let Some(s) = first_light {
            println!(
                "lights: first sync with bricks extracted {} samples from {} bricks in {:.3} ms; {syncs} syncs to near residency spent {:.3} ms, {} samples live",
                s.light_samples,
                s.light_bricks,
                s.light_us as f64 / 1e3,
                light_us as f64 / 1e3,
                mirror.light_samples()
            );
        }
        println!(
            "near residency ({} chunks resident and mirrored) after {:.0} ms",
            near.len(),
            ms_since(self.start)
        );
        mirror
    }

    pub(crate) fn resized(&mut self, gpu: &Gpu) {
        if let (Some(gfx), None) = (self.gfx.as_mut(), self.args.render) {
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
        self.save();
        self.hud.run.blocking_reads = self.gfx.as_ref().map(Gfx::blocking_reads);
        for line in self.queue.tally.summary() {
            println!("frames: {line}");
        }
        if let Some(line) = self.hud.run.line() {
            println!("{line}");
        }
        self.write_report();
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
mod tests;
