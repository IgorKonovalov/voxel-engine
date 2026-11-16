//! Start-up as stages of steps (ADR-0059): the level's first upload or the streamed world's
//! near residency, the light pre-roll ([`vxe_render::PreRoll`]) and the field's initial fill.
//! Under `--loading-screen off` [`Run::load_all`] steps them to the end inside `init`; under
//! `on` each frame spends `--load-slice` of wall time on steps, then draws a loading frame and
//! returns without touching any per-frame world path (the frame counter, the path player, the
//! recorder, the HUD's wall times). The bar reads a [`LoadProgress`] of the stages below,
//! weighted by their share of the loading time read on the RTX 3080 Laptop.

use crate::app::{diag, ms_since};
use crate::cli::RunMode;
use crate::edits::{ball, voxel_of};
use crate::now;
use crate::run::Run;
use crate::run::hud_feed::{band_census_line, field_line, field_pass_line};
use crate::run::queue::{COARSE_PRE_ROLL_MS, PRE_ROLL_SAMPLES, queue_view_angles};
use crate::setup::{Gfx, start_camera};
use crate::streaming::NEAR_D2;
use std::time::{Duration, Instant};
use vxe_app::{Flow, Gpu};
use vxe_render::{
    Camera, HudColour, LoadProgress, PreRoll, PreRollConfig, PreRollState, SyncStats, wgpu,
};
use vxe_world::ChunkCoord;

/// The pre-roll's cap on its step time, in ms.
pub(crate) const PRE_ROLL_CAP_MS: f64 = 120_000.0;

/// Cells of the loading frame's text bar.
const BAR_CELLS: usize = 40;

/// The stages' names, as the loading frame and the `loading frame` diag lines show them.
pub(crate) const UPLOAD: &str = "uploading the level";
pub(crate) const RESIDENCY: &str = "streaming the near chunks";
pub(crate) const NEAR: &str = "lighting the near chunks";
pub(crate) const COARSE: &str = "lighting the coarse pass";
pub(crate) const FIELD: &str = "filling the field";

/// The stages' weights: their share of the loading time on the RTX 3080 Laptop, headless at
/// 1280 x 720: the streamed cave's and the castle's near residency about 5 % and the
/// showcase's upload 2 %, the near chunks' light 23 to 30 % (63 % on the showcase), the coarse
/// pass 66 to 72 % (boxed at 5 s; 28 % on the showcase), the field's fill under 1 % in the
/// default look.
pub(crate) const UPLOAD_WEIGHT: f32 = 0.03;
pub(crate) const RESIDENCY_WEIGHT: f32 = 0.05;
pub(crate) const NEAR_WEIGHT: f32 = 0.30;
pub(crate) const COARSE_WEIGHT: f32 = 0.63;
pub(crate) const FIELD_WEIGHT: f32 = 0.02;

/// The coarse stage's count: thousandths of the larger of its chunks passed and its box spent.
const COARSE_UNITS: u64 = 1000;

/// The streamed world's near residency between steps.
pub(crate) struct Residency {
    near: Vec<ChunkCoord>,
    /// The light extraction of the first sync that scanned a brick, and of all of them.
    first_light: Option<SyncStats>,
    light_us: u64,
    syncs: u32,
    mirrored: usize,
    diag_next: Instant,
}

/// The level's first upload between steps: the syncs' totals.
#[derive(Default)]
pub(crate) struct Upload {
    syncs: u32,
    ms: f64,
    stats: SyncStats,
}

/// The light pre-roll between steps: the engine's [`PreRoll`], the camera it bakes for and
/// the work time spent, which its box and cap count.
pub(crate) struct PreRollRun {
    pre: PreRoll,
    camera: Camera,
    centre: ChunkCoord,
    work_ms: f64,
    longest_ms: f64,
}

/// The stage being stepped.
pub(crate) enum Stage {
    Upload(Upload),
    Residency(Residency),
    PreRoll(Box<PreRollRun>),
    Field,
    Done,
}

/// The loading state while start-up steps.
pub(crate) struct Loading {
    pub(crate) stage: Stage,
    pub(crate) progress: LoadProgress,
    /// Whether loading frames are drawn (`--loading-screen on`).
    screen: bool,
    /// Wall time a frame spends on steps, in ms.
    slice_ms: f64,
    /// When loading began (device ready).
    began: Instant,
    /// The scene's row.
    title: String,
    /// Loading frames drawn, and the start of the last one.
    pub(crate) frames: u32,
    last: Option<Instant>,
    /// The intervals between loading frames' starts, in ms.
    pub(crate) intervals: Vec<f64>,
    /// The longest single step of each stage, by name, in ms.
    longest: Vec<(&'static str, f64)>,
}

impl Loading {
    pub(crate) fn done(&self) -> bool {
        matches!(self.stage, Stage::Done)
    }

    /// The current stage's name.
    fn stage_name(&self) -> &'static str {
        match &self.stage {
            Stage::Upload(_) => UPLOAD,
            Stage::Residency(_) => RESIDENCY,
            Stage::PreRoll(p) => match p.pre.state() {
                PreRollState::Near { .. } => NEAR,
                _ => COARSE,
            },
            Stage::Field => FIELD,
            Stage::Done => "done",
        }
    }

    /// The current stage's count for the loading frame's row.
    fn detail(&self) -> String {
        match &self.stage {
            Stage::Upload(u) => format!("{} chunks", u.stats.chunks),
            Stage::Residency(r) => format!("{} / {} chunks", r.mirrored, r.near.len()),
            Stage::PreRoll(p) => match p.pre.state() {
                PreRollState::Near { settled, of } => format!("{settled} / {of} chunks"),
                PreRollState::Coarse { passed, of } => format!("{passed} / {of} chunks"),
                PreRollState::Done(s) => format!("{} chunks", s.baked),
            },
            Stage::Field | Stage::Done => String::new(),
        }
    }

    fn note_step(&mut self, name: &'static str, ms: f64) {
        match self.longest.iter_mut().find(|(n, _)| *n == name) {
            Some((_, l)) => *l = l.max(ms),
            None => self.longest.push((name, ms)),
        }
    }
}

/// The scene a loading frame names: the castle's or the pose's banner, else the source.
pub(crate) fn scene_title(run: &Run) -> String {
    if let Some((row, _)) = &run.scene {
        return row.clone();
    }
    if let Some((row, _)) = &run.pose {
        return row.clone();
    }
    match run.args.mode {
        RunMode::Cave => "cave".to_owned(),
        RunMode::Level => "level".to_owned(),
        RunMode::Showcase => "showcase".to_owned(),
        RunMode::Room(edge) => format!("room of {edge} voxels"),
        RunMode::ThinRig => "thin rig".to_owned(),
        RunMode::SeamRig => "seam rig".to_owned(),
    }
}

/// A `cells`-wide text bar filled to `fraction`.
pub(crate) fn text_bar(fraction: f32, cells: usize) -> String {
    let filled = ((fraction.clamp(0.0, 1.0) * cells as f32).floor() as usize).min(cells);
    format!("[{}{}]", "#".repeat(filled), ".".repeat(cells - filled))
}

/// The stages a run loads through, with their weights: the level's upload or the streamed
/// near residency, the pre-roll's two, and the field's fill when the run starts in the field.
pub(crate) fn load_stages(streamed: bool, field: bool) -> Vec<(&'static str, f32)> {
    let mut v = vec![if streamed {
        (RESIDENCY, RESIDENCY_WEIGHT)
    } else {
        (UPLOAD, UPLOAD_WEIGHT)
    }];
    v.push((NEAR, NEAR_WEIGHT));
    v.push((COARSE, COARSE_WEIGHT));
    if field {
        v.push((FIELD, FIELD_WEIGHT));
    }
    v
}

impl Run {
    /// Loading's start: the first stage, the level's upload or the streamed near residency.
    /// Under the screen the first stage's syncs take a time budget of the slice, so no step
    /// holds a loading frame for a whole upload; the run's own budget is restored when the
    /// pre-roll starts.
    pub(crate) fn start_loading(&mut self, screen: bool) -> Loading {
        let slice_ms = self.args.loading.slice_ms;
        let streamed = self.stream.is_some();
        if let Some(gfx) = self.gfx.as_mut() {
            gfx.mirror.set_focus(self.spawn);
            if screen {
                gfx.mirror
                    .set_sync_budget_time(Some(Duration::from_secs_f64(slice_ms * 1e-3)));
            }
        }
        let field = self.gfx.as_ref().is_some_and(|g| g.mirror.field_surface());
        let stage = if streamed {
            let spawn = self.spawn.chunk();
            let near: Vec<ChunkCoord> = ball(NEAR_D2)
                .into_iter()
                .map(|o| ChunkCoord([0, 1, 2].map(|k| spawn.0[k] + o[k])))
                .collect();
            diag(format_args!(
                "mirror created, streaming {} near chunks",
                near.len()
            ));
            Stage::Residency(Residency {
                near,
                first_light: None,
                light_us: 0,
                syncs: 0,
                mirrored: 0,
                diag_next: now(),
            })
        } else {
            Stage::Upload(Upload::default())
        };
        Loading {
            stage,
            progress: LoadProgress::new(&load_stages(streamed, field)),
            screen,
            slice_ms,
            began: now(),
            title: scene_title(self),
            frames: 0,
            last: None,
            intervals: Vec::new(),
            longest: Vec::new(),
        }
    }

    /// Every stage stepped to its end, in one call: `--loading-screen off`.
    pub(crate) fn load_all(&mut self, gpu: &Gpu, mut loading: Loading) {
        while !loading.done() && !self.failure.is_some() {
            self.load_step(gpu, &mut loading);
        }
        self.end_loading(&loading);
    }

    /// One step of the current stage, timed into the stage's longest; moves to the next stage
    /// when it ends.
    pub(crate) fn load_step(&mut self, gpu: &Gpu, loading: &mut Loading) {
        let name = loading.stage_name();
        let t = now();
        let Some(mut gfx) = self.gfx.take() else {
            loading.stage = Stage::Done;
            return;
        };
        let stage = std::mem::replace(&mut loading.stage, Stage::Done);
        loading.stage = match stage {
            Stage::Upload(u) => self.upload_step(gpu, &mut gfx, u, loading),
            Stage::Residency(r) => self.residency_step(gpu, &mut gfx, r, loading),
            Stage::PreRoll(p) => self.pre_roll_stage(gpu, &mut gfx, p, loading),
            Stage::Field => {
                if gfx.mirror.field_surface() {
                    println!("{}", field_pass_line("initial fill", gfx.timed_field(gpu)));
                    println!("{}", field_line(&gfx.mirror));
                }
                diag(format_args!("field filled"));
                loading.progress.finish();
                Stage::Done
            }
            Stage::Done => Stage::Done,
        };
        self.gfx = Some(gfx);
        if self.failure.is_some() {
            loading.stage = Stage::Done;
        }
        loading.note_step(name, ms_since(t));
    }

    /// One sync of the level's first upload; the upload ends when nothing is pending.
    fn upload_step(&mut self, gpu: &Gpu, gfx: &mut Gfx, mut u: Upload, l: &mut Loading) -> Stage {
        let t = now();
        let mut enc = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let synced = gfx.mirror.sync(&self.world, &gpu.queue, &mut enc);
        u.ms += ms_since(t);
        gpu.queue.submit([enc.finish()]);
        match synced {
            Ok(s) => {
                u.syncs += 1;
                let a = &mut u.stats;
                a.bytes += s.bytes;
                a.bricks += s.bricks;
                a.chunks += s.chunks;
                a.writes += s.writes;
                a.light_samples += s.light_samples;
                a.light_bricks += s.light_bricks;
                a.light_us += s.light_us;
            }
            Err(e) => {
                self.failure.set(format!("mirror sync failed: {e}"));
                return Stage::Done;
            }
        }
        let pending = gfx.mirror.pending_chunks() as u64;
        l.progress
            .set(u.stats.chunks, Some(u.stats.chunks + pending));
        if pending > 0 {
            return Stage::Upload(u);
        }
        let s = &u.stats;
        println!(
            "mirror: uploaded {} B ({} bricks, {} chunks, {} writes) in {:.0} ms over {} syncs; mirror {} B live, {} B allocated; lights: {} samples from {} bricks in {:.3} ms",
            s.bytes,
            s.bricks,
            s.chunks,
            s.writes,
            u.ms,
            u.syncs,
            gfx.mirror.mirror_bytes(),
            gfx.mirror.allocated_bytes(),
            s.light_samples,
            s.light_bricks,
            s.light_us as f64 / 1e3
        );
        diag(format_args!("first sync done"));
        self.enter_pre_roll(gpu, gfx, l)
    }

    /// One tick of the streamed world's near residency: update, poll, apply, sync, track; it
    /// ends once the chunks within squared distance `NEAR_D2` of the spawn chunk are resident
    /// and mirrored full.
    fn residency_step(
        &mut self,
        gpu: &Gpu,
        gfx: &mut Gfx,
        mut r: Residency,
        l: &mut Loading,
    ) -> Stage {
        let Some(stream) = self.stream.as_mut() else {
            return self.enter_pre_roll(gpu, gfx, l);
        };
        let mirror = &mut gfx.mirror;
        if now() >= r.diag_next {
            diag(format_args!(
                "streaming, {} syncs, {} of {} near chunks mirrored",
                r.syncs,
                r.mirrored,
                r.near.len()
            ));
            r.diag_next = now() + Duration::from_secs(2);
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
                r.syncs += 1;
                r.light_us += s.light_us;
                if s.light_bricks > 0 {
                    r.first_light.get_or_insert(s);
                }
            }
            Err(e) => {
                self.failure.set(format!("mirror sync failed: {e}"));
                return Stage::Done;
            }
        }
        stream.track(&ev, mirror);
        r.mirrored = r
            .near
            .iter()
            .filter(|&&c| mirror.is_mirrored(c) && !mirror.is_lod(c))
            .count();
        l.progress.set(r.mirrored as u64, Some(r.near.len() as u64));
        if r.mirrored < r.near.len() {
            std::thread::sleep(Duration::from_millis(1));
            return Stage::Residency(r);
        }
        if let Some(s) = r.first_light {
            println!(
                "lights: first sync with bricks extracted {} samples from {} bricks in {:.3} ms; {} syncs to near residency spent {:.3} ms, {} samples live",
                s.light_samples,
                s.light_bricks,
                s.light_us as f64 / 1e3,
                r.syncs,
                r.light_us as f64 / 1e3,
                mirror.light_samples()
            );
        }
        println!(
            "near residency ({} chunks resident and mirrored) after {:.0} ms",
            r.near.len(),
            ms_since(self.start)
        );
        diag(format_args!("near residency done"));
        self.enter_pre_roll(gpu, gfx, l)
    }

    /// Starts the pre-roll: the live queue started on an empty store with the run's settings,
    /// and the near chunks around the start camera listed.
    fn enter_pre_roll(&mut self, gpu: &Gpu, gfx: &mut Gfx, l: &mut Loading) -> Stage {
        // The run's own sync budget back: the streamed mirror's `--sync-ms`, none on the
        // level's, so the pre-roll streams as the unsliced start does.
        gfx.mirror.set_sync_budget_time(if self.stream.is_some() {
            self.args.sync_time
        } else {
            None
        });
        let frame = &mut gfx.frame;
        let settings = vxe_render::queue_settings();
        frame.start_bake_queue(
            &gpu.queue,
            &gfx.mirror,
            settings,
            vxe_render::DEFAULT_BAKE_SLICE_MS,
        );
        frame.set_bake_budget(self.args.bake.budget);
        frame.set_bake_coarse(self.args.bake.coarse, self.args.bake.near);
        frame.set_bake_digs(self.args.bake.edit_ms, self.args.bake.dig_reach);
        frame.set_bake_reach(self.args.bake.reach);
        if let Some(q) = frame.bake_queue_mut() {
            q.order = self.args.queue_order();
            q.withdraw = self.args.bake.queue_withdraw;
            q.census_every_opening = self.args.bake.queue_census;
        }
        println!("{}", self.queue_row());
        frame.take_prepared(&std::mem::take(&mut self.queue.prepared), &self.world);
        let camera = start_camera(&self.args, self.spawn);
        let centre = ChunkCoord(
            camera
                .position
                .to_array()
                .map(|v| (v / 64.0).floor() as i32),
        );
        let r = self.args.bake.near;
        let near: Vec<ChunkCoord> = (-r..=r)
            .flat_map(|z| {
                (-r..=r).flat_map(move |y| {
                    (-r..=r).map(move |x| {
                        ChunkCoord([centre.0[0] + x, centre.0[1] + y, centre.0[2] + z])
                    })
                })
            })
            .collect();
        l.progress.enter(NEAR);
        l.progress.set(0, Some(near.len() as u64));
        Stage::PreRoll(Box::new(PreRollRun {
            pre: PreRoll::new(PreRollConfig {
                near,
                coarse: self.args.bake.coarse,
                box_ms: COARSE_PRE_ROLL_MS,
                cap_ms: PRE_ROLL_CAP_MS,
                samples: PRE_ROLL_SAMPLES,
            }),
            camera,
            centre,
            work_ms: 0.0,
            longest_ms: 0.0,
        }))
    }

    /// One step of the pre-roll: streaming on the streamed world, then [`PreRoll::step`]. A
    /// failed sync or step records the session's failure and ends loading.
    fn pre_roll_stage(
        &mut self,
        gpu: &Gpu,
        gfx: &mut Gfx,
        mut p: Box<PreRollRun>,
        l: &mut Loading,
    ) -> Stage {
        let t = now();
        let camera = p.camera;
        let frame = &mut gfx.frame;
        let mirror = &mut gfx.mirror;
        if let Some(stream) = self.stream.as_mut() {
            stream
                .streamer
                .update(&self.world, &[voxel_of(camera.position)]);
            // Each tick's time, as the frame loop records it: `track` reads a chunk's wanted
            // tick through it.
            stream.tick_ms.push(ms_since(self.start));
            let ev = stream.streamer.poll(&self.world);
            mirror.apply_events(&ev);
            frame.take_prepared(&ev.prepared, &self.world);
            stream.track(&ev, mirror);
        }
        mirror.set_focus(voxel_of(camera.position));
        let stream = self.stream.as_ref();
        let resident_fn = |c: ChunkCoord| {
            stream.is_some_and(|s| s.streamer.tier(c) == Some(vxe_world::Tier::Full))
        };
        let resident: Option<&dyn Fn(ChunkCoord) -> bool> = stream
            .is_some()
            .then_some(&resident_fn as &dyn Fn(ChunkCoord) -> bool);
        let (forward, _, _) = camera.basis();
        let (half_angle, frustum) = queue_view_angles(camera.vfov, frame.output_size());
        let view = vxe_render::QueueView {
            eye: camera.position.to_array(),
            forward: forward.to_array(),
            resident,
            samples: Some(PRE_ROLL_SAMPLES),
            interval_s: 0.0,
            hold: stream.map(|s| s.rule.r_full + self.args.light.radius() + 1),
            half_angle,
            frustum: Some(frustum),
            full_radius: stream.map(|s| s.rule.r_full),
        };
        let state = p.pre.step(
            frame,
            &gpu.device,
            &gpu.queue,
            mirror,
            &self.world,
            &view,
            p.work_ms,
        );
        if stream.is_some() {
            std::thread::sleep(Duration::from_millis(1));
        }
        let step_ms = ms_since(t);
        p.work_ms += step_ms;
        p.longest_ms = p.longest_ms.max(step_ms);
        let state = match state {
            Ok(s) => s,
            Err(e) => {
                self.failure.set(e.to_string());
                return Stage::Done;
            }
        };
        match state {
            PreRollState::Near { settled, of } => {
                l.progress.set(settled as u64, Some(of as u64));
                Stage::PreRoll(p)
            }
            PreRollState::Coarse { passed, of } => {
                l.progress.enter(COARSE);
                let by_count = (passed as u64 * COARSE_UNITS) / (of.max(1) as u64);
                let by_box = p.pre.near_ms().map_or(0.0, |at| {
                    ((p.work_ms - at) / COARSE_PRE_ROLL_MS).clamp(0.0, 1.0)
                });
                let by_box = (by_box * COARSE_UNITS as f64) as u64;
                l.progress
                    .set(by_count.max(by_box).min(COARSE_UNITS), Some(COARSE_UNITS));
                Stage::PreRoll(p)
            }
            PreRollState::Done(s) => {
                self.end_pre_roll(frame, &p, &s);
                l.progress.enter(COARSE);
                l.progress.next();
                diag(format_args!("pre-roll done"));
                Stage::Field
            }
        }
    }

    /// Prints the pre-roll's lines and hands the live queue its full-first radius.
    fn end_pre_roll(
        &mut self,
        frame: &mut vxe_render::Frame,
        p: &PreRollRun,
        s: &vxe_render::PreRollSummary,
    ) {
        let st = frame.bake_queue().map(|q| q.stats()).unwrap_or_default();
        if s.capped {
            println!("bake queue: the pre-roll did not end in 120 s; going on");
        }
        println!(
            "bake queue: the camera's chunk {:?} and the {} within {} at 4096 by {:.0} ms ({} chunks, {:.3e} samples); the coarse pass at {} by {:.0} ms (+{:.0} ms, +{} chunks, +{:.3e} samples, {} coarse{}); {} listed from the workers; {:.0} ms since the start",
            p.centre.0,
            p.pre.config().near.len(),
            self.args.bake.near,
            s.near_ms,
            s.near_baked,
            s.near_samples as f64,
            self.args.bake.coarse,
            p.work_ms,
            p.work_ms - s.near_ms,
            s.baked - s.near_baked,
            (s.samples - s.near_samples) as f64,
            st.coarse,
            if s.boxed {
                format!(
                    ", ended by its {:.0} s box with the rest of the {} chunks to the queue",
                    COARSE_PRE_ROLL_MS / 1e3,
                    s.coarse_chunks
                )
            } else {
                String::new()
            },
            st.prepared,
            ms_since(self.start)
        );
        println!(
            "pre-roll: {} steps, {} chunks baked, {} samples, {:.0} ms of steps, the longest {:.1} ms",
            s.steps, s.baked, s.samples, p.work_ms, p.longest_ms
        );
        // The pre-roll's near chunks take the full count under `--near`; the live queue's
        // full-first radius starts here.
        frame.set_bake_full_first(self.args.full_first());
    }

    /// Loading's end: the band census and the loading lines.
    fn end_loading(&mut self, loading: &Loading) {
        if let Some(gfx) = self.gfx.as_ref()
            && self.args.look.band_census
            && self.stream.is_none()
        {
            println!("{}", band_census_line(&self.world, &gfx.mirror));
        }
        let longest = loading
            .longest
            .iter()
            .map(|(n, ms)| format!("{n} {ms:.1}"))
            .collect::<Vec<_>>()
            .join(", ");
        println!("loading: the longest step of each stage (ms): {longest}");
        if loading.screen {
            let worst = loading.intervals.iter().copied().fold(0.0, f64::max);
            println!(
                "loading: {} loading frames, longest frame {:.1} ms, {}; {:.0} ms from device ready to the last loading frame",
                loading.frames,
                worst,
                interval_line(&loading.intervals),
                ms_since(loading.began)
            );
        }
        diag(format_args!("init done"));
    }

    /// One loading frame: waits for the previous slice's GPU work (so a present never queues
    /// behind more than one slice of bake work), steps until the slice is spent or loading
    /// ends, then draws the loading rows over a cleared surface. The frame after the last
    /// loading frame is the first world frame.
    pub(crate) fn loading_frame(&mut self, gpu: &Gpu, surface: &wgpu::TextureView) -> Flow {
        let Some(mut loading) = self.loading.take() else {
            return Flow::Exit;
        };
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        let t0 = now();
        if let Some(last) = loading.last.replace(t0) {
            loading.intervals.push((t0 - last).as_secs_f64() * 1e3);
        }
        while !loading.done() && ms_since(t0) < loading.slice_ms {
            self.load_step(gpu, &mut loading);
        }
        if self.failure.is_some() {
            return Flow::Exit;
        }
        if let Some(gfx) = self.gfx.as_mut() {
            draw_loading(gpu, gfx, surface, &loading);
        }
        diag(format_args!(
            "loading frame {}: {} {:.6}",
            loading.frames,
            loading.stage_name(),
            loading.progress.fraction()
        ));
        loading.frames += 1;
        if loading.done() {
            self.end_loading(&loading);
        } else {
            self.loading = Some(loading);
        }
        Flow::Continue
    }
}

/// The loading frames' intervals as `interval p50 .. p95 .. max .. ms over n`.
fn interval_line(intervals: &[f64]) -> String {
    if intervals.is_empty() {
        return "no intervals".to_owned();
    }
    let mut v = intervals.to_vec();
    v.sort_by(f64::total_cmp);
    let p = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
    format!(
        "interval p50 {:.1} p95 {:.1} max {:.1} ms over {}",
        p(0.5),
        p(0.95),
        v[v.len() - 1],
        v.len()
    )
}

/// Clears `surface` and draws the loading rows: the scene, the stage and its count, the text
/// bar and the seconds since loading began.
fn draw_loading(gpu: &Gpu, gfx: &mut Gfx, surface: &wgpu::TextureView, loading: &Loading) {
    let fraction = loading.progress.fraction();
    let detail = loading.detail();
    let rows = [
        format!("loading {}", loading.title),
        format!("{}: {detail}", loading.stage_name()),
        format!(
            "{} {:3.0} %",
            text_bar(fraction, BAR_CELLS),
            fraction * 100.0
        ),
        format!("{:.1} s", ms_since(loading.began) / 1e3),
    ];
    let rows: [(&[u8], HudColour); 4] = [
        (rows[0].as_bytes(), HudColour::Yellow),
        (rows[1].as_bytes(), HudColour::White),
        (rows[2].as_bytes(), HudColour::White),
        (rows[3].as_bytes(), HudColour::White),
    ];
    gfx.hud.set_rows(&gpu.queue, &rows);
    let mut enc = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let _clear = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("loading clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: surface,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.02,
                        g: 0.02,
                        b: 0.03,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
    }
    let [w, h] = gpu.size;
    gfx.hud.encode(&mut enc, surface, w, h);
    gpu.queue.submit([enc.finish()]);
}

#[cfg(test)]
mod tests {
    use super::{COARSE, FIELD, NEAR, RESIDENCY, UPLOAD, load_stages, text_bar};

    /// The text bar fills `floor(fraction * cells)` cells and clamps outside 0 to 1.
    #[test]
    fn the_text_bar_fills_its_fraction() {
        assert_eq!(text_bar(0.0, 4), "[....]");
        assert_eq!(text_bar(0.5, 4), "[##..]");
        assert_eq!(text_bar(0.99, 4), "[###.]");
        assert_eq!(text_bar(1.0, 4), "[####]");
        assert_eq!(text_bar(1.5, 4), "[####]");
        assert_eq!(text_bar(-1.0, 4), "[....]");
    }

    /// A level loads through its upload, a streamed world through its near residency, then
    /// the pre-roll's two stages, then the field's fill when the run starts in the field.
    #[test]
    fn the_stages_follow_the_world_and_the_look() {
        let names = |s: Vec<(&'static str, f32)>| s.into_iter().map(|(n, _)| n).collect::<Vec<_>>();
        assert_eq!(names(load_stages(false, false)), [UPLOAD, NEAR, COARSE]);
        assert_eq!(
            names(load_stages(true, true)),
            [RESIDENCY, NEAR, COARSE, FIELD]
        );
    }
}
