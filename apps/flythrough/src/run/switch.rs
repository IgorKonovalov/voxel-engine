//! The in-process source switch (the menu's `source` and `trees` entries, `--switch-at`): the run is torn down (its server and
//! logic, its stream and its world, and the GPU mirror) and the next `--source` value's run is
//! built as `--source NAME` with the run's other flags would start it, while the window, the
//! `Frame` (its pipelines, targets and LUTs) and the sky's clock are kept. The frame's
//! histories, exposure and entity mirror start afresh; the light store is emptied by the new
//! run's live queue at its pre-roll. Under the loading screen the frame that asked is followed
//! by a loading frame naming the next world, presented before the teardown and the world's
//! build block, and the new run loads behind loading frames as start-up does (ADR-0059).

use super::hud_feed::HUD_SCALE;
use super::loading::{draw_view, scene_title};
use super::{GfxPlan, Run, reserve_mirror, set_plan};
use crate::app::{Flythrough, Pending, diag, ms_since};
use crate::cli::{Args, SOURCES, next_source, parse_args_from};
use crate::outdoor::{GRASS, SOIL};
use crate::setup::{Gfx, build_world, load_logic, start_camera};
use vxe_app::Gpu;
use vxe_render::{LoadingScreen, LoadingView, wgpu};
use vxe_world::{BRICK_VOXELS, ChunkCoord, VoxelCoord, World};

/// The flags of `argv` with every world-picking flag (`--source NAME`, `--level`) dropped and
/// `--source NAME` of `mode` appended.
pub(crate) fn argv_for(argv: &[String], mode: crate::cli::RunMode) -> Vec<String> {
    let mut out = Vec::with_capacity(argv.len() + 2);
    let mut it = argv.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--source" => {
                it.next();
            }
            "--level" => {}
            _ => out.push(a.clone()),
        }
    }
    out.push("--source".to_owned());
    out.push(mode.source_name().unwrap_or("cave").to_owned());
    out
}

/// The arguments of the first source after `args.mode` (in `--source`'s order) whose flags
/// parse beside the run's others; a source that refuses one prints why and the next is
/// tried. `--frames` and `--switch-at` count on from `frames_run`, the frames the run drew.
pub(crate) fn switched_args(args: &Args, frames_run: u32) -> Option<Args> {
    let mut mode = args.mode;
    for _ in 0..SOURCES.len() - 1 {
        mode = next_source(mode);
        match parse_args_from(argv_for(&args.argv, mode)) {
            Ok(mut a) => {
                a.frames = args.frames.map(|n| n.saturating_sub(frames_run));
                a.spawn_hash |= !args.switch_at.is_empty();
                a.switch_at = args
                    .switch_at
                    .iter()
                    .filter(|&&f| f > frames_run)
                    .map(|f| f - frames_run)
                    .collect();
                return Some(a);
            }
            Err(e) => println!(
                "switch: source {} refused: {e}",
                mode.source_name().unwrap_or("?")
            ),
        }
    }
    None
}

/// The flags of `argv` with `--trees` dropped and `--trees NAME` of `trees` appended.
pub(crate) fn argv_with_trees(argv: &[String], trees: crate::trees::TreeDensity) -> Vec<String> {
    let mut out = Vec::with_capacity(argv.len() + 2);
    let mut it = argv.iter();
    while let Some(a) = it.next() {
        if a == "--trees" {
            it.next();
        } else {
            out.push(a.clone());
        }
    }
    out.push("--trees".to_owned());
    out.push(trees.name().to_owned());
    out
}

/// The run's arguments with the next `--trees` density, `--frames` and `--switch-at` counted
/// on from `frames_run` as [`switched_args`] does; `None` when they do not parse.
pub(crate) fn retreed_args(args: &Args, frames_run: u32) -> Option<Args> {
    let trees = args.outdoor.trees().next();
    match parse_args_from(argv_with_trees(&args.argv, trees)) {
        Ok(mut a) => {
            a.frames = args.frames.map(|n| n.saturating_sub(frames_run));
            a.switch_at = args
                .switch_at
                .iter()
                .filter(|&&f| f > frames_run)
                .map(|f| f - frames_run)
                .collect();
            Some(a)
        }
        Err(e) => {
            println!("trees: {} refused: {e}", trees.name());
            None
        }
    }
}

/// The chunks within 2 of `spawn`'s chunk on each axis, in a fixed order.
fn spawn_block(spawn: VoxelCoord) -> Vec<ChunkCoord> {
    let c = spawn.chunk().0;
    let r = -2..=2;
    r.clone()
        .flat_map(|z| {
            r.clone()
                .flat_map(move |y| (-2..=2).map(move |x| [x, y, z]))
        })
        .map(|d| ChunkCoord([c[0] + d[0], c[1] + d[1], c[2] + d[2]]))
        .collect()
}

/// `spawn hash 0x... over 125 chunks around chunk [..], 0 voxels of ids 5 and 6`: the voxel
/// hash (`flythrough::chunks_hash`) of the chunks within 2 of `spawn`'s, and how many of their
/// voxels hold the outdoor rows' ids ([`GRASS`], [`SOIL`]); or how many are not resident.
pub(crate) fn spawn_hash_line(world: &World, spawn: VoxelCoord) -> String {
    let block = spawn_block(spawn);
    let c = spawn.chunk().0;
    let Some(hash) = crate::chunks_hash(world, &block) else {
        let resident = block
            .iter()
            .filter(|&&k| world.store().get(k).is_some())
            .count();
        return format!(
            "spawn hash: {resident} of {} chunks around chunk {c:?} resident",
            block.len()
        );
    };
    let mut outdoor_ids = 0u64;
    for k in &block {
        let Some(chunk) = world.store().get(*k) else {
            continue;
        };
        let chunk = chunk
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for b in chunk.bricks() {
            outdoor_ids += (0..BRICK_VOXELS)
                .filter(|&i| matches!(b.get(i), GRASS | SOIL))
                .count() as u64;
        }
    }
    format!(
        "spawn hash 0x{hash:016x} over {} chunks around chunk {c:?}, {outdoor_ids} voxels of ids \
         {GRASS} and {SOIL}",
        block.len()
    )
}

/// The kept `old` GPU objects under `plan`: the frame with `plan`'s settings, its histories,
/// exposure and entity mirror started afresh, and a new mirror in place of the old one.
fn rebuild_gfx(device: &wgpu::Device, queue: &wgpu::Queue, plan: GfxPlan, old: Gfx) -> Gfx {
    let Gfx {
        mirror,
        mut frame,
        presenter,
        timer,
        field_timer,
        hud,
        hud_timer,
    } = old;
    drop(mirror);
    frame.set_sizes(device, plan.internal, plan.output);
    set_plan(&mut frame, device, queue, &plan);
    frame.reset_history();
    frame.reset_exposure();
    frame.set_entity_mirror(vxe_render::EntityMirror::new(
        device,
        vxe_render::DEFAULT_ENTITY_POOL_BYTES,
    ));
    let mut mirror = reserve_mirror(device, plan.mirror, plan.field);
    if let Some(focus) = plan.focus {
        mirror.set_focus(focus);
    }
    Gfx {
        mirror,
        frame,
        presenter,
        timer,
        field_timer,
        hud,
        hud_timer,
    }
}

impl Run {
    /// Builds the run's GPU objects from `old`'s ([`rebuild_gfx`]) and loads the world. Under
    /// the loading screen the load steps behind loading frames drawn with `screen` (the one
    /// [`Flythrough::begin_switch`] drew, else a new one), as start-up's does after its
    /// pipelines; without it, as `--loading-screen off` does, the frame that switched holds
    /// until the near residency and the pre-roll end.
    fn init_switched(&mut self, gpu: &Gpu, old: Gfx, screen: Option<LoadingScreen>) {
        self.adapter = Some(gpu.adapter.get_info());
        self.surface = Some((gpu.surface_format, gpu.surface_color_space));
        let plan = self.gfx_plan(gpu);
        println!("{}", self.modes.line());
        self.gfx = Some(rebuild_gfx(&gpu.device, &gpu.queue, plan, old));
        if self.args.loading.screen_on(self.args.headless) {
            let screen = screen
                .unwrap_or_else(|| LoadingScreen::new(&gpu.device, gpu.surface_format, HUD_SCALE));
            self.loading = Some(self.start_loading_switched(screen));
            return;
        }
        let loading = self.start_loading_unscreened();
        self.load_all(gpu, loading);
    }
}

/// The loading frame's stage row while the old run is torn down and the next one's world is
/// built: `switching from cave`.
pub(crate) fn switching_stage(from: &Args) -> String {
    format!("switching from {}", scene_title(from))
}

impl Flythrough {
    /// The switch the run asked for, begun after the frame that asked: the next source's
    /// arguments ([`switched_args`]) or the next trees' ([`retreed_args`]), and, under the
    /// loading screen, a loading frame over `surface` naming what loads (`loading outdoor,
    /// trees sparse`), presented before the next frame's [`Flythrough::switch_source`] blocks
    /// on the teardown and the world's build. With nothing to switch to the run goes on.
    pub(crate) fn begin_switch(&mut self, gpu: &Gpu, surface: &wgpu::TextureView) {
        let Some(run) = self.run.as_mut() else {
            return;
        };
        let retree = std::mem::take(&mut run.retree) && !run.switch;
        run.switch = false;
        let next = if retree {
            retreed_args(&run.args, run.frame)
        } else {
            switched_args(&run.args, run.frame)
        };
        let Some(args) = next else {
            return;
        };
        if args.loading.screen_on(args.headless) {
            let mut screen = LoadingScreen::new(&gpu.device, gpu.surface_format, HUD_SCALE);
            let title = format!("loading {}", scene_title(&args));
            let stage = switching_stage(&run.args);
            draw_view(
                gpu,
                surface,
                &mut screen,
                &LoadingView {
                    title: &title,
                    stage: &stage,
                    detail: "",
                    progress: 0.0,
                    seconds: 0.0,
                    alpha: 1.0,
                },
            );
            diag(format_args!("switch splash drawn: {title}, {stage}"));
            self.splash = Some(screen);
        }
        self.switching = Some((args, retree));
    }

    /// The switch [`Flythrough::begin_switch`] began: the run over `args` in place of the
    /// current one, keeping the GPU objects but the mirror, the audio and the clock; `retree`
    /// when only the trees changed.
    pub(crate) fn switch_source(&mut self, gpu: &Gpu, args: Args, retree: bool) {
        let screen = self.splash.take();
        let Some(mut old) = self.run.take() else {
            return;
        };
        let (menu, notice) = (std::mem::take(&mut old.menu), old.notice.take());
        let (Some(gfx), clock) = (old.gfx.take(), old.modes.clock) else {
            (old.menu, old.notice) = (menu, notice);
            self.run = Some(old);
            return;
        };
        self.audio = old.audio.take();
        self.room_probe = old.room_probe.take();
        let (from, frame) = (old.args.mode, old.frame);
        drop(old);
        self.args = args;
        self.logic = Some(load_logic(&self.args.logic));
        let (spawn, pending) = build_world(&mut self.args, &self.failure);
        self.camera = start_camera(&self.args, spawn);
        match pending {
            Pending::Level(g) => match g.join().unwrap_or_else(|e| std::panic::resume_unwind(e)) {
                Ok(world) => self.start_run(world, None),
                Err(e) => return self.failure.set(e),
            },
            Pending::Failed => return,
            streamed => self.start_streamed_run(streamed),
        }
        let now = ms_since(self.start);
        let Some(run) = self.run.as_mut() else {
            return;
        };
        run.modes.clock = clock;
        run.modes.banner_at = now;
        // The menu stays as it was, so its world page can switch again.
        (run.menu, run.notice) = (menu, notice);
        let ops = run.net.as_ref().map_or(0, |n| n.server.ops().len());
        run.say(if retree {
            format!(
                "switch: trees {} after {frame} frames, op log {ops}",
                self.args.outdoor.trees().name()
            )
        } else {
            format!(
                "switch: source {} to {} after {frame} frames, op log {ops}",
                from.source_name().unwrap_or("?"),
                self.args.mode.source_name().unwrap_or("?"),
            )
        });
        run.init_switched(gpu, gfx, screen);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::argv;

    /// The switch cycles the sources in `--source`'s order and back, each starting with its
    /// own banner text (`source NAME`), and the outdoor source names the clock.
    #[test]
    fn the_switch_cycles_the_sources_and_the_banner_names_each() {
        let mut a = parse_args_from(argv(&[
            "--headless",
            "--frames",
            "600",
            "--switch-at",
            "200",
        ]))
        .expect("parses");
        let mut seen = Vec::new();
        for _ in 0..4 {
            a = switched_args(&a, 0).expect("every source parses beside these flags");
            let banner = a.start_modes(0.0).banner(0.0).expect("shows at the start");
            assert!(
                banner.lines().count() <= crate::hud::BANNER_ROWS
                    && banner.lines().all(|l| l.len() <= crate::hud::BANNER_COLS),
                "{banner}"
            );
            let row = banner.lines().skip(1).collect::<Vec<_>>().join(", ");
            seen.push(row.split(',').next().unwrap_or("").to_owned());
            assert_eq!(
                row.contains("time 12:00, day cycle off"),
                a.mode.is_outdoor(),
                "{row}"
            );
            assert!(a.headless && a.frames == Some(600));
        }
        assert_eq!(
            seen,
            [
                "source level",
                "source showcase",
                "source outdoor",
                "source cave"
            ]
        );
        let on = switched_args(&a, 250).expect("parses");
        assert_eq!((on.frames, on.switch_at.as_slice()), (Some(350), &[][..]));
    }

    /// A source that refuses the run's flags is passed over: the castle stands only in the
    /// cave, so the switch from a castle run finds no other source and the run goes on.
    #[test]
    fn a_source_that_refuses_the_flags_is_passed_over() {
        let a = parse_args_from(argv(&["--castle"])).expect("parses");
        assert!(switched_args(&a, 0).is_none());
        let v = argv_for(
            &argv(&["--level", "--seed", "3", "--source", "cave"]),
            crate::cli::RunMode::Outdoor,
        );
        assert_eq!(v, argv(&["--seed", "3", "--source", "outdoor"]));
    }

    /// The loading frame a switch draws names what loads and what it leaves: the menu's
    /// `source` entry from the cave reads `loading level` over `switching from cave`, and its
    /// `trees` entry on the outdoor hills `loading outdoor, trees forest` over `switching from
    /// outdoor, trees sparse`.
    #[test]
    fn the_switch_loading_frame_names_the_next_world() {
        let cave = parse_args_from(argv(&[])).expect("parses");
        let next = switched_args(&cave, 0).expect("the level parses");
        assert_eq!(scene_title(&next), "level");
        assert_eq!(switching_stage(&cave), "switching from cave");
        let outdoor = parse_args_from(argv(&["--source", "outdoor"])).expect("parses");
        let retreed = retreed_args(&outdoor, 0).expect("the next trees parse");
        assert_eq!(scene_title(&retreed), "outdoor, trees forest");
        assert_eq!(
            switching_stage(&outdoor),
            "switching from outdoor, trees sparse"
        );
    }
}
