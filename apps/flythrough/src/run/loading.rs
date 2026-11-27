//! Start-up as stages of steps (ADR-0059): the pipelines built ahead of the first world frame
//! (the frame's own and the entity set, [`svet_render::Frame::prepare_entities`]), the level's
//! first upload or the streamed world's near residency, the light pre-roll
//! ([`svet_render::PreRoll`]) and the field's initial fill.
//! Under `--loading-screen off` [`Run::load_all`] steps them to the end inside `init`; under
//! `on` the pipelines build on a worker thread ([`Build`]) while the frame loop keeps drawing,
//! and each frame spends `--load-slice` of wall time on steps (or waiting on the worker), then
//! draws a loading frame and returns without touching any per-frame world path (the frame
//! counter, the path player, the recorder, the HUD's wall times). The bar reads a
//! [`LoadProgress`] of the stages below, weighted by their share of the loading time read on
//! the RTX 3080 Laptop.

use crate::app::{diag, ms_since};
use crate::cli::{Args, PreRollMode, RunMode};
use crate::edits::{ball, voxel_of};
use crate::now;
use crate::run::hud_feed::HUD_SCALE;
use crate::run::hud_lines::{band_census_line, field_line, field_pass_line};
use crate::run::queue::{COARSE_PRE_ROLL_MS, PRE_ROLL_SAMPLES, queue_view_angles};
use crate::run::{
    ARMS_SET, ARMS_UNITS, BUILD_UNITS, ENTITY_SET, GfxPlan, Run, build_gfx, timed_prepare,
};
use crate::setup::{Gfx, castle_banner, start_camera};
use crate::streaming::NEAR_D2;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use svet_app::{Flow, Gpu, Input, KeyCode};
use svet_render::{
    Camera, EasedBar, LoadProgress, LoadingScreen, LoadingView, PreRoll, PreRollConfig,
    PreRollState, SyncStats, wgpu,
};
use svet_world::ChunkCoord;

/// The pre-roll's cap on its step time, in ms.
pub(crate) const PRE_ROLL_CAP_MS: f64 = 120_000.0;

/// The `first frame light:` line: the candidates with no block and with a coarse one by where
/// they lie against the view (the `bake queue view:` line's counts), read at the first opening
/// after the pre-roll, at frame `frame`.
pub(crate) fn first_light_line(st: &svet_render::QueueStats, frame: u32) -> String {
    let w = st.view_waiting;
    format!(
        "first frame light: waiting (blank/coarse) shell {}/{} in view {}/{} out {}/{}, read at frame {frame} (the first opening after the pre-roll)",
        w[0][0], w[0][1], w[1][0], w[1][1], w[2][0], w[2][1],
    )
}

/// The pre-roll's coarse box and cap in ms of work. `--pre-roll-stepped`: no box and no cap,
/// so the pre-roll ends by its chunks alone. `--pre-roll near|off`: the box is 0 ms, so the
/// pre-roll ends at the step after the near chunks' moment, stepped or not.
pub(crate) fn pre_roll_limits(args: &Args) -> (f64, f64) {
    let (box_ms, cap_ms) = if args.loading.pre_roll_stepped {
        (f64::INFINITY, f64::INFINITY)
    } else {
        (COARSE_PRE_ROLL_MS, PRE_ROLL_CAP_MS)
    };
    match args.pre_roll() {
        PreRollMode::Full => (box_ms, cap_ms),
        PreRollMode::Near | PreRollMode::Off => (0.0, cap_ms),
    }
}

/// The stages' names, as the loading frame and the `loading frame` diag lines show them.
pub(crate) const PIPELINES: &str = "building the pipelines";
pub(crate) const ARMS: &str = "building the light pass arms";
pub(crate) const UPLOAD: &str = "uploading the level";
pub(crate) const RESIDENCY: &str = "streaming the near chunks";
pub(crate) const NEAR: &str = "lighting the near chunks";
pub(crate) const COARSE: &str = "lighting the coarse pass";
pub(crate) const FIELD: &str = "filling the field";

/// The stages' weights: their share of the loading time on the RTX 3080 Laptop, headless at
/// 1280 x 720: the streamed cave's and the castle's near residency about 5 % and the
/// showcase's upload 2 %, the near chunks' light 23 to 30 % (63 % on the showcase), the coarse
/// pass 66 to 72 % (boxed at 5 s; 28 % on the showcase), the field's fill under 1 % in the
/// default look. Ahead of them the pipelines: under the screen the frame's creation and the
/// entity set, about 6 s each when the driver compiles them (together about 60 % of the
/// castle's loading), 0.2 to 0.3 s from a warm shader cache; the weight is the compiled
/// case's, against the other stages' sum of about 1.
pub(crate) const PIPELINES_WEIGHT: f32 = 1.6;
/// The light pass arms' build when the settings at start pick one: 18.2 s read cold on the
/// RTX 3080 Laptop (window 94a's run 6) against the pipelines' about 12 s.
pub(crate) const ARMS_WEIGHT: f32 = 2.4;
pub(crate) const UPLOAD_WEIGHT: f32 = 0.03;
pub(crate) const RESIDENCY_WEIGHT: f32 = 0.05;
pub(crate) const NEAR_WEIGHT: f32 = 0.30;
pub(crate) const COARSE_WEIGHT: f32 = 0.63;
pub(crate) const FIELD_WEIGHT: f32 = 0.02;

/// A loading frame that starts longer than this after the one before logs a `loading gap`
/// diag line naming the stage, in ms.
const GAP_LOG_MS: f64 = 50.0;

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

/// The run's GPU objects building on a worker thread ([`build_gfx`], the entity pipelines
/// included): pipeline creation is synchronous on the calling thread and takes seconds when
/// the driver compiles, so under the loading screen it runs off the event loop. Nothing reads
/// the frame or the mirror until [`Build::join`] hands them back: `Run::gfx` is `None` until
/// then, so a resize has nothing to size and the hand-back sizes the frame to the window.
pub(crate) struct Build {
    worker: Option<JoinHandle<Gfx>>,
    /// Units of the build done, of [`BUILD_UNITS`].
    built: Arc<AtomicU64>,
    /// When the worker started: the stage's time is the worker's wall time.
    began: Instant,
    /// The worker builds the light pass arms after the pipelines ([`ARMS`]).
    arms: bool,
}

impl Build {
    /// Starts the build of `plan` on a worker thread over clones of the device and the queue.
    pub(crate) fn spawn(gpu: &Gpu, plan: GfxPlan) -> Self {
        let built = Arc::new(AtomicU64::new(0));
        let arms = plan.arms;
        let (device, queue, done) = (gpu.device.clone(), gpu.queue.clone(), built.clone());
        let worker = std::thread::Builder::new()
            .name("start-up build".to_owned())
            .spawn(move || build_gfx(&device, &queue, plan, true, &done))
            .expect("the start-up build's thread starts");
        Self {
            worker: Some(worker),
            built,
            began: now(),
            arms,
        }
    }

    /// Whether the worker has ended (its objects wait in [`Build::join`]).
    fn finished(&self) -> bool {
        self.worker.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// Waits up to `limit` for the worker to end, checking every millisecond.
    fn wait(&self, limit: Duration) {
        let until = now() + limit;
        while !self.finished() && now() < until {
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// The units done so far, of [`BUILD_UNITS`].
    fn units(&self) -> u64 {
        self.built.load(Ordering::Relaxed)
    }

    /// The worker's objects, blocking until it ends; `None` once taken. A panic on the worker
    /// resumes here.
    fn join(&mut self) -> Option<Gfx> {
        let worker = self.worker.take()?;
        Some(
            worker
                .join()
                .unwrap_or_else(|e| std::panic::resume_unwind(e)),
        )
    }
}

/// The stage being stepped.
pub(crate) enum Stage {
    /// The pipelines on a worker under the loading screen ([`Build`]); the next stage starts
    /// when it hands the objects back.
    Build(Box<Build>),
    /// The entity pipelines' build without the screen, one step that blocks for its whole
    /// time, then the stage it holds.
    Pipelines(Box<Stage>),
    /// The light pass arms' build without the screen when the settings at start pick an arm,
    /// one blocking step, then the stage it holds.
    Arms(Box<Stage>),
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
    /// The loading frame, under `--loading-screen on`.
    screen: Option<LoadingScreen>,
    /// The bar's shown value.
    bar: EasedBar,
    /// `--loading-fade`, in ms.
    fade_ms: f64,
    /// When the last loading frame was drawn: the fade runs from there.
    ended: Option<Instant>,
    /// `Esc` (or `--quit-at-loading-frame`) came while the worker built: the run ends once it
    /// hands its objects back, the loading frames drawn meanwhile.
    quitting: bool,
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
    /// The step time of each stage, by name, in order, in ms.
    stage_ms: Vec<(&'static str, f64)>,
    /// The run was switched in process: its banner and the menu's notice are shown afresh
    /// from loading's end, so they are not spent behind loading frames.
    switched: bool,
}

impl Loading {
    pub(crate) fn done(&self) -> bool {
        matches!(self.stage, Stage::Done)
    }

    /// Whether the worker is still building.
    fn building(&self) -> bool {
        matches!(&self.stage, Stage::Build(b) if !b.finished())
    }

    /// The worker's objects, when loading still holds it; blocks until it ends. For the run's
    /// teardown.
    pub(crate) fn take_built(&mut self) -> Option<Gfx> {
        let Stage::Build(b) = &mut self.stage else {
            return None;
        };
        let gfx = b.join();
        diag(format_args!("start-up build joined at the run's end"));
        gfx
    }

    /// The current stage's name.
    fn stage_name(&self) -> &'static str {
        match &self.stage {
            Stage::Build(b) if b.arms && b.units() >= BUILD_UNITS => ARMS,
            Stage::Build(_) | Stage::Pipelines(_) => PIPELINES,
            Stage::Arms(_) => ARMS,
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
            Stage::Build(_) | Stage::Pipelines(_) | Stage::Arms(_) | Stage::Field | Stage::Done => {
                String::new()
            }
        }
    }

    fn note_step(&mut self, name: &'static str, ms: f64) {
        match self.longest.iter_mut().find(|(n, _)| *n == name) {
            Some((_, l)) => *l = l.max(ms),
            None => self.longest.push((name, ms)),
        }
        match self.stage_ms.iter_mut().find(|(n, _)| *n == name) {
            Some((_, t)) => *t += ms,
            None => self.stage_ms.push((name, ms)),
        }
    }
}

/// The scene a loading frame names: under `--tester` the path's scene and mode
/// (`castle - benchmark`), else the castle's or the pose's banner, else the source.
pub(crate) fn scene_title(args: &Args) -> String {
    if args.tester
        && let Some(path) = args.path
    {
        let mode = match args.path_mode {
            crate::path::PathMode::Bench => "benchmark",
            crate::path::PathMode::Tour => "tour",
        };
        return format!("{} - {mode}", path.scene.name());
    }
    if args.castle {
        return castle_banner(args.castle_shot);
    }
    if let Some(p) = args.pose {
        return crate::hud::pose_banner(p.name);
    }
    match args.mode {
        RunMode::Cave => "cave".to_owned(),
        RunMode::Level => "level".to_owned(),
        RunMode::Showcase => "showcase".to_owned(),
        RunMode::Outdoor => args.outdoor.trees().source_words().to_owned(),
        RunMode::Room(edge) => format!("room of {edge} voxels"),
        RunMode::ThinRig => "thin rig".to_owned(),
        RunMode::SeamRig => "seam rig".to_owned(),
    }
}

/// The stages a run loads through, with their weights: the pipelines, the light pass arms
/// when the settings at start pick one (`arms`), the level's upload or the streamed near
/// residency, the pre-roll's two, and the field's fill when the run starts in the field.
pub(crate) fn load_stages(streamed: bool, field: bool, arms: bool) -> Vec<(&'static str, f32)> {
    let mut v = vec![(PIPELINES, PIPELINES_WEIGHT)];
    if arms {
        v.push((ARMS, ARMS_WEIGHT));
    }
    v.push(if streamed {
        (RESIDENCY, RESIDENCY_WEIGHT)
    } else {
        (UPLOAD, UPLOAD_WEIGHT)
    });
    v.push((NEAR, NEAR_WEIGHT));
    v.push((COARSE, COARSE_WEIGHT));
    if field {
        v.push((FIELD, FIELD_WEIGHT));
    }
    v
}

impl Run {
    /// Loading's start under the screen: the worker's `build` first, drawn with `drawn` (the
    /// screen the first frame was drawn with, else a new one).
    pub(crate) fn start_loading(
        &mut self,
        gpu: &Gpu,
        drawn: Option<LoadingScreen>,
        build: Build,
    ) -> Loading {
        let screen =
            drawn.unwrap_or_else(|| LoadingScreen::new(&gpu.device, gpu.surface_format, HUD_SCALE));
        self.new_loading(Stage::Build(Box::new(build)), Some(screen))
    }

    /// Loading's start under the screen for a run switched in process
    /// (`run::switch`), its GPU objects rebuilt from the run before's: the entity pipelines
    /// (built already on the kept frame), then the level's upload or the streamed near
    /// residency, each stepped behind `screen`'s loading frames.
    pub(crate) fn start_loading_switched(&mut self, screen: LoadingScreen) -> Loading {
        let next = self.first_stage(true);
        let mut loading = self.new_loading(Stage::Pipelines(Box::new(next)), Some(screen));
        loading.switched = true;
        loading
    }

    /// Loading's start without the screen, the run's GPU objects built: the entity pipelines,
    /// then the level's upload or the streamed near residency.
    pub(crate) fn start_loading_unscreened(&mut self) -> Loading {
        let next = self.first_stage(false);
        self.new_loading(Stage::Pipelines(Box::new(next)), None)
    }

    fn new_loading(&self, stage: Stage, screen: Option<LoadingScreen>) -> Loading {
        let streamed = self.stream.is_some();
        Loading {
            stage,
            progress: LoadProgress::new(&load_stages(
                streamed,
                self.args.field_surface(),
                self.light_arms_at_start(),
            )),
            screen,
            bar: EasedBar::default(),
            fade_ms: self.args.loading.fade_ms,
            ended: None,
            quitting: false,
            slice_ms: self.args.loading.slice_ms,
            began: now(),
            title: scene_title(&self.args),
            frames: 0,
            last: None,
            intervals: Vec::new(),
            longest: Vec::new(),
            stage_ms: Vec::new(),
            switched: false,
        }
    }

    /// The stage after the pipelines, once the run holds its GPU objects: the level's upload
    /// or the streamed near residency. Under the screen (`sliced`) the first stage's syncs
    /// take a time budget of the slice, so no step holds a loading frame for a whole upload;
    /// the run's own budget is restored when the pre-roll starts.
    fn first_stage(&mut self, sliced: bool) -> Stage {
        if let Some(gfx) = self.gfx.as_mut() {
            gfx.mirror.set_focus(self.spawn);
            if sliced {
                gfx.mirror
                    .set_sync_budget_time(Some(Duration::from_secs_f64(
                        self.args.loading.slice_ms * 1e-3,
                    )));
            }
        }
        if self.stream.is_none() {
            return Stage::Upload(Upload::default());
        }
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
    }

    /// The worker's objects handed to the run (blocking until it ends): the frame sized to the
    /// window as it is now, then the next stage.
    fn hand_back(&mut self, gpu: &Gpu, mut build: Build, loading: &mut Loading) -> Stage {
        let Some(gfx) = build.join() else {
            return Stage::Done;
        };
        self.gfx = Some(gfx);
        self.resized(gpu);
        if build.arms {
            loading.progress.enter(ARMS);
        }
        loading.progress.next();
        self.first_stage(true)
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
        if matches!(loading.stage, Stage::Build(_)) {
            let Stage::Build(build) = std::mem::replace(&mut loading.stage, Stage::Done) else {
                unreachable!("matched above");
            };
            let worked = ms_since(build.began);
            loading.stage = self.hand_back(gpu, *build, loading);
            loading.note_step(name, worked.max(ms_since(t)));
            return;
        }
        let Some(mut gfx) = self.gfx.take() else {
            loading.stage = Stage::Done;
            return;
        };
        let stage = std::mem::replace(&mut loading.stage, Stage::Done);
        loading.stage = match stage {
            // Handled above, before the run's objects are taken.
            Stage::Build(_) => Stage::Done,
            Stage::Pipelines(next) => {
                // A walker may be placed on any frame, so the entity pipelines are built
                // before the first world frame, and the frame that draws the first part
                // builds none.
                timed_prepare(&gfx.frame, ENTITY_SET, || {
                    gfx.frame.prepare_entities(&gpu.device)
                });
                diag(format_args!("entity pipelines built"));
                loading.progress.next();
                if self.light_arms_at_start() {
                    Stage::Arms(next)
                } else {
                    *next
                }
            }
            Stage::Arms(next) => {
                timed_prepare(&gfx.frame, ARMS_SET, || {
                    gfx.frame.prepare_light_arms(&gpu.device)
                });
                diag(format_args!("light pass arms built"));
                loading.progress.next();
                *next
            }
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
        // The pre-roll's bakes read the sky and the canopy's style as the frame loop sets them,
        // so a pre-rolled block holds its sky visibility and a column's finality marks it.
        super::frame::set_sky_and_styles(
            &self.modes,
            self.args.mode.is_outdoor(),
            gpu,
            &mut gfx.frame,
        );
        let frame = &mut gfx.frame;
        let settings = svet_render::queue_settings();
        frame.start_bake_queue(
            &gpu.queue,
            &gfx.mirror,
            settings,
            svet_render::DEFAULT_BAKE_SLICE_MS,
        );
        frame.set_bake_budget(self.args.bake.budget);
        frame.set_bake_coarse(self.args.bake.coarse, self.args.bake.near);
        frame.set_bake_digs(self.args.bake.edit_ms, self.args.bake.dig_reach);
        frame.set_bake_ring1_stretch(self.args.bake.ring1_stretch);
        frame.set_bake_reach(self.args.bake.reach);
        frame.set_bake_tiers(self.args.bake.tiers);
        frame.set_bake_drain(self.args.bake.drain);
        frame.share_bake_world(self.world.clone());
        frame.set_bake_thread(&gpu.queue, self.args.bake_thread());
        frame.set_bake_depth(self.args.bake.depth);
        frame.set_bake_slots(self.args.bake.slots);
        if frame.bake_queue().is_some() {
            let (order, withdraw) = (self.args.queue_order(), self.args.bake.queue_withdraw);
            let (census, rule, audit) = (
                self.args.bake.queue_census,
                self.args.bake.queue_rule_reading,
                self.args.bake.queue_rule_audit,
            );
            frame.set_bake_queue(move |q| {
                q.order = order;
                q.withdraw = withdraw;
                q.census_every_opening = census;
                q.rule_reading = rule;
                q.rule_audit = audit;
            });
            self.queue.listing_due = self.args.bake.queue_census;
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
        let mode = self.args.pre_roll();
        let mut near: Vec<ChunkCoord> = (-r..=r)
            .flat_map(|z| {
                (-r..=r).flat_map(move |y| {
                    (-r..=r).map(move |x| {
                        ChunkCoord([centre.0[0] + x, centre.0[1] + y, centre.0[2] + z])
                    })
                })
            })
            .collect();
        // `--pre-roll off` holds no near chunk: the near stage is over at the first step.
        if mode == PreRollMode::Off {
            near.clear();
        }
        l.progress.enter(NEAR);
        l.progress.set(0, Some(near.len() as u64));
        let (box_ms, cap_ms) = pre_roll_limits(&self.args);
        Stage::PreRoll(Box::new(PreRollRun {
            pre: PreRoll::new(PreRollConfig {
                near,
                coarse: self.args.bake.coarse,
                box_ms,
                cap_ms,
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
        // `--pre-roll-stepped`: every submit of the earlier steps has finished before this
        // step polls, so the step reads the same read-backs under either driver (the frame
        // loop's slices and `load_all`'s loop); outside the step's timed span.
        if self.args.loading.pre_roll_stepped {
            let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        }
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
        let resident = stream.map(|s| svet_render::Residency::of_chunks(s.streamer.full_chunks()));
        let (forward, _, _) = camera.basis();
        let (half_angle, frustum, focal) = queue_view_angles(camera.vfov, frame.output_size());
        let view = svet_render::QueueView {
            eye: camera.position.to_array(),
            forward: forward.to_array(),
            resident,
            samples: Some(PRE_ROLL_SAMPLES),
            interval_s: 0.0,
            hold: stream.map(|s| s.rule.r_full + self.args.light.radius() + 1),
            half_angle,
            frustum: Some(frustum),
            full_radius: stream.map(|s| s.rule.r_full),
            lod_radius: stream.map(|s| {
                let r = s.streamer.config().residency;
                r.r_lod.max(r.r_full)
            }),
            focal_px: Some(focal),
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
                let box_ms = p.pre.config().box_ms;
                let by_box = p.pre.near_ms().map_or(0.0, |at| {
                    if box_ms > 0.0 {
                        ((p.work_ms - at) / box_ms).clamp(0.0, 1.0)
                    } else {
                        1.0
                    }
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
        frame: &mut svet_render::Frame,
        p: &PreRollRun,
        s: &svet_render::PreRollSummary,
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
                    p.pre.config().box_ms / 1e3,
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
        if let Some(q) = frame.bake_queue() {
            let qs = q.stats();
            println!(
                "pre-roll: column wait {}, {} jobs opened provisional, {} catch-ups queued, {} chunks provisional",
                if frame.lighting().column_wait {
                    "on"
                } else {
                    "off"
                },
                qs.provisional_opened,
                qs.provisional_requeued,
                qs.provisional
            );
        }
        // The pre-roll's near chunks take the full count under `--near`; the live queue's
        // full-first radius starts here.
        frame.set_bake_full_first(self.args.full_first());
        // The `first frame light:` line reads the census of the first opening after the
        // pre-roll, asked for here.
        self.queue.first_light = Some(st.openings);
        frame.set_bake_queue(svet_render::BakeQueue::request_census);
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
        if let Some(rec) = self.recorder.as_mut() {
            rec.load_stages.clone_from(&loading.stage_ms);
        }
        if loading.screen.is_some() {
            let worst = loading.intervals.iter().copied().fold(0.0, f64::max);
            println!(
                "loading: {} loading frames, longest frame {:.1} ms, {}; {:.0} ms from device ready to the last loading frame",
                loading.frames,
                worst,
                interval_line(&loading.intervals),
                ms_since(loading.began)
            );
        }
        if loading.switched {
            let at = ms_since(self.start);
            self.modes.banner_at = at;
            if let Some((_, said)) = self.notice.as_mut() {
                *said = at;
            }
        }
        diag(format_args!("init done"));
    }

    /// One loading frame: waits for the previous slice's GPU work (so a present never queues
    /// behind more than one slice of bake work), steps until the slice is spent or loading
    /// ends, then draws the loading frame with the eased bar. The frame after the last loading
    /// frame is the first world frame, over which [`Run::fade_loading`] fades the screen out.
    /// `Esc`, or `--quit-at-loading-frame` at its frame, ends the run before the slice: the
    /// ordinary teardown follows, and a benchmark's report reads `aborted` with 0 frames.
    pub(crate) fn loading_frame(
        &mut self,
        gpu: &Gpu,
        input: &Input,
        surface: &wgpu::TextureView,
    ) -> Flow {
        let Some(mut loading) = self.loading.take() else {
            return Flow::Exit;
        };
        if input.pressed(KeyCode::Escape) || self.args.loading.quit_at == Some(loading.frames) {
            loading.quitting = true;
        }
        // A quit while the worker builds waits for it behind loading frames, so the window
        // stays live and the teardown finds no build in flight.
        if loading.quitting && !loading.building() {
            println!("loading: ended at loading frame {}", loading.frames);
            self.loading = Some(loading);
            return Flow::Exit;
        }
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        let t0 = now();
        let dt = loading.last.replace(t0).map_or(0.0, |last| {
            let ms = (t0 - last).as_secs_f64() * 1e3;
            loading.intervals.push(ms);
            if ms > GAP_LOG_MS {
                diag(format_args!(
                    "loading gap {ms:.1} ms before loading frame {} ({})",
                    loading.frames,
                    loading.stage_name()
                ));
            }
            ms * 1e-3
        });
        while !loading.done() && ms_since(t0) < loading.slice_ms {
            if let Stage::Build(b) = &loading.stage {
                // The worker's wait spends the rest of the slice, so the loading frames keep
                // the slice's cadence while it builds.
                b.wait(Duration::from_secs_f64(
                    (loading.slice_ms - ms_since(t0)).max(0.0) * 1e-3,
                ));
                if !b.finished() {
                    if b.arms && b.units() >= BUILD_UNITS {
                        loading.progress.enter(ARMS);
                        loading
                            .progress
                            .set(b.units() - BUILD_UNITS, Some(ARMS_UNITS));
                    } else {
                        loading.progress.set(b.units(), Some(BUILD_UNITS));
                    }
                    break;
                }
            }
            if loading.quitting {
                break;
            }
            self.load_step(gpu, &mut loading);
        }
        if self.failure.is_some() {
            return Flow::Exit;
        }
        if loading.done() {
            loading.bar.snap();
        } else {
            loading.bar.update(loading.progress.fraction(), dt as f32);
        }
        draw_loading(gpu, surface, &mut loading, 1.0);
        diag(format_args!(
            "loading frame {}: {} {:.6}",
            loading.frames,
            loading.stage_name(),
            loading.progress.fraction()
        ));
        loading.frames += 1;
        if loading.done() {
            self.end_loading(&loading);
            // Kept to the first world frame even without a fade, which logs its gap.
            loading.ended = Some(now());
        }
        self.loading = Some(loading);
        Flow::Continue
    }

    /// Whether start-up is still stepping; `false` once the first world frame may draw, the
    /// fade included.
    pub(crate) fn is_loading(&self) -> bool {
        self.loading.as_ref().is_some_and(|l| !l.done())
    }
}

/// Over the first world frames after loading, the loading frame drawn over `surface` at an
/// alpha falling from 1 to 0 over `--loading-fade`; dropped once it reaches 0.
pub(crate) fn fade_loading(slot: &mut Option<Loading>, gpu: &Gpu, surface: &wgpu::TextureView) {
    let Some(loading) = slot.as_mut() else {
        return;
    };
    let Some(ended) = loading.ended else {
        return;
    };
    if let Some(last) = loading.last.take() {
        diag(format_args!(
            "first world frame drawn {:.1} ms after the last loading frame began",
            (now() - last).as_secs_f64() * 1e3
        ));
    }
    let alpha = if loading.fade_ms > 0.0 {
        1.0 - (now() - ended).as_secs_f64() * 1e3 / loading.fade_ms
    } else {
        0.0
    };
    if alpha <= 0.0 {
        *slot = None;
        return;
    }
    draw_loading(gpu, surface, loading, alpha as f32);
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

/// Draws the loading frame over `surface` at `alpha`: the scene, the stage and its count,
/// the eased bar and the seconds since loading began.
fn draw_loading(gpu: &Gpu, surface: &wgpu::TextureView, loading: &mut Loading, alpha: f32) {
    let detail = loading.detail();
    let stage = loading.stage_name();
    let seconds = ms_since(loading.began) as f32 * 1e-3;
    let progress = loading.bar.value();
    let title = format!("loading {}", loading.title);
    let Some(screen) = loading.screen.as_mut() else {
        return;
    };
    draw_view(
        gpu,
        surface,
        screen,
        &LoadingView {
            title: &title,
            stage,
            detail: &detail,
            progress,
            seconds,
            alpha,
        },
    );
}

/// Draws `view` with `screen` over `surface` at the window's current size (`gpu.size`, the
/// surface's): the one route of every loading frame, the first frame after the device
/// ([`crate::app::Flythrough`]'s splash) included, so each is the same picture centred in
/// the window as it is when drawn.
pub(crate) fn draw_view(
    gpu: &Gpu,
    surface: &wgpu::TextureView,
    screen: &mut LoadingScreen,
    view: &LoadingView<'_>,
) {
    let mut enc = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    screen.encode(&gpu.queue, &mut enc, surface, gpu.size, view);
    gpu.queue.submit([enc.finish()]);
}

#[cfg(test)]
mod tests {
    use super::{ARMS, COARSE, FIELD, NEAR, PIPELINES, RESIDENCY, UPLOAD, load_stages};

    /// A run first builds the entity pipelines, then the light pass arms when the settings at
    /// start pick one; then a level loads through its upload, a streamed world through its near
    /// residency, then the pre-roll's two stages, then the field's fill when the run starts in
    /// the field.
    #[test]
    fn the_stages_follow_the_world_and_the_look() {
        let names = |s: Vec<(&'static str, f32)>| s.into_iter().map(|(n, _)| n).collect::<Vec<_>>();
        assert_eq!(
            names(load_stages(false, false, false)),
            [PIPELINES, UPLOAD, NEAR, COARSE]
        );
        assert_eq!(
            names(load_stages(true, true, false)),
            [PIPELINES, RESIDENCY, NEAR, COARSE, FIELD]
        );
        assert_eq!(
            names(load_stages(true, false, true)),
            [PIPELINES, ARMS, RESIDENCY, NEAR, COARSE]
        );
    }
}
