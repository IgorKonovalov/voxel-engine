//! The engine's testbed (milestone 1 in `docs/roadmap.md`): a window showing the ray-marched
//! cave lit by a headlamp. Fly, dig, and read the per-material counts each edit returns.
//!
//! By default the cave is unbounded and streams around the camera (ADR-0003): full detail
//! within `r_full` chunks, mips out to `r_lod`. `--level` generates the bounded level instead.
//!
//! `flythrough [--seed N] [--level] [--vault] [--castle] [--castle-out PATH]
//! [--castle-shot gate|nave|window|dungeon] [--walk] [--render WxH] [--frames N] [--bench N]
//! [--autopilot] [--census] [--sync-budget MIB] [--mirror-budget MIB] [--residency R_FULL,R_LOD]
//! [--headless] [--look PRESET] [--rock-detail SCALE,STRENGTH] [--plane-fit FIT]
//! [--plane-offset OFFSET] [--face-mix K] [--detail-fine W] [--detail-after on|off] [--view VIEW]
//! [--detail]
//! [--translucent-layers N] [--hud] [--save PATH] [--replay PATH] [--logic native|wasm[=PATH]]
//! [--audio] [--audio-debug] [--room METRES] [--room-line] [--absorption A] [--mic]
//! [--echo-db DB] [--tail-db DB] [--predelay-ms MS] [--tail-glide] [--direct-delay]
//! [--flat-mix] [--pad on|off] [--emissive-ore on|off] [--lamp on|off] [--lamp-toggle N]
//! [--fade-reading]
//! [--shadow-walk full|coarse] [--filter on|off] [--light-radius N] [--sampler list|reuse]
//! [--history edits|gradient] [--lighting full|half] [--emissive-history N]
//! [--indirect on|off] [--probe-spacing N] [--probe-rays N] [--probe-slices N]
//! [--internal-scale 1|1.5|2] [--jitter on|off] [--sharpen S] [--upscaler off|own|fsr2]`.
//! `--render` fixes the frame's output resolution (the window scales it); `--internal-scale`
//! renders at the output size divided by it on each edge (default 1: no upscaler); `--jitter
//! on` moves every primary ray by the Halton (2, 3) sub-pixel jitter of the frame (default off:
//! through its pixel's centre), which the upscaler accumulates; the upscaler runs when the
//! scale is above 1 or the jitter is on (`--internal-scale 1.5 --jitter on` is the performance
//! option); `--sharpen S` sets the upscaler's RCAS sharpening, 0 to 1 (default 0.5, 0 off);
//! `--upscaler off|own|fsr2` picks the upscaler: `off` is Plan 0024's frame (scale 1, the
//! jitter off), `own` the upscaler of ADR-0024 at `--internal-scale` (the jitter on unless
//! `--jitter off`), `fsr2` FSR 2 (ADR-0034) at `--internal-scale` with its own jitter; without
//! it the upscaler is `own` when the scale is above 1 or the jitter is on, else `off`. In a
//! window `F10` cycles off, own and fsr2, printing the mode and showing it in a banner;
//! `--frames` exits after N frames, printing the `F1` readings over the last ones and the
//! stats HUD's rows; `--hud` starts with the stats HUD shown (`F2` toggles it); `--bench` sends N `B` bench inputs on the first
//! frame; `--autopilot` digs, flies 300 m out and back and prints the streaming readings (see
//! [`Autopilot`]); `--cutaway auto|on|off` sets the cutaway view's start state (default `auto`:
//! on while the camera's voxel is neither air nor translucent, so a camera in rock or outside
//! the world sees each cavern's far wall through the rock; `X` cycles the three, and the
//! autopilot never changes it); `--census` prints the world's brick census (`World::brick_census`) on exit,
//! after any bench; `--sync-budget` sets the mirror's upload bytes per frame; `--mirror-budget`
//! sets the VRAM the mirror reserves for its pool, masks and records at startup (ADR-0013;
//! default 3072 MiB streamed, and on the level what its bricks, the dig headroom and its chunks
//! need, with records for the residency's ball when streaming), and while streaming the full
//! radius follows the mirror's pressure (the fuller of its pool slots and records): a chunk down after 30 frames above 0.9, a chunk up after 300 frames below
//! 0.7, between 6 and the configured radius, each change printed and on the HUD;
//! `--residency` sets the streamed radii (default `flythrough::RESIDENCY`, hysteresis 1);
//! `--headless` runs the
//! same frames on an offscreen target with no window; `--walk` (with `--level`) starts in walk
//! mode, the body spawned in the spawn pocket. The fly-through starts in `contour-sharp`
//! (`LookPreset::DEFAULT`) with detail on,
//! the mirror holding contour attributes and rock its procedural detail (`--rock-detail` sets
//! rock's feature scale in voxels and strength, default `flythrough::ROCK_DETAIL`), and prints
//! that
//! look first; `--look` picks the starting preset (`flythrough::LookPreset`: `face`, `contour`,
//! `contour-sharp`, `contour-grain`, `trilinear8`, `gradient27`), a normal mode with its plane
//! fit, face mix, detail after lighting and rock's detail pair and fine weight, each part
//! overridable by `--plane-fit`, `--plane-offset`, `--face-mix`, `--detail-after`,
//! `--detail-fine` and `--rock-detail`; and `--look plain` starts in face mode without contour
//! attributes or rock detail, detail off (`N` then skips the contour presets, `--detail` turns
//! detail on), the look of Plan 0002's and Plan 0017's face readings. With contour attributes the autopilot's report adds the attribute
//! pass's work list, CPU and GPU time over the flight. `--view`
//! starts in a view (`shaded`, `normals`, `hitmask`, `steps`, `albedo`; with `--frames`, `steps`
//! prints the last frame's mean and p99 steps per ray on exit);
//! `--translucent-layers` sets how many translucent voxels are composited in front of the opaque
//! hit (default 4, 0 ignores them). `--emissive-ore off` keeps the ore dark (by default it
//! glows, `flythrough::ORE_EMISSION`, and lights the cave through the light pass's emissive
//! samples); `M` pulses its emission scale (the probe volume sees the ore at the pulse's
//! average); `--lamp off` turns the headlamp off; `--fade-reading` (headless) digs five times
//! along the camera ray, lets the light settle with the lamp off and then on, switches it off
//! and prints how the screen centre's light fades, by source (see [`FadeSample`]);
//! `--shadow-walk coarse` sends every shadow ray through the coarse walk (2^3 blocks of the
//! masks; with `--view steps` and `--frames`, the exit prints the lamp shadow rays' cost too);
//! `--filter off` shows the light pass's raw irradiance, unaccumulated and unfiltered;
//! `--light-radius N` sets the chunks around the camera whose emitters light the frame
//! (default 4); `--sampler reuse` adds last frame's reservoirs (ReSTIR) to the candidates;
//! `--history gradient` also shortens history where A-SVGF's gradient sees the light change;
//! `--lighting half` runs the lighting passes at half resolution on each edge;
//! `--emissive-history N` sets the frames static emissive light accumulates over (default 32;
//! the lamp's light keeps a short history); `--indirect off` turns the probe volume's indirect
//! light off (on by default, ADR-0023); `--probe-spacing N` sets the voxels between its probes
//! (default 16, 1.6 m; 4 to 64), `--probe-rays N` the rays each updated probe traces (default
//! 64, 1 to 64) and `--probe-slices N` the share it refreshes a frame, one in N (8, the
//! default, 16 or 32). In a window
//! `F3` toggles the shadow walk, `F4` the sampler, `F5` the history killer, `F6` the lighting
//! resolution, `F7` the lamp and `F8` indirect light, each printing the lighting modes; with the stats HUD shown,
//! two rows read every frame pass's GPU time and the light samples with those modes.
//! `N` cycles the look presets and `V` toggles detail, each naming the look on the banner, and
//! a fit change refits the mirror's bricks under the attribute cap (`refitting` on the banner). `--vault` places a 16-voxel room with an indestructible
//! shell 10 m along +x from the spawn, written over the cave as it generates (on the level, clamped
//! inside the extent).
//!
//! `--castle` (the streamed cave only; not with `--level`, `--vault` or `--room`) builds a
//! gothic castle at startup as a multi-model `.vox` scene (`flythrough::castle`), parses it
//! back, maps its palette to the castle's material rows and stamps it at generation into a
//! hall of air over a rock plinth, the gate facing the spawn. It prints each stage's time and
//! the prefabs' bytes, and the banner names it and the shot. `--castle-out PATH` (implies
//! `--castle`) also writes the generated `.vox` to `PATH`. `--castle-shot NAME` (implies
//! `--castle`) starts at one of `flythrough::castle::CastleShot`'s poses instead of the gate
//! (the spawn), and `I` moves the camera to the next shot, printing and showing its name.
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
//! A gamepad (`--pad on`, the default in a window; never headless or under the autopilot) flies
//! as the keys and mouse do: the left stick moves at a speed set by its deflection, the right
//! stick looks, `L1`/`R1` sink and rise, `R3` latches boost, and walking `South` jumps; `R2`
//! digs, `L2` fills, the D-pad's up and down step the dig radius, `North` toggles walking,
//! `West` cycles the cutaway, `East` knocks and `Start` toggles the stats HUD. A machine where `gilrs` cannot start runs
//! without a pad. The startup lines name the pad and its power or print `pad: none`; a pad
//! plugged in later becomes the active one, and each connect and disconnect prints a line. The
//! HUD shows the active pad's row.
//!
//! Collapse (bounded level, ADR-0008): after every dig the logic asks the server's coarse grid
//! which regions lost their support, and the fly-through prints each as `unsupported: N cells,
//! {material: count}`. Each region falls 8 voxels a tick, one tick a frame, through and
//! crushing the cells too thin to hold it (ADR-0016), until it lands (`landed after N ticks,
//! fell M voxels`, then `, crushed K voxels, P rubble` when it crushed any, piled on top of
//! it). A region that cannot move prints `wedged: N cells, {material: count}, cannot fall`,
//! and a refused drop `drop refused: ...`. `R` toggles removing it with rubble (a quarter of
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
//! The balance (`vxe_audio::MixSettings`) is live. The mixer sets the echo and tail levels by
//! the room's mean free path through its default room curves; `[` and `]` step a trim on the
//! echoes by 3 dB, `-` and `=` one on the tail (each -24 to +12 dB), `9` and `0` the tail's
//! pre-delay by 10 ms (0 to 200 ms), `O` switches the tail's lines between handing over and
//! gliding, and `U` switches the direct path's propagation delay. Each prints a line
//! `mix: echoes <trim> dB (room <curve> dB at mfp <m> m), tail <trim> dB (room <curve> dB),
//! pre-delay .. ms, tail lines .., direct delay ..`, which `F1`, the start (with `--audio` or
//! `--room-line`) and every room that moves a curve by 0.5 dB or more print too.
//! `--echo-db`, `--tail-db`, `--predelay-ms`, `--tail-glide` and `--direct-delay` set the same
//! at the start. `--flat-mix` sets both curves flat at 0 dB (the line reads `room flat`) and
//! the trims' ranges to -12 to +30 dB for the echoes and -30 to +6 dB for the tail, the
//! absolute levels of a balance picked before the curves.
//!
//! `--room METRES` runs in a closed cube of air of that edge inside 2 m of rock, spawned at its
//! centre, and implies `--room-line`: a line `room: V .. S .. mfp .. alpha .. T60 .. energy ..`
//! whenever the listener's room fan completes (`F1` and `--audio-debug` print it too). Without
//! `--audio`, `--room-line` runs the room fan with no output device: its block clock never
//! advances, so a fan completes 8 frames after the start or after a 1 m move.

mod fly;
mod pad;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

use fly::{CutawayMode, Fly, Walk};
use flythrough::castle;
use flythrough::cave::{CaveSource, LEVEL_CHUNKS, ROCK, RoomSource, VAULT_EDGE, vault};
use flythrough::hud::{self, Cadence, EditLine, FrameStats, HudStats, WorldSample};
use flythrough::logic::{BENCH_CARVES, BENCH_VERB, DIG_VERB, DigInput, FILL_VERB, REACH};
use flythrough::logic::{
    DROP_VERB, PILE_VERB, REMOVE_VERB, STAMP_VERB, parse_region_line, parse_wedged_line,
};
use flythrough::{
    LogicChoice, LookPreset, RESIDENCY, STREAM_MIRROR_BUDGET, ScrollSteps, cave_materials,
    chunk_ram, chunks_hash, level_mirror_budget, level_pool_units, mips_ram, percentile,
};
use pad::Pad;
use vxe_app::{App, Axis, Flow, Gpu, Input, KeyCode, MouseButton, PadButton, ShellConfig};
use vxe_audio::{
    Acoustics, BlockClock, Command, Hrtf, MixSettings, Mixer, Output, PlayParams, ReverbSettings,
    RoomEstimate, RoomTargets, Sound, TailLines, VoiceId,
};
use vxe_net::{Client, DEFAULT_SUPPORT, Event, Logic, LogicError, MemChannel, Server};
use vxe_render::UpscalerKind;
use vxe_render::glam::Vec3;
use vxe_render::gpu_world;
use vxe_render::readback::{self, StepHistogram};
use vxe_render::{
    AttrWorkStats, BELT_CHUNK_BYTES, Camera, FRAME_PASSES, Frame, GpuWorld, HistoryKill, HudColour,
    HudOverlay, LightParams, Lighting, LightingResolution, LodParams, LookParams, MarchTimer,
    MirrorConfig, NormalMode, PassTimer, PlaneFit, PlaneOffset, Presenter, RenderScale, Sampler,
    ShadowWalk, SyncStats, ViewMode, Window, wgpu,
};
use vxe_world::hash::mix64;
use vxe_world::{
    CellChange, ChunkCoord, ChunkExtent, CoarseGrid, EditOp, EditResult, FIXED_ONE, Fixed3,
    LogFile, MaterialId, MaterialTable, Mode, OpResult, PersistConfig, Residency, StreamConfig,
    Streamer, Tier, VoxelCoord, WalkableMap, WalkableParams, World,
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

/// A trigger presses (`R2` digs, `L2` fills) once pulled past this, and lets go under
/// [`TRIGGER_RELEASE`], so a trigger resting near a threshold does not repeat.
const TRIGGER_PRESS: f32 = 0.5;
const TRIGGER_RELEASE: f32 = 0.3;

/// Whether a trigger at `pull` presses this frame, with `held` its state since the last.
fn trigger_pressed(held: &mut bool, pull: f32) -> bool {
    if *held {
        *held = pull >= TRIGGER_RELEASE;
        false
    } else {
        *held = pull >= TRIGGER_PRESS;
        *held
    }
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
    /// `--adapter SUBSTRING`: a headless run takes the first adapter whose name contains it.
    adapter: Option<String>,
    walk: bool,
    /// `--vault`: the cave generates with the vault room ahead of the spawn.
    vault: bool,
    /// `--castle`: the streamed cave generates with the castle in its hall.
    castle: bool,
    /// `--castle-out PATH`: where to write the generated `.vox`.
    castle_out: Option<std::path::PathBuf>,
    /// `--castle-shot NAME`: the castle pose the camera starts at.
    castle_shot: castle::CastleShot,
    sync_budget: u64,
    /// `--mirror-budget`: the mirror's VRAM reservation in bytes.
    mirror_budget: Option<u64>,
    /// `--residency`: the streamed full and LOD radii in chunks, hysteresis as
    /// [`RESIDENCY`]'s.
    residency: Residency,
    audio: bool,
    /// `--audio-debug`: print the callback's readings and record its output.
    audio_debug: bool,
    /// `--look`: the starting preset with any part flag applied, with contour attributes and
    /// rock detail; `None` for `--look plain`, face mode without either. `contour-sharp` by
    /// default.
    look: Option<LookPreset>,
    /// Start with procedural detail on: by default, except under `--look plain`, where
    /// `--detail` turns it on.
    detail: bool,
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
    /// `--echo-db`, `--tail-db`, `--predelay-ms`, `--tail-glide`, `--direct-delay`.
    mix: Mix,
    /// `--cutaway`: the cutaway view's start state.
    cutaway: CutawayMode,
    /// `--pad on|off`: read a gamepad in a window (on by default); never headless or under
    /// the autopilot.
    pad: bool,
    /// `--emissive-ore on|off`: the ore row emits `flythrough::ORE_EMISSION` (on by default).
    emissive_ore: bool,
    /// `--lamp on|off`: the headlamp lights the frame (on by default).
    lamp: bool,
    /// `--lamp-toggle N`: switch the lamp every `N` frames, as `F7` does, timing the probe
    /// passes inside and outside the light-change windows the switches open.
    lamp_toggle: Option<u32>,
    /// `--fade-reading`: dig into the rock ahead, settle the ore light with the lamp off,
    /// then the lamp light with it on, switch it off and print how each light source at the
    /// screen centre fades (see [`FadeSample`]).
    fade_reading: bool,
    /// `--shadow-walk full|coarse`: the walk shadow rays take.
    shadow_walk: ShadowWalk,
    /// `--filter on|off`: the a-trous filter runs (on by default).
    filter: bool,
    /// `--light-radius N`: chunks around the camera whose emitters light the frame.
    light_radius: i32,
    /// `--sampler list|reuse`: the light pass's emissive sampler.
    sampler: Sampler,
    /// `--history edits|gradient`: what kills the temporal history.
    history: HistoryKill,
    /// `--lighting full|half`: the lighting passes' resolution.
    lighting: LightingResolution,
    /// `--emissive-history N`: frames the emissive light's history blends over at least.
    emissive_history: u32,
    /// `--indirect on|off`: indirect light from the probe volume (on by default).
    indirect: bool,
    /// `--probe-spacing N`: voxels between the probe volume's probes (default 16).
    probe_spacing: u32,
    /// `--probe-rays N`: rays each updated probe traces (default 64).
    probe_rays: u32,
    /// `--probe-slices N`: the volume refreshes one probe in N a frame (default 8).
    probe_slices: u32,
    /// `--internal-scale 1|1.5|2`: the output size over the internal size, on each edge.
    internal_scale: RenderScale,
    /// `--jitter on|off`: the primary rays move by the frame's sub-pixel jitter (on by
    /// default).
    jitter: bool,
    /// `--sharpen S`: the upscaler's sharpening strength, 0 to 1.
    sharpen: f32,
    /// `--jitter` was given.
    jitter_set: bool,
    /// `--upscaler off|own|fsr2`, when given.
    upscaler: Option<hud::UpscalerMode>,
}

/// The probe volume's bytes inside `Frame::frame_bytes`, by part.
fn probe_bytes_line(p: &vxe_render::ProbeVolume) -> String {
    format!(
        "probe volume {} B in frame_bytes (atlases {} B, state {} B, update list, rays and uniform {} B), spacing {} voxels",
        p.bytes(),
        p.atlas_bytes(),
        p.state_bytes(),
        p.bytes() - p.atlas_bytes() - p.state_bytes(),
        p.spacing()
    )
}

/// The internal scale by default (`--internal-scale`): the output size, with the jitter off
/// the frame runs no upscaler. `--internal-scale 1.5 --jitter on` is the performance option:
/// 720p for 1080p, about half the frame's GPU time on both adapters of `nfr.md` §1.
const DEFAULT_SCALE: RenderScale = RenderScale::One;

/// The jitter by default (`--jitter`): off.
const DEFAULT_JITTER: bool = false;

/// The upscaler's sharpening strength by default (`--sharpen`).
const DEFAULT_SHARPEN: f32 = 0.5;

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
    /// Whether the run reads a gamepad: `--pad on` in a window, without the autopilot.
    fn wants_pad(&self) -> bool {
        self.pad && !self.headless && !self.autopilot
    }

    /// The upscaler the fly-through starts with: `--upscaler`, else `own` when the scale is
    /// above 1 or the jitter is on, else `off`.
    fn start_upscaler(&self) -> hud::UpscalerMode {
        self.upscaler
            .unwrap_or(if self.internal_scale != RenderScale::One || self.jitter {
                hud::UpscalerMode::Own
            } else {
                hud::UpscalerMode::Off
            })
    }

    /// The frame's settings in upscaler `mode`: the upscaler, the internal scale and the
    /// jitter. `off` renders at scale 1 with the jitter off; `own` takes `--jitter`, on when
    /// the mode was chosen (`--upscaler` or `F10`) and it was not given; FSR 2 jitters by its
    /// own sequence.
    fn upscaler_settings(&self, mode: hud::UpscalerMode) -> (UpscalerKind, RenderScale, bool) {
        match mode {
            hud::UpscalerMode::Off => (UpscalerKind::Off, RenderScale::One, false),
            hud::UpscalerMode::Own => {
                let chosen = self.upscaler.is_some() || self.start_upscaler() != mode;
                let jitter = if self.jitter_set || !chosen {
                    self.jitter
                } else {
                    true
                };
                (
                    UpscalerKind::Own(vxe_render::marcher::Upscaler::default()),
                    self.internal_scale,
                    jitter,
                )
            }
            hud::UpscalerMode::Fsr2 => (
                UpscalerKind::Fsr2(vxe_render::Fsr2Settings {
                    sharpness: self.sharpen,
                    ..vxe_render::Fsr2Settings::default()
                }),
                self.internal_scale,
                false,
            ),
        }
    }

    /// The lighting the fly-through starts in, from its flags.
    fn start_lighting(&self) -> Lighting {
        Lighting {
            lamp: self.lamp,
            shadow_walk: self.shadow_walk,
            filter: self.filter,
            sampler: self.sampler,
            history: self.history,
            emissive_history: self.emissive_history,
            indirect: self.indirect,
            probe_rays: self.probe_rays,
            probe_slices: self.probe_slices,
            ..Lighting::default()
        }
    }

    /// The look the fly-through starts in.
    fn start_look(&self) -> LookParams {
        LookParams {
            normal_mode: self
                .look
                .map_or_else(NormalMode::default, |p| p.normal_mode),
            detail: self.detail,
            view: self.view,
            translucent_layers: self.translucent_layers,
            cutaway: false,
            tuning: self.look.map_or_else(Default::default, |p| p.tuning),
        }
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
        adapter: None,
        walk: false,
        vault: false,
        castle: false,
        castle_out: None,
        castle_shot: castle::CastleShot::Gate,
        sync_budget: SYNC_BUDGET_MIB << 20,
        mirror_budget: None,
        residency: RESIDENCY,
        audio: false,
        audio_debug: false,
        look: Some(LookPreset::DEFAULT),
        detail: false,
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
        mix: Mix::default(),
        cutaway: CutawayMode::Auto,
        pad: true,
        emissive_ore: true,
        lamp: true,
        lamp_toggle: None,
        fade_reading: false,
        indirect: true,
        probe_spacing: vxe_render::probes::DEFAULT_PROBE_SPACING,
        probe_rays: vxe_render::probes::RAYS_PER_PROBE,
        probe_slices: vxe_render::probes::SLICES,
        shadow_walk: ShadowWalk::default(),
        filter: true,
        light_radius: vxe_render::gpu_world::DEFAULT_LIGHT_RADIUS,
        sampler: Sampler::default(),
        history: HistoryKill::default(),
        lighting: LightingResolution::default(),
        emissive_history: vxe_render::DEFAULT_EMISSIVE_HISTORY,
        internal_scale: DEFAULT_SCALE,
        jitter: DEFAULT_JITTER,
        sharpen: DEFAULT_SHARPEN,
        jitter_set: false,
        upscaler: None,
    };
    let mut detail_flag = false;
    // The part flags, applied to the `--look` preset after every flag is read.
    let mut rock_detail = None;
    let mut plane_fit = None;
    let mut plane_offset = None;
    let mut face_mix = None;
    let mut detail_fine = None;
    let mut detail_after = None;
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
                    Some(LookPreset::from_name(&v).ok_or(format!(
                        "--look takes plain, {}, not {v}",
                        LookPreset::names()
                    ))?)
                };
            }
            "--rock-detail" => rock_detail = Some(parse_rock_detail(&value()?)?),
            "--plane-fit" => {
                let v = value()?;
                plane_fit = Some(match v.as_str() {
                    "uniform27" => PlaneFit::Uniform27,
                    "weighted27" => PlaneFit::Weighted27,
                    _ => {
                        return Err(format!(
                            "--plane-fit takes uniform27 or weighted27, not {v}"
                        ));
                    }
                });
            }
            "--plane-offset" => {
                let v = value()?;
                plane_offset = Some(match v.as_str() {
                    "fraction" => PlaneOffset::Fraction,
                    "volume" => PlaneOffset::VolumeMatched,
                    _ => return Err(format!("--plane-offset takes fraction or volume, not {v}")),
                });
            }
            "--face-mix" => face_mix = Some(parse_unit("--face-mix", &value()?, 0.5)?),
            "--detail-fine" => detail_fine = Some(parse_unit("--detail-fine", &value()?, 1.0)?),
            "--detail-after" => detail_after = Some(parse_on_off("--detail-after", &value()?)?),
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
            "--cutaway" => {
                let v = value()?;
                args.cutaway = CutawayMode::from_name(&v)
                    .ok_or(format!("--cutaway takes auto, on or off, not {v}"))?;
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
            "--echo-db" => args.mix.echo_db = Mix::parse(flag.as_str(), &value()?)?,
            "--tail-db" => args.mix.tail_db = Mix::parse(flag.as_str(), &value()?)?,
            "--predelay-ms" => args.mix.predelay_ms = Mix::parse(flag.as_str(), &value()?)?,
            "--tail-glide" => args.mix.glide = true,
            "--flat-mix" => args.mix.flat = true,
            "--direct-delay" => args.mix.direct_delay = true,
            "--mic" => {
                args.mic = true;
                args.audio = true;
            }
            "--pad" => {
                args.pad = match value()?.as_str() {
                    "on" => true,
                    "off" => false,
                    v => return Err(format!("--pad takes on or off, not {v}")),
                }
            }
            "--shadow-walk" => {
                let v = value()?;
                args.shadow_walk = ShadowWalk::from_name(&v)
                    .ok_or(format!("--shadow-walk takes full or coarse, not {v}"))?;
            }
            "--probe-rays" => {
                let v = value()?;
                args.probe_rays = v
                    .parse()
                    .ok()
                    .filter(|n| (1..=vxe_render::probes::RAYS_PER_PROBE).contains(n))
                    .ok_or(format!("--probe-rays takes 1 to 64 rays, not {v}"))?;
            }
            "--probe-slices" => {
                let v = value()?;
                args.probe_slices = v
                    .parse()
                    .ok()
                    .filter(|n| [8, 16, 32].contains(n))
                    .ok_or(format!("--probe-slices takes 8, 16 or 32, not {v}"))?;
            }
            "--probe-spacing" => {
                let v = value()?;
                args.probe_spacing = v
                    .parse()
                    .ok()
                    .filter(|n| (4..=64).contains(n))
                    .ok_or(format!("--probe-spacing takes 4 to 64 voxels, not {v}"))?;
            }
            "--emissive-history" => {
                let v = value()?;
                args.emissive_history =
                    v.parse()
                        .ok()
                        .filter(|n| (1..=4096).contains(n))
                        .ok_or(format!(
                            "--emissive-history takes 1 to 4096 frames, not {v}"
                        ))?;
            }
            "--internal-scale" => {
                let v = value()?;
                args.internal_scale = RenderScale::from_name(&v)
                    .ok_or(format!("--internal-scale takes 1, 1.5 or 2, not {v}"))?;
            }
            "--sharpen" => {
                let v = value()?;
                args.sharpen = v
                    .parse()
                    .ok()
                    .filter(|s: &f32| (0.0..=1.0).contains(s))
                    .ok_or(format!("--sharpen takes 0 to 1, not {v}"))?;
            }
            "--jitter" => {
                args.jitter = match value()?.as_str() {
                    "on" => true,
                    "off" => false,
                    v => return Err(format!("--jitter takes on or off, not {v}")),
                };
                args.jitter_set = true;
            }
            "--upscaler" => {
                let v = value()?;
                args.upscaler = Some(
                    hud::UpscalerMode::from_name(&v)
                        .ok_or(format!("--upscaler takes off, own or fsr2, not {v}"))?,
                );
            }
            "--lighting" => {
                let v = value()?;
                args.lighting = LightingResolution::from_name(&v)
                    .ok_or(format!("--lighting takes full or half, not {v}"))?;
            }
            "--history" => {
                let v = value()?;
                args.history = HistoryKill::from_name(&v)
                    .ok_or(format!("--history takes edits or gradient, not {v}"))?;
            }
            "--sampler" => {
                let v = value()?;
                args.sampler = Sampler::from_name(&v)
                    .ok_or(format!("--sampler takes list or reuse, not {v}"))?;
            }
            "--light-radius" => {
                let v = value()?;
                args.light_radius = v
                    .parse()
                    .ok()
                    .filter(|r| (0..=64).contains(r))
                    .ok_or(format!("--light-radius takes 0 to 64 chunks, not {v}"))?;
            }
            "--filter" => {
                args.filter = match value()?.as_str() {
                    "on" => true,
                    "off" => false,
                    v => return Err(format!("--filter takes on or off, not {v}")),
                }
            }
            "--lamp-toggle" => {
                let v = value()?;
                args.lamp_toggle = match v.parse::<u32>() {
                    Ok(n) if n > 0 => Some(n),
                    _ => return Err(format!("--lamp-toggle takes a frame count, not {v}")),
                }
            }
            "--fade-reading" => args.fade_reading = true,
            "--lamp" => {
                args.lamp = match value()?.as_str() {
                    "on" => true,
                    "off" => false,
                    v => return Err(format!("--lamp takes on or off, not {v}")),
                }
            }
            "--indirect" => {
                args.indirect = match value()?.as_str() {
                    "on" => true,
                    "off" => false,
                    v => return Err(format!("--indirect takes on or off, not {v}")),
                }
            }
            "--emissive-ore" => {
                args.emissive_ore = match value()?.as_str() {
                    "on" => true,
                    "off" => false,
                    v => return Err(format!("--emissive-ore takes on or off, not {v}")),
                }
            }
            "--level" => args.level = true,
            "--detail" => detail_flag = true,
            "--autopilot" => args.autopilot = true,
            "--headless" => args.headless = true,
            "--adapter" => args.adapter = Some(value()?),
            "--walk" => args.walk = true,
            "--hud" => args.hud = true,
            "--census" => args.census = true,
            "--vault" => args.vault = true,
            "--castle" => args.castle = true,
            "--castle-out" => {
                args.castle = true;
                args.castle_out = Some(value()?.into());
            }
            "--castle-shot" => {
                args.castle = true;
                let v = value()?;
                args.castle_shot = castle::CastleShot::parse(&v).ok_or_else(|| {
                    format!("--castle-shot takes gate, nave, window or dungeon, not {v}")
                })?;
            }
            "--audio" => args.audio = true,
            "--audio-debug" => {
                args.audio = true;
                args.audio_debug = true;
            }
            _ => return Err(format!("unknown argument {flag}")),
        }
    }
    args.detail = detail_flag || args.look.is_some();
    if let Some(p) = args.look.as_mut() {
        p.rock_detail = rock_detail.unwrap_or(p.rock_detail);
        p.tuning.fit.fit = plane_fit.unwrap_or(p.tuning.fit.fit);
        p.tuning.fit.offset = plane_offset.unwrap_or(p.tuning.fit.offset);
        p.tuning.face_mix = face_mix.unwrap_or(p.tuning.face_mix);
        p.detail_fine = detail_fine.unwrap_or(p.detail_fine);
        p.tuning.detail_after_lighting = detail_after.unwrap_or(p.tuning.detail_after_lighting);
    } else if rock_detail.is_some() {
        return Err("--rock-detail does not apply under --look plain".to_owned());
    } else if plane_fit.is_some()
        || plane_offset.is_some()
        || face_mix.is_some()
        || detail_fine.is_some()
        || detail_after.is_some()
    {
        return Err(
            "--plane-fit, --plane-offset, --face-mix, --detail-fine and --detail-after do not \
             apply under --look plain"
                .to_owned(),
        );
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
    if args.castle && (args.level || args.vault || args.room.is_some()) {
        return Err(
            "--castle stands in the streamed cave; it does not take --level (too short for \
             it), --vault (inside its box) or --room"
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
    args.mix.check()?;
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

/// A number in `[0, max]` for `flag`.
fn parse_unit(flag: &str, v: &str, max: f32) -> Result<f32, String> {
    let x: f32 = v
        .trim()
        .parse()
        .map_err(|_| format!("{flag} takes a number, not {v}"))?;
    if !(0.0..=max).contains(&x) {
        return Err(format!("{flag} must be in [0, {max}], not {x}"));
    }
    Ok(x)
}

/// `on` or `off` for `flag`.
fn parse_on_off(flag: &str, v: &str) -> Result<bool, String> {
    match v {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(format!("{flag} takes on or off, not {v}")),
    }
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
        PILE_VERB,
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

/// The probe update and gather passes' GPU time in a frame's `FRAME_PASSES` times.
fn probe_passes_ms(passes: &[f64]) -> f64 {
    FRAME_PASSES
        .iter()
        .zip(passes)
        .filter(|(name, _)| matches!(**name, "probes" | "gather"))
        .map(|(_, ms)| ms)
        .sum()
}

/// `--lamp-toggle`'s exit line: the probes plus gather median of the last [`SAMPLES`] frames
/// outside and inside a light-change window.
fn window_line(every: u32, ms: &[Vec<f64>; 2]) -> String {
    let tail = |v: &Vec<f64>| {
        let mut t = v[v.len().saturating_sub(SAMPLES)..].to_vec();
        if t.is_empty() {
            "n/a".to_owned()
        } else {
            format!("{:.3} ms", median(&mut t))
        }
    };
    format!(
        "lamp toggled every {every} frames: probes + gather median of the last {SAMPLES} outside a light-change window {} ({} frames), inside {} ({} frames)",
        tail(&ms[0]),
        ms[0].len(),
        tail(&ms[1]),
        ms[1].len()
    )
}

/// `--fade-reading`'s digs along the camera ray, one a frame from the first.
const FADE_DIGS: u32 = 5;
/// Frames the light settles in before each switch: the lamp off (the ore's light alone, the
/// baseline), then on.
const FADE_SETTLE: u32 = 256;
/// Frames read after the lamp goes off.
const FADE_TAIL: u32 = 200;
/// Frames the baseline and the lamp-on values average over, the last of each settle.
const FADE_MEAN: usize = 32;
/// A source whose lamp-on value exceeds its baseline by no more than this has no fade to
/// read.
const FADE_NO_LAMP: f64 = 1e-4;
/// The frame `--fade-reading` exits at.
const FADE_END: u32 = FADE_DIGS + 2 * FADE_SETTLE + FADE_TAIL;
/// Lighting pixels on each edge of the block around the screen centre a sample averages.
const FADE_BLOCK: u32 = 8;
/// The frames after the switch the bar is read over: under 10 % from FADE_BY through
/// FADE_THROUGH.
const FADE_BY: usize = 24;
const FADE_THROUGH: usize = 160;

/// Whether `--fade-reading`'s lamp is on at frame `frame`.
fn fade_lamp(frame: u32) -> bool {
    (FADE_DIGS + FADE_SETTLE..FADE_DIGS + 2 * FADE_SETTLE).contains(&frame)
}

/// One frame of `--fade-reading`: luminances averaged over the FADE_BLOCK square of lighting
/// pixels at the screen centre (the composite's output over the same square). `direct` is
/// the filter's irradiance (the light pass through its temporal history and the a-trous
/// filter, lamp and emissive), `lamp` and `emissive` the light pass's own estimate of each
/// before any history, `indirect` the probe gather's (which runs no filter); the composite
/// shades a hit with `albedo (direct + indirect) / PI`, so `lit`, their sum, is the wall's
/// radiance over its albedo. `output` is the composite's display-encoded output in [0, 1].
#[derive(Clone, Copy, Debug, Default)]
struct FadeSample {
    lit: f64,
    direct: f64,
    lamp: f64,
    emissive: f64,
    indirect: f64,
    output: f64,
    gpu_ms: f64,
}

impl FadeSample {
    fn read(gpu: &Gpu, frame: &Frame, gpu_ms: f64) -> Self {
        let lum = |p: [f32; 4]| {
            0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2])
        };
        let block = |t: &wgpu::Texture, edge: u32| {
            let (w, h) = (t.width(), t.height());
            let x0 = (w / 2).saturating_sub(edge / 2);
            let y0 = (h / 2).saturating_sub(edge / 2);
            move |i: u32| ((y0 + i / edge).min(h - 1) * w + (x0 + i % edge).min(w - 1)) as usize
        };
        let mean = |v: &[[f32; 4]], t: &wgpu::Texture, edge: u32, f: &dyn Fn([f32; 4]) -> f64| {
            let at = block(t, edge);
            (0..edge * edge).map(|i| f(v[at(i)])).sum::<f64>() / f64::from(edge * edge)
        };
        let (dev, q) = (&gpu.device, &gpu.queue);
        let filtered = readback::read_rgba16f(dev, q, frame.filtered());
        let raw = readback::read_rgba16f(dev, q, frame.irradiance());
        let indirect = readback::read_rgba16f(dev, q, frame.indirect_irradiance());
        let out_tex = frame.target();
        let out: Vec<[f32; 4]> = readback::read_rgba8(dev, q, out_tex)
            .chunks(4)
            .map(|p| [0, 1, 2, 3].map(|k| f32::from(p[k]) / 255.0))
            .collect();
        let scale = (out_tex.width() / frame.filtered().width()).max(1);
        let direct = mean(&filtered, frame.filtered(), FADE_BLOCK, &lum);
        let indirect = mean(&indirect, frame.indirect_irradiance(), FADE_BLOCK, &lum);
        Self {
            lit: direct + indirect,
            direct,
            lamp: mean(&raw, frame.irradiance(), FADE_BLOCK, &|p| f64::from(p[3])),
            emissive: mean(&raw, frame.irradiance(), FADE_BLOCK, &lum),
            indirect,
            output: mean(&out, out_tex, FADE_BLOCK * scale, &lum),
            gpu_ms,
        }
    }
}

/// One source of a [`FadeSample`], read out of it.
type FadeSource = fn(&FadeSample) -> f64;

/// `--fade-reading`'s report: for each source of [`FadeSample`], its baseline (the mean of
/// the last FADE_MEAN frames before the lamp first goes on), its lamp-on value (the last
/// FADE_MEAN with it on), the first frame after the switch whose excess over the baseline is
/// under 10 % of the lamp-on excess, the frames from FADE_BY through FADE_THROUGH over it, and
/// the first 32 frames of the tail.
fn fade_report(samples: &[FadeSample]) -> Vec<String> {
    let settle = FADE_SETTLE as usize;
    if samples.len() < 2 * settle + FADE_THROUGH {
        return vec![format!(
            "fade: {} samples, too few for a reading",
            samples.len()
        )];
    }
    let mut tail_ms: Vec<f64> = samples[2 * settle..].iter().map(|s| s.gpu_ms).collect();
    let ms = median(&mut tail_ms);
    let mut lines = vec![format!(
        "fade: the lamp off at frame {} after {} frames on; the frame's GPU time {ms:.3} ms (median of the tail)",
        FADE_DIGS + 2 * FADE_SETTLE,
        FADE_SETTLE
    )];
    let sources: [(&str, FadeSource); 6] = [
        ("composite (direct + indirect)", |s| s.lit),
        ("direct (filtered)", |s| s.direct),
        ("lamp (light pass)", |s| s.lamp),
        ("emissive (light pass)", |s| s.emissive),
        ("indirect (gather)", |s| s.indirect),
        ("output (display-encoded)", |s| s.output),
    ];
    let avg = |r: &[FadeSample], f: FadeSource| r.iter().map(f).sum::<f64>() / r.len() as f64;
    for (name, f) in sources {
        let base = avg(&samples[settle - FADE_MEAN..settle], f);
        let on = avg(&samples[2 * settle - FADE_MEAN..2 * settle], f);
        let tail: Vec<f64> = samples[2 * settle..].iter().map(f).collect();
        if on - base <= FADE_NO_LAMP {
            lines.push(format!(
                "fade: {name}: off {base:.5}, on {on:.5}; the lamp adds nothing here"
            ));
            continue;
        }
        let bar = 0.1 * (on - base);
        let under = |v: f64| v - base < bar;
        let first = tail.iter().position(|&v| under(v)).map(|i| i + 1);
        let over: Vec<usize> = (FADE_BY..=FADE_THROUGH)
            .filter(|&n| !under(tail[n - 1]))
            .collect();
        let first_text = first.map_or("not reached".to_owned(), |n| {
            format!("{n} ({:.0} ms)", n as f64 * ms)
        });
        lines.push(format!(
            "fade: {name}: off {base:.5}, on {on:.5}; first frame under 10 %: {first_text}; frames {FADE_BY}..={FADE_THROUGH} over it: {}; frames 1..=32 {:?}",
            if over.is_empty() {
                "none".to_owned()
            } else {
                format!("{} ({:?})", over.len(), &over[..over.len().min(8)])
            },
            tail[..32].iter().map(|v| (v * 1e4).round() / 1e4).collect::<Vec<_>>()
        ));
    }
    lines
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

/// The balance the keys and flags set over the acoustics: trims over the mixer's room curves
/// (or, with `--flat-mix`, over flat curves), and the room the curves read.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Mix {
    /// The echo trim, dB.
    echo_db: f32,
    /// The tail trim, dB.
    tail_db: f32,
    /// The tail's pre-delay, milliseconds.
    predelay_ms: f32,
    /// The tail's lines glide instead of handing over.
    glide: bool,
    /// The direct path's propagation delay.
    direct_delay: bool,
    /// `--flat-mix`: the trims over [`MixSettings::flat`] instead of the room curves.
    flat: bool,
    /// The mean free path of the latest room sent, metres.
    room: Option<f32>,
    /// The room the last printed curve values were read at.
    printed_room: Option<f32>,
}

impl Mix {
    /// The trims' range over the room curves.
    const TRIM_DB: std::ops::RangeInclusive<f32> = -24.0..=12.0;
    /// With `--flat-mix` the absolute ranges, so a flat balance replays as it was heard.
    const FLAT_ECHO_DB: std::ops::RangeInclusive<f32> = -12.0..=30.0;
    const FLAT_TAIL_DB: std::ops::RangeInclusive<f32> = -30.0..=6.0;
    const PREDELAY_MS: std::ops::RangeInclusive<f32> = 0.0..=200.0;
    /// A room that moves either curve by this much prints the mix line again.
    const REPRINT_DB: f32 = 0.5;

    fn echo_range(&self) -> std::ops::RangeInclusive<f32> {
        if self.flat {
            Self::FLAT_ECHO_DB
        } else {
            Self::TRIM_DB
        }
    }

    fn tail_range(&self) -> std::ops::RangeInclusive<f32> {
        if self.flat {
            Self::FLAT_TAIL_DB
        } else {
            Self::TRIM_DB
        }
    }

    /// `flag`'s value `v` as a finite number; [`Mix::check`] holds it to its range once every
    /// flag is read.
    fn parse(flag: &str, v: &str) -> Result<f32, String> {
        v.parse::<f32>()
            .ok()
            .filter(|x| x.is_finite())
            .ok_or_else(|| format!("{flag} takes a number, not {v}"))
    }

    /// Each flag's value within its range for this mix's mode.
    fn check(&self) -> Result<(), String> {
        for (flag, v, r) in [
            ("--echo-db", self.echo_db, self.echo_range()),
            ("--tail-db", self.tail_db, self.tail_range()),
            ("--predelay-ms", self.predelay_ms, Self::PREDELAY_MS),
        ] {
            if !r.contains(&v) {
                return Err(format!(
                    "{flag} takes {} to {}, not {v}",
                    r.start(),
                    r.end()
                ));
            }
        }
        Ok(())
    }

    /// Each value moved by its key's step and held in its range.
    fn step(self, echo_db: f32, tail_db: f32, predelay_ms: f32) -> Self {
        let clamp = |x: f32, r: std::ops::RangeInclusive<f32>| x.clamp(*r.start(), *r.end());
        Self {
            echo_db: clamp(self.echo_db + echo_db, self.echo_range()),
            tail_db: clamp(self.tail_db + tail_db, self.tail_range()),
            predelay_ms: clamp(self.predelay_ms + predelay_ms, Self::PREDELAY_MS),
            ..self
        }
    }

    fn settings(&self) -> MixSettings {
        let base = if self.flat {
            MixSettings::flat()
        } else {
            MixSettings::default()
        };
        MixSettings {
            echo_gain: 10f32.powf(self.echo_db / 20.0),
            direct_delay: self.direct_delay,
            reverb: ReverbSettings {
                level: 10f32.powf(self.tail_db / 20.0),
                predelay_s: self.predelay_ms / 1e3,
                lines: if self.glide {
                    TailLines::Glide
                } else {
                    TailLines::Handover
                },
                ..base.reverb
            },
            ..base
        }
    }

    /// The echo and tail curves' values at `room` (no room reads the curves' last knot).
    fn curve_db(&self, room: Option<f32>) -> (f32, f32) {
        let s = self.settings();
        let mfp = room.unwrap_or(f32::NAN);
        (s.echo_by_room.db_at(mfp), s.reverb.level_by_room.db_at(mfp))
    }

    /// Takes the mean free path of a room sent to the mixer; true when either curve moved by
    /// [`Mix::REPRINT_DB`] or more since the last printed line (which this then becomes).
    fn take_room(&mut self, mfp: f32) -> bool {
        self.room = Some(mfp);
        if self.flat {
            return false;
        }
        let (e0, t0) = self.curve_db(self.printed_room);
        let (e1, t1) = self.curve_db(self.room);
        let moved = (e1 - e0).abs() >= Self::REPRINT_DB || (t1 - t0).abs() >= Self::REPRINT_DB;
        if moved {
            self.printed_room = self.room;
        }
        moved
    }

    /// `mix: echoes <trim> dB (room <curve> dB at mfp <m> m), tail <trim> dB (room <curve> dB),
    /// pre-delay <ms>, tail lines <mode>, direct delay <on|off>`; `room flat` with
    /// `--flat-mix`, and `before any room` until a room arrives.
    fn line(&self) -> String {
        let (echo_room, tail_room) = if self.flat {
            ("room flat".to_owned(), "room flat".to_owned())
        } else {
            let (e, t) = self.curve_db(self.room);
            let at = self.room.map_or_else(
                || "before any room".to_owned(),
                |m| format!("at mfp {m:.3} m"),
            );
            (format!("room {e:+.1} dB {at}"), format!("room {t:+.1} dB"))
        };
        format!(
            "mix: echoes {:+.0} dB ({echo_room}), tail {:+.0} dB ({tail_room}), pre-delay {:.0} \
             ms, tail lines {}, direct delay {}",
            self.echo_db,
            self.tail_db,
            self.predelay_ms,
            if self.glide { "glide" } else { "handover" },
            if self.direct_delay { "on" } else { "off" }
        )
    }
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
    /// The balance a mixer would take, for the mix line's room curves.
    mix: Mix,
}

impl RoomProbe {
    fn new(absorption: f32, mix: Mix) -> Self {
        Self {
            acoustics: Acoustics::new(BlockClock::default()),
            table: acoustic_table(absorption),
            mix,
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
    /// The balance sent to the mixer.
    mix: Mix,
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
    fn open(debug: bool, absorption: f32, mic: bool, room_line: bool, mix: Mix) -> Option<Self> {
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
        let mut mixer = Mixer::new(hrtf, METRES_PER_VOXEL);
        mixer.set_mix(mix.settings());
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
                    mix,
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
    frame: Frame,
    presenter: Presenter,
    /// Times the frame's passes, `FRAME_PASSES` in order.
    timer: Option<PassTimer>,
    /// Times the attribute pass, with contour attributes and `TIMESTAMP_QUERY`.
    attr_timer: Option<MarchTimer>,
    hud: HudOverlay,
    /// Times the HUD pass, with `TIMESTAMP_QUERY`.
    hud_timer: Option<PassTimer>,
    /// The time of every `prepare_attrs` call, in milliseconds since the start, by call
    /// number from 1; a call made outside the frame path takes the next frame's time.
    attr_call_ms: Vec<f64>,
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

/// The pool's bins at a moment: per bin with a page, its slot size, live slots, spare units
/// (free slots and the rest of its cursor's page) and pages, then the spare units over every
/// bin against the live units, e.g. `bins: 2u 10 live 6 spare 1p, 3u 4 live 0 spare 1p;
/// spare 6 of 32 live units (18.75 %)`.
fn bins_line(mirror: &GpuWorld) -> String {
    let bins: Vec<String> = mirror
        .bin_stats()
        .iter()
        .filter(|b| b.pages > 0)
        .map(|b| {
            format!(
                "{}u {} live {} spare {}p",
                b.units,
                b.live,
                b.spare * u64::from(b.units),
                b.pages
            )
        })
        .collect();
    let (spare, live) = (mirror.spare_units(), mirror.live_units());
    format!(
        "bins: {}; spare {spare} of {live} live units ({:.2} %)",
        bins.join(", "),
        100.0 * spare as f64 / live.max(1) as f64
    )
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
    /// The frame's GPU time: the sum of its passes.
    gpu_ms: Vec<f64>,
    /// Each pass's GPU time, `FRAME_PASSES` in order.
    pass_ms: Vec<Vec<f64>>,
    /// The HUD pass's GPU time, on frames that draw it.
    hud_ms: Vec<f64>,
}

/// Every frame's render-thread CPU, its `update + poll + sync` (the sync alone without
/// streaming) and, on frames the march timer is on, the march's GPU time, over the whole run:
/// the `frames:` line printed at exit.
#[derive(Default)]
struct RunTimes {
    cpu_ms: Vec<f64>,
    stream_ms: Vec<f64>,
    gpu_ms: Vec<f64>,
}

impl RunTimes {
    fn push(&mut self, cpu_ms: f64, stream_ms: Option<f64>, gpu_ms: Option<f64>) {
        self.cpu_ms.push(cpu_ms);
        self.stream_ms.extend(stream_ms);
        self.gpu_ms.extend(gpu_ms);
    }

    /// p50, p99 and max of each series, `None` before the first frame.
    fn line(&self) -> Option<String> {
        if self.cpu_ms.is_empty() {
            return None;
        }
        let stats = |v: &[f64]| {
            let mut v = v.to_vec();
            format!(
                "p50 {:.3} ms, p99 {:.3} ms, max {:.3} ms",
                percentile(&mut v, 50.0),
                percentile(&mut v, 99.0),
                percentile(&mut v, 100.0)
            )
        };
        let gpu = if self.gpu_ms.is_empty() {
            "n/a (timer off)".to_owned()
        } else {
            format!(
                "{} over {} timed frames",
                stats(&self.gpu_ms),
                self.gpu_ms.len()
            )
        };
        Some(format!(
            "frames: {} frames; frame CPU {}; update + poll + sync {}; march GPU {gpu}",
            self.cpu_ms.len(),
            stats(&self.cpu_ms),
            stats(&self.stream_ms)
        ))
    }
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
    /// Last frame's pass readings, shown on the next frame's rows: the frame's total and
    /// each pass's, `FRAME_PASSES` in order.
    march_ms: Option<f32>,
    pass_ms: Option<Vec<f64>>,
    hud_ms: Option<f32>,
    cpu_ms: f32,
    /// The probe volume's live probes at the last world sample, with indirect light on.
    live_probes: Option<u32>,
    /// The last frame's steps view, read back while that view and the HUD are on.
    steps: Option<hud::StepsSample>,
    format_us: u32,
    page_bytes: u64,
    /// Reused buffers: the rows' text, their colours, and the strip's samples.
    text: String,
    colours: Vec<HudColour>,
    strip: Vec<f32>,
    /// The whole run's frame times, shown or not.
    run: RunTimes,
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
            pass_ms: None,
            hud_ms: None,
            cpu_ms: 0.0,
            live_probes: None,
            steps: None,
            format_us: 0,
            page_bytes: hud::page_bytes(),
            text: String::new(),
            colours: Vec::new(),
            strip: Vec::with_capacity(hud::RING),
            run: RunTimes::default(),
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

/// The two terms of `GpuWorld::pressure`: pool-unit occupancy and record occupancy.
fn pressure_terms(m: &GpuWorld) -> [f64; 2] {
    [
        m.live_units() as f64 / f64::from(m.reserved_units().max(1)),
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
    /// The last frame's `update`, `poll`, `apply_events` and `sync` CPU in milliseconds.
    stages_ms: [f64; 4],
    /// The last frame's light extraction inside `sync` (`SyncStats::light_us`), in
    /// milliseconds, its ranges evicted and refused, and how long the oldest chunk inside the
    /// light radius has waited for its range, in milliseconds.
    light_ms: f64,
    light_evicted: u64,
    light_refused: u64,
    light_wait_ms: f64,
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
    near: NearRing,
    flight_frames: u64,
    /// The first and last flight frame's time, in milliseconds since the start.
    flight_span_ms: Option<(f64, f64)>,
    unready_frames: u64,
    gpu_ms: Vec<f64>,
    cpu_ms: Vec<f64>,
    /// The streaming CPU's stages per flight frame: `update`, `poll`, `apply_events`, `sync`.
    stages_ms: [Vec<f64>; 4],
    /// The light extraction inside `sync`, per flight frame; the flight's peak of light
    /// samples in the mirror, its ranges evicted and refused, and the longest wait of a chunk
    /// inside the light radius, in milliseconds.
    light_ms: Vec<f64>,
    /// Each frame pass's GPU time per flight frame, `FRAME_PASSES` in order.
    pass_ms: Vec<Vec<f64>>,
    peak_light: u64,
    light_evicted: u64,
    light_refused: u64,
    light_wait_ms: f64,
    /// The attribute pass over the flight, with contour attributes.
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
    /// The most staging-belt chunks the mirror held over the flight.
    peak_belt: usize,
    generated_at_takeoff: u64,
}

/// The chunk offsets the unready-near count checks: every offset within `r_full - 2` chunks,
/// recomputed when the streamer's `r_full` changes (the pressure rule's `set_radii`).
struct NearRing {
    r_full: i32,
    offsets: Vec<[i32; 3]>,
    recomputes: u32,
}

impl NearRing {
    fn new(r_full: i32) -> Self {
        Self {
            r_full,
            offsets: ball((r_full - 2).max(0).pow(2)),
            recomputes: 0,
        }
    }

    /// Recomputes the offsets when `r_full` differs from the ring's.
    fn follow(&mut self, r_full: i32) {
        if r_full != self.r_full {
            let recomputes = self.recomputes + 1;
            *self = Self::new(r_full);
            self.recomputes = recomputes;
        }
    }

    /// Whether any chunk of the ring around `cam` is not `ready`.
    fn unready(&self, cam: ChunkCoord, ready: impl Fn(ChunkCoord) -> bool) -> bool {
        self.offsets
            .iter()
            .any(|o| !ready(ChunkCoord([0, 1, 2].map(|k| cam.0[k] + o[k]))))
    }
}

/// Per-frame readings of the attribute pass over the autopilot's flight: the work list's
/// length and the render thread's CPU time in `update_attrs` every frame, and the pass's GPU
/// time on the frames whose list is not empty.
#[derive(Default)]
struct AttrSamples {
    items: Vec<f64>,
    cpu_ms: Vec<f64>,
    gpu_ms: Vec<f64>,
    /// The CPU of `update_attrs`'s two halves: the list build and the pass recording.
    build_ms: Vec<f64>,
    encode_ms: Vec<f64>,
    /// The builder's counters per frame: shell visits, dirty-brick visits, whole-chunk item
    /// calls, hash lookups.
    counters: [Vec<f64>; 4],
    /// Every frame whose `update_attrs` took over [`HITCH_MS`]: its flight frame, CPU
    /// milliseconds and counters.
    hitches: Vec<(usize, f64, AttrWorkStats)>,
    /// How long the oldest waiting brick had waited when each call started, in milliseconds
    /// (0 when none waited).
    age_ms: Vec<f64>,
    /// The attribute pass's work-buffer builds after the first flight frame and the last.
    builds_first: Option<u32>,
    builds_last: u32,
}

/// The mirror's staging belt: its chunks, the host bytes they map at the chunk size, and how
/// often a burst replaced it.
fn belt_line(mirror: &GpuWorld) -> String {
    let n = mirror.belt_chunks();
    format!(
        "staging belt {n} chunks ({} B mapped), {} trims",
        n as u64 * BELT_CHUNK_BYTES,
        mirror.belt_trims()
    )
}

/// Before a `prepare_attrs` call at `now_ms`: stamps it and any earlier call not yet stamped
/// with `now_ms` in `call_ms` (by call number from 1), and returns how long the oldest brick
/// waiting in `mirror`'s carry-over queue has waited, 0 when none waits.
fn stamp_attr_call(mirror: &GpuWorld, call_ms: &mut Vec<f64>, now_ms: f64) -> f64 {
    let (calls, oldest) = mirror.attr_queue_age();
    if call_ms.len() < calls as usize + 1 {
        call_ms.resize(calls as usize + 1, now_ms);
    }
    oldest.map_or(0.0, |o| now_ms - call_ms[o as usize - 1])
}

/// Most bricks the streaming mirror's attribute pass computes per frame
/// ([`MirrorConfig::attr_cap`]); the rest waits, reading face mode.
const ATTR_CAP: usize = 8192;

/// An `update_attrs` call longer than this is listed with its counters.
const HITCH_MS: f64 = 16.0;

impl AttrSamples {
    /// Records one flight frame's pass.
    fn push(
        &mut self,
        st: AttrWorkStats,
        build_ms: f64,
        encode_ms: f64,
        gpu_ms: Option<f64>,
        age_ms: f64,
    ) {
        self.age_ms.push(age_ms);
        let cpu = build_ms + encode_ms;
        if cpu > HITCH_MS {
            self.hitches.push((self.items.len(), cpu, st));
        }
        self.items.push(f64::from(st.items));
        self.cpu_ms.push(cpu);
        self.build_ms.push(build_ms);
        self.encode_ms.push(encode_ms);
        let c = [st.shell_visits, st.dirty_visits, st.item_calls, st.lookups];
        for (v, n) in self.counters.iter_mut().zip(c) {
            v.push(n as f64);
        }
        self.gpu_ms.extend(gpu_ms);
    }
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
        let [b50, b99, bmax] = stats(&mut self.build_ms);
        let [e50, e99, emax] = stats(&mut self.encode_ms);
        let mut out = vec![
            format!(
                "autopilot: attribute work list: p50 {i50:.0}, p99 {i99:.0}, max {imax:.0} bricks over {frames} frames ({busy} with work)"
            ),
            format!(
                "autopilot: attribute pass CPU (update_attrs, render thread): p50 {c50:.3} ms, p99 {c99:.3} ms, max {cmax:.3} ms over {frames} frames"
            ),
        ];
        let [a50, a99, amax] = stats(&mut self.age_ms);
        out.push(format!(
            "autopilot: attribute carry-over age (oldest waiting brick at the start of each update_attrs): p50 {a50:.1} ms, p99 {a99:.1} ms, max {amax:.1} ms (bar 250 ms)"
        ));
        out.push(format!(
            "autopilot: attribute work buffer builds: {} after the first flight frame, {} at the end",
            self.builds_first.unwrap_or(0),
            self.builds_last
        ));
        out.push(format!(
            "autopilot: attribute pass CPU split: list build p50 {b50:.3} ms, p99 {b99:.3} ms, max {bmax:.3} ms; pass recording p50 {e50:.3} ms, p99 {e99:.3} ms, max {emax:.3} ms"
        ));
        for (name, v) in [
            "shell visits",
            "dirty-brick visits",
            "whole-chunk item calls",
            "hash lookups",
        ]
        .iter()
        .zip(&mut self.counters)
        {
            let [n50, n99, nmax] = stats(v);
            out.push(format!(
                "autopilot: attribute builder {name}: p50 {n50:.0}, p99 {n99:.0}, max {nmax:.0}"
            ));
        }
        for (frame, ms, st) in &self.hitches {
            out.push(format!(
                "autopilot: attribute hitch at flight frame {frame}: {ms:.3} ms, {} whole chunks, {} dirty bricks, {} shell chunks, {} shell visits, {} dirty visits, {} item calls, {} lookups, {} items",
                st.whole_chunks,
                st.dirty_bricks,
                st.shell_chunks,
                st.shell_visits,
                st.dirty_visits,
                st.item_calls,
                st.lookups,
                st.items
            ));
        }
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

/// The line for a logic's `wedged` line: `wedged: 9 cells, {rock: 4608}, cannot fall`; `None`
/// for any other line. A `drop refused` line is already prose and prints as it is.
fn wedged_terminal_line(table: &MaterialTable, line: &str) -> Option<String> {
    let (cells, counts) = parse_wedged_line(line)?;
    Some(format!(
        "wedged: {cells} cells, {{{}}}, cannot fall",
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
    /// prefab `P` stamps, ADR-0014) and [`DEFAULT_SUPPORT`] set (the support query whose
    /// threshold the logic's drops yield below), and the shared client.
    fn new(world: &Arc<World>, logic: Box<dyn Logic>, seed: u64) -> Self {
        let vault = Arc::new(vault());
        let mut prefabs = vxe_world::PrefabRegistry::new();
        prefabs
            .register(vault.clone())
            .expect("an empty registry takes the vault");
        let mut server = Server::with_prefabs(world.clone(), prefabs, logic, seed);
        server.set_support(DEFAULT_SUPPORT, &[]);
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
    /// The lighting modes `F3` to `F7` and `M` switch, and the banner naming them; the frame
    /// takes them at the start of each frame (`F6`'s resolution needs the device).
    modes: hud::Modes,
    /// The upscaler `F10` cycles and its banner; a change is applied at the next frame.
    upscaler: hud::UpscalerBanner,
    upscaler_changed: bool,
    /// The scene's banner row (`castle gate`) and when it was last shown, by the frame loop's
    /// clock; `None` without one.
    scene: Option<(String, f64)>,
    /// `--castle`'s current shot, which `I` advances; `None` without the castle.
    castle_shot: Option<castle::CastleShot>,
    look: LookParams,
    /// The preset `N` last chose, or `--look`'s with its part flags.
    preset: LookPreset,
    /// `N` changed the preset: its fit and material rows reach the mirror in the next frame.
    preset_changed: bool,
    /// Sets `look.cutaway` each frame of interactive flight; `X` or the pad's `West` cycles it.
    cutaway: CutawayMode,
    dig_radius: i64,
    /// Scroll banked toward the next dig-radius step.
    scroll: ScrollSteps,
    /// `L2` and `R2` held, for their press edges.
    triggers: [bool; 2],
    bench_runs: u64,
    frame: u32,
    gfx: Option<Gfx>,
    samples: Option<Samples>,
    /// Under `--lamp-toggle`, the probes plus gather GPU time of each frame outside and
    /// inside a light-change window.
    window_ms: [Vec<f64>; 2],
    /// Under `--fade-reading`, a sample a frame from the end of the digs on.
    fade: Vec<FadeSample>,
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

    /// Counts the probe volume's live probes for the HUD, with indirect light on: a blocking
    /// read of its state words, paid at the world sample's cadence.
    fn sample_probes(&mut self, gpu: &Gpu) {
        self.hud.live_probes = self.gfx.as_ref().and_then(|g| {
            g.frame.lighting().indirect.then(|| {
                let probes = g.frame.probes();
                let states = probes.read_states(&gpu.device, &gpu.queue);
                let origin = probes.origin_for(self.fly.camera.position.to_array());
                probes.live_count(&states, origin)
            })
        });
    }

    /// Refreshes the HUD's values in place for this frame, from the O(1) sources and the
    /// last world sample; the last edit is already there.
    fn refresh_hud_stats(&mut self) {
        let (mirror_bytes, pool_live, pool_free) = self.gfx.as_ref().map_or((0, 0, 0), |g| {
            (
                g.mirror.mirror_bytes(),
                g.mirror.live_bricks(),
                g.mirror.spare_slots().min(u64::from(u32::MAX)) as u32,
            )
        });
        let pressure = self.gfx.as_ref().map(|g| hud::Pressure {
            pressure: g.mirror.pressure(),
            budget_bytes: g.mirror.allocated_bytes(),
            r_full: self.stream.as_ref().map(|s| [s.rule.r_full, s.rule.max]),
        });
        let lighting = self.gfx.as_ref().and_then(|g| {
            let passes = self.hud.pass_ms.as_ref()?;
            Some(hud::LightingRow {
                passes: FRAME_PASSES
                    .iter()
                    .zip(passes)
                    .map(|(&n, &ms)| (n, ms as f32))
                    .collect(),
                light_samples: g.mirror.light_samples(),
                live_probes: self
                    .hud
                    .live_probes
                    .filter(|_| self.modes.lighting.indirect),
                modes: self.modes.line(),
                upscale: g
                    .frame
                    .upscaling()
                    .then(|| [g.frame.internal_size(), g.frame.output_size()]),
            })
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
        s.cutaway = self.look.cutaway.then_some(self.cutaway.name());
        s.chunk = voxel_of(pos).chunk().0;
        s.mode = mode;
        s.format_us = h.format_us;
        s.lighting = lighting;
    }

    /// The banner at the frame loop's clock: the scene's row while it shows (for
    /// `hud::BANNER_MS` after it was last shown), over the lighting modes' banner and the
    /// upscaler's, whichever show.
    fn banner(&self) -> Option<String> {
        let now = ms_since(self.start);
        let scene = self
            .scene
            .as_ref()
            .filter(|(_, at)| now - at < hud::BANNER_MS)
            .map(|(s, _)| s.to_owned());
        let rows: Vec<String> = [scene, self.modes.banner(now), self.upscaler.banner(now)]
            .into_iter()
            .flatten()
            .collect();
        if rows.is_empty() {
            None
        } else {
            Some(rows.join("\n"))
        }
    }

    /// Formats this frame's rows and strip into the overlay, timing the formatting. Allocates
    /// nothing once the buffers have grown to the rows' size.
    fn update_hud(&mut self, gpu: &Gpu) {
        self.refresh_hud_stats();
        let banner = self.banner();
        let h = &mut self.hud;
        h.format_rows();
        // The mode banner goes under the rows, hiding none of them.
        if let Some(b) = banner {
            for line in b.lines() {
                h.text.push_str(line);
                h.text.push('\n');
                h.colours.push(HudColour::Yellow);
            }
        }
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
        // The frame's history dies wherever the world changed.
        if let (Some(gfx), Some((min, max))) = (self.gfx.as_mut(), r.aabb) {
            gfx.frame.invalidate(min.0, max.0);
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

    /// The current preset when the table carries rock's detail (every look but `--look
    /// plain`).
    fn preset(&self) -> Option<&LookPreset> {
        self.args.look.is_some().then_some(&self.preset)
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
                        None => match wedged_terminal_line(self.world.materials(), line) {
                            Some(l) => println!("{l}"),
                            None => println!("{line}"),
                        },
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
                        // A drop happens every tick a region falls, and a pile ends a fall the
                        // logic's line reports: kept for the HUD only.
                        Ok(r) if verb == DROP_VERB || verb == PILE_VERB => {
                            if r.aabb().is_some() {
                                self.note_edit(verb, &r.edit_result(), ms);
                            }
                        }
                        // The logic's `drop refused` or `pile refused` line reports it.
                        Err(_) if verb == DROP_VERB || verb == PILE_VERB => {}
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

    /// The mirror sync after a bench, timed, with the bench line; with contour attributes, the
    /// attribute pass over the carves timed alone.
    fn bench_sync(&mut self, gpu: &Gpu, carves: usize, refused: usize, edit_ms: f64) {
        let Some(gfx) = self.gfx.as_mut() else { return };
        let t = now();
        let mut enc = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let synced = gfx.mirror.sync(&self.world, &gpu.queue, &mut enc);
        let sync_ms = ms_since(t);
        gpu.queue.submit([enc.finish()]);
        match synced {
            Ok(s) => println!(
                "bench: {carves} carves of radius 1 m within 10 m ({refused} refused): \
                 edits {edit_ms:.2} ms CPU; sync uploaded {} B ({} bricks, \
                 {} chunks, {} visited, {} writes) in {:.2} ms",
                s.bytes, s.bricks, s.chunks, s.chunks_visited, s.writes, sync_ms
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
            mut pass_ms,
            mut hud_ms,
        } = samples;
        let Some(gfx) = &self.gfx else { return };
        let [w, h] = [gfx.frame.target().width(), gfx.frame.target().height()];
        let [iw, ih] = gfx.frame.internal_size();
        let gpu = if gpu_ms.is_empty() {
            "n/a (no TIMESTAMP_QUERY)".to_owned()
        } else {
            let passes: Vec<String> = FRAME_PASSES
                .iter()
                .zip(pass_ms.iter_mut())
                .map(|(name, ms)| format!("{name} {:.3}", median(ms)))
                .collect();
            format!("{:.3} ms ({} ms)", median(&mut gpu_ms), passes.join(", "))
        };
        let hud = if hud_ms.is_empty() {
            String::new()
        } else {
            format!(", gpu hud {:.4} ms", median(&mut hud_ms))
        };
        println!(
            "frame ({w}x{h} from {iw}x{ih}, median of {}, {}): cpu {:.3} ms, gpu frame {gpu}{hud}; frame targets {} B; full bricks {}, mirror {} B live, {} B allocated",
            cpu_ms.len(),
            look_line(&self.look, self.preset()),
            median(&mut cpu_ms),
            gfx.frame.frame_bytes(),
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
            println!("{}", audio.mix.line());
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
        if let Some(p) = &self.room_probe {
            println!("{}", p.mix.line());
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
            level_pool_units(&self.world, self.args.look.is_some()) * gpu_world::UNIT_BYTES
        };
        (
            adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
            vxe_render::limits_for(adapter, pool),
        )
    }

    fn init(&mut self, gpu: &Gpu) {
        let output = self.args.render.unwrap_or(gpu.size);
        let timer = PassTimer::with_passes(&gpu.device, &gpu.queue, FRAME_PASSES.len() as u32);
        println!("device ready after {:.0} ms", ms_since(self.start));
        let attr_timer = self
            .args
            .look
            .and_then(|_| MarchTimer::new(&gpu.device, &gpu.queue));
        let (kind, scale, jitter) = self.args.upscaler_settings(self.upscaler.mode);
        let mut frame = Frame::with_sizes(&gpu.device, scale.internal(output), output);
        frame.set_probe_spacing(&gpu.device, self.args.probe_spacing);
        frame.set_jitter(&gpu.device, jitter);
        frame.set_upscaler_kind(&gpu.device, kind);
        frame.set_sharpness(&gpu.queue, self.args.sharpen);
        println!("{}", self.upscaler.line());
        frame.set_lighting(self.modes.lighting);
        frame.set_resolution(&gpu.device, self.modes.resolution);
        println!("{}", self.modes.line());
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
                Err(e) => eprintln!("mirror sync failed: {e}"),
            }
            mirror
        };
        let mut gfx = Gfx {
            mirror,
            frame,
            presenter: Presenter::new(&gpu.device, gpu.surface_format),
            timer,
            attr_timer,
            hud: HudOverlay::new(&gpu.device, gpu.surface_format, HUD_SCALE),
            hud_timer: PassTimer::new(&gpu.device, &gpu.queue),
            attr_call_ms: Vec::new(),
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
                level_mirror_budget(&self.world, chunks, self.args.look.is_some())
            }
        })
    }

    /// A mirror under `cfg` reserving [`Run::mirror_budget`], with contour attributes when
    /// `--look` was given. Prints what it reserved and how long creating its buffers took.
    fn new_mirror(&self, gpu: &Gpu, cfg: MirrorConfig) -> GpuWorld {
        let cfg = MirrorConfig {
            budget_bytes: self.mirror_budget(),
            light_radius: self.args.light_radius,
            ..cfg
        };
        let t = now();
        let mirror = match &self.args.look {
            Some(p) => {
                let mut m = GpuWorld::with_surface_attrs(&gpu.device, cfg);
                m.set_surface_fit(p.tuning.fit);
                m
            }
            None => GpuWorld::with_config(&gpu.device, cfg),
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
    fn init_stream(&mut self, gpu: &Gpu) -> GpuWorld {
        let mut mirror = self.new_mirror(
            gpu,
            MirrorConfig {
                sync_budget_bytes: self.args.sync_budget,
                record_chunks: Some(flythrough::stream_record_chunks(self.args.residency)),
                attr_cap: ATTR_CAP,
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
        // The light extraction of the first sync that scanned a brick, and of all of them.
        let mut first_light: Option<SyncStats> = None;
        let (mut light_us, mut syncs) = (0u64, 0u32);
        loop {
            stream.streamer.update(&self.world, &[self.spawn]);
            stream.tick_ms.push(ms_since(self.start));
            let ev = stream.streamer.poll(&self.world);
            mirror.apply_events(&ev);
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
                    eprintln!("mirror sync failed: {e}");
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

    fn resized(&mut self, gpu: &Gpu) {
        if let (Some(gfx), None) = (self.gfx.as_mut(), self.args.render) {
            let (_, scale, _) = self.args.upscaler_settings(self.upscaler.mode);
            gfx.frame
                .set_sizes(&gpu.device, scale.internal(gpu.size), gpu.size);
        }
    }

    /// One frame of streaming: update, poll, apply events and sync into the frame's `enc`,
    /// timed together.
    fn stream_step(&mut self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder) -> Result<(), String> {
        let (Some(stream), Some(gfx)) = (self.stream.as_mut(), self.gfx.as_mut()) else {
            return Ok(());
        };
        let cam = voxel_of(self.fly.camera.position);
        let t = now();
        stream.streamer.update(&self.world, &[cam]);
        let update_ms = ms_since(t);
        let tp = now();
        let ev = stream.streamer.poll(&self.world);
        let poll_ms = ms_since(tp);
        let ta = now();
        gfx.mirror.apply_events(&ev);
        gfx.mirror.set_focus(cam);
        let apply_ms = ms_since(ta);
        let ts = now();
        let synced = gfx.mirror.sync(&self.world, &gpu.queue, enc);
        let sync_ms = ms_since(ts);
        let cpu = ms_since(t);
        stream.stages_ms = [update_ms, poll_ms, apply_ms, sync_ms];
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
                stream.light_ms = s.light_us as f64 / 1e3;
                stream.light_evicted = s.light_evicted;
                stream.light_refused = s.light_refused;
                // The oldest wait in syncs, one sync per tick, as the time since that tick.
                let n = stream.tick_ms.len();
                let w = s.light_oldest_wait as usize;
                stream.light_wait_ms = if w > 0 && w < n {
                    stream.tick_ms[n - 1] - stream.tick_ms[n - 1 - w]
                } else {
                    0.0
                };
                (s.deferred_chunks, false, None)
            }
            Err(e) => (gfx.mirror.pending_chunks() as u64, true, Some(e)),
        };
        let m = &gfx.mirror;
        let [slots, records] = pressure_terms(m);
        if (deferred > 0 || empty) && stream.deferrals.syncs == 0 {
            let what = error.map_or_else(|| "deferred chunks".to_owned(), |e| e.to_string());
            eprintln!(
                "mirror: {deferred} chunks wait for space ({what}) at tick {tick} (pressure {:.4}, units {slots:.4}, records {records:.4}; {} spare slots, {} spare units, {} free pages, {} pages issued, r_full {})",
                m.pressure(),
                m.spare_slots(),
                m.spare_units(),
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
            gfx.frame.set_lod(LodParams {
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
            println!("{}", bins_line(&gfx.mirror));
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
                        if let (Some(gfx), Some((min, max))) = (self.gfx.as_mut(), r.aabb) {
                            gfx.frame.invalidate(min.0, max.0);
                        }
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
        let now_ms = ms_since(self.start);
        let (Some(p), Some(stream), Some(gfx)) =
            (self.pilot.as_mut(), self.stream.as_ref(), self.gfx.as_ref())
        else {
            return;
        };
        p.flight_frames += 1;
        p.flight_span_ms = Some(
            p.flight_span_ms
                .map_or((now_ms, now_ms), |(a, _)| (a, now_ms)),
        );
        let cam = voxel_of(self.fly.camera.position).chunk();
        p.near.follow(stream.streamer.config().residency.r_full);
        let unready = p.near.unready(cam, |c| {
            stream.streamer.tier(c) == Some(Tier::Full)
                && gfx.mirror.is_mirrored(c)
                && !gfx.mirror.is_lod(c)
        });
        p.unready_frames += u64::from(unready);
        if let Some(&cpu) = stream.cpu_ms.last() {
            p.cpu_ms.push(cpu);
            for (v, &ms) in p.stages_ms.iter_mut().zip(&stream.stages_ms) {
                v.push(ms);
            }
            p.light_ms.push(stream.light_ms);
            p.light_evicted += stream.light_evicted;
            p.light_refused += stream.light_refused;
            p.light_wait_ms = p.light_wait_ms.max(stream.light_wait_ms);
            p.peak_light = p.peak_light.max(gfx.mirror.light_samples());
        }
        p.peak_mirror = p.peak_mirror.max(gfx.mirror.mirror_bytes());
        p.peak_allocated = p.peak_allocated.max(gfx.mirror.allocated_bytes());
        p.peak_pressure = p.peak_pressure.max(gfx.mirror.pressure());
        p.peak_belt = p.peak_belt.max(gfx.mirror.belt_chunks());
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
        // The frame bar's credit: the flight's frames had the attribute pass's GPU time not
        // been spent, `F_d* = F_d T / (T - G)`, `T` the first to last flight frame and `G` the
        // pass's GPU milliseconds over the same frames.
        let t = p.flight_span_ms.map_or(0.0, |(a, b)| b - a);
        let g: f64 = p.attr.gpu_ms.iter().sum();
        let f = p.flight_frames as f64;
        let credited = if t > g { f * t / (t - g) } else { f };
        println!(
            "autopilot: frame bar: T {t:.1} ms, G {g:.1} ms, F_d {} frames, F_d* {credited:.1}",
            p.flight_frames
        );
        println!(
            "autopilot: unready-near frames {} of {} (chunks wanted full within r_full - 2 of the camera's, {} at the end; {} ring recomputes)",
            p.unready_frames,
            p.flight_frames,
            p.near.r_full - 2,
            p.near.recomputes
        );
        let mut cpu = p.cpu_ms.clone();
        println!(
            "autopilot: streaming CPU per frame (update + poll + sync): p50 {:.3} ms, p99 {:.3} ms, max {:.3} ms over {} frames (line 1.0 ms)",
            percentile(&mut cpu, 50.0),
            percentile(&mut cpu, 99.0),
            percentile(&mut cpu, 100.0),
            cpu.len()
        );
        for (name, v) in ["update", "poll", "apply_events", "sync"]
            .iter()
            .zip(&mut p.stages_ms)
        {
            println!(
                "autopilot: streaming stage {name}: p50 {:.3} ms, p99 {:.3} ms, max {:.3} ms",
                percentile(v, 50.0),
                percentile(v, 99.0),
                percentile(v, 100.0)
            );
        }
        println!(
            "autopilot: light extraction in sync: p50 {:.3} ms, p99 {:.3} ms, max {:.3} ms over {} frames; light samples peak {}, now {}; ranges evicted {}, refused {}; longest wait for a range inside the radius {:.0} ms",
            percentile(&mut p.light_ms, 50.0),
            percentile(&mut p.light_ms, 99.0),
            percentile(&mut p.light_ms, 100.0),
            p.light_ms.len(),
            p.peak_light,
            gfx.mirror.light_samples(),
            p.light_evicted,
            p.light_refused,
            p.light_wait_ms
        );
        let mut lat = stream.latency_ms[p.latency_from..].to_vec();
        println!(
            "autopilot: residency latency wanted to mirrored: p95 {:.0} ms, max {:.0} ms over {} chunks (proposal 500 ms)",
            percentile(&mut lat, 95.0),
            percentile(&mut lat, 100.0),
            lat.len()
        );
        println!(
            "autopilot: {}; peak {} chunks over the flight",
            belt_line(&gfx.mirror),
            p.peak_belt
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
            "autopilot: mirror budget {} B ({} units, {} records), peak live / budget {:.4}, peak pressure {:.4} (units {:.4}, records {:.4}), {} radius changes (r_full {} of {}), {} syncs with a deferral",
            gfx.mirror.allocated_bytes(),
            gfx.mirror.reserved_units(),
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
            let [w, h] = [gfx.frame.target().width(), gfx.frame.target().height()];
            println!(
                "autopilot: march GPU at {w}x{h} over the flight: p50 {:.3} ms, p99 {:.3} ms, max {:.3} ms over {} frames",
                percentile(&mut p.gpu_ms, 50.0),
                percentile(&mut p.gpu_ms, 99.0),
                percentile(&mut p.gpu_ms, 100.0),
                p.gpu_ms.len()
            );
            for (name, v) in FRAME_PASSES.iter().zip(&mut p.pass_ms) {
                println!(
                    "autopilot: pass {name}: p50 {:.3} ms, p99 {:.3} ms",
                    percentile(v, 50.0),
                    percentile(v, 99.0)
                );
            }
        }
        let [iw, ih] = gfx.frame.internal_size();
        println!(
            "autopilot: frame targets {} B at {}x{} from {iw}x{ih} ({} lighting)",
            gfx.frame.frame_bytes(),
            gfx.frame.target().width(),
            gfx.frame.target().height(),
            gfx.frame.resolution().name()
        );
        println!("autopilot: {}", probe_bytes_line(gfx.frame.probes()));
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
        if input.pressed(KeyCode::F2) || input.pad_pressed(PadButton::Start) {
            self.hud.visible = !self.hud.visible;
        }
        if self.frame == 0 {
            for _ in 0..self.args.bench {
                self.bench();
            }
        }
        if self.args.fade_reading && self.frame < FADE_DIGS {
            self.dig();
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
            self.cutaway
                .update(&mut self.look, &self.world, self.fly.camera.position);
        }
        self.audio_step();
        if let Some(n) = self.args.frames
            && self.frame + SAMPLES as u32 == n
        {
            self.samples = Some(Samples::default());
        }
        self.draw(gpu, surface, t0)
    }

    /// The keys, buttons and pad buttons of interactive flight; the autopilot takes none of
    /// them, so a stray click cannot change the world it measures. On a pad `R2` digs, `L2`
    /// fills, the D-pad's up and down step the dig radius as a scroll line does, `North`
    /// toggles walking, `West` cycles the cutaway as `X` does, and `East` knocks.
    fn handle_input(&mut self, input: &Input) {
        for (key, mode) in [
            (KeyCode::F3, hud::ModeKey::ShadowWalk),
            (KeyCode::F4, hud::ModeKey::Sampler),
            (KeyCode::F5, hud::ModeKey::History),
            (KeyCode::F6, hud::ModeKey::Resolution),
            (KeyCode::F7, hud::ModeKey::Lamp),
            (KeyCode::F8, hud::ModeKey::Indirect),
            (KeyCode::KeyM, hud::ModeKey::Pulse),
        ] {
            if input.pressed(key) {
                self.modes.press(mode, ms_since(self.start));
                println!("{}", self.modes.line());
            }
        }
        if input.pressed(KeyCode::F10) {
            let mode = self.upscaler.mode.next();
            let (_, scale, _) = self.args.upscaler_settings(mode);
            self.upscaler
                .change(mode, scale.name(), ms_since(self.start));
            self.upscaler_changed = true;
            println!("{}", self.upscaler.line());
        }
        if input.pressed(KeyCode::KeyL) {
            self.light.shadows = !self.light.shadows;
            println!(
                "shadow ray {}",
                if self.light.shadows { "on" } else { "off" }
            );
        }
        let scroll = input.scroll() + f32::from(u8::from(input.pad_pressed(PadButton::DpadUp)))
            - f32::from(u8::from(input.pad_pressed(PadButton::DpadDown)));
        if scroll != 0.0
            && let Some(r) = scroll_dig_radius(self.dig_radius, &mut self.scroll, scroll)
        {
            self.dig_radius = r;
            println!("dig radius {:.1} m", r as f64 / (10 * FIXED_ONE) as f64);
        }
        let r2 = trigger_pressed(&mut self.triggers[1], input.axis(Axis::TriggerR));
        if input.clicked(MouseButton::Left) || r2 {
            self.dig();
        }
        let l2 = trigger_pressed(&mut self.triggers[0], input.axis(Axis::TriggerL));
        if input.clicked(MouseButton::Right) || l2 {
            self.fill();
        }
        if input.pressed(KeyCode::KeyB) {
            self.bench();
        }
        if input.pressed(KeyCode::KeyP) {
            self.stamp();
        }
        if input.pressed(KeyCode::KeyG) || input.pad_pressed(PadButton::North) {
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
            self.preset = self.preset.next(contour);
            self.look.normal_mode = self.preset.normal_mode;
            self.look.tuning = self.preset.tuning;
            self.preset_changed = true;
            self.modes
                .show_look(&self.preset, self.look.detail, ms_since(self.start));
            println!("{}", look_line(&self.look, self.preset()));
        }
        if input.pressed(KeyCode::KeyI)
            && let Some(shot) = self.castle_shot.map(castle::CastleShot::next)
        {
            self.castle_shot = Some(shot);
            self.fly.camera = Camera {
                vfov: self.fly.camera.vfov,
                ..castle_camera(shot)
            };
            let line = castle_banner(shot);
            println!("{line}");
            self.scene = Some((line, ms_since(self.start)));
        }
        if input.pressed(KeyCode::KeyX) || input.pad_pressed(PadButton::West) {
            self.cutaway = self.cutaway.next();
            println!("cutaway {}", self.cutaway.name());
        }
        if input.pressed(KeyCode::KeyV) {
            self.look.detail = !self.look.detail;
            self.modes
                .show_look(&self.preset, self.look.detail, ms_since(self.start));
            println!("{}", look_line(&self.look, self.preset()));
        }
        if input.pressed(KeyCode::F1) {
            self.samples = Some(Samples::default());
        }
        if let Some(audio) = self.audio.as_mut() {
            if input.pressed(KeyCode::KeyH) || input.pad_pressed(PadButton::East) {
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
            let steps = [
                (KeyCode::BracketLeft, -3.0, 0.0, 0.0),
                (KeyCode::BracketRight, 3.0, 0.0, 0.0),
                (KeyCode::Minus, 0.0, -3.0, 0.0),
                (KeyCode::Equal, 0.0, 3.0, 0.0),
                (KeyCode::Digit9, 0.0, 0.0, -10.0),
                (KeyCode::Digit0, 0.0, 0.0, 10.0),
            ];
            let mut mix = audio.mix;
            let mut touched = false;
            for (key, echo, tail, pre) in steps {
                if input.pressed(key) {
                    mix = mix.step(echo, tail, pre);
                    touched = true;
                }
            }
            if input.pressed(KeyCode::KeyO) {
                mix.glide = !mix.glide;
                touched = true;
            }
            if input.pressed(KeyCode::KeyU) {
                mix.direct_delay = !mix.direct_delay;
                touched = true;
            }
            if touched {
                audio.mix = mix;
                // A full channel drops it; the next key sends the whole balance again.
                if let Err(e) = audio.output.send(Command::SetMix(mix.settings())) {
                    println!("mix: {e}");
                }
                println!("{}", mix.line());
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
                    if p.mix.take_room(r.mean_free_path_m) {
                        println!("{}", p.mix.line());
                    }
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
        let mut sent_mfp = None;
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
                    sent_mfp = Some(r.mean_free_path_m);
                    if debug {
                        println!("{}", room_sent_line(r));
                    }
                }
                let _ = output.send(cmd);
            },
        );
        audio.update_ms = ms_since(t);
        if let Some(mfp) = sent_mfp
            && audio.mix.take_room(mfp)
        {
            println!("{}", audio.mix.line());
        }
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
        // The sync's copies are recorded first, so the attribute pass and the march read
        // them in the same submission.
        let mut enc = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let mut stream_ms = None;
        if self.stream.is_some() {
            if let Err(e) = self.stream_step(gpu, &mut enc) {
                eprintln!("mirror sync failed: {e}");
                return Flow::Exit;
            }
            stream_ms = self.stream.as_ref().and_then(|s| s.cpu_ms.last().copied());
        } else if let Some(gfx) = self.gfx.as_mut() {
            // The light radius follows the camera on the bounded level too.
            gfx.mirror.set_focus(voxel_of(self.fly.camera.position));
            let t = now();
            match gfx.mirror.sync(&self.world, &gpu.queue, &mut enc) {
                Ok(s) => {
                    let ms = ms_since(t);
                    self.hud.sync = (s, ms);
                    stream_ms = Some(ms);
                }
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
            self.sample_probes(gpu);
        }
        // The steps row reads the last frame's target back before this frame's march
        // overwrites it: a blocking copy, paid only with the HUD and that view both on.
        self.hud.steps = None;
        if hud
            && self.look.view == ViewMode::Steps
            && let Some(gfx) = &self.gfx
        {
            let px = readback::read_rgba8(&gpu.device, &gpu.queue, gfx.frame.target());
            let width = gfx.frame.target().width() as usize;
            let h = StepHistogram::from_rgba8(&px, width);
            self.hud.steps = Some(hud::StepsSample::from_histogram(&h));
        }
        // With the stats HUD off the overlay draws the mode banner alone while it shows.
        let banner = (!hud).then(|| self.banner()).flatten();
        if hud {
            self.update_hud(gpu);
        } else if let (Some(b), Some(gfx)) = (&banner, self.gfx.as_mut()) {
            gfx.hud
                .set_lines(&gpu.queue, b, &[HudColour::Yellow; hud::BANNER_ROWS + 1]);
            gfx.hud.set_samples(&gpu.queue, &[], 0.0);
        }
        let flying = self
            .pilot
            .as_ref()
            .is_some_and(|p| matches!(p.leg, Leg::Out | Leg::Back));

        if let Some(n) = self.args.lamp_toggle
            && self.frame > 0
            && self.frame.is_multiple_of(n)
        {
            self.modes.press(hud::ModeKey::Lamp, ms_since(self.start));
        }
        if self.args.fade_reading && self.modes.lighting.lamp != fade_lamp(self.frame) {
            self.modes.press(hud::ModeKey::Lamp, ms_since(self.start));
        }
        let Some(gfx) = self.gfx.as_mut() else {
            return Flow::Exit;
        };
        if std::mem::take(&mut self.upscaler_changed) {
            let (kind, scale, jitter) = self.args.upscaler_settings(self.upscaler.mode);
            let output = gfx.frame.output_size();
            gfx.frame.set_jitter(&gpu.device, jitter);
            gfx.frame.set_upscaler_kind(&gpu.device, kind);
            gfx.frame
                .set_sizes(&gpu.device, scale.internal(output), output);
        }
        gfx.frame.set_lighting(self.modes.lighting);
        gfx.frame.set_resolution(&gpu.device, self.modes.resolution);
        // A preset's fit refits the mirror's bricks under the attribute cap and its rock
        // detail rewrites the material rows; `--look plain` has neither.
        if std::mem::take(&mut self.preset_changed) && self.args.look.is_some() {
            gfx.mirror.set_surface_fit(self.preset.tuning.fit);
            gfx.mirror.rewrite_materials(
                &preset_materials(&self.args, Some(&self.preset)),
                &gpu.queue,
            );
        }
        self.modes.set_refitting(gfx.mirror.refitting());
        gfx.frame.set_look(self.look);
        let scale = if self.modes.pulse {
            ore_pulse(ms_since(self.start))
        } else {
            1.0
        };
        // The probes see the pulse at its average, so it opens no light-change window; the
        // pixels' light keeps its full swing.
        gfx.frame
            .set_emission_scales(&gpu.queue, flythrough::cave::ORE, scale, ORE_PULSE_MEAN);
        let params = gfx.frame.params(&gfx.mirror, &self.fly.camera, &self.light);
        let timer = gfx.timer.as_ref().filter(|_| {
            self.samples.is_some()
                || flying
                || hud
                || self.args.lamp_toggle.is_some()
                || self.args.fade_reading
        });
        let hud_timer = gfx.hud_timer.as_ref().filter(|_| hud);
        // Under the autopilot's flight the attribute pass is timed: its GPU time when its
        // work list is not empty, and the render thread's CPU time building the list and
        // recording the pass.
        let attr_timer = gfx.attr_timer.as_ref().filter(|_| flying);
        // The age is read before the call takes from the queue: how long the oldest waiting
        // brick has read face mode when this frame starts.
        let attr_age_ms = stamp_attr_call(&gfx.mirror, &mut gfx.attr_call_ms, ms_since(self.start));
        let t_attr = now();
        let attr_stats = gfx.mirror.prepare_attrs();
        let attr_build_ms = ms_since(t_attr);
        let t_encode = now();
        let attr_items = gfx
            .mirror
            .encode_attrs(&mut enc, attr_timer.map(MarchTimer::writes));
        let attr_encode_ms = ms_since(t_encode);
        let attr_timer = attr_timer.filter(|_| attr_items > 0);
        if let Some(t) = attr_timer {
            t.resolve(&mut enc);
        }
        gfx.frame.encode(
            &gpu.device,
            &gpu.queue,
            &mut enc,
            &gfx.mirror,
            &params,
            timer,
        );
        if let Some(t) = timer {
            t.resolve(&mut enc);
        }
        gfx.frame.advance();
        gfx.presenter
            .encode(&gpu.device, &mut enc, gfx.frame.target(), surface);
        if hud || banner.is_some() {
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
        let pass_ms = timer.map(|t| t.read_all_ms(&gpu.device));
        let gpu_ms = pass_ms.as_ref().map(|p| p.iter().sum::<f64>());
        let attr_gpu_ms = attr_timer.map(|t| t.read_ms(&gpu.device));
        let hud_ms = hud_timer.map(|t| t.read_ms(&gpu.device));
        self.hud.run.push(cpu_ms, stream_ms, gpu_ms);
        self.hud.cpu_ms = cpu_ms as f32;
        self.hud.march_ms = gpu_ms.map(|v| v as f32);
        self.hud.pass_ms.clone_from(&pass_ms);
        if self.args.lamp_toggle.is_some()
            && let Some(p) = &pass_ms
        {
            let window = usize::from(gfx.frame.probe_light_window());
            self.window_ms[window].push(probe_passes_ms(p));
        }
        if self.args.fade_reading && self.frame >= FADE_DIGS {
            self.fade
                .push(FadeSample::read(gpu, &gfx.frame, gpu_ms.unwrap_or(0.0)));
        }
        self.hud.hud_ms = hud_ms.map(|v| v as f32);
        if flying && let (Some(p), Some(ms)) = (self.pilot.as_mut(), gpu_ms) {
            p.gpu_ms.push(ms);
            if let Some(passes) = &pass_ms {
                p.pass_ms.resize(passes.len(), Vec::new());
                for (v, &ms) in p.pass_ms.iter_mut().zip(passes) {
                    v.push(ms);
                }
            }
        }
        if flying
            && gfx.mirror.surface_attrs()
            && let Some(p) = self.pilot.as_mut()
        {
            p.attr.push(
                attr_stats,
                attr_build_ms,
                attr_encode_ms,
                attr_gpu_ms,
                attr_age_ms,
            );
            let builds = gfx.mirror.attr_buffer_builds();
            p.attr.builds_first.get_or_insert(builds);
            p.attr.builds_last = builds;
        }
        if let Some(s) = self.samples.as_mut() {
            s.cpu_ms.push(cpu_ms);
            if let Some(ms) = gpu_ms {
                s.gpu_ms.push(ms);
            }
            if let Some(p) = &pass_ms {
                s.pass_ms.resize(p.len(), Vec::new());
                for (v, &ms) in s.pass_ms.iter_mut().zip(p) {
                    v.push(ms);
                }
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
        if self.args.fade_reading && self.frame >= FADE_END {
            for line in fade_report(&self.fade) {
                println!("{line}");
            }
            return Flow::Exit;
        }
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
                let px = readback::read_rgba8(&gpu.device, &gpu.queue, gfx.frame.target());
                let width = gfx.frame.target().width() as usize;
                println!("{}", steps_line(&StepHistogram::from_rgba8(&px, width)));
                let shadow =
                    readback::read_rgba16f(&gpu.device, &gpu.queue, gfx.frame.irradiance());
                println!(
                    "{}",
                    shadow_steps_line(&shadow, gfx.frame.lighting().shadow_walk)
                );
            }
            if let Some(gfx) = &self.gfx {
                println!("mirror: {}", belt_line(&gfx.mirror));
            }
            if let Some(n) = self.args.lamp_toggle {
                println!("{}", window_line(n, &self.window_ms));
            }
            self.sample_world();
            self.sample_probes(gpu);
            self.refresh_hud_stats();
            let mut stats = self.hud.stats.clone();
            stats.world.age_frames = 0;
            println!("hud:\n{stats}");
            return Flow::Exit;
        }
        Flow::Continue
    }
}

/// The steps view's reading of the lamp shadow rays, from the light pass's counters (one
/// texel per hit, alpha 1): `shadow rays (full walk, 1 234 567 rays): steps mean 8.12, loads
/// mean 20.31, p99 64, pool reads mean 3.10`.
fn shadow_steps_line(texels: &[[f32; 4]], walk: ShadowWalk) -> String {
    let rays: Vec<&[f32; 4]> = texels.iter().filter(|t| t[3] > 0.5).collect();
    let n = rays.len().max(1) as f64;
    let mean = |k: usize| rays.iter().map(|t| f64::from(t[k])).sum::<f64>() / n;
    let mut loads: Vec<f64> = rays.iter().map(|t| f64::from(t[1])).collect();
    format!(
        "shadow rays ({} walk, {} rays): steps mean {:.2}, loads mean {:.2}, p99 {:.0}, pool reads mean {:.2}",
        walk.name(),
        rays.len(),
        mean(0),
        mean(1),
        percentile(&mut loads, 99.0),
        mean(2)
    )
}

/// The ore's emission scale `ms` milliseconds into the run while `M` pulses it: between 0.25
/// and 1.75, one cycle every 1.5 s, about [`ORE_PULSE_MEAN`].
fn ore_pulse(ms: f64) -> f32 {
    (f64::from(ORE_PULSE_MEAN) + 0.75 * (ms / 1500.0 * std::f64::consts::TAU).sin()) as f32
}

/// The ore pulse's time average, the scale the probe rays see the ore at, pulsing or not.
const ORE_PULSE_MEAN: f32 = 1.0;

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

/// `look face, detail off, view shaded` under `--look plain`, or with the preset's name,
/// parts and rock's detail: `look contour-sharp (normals contour, fit weighted27, face mix
/// 0, detail after lighting on), detail on (rock scale 4, strength 0.1, fine 0), view
/// shaded`.
fn look_line(look: &LookParams, preset: Option<&LookPreset>) -> String {
    let on = if look.detail { "on" } else { "off" };
    let view = look.view.name();
    match preset {
        None => format!("look {}, detail {on}, view {view}", look.normal_mode.name()),
        Some(p) => {
            let [scale, strength] = p.rock_detail;
            format!(
                "look {} (normals {}, {}), detail {on} (rock scale {scale}, strength {strength}, fine {}), view {view}",
                p.name,
                look.normal_mode.name(),
                p.parts(),
                p.detail_fine
            )
        }
    }
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
        if let Some(line) = self.hud.run.line() {
            println!("{line}");
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
    /// The gamepad, `None` under `--pad off`, headless, the autopilot, or when `gilrs`
    /// cannot start.
    pad: Option<Pad>,
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
                            stages_ms: [0.0; 4],
                            light_ms: 0.0,
                            light_evicted: 0,
                            light_refused: 0,
                            light_wait_ms: 0.0,
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
                near: NearRing::new(self.args.residency.r_full),
                flight_frames: 0,
                flight_span_ms: None,
                unready_frames: 0,
                peak_belt: 0,
                gpu_ms: Vec::new(),
                cpu_ms: Vec::new(),
                stages_ms: Default::default(),
                light_ms: Vec::new(),
                pass_ms: Vec::new(),
                peak_light: 0,
                light_evicted: 0,
                light_refused: 0,
                light_wait_ms: 0.0,
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
                modes: hud::Modes::new(self.args.start_lighting(), self.args.lighting, 0.0),
                upscaler: hud::UpscalerBanner::shown_from(
                    self.args.start_upscaler(),
                    {
                        let mode = self.args.start_upscaler();
                        self.args.upscaler_settings(mode).1.name()
                    },
                    0.0,
                ),
                upscaler_changed: false,
                scene: self
                    .args
                    .castle
                    .then(|| (castle_banner(self.args.castle_shot), ms_since(self.start))),
                castle_shot: self.args.castle.then_some(self.args.castle_shot),
                look: self.args.start_look(),
                preset: self.args.look.unwrap_or(LookPreset::FACE),
                preset_changed: false,
                cutaway: self.args.cutaway,
                dig_radius: DIG_RADIUS,
                scroll: ScrollSteps::default(),
                triggers: [false; 2],
                bench_runs: 0,
                frame: 0,
                gfx: None,
                samples: None,
                window_ms: [Vec::new(), Vec::new()],
                fade: Vec::new(),
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

    fn poll(&mut self, input: &mut Input) {
        let Some(pad) = self.pad.as_mut() else {
            return;
        };
        for line in pad.poll(input) {
            println!("{line}");
        }
        if let Some(run) = self.run.as_mut() {
            pad.write_row(run.hud.stats.pad.get_or_insert_with(Default::default));
        }
    }

    fn frame(&mut self, gpu: &Gpu, input: &Input, dt: f32, surface: &wgpu::TextureView) -> Flow {
        match self.run.as_mut() {
            Some(run) => run.frame(gpu, input, dt, surface),
            None => Flow::Exit,
        }
    }
}

/// Runs `app` without a window: a device on the high-performance adapter, or with
/// `--adapter` the first adapter whose name contains its substring (the high-performance one
/// when none does), and an offscreen target in place of the surface, the same frames until
/// the app exits. Teardown follows the windowed shell's order (the app, then the queue,
/// device and adapter), traced alike. The instance reads wgpu's environment variables
/// (`WGPU_BACKEND`, `WGPU_DX12_COMPILER`, ...) as the windowed shell's does.
fn run_headless(mut app: Flythrough, size: [u32; 2]) -> Result<(), String> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let named = app.args.adapter.as_deref().and_then(|want| {
        let all = pollster_block_on(instance.enumerate_adapters(wgpu::Backends::all()));
        let found = all.into_iter().find(|a| a.get_info().name.contains(want));
        if found.is_none() {
            eprintln!("--adapter {want:?}: no adapter's name contains it; the default is used");
        }
        found
    });
    let adapter = match named {
        Some(a) => a,
        None => pollster_block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|e| format!("no adapter: {e}"))?,
    };
    let info = adapter.get_info();
    println!(
        "headless on {} ({})",
        info.name,
        vxe_app::backend_label(info.backend)
    );
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

/// The cave's material table; under `--look`, with rock's procedural detail from its preset;
/// the ore emissive unless `--emissive-ore off`.
fn materials(args: &Args) -> MaterialTable {
    preset_materials(args, args.look.as_ref())
}

/// [`materials`] with rock's detail from `preset`, or none for `--look plain`; the castle's
/// rows under `--castle`.
fn preset_materials(args: &Args, preset: Option<&LookPreset>) -> MaterialTable {
    let table = preset.map_or_else(cave_materials, LookPreset::materials);
    let table = if args.emissive_ore {
        flythrough::with_emissive_ore(&table)
    } else {
        table
    };
    if args.castle {
        flythrough::castle_materials(&table)
    } else {
        table
    }
}

/// The camera at castle shot `shot`: the eye voxel's centre, its yaw and pitch.
fn castle_camera(shot: castle::CastleShot) -> Camera {
    let (eye, yaw, pitch) = shot.pose();
    Camera {
        position: eye.map(|v| v as f32 + 0.5).into(),
        yaw: (yaw as f32).to_radians(),
        pitch: (pitch as f32).to_radians(),
        vfov: 70f32.to_radians(),
    }
}

/// The castle's banner row for `shot`: `castle gate`.
fn castle_banner(shot: castle::CastleShot) -> String {
    format!("castle {}", shot.name())
}

/// `--castle`'s source over the cave of `seed`: the castle generated, encoded (and written to
/// `--castle-out`), parsed, mapped and placed, each stage's time printed, then the prefabs'
/// bytes.
fn castle_source(
    seed: u64,
    out: Option<&std::path::Path>,
) -> vxe_world::StampedSource<flythrough::cave::HallSource> {
    let t = now();
    let file = castle::castle_vox();
    let listed: usize = file.models.iter().map(|m| m.voxels.len()).sum();
    println!(
        "castle: generated {} tiles, {listed} voxels listed, in {:.0} ms",
        file.models.len(),
        ms_since(t)
    );
    let t = now();
    let bytes = vxe_assets::encode(&file);
    drop(file);
    println!(
        "castle: encoded {} bytes of .vox in {:.0} ms",
        bytes.len(),
        ms_since(t)
    );
    if let Some(path) = out {
        match std::fs::write(path, &bytes) {
            Ok(()) => println!("castle: wrote {}", path.display()),
            Err(e) => eprintln!("flythrough: --castle-out {}: {e}", path.display()),
        }
    }
    let t = now();
    let file = vxe_assets::VoxFile::parse(&bytes).unwrap_or_else(|e| {
        eprintln!("flythrough: the generated castle does not parse: {e}");
        std::process::exit(1);
    });
    drop(bytes);
    println!("castle: parsed in {:.0} ms", ms_since(t));
    let t = now();
    let placements = castle::castle_prefabs(&file).unwrap_or_else(|e| {
        eprintln!("flythrough: the castle's palette map: {e}");
        std::process::exit(1);
    });
    drop(file);
    let bytes = castle::prefab_bytes(placements.iter().map(|(p, _, _)| &**p));
    println!(
        "castle: mapped {} prefabs, {bytes} bytes, in {:.0} ms",
        placements.len(),
        ms_since(t)
    );
    let t = now();
    let source = castle::castle_source(seed, placements);
    println!("castle: placed in {:.0} ms", ms_since(t));
    source
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
    println!("{}", look_line(&args.start_look(), args.look.as_ref()));
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
        let mut streamer = if args.castle {
            Streamer::new(
                Arc::new(castle_source(seed, args.castle_out.as_deref())),
                cfg,
            )
        } else if args.vault {
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

    let camera = if args.castle {
        castle_camera(args.castle_shot)
    } else {
        Camera {
            position: spawn.0.map(|v| v as f32 + 0.5).into(),
            yaw: 0.0,
            pitch: 0.0,
            vfov: 70f32.to_radians(),
        }
    };
    let size = args.render.unwrap_or([1280, 720]);
    let headless = args.headless;
    if !headless && args.adapter.is_some() {
        eprintln!("--adapter applies to --headless runs; the window takes the default adapter");
    }
    let audio = if args.audio {
        Audio::open(
            args.audio_debug,
            args.absorption,
            args.mic,
            args.room_line,
            args.mix,
        )
    } else {
        None
    };
    let room_probe =
        (audio.is_none() && args.room_line).then(|| RoomProbe::new(args.absorption, args.mix));
    if args.audio || args.room_line {
        println!("{}", args.mix.line());
    }
    let pad = if args.wants_pad() { Pad::open() } else { None };
    if !headless && !args.autopilot {
        println!(
            "{}",
            pad.as_ref()
                .map_or_else(|| "pad: none".to_owned(), Pad::startup_line)
        );
    }
    let app = Flythrough {
        start,
        args,
        camera,
        logic: Some(logic),
        pending: Some(pending),
        run: None,
        audio,
        room_probe,
        pad,
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
        hud.stats.pad = Some(hud::PadRow {
            name: "PS5 Controller".to_owned(),
            power: Some(hud::PadPower::Charging(5)),
        });
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
        assert_eq!(hud.text.lines().count(), 13);
        assert_eq!(hud.strip.len(), 100);
    }
    use flythrough::cave::SHELL;

    fn argv(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| (*s).to_owned()).collect()
    }

    /// `--walk` and `G` refuse walk mode alike while streaming; the bounded level allows it.
    /// The unready-near ring follows the streamer's `r_full`: with chunks within 10 of the
    /// camera ready, the ring at the default 16 (offsets within 14) finds one unready; after the
    /// pressure rule's `set_radii` lowers `r_full` to 12, the recomputed ring (within 10) finds
    /// none, and a repeated radius recomputes nothing.
    #[test]
    fn the_unready_ring_is_recomputed_on_set_radii() {
        let mut streamer = Streamer::new(
            Arc::new(CaveSource::unbounded(1)),
            StreamConfig {
                residency: RESIDENCY,
                workers: 1,
                persist: PersistConfig::default(),
            },
        );
        let mut ring = NearRing::new(streamer.config().residency.r_full);
        assert_eq!(ring.offsets, ball(14 * 14));
        let cam = ChunkCoord([3, -2, 5]);
        let ready = |c: ChunkCoord| {
            (0..3)
                .map(|k| i64::from(c.0[k] - cam.0[k]).pow(2))
                .sum::<i64>()
                <= 100
        };
        assert!(ring.unready(cam, ready));
        streamer.set_radii(12, RESIDENCY.r_lod);
        ring.follow(streamer.config().residency.r_full);
        assert_eq!(ring.recomputes, 1);
        assert_eq!(ring.r_full, 12);
        assert_eq!(ring.offsets, ball(10 * 10));
        assert!(!ring.unready(cam, ready));
        ring.follow(streamer.config().residency.r_full);
        assert_eq!(ring.recomputes, 1);
    }

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

    /// `--pad` takes `on` or `off`, defaults on, and refuses anything else; headless and
    /// the autopilot never read a pad.
    #[test]
    fn pad_parses_on_and_off() {
        let a = parse_args_from(argv(&[])).expect("no arguments parse");
        assert!(a.pad && a.wants_pad());
        let on = parse_args_from(argv(&["--pad", "on"])).expect("on parses");
        assert!(on.pad);
        let off = parse_args_from(argv(&["--pad", "off"])).expect("off parses");
        assert!(!off.pad && !off.wants_pad());
        let e = parse_args_from(argv(&["--pad", "x"]))
            .err()
            .expect("x refused");
        assert_eq!(e, "--pad takes on or off, not x");
        assert!(parse_args_from(argv(&["--pad"])).is_err());
        let amd = parse_args_from(argv(&["--headless", "--adapter", "AMD"])).unwrap();
        assert_eq!(amd.adapter.as_deref(), Some("AMD"));
        assert!(parse_args_from(argv(&["--adapter"])).is_err());
        let headless = parse_args_from(argv(&["--pad", "on", "--headless"])).unwrap();
        assert!(!headless.wants_pad());
        let pilot = parse_args_from(argv(&["--autopilot"])).unwrap();
        assert!(!pilot.wants_pad());
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

    /// `--castle` streams: the bounded level is too short for it, the vault's origin is
    /// inside its box, and `--room` builds its own world. `--castle-out` implies it.
    #[test]
    fn castle_refuses_the_level_the_vault_and_the_room() {
        assert!(parse_args_from(argv(&["--castle"])).unwrap().castle);
        for other in [&["--level"][..], &["--vault"], &["--room", "12"]] {
            let mut a = vec!["--castle"];
            a.extend_from_slice(other);
            assert!(parse_args_from(argv(&a)).is_err(), "{a:?}");
        }
        let a = parse_args_from(argv(&["--castle-shot", "nave"])).unwrap();
        assert!(a.castle);
        assert_eq!(a.castle_shot, castle::CastleShot::Nave);
        assert!(parse_args_from(argv(&["--castle-shot", "keep"])).is_err());
        let a = parse_args_from(argv(&["--castle-out", "c.vox"])).unwrap();
        assert!(a.castle);
        assert_eq!(a.castle_out.as_deref(), Some(std::path::Path::new("c.vox")));
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

    /// The balance flags parse within their ranges, set the mixer's settings and print the
    /// mix line; the keys' steps stop at the ranges' ends (the trims at -24 and +12 dB over
    /// the room curves, the absolute ranges under `--flat-mix`).
    #[test]
    fn the_balance_flags_and_steps() {
        let a = parse_args_from(argv(&[
            "--room",
            "12",
            "--echo-db",
            "6",
            "--tail-db",
            "-9",
            "--predelay-ms",
            "30",
            "--tail-glide",
            "--direct-delay",
        ]))
        .expect("parses");
        assert_eq!(
            a.mix.line(),
            "mix: echoes +6 dB (room +6.0 dB before any room), tail -9 dB (room +0.0 dB), \
             pre-delay 30 ms, tail lines glide, direct delay on"
        );
        let s = a.mix.settings();
        assert!((s.echo_gain - 1.9953).abs() < 1e-4);
        assert!((s.reverb.level - 0.3548).abs() < 1e-4);
        assert_eq!(s.reverb.predelay_frames(), 1_440);
        assert_eq!(s.reverb.lines, TailLines::Glide);
        assert!(s.direct_delay);
        assert_eq!(s.echo_by_room, MixSettings::default().echo_by_room);
        assert_eq!(Mix::default().settings(), MixSettings::default());

        let flat = parse_args_from(argv(&["--flat-mix", "--echo-db", "9"])).expect("parses");
        assert_eq!(
            flat.mix.line(),
            "mix: echoes +9 dB (room flat), tail +0 dB (room flat), pre-delay 0 ms, tail lines \
             handover, direct delay off"
        );
        assert_eq!(
            flat.mix.settings(),
            MixSettings {
                echo_gain: 10f32.powf(9.0 / 20.0),
                ..MixSettings::flat()
            }
        );
        // The Phase 12 picks replay under --flat-mix; over the curves the trims stop at +12.
        assert!(parse_args_from(argv(&["--flat-mix", "--echo-db", "30"])).is_ok());
        assert!(parse_args_from(argv(&["--echo-db", "30", "--flat-mix"])).is_ok());
        for bad in [
            &["--echo-db", "13"][..],
            &["--tail-db", "-25"],
            &["--echo-db", "-25"],
            &["--tail-db", "13"],
            &["--flat-mix", "--echo-db", "31"],
            &["--flat-mix", "--tail-db", "7"],
            &["--predelay-ms", "-1"],
            &["--predelay-ms", "x"],
            &["--echo-db", "inf"],
        ] {
            assert!(parse_args_from(argv(bad)).is_err(), "{bad:?}");
        }
        let top = Mix::default().step(40.0, 40.0, 400.0);
        assert_eq!(
            (top.echo_db, top.tail_db, top.predelay_ms),
            (12.0, 12.0, 200.0)
        );
        let bottom = Mix::default().step(-40.0, -40.0, -10.0);
        assert_eq!(
            (bottom.echo_db, bottom.tail_db, bottom.predelay_ms),
            (-24.0, -24.0, 0.0)
        );
        let flat_top = flat.mix.step(40.0, 40.0, 0.0);
        assert_eq!((flat_top.echo_db, flat_top.tail_db), (30.0, 6.0));
        let flat_bottom = flat.mix.step(-40.0, -40.0, 0.0);
        assert_eq!((flat_bottom.echo_db, flat_bottom.tail_db), (-12.0, -30.0));
    }

    /// A room prints the mix line again when it moves either curve by 0.5 dB or more since the
    /// last printed line; under `--flat-mix` never.
    #[test]
    fn a_room_reprints_the_mix_line_when_a_curve_moves() {
        let mut mix = Mix::default();
        assert!(
            mix.take_room(2.666),
            "no room (+6 dB) to the 4 m room (+30 dB)"
        );
        assert_eq!(
            mix.line(),
            "mix: echoes +0 dB (room +30.0 dB at mfp 2.666 m), tail +0 dB (room -15.0 dB), \
             pre-delay 0 ms, tail lines handover, direct delay off"
        );
        assert!(!mix.take_room(2.6), "both curves hold below the first knot");
        assert!(mix.take_room(7.999));
        assert!((mix.curve_db(mix.room).0 - 15.0).abs() < 0.01);
        assert!(!mix.take_room(8.1), "+15.0 to +14.9 dB");
        assert!(
            mix.take_room(10.0),
            "+15.0 to +14.4 dB since the printed line"
        );
        let mut flat = Mix {
            flat: true,
            ..Mix::default()
        };
        assert!(!flat.take_room(2.666));
        assert_eq!(flat.room, Some(2.666));
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

    /// A fly-mode run on `world` with no window, GPU, stream or audio, its camera at the
    /// origin voxel's centre looking along +x, the native logic on its server.
    fn bare_run(world: World) -> Run {
        let world = Arc::new(world);
        let net = native_net(&world);
        let args = parse_args_from(argv(&["--level"])).expect("--level parses");
        let modes = hud::Modes::new(args.start_lighting(), args.lighting, 0.0);
        let camera = Camera {
            position: Vec3::splat(0.5),
            yaw: 0.0,
            pitch: 0.0,
            vfov: 1.0,
        };
        Run {
            start: now(),
            world,
            net: Some(net),
            spawn: voxel_of(camera.position),
            look: args.start_look(),
            preset: args.look.unwrap_or(LookPreset::FACE),
            preset_changed: false,
            args,
            fly: Fly::new(camera),
            light: LightParams::headlamp(),
            modes,
            upscaler: hud::UpscalerBanner::new(hud::UpscalerMode::Off, "1"),
            upscaler_changed: false,
            scene: None,
            castle_shot: None,
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
            cutaway: CutawayMode::Auto,
        }
    }

    /// Rock over chunks `[-1, 1)³` with a 16-voxel box carved around the origin.
    fn carved_rock() -> World {
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

    /// The edits one server tick of `run` applied.
    fn tick_edits(run: &mut Run) -> Vec<EditResult> {
        let net = run.net.as_mut().unwrap();
        assert_eq!(net.server.tick().logic_error, None);
        applied(&net.client.poll())
            .into_iter()
            .map(|r| match r {
                OpResult::Edit(e) => e,
                other => panic!("{other:?}"),
            })
            .collect()
    }

    /// `handle_input` digs on a left click and on `R2` pulled through `Input`'s setters
    /// alike: the dig reaches the server, which carves the same rock from the wall ahead;
    /// `R2` held on the next frame does not dig again, and released and pulled again does.
    /// `L2` fills.
    #[test]
    fn r2_digs_as_the_left_click_does_and_l2_fills() {
        let mut click = bare_run(carved_rock());
        let mut input = Input::default();
        input.button_down(MouseButton::Left);
        click.handle_input(&input);
        let by_click = tick_edits(&mut click);
        let [r] = &by_click[..] else {
            panic!("{by_click:?}")
        };
        assert_eq!(r.changed_from.len(), 1);
        assert_eq!(r.changed_from[0].0, ROCK);
        assert!(r.changed_from[0].1 > 0);

        let mut pad = bare_run(carved_rock());
        let mut input = Input::default();
        input.set_axis(Axis::TriggerR, 1.0);
        pad.handle_input(&input);
        assert_eq!(tick_edits(&mut pad), by_click);
        input.end_frame();
        pad.handle_input(&input);
        assert_eq!(tick_edits(&mut pad), [], "a held trigger digs once");
        input.set_axis(Axis::TriggerR, 0.0);
        pad.handle_input(&input);
        input.set_axis(Axis::TriggerR, 0.8);
        pad.handle_input(&input);
        assert_eq!(tick_edits(&mut pad).len(), 1, "pulled again, it digs again");

        let mut input = Input::default();
        input.set_axis(Axis::TriggerL, 1.0);
        pad.handle_input(&input);
        let filled = tick_edits(&mut pad);
        let [f] = &filled[..] else {
            panic!("{filled:?}")
        };
        assert_eq!(f.changed_from.len(), 1);
        assert_eq!(f.changed_from[0].0, 0, "the fill writes rock over air");
    }

    /// The D-pad steps the dig radius by a scroll line's step: up twice raises it from 0.5 m
    /// to 0.7 m, down once lowers it to 0.6 m.
    #[test]
    fn the_dpad_steps_the_dig_radius() {
        let mut run = bare_run(carved_rock());
        let mut input = Input::default();
        for _ in 0..2 {
            input.pad_down(PadButton::DpadUp);
            run.handle_input(&input);
            input.end_frame();
            input.pad_up(PadButton::DpadUp);
        }
        assert_eq!(run.dig_radius, DIG_RADIUS + 2 * DIG_STEP);
        input.pad_down(PadButton::DpadDown);
        run.handle_input(&input);
        assert_eq!(run.dig_radius, DIG_RADIUS + DIG_STEP);
    }

    /// With the castle, `I` moves the camera to the next shot's pose and puts its name on the
    /// banner: gate, nave, window, dungeon, gate. Without it, `I` does nothing.
    #[test]
    fn i_cycles_the_castle_shots() {
        let mut run = bare_run(carved_rock());
        let mut input = Input::default();
        input.key_down(KeyCode::KeyI);
        run.handle_input(&input);
        assert!(run.scene.is_none() && run.fly.camera.position == Vec3::splat(0.5));
        run.castle_shot = Some(castle::CastleShot::Gate);
        for want in ["nave", "window", "dungeon", "gate"] {
            input.end_frame();
            input.key_up(KeyCode::KeyI);
            input.key_down(KeyCode::KeyI);
            run.handle_input(&input);
            let shot = run.castle_shot.unwrap();
            assert_eq!(shot.name(), want);
            let (eye, _, _) = shot.pose();
            assert_eq!(
                run.fly.camera.position,
                Vec3::from(eye.map(|v| v as f32 + 0.5))
            );
            assert_eq!(run.scene.as_ref().unwrap().0, format!("castle {want}"));
            assert!(
                run.banner()
                    .unwrap()
                    .starts_with(&format!("castle {want}\n"))
            );
        }
        let (_, yaw, pitch) = castle::CastleShot::Nave.pose();
        let nave = castle_camera(castle::CastleShot::Nave);
        assert_eq!(
            (nave.yaw, nave.pitch),
            ((yaw as f32).to_radians(), (pitch as f32).to_radians())
        );
    }

    /// `X` and the pad's `West` (Square) cycle the cutaway alike: auto, on, off, auto.
    #[test]
    fn x_and_west_cycle_the_cutaway() {
        for press in [
            (|i: &mut Input| i.key_down(KeyCode::KeyX)) as fn(&mut Input),
            |i: &mut Input| i.pad_down(PadButton::West),
        ] {
            let mut run = bare_run(carved_rock());
            let mut input = Input::default();
            let mut seen = Vec::new();
            for _ in 0..3 {
                press(&mut input);
                run.handle_input(&input);
                seen.push(run.cutaway);
                input.end_frame();
                input.key_up(KeyCode::KeyX);
                input.pad_up(PadButton::West);
            }
            assert_eq!(seen, [CutawayMode::On, CutawayMode::Off, CutawayMode::Auto]);
        }
    }

    #[test]
    fn a_trigger_presses_past_half_and_lets_go_under_three_tenths() {
        let mut held = false;
        let pulls = [0.4, 0.5, 0.9, 0.35, 0.6, 0.25, 0.55];
        let presses = pulls.map(|p| trigger_pressed(&mut held, p));
        assert_eq!(presses, [false, true, false, false, false, false, true]);
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

    /// [`ledge_world`] with the pillar's top cell cut to a 32-voxel sliver
    /// `[32, 32, 39]..[36, 40, 40]` of `sliver` (the pillar stays full below z = 32), and an
    /// 8-voxel rock lump `[18, 36, 42]..[20, 38, 44]` in cell `(2, 4, 5)`, beside the ledge
    /// without touching it.
    fn sliver_ledge_world(sliver: MaterialId) -> World {
        let world = ledge_world();
        let f = |v: [i32; 3]| Fixed3(v.map(|c| i64::from(c) * FIXED_ONE));
        for (lo, hi, mode) in [
            ([32, 32, 32], [40, 40, 40], Mode::Carve),
            ([32, 32, 39], [36, 40, 40], Mode::Fill(sliver)),
            ([18, 36, 42], [20, 38, 44], Mode::Fill(ROCK)),
        ] {
            world.apply(&EditOp::box_(f(lo), f(hi), mode)).unwrap();
        }
        world
    }

    /// A dig of radius 2 voxels from the centre of [`sliver_ledge_world`]'s lump: the ray
    /// starts in rock, so the capsule is a sphere carving the lump's 8 voxels.
    fn lump_dig() -> DigInput {
        DigInput::Dig {
            origin: Fixed3([19 * FIXED_ONE, 37 * FIXED_ONE, 43 * FIXED_ONE]),
            dir: [FIXED_ONE, 0, 0],
            radius: 2 * FIXED_ONE,
        }
    }

    /// The query beside the ledge reports it (it rests on a cell of 32 voxels, half the
    /// threshold), and it falls through the sliver: the slab's bottom is z 40, the sliver at
    /// z 39 yields, z 38 to 32 is air and the pillar's full top cell holds z 31. Tick 1 drops 8
    /// and crushes the 32 sliver voxels; tick 2 drops 0 onto the pillar, and the logic piles
    /// the 32 as rubble on the landed slab: `C` = 576 columns, every 18th takes one voxel on
    /// the slab's top at z 39, so all 32 are at z 40.
    #[test]
    fn a_ledge_on_a_sliver_falls_through_it_and_piles_what_it_crushed() {
        let world = Arc::new(sliver_ledge_world(ROCK));
        let mut net = native_net(&world);
        let (lines, events) = net_tick(&mut net, &[lump_dig()]);
        assert_eq!(lines, ["op carved", "region 9 1:4608"]);
        let [OpResult::Edit(r)] = &applied(&events)[..] else {
            panic!("{events:?}");
        };
        assert_eq!(r.changed_from, [(ROCK, 8)]);
        let (lines, events) = net_tick(&mut net, &[]);
        assert_eq!(lines, ["op dropped"]);
        let [OpResult::Drop(d)] = &applied(&events)[..] else {
            panic!("{events:?}");
        };
        assert_eq!((d.dropped, d.crushed.as_slice()), (8, &[(ROCK, 32)][..]));
        let (lines, events) = net_tick(&mut net, &[]);
        assert_eq!(
            lines,
            [
                "op dropped",
                "op piled",
                "landed after 1 ticks, fell 8 voxels, crushed 32 voxels, 32 rubble"
            ]
        );
        let [OpResult::Drop(d), OpResult::Pile(p)] = &applied(&events)[..] else {
            panic!("{events:?}");
        };
        assert_eq!(d.dropped, 0);
        assert_eq!(p.placed, 32);
        assert_eq!(world.material_at([25, 26, 40]), Some(flythrough::RUBBLE));
        assert_eq!(
            world.count_in_box([24, 24, 32], [47, 47, 39]).unwrap(),
            [(ROCK, 4608)]
        );
        // `count_in_box` counts air too: 576 voxels in the layer, 32 of them rubble.
        assert_eq!(
            world.count_in_box([24, 24, 40], [47, 47, 40]).unwrap(),
            [(vxe_world::AIR, 544), (flythrough::RUBBLE, 32)]
        );
        let kinds: Vec<u8> = net
            .server
            .ops()
            .entries()
            .iter()
            .filter(|e| e.op().is_some())
            .map(|e| {
                let mut out = Vec::new();
                e.encode(&mut out);
                out[12]
            })
            .collect();
        assert_eq!(kinds, [1, 5, 5, 6]);
        assert_eq!(*net.server.coarse(), CoarseGrid::build(&world));
        // Nothing falls on.
        let (lines, events) = net_tick(&mut net, &[]);
        assert!(
            lines.is_empty() && events.is_empty(),
            "{lines:?} {events:?}"
        );
    }

    /// With the sliver made of the indestructible shell the ledge cannot move: its first tick
    /// prints `wedged`, and the logic remembers it. A second lump dig beside it reports
    /// nothing; a dig that takes voxels from the ledge reports it again with its new counts,
    /// and it is wedged again.
    #[test]
    fn a_ledge_on_a_shell_sliver_is_wedged_and_reported_again_only_once_changed() {
        let world = Arc::new(sliver_ledge_world(SHELL));
        let table = world.materials().clone();
        let mut net = native_net(&world);
        let (lines, _) = net_tick(&mut net, &[lump_dig()]);
        assert_eq!(lines, ["op carved", "region 9 1:4608"]);
        let (lines, events) = net_tick(&mut net, &[]);
        assert_eq!(lines, ["op dropped", "wedged 9 1:4608"]);
        assert_eq!(
            wedged_terminal_line(&table, &lines[1]).as_deref(),
            Some("wedged: 9 cells, {rock: 4608}, cannot fall")
        );
        let [OpResult::Drop(d)] = &applied(&events)[..] else {
            panic!("{events:?}");
        };
        assert_eq!(d.dropped, 0);
        let (lines, events) = net_tick(&mut net, &[]);
        assert!(
            lines.is_empty() && events.is_empty(),
            "{lines:?} {events:?}"
        );

        // A second lump in cell (6, 4, 5), beside the ledge, written past the server (its
        // grid takes the cell back from the dig's aabb): dug, and the ledge the query reports
        // is the remembered one.
        let f = |v: [i32; 3]| Fixed3(v.map(|c| i64::from(c) * FIXED_ONE));
        world
            .apply(&EditOp::box_(
                f([50, 36, 42]),
                f([52, 38, 44]),
                Mode::Fill(ROCK),
            ))
            .unwrap();
        let second = DigInput::Dig {
            origin: Fixed3([51 * FIXED_ONE, 37 * FIXED_ONE, 43 * FIXED_ONE]),
            dir: [FIXED_ONE, 0, 0],
            radius: 2 * FIXED_ONE,
        };
        let (lines, events) = net_tick(&mut net, &[second]);
        assert_eq!(lines, ["op carved"]);
        let [OpResult::Edit(r)] = &applied(&events)[..] else {
            panic!("{events:?}");
        };
        assert_eq!(r.changed_from, [(ROCK, 8)]);
        let (lines, events) = net_tick(&mut net, &[]);
        assert!(
            lines.is_empty() && events.is_empty(),
            "{lines:?} {events:?}"
        );

        // A dig into the ledge's corner changes its counts: reported, and wedged, again.
        let corner = DigInput::Dig {
            origin: Fixed3([25 * FIXED_ONE, 25 * FIXED_ONE, 46 * FIXED_ONE]),
            dir: [FIXED_ONE, 0, 0],
            radius: 2 * FIXED_ONE,
        };
        let (lines, events) = net_tick(&mut net, &[corner]);
        let [OpResult::Edit(r)] = &applied(&events)[..] else {
            panic!("{events:?}");
        };
        let left = 4608 - r.changed_from[0].1;
        assert_eq!(
            lines,
            ["op carved".to_owned(), format!("region 9 1:{left}")]
        );
        let (lines, _) = net_tick(&mut net, &[]);
        assert_eq!(
            lines,
            ["op dropped".to_owned(), format!("wedged 9 1:{left}")]
        );
    }

    /// A ledge with no floor under it in the one-chunk world falls five ticks of 8, z 40 to 0;
    /// on tick 6 the scan reaches chunk `[0, 0, -1]`, outside the extent, and the drop is
    /// refused. It crushed nothing, so no pile follows, and no later tick drops it again.
    #[test]
    fn a_ledge_falling_out_of_the_world_is_refused_once() {
        let world = Arc::new(World::generate(
            ChunkExtent::new([0, 0, 0], [1, 1, 1]),
            &vxe_world::UniformSource(vxe_world::AIR),
            &cave_materials(),
        ));
        let f = |v: [i32; 3]| Fixed3(v.map(|c| i64::from(c) * FIXED_ONE));
        for (lo, hi) in [([24, 24, 40], [48, 48, 48]), ([18, 36, 42], [20, 38, 44])] {
            world
                .apply(&EditOp::box_(f(lo), f(hi), Mode::Fill(ROCK)))
                .unwrap();
        }
        let mut net = native_net(&world);
        let (lines, _) = net_tick(&mut net, &[lump_dig()]);
        assert_eq!(lines, ["op carved", "region 9 1:4608"]);
        for tick in 1..=5 {
            let (lines, _) = net_tick(&mut net, &[]);
            assert_eq!(lines, ["op dropped"], "tick {tick}");
        }
        let (lines, events) = net_tick(&mut net, &[]);
        assert_eq!(
            lines,
            [
                "op dropped",
                "drop refused: chunk [0, 0, -1] is not resident, after 5 ticks, fell 40 voxels"
            ]
        );
        assert!(applied(&events).is_empty(), "{events:?}");
        assert!(
            events.iter().any(|e| matches!(
                e,
                Event::Refused {
                    op: vxe_world::Op::Drop { .. },
                    ..
                }
            )),
            "{events:?}"
        );
        let (lines, events) = net_tick(&mut net, &[]);
        assert!(
            lines.is_empty() && events.is_empty(),
            "{lines:?} {events:?}"
        );
        assert_eq!(
            world.count_in_box([24, 24, 0], [47, 47, 7]).unwrap(),
            [(ROCK, 4608)]
        );
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
    fn cutaway_parses_auto_on_and_off() {
        assert_eq!(
            parse_args_from(argv(&[])).unwrap().cutaway,
            CutawayMode::Auto
        );
        for (name, mode) in [
            ("auto", CutawayMode::Auto),
            ("on", CutawayMode::On),
            ("off", CutawayMode::Off),
        ] {
            assert_eq!(
                parse_args_from(argv(&["--cutaway", name])).unwrap().cutaway,
                mode
            );
        }
        assert_eq!(
            parse_args_from(argv(&["--cutaway", "x"])).err().as_deref(),
            Some("--cutaway takes auto, on or off, not x")
        );
    }

    /// Every lighting flag parses its values and refuses an unknown one.
    #[test]
    fn lighting_flags_parse_and_refuse_unknown_values() {
        let parse = |a: &[&str]| parse_args_from(argv(a));
        let a = parse(&[
            "--emissive-ore",
            "off",
            "--lamp",
            "off",
            "--filter",
            "off",
            "--indirect",
            "off",
        ])
        .unwrap();
        assert!(!a.emissive_ore && !a.lamp && !a.filter && !a.indirect);
        let a = parse(&[
            "--shadow-walk",
            "coarse",
            "--sampler",
            "list",
            "--history",
            "gradient",
            "--lighting",
            "full",
            "--light-radius",
            "6",
        ])
        .unwrap();
        assert_eq!(a.shadow_walk, ShadowWalk::Coarse);
        assert_eq!(a.sampler, Sampler::List);
        assert_eq!(a.history, HistoryKill::Gradient);
        assert_eq!(a.lighting, LightingResolution::Full);
        assert_eq!(a.light_radius, 6);
        let d = parse(&[]).unwrap();
        assert!(d.emissive_ore && d.lamp && d.filter && d.indirect);
        assert!(d.start_lighting().indirect);
        assert_eq!(d.probe_spacing, 16);
        assert_eq!(parse(&["--probe-spacing", "8"]).unwrap().probe_spacing, 8);
        let p = parse(&["--probe-rays", "32", "--probe-slices", "16"]).unwrap();
        assert_eq!(
            (p.start_lighting().rays(), p.start_lighting().slices()),
            (32, 16)
        );
        assert!(parse(&["--probe-slices", "12"]).is_err());
        assert_eq!(
            (
                d.shadow_walk,
                d.sampler,
                d.history,
                d.lighting,
                d.light_radius
            ),
            (
                ShadowWalk::Coarse,
                Sampler::Reuse,
                HistoryKill::Edits,
                LightingResolution::Half,
                4
            )
        );
        // The line the fly-through prints at the start with no lighting flags.
        assert_eq!(
            hud::Modes::new(d.start_lighting(), d.lighting, 0.0).line(),
            "lighting half, shadow walk coarse, sampler reuse, history edits, lamp on, indirect on at 64 rays 1/8, filter on, ore pulse off, emissive history 32"
        );
        for (flag, err) in [
            ("--emissive-ore", "--emissive-ore takes on or off, not x"),
            ("--lamp", "--lamp takes on or off, not x"),
            ("--indirect", "--indirect takes on or off, not x"),
            (
                "--probe-spacing",
                "--probe-spacing takes 4 to 64 voxels, not x",
            ),
            ("--probe-rays", "--probe-rays takes 1 to 64 rays, not x"),
            ("--probe-slices", "--probe-slices takes 8, 16 or 32, not x"),
            ("--filter", "--filter takes on or off, not x"),
            ("--shadow-walk", "--shadow-walk takes full or coarse, not x"),
            ("--sampler", "--sampler takes list or reuse, not x"),
            ("--history", "--history takes edits or gradient, not x"),
            ("--lighting", "--lighting takes full or half, not x"),
            (
                "--light-radius",
                "--light-radius takes 0 to 64 chunks, not x",
            ),
            (
                "--emissive-history",
                "--emissive-history takes 1 to 4096 frames, not x",
            ),
        ] {
            assert_eq!(parse(&[flag, "x"]).err().as_deref(), Some(err), "{flag}");
        }
    }

    /// `--internal-scale`, `--jitter` and `--sharpen` parse their values, default to 1, off
    /// and 0.5, and refuse an unknown value.
    #[test]
    fn internal_scale_and_jitter_parse_and_refuse_unknown_values() {
        let parse = |a: &[&str]| parse_args_from(argv(a));
        let d = parse(&[]).unwrap();
        assert_eq!((d.internal_scale, d.jitter), (RenderScale::One, false));
        assert_eq!(d.sharpen, 0.5);
        assert_eq!(parse(&["--sharpen", "0"]).unwrap().sharpen, 0.0);
        assert_eq!(parse(&["--sharpen", "0.8"]).unwrap().sharpen, 0.8);
        for (name, scale) in [
            ("1", RenderScale::One),
            ("1.5", RenderScale::OneHalf),
            ("2", RenderScale::Two),
        ] {
            let a = parse(&["--internal-scale", name, "--jitter", "on"]).unwrap();
            assert_eq!((a.internal_scale, a.jitter), (scale, true));
        }
        assert!(!parse(&["--jitter", "off"]).unwrap().jitter);
        assert!(parse(&["--jitter", "on"]).unwrap().jitter);
        assert_eq!(
            RenderScale::OneHalf.internal([1920, 1080]),
            [1280, 720],
            "1.5 renders 720p for 1080p"
        );
        assert_eq!(RenderScale::OneHalf.internal([384, 216]), [256, 144]);
        assert_eq!(RenderScale::Two.internal([1920, 1080]), [960, 540]);
        for (flag, v, err) in [
            (
                "--internal-scale",
                "3",
                "--internal-scale takes 1, 1.5 or 2, not 3",
            ),
            (
                "--internal-scale",
                "1.0",
                "--internal-scale takes 1, 1.5 or 2, not 1.0",
            ),
            ("--jitter", "x", "--jitter takes on or off, not x"),
            ("--sharpen", "1.5", "--sharpen takes 0 to 1, not 1.5"),
            ("--sharpen", "x", "--sharpen takes 0 to 1, not x"),
            (
                "--upscaler",
                "fsr",
                "--upscaler takes off, own or fsr2, not fsr",
            ),
        ] {
            assert_eq!(parse(&[flag, v]).err().as_deref(), Some(err), "{flag} {v}");
        }
    }

    /// `--upscaler` picks the frame's upscaler, scale and jitter: `off` is scale 1 with the
    /// jitter off whatever `--internal-scale` says; `own` jitters unless `--jitter off`; FSR 2
    /// takes `--internal-scale` and `--sharpen`. Without it the old flags decide.
    #[test]
    fn upscaler_picks_the_kind_scale_and_jitter() {
        let parse = |a: &[&str]| parse_args_from(argv(a));
        let settings = |a: &[&str]| {
            let args = parse(a).unwrap();
            let (kind, scale, jitter) = args.upscaler_settings(args.start_upscaler());
            (kind.name(), scale, jitter)
        };
        assert_eq!(settings(&[]), ("off", RenderScale::One, false));
        assert_eq!(
            settings(&["--internal-scale", "1.5", "--jitter", "on"]),
            ("own", RenderScale::OneHalf, true)
        );
        assert_eq!(
            settings(&["--internal-scale", "1.5"]),
            ("own", RenderScale::OneHalf, false)
        );
        assert_eq!(
            settings(&["--upscaler", "off", "--internal-scale", "1.5"]),
            ("off", RenderScale::One, false)
        );
        assert_eq!(
            settings(&["--upscaler", "own", "--internal-scale", "1.5"]),
            ("own", RenderScale::OneHalf, true)
        );
        assert_eq!(
            settings(&["--upscaler", "own", "--jitter", "off"]),
            ("own", RenderScale::One, false)
        );
        assert_eq!(
            settings(&["--upscaler", "fsr2", "--internal-scale", "1.5"]),
            ("fsr2", RenderScale::OneHalf, false)
        );
        let args = parse(&["--upscaler", "fsr2", "--sharpen", "0.2"]).unwrap();
        match args.upscaler_settings(hud::UpscalerMode::Fsr2).0 {
            UpscalerKind::Fsr2(s) => assert_eq!(s.sharpness, 0.2),
            k => panic!("{k:?}"),
        }
        // Cycled to with `F10` from `off`, our upscaler jitters.
        let args = parse(&["--internal-scale", "1.5", "--upscaler", "off"]).unwrap();
        assert!(args.upscaler_settings(hud::UpscalerMode::Own).2);
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

    /// No flag starts in contour-sharp with detail on and rock's detail, `--look contour` in
    /// the contour look with every tuning part off; `--look plain` starts in
    /// face mode with neither, and `--detail` turns detail on there.
    #[test]
    fn the_default_look_is_contour_with_detail_and_plain_is_face() {
        let a = parse_args_from(argv(&[])).unwrap();
        assert_eq!(
            look_line(&a.start_look(), a.look.as_ref()),
            "look contour-sharp (normals contour, fit weighted27, face mix 0, detail after lighting on), detail on (rock scale 4, strength 0.1, fine 0), view shaded"
        );
        assert_eq!(a.look, Some(LookPreset::CONTOUR_SHARP));
        let c = parse_args_from(argv(&["--look", "contour"])).unwrap();
        assert_eq!(
            look_line(&c.start_look(), c.look.as_ref()),
            "look contour (normals contour, fit uniform27, face mix 0, detail after lighting off), detail on (rock scale 4, strength 0.1, fine 0), view shaded"
        );
        assert_eq!(flythrough::ROCK_DETAIL, [4.0, 0.1]);
        let a = parse_args_from(argv(&["--look", "face"])).unwrap();
        assert_eq!((a.look, a.detail), (Some(LookPreset::FACE), true));
        let a = parse_args_from(argv(&["--look", "plain"])).unwrap();
        assert_eq!((a.look, a.detail), (None, false));
        assert_eq!(
            look_line(&a.start_look(), a.look.as_ref()),
            "look face, detail off, view shaded"
        );
        let a = parse_args_from(argv(&["--look", "plain", "--detail"])).unwrap();
        assert!(a.detail);
        assert!(parse_args_from(argv(&["--look", "plain", "--rock-detail", "4,0.2"])).is_err());
        assert!(parse_args_from(argv(&["--look", "plain", "--detail-after", "on"])).is_err());
        assert_eq!(
            parse_args_from(argv(&["--attrib-quant", "none"]))
                .err()
                .as_deref(),
            Some("unknown argument --attrib-quant")
        );
    }

    /// `--look` pins each preset by name with its parts on the startup line, an unknown name
    /// lists the valid ones, and the part flags override the preset's parts.
    #[test]
    fn look_presets_parse_by_name_with_their_parts() {
        let line = |v: &[&str]| {
            let a = parse_args_from(argv(v)).unwrap();
            look_line(&a.start_look(), a.look.as_ref())
        };
        assert_eq!(
            line(&["--look", "contour-sharp"]),
            "look contour-sharp (normals contour, fit weighted27, face mix 0, detail after lighting on), detail on (rock scale 4, strength 0.1, fine 0), view shaded"
        );
        assert_eq!(
            line(&["--look", "contour-grain"]),
            "look contour-grain (normals contour, fit uniform27, face mix 0, detail after lighting on), detail on (rock scale 2, strength 0.3, fine 0.5), view shaded"
        );
        for p in LookPreset::ALL {
            let a = parse_args_from(argv(&["--look", p.name])).unwrap();
            assert_eq!(a.look, Some(p), "{}", p.name);
            assert_eq!(a.start_look().normal_mode, p.normal_mode, "{}", p.name);
            assert_eq!(a.start_look().tuning, p.tuning, "{}", p.name);
        }
        assert_eq!(
            parse_args_from(argv(&["--look", "contour-soft"]))
                .err()
                .as_deref(),
            Some(
                "--look takes plain, face, contour, contour-sharp, contour-grain, trilinear8, gradient27, not contour-soft"
            )
        );
        assert_eq!(
            line(&[
                "--look",
                "contour",
                "--plane-fit",
                "weighted27",
                "--plane-offset",
                "volume",
                "--face-mix",
                "0.25",
                "--detail-fine",
                "0.5",
                "--detail-after",
                "on",
                "--rock-detail",
                "2,0.2",
            ]),
            "look contour (normals contour, fit weighted27 + volume, face mix 0.25, detail after lighting on), detail on (rock scale 2, strength 0.2, fine 0.5), view shaded"
        );
        assert!(parse_args_from(argv(&["--plane-fit", "sobel"])).is_err());
        assert!(parse_args_from(argv(&["--face-mix", "0.6"])).is_err());
        assert!(parse_args_from(argv(&["--detail-fine", "-1"])).is_err());
        assert!(parse_args_from(argv(&["--detail-after", "yes"])).is_err());
    }

    #[test]
    fn the_look_line_names_the_preset_under_look() {
        let look = LookParams {
            normal_mode: NormalMode::Face,
            detail: true,
            ..LookParams::default()
        };
        let p = LookPreset {
            rock_detail: [4.0, 0.3],
            ..LookPreset::FACE
        };
        assert_eq!(
            look_line(&look, Some(&p)),
            "look face (normals face, fit uniform27, face mix 0, detail after lighting off), detail on (rock scale 4, strength 0.3, fine 0), view shaded"
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
