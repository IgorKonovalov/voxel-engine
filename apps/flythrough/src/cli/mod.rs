//! The engine's testbed (milestone 1 in `docs/roadmap.md`): a window showing the ray-marched
//! cave lit by a headlamp. Fly, dig, and read the per-material counts each edit returns.
//!
//! By default the cave is unbounded and streams around the camera (ADR-0003): full detail
//! within `r_full` chunks, mips out to `r_lod`. `--level` generates the bounded level instead.
//! `--source cave|level|showcase` picks the world: the streamed cave (the default), the level
//! (as `--level`), or the showcase cavern (`flythrough::showcase`), a bounded hall lit by four
//! emissive ores, started at its `hall` shot with the lamp off (`--lamp on` or `F7` turns it on)
//! and a light radius of 19 chunks unless `--light-radius` is given; it walks, digs and
//! collapses as the level does, and does not take `--save`, `--replay`, `--vault`,
//! `--autopilot`, `--room` or `--castle`. The banner's line then starts `source showcase`.
//! `--shot hall|chasm|crystals` (with the showcase) starts at that shot's pose and exposure
//! instead of `hall`'s. `--pose vein|patch` (with `--level`) starts at the settle rig's still
//! camera of that name (`flythrough::VEIN_POSE`, `PATCH_POSE`) on auto exposure unless
//! `--exposure` is given; the banner shows `pose vein` until the camera first moves.
//!
//! `flythrough [--seed N] [--level] [--pose vein|patch] [--source cave|level|showcase] [--shot NAME] [--vault] [--castle] [--castle-out PATH]
//! [--castle-shot gate|nave|window|dungeon] [--walk] [--render WxH] [--frames N] [--bench N]
//! [--autopilot] [--census] [--sync-budget MIB] [--sync-ms MS] [--workers N] [--mirror-budget MIB] [--residency R_FULL,R_LOD]
//! [--headless] [--look PRESET] [--face-mix K] [--far-normal LO,HI] [--band-census] [--view VIEW]
//! [--translucent-layers N] [--hud] [--save PATH] [--replay PATH] [--logic native|wasm[=PATH]]
//! [--audio] [--audio-debug] [--room METRES] [--room-line] [--thin-rig] [--seam-rig] [--absorption A] [--mic]
//! [--echo-db DB] [--tail-db DB] [--predelay-ms MS] [--tail-glide] [--direct-delay]
//! [--flat-mix] [--pad on|off] [--emissive-ore on|off] [--lamp on|off] [--lamp-toggle N]
//! [--fade-reading]
//! [--shadow-walk full|coarse] [--filter on|off] [--light-radius N]
//! [--history edits|gradient] [--lighting full|half]
//! [--lamp-bounce on|off] [--probe-spacing N] [--probe-rays N] [--probe-slices N]
//! [--internal-scale 1|1.5|2] [--jitter on|off] [--sharpen S] [--upscaler off|own|fsr2] [--pan DEG_PER_S]
//! [--jitter-phases N] [--dither on|off] [--pace free|compositor]
//! [--exposure auto|E]
//! [--adaptation eye|symmetric|instant] [--adapt LIGHT,CONE,ROD,FLOOR] [--metering centre|average]
//! [--path cave|showcase|castle] [--path-mode bench|tour] [--report DIR] [--tester]`.
//! `--path NAME` flies that scene's camera path (`flythrough::path`; the scene's own flags and
//! seed 1 are required): `--path-mode bench` (the default) once by wall time (1/60 s a headless
//! frame), then exits; `tour` loops it, the first move, look or dig hands the camera to the
//! keys and `T` resumes the path from its nearest point. The banner shows the mode always
//! (`benchmark 23 s / 37 s`). `--report DIR` writes `DIR/run.json` at exit
//! (`flythrough::report`). `--tester` leaves movement, look, the dig, `F1` (a help panel), `T`
//! and `Esc` and turns every other key off, the stats HUD among them. `--check-args` parses the
//! arguments, prints `flythrough: arguments ok` and exits 0, opening nothing.
//! `--help` prints every flag the parser reads, one row each from the flag table ([`flags`]),
//! and exits 0; the removed flags are rows too, each refused with its own message.
//! `--exposure auto` (the default) adapts like an eye (ADR-0033), `--exposure E` fixes it
//! (`--exposure 1` is the old look); `--adaptation` picks the auto preset (`eye`: 0.15 s toward
//! bright, 2 s then 10 s below a cone floor of 2^-6 toward dark, at most 16; `symmetric`: 0.67 s
//! both ways; `instant`), `--adapt` replaces its three time constants (seconds) and its floor, and
//! `--metering` weights the meter toward the view's centre or not. `Q` cycles `auto` with the
//! preset, `auto symmetric` and `manual` (holding the exposure of the moment); the banner names
//! the mode and its exposure in EV, the stats HUD's exposure row adds the metered scene and the
//! settled exposure, and stdout prints that row every half second while the exposure moves.
//! `--render` fixes the frame's output resolution (the window scales it); `--internal-scale`
//! renders at the output size divided by it on each edge (default 1.5: 720p for 1080p, through
//! our upscaler); `--jitter on|off` moves every primary ray by the Halton (2, 3) sub-pixel
//! jitter of the frame or through its pixel's centre, which the upscaler accumulates (our
//! upscaler jitters unless `--jitter off`; with no upscaler it is off unless `--jitter on`);
//! the upscaler runs when the scale is above 1 or the jitter is on; `--sharpen S` sets the
//! upscaler's RCAS sharpening, 0 to 1 (default 0.5, 0 off); `--upscaler off|own|fsr2` picks
//! the upscaler: `off` is Plan 0024's frame (scale 1, the jitter off), `own` the upscaler of
//! ADR-0024 at `--internal-scale`, `fsr2` FSR 2 (ADR-0034) at `--internal-scale` with its own
//! jitter; without it the upscaler is `off` at scale 1 with the jitter off, `own` when
//! `--jitter` is given (only `own` takes it), else `fsr2`, so no flag starts in `fsr2` at 1.5
//! and `--internal-scale 1` in `off`. The quality setting is FSR 2 at scale 1 with the far
//! normal scaled to the same voxels, `--upscaler fsr2 --internal-scale 1 --far-normal 3,9`
//! (about twice the default's march). In a
//! window `F10` cycles off, own and fsr2, printing the mode and showing it in a banner, `Z`
//! cycles the sharpening (0.5, 0.25, 0; the upscaler keeps its history), `V` the far normal
//! (`[2, 6]`, `[1, 2]`, off), and `--pan DEG_PER_S` turns the camera at a constant yaw rate
//! (positive to the left, by the frame's measured interval) that `Period` stops and starts;
//! the stats HUD names all four on one row; FSR 2's candidate knobs, off by default, are
//! `--jitter-phases N` (its jitter's phase count) and `--dither on` (a static one-level dither
//! in its tone pass), each named in the banner's fsr2 line when set;
//! `--frames` exits after N frames, printing the `F1` readings over the last ones and the
//! stats HUD's rows; `--pace free|compositor` (default `free`) picks whether the window
//! asks the compositor for a frame callback before each present (`vxe_app::Pacing`): with it
//! the next frame starts only after the compositor showed this one; `free` starts it at once.
//! The banner's `pace` row names the mode for `hud::BANNER_MS` after the start.
//! The stats HUD's `pace` row names the mode beside the wall, CPU and GPU time of a frame, and
//! the `frames:` line at exit gives the wall time between frames beside the CPU and GPU; `--hud` starts with the stats HUD shown (`F2` toggles it); `--bench` sends N `B` bench inputs on the first
//! frame; `--autopilot` digs, flies 300 m out and back and prints the streaming readings (see
//! [`Autopilot`]); `--cutaway auto|on|off` sets the cutaway view's start state (default `auto`:
//! on while the camera's voxel is neither air nor translucent, so a camera in rock or outside
//! the world sees each cavern's far wall through the rock; `X` cycles the three, and the
//! autopilot never changes it); `--census` prints the world's brick census (`World::brick_census`) on exit,
//! after any bench; `--sync-budget` sets the mirror's upload bytes per frame and `--sync-ms` its
//! upload time per frame (default none; 0 is none); `--workers N` sets the streamer's
//! generation workers (default one fewer than the logical cores, at least 1); `--mirror-budget`
//! sets the VRAM the mirror reserves for its pool, masks and records at startup (ADR-0013;
//! default 3072 MiB streamed, and on the level what its bricks, the dig headroom and its chunks
//! need, with records for the residency's ball when streaming), and while streaming the full
//! radius follows the mirror's pressure (the fuller of its pool slots and records): a chunk down after 30 frames above 0.9, a chunk up after 300 frames below
//! 0.7, between 6 and the configured radius, each change printed and on the HUD;
//! `--residency` sets the streamed radii (default `flythrough::RESIDENCY`, hysteresis 1);
//! `--headless` runs the
//! same frames on an offscreen target with no window; `--walk` (with `--level`) starts in walk
//! mode, the body spawned in the spawn pocket. The fly-through starts in `face`
//! (`LookPreset::DEFAULT`, honest cubes with the far normal at `[2, 6]`, ADR-0041), and prints
//! that look first; `--look` picks
//! the starting preset (`flythrough::LookPreset`: `face`, `field`, `trilinear8`, `gradient27`),
//! a normal mode with its face mix, overridable by `--face-mix`. `--far-normal LO,HI` (`--look face` starts with it off) blends
//! face mode's shading normal toward the 27-count gradient where a voxel spans fewer than `HI`
//! pixels, all the way at `LO` (`LookParams::far_normal`); the banner and the look line show it
//! in face mode, `far normal off` in the others, and `N` leaves it alone. Only a run started under
//! `--look field` builds the field's band slots and runs the field pass; any other run's `N`
//! skips the field. With the field's band slots the autopilot's report adds the field pass's
//! band slots, CPU and GPU time over the flight, and the frame line the smooth share of the
//! last frame's hits. `--view`
//! starts in a view (`shaded`, `normals`, `hitmask`, `steps`, `cracks`, `albedo`; with
//! `--frames`, `steps` prints the last frame's mean and p99 steps per ray on exit, and `cracks`
//! its leaks' share); `F9` cycles the views in that order, the banner naming the view
//! and the stats HUD adding the cracks view's shares row (`vxe_render::ViewMode::Cracks`);
//! `--translucent-layers` sets how many translucent voxels are composited in front of the opaque
//! hit (default 4, 0 ignores them). `--emissive-ore off` keeps the ore dark (by default it
//! glows, `flythrough::ORE_EMISSION`, and lights the cave through the light pass's emissive
//! samples); `M` pulses its emission scale (the probe volume sees the ore at the pulse's
//! average); `--lamp off` turns the headlamp off; `--fade-reading` (headless) digs five times
//! along the camera ray, lets the light settle with the lamp off and then on, switches it off
//! and prints how the screen centre's light fades, by source (see [`FadeSample`]);
//! `--shadow-walk coarse` sends every shadow ray through the coarse walk (2^3 blocks of the
//! masks; with `--view steps` and `--frames`, the exit prints the lamp shadow rays' cost too);
//! `--filter off` shows the temporal pass's accumulation unfiltered;
//! `--light-radius N` sets the chunks around the camera whose emitters light the frame
//! (default 4);
//! `--history gradient` also shortens history where A-SVGF's gradient sees the light change;
//! `--lighting half` runs the lighting passes at half resolution on each edge;
//! the light is the store (ADR-0046), filled from nothing by the live queue at the brick level (8 voxels, 4096 samples a
//! brick direction): before the first frame, at full GPU, the chunks within `--near R` (default
//! 2, Chebyshev) of the camera's at 4096 and every other mirrored chunk a coarse pass at
//! `--coarse N` (default 256; 0 none, and then the near chunks alone); after it, a slice a frame
//! refines the coarse chunks nearest first, each fading from its coarse value, and a chunk the
//! mirror gains takes its coarse pass first, unless it lies within `--full-first off|N` of the
//! camera's (default `--near`'s radius; `off` gives every blank chunk the coarse pass first; key
//! `6` toggles `off` and `2`); its row in the banner, its depth on the HUD's store
//! row and a `bake queue:` line a second on stdout; `--bake-slice MS` sets the slice (default
//! 1.5), `--bake-budget frame:MS|second:MS,CAP` the budget as a slice a frame or milliseconds a
//! second capped at a share of the frame (ADR-0052; key `5` cycles it);
//! `--missing-light black|neighbour|parent|chain` what a cube face with no baked value reads
//! (ADR-0053: its chunk's summary, its face neighbours', its parent cells'; default `chain`; key
//! `7` cycles it; the banner's queue row and the HUD's store row name it, and the `bake queue:`
//! line prints the no-value, fallback and still-black shares); `--reach full|partial` whether a
//! chunk whose light reach crosses the streamer's full radius waits for it (`full`, the default)
//! or bakes from the chunks resident full, flagged partial and re-queued as a refinement when a
//! chunk within its reach arrives (`partial`; key `8` toggles it; the banner's queue row, the
//! HUD's queue row and the `bake queue step:` line name it); a dig re-bakes its rings in the queue, ahead of
//! every other job, the slice growing by `--edit-ms MS` (default 3) while they wait, and
//! `--dig-reach off` turns off ring 3 (the chunks within the light radius of a dig that touched
//! lit bricks); a `dig settled:` line when a dig's rings are baked; `--light`, `--cut`, `--show`,
//! `--reference`, `--bake-file`, `--queue-file`, `--bake-radius`, `--blend`, `--preload` and
//! `--preload-claim` are refused, naming Plan 0059; `--glide V` moves the
//! camera along its heading at `V` voxels a second (`--pose vein-flight` starts the settle rig's
//! flight to the vein);
//! `--steady`, `--cache-read`, `--sampler`, `--emissive-history`, `--emissive-guard` and
//! `--reuse-m-cap` are refused, naming Plan 0057; `--lamp-bounce off` turns the lamp's bounce
//! from the probe volume off (on by default, ADR-0043's lamp channel; `--indirect on|off` is its
//! alias); `--probe-spacing N` sets the voxels between its probes
//! (default 16, 1.6 m; 4 to 64), `--probe-rays N` the rays each updated probe traces (default
//! 64, 1 to 64) and `--probe-slices N` the share it refreshes a frame, one in N (8, the
//! default, 16 or 32). In a window
//! `F3` toggles the shadow walk, `F5` the history killer, `F6` the lighting
//! resolution, `F7` the lamp, `F8` the lamp's bounce, `Q` the exposure, `Comma` the store read
//! (`--store-read face|normal`, ADR-0049, default `normal`) and `Semicolon` the light's filter
//! weights (`--light-weights normal|face`, default `face`), each printing the lighting modes; with the stats HUD shown,
//! two rows read every frame pass's GPU time and the light samples with those modes.
//! `N` cycles the look presets, naming the look on the banner.
//! `--vault` places a 16-voxel room with an indestructible
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
//! stopped, and the session continues without it. The native logic is fail-stop: a panic in it
//! unwinds out of the server's tick and ends the process, as any panic does. Replay
//! re-simulates through the chosen logic too. A session that ends on an error (a replay log that
//! does not read or re-simulate, a mirror sync that fails, an autopilot settle that times out)
//! runs the normal teardown, then prints the error once and exits with status 1.
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
//!
//! `--thin-rig` runs in the bounded thin-feature scene (`flythrough::thin_rig`: a trunk on a
//! floor with free one-, two- and three-voxel rods, a diagonal branch and two sheets of the
//! rig's wood), from a camera in front of the trunk; not with `--level`, `--source showcase`,
//! `--castle`, `--autopilot`, `--vault`, `--room`, `--save` or `--replay`. The wood's row is
//! crisp (`MaterialFlags::CRISP`): the field surface draws it as cubes.
//!
//! `--seam-rig` runs in the bounded seam scene (`flythrough::thin_rig::SeamRig`: a flat rock
//! wall with an 8 x 8 inlay of the rig's wood set flush in its face, and an L of two wood
//! slabs on the rock floor), from a camera facing the wall; the same restrictions as
//! `--thin-rig`, and the wood's row crisp the same way.
//!
//! `--crisp none|rock|all` (default none) marks rows of the table crisp when it is built: the
//! cave's rock row, or every row that is not translucent (a testbed switch for the field
//! surface's per-material cost). Start-up prints `world crisp rows: ...`.

use crate::audio::Mix;
use crate::castle;
use crate::fly::CutawayMode;
use crate::hud::{self};
use crate::path::{CameraPath, PathMode};
use crate::setup::CrispRows;
use crate::showcase::Shot;
use crate::{LogicChoice, LookPreset};
use vxe_app::Pacing;
use vxe_render::UpscalerKind;
use vxe_render::{
    AutoExposure, Exposure, HistoryKill, Lighting, LightingResolution, LookParams, NormalMode,
    RenderScale, ShadowWalk, ViewMode,
};
use vxe_world::Residency;

pub mod flags;
mod parse;
pub mod table;

pub use flags::{GONE_SINCE_0057, gone_since_0057, help, lighting_flags};
pub use parse::*;

/// The world a run builds; the flags that pick one (`--source`, `--level`, `--room`,
/// `--thin-rig`, `--seam-rig`) exclude each other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunMode {
    /// The streamed cave (the default), with the castle or the vault if asked.
    Cave,
    /// The bounded level.
    Level,
    /// The bounded showcase cavern.
    Showcase,
    /// A closed cube of air of this edge in voxels inside 2 m of rock.
    Room(i32),
    /// The bounded thin-feature scene.
    ThinRig,
    /// The bounded seam scene.
    SeamRig,
}

impl RunMode {
    pub(crate) fn is_level(self) -> bool {
        self == RunMode::Level
    }

    pub(crate) fn is_showcase(self) -> bool {
        self == RunMode::Showcase
    }

    pub(crate) fn is_thin_rig(self) -> bool {
        self == RunMode::ThinRig
    }

    pub(crate) fn is_seam_rig(self) -> bool {
        self == RunMode::SeamRig
    }

    /// The room's edge in voxels, for [`RunMode::Room`].
    pub(crate) fn room(self) -> Option<i32> {
        match self {
            RunMode::Room(edge) => Some(edge),
            _ => None,
        }
    }
}

/// The audio's flags.
#[derive(Clone)]
pub struct AudioArgs {
    /// `--audio`: open the default output device.
    pub(crate) on: bool,
    /// `--audio-debug`: print the callback's readings and record its output.
    pub(crate) debug: bool,
    /// `--room-line`: print the room line whenever a fan completes (implied by `--room`).
    pub(crate) room_line: bool,
    /// `--absorption`: the cave row's absorption per metre.
    pub(crate) absorption: f32,
    /// `--mic`: play the default input device at the listener, heard off the walls.
    pub(crate) mic: bool,
    /// `--echo-db`, `--tail-db`, `--predelay-ms`, `--tail-glide`, `--direct-delay`.
    pub(crate) mix: Mix,
}

/// The lighting's flags. The fly-through and its examples read the shared rows
/// ([`lighting_flags`]) into it; the rest are the fly-through's.
#[derive(Clone, Debug, PartialEq)]
pub struct LightingArgs {
    /// `--lamp on|off`: the headlamp lights the frame; unset, the program's default (the
    /// fly-through's: off on the showcase, else on).
    pub lamp: Option<bool>,
    /// `--lamp-bounce on|off` (alias `--indirect`): the lamp's bounce from the probe volume.
    pub lamp_bounce: bool,
    /// `--shadow-walk full|coarse`: the walk shadow rays take.
    pub shadow_walk: ShadowWalk,
    /// `--store-read face|normal`: how a cube face reads the store (ADR-0049; `Comma`).
    pub store_read: vxe_render::StoreRead,
    /// `--light-weights normal|face`: the normal the light's filters compare (`Semicolon`).
    pub light_weights: vxe_render::LightWeights,
    /// `--filter on|off`: the a-trous filter runs.
    pub filter: bool,
    /// `--light-radius N`: chunks around the camera whose emitters light the frame; unset,
    /// the source's.
    pub radius: Option<i32>,
    /// `--history edits|gradient`: what kills the temporal history.
    pub history: HistoryKill,
    /// `--lighting full|half`: the lighting passes' resolution.
    pub resolution: LightingResolution,
    /// `--probe-spacing N`: voxels between the probe volume's probes.
    pub probe_spacing: u32,
    /// `--probe-rays N`: rays each updated probe traces.
    pub probe_rays: u32,
    /// `--probe-slices N`: the volume refreshes one probe in N a frame.
    pub probe_slices: u32,
    /// `--emissive-ore on|off`: the ore row emits `flythrough::ORE_EMISSION` (on by default).
    pub(crate) emissive_ore: bool,
    /// `--lamp-toggle N`: switch the lamp every `N` frames, as `F7` does, timing the probe
    /// passes inside and outside the light-change windows the switches open.
    pub(crate) lamp_toggle: Option<u32>,
    /// `--fade-reading`: dig into the rock ahead, settle the ore light with the lamp off,
    /// then the lamp light with it on, switch it off and print how each light source at the
    /// screen centre fades (see `run::fade::FadeSample`).
    pub(crate) fade_reading: bool,
    /// `--exposure auto|E`: a manual exposure, or `None` for auto (the default).
    pub(crate) exposure: Option<f32>,
    /// `--adaptation eye|symmetric|instant`: the auto exposure's preset (eye by default).
    pub(crate) adaptation: hud::Adaptation,
    /// `--adapt LIGHT,CONE,ROD,FLOOR`: the preset's time constants in seconds and its cone
    /// floor, replaced.
    pub(crate) adapt: Option<[f32; 4]>,
    /// `--metering centre|average`: how the meter weights the frame (centre by default).
    pub(crate) metering: vxe_render::Metering,
}

impl LightingArgs {
    /// The flags' defaults over `base`'s switches and walks: the fly-through's start from its
    /// own defaults, an example's from the `Lighting` it starts with.
    pub fn from_lighting(base: &Lighting, probe_spacing: u32) -> Self {
        LightingArgs {
            lamp: None,
            lamp_bounce: base.lamp_bounce,
            shadow_walk: base.shadow_walk,
            store_read: base.store_read,
            light_weights: base.light_weights,
            filter: base.filter,
            radius: None,
            history: base.history,
            resolution: LightingResolution::default(),
            probe_spacing,
            probe_rays: base.probe_rays,
            probe_slices: base.probe_slices,
            emissive_ore: true,
            lamp_toggle: None,
            fade_reading: false,
            exposure: None,
            adaptation: hud::Adaptation::Eye,
            adapt: None,
            metering: vxe_render::Metering::default(),
        }
    }

    /// The lamp: `--lamp`, else on.
    pub fn lamp(&self) -> bool {
        self.lamp.unwrap_or(true)
    }

    /// The light radius: `--light-radius`, else `vxe_render::DEFAULT_LIGHT_RADIUS`.
    pub fn radius(&self) -> i32 {
        self.radius.unwrap_or(vxe_render::DEFAULT_LIGHT_RADIUS)
    }

    /// `base` with the shared rows' switches, walks and probe counts applied.
    pub fn apply(&self, base: Lighting) -> Lighting {
        Lighting {
            lamp: self.lamp(),
            shadow_walk: self.shadow_walk,
            store_read: self.store_read,
            light_weights: self.light_weights,
            filter: self.filter,
            history: self.history,
            lamp_bounce: self.lamp_bounce,
            probe_rays: self.probe_rays,
            probe_slices: self.probe_slices,
            ..base
        }
    }
}

/// The look's and the upscaler's flags.
#[derive(Clone)]
pub struct LookArgs {
    /// `--look`: the starting preset with any part flag applied. `face` by default; only
    /// `field` builds the field's band slots ([`Args::field_surface`]).
    pub(crate) preset: LookPreset,
    pub(crate) view: ViewMode,
    /// `--translucent-layers`: translucent voxels composited in front of the opaque hit.
    pub(crate) translucent_layers: u32,
    /// `--cutaway`: the cutaway view's start state.
    pub(crate) cutaway: CutawayMode,
    /// `--crisp`: the rows of the table marked `MaterialFlags::CRISP` when it is built.
    pub(crate) crisp: CrispRows,
    /// `--band-census`: print the field surface's band census (`flythrough::band_census`) over
    /// the chunks mirrored full after the level's first fill, or at the stream's full
    /// residency.
    pub(crate) band_census: bool,
    /// `--internal-scale 1|1.5|2`: the output size over the internal size, on each edge.
    pub(crate) internal_scale: RenderScale,
    /// `--jitter on|off`: the primary rays move by the frame's sub-pixel jitter. Our
    /// upscaler takes it only when given ([`LookArgs::jitter_set`]) and jitters otherwise.
    pub(crate) jitter: bool,
    /// `--jitter` was given.
    pub(crate) jitter_set: bool,
    /// `--sharpen S`: the upscaler's sharpening strength, 0 to 1.
    pub(crate) sharpen: f32,
    /// `--pan DEG_PER_S`: a constant yaw rate, degrees a second, positive to the left.
    pub(crate) pan: Option<f32>,
    /// `--jitter-phases N`: FSR 2's jitter phase count (`Fsr2Settings::jitter_phases`).
    pub(crate) jitter_phases: Option<u32>,
    /// `--dither on|off`: FSR 2's tone pass adds its static one-level dither
    /// (`Fsr2Settings::dither`).
    pub(crate) dither: bool,
    /// `--upscaler off|own|fsr2`, when given.
    pub(crate) upscaler: Option<hud::UpscalerMode>,
}

/// The light store's and the live queue's flags.
#[derive(Clone)]
pub struct BakeArgs {
    /// `--bake-budget frame:MS|second:MS,CAP` (or `--bake-slice MS`, `frame:MS`): the live
    /// queue's bake budget (default `frame:` `vxe_render::DEFAULT_BAKE_SLICE_MS`); key `5`
    /// cycles it through `BUDGET_CYCLE`.
    pub(crate) budget: vxe_render::Budget,
    /// `--coarse N`: the live queue's coarse pass, `N` samples a brick direction (0 none;
    /// default `vxe_render::DEFAULT_COARSE_SAMPLES`).
    pub(crate) coarse: u32,
    /// `--near R`: the chunks within `R` (Chebyshev) of the camera's baked at the full count
    /// before the first frame (default `vxe_render::DEFAULT_NEAR`).
    pub(crate) near: i32,
    /// `--full-first off|N`: the live queue's full-first radius after the pre-roll
    /// (`vxe_render::BakeQueue::full_first`); `None` follows `--near`. `6` toggles it
    /// between `off` and `2`.
    pub(crate) full_first: Option<Option<i32>>,
    /// `--missing-light black|neighbour|parent|chain`: what a cube face with no baked value
    /// reads (ADR-0053); `7` cycles it.
    pub(crate) missing_light: vxe_render::MissingLight,
    /// `--reach full|partial`: whether a chunk whose reach crosses the streamer's full radius
    /// bakes from the chunks resident full (`vxe_render::ReachMode`); `8` toggles it.
    pub(crate) reach: vxe_render::ReachMode,
    /// `--queue-order score|distance`: the live queue's order (ADR-0048's score by default, or
    /// the class and the distance first); `1` cycles score, score with the class as a tier,
    /// distance.
    pub(crate) queue_distance: bool,
    /// `--queue-weights A,R` or `A,tier`: the score's angle weight (`2` halves it, `3` doubles
    /// it) and its refine weight, or the class as the leading key (defaults
    /// `vxe_render::QUEUE_ANGLE_WEIGHT` and the class as the tier, ADR-0046's
    /// coarse-first rule; `QUEUE_REFINE_WEIGHT` is the refine weight `1` cycles to).
    pub(crate) queue_angle: f32,
    pub(crate) queue_refine: f32,
    pub(crate) queue_tier: bool,
    /// `--queue-withdraw on|off`: whether the live queue withdraws the jobs of chunks the camera
    /// has passed (default on); `4` toggles it.
    pub(crate) queue_withdraw: bool,
    /// `--queue-census`: the live queue reads its view census, the in-view ages and the angle
    /// weights at every opening (a reading; by default once a second, for the stdout line and
    /// the HUD's queue row, and no angle weights).
    pub(crate) queue_census: bool,
    /// `--edit-ms MS`: the live queue's slice grows by `MS` a frame while a dig's bricks wait
    /// (default `vxe_render::DEFAULT_BAKE_EDIT_MS`).
    pub(crate) edit_ms: f64,
    /// `--dig-reach on|off`: whether a dig that touched lit bricks re-bakes the chunks within the
    /// light radius (ring 3; on by default).
    pub(crate) dig_reach: bool,
    /// `--read-missing`: while a dig's rings 1 and 2 wait, the store's no-value count is read back
    /// each frame (blocking) for the `dig settled:` line.
    pub(crate) read_missing: bool,
}

/// The fly-through's arguments: the run's own, the world ([`RunMode`]) and the four groups.
#[derive(Clone)]
pub struct Args {
    pub(crate) seed: u64,
    pub(crate) render: Option<[u32; 2]>,
    pub(crate) frames: Option<u32>,
    /// `--pace`: whether the window waits for the compositor's frame callback.
    pub(crate) pace: Pacing,
    pub(crate) bench: u32,
    /// The world the run builds.
    pub(crate) mode: RunMode,
    /// `--shot NAME`: the showcase shot the camera starts at, and its exposure.
    pub(crate) shot: Shot,
    /// `--pose NAME`: the settle rig's still camera the bounded level starts at (its exposure
    /// is not applied).
    pub(crate) pose: Option<crate::Pose>,
    pub(crate) autopilot: bool,
    pub(crate) headless: bool,
    /// `--adapter SUBSTRING`: the run takes the first adapter whose name contains it (in a
    /// window, of those that present to it).
    pub(crate) adapter: Option<String>,
    pub(crate) walk: bool,
    /// `--vault`: the cave generates with the vault room ahead of the spawn.
    pub(crate) vault: bool,
    /// `--castle`: the streamed cave generates with the castle in its hall.
    pub(crate) castle: bool,
    /// `--castle-out PATH`: where to write the generated `.vox`.
    pub(crate) castle_out: Option<std::path::PathBuf>,
    /// `--castle-shot NAME`: the castle pose the camera starts at.
    pub(crate) castle_shot: castle::CastleShot,
    pub(crate) sync_budget: u64,
    /// `--sync-ms`: the mirror's upload time per frame; `None` (the default, or `--sync-ms 0`)
    /// for no limit.
    pub(crate) sync_time: Option<std::time::Duration>,
    /// `--workers`: the streamer's generation workers; `None` for `setup::default_workers`.
    pub(crate) workers: Option<usize>,
    /// `--mirror-budget`: the mirror's VRAM reservation in bytes.
    pub(crate) mirror_budget: Option<u64>,
    /// `--residency`: the streamed full and LOD radii in chunks, hysteresis as
    /// `RESIDENCY`'s.
    pub(crate) residency: Residency,
    /// `--hud`: start with the stats HUD shown.
    pub(crate) hud: bool,
    /// `--census`: print the brick census on exit.
    pub(crate) census: bool,
    /// `--save`: where the session's log file is written on exit.
    pub(crate) save: Option<std::path::PathBuf>,
    /// `--replay`: the log file whose session is re-simulated before the first frame.
    pub(crate) replay: Option<std::path::PathBuf>,
    /// `--logic`: the server's logic, native or the hosted component.
    pub(crate) logic: LogicChoice,
    /// `--pad on|off`: read a gamepad in a window (on by default); never headless or under
    /// the autopilot.
    pub(crate) pad: bool,
    /// `--dig-at F,..` and `--bench-at F,..`: a dig (the left click) or a `B` bench input at each
    /// of these frames, for a headless reading.
    pub(crate) dig_at: Vec<u32>,
    pub(crate) bench_at: Vec<u32>,
    /// `--glide V`: the camera moves along its heading at `V` voxels a second (1/60 s a headless
    /// frame), for a flight reading.
    pub(crate) glide: Option<f32>,
    /// `--path NAME`: the camera path the run flies (`flythrough::path`), and `--path-mode`.
    pub(crate) path: Option<&'static CameraPath>,
    pub(crate) path_mode: PathMode,
    /// `--report DIR`: where the path run writes `run.json`.
    pub(crate) report: Option<std::path::PathBuf>,
    /// `--check-args`: parse the arguments, print `arguments ok` and exit 0 (the tester
    /// build's launcher test checks its command lines with it).
    pub check_args: bool,
    /// `--help`: print the flag table ([`help`]) and exit 0.
    pub help: bool,
    /// `--tester`: only movement, look, the dig, `F1`, `T` and `Esc` act
    /// (`flythrough::path::tester_action`), and the banner shows the path's mode alone.
    pub(crate) tester: bool,
    pub(crate) audio: AudioArgs,
    pub(crate) light: LightingArgs,
    pub(crate) look: LookArgs,
    pub(crate) bake: BakeArgs,
}

/// The internal scale by default (`--internal-scale`): 1.5, so the fly-through starts in our
/// upscaler at 720p for 1080p (the owner's default after ADR-0034's comparison, Plan 0041
/// Phase 7). `--upscaler off` or `--internal-scale 1` is Plan 0024's frame.
pub(crate) const DEFAULT_SCALE: RenderScale = RenderScale::OneHalf;

/// The jitter without an upscaler by default (`--jitter`): off. Our upscaler jitters unless
/// `--jitter off` is given.
pub(crate) const DEFAULT_JITTER: bool = false;

/// The upscaler's sharpening strength by default (`--sharpen`).
pub(crate) const DEFAULT_SHARPEN: f32 = 0.5;

/// The light radius of `--source showcase` unless `--light-radius` is given, in chunks: the
/// farthest pair of its 14 x 12 x 7 chunks is `sqrt(13^2 + 11^2 + 6^2) = 18.06` apart.
pub(crate) const SHOWCASE_LIGHT_RADIUS: i32 = 19;

/// Why walk mode is refused while streaming, for `--walk` without `--level` and for `G`.
pub(crate) const WALK_NEEDS_LEVEL: &str =
    "walk mode needs --level (the coarse grid is not wired to streaming)";

/// Why `--save` and `--replay` are refused while streaming: a replay regenerates a bounded
/// world, and a streaming session's residency is not in its log.
pub(crate) const SAVE_NEEDS_LEVEL: &str =
    "--save and --replay need --level (replay rebuilds a bounded world)";

/// Walk mode is available only on the bounded level.
pub(crate) fn walk_allowed(streaming: bool) -> Result<(), &'static str> {
    if streaming {
        Err(WALK_NEEDS_LEVEL)
    } else {
        Ok(())
    }
}

impl Args {
    /// Whether the run reads a gamepad: `--pad on` in a window, without the autopilot.
    pub(crate) fn wants_pad(&self) -> bool {
        self.pad && !self.headless && !self.autopilot
    }

    /// The auto exposure of `--adaptation`, `--adapt` and `--metering`.
    pub(crate) fn start_auto(&self) -> AutoExposure {
        let mut a = self.light.adaptation.preset();
        if let Some([light, cone, rod, floor]) = self.light.adapt {
            a.tau_light = light;
            a.tau_cone = cone;
            a.tau_rod = rod;
            a.cone_floor = floor;
        }
        a.metering = self.light.metering;
        a
    }

    /// The live queue's full-first radius: `--full-first`, else `--near`'s.
    pub(crate) fn full_first(&self) -> Option<i32> {
        self.bake.full_first.unwrap_or(Some(self.bake.near))
    }

    /// The live queue's order and withdrawal in words: `order score, angle 4, refine 2;
    /// withdraw on`.
    pub(crate) fn queue_words(&self) -> String {
        format!(
            "{}; withdraw {}",
            self.queue_order().words(),
            if self.bake.queue_withdraw {
                "on"
            } else {
                "off"
            }
        )
    }

    /// The live queue's order from `--queue-order` and `--queue-weights` (and their keys).
    pub(crate) fn queue_order(&self) -> vxe_render::QueueOrder {
        use vxe_render::{QueueOrder, Refine};
        if self.bake.queue_distance {
            QueueOrder::Distance
        } else {
            QueueOrder::Score {
                angle: self.bake.queue_angle,
                refine: if self.bake.queue_tier {
                    Refine::Tier
                } else {
                    Refine::Weight(self.bake.queue_refine)
                },
            }
        }
    }

    /// Every toggle's mode at the start (`toggles::TOGGLES`), the banner shown from `now_ms`, the auto exposure's
    /// slot `start_auto` under `--adaptation`'s name; the line names the showcase source first
    /// and the probe spacing while the lamp's bounce is on, and the look's rows `--view`'s view.
    pub(crate) fn start_modes(&self, now_ms: f64) -> hud::Modes {
        let mut m = hud::Modes::new(self.start_lighting(), self.light.resolution, now_ms)
            .with_source(self.mode.is_showcase().then_some("showcase"))
            .with_probe_spacing(self.light.probe_spacing);
        m.set_auto(self.light.adaptation, self.start_auto());
        m.show_look(&self.look.preset, self.look.preset.far_normal, now_ms);
        m.show_view(self.look.view, now_ms);
        let upscaler = self.start_upscaler();
        m.upscaler = hud::UpscalerBanner::shown_from(
            upscaler,
            self.upscaler_settings(upscaler).1.name(),
            now_ms,
        )
        .with_sharpen(self.look.sharpen)
        .with_fsr2_knobs(hud::fsr2_knob_words(
            self.look.jitter_phases,
            self.look.dither,
        ));
        m.scale = self.look.internal_scale.name();
        m.pan = self.look.pan.map(|d| hud::Pan::new(d, now_ms));
        m.look = self.start_look();
        m.preset = self.look.preset;
        m.field_surface = self.field_surface();
        m.cutaway = self.look.cutaway;
        let b = &self.bake;
        m.queue = hud::QueueModes {
            distance: b.queue_distance,
            tier: b.queue_tier,
            angle: b.queue_angle,
            refine: b.queue_refine,
            withdraw: b.queue_withdraw,
            budget: b.budget,
            full_first: self.full_first(),
            missing_light: b.missing_light,
            reach: b.reach,
        };
        m
    }

    /// The upscaler the fly-through starts with: `--upscaler`, else `off` at scale 1 with the
    /// jitter off, `own` when `--jitter` is given (FSR 2 jitters by its own sequence and
    /// ignores it), else `fsr2`.
    pub(crate) fn start_upscaler(&self) -> hud::UpscalerMode {
        self.look.upscaler.unwrap_or(
            if self.look.internal_scale == RenderScale::One && !self.look.jitter {
                hud::UpscalerMode::Off
            } else if self.look.jitter_set {
                hud::UpscalerMode::Own
            } else {
                hud::UpscalerMode::Fsr2
            },
        )
    }

    /// The frame's settings in upscaler `mode`: the upscaler, the internal scale and the
    /// jitter. `off` renders at scale 1 with the jitter off; `own` jitters unless `--jitter
    /// off`; FSR 2 jitters by its own sequence.
    pub(crate) fn upscaler_settings(
        &self,
        mode: hud::UpscalerMode,
    ) -> (UpscalerKind, RenderScale, bool) {
        match mode {
            hud::UpscalerMode::Off => (UpscalerKind::Off, RenderScale::One, false),
            hud::UpscalerMode::Own => {
                let jitter = !self.look.jitter_set || self.look.jitter;
                (
                    UpscalerKind::Own(vxe_render::Upscaler::default()),
                    self.look.internal_scale,
                    jitter,
                )
            }
            hud::UpscalerMode::Fsr2 => (
                UpscalerKind::Fsr2(vxe_render::Fsr2Settings {
                    sharpness: self.look.sharpen,
                    jitter_phases: self.look.jitter_phases,
                    dither: self.look.dither,
                    ..vxe_render::Fsr2Settings::default()
                }),
                self.look.internal_scale,
                false,
            ),
        }
    }

    /// The lighting the fly-through starts in, from its flags.
    pub(crate) fn start_lighting(&self) -> Lighting {
        self.light.apply(Lighting {
            exposure: match self.light.exposure {
                Some(e) => Exposure::Manual(e),
                None => Exposure::Auto(self.start_auto()),
            },
            ..Lighting::default()
        })
    }

    /// Whether the run starts in the field look, so its mirror holds the field's band slots
    /// and the field pass runs; no other run builds a band slot.
    pub(crate) fn field_surface(&self) -> bool {
        self.look.preset.normal_mode == NormalMode::Contour
    }

    /// The look the fly-through starts in.
    pub(crate) fn start_look(&self) -> LookParams {
        LookParams {
            normal_mode: self.look.preset.normal_mode,
            view: self.look.view,
            translucent_layers: self.look.translucent_layers,
            cutaway: false,
            tuning: self.look.preset.tuning,
            far_normal: self.look.preset.far_normal,
        }
    }
}

#[cfg(test)]
mod tests;
