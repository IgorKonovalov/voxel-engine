//! The engine's testbed (milestone 1 in `docs/roadmap.md`): a window showing the ray-marched
//! cave lit by a headlamp. Fly, dig, and read the per-material counts each edit returns.
//!
//! By default the cave is unbounded and streams around the camera (ADR-0003): full detail
//! within `r_full` chunks, mips out to `r_lod`. `--level` generates the bounded level instead.
//!
//! `flythrough [--seed N] [--level] [--vault] [--walk] [--render WxH] [--frames N] [--bench N]
//! [--autopilot] [--census] [--sync-budget MIB] [--mirror-budget MIB] [--residency R_FULL,R_LOD]
//! [--headless] [--look MODE] [--rock-detail SCALE,STRENGTH] [--view VIEW] [--detail]
//! [--translucent-layers N] [--hud] [--save PATH] [--replay PATH] [--logic native|wasm[=PATH]]
//! [--audio] [--audio-debug] [--room METRES] [--room-line] [--absorption A] [--mic]`.
//! `--render` fixes the marcher's resolution (the window scales it); `--frames` exits after N
//! frames, printing the `F1` readings over the last ones and the stats HUD's rows; `--hud` starts
//! with the stats HUD shown (`F2` toggles it); `--bench` sends N `B` bench inputs on the first
//! frame; `--autopilot` digs, flies 300 m out and back and prints the streaming readings (see
//! [`Autopilot`]); `--census` prints the world's brick census (`World::brick_census`) on exit,
//! after any bench; `--sync-budget` sets the mirror's upload bytes per frame; `--mirror-budget`
//! sets the VRAM the mirror reserves for its pool, masks and records at startup (ADR-0013;
//! default 3072 MiB streamed, and on the level what its bricks, the dig headroom and its chunks
//! need, with records for the residency's ball when streaming), and while streaming the full
//! radius follows the mirror's pressure (the fuller of its pool slots and records): a chunk down after 30 frames above 0.9, a chunk up after 300 frames below
//! 0.7, between 6 and the configured radius, each change printed and on the HUD;
//! `--residency` sets the streamed radii (default `flythrough::RESIDENCY`, hysteresis 1);
//! `--headless` runs the
//! same frames on an offscreen target with no window; `--walk` (with `--level`) starts in walk
//! mode, the body spawned in the spawn pocket. The fly-through starts in contour with detail on,
//! the mirror holding attribute pages and rock its procedural detail (`--rock-detail` sets rock's
//! feature scale in voxels and strength, default `flythrough::ROCK_DETAIL`), and prints that
//! look first; `--look` picks the starting normal mode (`face`, `trilinear8`, `gradient27`,
//! `contour`), and `--look plain` starts in face mode without attribute pages or rock detail,
//! detail off (`N` then skips contour, `--detail` turns detail on), the look of Plan 0002's and
//! Plan 0017's face readings. With attribute pages the autopilot's report adds the attribute
//! pass's work list, CPU and GPU time over the flight. `--view`
//! starts in a view (`shaded`, `normals`, `hitmask`, `steps`, `albedo`; with `--frames`, `steps`
//! prints the last frame's mean and p99 steps per ray on exit);
//! `--translucent-layers` sets how many translucent voxels are composited in front of the opaque
//! hit (default 4, 0 ignores them). `N` cycles the normal mode and `V` toggles detail. `--vault` places a 16-voxel room with an indestructible
//! shell 10 m along +x from the spawn, written over the cave as it generates (on the level, clamped
//! inside the extent).
//!
//! `P` stamps that 16-voxel room at runtime, unrotated, with its minimum corner at the voxel
//! the dig ray hits, and prints what it changed like a dig does.
//!
//! Every edit goes through the simulation split (ADR-0007, ADR-0010): digs, fills, stamps, the
//! bench and `R` are inputs a shared client sends to a server running
//! `flythrough::logic::DigLogic` (`--logic
//! native`, the default) or the same logic as a component hosted on `wasmtime` (`--logic wasm`,
//! from `target/guests/flythrough-logic.wasm`, or `--logic wasm=PATH`; needs a build with
//! `--features wasm`, and `scripts/build-guests.sh` builds the component). The fly-through prints
//! which runs; a hosted logic that fails (a trap, its fuel, its memory cap) is printed once and
//! stopped, and the session continues without it. Replay re-simulates through the chosen logic too.
//! One server tick runs per frame before the mirror sync, and the printed edit lines come from the
//! client's events. With `--level`, `--save` writes the session's log file on exit and prints the
//! world's hash; `--replay` rebuilds a saved session's world by re-simulating its inputs before the
//! first frame and prints the same hash. Everything the session changes is in the log: digs,
//! fills, the bench, `P`'s stamps (the save's header carries the room's content, ADR-0014) and
//! the collapse's drops and removals. Only the autopilot's carve (streaming only) applies to the
//! world directly. `--vault` combines with both, but the room placed at generation is not an
//! op and the save does not record it: a session saved with `--vault` replays to its world
//! with `--replay PATH --vault`.
//!
//! Keys: `G` toggles fly and walk mode (on the bounded level only; streaming refuses it, as it
//! refuses `--walk`); in walk mode `W A S D` walk at 3 m/s and `Space` jumps, or, held against
//! a wall with a standable top within 1.2 m, climbs it. On the bounded level the fly-through
//! keeps a coarse grid and walkable map updated from every edit; `K` prints them.
//!
//! Collapse (bounded level, ADR-0008): after every dig the logic asks the server's coarse grid
//! which regions lost their support, and the fly-through prints each as `unsupported: N cells,
//! {material: count}`. Each region falls 8 voxels a tick, one tick a frame, until it lands
//! (`landed after N ticks, fell M voxels`); `R` toggles removing it with rubble (a quarter of
//! its volume) instead. `F1` adds the region count of the last dig's query.
//!
//! `--audio` opens the default output device, `H` plays a knock and `J` a looping hum at the point
//! the last left click dug; every frame probes their occlusion through the world. `T` plays a
//! looping pulse (a knock each second) there, to hear the echoes in its gaps; `E` toggles the
//! early reflections. `--audio-debug` is
//! `--audio` plus a line a second of the output callback's readings, a `room sent: T60 .. mfp ..
//! energy ..` line for every room the tail is sent, a line when room fans start being discarded
//! (the listener inside a solid) and one when they stop, and on exit writes the last
//! 60 s the callback handed the device to `flythrough-audio-debug.wav`. `FLYTHROUGH_HRIR=<pack>`
//! in the environment loads that HRIR pack instead of the shipped one. `Y` toggles the late
//! tail. `--absorption A` sets the cave row's absorption per metre (default 0.3). `--mic`
//! (implies `--audio`) plays the default input device at the listener with its direct path
//! off, so it is heard only off the walls and in the tail; wear headphones.
//!
//! `--room METRES` runs in a closed cube of air of that edge inside 2 m of rock, spawned at its
//! centre, and implies `--room-line`: a line `room: V .. S .. mfp .. alpha .. T60 .. energy ..`
//! whenever the listener's room fan completes (`F1` and `--audio-debug` print it too). Without
//! `--audio`, `--room-line` runs the room fan with no output device: its block clock never
//! advances, so a fan completes 8 frames after the start or after a 1 m move.

mod fly;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

use fly::{Fly, Walk};
use flythrough::cave::{CaveSource, LEVEL_CHUNKS, ROCK, RoomSource, VAULT_EDGE, vault};
use flythrough::hud::{self, Cadence, EditLine, FrameStats, HudStats, WorldSample};
use flythrough::logic::{BENCH_CARVES, BENCH_VERB, DIG_VERB, DigInput, FILL_VERB, REACH};
use flythrough::logic::{DROP_VERB, REMOVE_VERB, STAMP_VERB, parse_region_line};
use flythrough::{
    LogicChoice, RESIDENCY, STREAM_MIRROR_BUDGET, ScrollSteps, cave_materials, chunk_ram,
    chunks_hash, level_mirror_budget, level_pool_slots, mips_ram, percentile,
};
use vxe_app::{App, Flow, Gpu, Input, KeyCode, MouseButton, ShellConfig};
use vxe_audio::{
    Acoustics, BlockClock, Command, Hrtf, Mixer, Output, PlayParams, RoomEstimate, RoomTargets,
    Sound, VoiceId,
};
use vxe_net::{Client, Event, Logic, LogicError, MemChannel, Server};
use vxe_render::glam::Vec3;
use vxe_render::gpu_world;
use vxe_render::readback::{self, StepHistogram};
use vxe_render::{
    Camera, GpuWorld, HudColour, HudOverlay, LightParams, LodParams, LookParams, MarchTimer,
    Marcher, MirrorConfig, NormalMode, PassTimer, Presenter, SyncStats, ViewMode, Window, wgpu,
};
use vxe_world::hash::mix64;
use vxe_world::{
    CellChange, ChunkCoord, ChunkExtent, CoarseGrid, EditOp, EditResult, FIXED_ONE, Fixed3,
    LogFile, MaterialId, MaterialTable, Mode, OpResult, PersistConfig, Residency, StreamConfig,
    Streamer, SupportParams, Tier, VoxelCoord, WalkableMap, WalkableParams, World,
};

/// Dig radius at start (0.5 m), its scroll step (0.1 m) and its range.
const DIG_RADIUS: i64 = 5 * FIXED_ONE;
const DIG_STEP: i64 = FIXED_ONE;
const DIG_RANGE: (i64, i64) = (FIXED_ONE, 30 * FIXED_ONE);
/// The dig radius after a frame's `delta` scroll lines banked in `scroll`: whole steps of
/// [`DIG_STEP`], clamped to [`DIG_RANGE`]; `None` when it does not change. At either end of
/// the range, scroll pushing past it is dropped rather than banked, so the first line back
/// steps at once.
fn scroll_dig_radius(radius: i64, scroll: &mut ScrollSteps, delta: f32) -> Option<i64> {
    let steps = scroll.add(delta);
    let r = (radius + steps * DIG_STEP).clamp(DIG_RANGE.0, DIG_RANGE.1);
    if (r == DIG_RANGE.1 && scroll.carry() > 0.0) || (r == DIG_RANGE.0 && scroll.carry() < 0.0) {
        scroll.clear();
    }
    (r != radius).then_some(r)
}

/// Radius of the right-click fill: 0.3 m.
const FILL_RADIUS: i64 = 3 * FIXED_ONE;
/// The pool size a streaming device's limits are requested for; capped at the adapter.
const STREAM_POOL_BYTES: u64 = 2 << 30;
/// Frames `F1` averages over.
const SAMPLES: usize = 30;
/// The first frame waits for the chunks within this squared distance of the spawn chunk.
const NEAR_D2: i32 = 4;
/// Default mirror upload budget per frame.
const SYNC_BUDGET_MIB: u64 = 2;
/// Autopilot: dig reach (30 m) and radius (1 m), flight speed (10 m/s) and leg length.
const PILOT_REACH: i64 = 300 * FIXED_ONE;
const PILOT_RADIUS: i64 = 10 * FIXED_ONE;
const PILOT_SPEED: f32 = 100.0;
const PILOT_LEG_SECS: f64 = 30.0;
/// Walkable cells: at most 64 solid voxels, on a floor cell of at least 448, under two open
/// cells (1.6 m).
const WALKABLE: WalkableParams = WalkableParams {
    max_solid: 64,
    min_floor: 448,
    clearance: 2,
};
/// The stats HUD's glyph scale: an 8 x 8 glyph covers 16 x 16 pixels.
const HUD_SCALE: u32 = 2;
/// Visible frames between two samples of the world's counts; see [`Cadence`]. One sample per
/// frame-time window, because one `World::stats` call on the streamed cave at full residency
/// costs more than the 1 ms streaming line (measured by the ignored test
/// `world_stats_call_on_the_streamed_world_at_full_residency`), so its frame reaches at most
/// the window's max and not its p99.
const HUD_STATS_EVERY: u32 = 240;
/// The support query the logic asks after a dig, set on the server: cells of at least 64
/// solid voxels bear load, a flood gives up past 4096 cells, and the level's faces anchor
/// (ADR-0008).
const SUPPORT: SupportParams = SupportParams {
    solid_min: 64,
    max_region: 4096,
    boundary_anchors: true,
};
/// Metres per voxel, for the audio mixer's distances.
const METRES_PER_VOXEL: f32 = 0.1;
/// The knock: 50 ms of seeded noise under an exponential decay of 10 ms time constant.
const KNOCK_SEED: u64 = 0x0006_0001;
const KNOCK_SECONDS: f32 = 0.05;
const KNOCK_DECAY_SECONDS: f32 = 0.01;
/// The hums `J` plays in turn, one note per press so each source can be told apart by ear:
/// an A minor pentatonic from A3 to D5, rounded to whole hertz so one second holds a whole
/// number of cycles and loops cleanly.
const HUM_HZ: [u32; 8] = [220, 262, 294, 330, 392, 440, 523, 587];
const HUM_AMPLITUDE: f32 = 0.3;
/// The acoustic triple (absorption per metre, transmission, resonance) the probes give every
/// solid cave row; the cave's own table leaves it zero, which would block all sound.
const CAVE_ACOUSTIC: [f32; 3] = [0.3, 0.5, 0.0];

#[allow(
    clippy::disallowed_methods,
    reason = "the fly-through times startup, edits and frames for its printed readings"
)]
fn now() -> Instant {
    Instant::now()
}

fn ms_since(t: Instant) -> f64 {
    (now() - t).as_secs_f64() * 1e3
}

#[derive(Clone)]
struct Args {
    seed: u64,
    render: Option<[u32; 2]>,
    frames: Option<u32>,
    bench: u32,
    level: bool,
    autopilot: bool,
    headless: bool,
    walk: bool,
    /// `--vault`: the cave generates with the vault room ahead of the spawn.
    vault: bool,
    sync_budget: u64,
    /// `--mirror-budget`: the mirror's VRAM reservation in bytes.
    mirror_budget: Option<u64>,
    /// `--residency`: the streamed full and LOD radii in chunks, hysteresis as
    /// [`RESIDENCY`]'s.
    residency: Residency,
    audio: bool,
    /// `--audio-debug`: print the callback's readings and record its output.
    audio_debug: bool,
    /// `--look`: the starting normal mode, with attribute pages and rock detail; `None` for
    /// `--look plain`, face mode without either. Contour by default.
    look: Option<NormalMode>,
    /// Start with procedural detail on: by default, except under `--look plain`, where
    /// `--detail` turns it on.
    detail: bool,
    /// `--rock-detail`: rock's feature scale (voxels) and strength under `--look`.
    rock_detail: [f32; 2],
    view: ViewMode,
    /// `--translucent-layers`: translucent voxels composited in front of the opaque hit.
    translucent_layers: u32,
    /// `--hud`: start with the stats HUD shown.
    hud: bool,
    /// `--census`: print the brick census on exit.
    census: bool,
    /// `--save`: where the session's log file is written on exit.
    save: Option<std::path::PathBuf>,
    /// `--replay`: the log file whose session is re-simulated before the first frame.
    replay: Option<std::path::PathBuf>,
    /// `--logic`: the server's logic, native or the hosted component.
    logic: LogicChoice,
    /// `--room`: the edge, in voxels, of the closed cube of air the fly-through runs in.
    room: Option<i32>,
    /// `--room-line`: print the room line whenever a fan completes (implied by `--room`).
    room_line: bool,
    /// `--absorption`: the cave row's absorption per metre.
    absorption: f32,
    /// `--mic`: play the default input device at the listener, heard off the walls.
    mic: bool,
}

/// Why walk mode is refused while streaming, for `--walk` without `--level` and for `G`.
const WALK_NEEDS_LEVEL: &str =
    "walk mode needs --level (the coarse grid is not wired to streaming)";

/// Why `--save` and `--replay` are refused while streaming: a replay regenerates a bounded
/// world, and a streaming session's residency is not in its log.
const SAVE_NEEDS_LEVEL: &str = "--save and --replay need --level (replay rebuilds a bounded world)";

/// Walk mode is available only on the bounded level.
fn walk_allowed(streaming: bool) -> Result<(), &'static str> {
    if streaming {
        Err(WALK_NEEDS_LEVEL)
    } else {
        Ok(())
    }
}

impl Args {
    /// The look the fly-through starts in.
    fn start_look(&self) -> LookParams {
        LookParams {
            normal_mode: self.look.unwrap_or_default(),
            detail: self.detail,
            view: self.view,
            translucent_layers: self.translucent_layers,
        }
    }

    /// Rock's detail pair, in every look but `--look plain`.
    fn rock_detail(&self) -> Option<[f32; 2]> {
        self.look.is_some().then_some(self.rock_detail)
    }
}

fn parse_args() -> Result<Args, String> {
    parse_args_from(std::env::args().skip(1))
}

/// The arguments after the program name.
fn parse_args_from(argv: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut args = Args {
        seed: 1,
        render: None,
        frames: None,
        bench: 0,
        level: false,
        autopilot: false,
        headless: false,
        walk: false,
        vault: false,
        sync_budget: SYNC_BUDGET_MIB << 20,
        mirror_budget: None,
        residency: RESIDENCY,
        audio: false,
        audio_debug: false,
        look: Some(NormalMode::Contour),
        detail: false,
        rock_detail: flythrough::ROCK_DETAIL,
        view: ViewMode::Shaded,
        translucent_layers: LookParams::default().translucent_layers,
        hud: false,
        census: false,
        save: None,
        replay: None,
        logic: LogicChoice::Native,
        room: None,
        room_line: false,
        absorption: CAVE_ACOUSTIC[0],
        mic: false,
    };
    let mut detail_flag = false;
    let mut it = argv.into_iter();
    while let Some(flag) = it.next() {
        let mut value = || it.next().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--seed" => args.seed = value()?.parse().map_err(|e| format!("--seed: {e}"))?,
            "--render" => {
                let v = value()?;
                let (w, h) = v.split_once('x').ok_or("--render takes WxH")?;
                let parse = |s: &str| s.parse::<u32>().map_err(|e| format!("--render: {e}"));
                args.render = Some([parse(w)?, parse(h)?]);
            }
            "--frames" => {
                args.frames = Some(value()?.parse().map_err(|e| format!("--frames: {e}"))?)
            }
            "--bench" => args.bench = value()?.parse().map_err(|e| format!("--bench: {e}"))?,
            "--sync-budget" => {
                let mib: u64 = value()?
                    .parse()
                    .map_err(|e| format!("--sync-budget: {e}"))?;
                args.sync_budget = mib << 20;
            }
            "--residency" => args.residency = parse_residency(&value()?)?,
            "--mirror-budget" => {
                let mib: u64 = value()?
                    .parse()
                    .map_err(|e| format!("--mirror-budget: {e}"))?;
                args.mirror_budget = Some(mib << 20);
            }
            "--look" => {
                let v = value()?;
                args.look = if v == "plain" {
                    None
                } else {
                    Some(NormalMode::from_name(&v).ok_or(format!(
                        "--look takes plain, face, trilinear8, gradient27 or contour, not {v}"
                    ))?)
                };
            }
            "--rock-detail" => args.rock_detail = parse_rock_detail(&value()?)?,
            "--view" => {
                let v = value()?;
                args.view = ViewMode::from_name(&v).ok_or(format!(
                    "--view takes shaded, normals, hitmask, steps or albedo, not {v}"
                ))?;
            }
            "--translucent-layers" => {
                let v = value()?;
                args.translucent_layers = v
                    .parse()
                    .map_err(|_| format!("--translucent-layers takes a count, not {v}"))?;
            }
            "--save" => args.save = Some(value()?.into()),
            "--replay" => args.replay = Some(value()?.into()),
            "--logic" => args.logic = LogicChoice::parse(&value()?)?,
            "--room" => {
                let v = value()?;
                let m: f32 = v
                    .parse()
                    .map_err(|_| format!("--room takes an edge in metres, not {v}"))?;
                if !(1.0..=100.0).contains(&m) {
                    return Err(format!("--room takes 1 to 100 metres, not {v}"));
                }
                args.room = Some((m / METRES_PER_VOXEL).round() as i32);
                args.room_line = true;
            }
            "--absorption" => {
                let v = value()?;
                let a: f32 = v
                    .parse()
                    .map_err(|_| format!("--absorption takes a value in [0, 1], not {v}"))?;
                if !(0.0..=1.0).contains(&a) {
                    return Err(format!("--absorption takes a value in [0, 1], not {v}"));
                }
                args.absorption = a;
            }
            "--room-line" => args.room_line = true,
            "--mic" => {
                args.mic = true;
                args.audio = true;
            }
            "--level" => args.level = true,
            "--detail" => detail_flag = true,
            "--autopilot" => args.autopilot = true,
            "--headless" => args.headless = true,
            "--walk" => args.walk = true,
            "--hud" => args.hud = true,
            "--census" => args.census = true,
            "--vault" => args.vault = true,
            "--audio" => args.audio = true,
            "--audio-debug" => {
                args.audio = true;
                args.audio_debug = true;
            }
            _ => return Err(format!("unknown argument {flag}")),
        }
    }
    args.detail = detail_flag || args.look.is_some();
    if args.rock_detail != flythrough::ROCK_DETAIL && args.look.is_none() {
        return Err("--rock-detail does not apply under --look plain".to_owned());
    }
    if args.autopilot && args.level {
        return Err("--autopilot streams; it does not take --level".to_owned());
    }
    if args.room.is_some()
        && (args.level
            || args.autopilot
            || args.vault
            || args.save.is_some()
            || args.replay.is_some())
    {
        return Err(
            "--room builds its own world; it does not take --level, --autopilot, --vault, \
             --save or --replay"
                .to_owned(),
        );
    }
    if args.walk {
        walk_allowed(!args.level && args.room.is_none()).map_err(|e| format!("--walk: {e}"))?;
    }
    if (args.save.is_some() || args.replay.is_some()) && !args.level {
        return Err(SAVE_NEEDS_LEVEL.to_owned());
    }
    if args.save.is_some() && args.replay.is_some() {
        return Err("--save and --replay do not combine".to_owned());
    }
    if matches!(args.logic, LogicChoice::Wasm(_)) && !cfg!(feature = "wasm") {
        return Err(flythrough::WASM_NEEDS_FEATURE.to_owned());
    }
    Ok(args)
}

/// `R_FULL,R_LOD`: radii in chunks with `1 <= R_FULL <= R_LOD`, hysteresis as
/// [`RESIDENCY`]'s.
fn parse_residency(v: &str) -> Result<Residency, String> {
    let err = || format!("--residency takes R_FULL,R_LOD (e.g. 14,22), not {v}");
    let (full, lod) = v.split_once(',').ok_or_else(err)?;
    let parse = |s: &str| s.trim().parse::<i32>().map_err(|_| err());
    let [r_full, r_lod] = [parse(full)?, parse(lod)?];
    if r_full < 1 || r_lod < r_full {
        return Err(err());
    }
    Ok(Residency {
        r_full,
        r_lod,
        ..RESIDENCY
    })
}

/// `SCALE,STRENGTH`: a feature scale in voxels above 0 and a strength in `[0, 1]`.
fn parse_rock_detail(v: &str) -> Result<[f32; 2], String> {
    let err = || format!("--rock-detail takes SCALE,STRENGTH (e.g. 4,0.3), not {v}");
    let (scale, strength) = v.split_once(',').ok_or_else(err)?;
    let parse = |s: &str| s.trim().parse::<f32>().map_err(|_| err());
    let [scale, strength] = [parse(scale)?, parse(strength)?];
    if !(scale > 0.0 && scale.is_finite()) {
        return Err(format!("--rock-detail: scale must be above 0, not {scale}"));
    }
    if !(0.0..=1.0).contains(&strength) {
        return Err(format!(
            "--rock-detail: strength must be in [0, 1], not {strength}"
        ));
    }
    Ok([scale, strength])
}

/// The verb of a `DigLogic` `op` line, as the `'static` the edit lines and the HUD hold;
/// `None` for a line that is not one of its verbs.
fn op_verb(verb: &str) -> Option<&'static str> {
    [
        DIG_VERB,
        FILL_VERB,
        BENCH_VERB,
        STAMP_VERB,
        DROP_VERB,
        REMOVE_VERB,
    ]
    .into_iter()
    .find(|&v| v == verb)
}

/// One printed line per edit: `carved: rock 1234, ore 12 (3 chunks, 0.21 ms)`.
fn edit_line(verb: &'static str, r: &EditResult, table: &MaterialTable, ms: f64) -> String {
    edit_record(verb, r, table, ms).to_string()
}

/// The edit as the HUD's last-edit row holds it.
fn edit_record(verb: &'static str, r: &EditResult, table: &MaterialTable, ms: f64) -> EditLine {
    EditLine {
        verb,
        counts: r
            .changed_from
            .iter()
            .map(|&(m, n)| (material_name(table, m), n))
            .collect(),
        chunks: r.touched.len() as u32,
        ms: ms as f32,
    }
}

fn material_name(table: &MaterialTable, m: MaterialId) -> String {
    table
        .get(m)
        .map_or_else(|| format!("#{m}"), |row| row.name.clone())
}

/// The process's peak resident set (`VmHWM`) in bytes, read from `/proc/self/status`; `None`
/// where that file does not exist or does not carry the line.
fn peak_rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
    let kb: u64 = line
        .trim_start_matches("VmHWM:")
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse()
        .ok()?;
    Some(kb * 1024)
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// SplitMix64 noise in `[-1, 1)` under an exponential decay: the knock `H` plays.
fn knock() -> Sound {
    Sound::new(knock_samples())
}

/// The `T` pulse: one knock at the start of each second, silence after it, so a looping pulse
/// leaves gaps in which the echoes off far walls can be heard on their own.
fn pulse() -> Sound {
    let mut samples = knock_samples();
    samples.resize(vxe_audio::RATE as usize, 0.0);
    Sound::new(samples)
}

/// The knock's samples.
fn knock_samples() -> Vec<f32> {
    let mut state = KNOCK_SEED;
    let len = (KNOCK_SECONDS * vxe_audio::RATE as f32) as usize;
    let samples: Vec<f32> = (0..len)
        .map(|n| {
            state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let noise = (mix64(state) >> 40) as f32 / (1u64 << 23) as f32 - 1.0;
            let t = n as f32 / vxe_audio::RATE as f32;
            0.8 * noise * (-t / KNOCK_DECAY_SECONDS).exp()
        })
        .collect();
    samples
}

/// The cave's material table with [`CAVE_ACOUSTIC`] on every row but air, its absorption
/// replaced by `absorption` (`--absorption`).
fn acoustic_table(absorption: f32) -> MaterialTable {
    let mut t = MaterialTable::new();
    for row in cave_materials().rows().iter().skip(1) {
        t.push(vxe_world::Material {
            acoustic: [absorption, CAVE_ACOUSTIC[1], CAVE_ACOUSTIC[2]],
            ..row.clone()
        });
    }
    t
}

/// The room line: `room: V <m3> S <m2> mfp <m> alpha <a> T60 <s> energy <dB>`.
fn room_line(r: &RoomEstimate) -> String {
    format!(
        "room: V {:.1} m3 S {:.1} m2 mfp {:.3} m alpha {:.4} T60 {:.3} s energy {:+.1} dB",
        r.volume_m3,
        r.surface_m2,
        r.mean_free_path_m,
        r.absorption,
        r.t60_s,
        10.0 * r.energy.max(1e-12).log10()
    )
}

/// `--audio-debug`'s line for each room sent to the mixer:
/// `room sent: T60 <s> mfp <m> energy <dB>`.
fn room_sent_line(r: &RoomTargets) -> String {
    format!(
        "room sent: T60 {:.3} s mfp {:.3} m energy {:+.1} dB",
        r.t60_s,
        r.mean_free_path_m,
        10.0 * r.energy.max(1e-12).log10()
    )
}

/// The voice the microphone plays as. The output numbers its voices up from 0, so this one
/// is never handed out in a session.
const MIC_VOICE: VoiceId = VoiceId(u32::MAX);

/// `--mic`: the input stream and whether its period has been printed.
struct Mic {
    input: vxe_audio::Input,
    period_printed: bool,
    stream_errors: u64,
}

/// `--room-line` without an output device: the room fan on a block clock that never
/// advances, its commands dropped.
struct RoomProbe {
    acoustics: Acoustics,
    table: MaterialTable,
}

impl RoomProbe {
    fn new(absorption: f32) -> Self {
        Self {
            acoustics: Acoustics::new(BlockClock::default()),
            table: acoustic_table(absorption),
        }
    }
}

/// One second of a `J` hum at `hz`.
fn hum(hz: u32) -> Sound {
    let rate = vxe_audio::RATE as f32;
    Sound::new(
        (0..vxe_audio::RATE)
            .map(|n| HUM_AMPLITUDE * (std::f32::consts::TAU * hz as f32 * n as f32 / rate).sin())
            .collect::<Vec<_>>(),
    )
}

/// The audio of `--audio`: the output stream, the sounds the keys play and the acoustics
/// probing them.
struct Audio {
    output: Output,
    knock: Sound,
    /// The looping knock `T` plays.
    pulse: Sound,
    /// One hum per [`HUM_HZ`] note.
    hums: Vec<Sound>,
    /// The note the next `J` plays, an index into [`HUM_HZ`].
    next_hum: usize,
    /// Where the last left click dug, in voxel units.
    dig_point: Option<[f32; 3]>,
    acoustics: Acoustics,
    /// The cave's rows with [`CAVE_ACOUSTIC`] on every solid one.
    table: MaterialTable,
    /// The voices playing and where, in voxel units.
    voices: Vec<(VoiceId, [f32; 3])>,
    /// Stream errors (underruns among them) already reported.
    stream_errors: u64,
    /// `--audio-debug`: print the callback's readings each second.
    debug: bool,
    /// When the callback's readings were last taken.
    stats_at: Instant,
    /// The longest callback (the mixer's render of one device buffer, timed inside the
    /// callback) over the last whole second, in milliseconds.
    callback_max_ms: f32,
    /// Wall time of the last `Acoustics::update`, in milliseconds.
    update_ms: f64,
    /// `--room-line`: print the room line whenever a fan completes.
    room_line: bool,
    mic: Option<Mic>,
    /// Fans discarded in a row so far (the listener inside a solid), for `--audio-debug`.
    discarded_run: u64,
}

/// An environment variable naming an HRIR pack to use instead of the shipped one, for
/// listening comparisons.
const HRIR_ENV: &str = "FLYTHROUGH_HRIR";

/// Seconds of output `--audio-debug` keeps, and where it writes them.
const AUDIO_DEBUG_SECONDS: f32 = 60.0;
const AUDIO_DEBUG_WAV: &str = "flythrough-audio-debug.wav";

impl Drop for Audio {
    fn drop(&mut self) {
        if !self.debug {
            return;
        }
        let samples = self.output.take_recording();
        match std::fs::write(AUDIO_DEBUG_WAV, wav_f32_stereo(&samples)) {
            Ok(()) => println!(
                "audio-debug: wrote {:.1} s of the device's output to {AUDIO_DEBUG_WAV}",
                samples.len() as f32 / 2.0 / vxe_audio::RATE as f32
            ),
            Err(e) => println!("audio-debug: {AUDIO_DEBUG_WAV}: {e}"),
        }
    }
}

/// A 32-bit float stereo WAV file of interleaved `samples` at the mixer's rate.
fn wav_f32_stereo(samples: &[f32]) -> Vec<u8> {
    let data = (samples.len() * 4) as u32;
    let rate = vxe_audio::RATE;
    let mut b = Vec::with_capacity(44 + data as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 8).to_le_bytes());
    b.extend_from_slice(&8u16.to_le_bytes());
    b.extend_from_slice(&32u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    for &x in samples {
        b.extend_from_slice(&x.to_le_bytes());
    }
    b
}

impl Audio {
    /// Plays the sound `name` names (`knock`, `hum` or `pulse`) at the dig point with `params`.
    fn play(&mut self, name: &str, params: impl Fn([f32; 3]) -> PlayParams) {
        let Some(p) = self.dig_point else {
            println!("{name}: no dig point yet; left-click first");
            return;
        };
        let sound = if name == "hum" {
            let i = self.next_hum;
            self.next_hum = (i + 1) % self.hums.len();
            println!(
                "hum: note {} of {}, {} Hz, at [{:.1}, {:.1}, {:.1}]",
                i + 1,
                self.hums.len(),
                HUM_HZ[i],
                p[0],
                p[1],
                p[2]
            );
            &self.hums[i]
        } else if name == "pulse" {
            &self.pulse
        } else {
            &self.knock
        };
        match self.output.play(sound, params(p)) {
            Ok(id) => self.voices.push((id, p)),
            Err(e) => println!("{name}: {e}"),
        }
    }

    /// Opens the default device; prints why when it cannot. `debug` meters and records the
    /// callback; `absorption` is the cave row's; `mic` also opens the default input device and
    /// plays it at the listener with its direct path off; `room_line` prints each room.
    fn open(debug: bool, absorption: f32, mic: bool, room_line: bool) -> Option<Self> {
        let t = now();
        let hrtf = match std::env::var_os(HRIR_ENV) {
            Some(path) => match std::fs::read(&path)
                .map_err(|e| e.to_string())
                .and_then(|b| Hrtf::load(&b).map_err(|e| e.to_string()))
            {
                Ok(h) => {
                    println!(
                        "audio: HRIR pack from {HRIR_ENV}={}",
                        path.to_string_lossy()
                    );
                    std::sync::Arc::new(h)
                }
                Err(e) => {
                    println!(
                        "audio: {HRIR_ENV}={}: {e}; using the shipped pack",
                        path.to_string_lossy()
                    );
                    Hrtf::sadie2_d2()
                }
            },
            None => Hrtf::sadie2_d2(),
        };
        println!(
            "audio: HRIR pack loaded in {:.1} ms ({} directions, {} taps)",
            ms_since(t),
            hrtf.len(),
            hrtf.taps()
        );
        let mixer = Mixer::new(hrtf, METRES_PER_VOXEL);
        let acoustics = Acoustics::new(mixer.clock());
        match Output::open(mixer) {
            Ok(output) => {
                println!("audio: {}", output.device());
                if debug {
                    output.record(AUDIO_DEBUG_SECONDS);
                }
                let mic = if mic { open_mic(&output) } else { None };
                let mut voices = Vec::new();
                if mic.is_some() {
                    voices.push((MIC_VOICE, [0.0; 3]));
                }
                Some(Self {
                    output,
                    knock: knock(),
                    pulse: pulse(),
                    hums: HUM_HZ.iter().map(|&hz| hum(hz)).collect(),
                    next_hum: 0,
                    dig_point: None,
                    acoustics,
                    table: acoustic_table(absorption),
                    voices,
                    stream_errors: 0,
                    debug,
                    stats_at: now(),
                    callback_max_ms: 0.0,
                    update_ms: 0.0,
                    room_line,
                    mic,
                    discarded_run: 0,
                })
            }
            Err(e) => {
                println!("audio: off, {e}");
                None
            }
        }
    }
}

/// Opens the default input device into a stream played at the listener with its direct path
/// off; prints the device, or why it could not.
fn open_mic(output: &Output) -> Option<Mic> {
    println!("mic: wear headphones: from speakers the microphone hears the tail and feeds it back");
    let (writer, reader) = vxe_audio::Stream::new();
    match vxe_audio::Input::open(writer) {
        Ok(input) => {
            println!("mic: {}", input.device());
            let params = PlayParams {
                direct: false,
                ..PlayParams::at([0.0; 3])
            };
            if let Err(e) = output.send(Command::PlayStream(MIC_VOICE, reader, params)) {
                println!("mic: off, {e}");
                return None;
            }
            Some(Mic {
                input,
                period_printed: false,
                stream_errors: 0,
            })
        }
        Err(e) => {
            println!("mic: off, {e}");
            None
        }
    }
}

fn voxel_of(p: Vec3) -> VoxelCoord {
    VoxelCoord(p.to_array().map(|v| v.floor() as i32))
}

/// Chunk offsets within squared distance `r2`.
fn ball(r2: i32) -> Vec<[i32; 3]> {
    let r = (0..).find(|r| r * r > r2).unwrap_or(1);
    let mut v = Vec::new();
    for x in -r..=r {
        for y in -r..=r {
            for z in -r..=r {
                if x * x + y * y + z * z <= r2 {
                    v.push([x, y, z]);
                }
            }
        }
    }
    v
}

struct Gfx {
    mirror: GpuWorld,
    marcher: Marcher,
    presenter: Presenter,
    timer: Option<MarchTimer>,
    /// Times the attribute pass, with attribute pages and `TIMESTAMP_QUERY`.
    attr_timer: Option<MarchTimer>,
    hud: HudOverlay,
    /// Times the HUD pass, with `TIMESTAMP_QUERY`.
    hud_timer: Option<PassTimer>,
}

impl Gfx {
    /// Runs the attribute pass over what changed, alone, and waits for it: the bricks in its
    /// work list and its GPU time when timed.
    fn timed_attrs(&mut self, gpu: &Gpu) -> (usize, Option<f64>) {
        let mut enc = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let timer = self.attr_timer.as_ref();
        let n = self
            .mirror
            .update_attrs_timed(&mut enc, timer.map(MarchTimer::writes));
        if let Some(t) = timer.filter(|_| n > 0) {
            t.resolve(&mut enc);
        }
        gpu.queue.submit([enc.finish()]);
        let ms = timer.filter(|_| n > 0).map(|t| t.read_ms(&gpu.device));
        (n, ms)
    }
}

/// `attrib: 383608 bricks in 12.345 ms GPU (initial fill)`.
fn attr_line(what: &str, (n, ms): (usize, Option<f64>)) -> String {
    let ms = ms.map_or("n/a (no TIMESTAMP_QUERY)".to_owned(), |ms| {
        format!("{ms:.3} ms")
    });
    format!("attrib: {n} bricks in {ms} GPU ({what})")
}

#[derive(Default)]
struct Samples {
    cpu_ms: Vec<f64>,
    gpu_ms: Vec<f64>,
    /// The HUD pass's GPU time, on frames that draw it.
    hud_ms: Vec<f64>,
}

/// The stats HUD's state. The frame-time ring and the last sync and edit are kept on every
/// frame; the world's counts are sampled at the cadence, and the rows formatted, only while
/// the HUD is shown.
struct Hud {
    visible: bool,
    frames: FrameStats,
    cadence: Cadence,
    world: WorldSample,
    /// The process's RSS, read with the world sample: reading `/proc/self/statm` costs a
    /// tenth of a millisecond or more in this process, too much for every frame.
    rss_bytes: Option<u64>,
    /// The last mirror sync and its render-thread time in milliseconds.
    sync: (SyncStats, f64),
    /// The rows' values, refreshed in place on every shown frame. Its `last_edit` is the only
    /// copy of the last edit, so a frame clones nothing.
    stats: HudStats,
    /// Last frame's pass readings, shown on the next frame's rows.
    march_ms: Option<f32>,
    hud_ms: Option<f32>,
    cpu_ms: f32,
    /// The last frame's steps view, read back while that view and the HUD are on.
    steps: Option<hud::StepsSample>,
    format_us: u32,
    page_bytes: u64,
    /// Reused buffers: the rows' text, their colours, and the strip's samples.
    text: String,
    colours: Vec<HudColour>,
    strip: Vec<f32>,
}

impl Hud {
    fn new(visible: bool) -> Self {
        Self {
            visible,
            frames: FrameStats::new(),
            cadence: Cadence::new(HUD_STATS_EVERY),
            world: WorldSample::default(),
            rss_bytes: None,
            sync: (SyncStats::default(), 0.0),
            stats: HudStats::default(),
            march_ms: None,
            hud_ms: None,
            cpu_ms: 0.0,
            steps: None,
            format_us: 0,
            page_bytes: hud::page_bytes(),
            text: String::new(),
            colours: Vec::new(),
            strip: Vec::with_capacity(hud::RING),
        }
    }

    /// Formats `stats` into the reused row and colour buffers, timing it, and copies the
    /// frame window into the reused strip.
    fn format_rows(&mut self) {
        let t = now();
        self.stats.write_rows(&mut self.text, &mut self.colours);
        self.format_us = ((now() - t).as_secs_f64() * 1e6) as u32;
        self.strip.clear();
        self.strip.extend(self.frames.samples().map(|s| s * 1000.0));
    }
}

/// Mirror pressure (`GpuWorld::pressure`, the fuller of pool slots and records) above which the full radius falls by a
/// chunk once it has held for `PRESSURE_HIGH_FRAMES` frames, and below which it rises by one
/// once it has held for `PRESSURE_LOW_FRAMES` (ADR-0013).
const PRESSURE_HIGH: f32 = hud::PRESSURE_HIGH;
const PRESSURE_HIGH_FRAMES: u32 = 30;
const PRESSURE_LOW: f32 = flythrough::PRESSURE_LOW;
const PRESSURE_LOW_FRAMES: u32 = 300;
/// The full radius the pressure rule never lowers below, in chunks.
const R_FULL_FLOOR: i32 = 6;

/// The two terms of `GpuWorld::pressure`: pool-slot occupancy (two attribute slots with each
/// brick in an attribute page) and record occupancy.
fn pressure_terms(m: &GpuWorld) -> [f64; 2] {
    let slots = f64::from(m.live_slots()) + 2.0 * f64::from(m.live_attr_slots());
    [
        slots / f64::from(m.reserved_slots().max(1)),
        m.mirrored_chunks() as f64 / f64::from(m.reserved_records().max(1)),
    ]
}

/// The fly-through's answer to mirror pressure: the full radius between a floor and the
/// configured radius, a chunk down after `PRESSURE_HIGH_FRAMES` frames above
/// `PRESSURE_HIGH`, a chunk up after `PRESSURE_LOW_FRAMES` frames below `PRESSURE_LOW`. A
/// change restarts both counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RadiusRule {
    r_full: i32,
    max: i32,
    above: u32,
    below: u32,
}

impl RadiusRule {
    fn new(max: i32) -> Self {
        Self {
            r_full: max,
            max,
            above: 0,
            below: 0,
        }
    }

    /// The floor: `R_FULL_FLOOR`, or the configured radius when that is smaller.
    fn floor(&self) -> i32 {
        R_FULL_FLOOR.min(self.max)
    }

    /// Takes one frame's pressure; returns the new full radius when it changes.
    fn step(&mut self, pressure: f32) -> Option<i32> {
        self.above = if pressure > PRESSURE_HIGH {
            self.above + 1
        } else {
            0
        };
        self.below = if pressure < PRESSURE_LOW {
            self.below + 1
        } else {
            0
        };
        let r = if self.above >= PRESSURE_HIGH_FRAMES && self.r_full > self.floor() {
            self.r_full - 1
        } else if self.below >= PRESSURE_LOW_FRAMES && self.r_full < self.max {
            self.r_full + 1
        } else {
            return None;
        };
        self.r_full = r;
        self.above = 0;
        self.below = 0;
        Some(r)
    }
}

/// The mirror's deferrals (ADR-0013): a sync that defers a chunk, or an empty sync that
/// uploads none, extends a deferral streak; a sync that defers nothing ends it. Keeps the
/// deferred-chunk total (a chunk deferred in five syncs counts five), the syncs with a
/// deferral and the empty ones among them, the lowest and highest pressure and terms at a
/// deferral, and the current and longest streak in syncs, an open streak included.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Deferrals {
    chunks: u64,
    syncs: u64,
    empty: u64,
    /// Lowest and highest: pressure, slot term, record term.
    low: [f64; 3],
    high: [f64; 3],
    streak: u64,
    longest: u64,
}

impl Deferrals {
    /// Notes one sync's `deferred` chunks, `empty` when it uploaded none, with the mirror's
    /// pressure and terms `v`. Returns the length of a streak this sync ends.
    fn note(&mut self, deferred: u64, empty: bool, v: [f64; 3]) -> Option<u64> {
        if deferred == 0 && !empty {
            let ended = (self.streak > 0).then_some(self.streak);
            self.streak = 0;
            return ended;
        }
        if self.syncs == 0 {
            (self.low, self.high) = (v, v);
        }
        self.low = [0, 1, 2].map(|k| self.low[k].min(v[k]));
        self.high = [0, 1, 2].map(|k| self.high[k].max(v[k]));
        self.chunks += deferred;
        self.syncs += 1;
        self.empty += u64::from(empty);
        self.streak += 1;
        self.longest = self.longest.max(self.streak);
        None
    }
}

/// The streaming state of the default mode.
struct Stream {
    streamer: Streamer,
    /// The full radius under mirror pressure, and the changes it made.
    rule: RadiusRule,
    radius_changes: u32,
    /// Chunks the mirror deferred for space, and the streaks of syncs that deferred.
    deferrals: Deferrals,
    /// Milliseconds since start at each `update` call, `tick_ms[t - 1]` for tick `t`.
    tick_ms: Vec<f64>,
    /// Chunks loaded full and not yet mirrored, with the tick that first wanted them.
    awaiting: HashMap<ChunkCoord, u64>,
    /// Wanted-to-mirrored latency of each chunk mirrored full, in milliseconds.
    latency_ms: Vec<f64>,
    /// Render-thread CPU of `update + poll + sync`, per frame.
    cpu_ms: Vec<f64>,
    full_residency: bool,
    workers: usize,
}

impl Stream {
    /// Records every chunk `loaded` full with its wanted tick, and moves the ones the mirror
    /// now holds full into `latency_ms`.
    fn track(&mut self, ev: &vxe_world::StreamEvents, mirror: &GpuWorld) {
        let tick = self.streamer.stats().ticks;
        let full = ev.loaded.iter().filter(|(_, t)| *t == Tier::Full);
        for (&(c, _), &lat) in full.zip(&ev.latency_ticks) {
            self.awaiting.insert(c, tick.saturating_sub(u64::from(lat)));
        }
        let now_ms = self.tick_ms.last().copied().unwrap_or(0.0);
        let tick_ms = &self.tick_ms;
        let latency = &mut self.latency_ms;
        self.awaiting.retain(|&c, &mut wanted| {
            if mirror.is_mirrored(c) && !mirror.is_lod(c) {
                let at = tick_ms
                    .get(wanted.max(1) as usize - 1)
                    .copied()
                    .unwrap_or(now_ms);
                latency.push(now_ms - at);
                false
            } else {
                true
            }
        });
    }
}

/// The autopilot's legs (`--autopilot`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Leg {
    /// Waiting for full residency, then digging.
    Settle,
    Out,
    Back,
    /// Waiting for the dug chunks to be full and mirrored again.
    Return,
}

/// `--autopilot`: waits for full residency, carves 1 m at the first rock within 30 m along +x
/// and records the touched chunks' hash, flies +x at 10 m/s for 30 s (beyond the eviction
/// radius) and back, waits for the touched chunks to be full again, and compares the hash.
/// Over the flight it counts unready-near frames (a chunk wanted full within `r_full - 2`
/// chunks of the camera's not yet resident and mirrored full), and samples streaming CPU,
/// latency, memory and march-pass GPU time.
struct Autopilot {
    leg: Leg,
    leg_start: Instant,
    origin: Vec3,
    touched: Vec<ChunkCoord>,
    hash: u64,
    near: Vec<[i32; 3]>,
    flight_frames: u64,
    unready_frames: u64,
    gpu_ms: Vec<f64>,
    cpu_ms: Vec<f64>,
    /// The attribute pass over the flight, with attribute pages.
    attr: AttrSamples,
    latency_from: usize,
    peak_mirror: u64,
    peak_allocated: u64,
    /// The highest mirror pressure over the flight, and of each of its terms: pool-slot
    /// and record occupancy.
    peak_pressure: f32,
    peak_terms: [f64; 2],
    peak_resident: u64,
    peak_persisted: u64,
    generated_at_takeoff: u64,
}

/// Per-frame readings of the attribute pass over the autopilot's flight: the work list's
/// length and the render thread's CPU time in `update_attrs` every frame, and the pass's GPU
/// time on the frames whose list is not empty.
#[derive(Default)]
struct AttrSamples {
    items: Vec<f64>,
    cpu_ms: Vec<f64>,
    gpu_ms: Vec<f64>,
}

impl AttrSamples {
    /// The report's lines: p50, p99 and max of each reading.
    fn lines(&mut self) -> Vec<String> {
        let stats = |v: &mut Vec<f64>| {
            [50.0, 99.0, 100.0].map(|p| if v.is_empty() { 0.0 } else { percentile(v, p) })
        };
        let frames = self.items.len();
        let busy = self.items.iter().filter(|&&n| n > 0.0).count();
        let [i50, i99, imax] = stats(&mut self.items);
        let [c50, c99, cmax] = stats(&mut self.cpu_ms);
        let mut out = vec![
            format!(
                "autopilot: attribute work list: p50 {i50:.0}, p99 {i99:.0}, max {imax:.0} bricks over {frames} frames ({busy} with work)"
            ),
            format!(
                "autopilot: attribute pass CPU (update_attrs, render thread): p50 {c50:.3} ms, p99 {c99:.3} ms, max {cmax:.3} ms over {frames} frames"
            ),
        ];
        if self.gpu_ms.is_empty() {
            out.push("autopilot: attribute pass GPU n/a (no TIMESTAMP_QUERY or no work)".into());
        } else {
            let n = self.gpu_ms.len();
            let [g50, g99, gmax] = stats(&mut self.gpu_ms);
            out.push(format!(
                "autopilot: attribute pass GPU: p50 {g50:.3} ms, p99 {g99:.3} ms, max {gmax:.3} ms over the {n} frames with work"
            ));
        }
        out
    }
}

/// The bounded level's coarse grid and walkable map, updated from every edit the client is
/// told of, and what the last dig's support query found.
struct Coarse {
    grid: CoarseGrid,
    map: WalkableMap,
    /// Cells the last edit changed, and the grid plus walkable update time in microseconds.
    last: Option<(usize, f64)>,
    /// Regions the logic reported after the last applied dig.
    query: Option<usize>,
}

impl Coarse {
    fn build(world: &World) -> Self {
        let t = now();
        let grid = CoarseGrid::build(world);
        let map = WalkableMap::build(&grid, WALKABLE);
        println!(
            "coarse: {} chunks, {} bytes, {} walkable cells, built in {:.0} ms",
            grid.len(),
            grid.bytes(),
            map.count(),
            ms_since(t)
        );
        Self {
            grid,
            map,
            last: None,
            query: None,
        }
    }

    /// Updates the grid and the walkable map from `r`; returns the grid's changed cells.
    fn update(&mut self, world: &World, r: &EditResult) -> Vec<CellChange> {
        let t = now();
        let changes = self.grid.update(world, r);
        self.map.update(&self.grid, &changes);
        self.last = Some((changes.len(), ms_since(t) * 1e3));
        changes
    }

    /// The `K` line.
    fn line(&self) -> String {
        match self.last {
            Some((m, us)) => format!(
                "coarse: {} walkable cells, last edit changed {m} cells in {us:.0} us",
                self.map.count()
            ),
            None => format!("coarse: {} walkable cells, no edit yet", self.map.count()),
        }
    }

    /// The `F1` collapse line.
    fn query_line(&self) -> String {
        match self.query {
            Some(n) => format!("collapse: last dig's query found {n} regions"),
            None => "collapse: no query yet".to_owned(),
        }
    }
}

/// The line for a logic's `region` line: `unsupported: 9 cells, {rock: 4608}`; `None` for any
/// other line.
fn unsupported_line(table: &MaterialTable, line: &str) -> Option<String> {
    let (cells, counts) = parse_region_line(line)?;
    Some(format!(
        "unsupported: {cells} cells, {{{}}}",
        counts_line(table, &counts)
    ))
}

/// The line for an applied removal: `removed: {rock: 4608}, 1152 rubble`.
fn removed_line(table: &MaterialTable, r: &vxe_world::RemoveResult) -> String {
    format!(
        "removed: {{{}}}, {} rubble",
        counts_line(table, &r.removed),
        r.placed
    )
}

/// `rock: 4608, ore: 12`: per-material counts by name.
fn counts_line(table: &MaterialTable, counts: &[(MaterialId, u32)]) -> String {
    counts
        .iter()
        .map(|&(m, n)| format!("{}: {n}", material_name(table, m)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The simulation split around the world: the server running the fly-through's logic
/// (`--logic`) and the shared client every edit goes through.
struct Net {
    server: Server,
    client: Client,
    /// Whether the logic's stop has been printed: a hosted logic whose restarts are spent
    /// reports [`LogicError::Stopped`] on every later tick, printed only the first time.
    stop_printed: bool,
}

impl Net {
    /// The server on `world` with `logic`, the vault registered on it and on the client (the
    /// prefab `P` stamps, ADR-0014) and the collapse's [`SUPPORT`] set, and the shared client.
    fn new(world: &Arc<World>, logic: Box<dyn Logic>, seed: u64) -> Self {
        let vault = Arc::new(vault());
        let mut prefabs = vxe_world::PrefabRegistry::new();
        prefabs
            .register(vault.clone())
            .expect("an empty registry takes the vault");
        let mut server = Server::with_prefabs(world.clone(), prefabs, logic, seed);
        server.set_support(SUPPORT, &[]);
        let (near, far) = MemChannel::pair();
        server.attach(far);
        let mut client = Client::shared(world.clone(), near);
        client
            .register_prefab(vault)
            .expect("an empty registry takes the vault");
        let mut net = Self {
            server,
            client,
            stop_printed: false,
        };
        if let Some(e) = net.server.init_error().cloned()
            && let Some(line) = net.logic_failure(&e)
        {
            println!("{line}");
        }
        net
    }

    /// The line to print for the logic's error `e`: every failure and every restart, and
    /// [`LogicError::Stopped`] the first time only.
    fn logic_failure(&mut self, e: &LogicError) -> Option<String> {
        match e {
            LogicError::Stopped { .. } => (!std::mem::replace(&mut self.stop_printed, true))
                .then(|| format!("logic: {e}; the session continues without it")),
            _ => Some(format!("logic: {e}")),
        }
    }
}

/// Bench carves the client has seen this tick, summed until a bench's [`BENCH_CARVES`] are in.
#[derive(Default)]
struct BenchSum {
    carves: usize,
    refused: usize,
    counts: BTreeMap<MaterialId, u32>,
    touched: BTreeSet<ChunkCoord>,
}

/// The running fly-through, once the world exists.
struct Run {
    start: Instant,
    /// The server's world, which the shared client, the mirror and the queries read.
    world: Arc<World>,
    /// `None` only while dropping.
    net: Option<Net>,
    /// The voxel the camera starts in; the mirror uploads nearest it first.
    spawn: VoxelCoord,
    args: Args,
    fly: Fly,
    light: LightParams,
    look: LookParams,
    dig_radius: i64,
    /// Scroll banked toward the next dig-radius step.
    scroll: ScrollSteps,
    bench_runs: u64,
    frame: u32,
    gfx: Option<Gfx>,
    samples: Option<Samples>,
    stream: Option<Stream>,
    pilot: Option<Autopilot>,
    /// Walk mode's body, `None` in fly mode.
    walk: Option<Walk>,
    coarse: Option<Coarse>,
    hud: Hud,
    audio: Option<Audio>,
    /// `--room-line` without an output device.
    room_probe: Option<RoomProbe>,
}

impl Run {
    /// Prints the edit's line and keeps it for the HUD.
    fn record_edit(&mut self, verb: &'static str, r: &EditResult, ms: f64) {
        println!("{}", self.note_edit(verb, r, ms));
    }

    /// Keeps the edit for the HUD's last-edit row without printing it, for an edit whose
    /// printed line has another form (the collapse's drops and removals); returns the record.
    fn note_edit(&mut self, verb: &'static str, r: &EditResult, ms: f64) -> &EditLine {
        self.hud
            .stats
            .last_edit
            .insert(edit_record(verb, r, self.world.materials(), ms))
    }

    /// Samples the world's counts for the HUD: `World::stats`, and `Streamer::stats` for the
    /// LOD-only chunks while streaming, timed together; then the process's RSS.
    fn sample_world(&mut self) {
        let t = now();
        let s = self.world.stats();
        let lod = self
            .stream
            .as_ref()
            .map(|st| st.streamer.stats().resident_lod as u32);
        // The world keeps no brick-array pool: no spare arrays to count.
        self.hud.world = WorldSample::from_stats(&s, lod, 0, ms_since(t) as f32);
        self.hud.rss_bytes = hud::rss_bytes(self.hud.page_bytes);
    }

    /// Refreshes the HUD's values in place for this frame, from the O(1) sources and the
    /// last world sample; the last edit is already there.
    fn refresh_hud_stats(&mut self) {
        let (mirror_bytes, pool_live, pool_free) = self.gfx.as_ref().map_or((0, 0, 0), |g| {
            (
                g.mirror.mirror_bytes(),
                g.mirror.live_slots(),
                g.mirror.free_slots().len() as u32,
            )
        });
        let pressure = self.gfx.as_ref().map(|g| hud::Pressure {
            pressure: g.mirror.pressure(),
            budget_bytes: g.mirror.allocated_bytes(),
            r_full: self.stream.as_ref().map(|s| [s.rule.r_full, s.rule.max]),
        });
        let pos = self.fly.camera.position;
        let mode = if self.walk.is_some() {
            hud::Mode::Walk
        } else {
            hud::Mode::Fly
        };
        let h = &mut self.hud;
        let (sync, sync_ms) = h.sync;
        let s = &mut h.stats;
        s.fps = h.frames.fps();
        s.frame_ms = h.frames.frame_ms();
        s.cpu_ms = h.cpu_ms;
        s.march_ms = h.march_ms;
        s.hud_ms = h.hud_ms;
        s.upload_bytes = sync.bytes;
        s.upload_ms = sync_ms as f32;
        s.upload_chunks = sync.chunks as u32;
        s.mirror_bytes = mirror_bytes;
        s.pool_live = pool_live;
        s.pool_free = pool_free;
        s.pressure = pressure;
        s.world = WorldSample {
            age_frames: h.cadence.age(),
            ..h.world
        };
        s.rss_bytes = h.rss_bytes;
        s.steps = h.steps;
        s.camera = pos.to_array();
        s.chunk = voxel_of(pos).chunk().0;
        s.mode = mode;
        s.format_us = h.format_us;
    }

    /// Formats this frame's rows and strip into the overlay, timing the formatting. Allocates
    /// nothing once the buffers have grown to the rows' size.
    fn update_hud(&mut self, gpu: &Gpu) {
        self.refresh_hud_stats();
        let h = &mut self.hud;
        h.format_rows();
        let Some(gfx) = self.gfx.as_mut() else { return };
        gfx.hud.set_lines(&gpu.queue, &h.text, &h.colours);
        gfx.hud.set_samples(&gpu.queue, &h.strip, hud::CEILING_MS);
    }

    /// Keeps the coarse grid and the walkable map current with `r` (nothing while
    /// streaming).
    fn edited(&mut self, r: &EditResult) {
        if let Some(c) = self.coarse.as_mut() {
            c.update(&self.world, r);
        }
    }

    /// `G`: walk mode with the body under the camera, or back to flight.
    fn toggle_walk(&mut self) {
        if self.walk.take().is_some() {
            println!("fly mode");
            return;
        }
        if let Err(e) = walk_allowed(self.stream.is_some()) {
            println!("walk: {e}; still flying");
            return;
        }
        match Walk::at_camera(&self.world, &self.fly.camera) {
            Some(w) => {
                self.walk = Some(w);
                println!("walk mode");
            }
            None => println!("walk: no room for the body here; still flying"),
        }
    }

    fn coarse_line(&self) -> String {
        match &self.coarse {
            None => "coarse: not kept while streaming".to_owned(),
            Some(c) => c.line(),
        }
    }

    /// Rock's detail pair when the table carries it (every look but `--look plain`).
    fn rock_detail(&self) -> Option<[f32; 2]> {
        self.args.rock_detail()
    }

    fn camera_ray(&self) -> (Fixed3, [i64; 3]) {
        let c = &self.fly.camera;
        let origin = c
            .position
            .to_array()
            .map(|v| (v * FIXED_ONE as f32).floor() as i64);
        let dir = c
            .basis()
            .0
            .to_array()
            .map(|v| (v * FIXED_ONE as f32).round() as i64);
        (Fixed3(origin), dir)
    }

    fn send(&self, input: DigInput) {
        if let Some(net) = &self.net {
            net.client.send_input(input.encode());
        }
    }

    /// Left click: a dig along the camera ray.
    fn dig(&mut self) {
        let (origin, dir) = self.camera_ray();
        if let Some(audio) = self.audio.as_mut() {
            let t = self
                .world
                .raycast(origin, dir, REACH)
                .ok()
                .flatten()
                .map_or(REACH, |h| h.t);
            audio.dig_point = Some(
                [0, 1, 2].map(|k| (origin.0[k] + dir[k] * t / FIXED_ONE) as f32 / FIXED_ONE as f32),
            );
        }
        self.send(DigInput::Dig {
            origin,
            dir,
            radius: self.dig_radius,
        });
    }

    /// `P`: a stamp input, the vault (registered on the server by its content id) at the
    /// first solid voxel the camera ray enters.
    fn stamp(&mut self) {
        let (origin, dir) = self.camera_ray();
        self.send(DigInput::Stamp {
            origin,
            dir,
            prefab: vault().id(),
        });
    }

    /// Right click: a fill where the camera ray hits.
    fn fill(&mut self) {
        let (origin, dir) = self.camera_ray();
        self.send(DigInput::Fill {
            origin,
            dir,
            radius: FILL_RADIUS,
            material: ROCK,
        });
    }

    /// `B`: a bench input, its carves seeded from `--seed` and the count of benches so far,
    /// centred on the camera's voxel.
    fn bench(&mut self) {
        let origin = self
            .fly
            .camera
            .position
            .to_array()
            .map(|v| v.floor() as i64);
        let seed = self.args.seed ^ self.bench_runs.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        self.bench_runs += 1;
        self.send(DigInput::Bench { origin, seed });
    }

    /// The frame's server tick, timed, then the client's events: one printed line per dig,
    /// fill, stamp, removal and bench, the logic's other lines (the unsupported regions with
    /// material names), the coarse grid updated from every applied result. A tick that
    /// finished a bench syncs the mirror at once and prints the bench line.
    fn tick(&mut self, gpu: &Gpu) {
        let Some(net) = self.net.as_mut() else { return };
        let t = now();
        let report = net.server.tick();
        let ms = ms_since(t);
        for e in [report.restart, report.logic_error].into_iter().flatten() {
            if let Some(line) = net.logic_failure(&e) {
                println!("{line}");
            }
        }
        let events = net.client.poll();

        let mut ops = std::collections::VecDeque::new();
        let mut bench = BenchSum::default();
        let mut benches = (0, 0);
        for event in events {
            let Event::Logic { bytes, .. } = event else {
                ops.push_back(event);
                continue;
            };
            for line in String::from_utf8_lossy(&bytes).lines() {
                let Some(verb) = line.strip_prefix("op ").and_then(op_verb) else {
                    match unsupported_line(self.world.materials(), line) {
                        Some(l) => {
                            if let Some(c) = self.coarse.as_mut() {
                                c.query = Some(c.query.map_or(1, |n| n + 1));
                            }
                            println!("{l}");
                        }
                        None => println!("{line}"),
                    }
                    continue;
                };
                let result = match ops.pop_front() {
                    Some(Event::Applied { result, .. }) => Ok(result),
                    Some(Event::Refused { error, .. }) => Err(error),
                    _ => continue,
                };
                if let Ok(r) = &result {
                    self.edited(&r.edit_result());
                }
                if verb == DIG_VERB
                    && result.is_ok()
                    && let Some(c) = self.coarse.as_mut()
                {
                    // The region lines that follow this dig's are what its query found.
                    c.query = Some(0);
                }
                if verb != BENCH_VERB {
                    match result {
                        // A drop happens every tick a region falls: kept for the HUD only.
                        Ok(r) if verb == DROP_VERB => {
                            if r.aabb().is_some() {
                                self.note_edit(verb, &r.edit_result(), ms);
                            }
                        }
                        Ok(OpResult::Remove(r)) => {
                            println!("{}", removed_line(self.world.materials(), &r));
                            self.note_edit(verb, &r.edit_result(), ms);
                        }
                        Ok(r) => self.record_edit(verb, &r.edit_result(), ms),
                        Err(e) => println!("{verb}: refused, {e}"),
                    }
                    continue;
                }
                let result = result.map(|r| r.edit_result());
                match result {
                    Ok(r) => {
                        for (m, n) in r.changed_from {
                            *bench.counts.entry(m).or_default() += n;
                        }
                        bench.touched.extend(r.touched);
                    }
                    Err(_) => bench.refused += 1,
                }
                bench.carves += 1;
                if bench.carves == BENCH_CARVES {
                    let done = std::mem::take(&mut bench);
                    let total = EditResult {
                        changed_from: done.counts.into_iter().collect(),
                        touched: done.touched.into_iter().collect(),
                        aabb: None,
                    };
                    self.record_edit("carved", &total, ms);
                    benches = (benches.0 + 1, benches.1 + done.refused);
                }
            }
        }
        if benches.0 > 0 {
            self.bench_sync(gpu, benches.0 * BENCH_CARVES, benches.1, ms);
        }
    }

    /// The mirror sync after a bench, timed, with the bench line; with attribute pages, the
    /// attribute pass over the carves timed alone.
    fn bench_sync(&mut self, gpu: &Gpu, carves: usize, refused: usize, edit_ms: f64) {
        let Some(gfx) = self.gfx.as_mut() else { return };
        let t = now();
        match gfx.mirror.sync(&self.world, &gpu.queue) {
            Ok(s) => println!(
                "bench: {carves} carves of radius 1 m within 10 m ({refused} refused): \
                 edits {edit_ms:.2} ms CPU; sync uploaded {} B ({} bricks, \
                 {} chunks, {} visited, {} writes) in {:.2} ms",
                s.bytes,
                s.bricks,
                s.chunks,
                s.chunks_visited,
                s.writes,
                ms_since(t)
            ),
            Err(e) => println!("bench: sync failed, {e}"),
        }
        if gfx.mirror.surface_attrs() {
            let what = format!("{carves} carves of radius 1 m");
            println!("{}", attr_line(&what, gfx.timed_attrs(gpu)));
        }
    }

    /// `--save`: the session's log file, and the world's hash.
    fn save(&self) {
        let (Some(path), Some(net)) = (&self.args.save, &self.net) else {
            return;
        };
        let file = LogFile {
            seed: self.args.seed,
            source_version: CaveSource::VERSION,
            extent: self.world.extent(),
            ops: net.server.ops().clone(),
            inputs: net.server.inputs().clone(),
            prefabs: net.server.prefabs().subset(net.server.ops().prefab_ids()),
        };
        println!("world hash 0x{:016x}", self.world.voxel_hash());
        let written = std::fs::File::create(path).and_then(|mut f| file.write_to(&mut f));
        match written {
            Ok(()) => println!(
                "saved {} inputs, {} ops to {}",
                file.inputs.len(),
                file.ops.len(),
                path.display()
            ),
            Err(e) => eprintln!("save to {} failed: {e}", path.display()),
        }
    }

    fn report(&mut self, samples: Samples) {
        let Samples {
            mut cpu_ms,
            mut gpu_ms,
            mut hud_ms,
        } = samples;
        let Some(gfx) = &self.gfx else { return };
        let [w, h] = [gfx.marcher.target().width(), gfx.marcher.target().height()];
        let gpu = if gpu_ms.is_empty() {
            "n/a (no TIMESTAMP_QUERY)".to_owned()
        } else {
            format!("{:.3} ms", median(&mut gpu_ms))
        };
        let hud = if hud_ms.is_empty() {
            String::new()
        } else {
            format!(", gpu hud {:.4} ms", median(&mut hud_ms))
        };
        println!(
            "frame ({w}x{h}, median of {}, {}): cpu {:.3} ms, gpu march {gpu}{hud}; full bricks {}, mirror {} B live, {} B allocated",
            cpu_ms.len(),
            look_line(&self.look, self.rock_detail()),
            median(&mut cpu_ms),
            self.world.stats().full_bricks,
            gfx.mirror.mirror_bytes(),
            gfx.mirror.allocated_bytes()
        );
        if let Some(audio) = &self.audio {
            println!(
                "audio: {} voices, callback max {:.3} ms over the last second, {} active taps, \
                 last acoustics update {:.3} ms",
                audio.voices.len(),
                audio.callback_max_ms,
                audio.acoustics.active_taps(),
                audio.update_ms
            );
            if let Some(mic) = &audio.mic {
                println!(
                    "mic: {}, {} frames dropped",
                    mic.input.device(),
                    mic.input.dropped_frames()
                );
            }
        }
        let room = self
            .audio
            .as_ref()
            .and_then(|a| a.acoustics.room())
            .or_else(|| self.room_probe.as_ref().and_then(|p| p.acoustics.room()));
        if let Some(r) = room {
            println!("{}", room_line(&r));
        }
        if let Some(c) = &self.coarse {
            println!("{}", c.query_line());
        }
    }

    /// RAM held by resident chunks: full chunks' voxels and LOD-only chunks' mips.
    fn resident_ram(&self) -> u64 {
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
}

impl Run {
    fn device_request(&mut self, adapter: &wgpu::Adapter) -> (wgpu::Features, wgpu::Limits) {
        let pool = if self.stream.is_some() {
            STREAM_POOL_BYTES
        } else {
            let full = self.world.stats().full_bricks as u64;
            level_pool_slots(full, self.args.look.is_some()) * gpu_world::SLOT_BYTES
        };
        (
            adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
            vxe_render::limits_for(adapter, pool),
        )
    }

    fn init(&mut self, gpu: &Gpu) {
        let [w, h] = self.args.render.unwrap_or(gpu.size);
        let timer = MarchTimer::new(&gpu.device, &gpu.queue);
        println!("device ready after {:.0} ms", ms_since(self.start));
        let attr_timer = self
            .args
            .look
            .and_then(|_| MarchTimer::new(&gpu.device, &gpu.queue));
        let mut marcher = Marcher::new(&gpu.device, w, h);
        let mirror = if self.stream.is_some() {
            marcher.set_lod(LodParams {
                r_full: self.args.residency.r_full,
                forced_level: None,
            });
            self.init_stream(gpu)
        } else {
            let mut mirror =
                self.new_mirror(gpu, MirrorConfig::new(Window::Fixed(self.world.extent())));
            mirror.set_focus(self.spawn);
            let t = now();
            match mirror.sync(&self.world, &gpu.queue) {
                Ok(s) => println!(
                    "mirror: uploaded {} B ({} bricks, {} chunks, {} writes) in {:.0} ms; mirror {} B live, {} B allocated",
                    s.bytes,
                    s.bricks,
                    s.chunks,
                    s.writes,
                    ms_since(t),
                    mirror.mirror_bytes(),
                    mirror.allocated_bytes()
                ),
                Err(e) => eprintln!("mirror sync failed: {e}"),
            }
            mirror
        };
        let mut gfx = Gfx {
            mirror,
            marcher,
            presenter: Presenter::new(&gpu.device, gpu.surface_format),
            timer,
            attr_timer,
            hud: HudOverlay::new(&gpu.device, gpu.surface_format, HUD_SCALE),
            hud_timer: PassTimer::new(&gpu.device, &gpu.queue),
        };
        if gfx.mirror.surface_attrs() {
            println!("{}", attr_line("initial fill", gfx.timed_attrs(gpu)));
        }
        self.gfx = Some(gfx);
    }

    /// The mirror's VRAM budget: `--mirror-budget`, else the level's from its content and
    /// extent, else the streamed default.
    fn mirror_budget(&self) -> u64 {
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
                let full = self.world.stats().full_bricks as u64;
                level_mirror_budget(full, chunks, self.args.look.is_some())
            }
        })
    }

    /// A mirror under `cfg` reserving [`Run::mirror_budget`], with attribute pages when
    /// `--look` was given. Prints what it reserved and how long creating its buffers took.
    fn new_mirror(&self, gpu: &Gpu, cfg: MirrorConfig) -> GpuWorld {
        let cfg = MirrorConfig {
            budget_bytes: self.mirror_budget(),
            ..cfg
        };
        let t = now();
        let mirror = if self.args.look.is_some() {
            GpuWorld::with_surface_attrs(&gpu.device, cfg)
        } else {
            GpuWorld::with_config(&gpu.device, cfg)
        };
        let ms = ms_since(t);
        let l = mirror.shard_layout();
        println!(
            "mirror: reserved {} B of a {} B budget ({} slots over {} shards, {} records) in {ms:.1} ms",
            mirror.allocated_bytes(),
            cfg.budget_bytes,
            l.reserved_slots,
            l.reserved_slots.div_ceil(l.slots_per_shard),
            l.reserved_records
        );
        mirror
    }

    /// The streaming mirror, returned once the chunks within squared distance `NEAR_D2` of
    /// the spawn chunk are resident and mirrored full.
    fn init_stream(&mut self, gpu: &Gpu) -> GpuWorld {
        let mut mirror = self.new_mirror(
            gpu,
            MirrorConfig {
                sync_budget_bytes: self.args.sync_budget,
                record_chunks: Some(flythrough::stream_record_chunks(self.args.residency)),
                ..MirrorConfig::new(Window::Follow {
                    side: flythrough::window_side(self.args.residency),
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
        loop {
            stream.streamer.update(&self.world, &[self.spawn]);
            stream.tick_ms.push(ms_since(self.start));
            let ev = stream.streamer.poll(&self.world);
            mirror.apply_events(&ev);
            if let Err(e) = mirror.sync(&self.world, &gpu.queue) {
                eprintln!("mirror sync failed: {e}");
                break;
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
        println!(
            "near residency ({} chunks resident and mirrored) after {:.0} ms",
            near.len(),
            ms_since(self.start)
        );
        mirror
    }

    fn resized(&mut self, gpu: &Gpu) {
        if let (Some(gfx), None) = (self.gfx.as_mut(), self.args.render) {
            gfx.marcher.resize(&gpu.device, gpu.size[0], gpu.size[1]);
        }
    }

    /// One frame of streaming: update, poll, apply events and sync, timed together.
    fn stream_step(&mut self, gpu: &Gpu) -> Result<(), String> {
        let (Some(stream), Some(gfx)) = (self.stream.as_mut(), self.gfx.as_mut()) else {
            return Ok(());
        };
        let cam = voxel_of(self.fly.camera.position);
        let t = now();
        stream.streamer.update(&self.world, &[cam]);
        let ev = stream.streamer.poll(&self.world);
        gfx.mirror.apply_events(&ev);
        gfx.mirror.set_focus(cam);
        let ts = now();
        let synced = gfx.mirror.sync(&self.world, &gpu.queue);
        let sync_ms = ms_since(ts);
        let cpu = ms_since(t);
        stream.tick_ms.push(ms_since(self.start));
        stream.cpu_ms.push(cpu);
        for e in &ev.errors {
            eprintln!("stream: {e:?}");
        }
        // A sync writes the nearest chunks that fit and defers the rest, which stay queued; an
        // empty sync (nothing fit) is an error that wrote nothing. The pressure rule below
        // lowers the radius until they fit.
        let tick = stream.streamer.stats().ticks;
        let (deferred, empty, error) = match synced {
            Ok(s) => {
                self.hud.sync = (s, sync_ms);
                (s.deferred_chunks, false, None)
            }
            Err(e) => (gfx.mirror.pending_chunks() as u64, true, Some(e)),
        };
        let m = &gfx.mirror;
        let [slots, records] = pressure_terms(m);
        if (deferred > 0 || empty) && stream.deferrals.syncs == 0 {
            let what = error.map_or_else(|| "deferred chunks".to_owned(), |e| e.to_string());
            eprintln!(
                "mirror: {deferred} chunks wait for space ({what}) at tick {tick} (pressure {:.4}, slots {slots:.4}, records {records:.4}; {} plain free slots, {} spare slots, {} free pages, {} pages issued, r_full {})",
                m.pressure(),
                m.free_slots().len(),
                m.spare_slots(),
                m.free_pages().len(),
                m.pages_issued(),
                stream.rule.r_full
            );
        }
        let ended =
            stream
                .deferrals
                .note(deferred, empty, [f64::from(m.pressure()), slots, records]);
        if let Some(n) = ended {
            println!("mirror: deferral streak of {n} syncs ended at tick {tick}");
        }
        let pressure = gfx.mirror.pressure();
        if let Some(r_full) = stream.rule.step(pressure) {
            let r = stream.streamer.config().residency;
            stream.streamer.set_radii(r_full, r.r_lod);
            gfx.marcher.set_lod(LodParams {
                r_full,
                forced_level: None,
            });
            stream.radius_changes += 1;
            println!(
                "radius: r_full {} -> {r_full} at mirror pressure {pressure:.3} after {:.0} ms (tick {tick})",
                r.r_full,
                ms_since(self.start)
            );
        }
        stream.track(&ev, &gfx.mirror);
        let st = stream.streamer.stats();
        if !stream.full_residency
            && st.in_flight == 0
            && st.queued == 0
            && gfx.mirror.pending_chunks() == 0
        {
            stream.full_residency = true;
            println!(
                "full residency after {:.0} ms: {} full, {} LOD-only chunks, mirror {} B live, {} B allocated, pressure {:.3}, r_full {}",
                ms_since(self.start),
                st.resident_full,
                st.resident_lod,
                gfx.mirror.mirror_bytes(),
                gfx.mirror.allocated_bytes(),
                gfx.mirror.pressure(),
                stream.rule.r_full
            );
        }
        Ok(())
    }

    /// Moves the camera along the autopilot's legs; returns `Flow::Exit` once it has printed
    /// its readings.
    fn pilot_step(&mut self) -> Flow {
        let (Some(p), Some(stream), Some(gfx)) =
            (self.pilot.as_mut(), self.stream.as_ref(), self.gfx.as_ref())
        else {
            return Flow::Continue;
        };
        let elapsed = (now() - p.leg_start).as_secs_f64();
        match p.leg {
            Leg::Settle => {
                if !stream.full_residency {
                    return Flow::Continue;
                }
                let o = centre_fixed(self.fly.camera.position);
                let op = EditOp::tool_along_ray(
                    &self.world,
                    o,
                    [FIXED_ONE, 0, 0],
                    PILOT_REACH,
                    PILOT_RADIUS,
                    Mode::Carve,
                );
                match self.world.apply(&op) {
                    Ok(r) => {
                        println!(
                            "autopilot: {}",
                            edit_line("carved", &r, self.world.materials(), 0.0)
                        );
                        p.touched = r.touched.clone();
                    }
                    Err(e) => println!("autopilot: carve refused, {e}"),
                }
                if p.touched.is_empty() {
                    println!("autopilot: nothing carved within 30 m along +x; no return check");
                }
                p.hash = chunks_hash(&self.world, &p.touched).unwrap_or(0);
                p.origin = self.fly.camera.position;
                p.leg = Leg::Out;
                p.leg_start = now();
                p.latency_from = stream.latency_ms.len();
                p.generated_at_takeoff = stream.streamer.stats().generated_chunks;
                println!("autopilot: flying +x at 10 m/s for 30 s");
            }
            Leg::Out | Leg::Back => {
                let s = elapsed.min(PILOT_LEG_SECS) as f32;
                let along = if p.leg == Leg::Out {
                    s
                } else {
                    PILOT_LEG_SECS as f32 - s
                };
                self.fly.camera.position = p.origin + Vec3::X * (PILOT_SPEED * along);
                self.fly.camera.yaw = if p.leg == Leg::Out {
                    0.0
                } else {
                    std::f32::consts::PI
                };
                self.fly.camera.pitch = 0.0;
                if elapsed >= PILOT_LEG_SECS {
                    if p.leg == Leg::Out {
                        p.leg = Leg::Back;
                        println!("autopilot: flying back for 30 s");
                    } else {
                        self.fly.camera.yaw = 0.0;
                        p.leg = Leg::Return;
                        println!("autopilot: back at the dig; waiting for its chunks");
                    }
                    p.leg_start = now();
                }
            }
            Leg::Return => {
                let back = p.touched.iter().all(|&c| {
                    stream.streamer.tier(c) == Some(Tier::Full)
                        && gfx.mirror.is_mirrored(c)
                        && !gfx.mirror.is_lod(c)
                });
                if back {
                    let hash = chunks_hash(&self.world, &p.touched);
                    let verdict = if hash == Some(p.hash) {
                        "hash match"
                    } else {
                        "hash MISMATCH"
                    };
                    println!("return: {} chunks, {verdict}", p.touched.len());
                    self.pilot_report();
                    return Flow::Exit;
                }
            }
        }
        Flow::Continue
    }

    /// Per-frame flight readings: unready-near count, streaming CPU, memory peaks.
    fn pilot_sample(&mut self) {
        let in_flight = self
            .pilot
            .as_ref()
            .is_some_and(|p| matches!(p.leg, Leg::Out | Leg::Back));
        if !in_flight {
            return;
        }
        let resident = if self.frame.is_multiple_of(60) {
            Some(self.resident_ram())
        } else {
            None
        };
        let (Some(p), Some(stream), Some(gfx)) =
            (self.pilot.as_mut(), self.stream.as_ref(), self.gfx.as_ref())
        else {
            return;
        };
        p.flight_frames += 1;
        let cam = voxel_of(self.fly.camera.position).chunk();
        let unready = p.near.iter().any(|o| {
            let c = ChunkCoord([0, 1, 2].map(|k| cam.0[k] + o[k]));
            !(stream.streamer.tier(c) == Some(Tier::Full)
                && gfx.mirror.is_mirrored(c)
                && !gfx.mirror.is_lod(c))
        });
        p.unready_frames += u64::from(unready);
        if let Some(&cpu) = stream.cpu_ms.last() {
            p.cpu_ms.push(cpu);
        }
        p.peak_mirror = p.peak_mirror.max(gfx.mirror.mirror_bytes());
        p.peak_allocated = p.peak_allocated.max(gfx.mirror.allocated_bytes());
        p.peak_pressure = p.peak_pressure.max(gfx.mirror.pressure());
        let terms = pressure_terms(&gfx.mirror);
        p.peak_terms = [0, 1].map(|k| p.peak_terms[k].max(terms[k]));
        p.peak_persisted = p
            .peak_persisted
            .max(stream.streamer.stats().persisted_ram_bytes);
        if let Some(r) = resident {
            p.peak_resident = p.peak_resident.max(r);
        }
    }

    fn pilot_report(&mut self) {
        let resident = self.resident_ram();
        let (Some(p), Some(stream), Some(gfx)) =
            (self.pilot.as_mut(), self.stream.as_mut(), self.gfx.as_ref())
        else {
            return;
        };
        let r = self.args.residency;
        println!(
            "autopilot: unready-near frames {} of {} (chunks wanted full within {} of the camera's)",
            p.unready_frames,
            p.flight_frames,
            r.r_full - 2
        );
        let mut cpu = p.cpu_ms.clone();
        println!(
            "autopilot: streaming CPU per frame (update + poll + sync): p50 {:.3} ms, p99 {:.3} ms, max {:.3} ms over {} frames (line 1.0 ms)",
            percentile(&mut cpu, 50.0),
            percentile(&mut cpu, 99.0),
            percentile(&mut cpu, 100.0),
            cpu.len()
        );
        let mut lat = stream.latency_ms[p.latency_from..].to_vec();
        println!(
            "autopilot: residency latency wanted to mirrored: p95 {:.0} ms, max {:.0} ms over {} chunks (proposal 500 ms)",
            percentile(&mut lat, 95.0),
            percentile(&mut lat, 100.0),
            lat.len()
        );
        let st = stream.streamer.stats();
        println!(
            "autopilot: peak mirror {} B live, {} B allocated, resident RAM peak {} B (now {} B), process peak RSS {} (dev reference), persisted RAM peak {} B",
            p.peak_mirror,
            p.peak_allocated,
            p.peak_resident.max(resident),
            resident,
            peak_rss_bytes().map_or_else(|| "n/a".to_owned(), |b| format!("{b} B")),
            p.peak_persisted.max(st.persisted_ram_bytes)
        );
        println!(
            "autopilot: mirror budget {} B ({} slots, {} records), peak live / budget {:.4}, peak pressure {:.4} (slots {:.4}, records {:.4}), {} radius changes (r_full {} of {}), {} syncs with a deferral",
            gfx.mirror.allocated_bytes(),
            gfx.mirror.reserved_slots(),
            gfx.mirror.reserved_records(),
            p.peak_mirror as f64 / gfx.mirror.allocated_bytes() as f64,
            p.peak_pressure,
            p.peak_terms[0],
            p.peak_terms[1],
            stream.radius_changes,
            stream.rule.r_full,
            r.r_full,
            stream.deferrals.syncs
        );
        if stream.deferrals.syncs > 0 {
            let f = stream.deferrals;
            println!(
                "autopilot: deferrals: {} chunks over {} syncs, {} of them empty; pressure {:.4} to {:.4} (slots {:.4} to {:.4}, records {:.4} to {:.4}); longest streak {} syncs ({} open at the end)",
                f.chunks,
                f.syncs,
                f.empty,
                f.low[0],
                f.high[0],
                f.low[1],
                f.high[1],
                f.low[2],
                f.high[2],
                f.longest,
                f.streak
            );
        }
        if p.gpu_ms.is_empty() {
            println!("autopilot: march GPU n/a (no TIMESTAMP_QUERY)");
        } else {
            let [w, h] = [gfx.marcher.target().width(), gfx.marcher.target().height()];
            println!(
                "autopilot: march GPU at {w}x{h} over the flight: p50 {:.3} ms, p99 {:.3} ms, max {:.3} ms over {} frames",
                percentile(&mut p.gpu_ms, 50.0),
                percentile(&mut p.gpu_ms, 99.0),
                percentile(&mut p.gpu_ms, 100.0),
                p.gpu_ms.len()
            );
        }
        if !p.attr.items.is_empty() {
            for line in p.attr.lines() {
                println!("{line}");
            }
        }
        let generated = st.generated_chunks - p.generated_at_takeoff;
        let secs = 2.0 * PILOT_LEG_SECS + (now() - p.leg_start).as_secs_f64();
        println!(
            "autopilot: generated {generated} chunks in about {secs:.1} s on {} workers: {:.3e} voxels/s/worker",
            stream.workers,
            generated as f64 * 262_144.0 / secs / stream.workers as f64
        );
    }

    fn frame(&mut self, gpu: &Gpu, input: &Input, dt: f32, surface: &wgpu::TextureView) -> Flow {
        let t0 = now();
        if input.pressed(KeyCode::Escape) {
            return Flow::Exit;
        }
        // The first frame's delta spans startup, not a frame.
        if self.frame > 0 {
            self.hud.frames.record(dt);
        }
        if input.pressed(KeyCode::F2) {
            self.hud.visible = !self.hud.visible;
        }
        if self.frame == 0 {
            for _ in 0..self.args.bench {
                self.bench();
            }
        }
        if self.pilot.is_some() {
            if self.pilot_step() == Flow::Exit {
                return Flow::Exit;
            }
        } else {
            match self.walk.as_mut() {
                Some(w) => {
                    for line in w.update(&self.world, input, &mut self.fly.camera, dt) {
                        println!("{line}");
                    }
                }
                None => self.fly.update(input, dt),
            }
            self.handle_input(input);
        }
        self.audio_step();
        if let Some(n) = self.args.frames
            && self.frame + SAMPLES as u32 == n
        {
            self.samples = Some(Samples::default());
        }
        self.draw(gpu, surface, t0)
    }

    /// The keys and buttons of interactive flight; the autopilot takes none of them, so a
    /// stray click cannot change the world it measures.
    fn handle_input(&mut self, input: &Input) {
        if input.pressed(KeyCode::KeyL) {
            self.light.shadows = !self.light.shadows;
            println!(
                "shadow ray {}",
                if self.light.shadows { "on" } else { "off" }
            );
        }
        if input.scroll() != 0.0
            && let Some(r) = scroll_dig_radius(self.dig_radius, &mut self.scroll, input.scroll())
        {
            self.dig_radius = r;
            println!("dig radius {:.1} m", r as f64 / (10 * FIXED_ONE) as f64);
        }
        if input.clicked(MouseButton::Left) {
            self.dig();
        }
        if input.clicked(MouseButton::Right) {
            self.fill();
        }
        if input.pressed(KeyCode::KeyB) {
            self.bench();
        }
        if input.pressed(KeyCode::KeyP) {
            self.stamp();
        }
        if input.pressed(KeyCode::KeyG) {
            self.toggle_walk();
        }
        if input.pressed(KeyCode::KeyK) {
            println!("{}", self.coarse_line());
        }
        if input.pressed(KeyCode::KeyR) {
            match self.coarse {
                Some(_) => self.send(DigInput::Crumble),
                None => println!("collapse: not kept while streaming"),
            }
        }
        if input.pressed(KeyCode::KeyN) {
            let contour = self.gfx.as_ref().is_some_and(|g| g.mirror.surface_attrs());
            self.look.normal_mode = self.look.normal_mode.next(contour);
            println!("{}", look_line(&self.look, self.rock_detail()));
        }
        if input.pressed(KeyCode::KeyV) {
            self.look.detail = !self.look.detail;
            println!("{}", look_line(&self.look, self.rock_detail()));
        }
        if input.pressed(KeyCode::F1) {
            self.samples = Some(Samples::default());
        }
        if let Some(audio) = self.audio.as_mut() {
            if input.pressed(KeyCode::KeyH) {
                audio.play("knock", PlayParams::at);
            }
            if input.pressed(KeyCode::KeyT) {
                audio.play("pulse", |p| PlayParams {
                    looping: true,
                    ..PlayParams::at(p)
                });
            }
            if input.pressed(KeyCode::KeyE) {
                let on = !audio.acoustics.reflections_on();
                audio.acoustics.set_reflections(on);
                println!("echoes {}", if on { "on" } else { "off" });
            }
            if input.pressed(KeyCode::KeyY) {
                let on = !audio.acoustics.reverb_on();
                audio.acoustics.set_reverb(on);
                println!("tail {}", if on { "on" } else { "off" });
            }
            if input.pressed(KeyCode::KeyJ) {
                audio.play("hum", |p| PlayParams {
                    looping: true,
                    ..PlayParams::at(p)
                });
            }
        }
    }

    /// Sends the camera to the mixer as its listener and probes the voices' occlusion.
    fn audio_step(&mut self) {
        let listener = flythrough::listener(&self.fly.camera);
        let Some(audio) = self.audio.as_mut() else {
            if let Some(p) = self.room_probe.as_mut() {
                let mut completed = false;
                p.acoustics.update(
                    &self.world,
                    &p.table,
                    &listener,
                    &[],
                    METRES_PER_VOXEL,
                    &mut |cmd| completed |= matches!(cmd, Command::SetRoom(_)),
                );
                if completed && let Some(r) = p.acoustics.room() {
                    println!("{}", room_line(&r));
                }
            }
            return;
        };
        // A full channel drops this frame's listener; the next frame sends a fresh one.
        let _ = audio.output.set_listener(listener);
        let errors = audio.output.stream_errors();
        if errors > audio.stream_errors {
            println!("audio: {errors} stream errors so far (an underrun is heard as a click)");
            audio.stream_errors = errors;
        }
        if ms_since(audio.stats_at) >= 1000.0 {
            let s = audio.output.take_stats();
            audio.callback_max_ms = s.busy_max_ms;
            audio.stats_at = now();
            if audio.debug {
                println!(
                    "audio-debug: {} callbacks of {}..{} frames, busy max {:.3} ms, gap max {:.2} ms, {} late, peak {:.3}, {} clipped, kink max {:.2e}, {} voices",
                    s.callbacks,
                    s.frames_min,
                    s.frames_max,
                    s.busy_max_ms,
                    s.gap_max_ms,
                    s.late,
                    s.peak,
                    s.clipped,
                    s.kink_max,
                    audio.voices.len()
                );
            }
        }
        for id in audio.output.finished() {
            audio.voices.retain(|(v, _)| *v != id);
        }
        if let Some(mic) = audio.mic.as_mut() {
            // The microphone's voice stands at the listener.
            let _ = audio.output.set_position(MIC_VOICE, listener.position);
            if let Some(v) = audio.voices.iter_mut().find(|(v, _)| *v == MIC_VOICE) {
                v.1 = listener.position;
            }
            let period = mic.input.period_frames();
            if !mic.period_printed && period > 0 {
                println!(
                    "mic: period {period} frames ({:.1} ms)",
                    period as f32 * 1e3 / vxe_audio::RATE as f32
                );
                mic.period_printed = true;
            }
            let errors = mic.input.stream_errors();
            if errors > mic.stream_errors {
                println!("mic: {errors} stream errors so far");
                mic.stream_errors = errors;
            }
        }
        let output = &audio.output;
        let t = now();
        let mut completed = false;
        let debug = audio.debug;
        let discarded = audio.acoustics.discarded_fans();
        audio.acoustics.update(
            &self.world,
            &audio.table,
            &listener,
            &audio.voices,
            METRES_PER_VOXEL,
            // A full channel drops these targets; the voice is probed again within 500 ms.
            &mut |cmd| {
                if let Command::SetRoom(r) = &cmd {
                    completed = true;
                    if debug {
                        println!("{}", room_sent_line(r));
                    }
                }
                let _ = output.send(cmd);
            },
        );
        audio.update_ms = ms_since(t);
        let now_discarded = audio.acoustics.discarded_fans();
        if debug {
            if now_discarded > discarded && audio.discarded_run == 0 {
                println!("room: fan discarded, the listener is inside a solid");
            }
            if now_discarded > discarded {
                audio.discarded_run += now_discarded - discarded;
            } else if audio.discarded_run > 0 {
                println!(
                    "room: out of the solid after {} discarded fans",
                    audio.discarded_run
                );
                audio.discarded_run = 0;
            }
        }
        if completed
            && (audio.room_line || audio.debug)
            && let Some(r) = audio.acoustics.room()
        {
            println!("{}", room_line(&r));
        }
    }

    /// Ticks the server, then streams, syncs, marches and presents one frame begun at `t0`.
    fn draw(&mut self, gpu: &Gpu, surface: &wgpu::TextureView, t0: Instant) -> Flow {
        self.tick(gpu);
        if self.stream.is_some() {
            if let Err(e) = self.stream_step(gpu) {
                eprintln!("mirror sync failed: {e}");
                return Flow::Exit;
            }
        } else if let Some(gfx) = self.gfx.as_mut() {
            let t = now();
            match gfx.mirror.sync(&self.world, &gpu.queue) {
                Ok(s) => self.hud.sync = (s, ms_since(t)),
                Err(e) => {
                    eprintln!("mirror sync failed: {e}");
                    return Flow::Exit;
                }
            }
        }
        self.pilot_sample();
        let hud = self.hud.visible;
        if self.hud.cadence.due(hud) {
            self.sample_world();
        }
        // The steps row reads the last frame's target back before this frame's march
        // overwrites it: a blocking copy, paid only with the HUD and that view both on.
        self.hud.steps = None;
        if hud
            && self.look.view == ViewMode::Steps
            && let Some(gfx) = &self.gfx
        {
            let px = readback::read_rgba8(&gpu.device, &gpu.queue, gfx.marcher.target());
            let width = gfx.marcher.target().width() as usize;
            let h = StepHistogram::from_rgba8(&px, width);
            self.hud.steps = Some(hud::StepsSample::from_histogram(&h));
        }
        if hud {
            self.update_hud(gpu);
        }
        let flying = self
            .pilot
            .as_ref()
            .is_some_and(|p| matches!(p.leg, Leg::Out | Leg::Back));

        let Some(gfx) = self.gfx.as_mut() else {
            return Flow::Exit;
        };
        gfx.marcher.set_look(self.look);
        let params = gfx
            .marcher
            .params(&gfx.mirror, &self.fly.camera, &self.light);
        let timer = gfx
            .timer
            .as_ref()
            .filter(|_| self.samples.is_some() || flying || hud);
        let hud_timer = gfx.hud_timer.as_ref().filter(|_| hud);
        let mut enc = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        // Under the autopilot's flight the attribute pass is timed: its GPU time when its
        // work list is not empty, and the render thread's CPU time building the list and
        // recording the pass.
        let attr_timer = gfx.attr_timer.as_ref().filter(|_| flying);
        let t_attr = now();
        let attr_items = gfx
            .mirror
            .update_attrs_timed(&mut enc, attr_timer.map(MarchTimer::writes));
        let attr_cpu_ms = ms_since(t_attr);
        let attr_timer = attr_timer.filter(|_| attr_items > 0);
        if let Some(t) = attr_timer {
            t.resolve(&mut enc);
        }
        gfx.marcher.encode(
            &gpu.device,
            &gpu.queue,
            &mut enc,
            &gfx.mirror,
            &params,
            timer.map(MarchTimer::writes),
        );
        if let Some(t) = timer {
            t.resolve(&mut enc);
        }
        gfx.presenter
            .encode(&gpu.device, &mut enc, gfx.marcher.target(), surface);
        if hud {
            let [w, h] = gpu.size;
            gfx.hud.encode_timed(
                &mut enc,
                surface,
                w,
                h,
                hud_timer.map(PassTimer::render_writes),
            );
            if let Some(t) = hud_timer {
                t.resolve(&mut enc);
            }
        }
        gpu.queue.submit([enc.finish()]);
        let cpu_ms = ms_since(t0);

        if self.frame == 0 {
            // The first frame counts once the GPU has finished it.
            let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
            println!("first frame after {:.0} ms", ms_since(self.start));
        }
        let gpu_ms = timer.map(|t| t.read_ms(&gpu.device));
        let attr_gpu_ms = attr_timer.map(|t| t.read_ms(&gpu.device));
        let hud_ms = hud_timer.map(|t| t.read_ms(&gpu.device));
        self.hud.cpu_ms = cpu_ms as f32;
        self.hud.march_ms = gpu_ms.map(|v| v as f32);
        self.hud.hud_ms = hud_ms.map(|v| v as f32);
        if flying && let (Some(p), Some(ms)) = (self.pilot.as_mut(), gpu_ms) {
            p.gpu_ms.push(ms);
        }
        if flying
            && gfx.mirror.surface_attrs()
            && let Some(p) = self.pilot.as_mut()
        {
            p.attr.items.push(attr_items as f64);
            p.attr.cpu_ms.push(attr_cpu_ms);
            p.attr.gpu_ms.extend(attr_gpu_ms);
        }
        if let Some(s) = self.samples.as_mut() {
            s.cpu_ms.push(cpu_ms);
            if let Some(ms) = gpu_ms {
                s.gpu_ms.push(ms);
            }
            if let Some(ms) = hud_ms {
                s.hud_ms.push(ms);
            }
            if s.cpu_ms.len() == SAMPLES {
                let s = self.samples.take().unwrap_or_default();
                self.report(s);
            }
        }
        self.frame += 1;
        if self.args.frames.is_some_and(|n| self.frame >= n) {
            if let Some(w) = &self.walk {
                println!(
                    "walk: foot at {:?} (fixed), {}",
                    w.state.foot.0,
                    if w.state.grounded {
                        "grounded"
                    } else {
                        "airborne"
                    }
                );
            }
            if self.coarse.is_some() {
                println!("{}", self.coarse_line());
            }
            if self.look.view == ViewMode::Steps
                && let Some(gfx) = &self.gfx
            {
                let px = readback::read_rgba8(&gpu.device, &gpu.queue, gfx.marcher.target());
                let width = gfx.marcher.target().width() as usize;
                println!("{}", steps_line(&StepHistogram::from_rgba8(&px, width)));
            }
            self.sample_world();
            self.refresh_hud_stats();
            let mut stats = self.hud.stats.clone();
            stats.world.age_frames = 0;
            println!("hud:\n{stats}");
            return Flow::Exit;
        }
        Flow::Continue
    }
}

/// The steps view's reading of one frame: `steps per ray (2073600 rays): mean 41.27, p99 133,
/// block-max mean 88.10; loads mean 12.40, p99 61; pool reads mean 3.02, p99 18`. Counts clamp
/// at 255 as the view does; the block-max mean is over 8 × 8 pixel blocks of each block's
/// maximum steps; loads are the ray's 32-bit storage loads from the mirror, pool reads the
/// pool loads among them.
fn steps_line(h: &StepHistogram) -> String {
    format!(
        "steps per ray ({} rays): mean {:.2}, p99 {}, block-max mean {:.2}; loads mean {:.2}, p99 {}; pool reads mean {:.2}, p99 {}",
        h.rays(),
        h.mean_steps(),
        h.p99_steps(),
        h.block_max_mean(),
        h.mean_loads(),
        h.p99_loads(),
        h.mean_pool_reads(),
        h.p99_pool_reads()
    )
}

/// `look face, detail off, view shaded`, or with rock's detail under `--look`:
/// `look face, detail off (rock scale 4, strength 0.3), view shaded`.
fn look_line(look: &LookParams, rock_detail: Option<[f32; 2]>) -> String {
    let rock = rock_detail.map_or_else(String::new, |[scale, strength]| {
        format!(" (rock scale {scale}, strength {strength})")
    });
    format!(
        "look {}, detail {}{rock}, view {}",
        look.normal_mode.name(),
        if look.detail { "on" } else { "off" },
        look.view.name()
    )
}

/// Logs a teardown stage before and after it runs, like the shell's, so a hang at exit
/// names its stage.
fn teardown_stage(what: &str, f: impl FnOnce()) {
    log::info!("teardown: dropping the {what}");
    let t = now();
    f();
    log::info!("teardown: {what} dropped in {:.1} ms", ms_since(t));
}

impl Drop for Run {
    /// Writes the `--save` file, drops the server and client, stops and joins the streamer's
    /// workers, then drops the GPU mirror and passes, then the world (swapped for an empty
    /// one, whose drop is free; the server and client held the other references).
    fn drop(&mut self) {
        self.save();
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

/// The fixed-point centre of the voxel holding `p`.
fn centre_fixed(p: Vec3) -> Fixed3 {
    vxe_world::centre_of_voxel(voxel_of(p))
}

/// What the app builds its [`Run`] from once the device's limits are asked for.
enum Pending {
    /// The bounded level, generating on its own threads.
    Level(JoinHandle<World>),
    /// The streaming world, its workers already generating around the spawn.
    Stream(Box<(World, Streamer, usize)>),
}

/// The app around [`Run`]: the world generates on its own threads while the shell opens the
/// window and creates the device (instance creation alone can take over a second), and is
/// joined when the device limits need its size.
struct Flythrough {
    start: Instant,
    args: Args,
    camera: Camera,
    /// The server's logic, loaded before the window opens; taken when the run starts.
    logic: Option<Box<dyn Logic>>,
    pending: Option<Pending>,
    run: Option<Run>,
    audio: Option<Audio>,
    room_probe: Option<RoomProbe>,
}

impl App for Flythrough {
    fn device_request(&mut self, adapter: &wgpu::Adapter) -> (wgpu::Features, wgpu::Limits) {
        if let Some(p) = self.pending.take() {
            let (world, stream) = match p {
                Pending::Level(g) => (
                    g.join().unwrap_or_else(|e| std::panic::resume_unwind(e)),
                    None,
                ),
                Pending::Stream(boxed) => {
                    let (world, streamer, workers) = *boxed;
                    (
                        world,
                        Some(Stream {
                            rule: RadiusRule::new(streamer.config().residency.r_full),
                            radius_changes: 0,
                            deferrals: Deferrals::default(),
                            streamer,
                            tick_ms: Vec::new(),
                            awaiting: HashMap::new(),
                            latency_ms: Vec::new(),
                            cpu_ms: Vec::new(),
                            full_residency: false,
                            workers,
                        }),
                    )
                }
            };
            let pilot = self.args.autopilot.then(|| Autopilot {
                leg: Leg::Settle,
                leg_start: self.start,
                origin: self.camera.position,
                touched: Vec::new(),
                hash: 0,
                near: ball((self.args.residency.r_full - 2).pow(2)),
                flight_frames: 0,
                unready_frames: 0,
                gpu_ms: Vec::new(),
                cpu_ms: Vec::new(),
                attr: AttrSamples::default(),
                latency_from: 0,
                peak_mirror: 0,
                peak_allocated: 0,
                peak_pressure: 0.0,
                peak_terms: [0.0; 2],
                peak_resident: 0,
                peak_persisted: 0,
                generated_at_takeoff: 0,
            });
            let coarse = stream.is_none().then(|| Coarse::build(&world));
            let world = Arc::new(world);
            let logic = self
                .logic
                .take()
                .expect("the logic is loaded once, in main");
            let net = Net::new(&world, logic, self.args.seed);
            let walk = if self.args.walk {
                let w = Walk::at_camera(&world, &self.camera);
                if w.is_none() {
                    println!("walk: no room for the body at the spawn; flying");
                }
                w
            } else {
                None
            };
            self.run = Some(Run {
                start: self.start,
                world,
                net: Some(net),
                spawn: voxel_of(self.camera.position),
                args: self.args.clone(),
                fly: Fly::new(self.camera),
                light: LightParams::headlamp(),
                look: self.args.start_look(),
                dig_radius: DIG_RADIUS,
                scroll: ScrollSteps::default(),
                bench_runs: 0,
                frame: 0,
                gfx: None,
                samples: None,
                stream,
                pilot,
                walk,
                coarse,
                hud: Hud::new(self.args.hud),
                audio: self.audio.take(),
                room_probe: self.room_probe.take(),
            });
        }
        match self.run.as_mut() {
            Some(run) => run.device_request(adapter),
            None => (wgpu::Features::empty(), wgpu::Limits::default()),
        }
    }

    fn init(&mut self, gpu: &Gpu) {
        if let Some(run) = self.run.as_mut() {
            run.init(gpu);
        }
    }

    fn resized(&mut self, gpu: &Gpu) {
        if let Some(run) = self.run.as_mut() {
            run.resized(gpu);
        }
    }

    fn frame(&mut self, gpu: &Gpu, input: &Input, dt: f32, surface: &wgpu::TextureView) -> Flow {
        match self.run.as_mut() {
            Some(run) => run.frame(gpu, input, dt, surface),
            None => Flow::Exit,
        }
    }
}

/// Runs `app` without a window: a device on the high-performance adapter and an offscreen
/// target in place of the surface, the same frames until the app exits. Teardown follows
/// the windowed shell's order (the app, then the queue, device and adapter), traced alike.
fn run_headless(mut app: Flythrough, size: [u32; 2]) -> Result<(), String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster_block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .map_err(|e| format!("no adapter: {e}"))?;
    println!("headless on {}", adapter.get_info().name);
    let (required_features, required_limits) = app.device_request(&adapter);
    let (device, queue) = pollster_block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features,
        required_limits,
        ..Default::default()
    }))
    .map_err(|e| format!("no device: {e}"))?;
    let surface_format = wgpu::TextureFormat::Rgba8Unorm;
    let gpu = Gpu {
        adapter,
        device,
        queue,
        surface_format,
        size,
    };
    app.init(&gpu);
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("headless surface"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: surface_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let input = Input::default();
    let mut last = now();
    loop {
        let t = now();
        let dt = (t - last).as_secs_f32();
        last = t;
        if app.frame(&gpu, &input, dt, &view) == Flow::Exit {
            log::info!("teardown: the app asked to exit");
            break;
        }
        // Keep the queue from running ahead of the GPU, as presenting would.
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    }
    drop(view);
    drop(target);
    teardown_stage("app", || drop(app));
    let Gpu {
        adapter,
        device,
        queue,
        ..
    } = gpu;
    teardown_stage("queue", || drop(queue));
    teardown_stage("device", || drop(device));
    teardown_stage("adapter", || drop(adapter));
    Ok(())
}

/// Drives a future to completion on this thread; `wgpu`'s native futures are ready at once
/// or after a device poll.
fn pollster_block_on<F: std::future::Future>(f: F) -> F::Output {
    let waker = std::task::Waker::noop();
    let mut cx = std::task::Context::from_waker(waker);
    let mut f = std::pin::pin!(f);
    loop {
        if let std::task::Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::yield_now();
    }
}

/// The cave's material table; under `--look`, with rock's procedural detail at
/// `--rock-detail`.
fn materials(args: &Args) -> MaterialTable {
    if args.look.is_some() {
        flythrough::look_materials_with(args.rock_detail)
    } else {
        cave_materials()
    }
}

fn main() {
    // The fly-through's own lines and the shell's teardown trace show by default; RUST_LOG
    // overrides.
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("error,vxe_app=info,flythrough=info"),
    )
    .init();
    let start = now();
    let mut args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("flythrough: {e}");
            std::process::exit(2);
        }
    };

    let load_logic = |choice: &LogicChoice| match choice.load() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("flythrough: --logic {choice}: {e}");
            std::process::exit(1);
        }
    };
    println!("{}", look_line(&args.start_look(), args.rock_detail()));
    let logic = load_logic(&args.logic);
    println!("logic: {}", args.logic);

    let seed = args.seed;
    let (spawn, pending) = if let Some(path) = args.replay.clone() {
        let replay_logic = load_logic(&args.logic);
        let file = match flythrough::read_log_file(&path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("flythrough: --replay {}: {e}", path.display());
                std::process::exit(1);
            }
        };
        // The session continues under the save's seed.
        args.seed = file.seed;
        let table = materials(&args);
        let source = CaveSource::new(file.seed, file.extent);
        let with_vault = args.vault;
        if with_vault {
            println!(
                "vault: {VAULT_EDGE}^3 shell room at {:?}",
                source.vault_origin().0
            );
        }
        let generation = std::thread::spawn(move || {
            let (world, ops) = match flythrough::replay_log_on(
                &file,
                flythrough::Mode::Inputs,
                ChunkExtent::new([0, 0, 0], LEVEL_CHUNKS),
                &table,
                replay_logic,
                with_vault,
            ) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("flythrough: --replay {}: {e}", path.display());
                    std::process::exit(1);
                }
            };
            println!(
                "replayed {} inputs, {} ops, hash 0x{:016x}",
                file.inputs.len(),
                ops.len(),
                world.voxel_hash()
            );
            if ops != file.ops {
                println!("replay: the re-simulated op log differs from the saved one");
            }
            world
        });
        (source.spawn(), Pending::Level(generation))
    } else if let Some(edge) = args.room {
        let source = RoomSource::new(edge);
        let table = materials(&args);
        println!(
            "room: a closed {:.1} m cube of air in 2 m of rock, {:?} chunks",
            edge as f32 * METRES_PER_VOXEL,
            source.extent.max.0
        );
        let generation =
            std::thread::spawn(move || World::generate(source.extent, &source, &table));
        (source.spawn(), Pending::Level(generation))
    } else if args.level {
        let source = CaveSource::new(seed, ChunkExtent::new([0, 0, 0], LEVEL_CHUNKS));
        if args.vault {
            println!(
                "vault: {VAULT_EDGE}^3 shell room at {:?}",
                source.vault_origin().0
            );
        }
        let table = materials(&args);
        let with_vault = args.vault;
        let generation = std::thread::spawn(move || {
            let t = now();
            let world = if with_vault {
                World::generate(source.extent, &source.with_vault(), &table)
            } else {
                World::generate(source.extent, &source, &table)
            };
            println!(
                "generated seed {seed} over {LEVEL_CHUNKS:?} chunks in {:.0} ms: {} full bricks",
                ms_since(t),
                world.stats().full_bricks
            );
            world
        });
        (source.spawn(), Pending::Level(generation))
    } else {
        let source = CaveSource::unbounded(seed);
        let workers = std::thread::available_parallelism()
            .map_or(1, usize::from)
            .saturating_sub(1)
            .max(1);
        let world = World::new(&materials(&args));
        let cfg = StreamConfig {
            residency: args.residency,
            workers,
            persist: PersistConfig::default(),
        };
        let mut streamer = if args.vault {
            println!(
                "vault: {VAULT_EDGE}^3 shell room at {:?}",
                source.vault_origin().0
            );
            Streamer::new(Arc::new(source.with_vault()), cfg)
        } else {
            Streamer::new(Arc::new(source), cfg)
        };
        // Workers start on the spawn's chunks while the shell opens the window.
        streamer.update(&world, &[source.spawn()]);
        println!(
            "streaming seed {seed}: r_full {}, r_lod {}, h {}, {workers} workers, window {}",
            args.residency.r_full,
            args.residency.r_lod,
            args.residency.hysteresis,
            flythrough::window_side(args.residency)
        );
        (
            source.spawn(),
            Pending::Stream(Box::new((world, streamer, workers))),
        )
    };

    let camera = Camera {
        position: spawn.0.map(|v| v as f32 + 0.5).into(),
        yaw: 0.0,
        pitch: 0.0,
        vfov: 70f32.to_radians(),
    };
    let size = args.render.unwrap_or([1280, 720]);
    let headless = args.headless;
    let audio = if args.audio {
        Audio::open(args.audio_debug, args.absorption, args.mic, args.room_line)
    } else {
        None
    };
    let room_probe = (audio.is_none() && args.room_line).then(|| RoomProbe::new(args.absorption));
    let app = Flythrough {
        start,
        args,
        camera,
        logic: Some(logic),
        pending: Some(pending),
        run: None,
        audio,
        room_probe,
    };
    let result = if headless {
        run_headless(app, size)
    } else {
        let config = ShellConfig {
            title: "flythrough".to_owned(),
            size,
            lock_cursor: true,
        };
        vxe_app::run(config, app).map_err(|e| e.to_string())
    };
    if let Err(e) = result {
        eprintln!("flythrough: {e}");
        std::process::exit(1);
    }
    log::info!("teardown: done, exiting the process");
}

/// Bytes allocated on each thread, counted for the HUD's no-allocation test.
#[cfg(test)]
mod counting {
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

#[cfg(test)]
#[global_allocator]
static ALLOC: counting::Counting = counting::Counting;

#[cfg(test)]
mod tests {
    use super::*;

    /// 32 frames of an eighth of a line raise the dig radius from 0.5 m by four steps to
    /// 0.9 m, each step reported once. At 3.0 m further upward scroll reports nothing and
    /// banks nothing, so one line down steps to 2.9 m at once.
    #[test]
    fn smooth_scrolling_steps_the_dig_radius() {
        let mut scroll = ScrollSteps::default();
        let mut r = DIG_RADIUS;
        let mut changes = Vec::new();
        for _ in 0..32 {
            if let Some(n) = scroll_dig_radius(r, &mut scroll, 0.125) {
                r = n;
                changes.push(r);
            }
        }
        assert_eq!(changes, [6, 7, 8, 9].map(|v| v * FIXED_ONE));
        assert_eq!(r, 9 * FIXED_ONE);

        let mut r = 30 * FIXED_ONE;
        for delta in [0.125, 0.5, 1.0, 0.875, 0.125] {
            assert_eq!(scroll_dig_radius(r, &mut scroll, delta), None);
        }
        assert_eq!(scroll.carry(), 0.0);
        r = scroll_dig_radius(r, &mut scroll, -1.0).expect("one line down steps");
        assert_eq!(r, 29 * FIXED_ONE);
    }

    /// A shown HUD allocates nothing per frame once its buffers have grown: 100 frames of
    /// recording a frame time, refreshing the values a frame changes, and formatting the
    /// rows and the strip, with a last edit held; frames 2 to 100 allocate 0 bytes. The
    /// overlay's half (rows and samples written, the pass encoded) is
    /// `vxe-render`'s `the_overlay_allocates_nothing_after_its_first_frame`.
    #[test]
    fn a_shown_hud_allocates_nothing_after_its_first_frame() {
        let mut hud = Hud::new(true);
        hud.stats = HudStats::fixed();
        let mut per_frame = Vec::with_capacity(100);
        for frame in 0..100u32 {
            let before = counting::bytes();
            hud.frames.record(0.016 + frame as f32 * 1e-5);
            hud.stats.fps = hud.frames.fps();
            hud.stats.frame_ms = hud.frames.frame_ms();
            hud.stats.format_us = hud.format_us;
            hud.format_rows();
            per_frame.push(counting::bytes() - before);
        }
        assert!(per_frame[0] > 0, "the first frame grows the buffers");
        assert_eq!(per_frame[1..], [0; 99]);
        assert_eq!(hud.text.lines().count(), 12);
        assert_eq!(hud.strip.len(), 100);
    }
    use flythrough::cave::SHELL;

    fn argv(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| (*s).to_owned()).collect()
    }

    /// `--walk` and `G` refuse walk mode alike while streaming; the bounded level allows it.
    /// The radius falls a chunk after 30 frames above 0.9, not before, and not below the
    /// floor; it rises a chunk after 300 frames below 0.7, not past the configured radius; a
    /// frame between the lines restarts both counts.
    #[test]
    fn the_radius_follows_the_pressure_between_its_floor_and_the_configured_radius() {
        let mut r = RadiusRule::new(8);
        for _ in 0..29 {
            assert_eq!(r.step(0.95), None);
        }
        assert_eq!(r.step(0.95), Some(7));
        assert_eq!(r.step(0.95), None, "the count restarts after a change");
        for _ in 0..28 {
            assert_eq!(r.step(0.95), None);
        }
        assert_eq!(r.step(0.95), Some(6));
        for _ in 0..100 {
            assert_eq!(r.step(1.0), None, "6 is the floor");
        }
        for _ in 0..299 {
            assert_eq!(r.step(0.5), None);
        }
        assert_eq!(
            r.step(0.8),
            None,
            "a frame between the lines restarts the count"
        );
        for _ in 0..299 {
            assert_eq!(r.step(0.5), None);
        }
        assert_eq!(r.step(0.5), Some(7));
        for _ in 0..299 {
            assert_eq!(r.step(0.5), None);
        }
        assert_eq!(r.step(0.5), Some(8));
        for _ in 0..1000 {
            assert_eq!(r.step(0.0), None, "8 is the configured radius");
        }
        // A configured radius under the floor is its own floor.
        let mut small = RadiusRule::new(4);
        for _ in 0..100 {
            assert_eq!(small.step(1.0), None);
        }
    }

    /// A deferral extends a streak, an empty sync counts, a sync with none ends it, and a
    /// streak open at the end counts toward the longest.
    #[test]
    fn deferral_streaks_count_empty_syncs_and_an_open_end() {
        let mut d = Deferrals::default();
        assert_eq!(d.note(3, false, [0.91, 0.91, 0.6]), None);
        assert_eq!(d.note(5, true, [0.95, 0.95, 0.5]), None);
        assert_eq!(d.note(0, false, [0.8, 0.8, 0.5]), Some(2));
        assert_eq!(d.note(0, false, [0.8, 0.8, 0.5]), None);
        for _ in 0..3 {
            d.note(1, false, [0.92, 0.92, 0.7]);
        }
        assert_eq!((d.chunks, d.syncs, d.empty), (11, 5, 1));
        assert_eq!(d.low, [0.91, 0.91, 0.5]);
        assert_eq!(d.high, [0.95, 0.95, 0.7]);
        assert_eq!(
            (d.streak, d.longest),
            (3, 3),
            "the open streak is the longest"
        );
    }

    #[test]
    fn residency_parses_two_radii() {
        let r = parse_args_from(argv(&["--residency", "14,22"]))
            .unwrap()
            .residency;
        assert_eq!(
            (r.r_full, r.r_lod, r.hysteresis),
            (14, 22, RESIDENCY.hysteresis)
        );
        assert_eq!(parse_args_from(argv(&[])).unwrap().residency, RESIDENCY);
        for bad in ["14", "0,4", "8,6", "a,b"] {
            assert!(
                parse_args_from(argv(&["--residency", bad])).is_err(),
                "{bad}"
            );
        }
        assert_eq!(
            parse_args_from(argv(&["--mirror-budget", "1024"]))
                .unwrap()
                .mirror_budget,
            Some(1 << 30)
        );
    }

    #[test]
    fn walk_mode_is_refused_while_streaming() {
        assert_eq!(
            parse_args_from(argv(&["--walk"])).err().as_deref(),
            Some("--walk: walk mode needs --level (the coarse grid is not wired to streaming)")
        );
        let a = parse_args_from(argv(&["--walk", "--level"])).expect("--walk --level parses");
        assert!(a.walk && a.level);
        assert_eq!(walk_allowed(true), Err(WALK_NEEDS_LEVEL));
        assert_eq!(walk_allowed(false), Ok(()));
    }

    /// `--save` and `--replay` take a path, need the bounded level, and do not combine.
    #[test]
    fn save_and_replay_need_the_level() {
        for flag in ["--save", "--replay"] {
            assert_eq!(
                parse_args_from(argv(&[flag, "a.log"])).err().as_deref(),
                Some(SAVE_NEEDS_LEVEL)
            );
        }
        let a = parse_args_from(argv(&["--level", "--bench", "1", "--census"]))
            .expect("--level --bench 1 --census parses");
        assert!(a.census && a.bench == 1);
        let a = parse_args_from(argv(&["--level", "--bench", "1", "--save", "a.log"]))
            .expect("--level --bench 1 --save parses");
        assert_eq!(a.save.as_deref(), Some(std::path::Path::new("a.log")));
        assert_eq!(a.bench, 1);
        assert!(parse_args_from(argv(&["--level", "--save", "a", "--replay", "b"])).is_err());
        assert!(parse_args_from(argv(&["--level", "--replay"])).is_err());
    }

    /// `--logic` takes `native`, `wasm` or `wasm=PATH`; without the `wasm` feature `wasm` is
    /// refused with a message naming the feature.
    #[test]
    fn logic_selects_native_or_the_component() {
        let a = parse_args_from(argv(&[])).expect("no arguments parse");
        assert_eq!(a.logic, LogicChoice::Native);
        assert!(parse_args_from(argv(&["--logic", "lua"])).is_err());
        assert!(parse_args_from(argv(&["--logic"])).is_err());
        let wasm = parse_args_from(argv(&["--logic", "wasm=x.wasm"]));
        if cfg!(feature = "wasm") {
            assert_eq!(
                wasm.expect("parses").logic,
                LogicChoice::Wasm("x.wasm".into())
            );
        } else {
            let e = wasm.err().expect("refused without the feature");
            assert!(e.contains("--features wasm"), "{e}");
        }
    }

    /// Every failure and every restart of the logic prints a line; the stop prints once,
    /// naming that the session goes on, and the `Stopped` reports after it print nothing.
    #[test]
    fn each_failure_and_restart_prints_and_the_stop_prints_once() {
        let world = Arc::new(World::new(&cave_materials()));
        let mut net = Net::new(&world, LogicChoice::Native.load().expect("native"), 0);
        assert_eq!(
            net.logic_failure(&LogicError::Fuel { tick: 3 }).as_deref(),
            Some("logic: logic ran out of fuel at tick 3")
        );
        assert_eq!(
            net.logic_failure(&LogicError::Restarted {
                tick: 4,
                after: 3,
                restarts: 1
            })
            .as_deref(),
            Some(
                "logic: logic restarted from its seed at tick 4 after it failed at tick 3 (restart 1)"
            )
        );
        assert_eq!(
            net.logic_failure(&LogicError::Stopped {
                tick: 12,
                since: 11
            })
            .as_deref(),
            Some(
                "logic: logic not run at tick 12: stopped since it failed at tick 11; the session \
                 continues without it"
            )
        );
        assert_eq!(
            net.logic_failure(&LogicError::Stopped {
                tick: 13,
                since: 11
            }),
            None
        );
    }

    /// Every edit is in the log now, so `--vault` combines with `--save` and `--replay` (the
    /// room placed at generation is not an op; `--replay` takes `--vault` to regenerate it).
    #[test]
    fn vault_combines_with_save_and_replay() {
        for flag in ["--save", "--replay"] {
            let a = parse_args_from(argv(&["--level", "--vault", flag, "a.log"]))
                .unwrap_or_else(|e| panic!("--level --vault {flag} parses: {e}"));
            assert!(a.level && a.vault);
        }
    }

    /// `--room` takes metres and implies the room line; `--mic` implies `--audio`;
    /// `--absorption` is checked; a room builds its own world.
    #[test]
    fn room_mic_and_absorption_parse() {
        let a = parse_args_from(argv(&["--room", "12"])).expect("--room 12 parses");
        assert_eq!(a.room, Some(120));
        assert!(a.room_line && !a.audio);
        assert_eq!(a.absorption, CAVE_ACOUSTIC[0]);
        let a = parse_args_from(argv(&["--mic", "--absorption", "0.5", "--room-line"]))
            .expect("parses");
        assert!(a.mic && a.audio && a.room_line);
        assert_eq!(a.absorption, 0.5);
        assert!(parse_args_from(argv(&["--absorption", "1.5"])).is_err());
        assert!(parse_args_from(argv(&["--room", "0.5"])).is_err());
        assert!(parse_args_from(argv(&["--room", "12", "--level"])).is_err());
        assert!(parse_args_from(argv(&["--room", "4", "--walk"])).is_ok());
    }

    #[test]
    fn an_edit_prints_one_line_with_material_names() {
        let r = EditResult {
            changed_from: vec![(1, 1234), (2, 12)],
            touched: vec![
                ChunkCoord([0, 0, 0]),
                ChunkCoord([1, 0, 0]),
                ChunkCoord([1, 1, 0]),
            ],
            aabb: None,
        };
        assert_eq!(
            edit_line("carved", &r, &cave_materials(), 0.214),
            "carved: rock 1234, ore 12 (3 chunks, 0.21 ms)"
        );
        assert_eq!(
            edit_line("filled", &EditResult::default(), &cave_materials(), 0.0),
            "filled: nothing (0 chunks, 0.00 ms)"
        );
    }

    /// The `K` bookkeeping, scripted: an edit updates the kept grid and walkable map to what
    /// a rebuild from the edited world gives, and records the changed-cell count.
    #[test]
    fn an_edit_keeps_the_coarse_grid_and_walkable_map_current() {
        let world = World::generate(
            ChunkExtent::new([-1, -1, -1], [1, 1, 1]),
            &vxe_world::UniformSource(ROCK),
            &cave_materials(),
        );
        let mut c = Coarse::build(&world);
        assert_eq!(c.map.count(), 0, "a solid world has no walkable cell");
        // A 2 m box carved out of the solid: 27 bricks touched, a floor under open cells.
        let r = world
            .apply(&EditOp::box_(
                Fixed3([-8 * FIXED_ONE, -8 * FIXED_ONE, -8 * FIXED_ONE]),
                Fixed3([16 * FIXED_ONE, 16 * FIXED_ONE, 16 * FIXED_ONE]),
                Mode::Carve,
            ))
            .unwrap();
        c.update(&world, &r);
        let rebuilt = CoarseGrid::build(&world);
        let expected = WalkableMap::build(&rebuilt, WALKABLE);
        assert_eq!(c.last.map(|(m, _)| m), Some(27));
        assert!(expected.count() > 0);
        assert_eq!(c.map.cells(), expected.cells());
        assert!(c.line().starts_with(&format!(
            "coarse: {} walkable cells, last edit changed 27 cells in ",
            expected.count()
        )));
    }

    /// One server tick of `net` on `inputs`, sent by its client: the logic's broadcast lines
    /// and every event the client polled.
    fn net_tick(net: &mut Net, inputs: &[DigInput]) -> (Vec<String>, Vec<Event>) {
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
    fn applied(events: &[Event]) -> Vec<OpResult> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Applied { result, .. } => Some(result.clone()),
                _ => None,
            })
            .collect()
    }

    fn native_net(world: &Arc<World>) -> Net {
        Net::new(world, Box::new(flythrough::logic::DigLogic::default()), 1)
    }

    /// `P` headless: from inside a carved 16-voxel box in solid rock, the ray along +x enters
    /// rock at x = 8 and the vault lands there, stamped by the logic through the server; aimed
    /// so the vault crosses the extent's edge it is refused; aimed out of the world it misses.
    #[test]
    fn p_stamps_the_vault_at_the_aim_point() {
        let table = cave_materials();
        let world = Arc::new(World::generate(
            ChunkExtent::new([-1, -1, -1], [1, 1, 1]),
            &vxe_world::UniformSource(ROCK),
            &table,
        ));
        world
            .apply(&EditOp::box_(
                Fixed3([-8 * FIXED_ONE; 3]),
                Fixed3([8 * FIXED_ONE; 3]),
                Mode::Carve,
            ))
            .unwrap();
        let mut net = native_net(&world);
        let stamp = |origin| DigInput::Stamp {
            origin,
            dir: [FIXED_ONE, 0, 0],
            prefab: vault().id(),
        };
        let centre = Fixed3([FIXED_ONE / 2; 3]);
        let (lines, events) = net_tick(&mut net, &[stamp(centre)]);
        assert_eq!(lines, ["op stamped"]);
        let [OpResult::Edit(r)] = &applied(&events)[..] else {
            panic!("{events:?}");
        };
        assert_eq!(r.changed_from, vec![(ROCK, 4096)]);
        assert_eq!(r.touched, vec![ChunkCoord([0, 0, 0])]);
        assert_eq!(
            world.count_in_box([8, 0, 0], [23, 15, 15]).unwrap(),
            vec![(0, 2744), (SHELL, 1352)]
        );
        assert_eq!(
            world.count_in_box([8, 0, 0], [24, 16, 16]).unwrap(),
            vec![(0, 2744), (ROCK, 817), (SHELL, 1352)]
        );
        assert_eq!(
            edit_line(STAMP_VERB, r, &table, 0.5),
            "stamped: rock 4096 (1 chunks, 0.50 ms)"
        );
        assert!(
            matches!(&net.server.ops().entries()[0], vxe_world::LogEntry::Op { op: vxe_world::Op::Stamp { prefab, .. }, .. } if *prefab == vault().id())
        );

        // From inside rock at x = 51 the ray hits that voxel, and a vault there would reach
        // x = 66, past the resident x <= 63: refused, nothing written.
        let past = Fixed3([51 * FIXED_ONE + FIXED_ONE / 2, FIXED_ONE / 2, FIXED_ONE / 2]);
        let before = world.count_in_box([-64; 3], [63; 3]).unwrap();
        let (lines, events) = net_tick(&mut net, &[stamp(past)]);
        assert_eq!(lines, ["op stamped"]);
        assert!(
            events.iter().any(|e| matches!(
                e,
                Event::Refused { error, .. }
                    if error.to_string() == "chunk [1, 0, 0] is not resident"
            )),
            "{events:?}"
        );
        assert_eq!(world.count_in_box([-64; 3], [63; 3]).unwrap(), before);

        let outside = Fixed3([200 * FIXED_ONE, 0, 0]);
        let (lines, _) = net_tick(&mut net, &[stamp(outside)]);
        assert_eq!(lines, ["stamped: no surface within 10 m"]);
    }

    const PILLAR: ([i32; 3], [i32; 3]) = ([32, 32, 8], [40, 40, 40]);

    /// One chunk of air holding a rock floor, a pillar and a 3 x 3-cell ledge on it.
    fn ledge_world() -> World {
        let world = World::generate(
            ChunkExtent::new([0, 0, 0], [1, 1, 1]),
            &vxe_world::UniformSource(vxe_world::AIR),
            &cave_materials(),
        );
        for (lo, hi) in [
            ([0, 0, 0], [64, 64, 8]),
            PILLAR,
            ([24, 24, 40], [48, 48, 48]),
        ] {
            let f = |v: [i32; 3]| Fixed3(v.map(|c| i64::from(c) * FIXED_ONE));
            world
                .apply(&EditOp::box_(f(lo), f(hi), Mode::Fill(ROCK)))
                .unwrap();
        }
        world
    }

    /// A 1 m dig whose origin is inside the pillar, at its axis 16 voxels up: the ray starts
    /// in rock, so the capsule is a sphere there, cutting the pillar through from z = 15 to
    /// z = 32 and leaving its top cell on the ledge.
    fn pillar_cut() -> DigInput {
        DigInput::Dig {
            origin: Fixed3([36 * FIXED_ONE, 36 * FIXED_ONE, 24 * FIXED_ONE]),
            dir: [FIXED_ONE, 0, 0],
            radius: 10 * FIXED_ONE,
        }
    }

    /// The dig's collapse, scripted through the logic: cutting the pillar reports the ledge
    /// with the pillar's top, which falls a cell a tick and lands on the pillar's stump; every
    /// drop is an op in the log, and the server's grid follows the world.
    #[test]
    fn cutting_the_pillar_drops_the_ledge_a_cell_a_tick() {
        let world = Arc::new(ledge_world());
        let table = world.materials().clone();
        let mut net = native_net(&world);
        let (lines, events) = net_tick(&mut net, &[pillar_cut()]);
        assert_eq!(lines, ["op carved", "region 10 1:5024"]);
        assert_eq!(
            unsupported_line(&table, &lines[1]).as_deref(),
            Some("unsupported: 10 cells, {rock: 5024}")
        );
        assert_eq!(applied(&events).len(), 1);
        let mut dropped = Vec::new();
        for tick in 1..=3 {
            let (lines, events) = net_tick(&mut net, &[]);
            let [OpResult::Drop(d)] = &applied(&events)[..] else {
                panic!("tick {tick}: {events:?}");
            };
            dropped.push(d.dropped);
            if tick < 3 {
                assert_eq!(lines, ["op dropped"], "tick {tick}");
            } else {
                assert_eq!(
                    lines,
                    ["op dropped", "landed after 3 ticks, fell 18 voxels"]
                );
            }
        }
        assert_eq!(dropped, [8, 8, 2]);
        // Nothing falls on.
        let (lines, events) = net_tick(&mut net, &[]);
        assert!(
            lines.is_empty() && events.is_empty(),
            "{lines:?} {events:?}"
        );
        assert_eq!(net.server.ops().len(), 4);
        assert_eq!(
            world.count_in_box([24, 24, 22], [47, 47, 29]).unwrap(),
            [(ROCK, 4608)]
        );
        assert_eq!(*net.server.coarse(), CoarseGrid::build(&world));
    }

    /// `R`: the ledge is removed instead, a quarter of it piled as rubble.
    #[test]
    fn with_r_the_ledge_is_removed_leaving_a_quarter_as_rubble() {
        let world = Arc::new(ledge_world());
        let table = world.materials().clone();
        let mut net = native_net(&world);
        let (lines, _) = net_tick(&mut net, &[DigInput::Crumble]);
        assert_eq!(lines, ["collapse: remove unsupported regions with rubble"]);
        let (lines, events) = net_tick(&mut net, &[pillar_cut()]);
        assert_eq!(lines, ["op carved", "region 10 1:5024", "op removed"]);
        let [_, OpResult::Remove(r)] = &applied(&events)[..] else {
            panic!("{events:?}");
        };
        assert_eq!(r.removed, [(ROCK, 5024)]);
        assert_eq!(r.placed, 5024 / 4);
        assert_eq!(
            removed_line(&table, r),
            "removed: {rock: 5024}, 1256 rubble"
        );
        assert_eq!(
            world
                .count_in_box([0, 0, 0], [63, 63, 63])
                .unwrap()
                .into_iter()
                .find(|&(m, _)| m == flythrough::RUBBLE),
            Some((flythrough::RUBBLE, 1256))
        );
        // Nothing falls after a removal.
        let (lines, _) = net_tick(&mut net, &[]);
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(*net.server.coarse(), CoarseGrid::build(&world));
        let (lines, _) = net_tick(&mut net, &[DigInput::Crumble]);
        assert_eq!(lines, ["collapse: drop unsupported regions"]);
    }

    /// The support query after digs from the spawn of the seed-1 level, one along each axis
    /// at the default radius, through the logic, printed. Run with `cargo test --release -p
    /// flythrough -- --ignored --nocapture a_dig_in_the_level_prints_the_query`.
    #[test]
    #[ignore = "a reading on the generated level, run in release"]
    fn a_dig_in_the_level_prints_the_query() {
        let source = CaveSource::new(1, ChunkExtent::new([0, 0, 0], LEVEL_CHUNKS));
        let world = Arc::new(World::generate(source.extent, &source, &cave_materials()));
        let mut net = native_net(&world);
        let origin = vxe_world::centre_of_voxel(source.spawn());
        let f = FIXED_ONE;
        for dir in [
            [f, 0, 0],
            [-f, 0, 0],
            [0, f, 0],
            [0, -f, 0],
            [0, 0, f],
            [0, 0, -f],
        ] {
            let t = now();
            let (lines, _) = net_tick(
                &mut net,
                &[DigInput::Dig {
                    origin,
                    dir,
                    radius: DIG_RADIUS,
                }],
            );
            println!("dig {dir:?}: tick {:.3} ms; {lines:?}", ms_since(t));
        }
    }

    #[test]
    fn rock_detail_parses_scale_and_strength() {
        assert_eq!(parse_rock_detail("8,0.6"), Ok([8.0, 0.6]));
        assert_eq!(parse_rock_detail("2.5, 0"), Ok([2.5, 0.0]));
        assert!(parse_rock_detail("4").is_err());
        assert!(parse_rock_detail("0,0.3").is_err());
        assert!(parse_rock_detail("4,1.5").is_err());
        assert!(parse_rock_detail("4,x").is_err());
    }

    /// No flag starts in contour with detail on and rock's detail; `--look plain` starts in
    /// face mode with neither, and `--detail` turns detail on there.
    #[test]
    fn the_default_look_is_contour_with_detail_and_plain_is_face() {
        let a = parse_args_from(argv(&[])).unwrap();
        assert_eq!(
            look_line(&a.start_look(), a.rock_detail()),
            "look contour, detail on (rock scale 4, strength 0.1), view shaded"
        );
        assert_eq!(flythrough::ROCK_DETAIL, [4.0, 0.1]);
        let a = parse_args_from(argv(&["--look", "face"])).unwrap();
        assert_eq!((a.look, a.detail), (Some(NormalMode::Face), true));
        let a = parse_args_from(argv(&["--look", "plain"])).unwrap();
        assert_eq!((a.look, a.detail, a.rock_detail()), (None, false, None));
        assert_eq!(
            look_line(&a.start_look(), a.rock_detail()),
            "look face, detail off, view shaded"
        );
        let a = parse_args_from(argv(&["--look", "plain", "--detail"])).unwrap();
        assert!(a.detail);
        assert!(parse_args_from(argv(&["--look", "plain", "--rock-detail", "4,0.2"])).is_err());
        assert_eq!(
            parse_args_from(argv(&["--attrib-quant", "none"]))
                .err()
                .as_deref(),
            Some("unknown argument --attrib-quant")
        );
    }

    #[test]
    fn the_look_line_names_rock_detail_under_look() {
        let look = LookParams {
            normal_mode: NormalMode::Face,
            detail: true,
            ..LookParams::default()
        };
        assert_eq!(
            look_line(&look, Some([4.0, 0.3])),
            "look face, detail on (rock scale 4, strength 0.3), view shaded"
        );
        assert_eq!(look_line(&look, None), "look face, detail on, view shaded");
    }

    /// The cost of one `World::stats` call on the streamed cave at its spawn after full
    /// residency, median of 10, which sets [`HUD_STATS_EVERY`]. Run by hand in release:
    /// `cargo test --release -p flythrough --bin flythrough -- --ignored world_stats_call`.
    #[test]
    #[ignore = "a timing reading, run by hand in release"]
    fn world_stats_call_on_the_streamed_world_at_full_residency() {
        let source = CaveSource::unbounded(1);
        let spawn = source.spawn();
        let world = World::new(&cave_materials());
        let workers = std::thread::available_parallelism()
            .map_or(1, usize::from)
            .saturating_sub(1)
            .max(1);
        let mut streamer = Streamer::new(
            Arc::new(source),
            StreamConfig {
                residency: RESIDENCY,
                workers,
                persist: PersistConfig::default(),
            },
        );
        loop {
            streamer.update(&world, &[spawn]);
            let _ = streamer.poll(&world);
            let st = streamer.stats();
            if st.ticks > 1 && st.in_flight == 0 && st.queued == 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let mut ms: Vec<f64> = (0..10)
            .map(|_| {
                let t = now();
                std::hint::black_box(world.stats());
                ms_since(t)
            })
            .collect();
        let s = world.stats();
        let st = streamer.stats();
        println!(
            "world stats call: median {:.3} ms of 10 (min {:.3}, max {:.3}); {} chunks, {} full bricks; streamer {} full, {} lod",
            median(&mut ms),
            ms[0],
            ms[9],
            s.chunks,
            s.full_bricks,
            st.resident_full,
            st.resident_lod
        );
    }

    #[test]
    fn the_first_frame_waits_for_33_chunks() {
        assert_eq!(ball(NEAR_D2).len(), 33);
    }
}
