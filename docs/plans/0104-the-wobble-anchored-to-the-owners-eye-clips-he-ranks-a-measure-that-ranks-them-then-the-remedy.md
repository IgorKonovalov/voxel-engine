# 0104: The wobble anchored to the owner's eye: clips he ranks, a measure that ranks them, then the remedy

> **Status:** in progress: **closing. The owner answered question 4 (a), close (2026-11-26):
> no remedy; Phase 6 (6a, 6b, 6c) is next, then the fresh-session close review** ("Amended
> 2026-11-26 (owner)"). Before it: Phase 5 landed
> (2026-11-26): 5a holds by its refutation rule (`T b`'s clip total 0.01515 -> 0.01123 held, line
> (b) at T in his order; the under-0.002 figure missed on each hold's first window), 5b reads the
> upscaler (outlines 0.007 levels with it off). Before it: Phases 0, 1, 2, 0b and 4 landed (2026-11-25); stopped for the
> architect after Phase 4 (neither candidate holds at its artifact; candidate 7's drop rule
> fires; the maps for the owner's question 4 in `live/p4/index.md`) ("Amended 2026-11-25 (architect, after Phases 0 and 2)"; Phase 3 does not
> run: every subject anchored); approved (owner's go, 2026-11-25; drafted the same day with no interview from
> Plan 0100's close; questions 1 to 3 answered (a), (a), (a), recorded under "Risks & open
> questions"). **Order (owner, question 1 (a)):** Phases 0 to 2 and 4 run as a lane beside
> Plan 0099 now; Phase 3 and every remedy phase start on a main that holds Plan 0099.
> **Created:** 2026-11-25
> **Milestone:** M10: Outdoors in [`docs/roadmap.md`](../roadmap.md)
> **Related ADRs:** [ADR-0084](../adrs/0084-the-key-lights-visibility-on-a-world-face-is-fixed-per-face-the-columns-horizon-where-lit-a-cover-source-where-shadowed.md)
> (accepted with an Outcome; it defers the wobble, the near shadow's hole and sun under cover
> as a default to this plan, and names the contact ray as the one exception "a cover source
> only adds" may take),
> [ADR-0082](../adrs/0082-the-light-pass-keeps-no-screen-history-by-default-and-reads-smooth-sources-to-the-mid-range.md)
> (the calm defaults),
> [ADR-0076](../adrs/0076-light-stability-rules-world-history-per-source-a-fade-per-transition-and-a-ratcheted-pop-counter.md)
> (where a history lives, one fade a transition),
> [ADR-0055](../adrs/0055-render-settings-are-one-struct-with-an-apply-class-per-field-and-profiles-are-named-values.md)
> (the settings rows). **No ADR is drafted with this plan**: none is owed until a remedy has
> a rejected alternative, and none is picked here.
> **Source:** [Plan 0100](done/0100-a-sun-fixed-in-the-world-the-key-shadow-a-face-at-a-time-the-coarse-tier-held-still-and-sun-under-cover.md)
> (closed 2026-11-25, v0.75.0), its "Amended 2026-11-25 (architect, Phase 8b did not reproduce
> the window)", which wrote this plan's Phases 1 to 4 as F1 to F4, and its "The close"; the
> readings in [`logs/0100.md`](logs/0100.md): "Phase 8" (the owner's words), "The architect's
> code reading after the window", "Phase 8b", "The close review: the readings taken with no
> device wait".

## TL;DR

The owner sees a wobble on sunlit parts, at rest and in motion, and Plan 0100 closed with it
still there and **with no reading that shows what he sees**. This plan builds no remedy until
that is repaired. It renders his move at his window's size to five lossless clips he watches
in a video player, and he answers of each: is the wobble in it, where, when, in which words;
and he ranks two pairs blind. A subject he answers "no" to gets his own free flight recorded
in a window and rendered again headless. Then two candidate measures are tried against what
he ranked, and each is out at one failure. **After that the plan stops for the architect**:
the remedy phases are written as an amendment from the anchored clips and the measure. The
first thing the owner sees is `live/anchor/play.sh`: a one-pixel checker card, then five
clips, five to ten minutes.

## Context & problem

Four things Plan 0100 left open, all of them the sun's term at small faces (the roadmap's
M10; `docs/nfr.md` is not touched by them yet):

- **The wobble on sunlit parts**, at rest and in motion ("the wobble is located only on
  sunned parts", "more wobble on the trees and terrase" with no upscaler; the owner's words
  are verbatim in `logs/0100.md`, "Phase 8").
- **The tunnel slot's shimmer**: the edges and dots of shadows, which the owner names as the
  wobble's kind (Plan 0100's question 9), under every arm, `off` included.
- **The near shadow's hole** ("shadow missing"): the column horizon is read 2.5 voxels off
  the face, so a caster inside the lift casts nothing; 16.4 % of the terraces' key-lit
  pixels read lit where 64 disc rays read shadow (Plan 0100 Phase 5b, a CPU reading).
- **Sun under cover as a default.** `lighting.key_cover` stays `off` on every profile (his
  question 8, (c)); `ray` read calmest to him under the canopy, not blind.

Why no reading shows it, in short (the long form is the closed plan's "Why the measure and
the eye disagree"): Phase 8b counted a pixel flipping by over 4 levels and back in two
frames. That is a 30 Hz event, it counts pixels and not contrast, its classes cut the
picture where the eye does not, the rig did not fly where he flew, and the camera's own
motion over fine geometry is in the count. Two of his three orderings came out reversed or
flat. "Wobble" is his word and may name two or three artifacts; nobody has asked him which.

So the problem this plan starts with is not the light. It is that there is no artifact, on
disk, of which the owner has said "that is it, there". Every remedy tried so far was judged
by a figure that was never shown to follow his eye.

**What the tree holds** (`02e80ce4`): the fixed rig of Phase 8b is
`apps/flythrough/tests/gpu_wobble/owner_move.rs` (it waits on the device every settle
frame); the other rigs of the `gpu_wobble` binary do not (the close review's table). Plan
0100's Phase 9 removed `ray-record` and `centre` from the code and from the rig's tokens;
`lighting.key_cover off|ray|record`, `lighting.key_at hit|face|face-smooth` and
`lighting.key_horizon march|walk` stand as settings, every profile at the first of each.

## Decision

**Anchor first, measure second, remedy third, and stop between the second and the third.**
Phases 1 to 4 are Plan 0100's F1 to F4, carried over with the adjustments the tree asks for
and no others (each adjustment is marked). The remedy is not chosen here: the candidates
are listed under "After Phase 4" with what each would cost and what would falsify it.

We rejected drafting the remedy phases now from the code reading, because that is what Plan
0100 did three times (the face's centre, the walked horizon, the stage split) and each was
judged by a measure that did not rank what the owner ranked. We rejected a third measure
ahead of the clips for the same reason. We rejected a window session as the anchor, because
a window run leaves nothing to point at afterwards: a clip can be paused, stepped and laid
under a map.

### Rules this plan inherits

- **Never hold the camera in a window run** (owner, 2026-11-25): no `--pre-roll-drain` hold
  at a launch; the camera is free from the first frame. Phases 1 and 2 open no engine
  window at all (the clips play in `mpv`); Phase 3 and every later window obey it.
- **A no-upscaler reference is rendered at the window's own internal size** (1687 x 919
  under FSR 2 at 1.5 in the 2530 x 1378 window), never at a size borrowed from another
  window. Clip `T b` is not a reference: it reproduces his run 1b as the window drew it
  (1280 x 720 stretched), because that is the ordering he gave.
- **Every rig that steps the live bake queue waits on the device each frame and checks the
  store is baked before it reads.** A count of frames is not a measure of baking.
- **A reading starts at the owner's pose**: his eyes, his window's size, his move.
- **Exploration mode** (lane rule 2b, `.claude/skills/orchestrate/references/lane-rules.md`)
  for every phase before a pick: the owner's eye is the gate, bars are readings, checks a
  step are build, clippy, fmt and the quick tier, no gpu tier a step. CPU tests of data and
  API land with their phase.
- **Never simplify a scene so a check passes** (owner, 2026-11-22): the tunnel's roof slot,
  the terraces and the forest stay as they are.
- **A figure is called a measure of the wobble only after it ranks clips the owner ranked by
  eye.** Until then it is a reading.

## Architecture diagram

```mermaid
flowchart TD
    P0[Phase 0: the rigs read a baked store] --> P4
    P1[Phase 1: five lossless clips, a card] --> P2{Phase 2: the owner watches}
    P2 -- "anchored: yes or partly, with a place" --> P4[Phase 4: two measures against his ranking]
    P2 -- "not shown: no" --> P3[Phase 3: his own flight, recorded and re-rendered]
    P3 --> P2b{the same questions}
    P2b -- anchored --> P4
    P2b -- "not shown headless" --> A
    P2 -- "no subject anchored, none left for Phase 3" --> A
    P4 -- "one holds everywhere: the measure" --> A[The stop: the architect amends]
    P4 -- "both fail: clips and his eye alone" --> A
    A --> R[Remedy phases, an ADR where one has a rejected alternative]
```

## Implementation phases

Each phase ships as its own commit. Phases 0 to 4 are exploration. Phase 0's order against
Phase 1 is free: Phase 1 builds on the Phase 8b rig, which already waits on the device, so
**if the owner is at hand the lane does Phase 1 first** and Phase 0 while Phase 2 waits for
him. Phase 0 is committed before Phase 4 starts.

Every phase carries exactly one `**Owner skill:**`: `dev` or `human`.

### Phase 0: The wobble rigs read a baked store (the audit owed by Plan 0100's close)
- **Owner skill:** dev
- **What:** One settle the `gpu_wobble` binary's rigs share, and the readings that say what
  the old settle hid. No engine behaviour changes.
  - **The settle:** the queue stepped with the fly-through's knobs and the view's frustum,
    **a device wait every frame** (`wgpu::PollType::wait_indefinitely`, as
    `support::queue`'s rig and `owner_move.rs` do), ended when no chunk in view waits, no
    job is in flight and no block fades; then **a census of the store** (chunks in view
    that hold a fresh record against chunks in view) printed as one line. A reading whose
    census is not whole says so in its own header line and is not tabled as a baked picture.
    It lives in `apps/flythrough/tests/support/wobble.rs` or a sibling under `support/`;
    **`support/defects.rs` is not edited** (Plan 0099's Phases 1 and 3 change it).
  - **The audit:** every entry of `gpu_wobble.rs`, `gpu_wobble/{terraces,coarse,cover,
    cover_arms,fsr,walked}.rs` and `support/wobble.rs` that steps the queue: how it steps,
    what it checks before it reads, the census under its settle as it stands today, and
    whether it now takes the shared settle. The close review's table in `logs/0100.md` is
    the starting list, not the result.
  - **The two asserting soak tests** of the binary,
    `the_defaults_hold_still_at_rest_and_the_far_holds` and
    `the_far_moves_no_worse_than_the_before` (`SETTLE` and `GATE_SETTLE`, 120 unread frames
    each): the census at their first read under the old settle, then each moved to the
    shared settle and run by name on lavapipe and on the RTX. **Their bars are not retuned
    here.** If a bar fails on a baked store the dev reports the figures and stops: the bar
    is Plan 0098's contract and the architect rules on it.
  - **Plan 0098's Phase 9 table** (2.4 to 3.9 %, read on the same settle): its rows read
    once more under the shared settle and printed beside the old ones, as readings.
  - **The W1 mismatch:** Plan 0100's Phases 5 and 5b read 1 404 to 1 906 lighting pixels
    darker on the GPU than the CPU port of the horizon at W1's eye, every one the GPU under
    the port, most on coarse blocks at 128 to 512 voxels. One reading under the shared
    settle (`gpu_wobble/cover.rs`, `walked.rs`), the mismatches split three ways: by the
    block's tier, by whether the hit's chunk is mirrored whole or as a coarse level, and by
    the distance of the map cell that decides. **Stop rule:** one cause that covers nine
    tenths of them is named, or the three splits are reported with "unexplained". A fault
    of the port is fixed in the port. A fault of the shader is not fixed here: it is a
    finding for the architect.
  - **The comment:** one line at the `LIGHTING2_*` constants saying bit 3 of
    `Params::lighting2` is free (`centre` held it).
  - **Not taken:** the gate-sized rig for the bake's key ray at its cap ("Plan 0100's
    leftovers" in [`docs/backlog.md`](../backlog.md)). It belongs in
    `apps/flythrough/tests/gpu_bake_rule.rs`, which Plan 0101's Phase 2 edits; it stays in
    the backlog for that plan.
- **Files touched:** `apps/flythrough/tests/support/wobble.rs` (or a sibling),
  `apps/flythrough/tests/gpu_wobble.rs`, `apps/flythrough/tests/gpu_wobble/`,
  `crates/svet-render/src/frame/settings.rs` (the comment only), `docs/plans/logs/0104.md`.
- **Done when:** the log holds the audit's table (a row a rig entry: steps how, checks
  what, the census before, converted or why not); for each of the two soak tests the census
  before, and its result on a whole census on lavapipe and on the RTX with the wall time
  (both are soak-marked today and stay so; they are run by name, not as a tier); Plan
  0098's Phase 9 rows old beside new; the W1 line (a cause with its count, or the three
  splits and "unexplained"). No reading of the binary steps the queue without the wait.
  Quick tier, clippy and fmt over the changed files.

### Phase 1: The owner's move as clips he can watch (exploration)
- **Owner skill:** dev
- **What:** No production code. The Phase 8b rig renders the move to lossless video at the
  window's size, as the window drew it.
  - **As the window:** 2530 x 1378, the RTX, the fly-through's bake, the queue stepping with a
    device wait a frame, **the exposure not pinned** (metered as the window meters it), no
    HUD. The settle is Phase 0's where it exists, else `owner_move.rs`'s own (the same
    wait); either way the census line is printed and whole before frame 1. *(Adjusted: the
    census.)*
  - **The bake's mode is said on every clip's header line.** On main today the queue has one
    mode, the step on the calling thread. If the lane's base holds Plan 0099's Phase 3 the
    clips render in lockstep and say so; a window then runs `bake thread on` by default,
    which changes when a landing shows. *(Added: Plan 0099.)*
  - **The move** (`SVET_MOVE=window`): W as the fly-through flies it, along the view with its
    pitch, until the first solid along the view ray is 60 voxels off or 2 s have passed; then
    level along the heading to 5 s in all; a stop held 3 s; a turn of 60 degrees over 40
    frames; held 3 s. If the clearance rule cuts the fly under 4 s the dev turns the level
    leg's heading by 15-degree steps until 5 s fit, and records the heading. About 12 s
    (645 frames at the step below).
  - **The frame step is 1/55 s** (three refreshes of the owner's 165 Hz panel a frame, so
    the playback's cadence is even). A guess that an even cadence is the fairer one to judge
    by; the window's own was uneven.
  - **The clips** (five):
    - `T`: the terraces eye, a pair. The defaults, and run 1b as the window drew it:
      upscaler off at 1280 x 720, stretched to 2530 x 1378 by the filter the fly-through's
      present uses (the dev reads which and names it).
    - `C`: the canopy, a pair, the defaults and `key cover ray`. From a start within 30 m of
      the canopy eye that the dev picks by `calm`'s class map so that the view through the
      move holds at least 10 % sunlit ground, 5 % sunlit leaf and 20 % shadowed pixels; the
      eye and angles recorded. If no such start exists the nearest is used and its shares
      said.
    - `S`: the tunnel's slot under the defaults (`--source outdoor --spawn inside --time
      12:00`, Plan 0100 Phase 5's eye), one clip.
    The eyes are in `logs/0100.md`, "Phase 8" ("The poses").
  - **Blind pairs:** each pair's two files are named `a` and `b` by a coin the runner throws;
    the key goes to `key.txt`, which the owner is not shown until he has answered.
  - **The format:** frames to `ffmpeg` (`libx264rgb -qp 0 -preset ultrafast`, Matroska):
    lossless, RGB, no scaling, so nothing the encoder does can hide or add a shimmer. A GIF
    at this size is not watchable, and `scripts/record-clip.sh`'s MP4 is 4:2:0 and lossy. If
    the decode is too slow the fallback is `-crf 6 -pix_fmt yuv444p -tune fastdecode`, said
    in the log. A sixth file, `card`: 3 s of a one-pixel checkerboard at 2530 x 1378.
    `ffmpeg`, `ffprobe` and `mpv` are tools of the dev box, not dependencies of the
    workspace; the dev records their versions and adds nothing to `Cargo.toml`. *(Added.)*
  - **The frames are written out, and not kept raw.** A frame is 2530 x 1378 x 4 bytes =
    13.9 MB, a clip of 645 frames 9.0 GB, five clips 45 GB: the rig streams a clip's frames
    to the encoder (or writes and deletes a clip at a time) under the untracked `live/`, and
    keeps frame 1 of each as a PNG for the byte check. *(Added: the arithmetic.)*
  - **The runner and the player**, under the untracked `live/anchor/`: `run.sh` (each clip
    under the perf lock, as Plan 0100's `live/p8b/run.sh`), `play.sh` (`mpv --fs
    --video-unscaled=yes --loop-file=inf --video-sync=display-resample --interpolation=no`
    over the files in the order card, T a, T b, C a, C b, S), `ask.txt` (Phase 2's questions
    and the keys).
  - **Kept for Phase 4:** each clip's camera a frame (eye, yaw, pitch) **and the frame's
    exposure** as text beside it. *(Adjusted: the exposure is metered now, and Phase 4's
    first candidate divides by it.)*
- **Files touched:** `apps/flythrough/tests/gpu_wobble/owner_move.rs` and a sibling under
  `gpu_wobble/` (new code goes to the sibling: `gpu_wobble.rs` is near the file-size gate's
  cap), `apps/flythrough/tests/gpu_wobble.rs` (the `mod` line), `docs/plans/logs/0104.md`.
- **Done when:** the six files exist; `ffprobe` reads 2530 x 1378 at 55 frames a second and
  an RGB pixel format on each; `ffmpeg -benchmark -i <clip> -f null -` decodes each at twice
  real time or more on the dev box (else the fallback, said); frame 1 of each clip decoded
  back equals the rig's own frame byte for byte; the dev has read one decoded frame of each
  clip by eye and says what it shows (the sun's patches there, no unbaked black); the log
  holds the commands of `run.sh` and `play.sh` verbatim, each clip's eye, angles, fly length,
  sunlit share, census line and bake mode, the stretch filter's name, the tools' versions,
  and the load beside each run. Quick tier, clippy and fmt over the changed test files.

### Phase 2: The owner watches the clips
- **Owner skill:** human
- **What:** The orchestrator runs `live/anchor/play.sh` in the owner's terminal (a video
  player, not a window run of the engine: no camera to hold). Five to ten minutes. Keys:
  Enter the next clip, Space pause, `.` and `,` a frame on and back, `[` half speed, `s` a
  screenshot, `i` the player's statistics.
  - **First, the card:** it shows an even fine grey with no bands, and `i` reads no dropped
    frames. Bands mean the player is scaling; the orchestrator fixes the player before any
    answer is taken.
  - **Asked of each clip** (T a, T b, C a, C b, S): (1) Is the wobble you saw in the window
    in this clip: yes, partly, or no, the window showed something this does not? (2) If yes
    or partly: pause where it is clearest, press `s`, and say where in the picture and when
    (flying, at the stop, turning, held). (3) Which words fit: an edge of light stepping or
    crawling along a surface; dots sparkling; a patch changing shape or brightness; the
    whole picture's brightness; something else.
  - **Asked of each pair:** which of `a` and `b` wobbles more, or the same?
- **Files touched:** `docs/plans/logs/0104.md` (written by the lane from the orchestrator's
  record).
- **Done when:** the log holds his answers verbatim, the screenshots' paths, and `key.txt`
  opened after. Then one line a subject:
  - **anchored** (a yes or partly with a place): that clip, place and moment are what every
    later measure must rank;
  - **not shown** (a no): Phase 3 runs for that subject;
  - for a pair, the window's ordering **confirmed blind**, **not confirmed** (the same, or
    the other way: that ordering is dropped as never established, and said so), or void (the
    subject not shown).
  If no subject is anchored, Phase 4 does not start.

### Phase 3: The owner's own flight, recorded and rendered again (exploration; only for a subject Phase 2 marks not shown)
- **Owner skill:** dev
- **What:** The fly-through gains `--track-out FILE` (in a window: every frame's eye, yaw,
  pitch and frame time, and a mark at each press of a key the dev picks, **shown on the
  HUD's mode row while it records**, with the count of marks) and `--track FILE` with
  `--frames-dir DIR` (headless: the same binary flies the recorded cameras with the recorded
  frame times and writes every frame). No engine crate changes. Then one window run a
  subject not shown, launched with the camera free from the first frame, a minute each: the
  owner flies as he did and presses the key when he sees the wobble. The dev renders 5 s
  each side of every mark from the track, encodes as Phase 1, and Phase 2's questions are
  asked of those clips.
  - **The track's header names the window's size, its profile and its bake mode**, and the
    headless render names its own. A window on a main that holds Plan 0099 runs `bake
    thread on`; the headless render is lockstep. Where the two differ the log says so
    beside the owner's answer. *(Added: Plan 0099.)*
  - **Its base holds Plan 0099's Phase 3** (question 1): both edit
    `apps/flythrough/src/cli/` and `apps/flythrough/src/run/`. *(Added.)*
- **Files touched:** `apps/flythrough/src/cli/`, `apps/flythrough/src/run/`,
  `apps/flythrough/src/path.rs` (or a sibling), `apps/flythrough/src/hud/` (the row),
  `apps/flythrough/tests/path.rs`, `apps/flythrough/tests/gpu_track.rs` (new),
  `docs/flythrough.md`, `docs/plans/logs/0104.md`.
- **Done when:** CPU (under 1 s each): a written track parses back equal; `--track` with no
  `--headless` and `--frames-dir` with no `--track` are refused; the HUD's mode row holds
  the recording's word and the mark count while `--track-out` is set, and neither without
  it. GPU (binary `gpu_track`, lavapipe, under 10 s, a guess): a 30-frame track rendered
  twice headless gives the same last frame where no bake lands. The log holds the window's
  launch lines, the marks, the clips, and the owner's answers verbatim with **anchored** or
  **not shown headless** a subject. A subject not shown from his own track is the result:
  the difference is then between the window's present and the headless frame (or between
  the two bake modes), and the architect is called before anything else is built.

### Phase 4: A measure that ranks what the owner ranked (exploration, a reading)
- **Owner skill:** dev
- **What:** No production code. Two candidate measures, each computed on the decoded frames
  of the anchored clips (the files the owner judged) with each clip's camera track and the
  rig's world. Neither is known to work; each is a guess at what the eye follows.
  - **Candidate 1, the light of a world point over time.** Points fixed in the world: the
    centres of the faces visible in a clip's first frame, re-seeded each second. Each frame a
    point is projected with the frame's camera, kept while a ray from the eye reaches it,
    and the display's luma is read there, divided by the frame's exposure. On a face under a
    fixed sun that series is flat; the camera's motion is removed by construction. Its
    energy between 1 and 15 Hz, as a contrast against its own mean, is the point's score.
    Scores are summed into screen tiles of 64 x 64 pixels by the area the points cover, and
    printed as a map over a frame and as a total a segment.
  - **Candidate 2, a lit edge's place on its face.** For every face that covers at least 16
    display pixels for at least 30 frames: the share of its pixels over the mid luma between
    the clip's lit and shadowed levels, a frame. Under a fixed sun that share changes only
    with the angle of view, slowly. Its changes of over a tenth between frames a quarter
    second apart are counted, weighted by the face's pixels, a tile and a segment.
  - **Shown as maps the owner's screenshots can be laid on.**
- **Files touched:** `apps/flythrough/tests/gpu_wobble/` (a sibling reading),
  `docs/plans/logs/0104.md`.
- **Done when:** the log holds, a candidate and a clip, the map (under `live/`) and the
  totals by segment, and for each anchored subject three lines, each **holds** or **fails**:
  (a) the place the owner named is among the top tenth of tiles by score at the moment he
  named; (b) every pair he ranked blind is ranked the same way by the clip totals, and a
  pair he called the same differs by under a fifth; (c) a clip he answered "no" to scores
  under every clip he answered "yes" to at the same subject. **A candidate that fails one
  line at an anchored subject is not the measure**, and its stage rows are not read. If one
  holds everywhere it is this plan's measure, and Plan 0100 Phase 8b's stage rows are read
  again under it (the rows only; no remedy). If both fail the dev reports both tables and
  the owner's words beside them. **Either way the dev stops here and the architect is
  called.**

## Amended 2026-11-25 (architect, after Phases 0 and 2)

Three rulings on what Phases 0 and 2 read, from the readings log ("The forest soak test's
time", "The W1 mismatch", "Phase 2") and the rig's code. No GPU run stands behind this
section: every time below that is not quoted from the log is a projection and says so.
**Order: Phase 0b, then Phase 4.** Phase 0b lands before this lane merges into main in any
case: as the tree stands the weekly soak (`soak.yml`, lavapipe) would run a 12 538 s test.

**One rule for every run of this plan from here on (owner, 2026-11-25):** no run is started
that projects past 20 minutes under the perf lock. A run prints its projection from its first
frames (the settle's samples a second against the samples the same settle took on the RTX; a
segment's first 30 frames against its length) and the dev stops it when the projection passes
the cap. The gate tier is the most any step runs.

### Phase 0b: The forest soak test asserts on one baked store (the remedy for its time)
- **Owner skill:** dev
- **Why:** `the_defaults_hold_still_at_rest_and_the_far_holds` reads 771 s on the RTX and
  12 538 s on lavapipe on a whole census (owner: "such long test is exactly what we are
  trying to avoid"). The log's own figures say where the time goes. The test bakes six fresh
  stores from one eye with the same knobs and asserts on two of the six arms (the rest arms,
  `default,pin,nqueue`, at fly 180 and fly 20); the stepping and native arms are printed. It
  reads 5 550 frames after the settles (0.131 s a frame on the RTX, 0.45 s on lavapipe), of
  which the two asserted stops need 1 050. On lavapipe the settle draws 4 727 frames an arm
  because the queue's sample cap a step sits at its floor there (16 384 samples: the slice
  is sized from GPU timestamps, which lavapipe does not give), so about 77 million samples
  go out a floor's worth a drawn frame.
- **What:** No engine behaviour changes; `apps/flythrough/tests/` only. Three levers, all
  three taken, in this order.
  1. **The test runs what it asserts, on one store.** One frame, one settle, then the rest
     protocol twice from the spawn's camera with the queue stopped (fly 180, then fly 20):
     the exposure's pin, the still, the fly, the stop. The protocol ends at the stop: the
     turn and the hold after it assert nothing. With the queue stopped the store does not
     change between the two reads. The four printed arms (`default,pin` and
     `default,pin,native` at both flies) leave the test; they are `reading::wobble` under
     `SVET_WOBBLE`, which exists, and the test's doc comment names the specs.
  2. **The settle's pace is not the fly-through's.** While it settles, `support::baked::
     settle` steps the queue with a slice long enough that a step's sample cap is the
     submit's own (`Budget::PerFrame` raised for the settle, the budget before restored at
     its end, as `wobble_on` already does for `pre=MS`). The knobs that decide what is
     baked (coarse samples, near and full-first radii, tiers by size on screen) stay the
     fly-through's. This changes every rig that takes the shared settle, so it is checked:
     see the done-when's second line.
  3. **On a software adapter the test's frame is half each way** (640 x 360 internal, 960 x
     540 out, the arm's `out=` token; `run_segment` sizes its buffers from the `OUT`
     constant today and must take the arm's). It is the one lever on the cost of a read
     frame there. The near bars are absolute and keep their constants at both sizes; the
     far +30 bar is a constant a size (below).
- **The far +30 bars are re-set.** `REST_FAR30` (9.98 and 11.39, plus 2) was read on a store
  with 5 of 59 chunks baked: it measured the far landing during the stop. On a whole census
  the same figure reads 5.44 and 4.88 on the RTX (5.64 and 4.84 on lavapipe), so the old
  bar would let the far double before it failed. The rule is unchanged (Plan 0098 Phase 11:
  the largest of three readings plus 2 points), the readings are new: at 1080p, the largest
  of three RTX readings on a whole census (Phase 0's and two of this phase's); at the half
  frame, the largest of three lavapipe readings of this phase. From Phase 0's figures alone
  the 1080p bars would be 7.64 (fly 180) and 6.88 (fly 20); the dev writes the constants
  from the three readings and the log holds all of them. `REST_NEAR0` and `REST_NEAR_PAIR`
  are not moved (the readings are 0.00 to 0.05 against 2.05 and 2.5; they are Plan 0095's
  and Plan 0098's bars as written).
- **The 727 s on the RTX after the settles** falls with the arms (lever 1) and is not cut
  further: what is left is the rig's reads a frame (hits, sources, classes), the instrument
  every wobble reading shares, and a leaner second path for the test would be a second
  instrument to keep equal to the first.
- **Rejected:** *the RTX only, with a cheaper lavapipe proxy*: a bar no scheduled machine
  runs goes stale unseen, and the tiers have no class for an asserting test run by hand
  (the weekly soak on lavapipe is the soak mark's meaning); the binary's other soak test
  (`the_far_moves_no_worse_than_the_before`, 101 s on lavapipe) asserts a different claim.
  It is the fallback only if the bar below is missed with all three levers, and then the
  architect rules. *Fewer asserted arms* (dropping fly 20, or a bar): each is a clause of
  Plan 0098's contract, and lever 1 already removes every arm that asserts nothing. *A
  smaller view on both adapters*: 1080p through FSR 2 at 1.5 is the size the contract was
  ruled at, and the RTX pays about 150 s for it. *A shorter stop*: the bars are frame 30
  against frame 300.
- **Files touched:** `apps/flythrough/tests/gpu_wobble.rs`,
  `apps/flythrough/tests/support/baked.rs`, `docs/plans/logs/0104.md`.
- **Done when** (binary `gpu_wobble`, the test by name; it keeps `#[ignore = "soak"]`):
  - the test asserts the three rest bars at fly 180 and fly 20 on one store whose census
    line says whole, and fails when a census is not whole;
  - the faster settle bakes the same store: on the RTX at 1080p the census (chunks in
    view, settled, coarse) equals Phase 0's 60/60 with 54 coarse, and the rest figures read
    within 1 point of Phase 0's (far +30 5.44 and 4.88; near pair and stop +0 near under
    0.1); a difference is reported and the lever withdrawn, not tuned around;
  - **the time bar: 180 s on the RTX and 600 s on lavapipe on the dev box**, wall time of
    the test alone under the perf lock, the load beside it. Projected, not read: the RTX
    about 150 s (1 050 read frames at 0.131 s, a 7 s settle, the forest's generation);
    lavapipe has no projection worth the name until levers 2 and 3 are read, which is why
    the first lavapipe run prints one and is stopped past 20 minutes;
  - the log holds the settle's frames and seconds before and after lever 2 on both
    adapters, three far +30 readings a size, the constants written from them, and the
    reading specs that replace the printed arms. Quick tier, clippy and fmt over the
    changed files. If the lavapipe bar is missed the dev reports the figures and stops.

### The W1 mismatch: one narrower split in Phase 4, with a stop rule
The reading (1 859 lighting pixels where the GPU's key value is under the CPU port's
horizon, inside Plan 0100's 1 404 to 1 906; coarse 1 562, 128 to 512 voxels 1 555) is
unexplained by its three splits, but it carries a lead the splits did not use: **every
mismatch has the key slot's source "the mean"** (`defects::key_slot`'s 0), not "the
horizon" (2). The port is being compared against a value that, at those pixels, the shader
did not read from the horizon. Whether that explains it cannot be said yet, because the
agreeing pixels were not split by source. It is kept in this plan, as one reading added to
Phase 4, because its band (128 to 512 voxels, sunlit, darker than it should be) is where
the owner puts his second artifact ("changing brightness on distant trees that were
lighted").

- **The split (Phase 4, first step, RTX, W1's eye, one run of seconds after its settle):**
  all compared pixels by key slot source against agree or mismatch, by the hit's distance
  band; and, from the shader, which path writes source 0 at 32 to 512 voxels and what it
  averages.
- **Stop rule.** (a) Nine tenths of the pixels with source "the mean" in the band mismatch,
  or the path named is a filtered value the port does not model: the line is closed as a
  difference of sources, the port's comparison is restricted to pixels whose source is the
  horizon, and the count there is the new W1 figure. (b) The path's value can change
  between frames at a fixed world point (it mixes a landing, a history or a jittered read):
  it becomes a seventh candidate in "After Phase 4", and Phase 4's maps say whether the
  owner's places lie on such pixels. (c) Neither: the tables go to
  [`docs/backlog.md`](../backlog.md) under "Plan 0100's leftovers" as unexplained, by the
  architect at the stop, and this plan carries it no further. No shader is changed in any
  case.

### Phase 3 does not run
Every subject is anchored (Phase 2: T, C and S, no "not shown"). Phase 3's `--track-out`
and `--track` flags are not built by this plan. If a remedy later needs the owner's own
flight, the amendment that writes the remedy phases brings Phase 3 back by name.

### Phase 4, amended: two artifacts, each with its own subjects
The owner's words (Phase 2, verbatim in the log): "all wobbles were edge of light stepping
on slopes, and dots sparkling, or maybe its changing brightness on distant trees that were
lighted", and "it was almost good, esp Tb". Phase 4 as written predates them. Its two
candidates do aim at the two artifacts (candidate 2 at the stepping edge, candidate 1 at a
point's brightness), but four things in it would fail a candidate by construction at the
places he named. Phase 4's text above stands; where this section differs, this section
rules.

- **The two artifacts and where they are anchored.**
  - **A, an edge of light stepping on slopes:** `T a`, the top of the picture at about 3 s,
    flying (window 2 to 4 s); `S`, the slopes, slight, in the turn. Pair: `T a` over `T b`.
  - **B, dots sparkling or the brightness of distant sunlit trees changing** (he is not
    sure which; both are read): `C a`, the lit trees in the top left, in the fly; `C b`,
    the sunlit trees, in the turn. Pair: `C a` over `C b` (read from "there is wobble"
    against "a bit", not stated as a ranking: the log says so, and so does the result).
  - **Not classed:** `S`, the change in the shadow at the tunnel's end where it meets the
    ground ("clearly seen"). It is named before it is measured: the dev renders `S`'s track
    again with the queue's counters a frame and says whether the change falls on a frame
    where a block lands or the exposure steps. A landing or an exposure step is not a
    wobble of either kind: it goes to the stop as its own line ("Carried to the amendment"
    already holds both sources) and is left out of both candidates' lines.
- **A candidate is judged an artifact, not over all subjects.** Candidate 2 is tried on A's
  subjects, candidate 1 on B's; each is also printed on the other's, as a reading. "Fails
  one line at an anchored subject" now reads: at a subject of the artifact it is tried on.
  The plan may leave Phase 4 with a measure for one artifact, for both, or for neither.
- **The unit grows to what the eye can resolve.** Candidate 2 as written takes faces of at
  least 16 display pixels, and candidate 1 a face's centre. At the top of `T a` and on
  distant crowns a face is under a pixel, so the first would score nothing at his place and
  the second would read the camera's motion across sub-pixel leaves. For both candidates
  the unit is a face where it covers 16 display pixels, else the visible faces of one normal
  inside a world cube of 2^k voxels, the smallest k at which they cover 16. Candidate 1's
  series is the unit's mean display luma over the exposure; candidate 2's is the unit's lit
  share. For B a second series is printed beside candidate 1: the count of the unit's
  pixels that leave its lit median by over a tenth for one or two frames and return (the
  dots), as a reading with no line of its own unless candidate 1 fails and it holds.
- **Line (a), restated so a region can meet it.** His places are regions (the top third of
  the picture; the top-left ninth), not tiles. (a) holds when, in his moment, the region
  holds at least twice its share of the scored tiles among the top tenth by score.
- **Line (c) is void:** no clip was answered "no". `T b` ("almost no wobble") is already
  the lower side of line (b) at T.
- **The turn is 40 frames (0.73 s).** `C b`'s and `S`'s moment is the turn, where a
  one-second window does not fit: there the series is the segment and the band starts at
  1.4 Hz.
- **The runs, each under the cap** (RTX only, question 2 (a); one clip or one eye a hold of
  the perf lock). Visibility of a unit a frame is read from the rig's hit positions on a
  render of the clip's own track, not from CPU rays (a ray a face a frame over 645 frames
  at 2530 x 1378 is hours). A render is the shared settle (6 to 15 s at these eyes, Phase
  1's table) and at most 645 frames with one read back each: under 5 minutes a clip, a
  guess, projected from its first 30 frames and stopped past the cap. The scoring is CPU
  work outside the lock; its units are sampled by a stride the dev picks so a clip scores
  in under 5 minutes, and the stride is logged. Plan 0100 Phase 8b's stage rows, read
  again only under a candidate that holds, run one arm a hold under the same rule.
- **What "almost good" changes.** The defaults (`T b`) are close to accepted, and A's
  clearest anchor (`T a`) is not a default: it is the no-upscaler arm. The anchors under
  the defaults are `C a` and `S`. Phase 4 runs as amended either way, since a measure is
  what every later light plan lacks; whether a remedy follows is question 4.
- **Done when**, added to Phase 4's: the W1 split with its stop rule's letter; `S`'s shadow
  change named (a landing, an exposure step, or neither) with the frame; for each artifact
  and candidate the lines (a) and (b) as **holds** or **fails** at that artifact's
  subjects; every run's projection and wall time beside its figures.

### Phase 0b, ruled after its readings (architect, 2026-11-25, late)

From the log's "Phase 0b"; no run stands behind this note. Phase 0b is accepted: both time
bars are met (113 to 146 s on the RTX, 250 to 255 s on lavapipe quiet).

- **Lever 2 is kept.** The check as written above ("the census equals Phase 0's 60/60 with
  54 coarse") was the wrong instrument, and the dev was right not to withdraw on it. The
  census counts the distinct chunks under the lighting pixels' hits of the settle's last
  frame: it is a property of that frame, not of the store. From one eye and equal samples
  it reads 59, 60 or 61 on the RTX with the lever and without it, and Phase 0's own arms
  read 59 to 61. A chunk that enters or leaves the count by one is a chunk a few pixels
  touch in one frame and not in another; lever 2 changes the frame the settle ends on (1 392
  against 3 618 on lavapipe), which is enough to move it. That the cause is the jitter's
  phase is the dev's guess and stays a guess.
- **What the readings do show:** the samples drawn are equal to the sample with and without
  the lever on each adapter (346 782 647 and 236 349 415), every census is whole, and the
  rest figures agree (RTX within 1 point; lavapipe 8.99 against 8.98 and 8.30 against 8.31).
  Equal samples from one eye under one set of knobs means the same jobs at the same tiers
  were drawn; it does not by itself show the same records were stored.
- **The check, restated.** "The same store" is a tally over the store, not over a frame:
  every chunk of the rig's world, settled or not, coarse or full, at the settle's end; with
  the samples drawn. Two settles bake the same store when the tallies and the samples are
  equal. The census in view stays what it was written for: the gate that a reading's own
  picture is baked (whole or NOT WHOLE), with no figure compared across runs.
- **One reading owed, not blocking** (for Phase 4's agent, at a moment the lock is free):
  the store's tally in the settle's end line, and the forest test's settle on lavapipe at
  the half frame with lever 2 and without it (about 141 s and 338 s; lavapipe repeats to
  the sample, the RTX does not). Equal tallies close the 58 against 59. Unequal tallies are
  reported with the chunks that differ, lever 2 is withdrawn from the shared settle, and
  the architect is called: the test then reads 443 s on lavapipe, still under its bar.
- **The further cut (lifting the drain's line slice during the settle): no.** It would take
  most of lavapipe's remaining 141 s, but the bar is met with a wide margin, the drain is
  also what the settle's own end condition reads, and each change to the shared settle
  needs the equality check above, which has no instrument until the tally exists. It is
  not owed; it may be asked for again if the weekly soak's time becomes a complaint.
- **The far +30 constants are confirmed:** 5.52 and 4.88 at 1080p, 8.99 and 8.30 at the
  half frame, each plus 2 at the assert. They follow the rule as written (the largest of
  three readings). Two more RTX readings at fly 180 lie above 5.52 (5.71 the verifying run,
  5.81 without lever 2): the spread over five runs is 5.44 to 5.81, inside the 2 points the
  rule adds for it, so the constant is not moved to follow them. The half frame reads
  higher than 1080p (8.99 against about 5.5), which is why the bar is a constant a size.

### Phase 4 steps 1 and 2, ruled (architect, 2026-11-25, after the readings)

From the log's "Phase 4"; no run stands behind this note.

- **Lever 2 stands, closed.** The store's tallies with and without it are equal byte for
  byte (1 445 chunks, one digest, equal samples). The 58 against 59 was the frame's census.
- **The W1 mismatch: exit (a), and exit (b) with it.**
  - **(a), by its second clause.** Every mismatch is source 0 and the horizon source has 0
    mismatches of 180 532, so the port agrees with the shader wherever the shader reads
    what the port models. Source 0 is the running mean of the key ray's samples (one
    jittered disc ray a pixel a frame, where `key_from_store` is false), a filtered value
    the port does not model. The nine-tenths clause is not met (10.2 % of source 0
    mismatches in the band) and is not needed. **The W1 line is closed as a difference of
    sources:** the port's comparison is restricted to pixels of source 2, and the W1 figure
    is 0 of 180 532. Plan 0100's 1 404 to 1 906 were never a fault of the horizon.
  - **(b) as a seventh candidate in "After Phase 4": the key ray's history mean where the
    store is not read.** Its value at a fixed world point can change between frames by
    construction: the ray is jittered, the mean starts at the column horizon when the
    history resets (a view change) and moves to the rays' mean as the history is taken,
    and it sits in the 32 to 512 voxel band, where the owner puts the brightness of
    distant sunlit trees. What it would change, its cost and its control arm are the
    remedy amendment's to write, not this note's; nothing is built for it now. What would
    falsify it is step 3's reading below.
  - **What step 3 shows for it** (on the renders of `C a`, `C b`, `T a` and `S` it makes
    anyway; the key slot read back with the hits): (1) a map a frame of the lighting
    pixels by key slot source, laid as the candidates' maps are; (2) source 0's share of
    the lighting pixels in the owner's region in his moment against its share of the
    picture, as a ratio (line (a)'s form: twice or more is "lies on his place"); (3) the
    slot's value on units fixed in the world, its mean change a frame at source 0 against
    source 2, in his moment and in the first 40 frames of each hold; (4) of the tiles in
    the top tenth by candidate 1, the share of their lighting pixels that are source 0.
    **Candidate 7 is dropped at the stop** if (2) is under 1 at both `C` clips or (3) shows
    source 0 changing no more than source 2. `C b` runs `key cover ray`, whose slot may
    hold other sources: the rows are printed by source as read, with none merged.
- **`S`'s shadow change: split in two, the control under a pinned exposure.** The owner's
  frame (413, the hold after the turn) has no landing and no exposure step, inside an
  exposure ramp that is flat by about frame 432. It is neither named cause exactly, so the
  rule "a landing or an exposure step goes out" does not settle it.
  - **The whole-picture part is the exposure ramp after a turn.** It is its own line at
    the stop ("Carried to the amendment" already holds the exposure as a source at `S`)
    and is in neither candidate. Both candidates already divide by the frame's exposure or
    must: candidate 2's lit and shadowed levels are taken on luma over the exposure.
  - **The part on the outlines of the sunlit patches is not classed out.** A uniform gain
    does not change outlines more than faces, and the frames after a turn are also where
    the key ray's history mean leaves its seed (candidate 7). One control reading names
    it: `S`'s track rendered once more with the exposure pinned at the value of the turn's
    last frame (397), and the difference of frames 398 and 440 beside the unpinned one
    (about 70 s on the RTX by the log's re-render). Outlines still changing under the pin
    is light, and step 3 scores `S`'s window 398 to 440 with both candidates and the
    source map, as a reading. Outlines flat under the pin is the exposure alone, and the
    window is left out.
  - **Either way the window is in no candidate's lines (a) and (b).** He did not class it
    as either artifact, and one clip with no pair cannot fail or hold a candidate. `S`'s
    line for artifact A stays the slopes in the turn.

No new question for the owner: question 4 stands as asked.

### Question for the owner (added)

4. **After Phase 4, does a remedy follow?** You called the defaults "almost good, esp Tb".
   (a) Phase 4 runs, then you are shown its maps over your screenshots and decide: a small
   remedy at the canopy and the slot, or the plan closes with the measure and the wobble
   accepted as it is. (b) Stop now: close the plan after Phase 0b with no measure. (c)
   Remedy phases whatever Phase 4 reads. **Recommended: (a).** Phase 4 is rig work on the
   RTX with no window, and it leaves a figure shown to follow your eye, which the next
   light plan needs whether or not this one builds anything.

## Amended 2026-11-26 (architect, after Phase 4)

Written by a fresh architect from the log's "Phase 4", the scoring code
(`gpu_wobble/measure.rs`, `keys.rs`), the maps under `live/p4/` and the owner's files. One
reading of the architect's own stands behind it, CPU only, marked where it is used; the dev
repeats it in the log (Phase 5). Nothing above is rewritten; where this section differs from
"Phase 4, amended", this section rules.

### 1. What Phase 4's result means

**Candidate 2 at T: the miss on clip totals is the instrument's, not his eye's.**

- The unit a display sample belongs to is read each frame from the hit of the sample's
  internal pixel (`keys.rs`), and under FSR 2 that hit is jittered while the picture the
  owner saw is the upscaler's output. A sample near a unit's border changes unit from frame
  to frame with the camera still. A unit holds 4 to 16 samples (16 to 64 display pixels on a
  stride of 2), so one sample entering or leaving moves its lit share by 0.06 to 0.25, and
  the score counts a change over 0.1.
- The log already shows it: on `T b`'s holds the key slot's value on units changes 0.0059 a
  frame at source 2, the column horizon, a value fixed per face (ADR-0084) under a camera
  that does not move. On `T a` (no jitter) the same figure is 0.0000.
- The picture itself is nearly still there. The architect's reading (`ffmpeg`, `tblend`
  difference on grey, consecutive frames of his files): `T b` frames 600 to 620, 0.076
  levels mean, 0.018 % of pixels over 8 levels; `T a` 0.000 and 0; flying (150 to 170)
  13.0 and 14.0 levels, 14.8 % and 15.2 %. The score reads `T b`'s holds at a third to a
  half of its flying windows (0.007 to 0.013 against 0.019 to 0.027); the picture's own
  change there is under a hundredth of the flying one.
- So "a steady shimmer under jitter that his eye discounts" is not what the hold figures
  show. They show the sampling. A real shimmer at 0.018 % of pixels may exist; it is small
  and he reported none.
- The hold term alone turns line (b): the windows with the camera moving (the fly and the
  turn, from the log's rows) read about 0.031 for `T a` against 0.022 for `T b`, his order.
  That is arithmetic on a failed line, not a pass. Phase 5 reads it properly.
- **Established:** candidate 2 puts the stepping edge where he pointed in `T a` (0.919
  against 0.323, a place fixed in the picture); it orders T his way wherever the camera
  moves. **Not established:** that it is a measure. T is one pair, and its clips differ in
  resolution as well as in the upscaler (1280 x 720 stretched against 1687 x 919 through
  FSR 2), so it cannot say that the score follows a change of the light at one resolution.

**Candidate 1 at C: nothing is established, either way.**

- The order it is failed on was never given as a ranking. He said "there is wobble" of
  `C a` while flying and "a bit" of `C b` while turning: two moments, two motions. The two
  clips differ by 10 % in every window, inside the fifth that line (b) itself allows a pair
  called the same.
- The totals are taken over the whole picture as a contrast against each unit's own mean.
  68 % of the canopy picture is shadowed and he named the lit trees; a shadowed unit at 1.4
  to 2.0 against a lit level of 61.8 weighs the same as a lit crown.
- In motion a crown's unit is sparse leaves in front of dark gaps, and its mean changes
  with which samples reach it. That may be exactly what he calls dots sparkling, and no
  light setting changes it; `key cover ray` changing it by a tenth is as likely the ray's
  own noise in the shadow.
- The line (a) miss at `C a` (1.89 against 2) carries nothing: the map shows the sunlit
  crowns hot across the whole top, right as much as left, and he named where he looked.
- **Established:** candidate 1 lights up sunlit crowns in motion. **Not established:** that
  it orders anything; and no pair exists that could test it.

**The dev's region rules** (`C b`: tiles at least 5 % sunlit leaf; `S`: at least 5 % sunlit
and not leaf) are fair readings of his words and invalidate no line. They make line (a) weak
there: a score of brightness activity can hardly land anywhere else. Only `T a`'s and
`C a`'s regions are places fixed in the picture, and only those two lines (a) are tests.

**Candidate 7 is dropped**, as its rule says: no sun-facing sample of his five clips reads
the key ray's history mean. It goes to no backlog entry; W1's line stays closed.

**`S` after the turn.** `control-pinned-at-397-398-440-x16.png` shows every edge of the
picture changing in the 0.8 s after the turn: the block edges of the shadowed wall and the
silhouettes against the sky as much as the lit treads; 2 s later nearly nothing. A process
that touches every geometric edge alike is not the sun's term. The dev's reading (FSR 2's
history filling after a turn) is the likely one and is not yet shown: Phase 5 shows it or
refutes it in one render. The whole picture's part is the exposure settling after the turn
(2.70 levels against 0.08 pinned), which is established.

**In one paragraph.** Under the defaults the stepping edge is close to gone ("almost no
wobble" at `T b`, "very little" on `S`'s slopes). What is left under the defaults is the
sunlit crowns in motion (`C a`) and the second after a turn (`S`). Neither is shown to be
the sun's visibility, which is what this plan's six candidates remedy.

### 2. The next step

**Tonight: Phase 5 below, two readings, minutes each. Then the owner's question, and the
recommended answer to it is to close.**

- **Rejected for tonight: closing at once** (the plan's outcome being the anchors, the
  tools and the audit). Closing with the wobble parked is his decision (question 4), and
  the two readings cost minutes and change what he is told: whether the plan leaves a
  reading for the stepping edge, and whether the second after a turn belongs to the light.
- **Rejected: a remedy candidate judged by blind clips, now.** A remedy is a stability
  decision and his. And the code reading names none that is small: of the six candidates
  five change the sun's term, and the sun's term is not shown to be what is left under the
  defaults. The one setting he has twice found calmer at the canopy, `key cover ray`, is
  offered in the question as the only round worth his eye.
- **Rejected: more measure variants.** Phase 5 is one correction that the code reading
  names, with what refutes it written down first. Whatever it reads, no further variant of
  either candidate is tried in this plan.

### Phase 5: Two readings that separate the causes (exploration, a reading)
- **Owner skill:** dev
- **What:** No production code; `apps/flythrough/tests/gpu_wobble/` and the untracked
  `live/p4/` only. The cap and the runner are Phase 4's.
  - **5a, the unit map held still where the camera is still (CPU, outside the lock, about
    half a minute a clip).** In every hold window a sample's unit is the one the window's
    first frame gives it, for the whole window; the luma is each frame's own, as now. The
    moving windows are scored as now. Printed for the five clips: each hold window old
    beside new (candidate 1, candidate 2, dots), the clip totals old beside new, line (b)
    at T for candidate 2. Beside each hold window, the picture's own change: the mean
    absolute difference of consecutive frames and the share of pixels over 8 levels (the
    architect's reading above, repeated on every hold).
  - **What 5a is expected to read, and what refutes it.** Expected: `T b`'s hold windows
    fall under 0.002 on candidate 2 and the clip totals read about 0.015 (`T a`) against
    0.011 (`T b`). Refuted: `T b`'s holds stay over a third of what they read now. Then the
    sampling is not the cause, candidate 2 stays failed at T, and nothing more is tried.
  - **One row beside it, with no line:** candidate 1 over the units whose samples are
    mostly sunlit leaf, `C a` beside `C b`, the clip totals and his two windows. It says
    whether `key cover ray`'s extra tenth sits in the shadow. It holds or fails nothing:
    C's order was never given as a ranking.
  - **5b, `S` after the turn with no upscaler (RTX, one hold of the lock, about a minute;
    projected from its first 31 frames).** `S`'s track rendered with the upscaler off at
    the window's internal size (1687 x 919, presented as `T a`'s arm presents), the
    exposure pinned after frame 397. The clip rig's `native` arm takes `T` only today and
    is widened to `S`. Then `control`'s figures and pictures on frames 398 against 440 and
    500 against 542. With no upscaler, no jitter and a still camera two frames differ only
    where the light does.
  - **How 5b is read.** Outlines at 398 against 440 under 3 levels (the settled pair reads
    2.3 with FSR 2): the change after a turn in his clip is the upscaler filling its
    history, not the light. Over 11 levels (half of his clip's 22): it is the light, and
    the dev says on which outlines and which queue counter or fade moves in those frames.
    Between: both, with the figures.
  - **`live/p4/index.md` is rewritten for the morning** as section 3 below rules.
- **Files touched:** `apps/flythrough/tests/gpu_wobble/measure.rs`, `clips.rs`,
  `docs/plans/logs/0104.md`.
- **Done when:** the log holds 5a's table with **holds** or **refuted** against the two
  figures written above, the sunlit-leaf row, 5b's two pairs beside Phase 4's three with
  one of the three readings named, each run's projection, wall time and load; `index.md`
  as ruled; quick tier, clippy and fmt over the changed test files. **If 5a holds**,
  candidate 2 is entered as this plan's reading for the stepping edge, provisional: it
  held (a) at a fixed place and (b) on one pair, and it is called a measure when it ranks
  a second pair he ranked blind. Plan 0100 Phase 8b's stage rows are not read under it
  tonight. The dev stops after Phase 5 either way.

### 3. What the owner is shown in the morning

**Four pictures, in this order, each with its one sentence.** The rest of `live/p4/` stays
on disk and is not put in front of him.

1. `map-T-a-c2.png`: "The clip with no upscaler, at your screenshot: the hot squares are
   the far stepped slope at the top, where you pointed."
2. `map-T-b-c2.png`: "The same moment under the defaults, which you called almost no
   wobble: the same slope, about a quarter cooler while flying."
3. `map-C-a-c1.png`: "The forest under the defaults, at your screenshot: the sunlit crowns
   flicker across the whole top of the picture, not only the top left."
4. `control-pinned-at-397-398-440-x16.png` beside `control-pinned-at-397-500-542-x16.png`:
   "The tunnel, what changes in the second after the turn and what changes two seconds
   later: every thin white line is an edge that moved", with Phase 5b's one line under it
   (the upscaler, the light, or both).

**`live/p4/index.md` is not fit as it stands. The dev changes it** (Phase 5):

- It shows fifteen pictures in four tables. It shows the four above, in that order, and
  names the others in one closing line.
- Each map is tinted against its own largest value, so `map-T-b-c2.png` looks as hot as
  `map-T-a-c2.png` while the text says "cooler". The two T maps are drawn again on one
  shared scale (the larger of the two maxima), or the sentence goes.
- "T b keeps a small steady shimmer ... the upscaler's jitter" is replaced by what 5a
  reads. As written it tells him his picture shimmers where the score's sampling does.
- The difference pictures are near black: one sentence says they are sixteen times
  brightened and how to read a line.
- It opens with what is left under the defaults, in his words, not with the scores; the
  scores' definitions move under the pictures.
- The key slot's source section goes (candidate 7 is dropped); one line says so.
- It ends with the question below, verbatim.

**The question (question 4, restated; it replaces the wording under "Question for the
owner (added)", whose choice (a) he is now at).**

4. **The defaults are, in your words, almost good. Two things are left under them: sunlit
   tree crowns sparkle while you move, and the picture settles for about a second after a
   turn. What now?**
   - **(a) Close the plan. Recommended.** The two leftovers go to the backlog with your
     clips and screenshots as their anchors, beside "Distant trees that do not wobble".
     *For:* the stepping edge you first complained of is close to gone under the defaults;
     what is left is not shown to be the sun's shadow, which is all this plan's remedies
     change; the trees and the upscaler deserve their own plan with an interview. The clip
     rig, the blind pairs, the baked settle and the soak test that now reads a baked store
     stay. *Against:* a known flicker stays in the default picture, parked, and stable
     image is your first priority.
   - **(b) One more blind round, at the forest only.** You watch the two forest clips
     again and one more pair from a second forest eye, defaults against `key cover ray`,
     and say which of each pair wobbles more. If `ray` is calmer in both, its cost is read
     on the RTX and the Radeon and it becomes the default by an amendment; if not, (a).
     *For:* it is the one change you have twice found calmer there, the setting exists,
     and your eye is the only instrument that has repeated itself. *Against:* about a day
     of lane work and ten minutes of yours; `C b` still showed "a bit" when turning; you
     kept `ray` off as a default in Plan 0100 (its question 8).
   - **(c) A remedy for the second after a turn.** Only worth asking if Phase 5b reads
     "the light". If it reads "the upscaler", this is FSR 2's own behaviour and belongs in
     the backlog entry under (a).
   - **With (a), one thing more:** your answer 3 (a) removes every unpicked Plan 0100
     setting at the close. **Recommended:** remove `key_cover record`, `key_at face` and
     `face-smooth` and `key_horizon walk` as answered, and keep `key_cover ray` alone,
     off, because it is the parked forest item's one control and costs nothing when off
     (your rule of 2026-11-12). The close review records that against ADR-0084. *Or:*
     remove it too, as answered; a later plan builds it again if wanted.

### 4. Phase 0b and main: no partial close

- **The premise does not hold as it was put.** The weekly soak (`soak.yml`, Mondays 03:00
  UTC) runs what is on `origin`, and `origin/main` is behind the local main; a merge into
  the local main changes no CI run until the owner asks for a push. Main holds the test as
  it was before Phase 0 (120 unread frames, passing on a store 5 of 59 chunks baked, with
  the old bars), not the 12 538 s test: that one only ever existed on this lane. The old
  test asserts less than it claims; it does not run long.
- **So nothing merges tonight.** A merge into main takes the gate tier at the lane's tip,
  about a quarter of an hour of the GPU lock, and tonight it would buy nothing.
- **If he answers (a):** the plan closes in the usual ceremony and the lane merges whole,
  days before the next weekly run. The push is his word, as always.
- **If the lane stays open past Sunday 2026-11-29** (answer (b) or (c)), the orchestrator
  makes one interim merge, which is not a close: the gate tier at the lane's tip and the
  two soak tests by name on lavapipe (about 250 s and 100 s), then
  `git merge --no-ff plan-0104` in the main checkout with a message that says "interim:
  Phases 0, 0b, 1, 2, 4, 5; the plan stays open". No version, no tag, no move to `done/`,
  no close review, the index row unchanged, the worktree and the branch kept; the lane
  goes on from the same branch and merges again at its close. The tip and not Phase 0b's
  commit alone: the lane changes test code, one comment line and documents, its readings
  sit under `reading::` (in no tier), and the tip holds the store's tally that closed
  lever 2. Picking Phase 0b's commit out would write it twice into the history.

## Amended 2026-11-26 (owner)

The owner's answer to question 4 as restated above, and the plan's last dev phase. No run
stands behind this section; every time in it that is not quoted from the log is a guess.
Nothing above is rewritten; where this section differs, this section rules.

### The answer

**Question 4: (a), close** (owner, 2026-11-26, morning, after `live/p4/index.md`). The plan
closes with no remedy. The two things left under the defaults are parked in
[`docs/backlog.md`](../backlog.md), each with his clips as its anchor:

- **Sunlit tree crowns sparkle while he moves** (`C a`, `C b`): "Sunlit tree crowns sparkle
  in motion under the defaults".
- **The picture settles for about a second after a turn** (`S`): "A second of settling after
  a turn: FSR 2's history". Phase 5b read it as the upscaler's: the outlines at frames 398
  against 440 change 22.05 levels with FSR 2 and the exposure pinned, 0.007 with no upscaler.

**The sub-question ("with (a), one thing more") was not answered.** It is not a stability
question, so it takes the recommended option (his standing rule of 2026-11-03):
`lighting.key_cover ray` stays a setting, off on every profile, as the parked crowns' one
control (his rule of 2026-11-12: a mode that costs nothing when off stays). His answer 3 (a)
holds for the rest: `key_cover record`, `key_at face` and `face-smooth`, and `key_horizon
walk` are removed at the close. A setting left with one value is no setting, so
`lighting.key_at` and `lighting.key_horizon` go whole; the horizon is read at the hit by the
march, as every profile reads it today.

### What the answer closes

- **No remedy phase is written.** The six candidates of "After Phase 4" stay a list in a
  closed plan; candidate 7 was dropped at Phase 4. The stop's amendment is this section.
- **No further blind round** (choice (b), the forest under `key cover ray`) and **no remedy
  for the second after a turn** (choice (c)).
- **Phase 3 stays not run**; `--track-out` and `--track` are not built.
- **No measure.** Candidate 2 with held units is this plan's **reading** for the stepping
  edge, provisional: line (a) at a place fixed in the picture (`T a`) and line (b) on one
  pair (`T a` 0.01507 against `T b` 0.01123). It is called a measure when it ranks a second
  pair he ranked blind. Candidate 1 orders nothing. Plan 0100 Phase 8b's stage rows are not
  read under either.
- **No ADR.** No remedy has a rejected alternative. The removal of the three settings is
  ADR-0084's own clause ("a setting the follow-on plan does not pick is removed there");
  that `key_cover ray` stays is recorded against it by the close review, as an addition to
  its Outcome.
- **No default, no golden and no image bar changes.** The "last dev phase" that "After
  Phase 4" names (picks as defaults with their goldens) has no pick to write. Phase 6 below
  takes its place.
- **Also parked, in the backlog's "The sun's term at small faces":**
  the near shadow's hole with the contact ray as its arm, and sun under cover as a default.
  Neither was taken up here.

### What stays and what goes (the plan decides; the dev does not re-decide it)

| thing | ruling |
|---|---|
| `lighting.key_cover` | stays, `off` and `ray`, `off` on every profile; its row, `--key-cover off\|ray`, its menu entry and HUD word stay as spelled; `inert.rs`'s clause on it stays |
| `key_cover record` | removed: the enum's value, `LIGHTING2_KEY_COVER_RECORD`, the shader paths only it reaches (the face's own cell's key read in `temporal.wgsl`, the record clause of `key_cover_far` in `pair.wgsl`), the settings walk's special case |
| `lighting.key_at` | removed whole: `KeyAt`, the field, the row, `--key-at`, the menu entry, the HUD word, the two `lighting2` bits; `key_horizon_at` reads at the hit |
| `lighting.key_horizon` | removed whole: `KeyHorizon`, the field, the row, `--key-horizon`, the menu entry, the HUD word, `LIGHTING2_KEY_WALK`, `key_walk.wgsl` and `KEY_WALK_WGSL`, the column map's coarse rows with their upload, `column_coarse_texel`, `GpuWorld::column_coarse_bytes` and their tests |
| `Params::lighting2`'s freed bits | left free and named free in the comment that names bit 3; no bit is renumbered |
| `key far baked`, `key far column`, the bake's key ray at its cap | untouched: they are ADR-0079's and ADR-0084's standing clauses, not this plan's controls |
| `support/baked.rs` (the shared settle, the census, the store's tally) | stays as it is, lever 2 included |
| the two soak tests of `gpu_wobble` | stay `#[ignore = "soak"]` as Phase 0b left them, the far +30 constants as confirmed |
| `gpu_wobble/clips.rs` (the clip rig) | stays, a `mod reading`; it is how an anchor is rendered again |
| `gpu_wobble/keys.rs`, `measure.rs` | stay as readings, cut to what a future plan reads: candidate 2 and candidate 1 with the dots, hold windows scored with held units only, the sunlit-leaf row, the picture's own change, the shared-scale maps, `control`. Candidate 7 goes whole (its maps, ratio, change rows and top-tenth share, and the key slot's source and value in the `.keys` record if nothing else reads them) |
| `gpu_wobble/walked.rs`, `cover_arms.rs`, `cover.rs`, the rig's tokens | what reads only a removed value goes (the `walk` rows, the `record` arm, `keyat=`, `keyhorizon=`); `keycover=off\|ray` stays; the W1 comparison stays, restricted to the horizon's source as ruled |
| `gpu_wobble/owner_move.rs` and the binary's other readings | stay |
| `live/p4/*.keys` (about 15 GB, untracked) | deleted; `keys.rs` renders them again from a clip's track in minutes |
| `live/anchor/` and the pictures of `live/p4/` (untracked) | kept: they are the backlog's anchors. The orchestrator copies them to the main checkout's `live/anchor-0104/` before the worktree is removed |

### Phase 6: The close's code, readings and gate (three parts, one dev brief each)

Not exploration: rule 2 of the lane rules holds as written. The parts run in this order,
each its own commit. The owner's rules stand in every part: no run projected past 20 minutes
under the perf lock, the gate tier is the most any step runs, no scene is simplified,
nothing is windowed.

**Phase 6a: Plan 0100's unpicked settings removed**
- **Owner skill:** dev
- **What:** The table above, rows one to six, in the engine, the fly-through and every test
  that names a removed value. A test that asserts only a removed value goes with it. A test
  that asserts a kept claim through a removed value is pointed at the kept path, its claim
  and bar as written; one that cannot be is reported, not dropped. No default changes, so
  no golden is blessed. `docs/flythrough.md` follows `flythrough --help` and `--keys`: the
  page names `key cover` as `off, ray` and no longer names `key at` or `key horizon`.
  `docs/nfr/03-memory.md`'s entry "The column map's coarse levels" is closed with the bytes
  the removal returns (arithmetic, the window sides it names). `CHANGELOG.md`, the version,
  `docs/roadmap.md` and ADR-0084 are the close ceremony's, not this part's.
- **Files touched (at least):** `crates/svet-render/src/frame/settings.rs`,
  `frame/settings/sky_light.rs`, `frame/compose.rs`, `frame/pipelines.rs`, `frame/mod.rs`,
  `settings.rs`, `lib.rs`, `gpu_world/columns.rs`, `gpu_world/mod.rs`, `layout_tests.rs`,
  `shaders/key_horizon.wgsl`, `key_walk.wgsl` (deleted), `pair.wgsl`, `temporal.wgsl`,
  `sky_light.wgsl`, `common.wgsl`, their unit tests, `crates/svet-render/tests/gpu_sun*`;
  `apps/flythrough/src/cli/`, `toggles.rs`, `hud/modes.rs`, their tests,
  `apps/flythrough/tests/gpu_wobble.rs` and `gpu_wobble/{walked,cover,cover_arms}.rs`;
  `docs/flythrough.md`, `docs/nfr/03-memory.md`, `docs/plans/logs/0104.md`.
- **Done when:**
  - `grep` finds `key_at`, `key_horizon` (the setting, not the shader file
    `key_horizon.wgsl` and its `column_key_horizon`), `KeyAt`, `KeyHorizon`, `key_walk` and
    `KeyCover::Record` nowhere under `crates/` and `apps/`; `--key-cover record`,
    `--key-at face` and `--key-horizon walk` each fail to parse with the flag's own error,
    asserted by a CLI test;
  - a settings file that names a removed row: what the reader does with it (an error by
    name, or the row ignored) is read from the code and written in the log, not changed;
  - the frame's and the mirror's byte assertions that move, move by the coarse rows alone,
    old beside new in the log;
  - the quick tier, the phase's clippy, fmt, every `scripts/check-*.mjs`; then one hold of
    the perf lock, `--profile gpu --binaries gpu_sun gpu_settings gpu_golden
    gpu_outdoor_golden` plus the binaries the script adds for the touched files, projected
    from its first binaries and split in two holds if past 20 minutes: 0 failures, **no
    golden differs**. `gpu_wobble` builds; its soak tests are not run here (6c).
- **Stops, reported:** a golden or an image bar that moves under the defaults (then a
  removed path was reached by a default, and the architect rules); a kept test whose claim
  cannot be made without a removed value; a consumer-facing type outside this list that
  must change.

**Phase 6b: The rig cut to what stays, the anchors named, the pins**
- **Owner skill:** dev
- **What:** No production code.
  1. **The rig**, rows seven to thirteen of the table: `measure.rs` and `keys.rs` cut as
     ruled, the module comments rewritten to say what the file is for now (no plan-relative
     history), the binary's doc comment naming each reading's spec. The check that the cut
     changed no figure is CPU only: `SVET_MEASURE=T-a,T-b` scored before the `.keys` are
     deleted, on the cut code, reads `T a` 0.01507 and `T b` 0.01123 (candidate 2, held)
     and the sunlit-leaf row is repeated on `C-a,C-b` (0.02480 against 0.02476). If the
     `.keys` record's layout changes with the cut, the check runs before that change and
     the log says so.
  2. **`live/p4/*.keys` deleted**, after the check; the bytes freed in the log.
  3. **The anchors named.** In the log, one table: each of the five clips of `live/anchor/`
     and each picture the backlog entries name, with its `sha256sum`, its size and the
     `SVET_CLIP` or `SVET_MEASURE` spec that renders it again. Nothing is copied or moved:
     the orchestrator does that at the merge.
  4. **The pins.** `docs/nfr/07-startup-and-build.md`'s soak table: the forest soak test's
     time as Phase 0b read it (the RTX and lavapipe, quiet, with the frame sizes) and the
     binary's other soak test on the shared settle, each with machine, adapter, load and
     commit, quoted from the log, not read again. `docs/nfr.md` section 7 only if its text
     names either test. No frame, memory or edit line of `docs/nfr.md` moves but 6a's.
  5. **The log's ends tied:** a "Phase 6" section; any resume notes deleted.
- **Files touched:** `apps/flythrough/tests/gpu_wobble.rs`, `gpu_wobble/measure.rs`,
  `keys.rs`, `docs/nfr/07-startup-and-build.md`, `docs/plans/logs/0104.md`.
- **Done when:** the two figures of item 1 repeat to the last digit printed, or the
  difference is reported and the cut that caused it withdrawn; `ls live/p4/*.keys` finds
  nothing; the anchors' table is in the log; the quick tier, the phase's clippy (it builds
  `gpu_wobble`), fmt, every `scripts/check-*.mjs`. No GPU, no lock.

**Phase 6c: Main merged in, and the tip's gate in parts**
- **Owner skill:** dev
- **The order to expect.** Plan 0099 closes and merges into main first (his question 1 (a)
  put this plan's last phases on a main that holds it; its lane has already merged main and
  gated). **6c starts on the orchestrator's word that main holds Plan 0099.** If the
  orchestrator says this plan goes first instead, main is still this lane's base
  (`c87bf7a6`), `git merge --no-ff main` reports nothing to merge, no merge commit is made,
  the log says so, and the gate below runs on the lane's tip; Plan 0099 then merges main
  again in its own lane. **Whichever plan merges second takes the next version**: the
  number is the close ceremony's, and the dev writes none.
- **What:** `git merge --no-ff main` into `plan-0104`, conflicts resolved with both sides'
  behaviour kept (Plan 0099's flag and rows beside this plan's removed ones in
  `apps/flythrough/src/cli/`, `toggles.rs` and `hud/modes.rs` are the likely place),
  committed as lane rule 1 words it. Then the gate on the merged tree, in parts:
  - outside the lock: the build; `cargo clippy --workspace --all-targets -- -D warnings`;
    fmt; `cargo deny check`; the doctests; the gate tier's CPU part
    (`--profile gate -E 'not binary(/^gpu_/)'`); every `scripts/check-*.mjs`; the test
    archive for the holds below;
  - under the lock, from the archive: the gate tier's `gpu_*` binaries in holds of at most
    20 minutes each, the binaries listed first and dealt into holds by their times in the
    last gate log, a hold projected past the line from its first binaries stopped and
    split. Plan 0099's Phase 6c rig is the model (`live/p6c-gpu.sh` in the plan-0099
    worktree, read there, written anew under this lane's `live/`; nothing is run in
    another worktree);
  - under the lock, each its own hold: `node scripts/test-changed.mjs --profile gate --sole
    --since main`; the two soak tests of `gpu_wobble` by name on lavapipe (about 250 s and
    100 s by Phase 0b's readings), since this lane changed both.
  Then the Implementation log's row for Phase 6 and the Close triggers, as the template
  asks.
- **Done when:** every part's count, wall and load in the log, the parts' walls summed
  against `docs/nfr.md` section 7; no golden moved, or the one that did is reported with
  the frame it differs in, not re-blessed; both soak tests pass inside Phase 0b's time
  bars (600 s the forest's on lavapipe), their times in the log.
- **Stops, reported:** a conflict whose two sides cannot both be kept; a test that fails on
  the merged tree and passes on each parent; the `gpu_*` binaries cannot be dealt into
  holds under the line; a soak test over its bar.

### What the close review checks

A fresh session, after 6c. The gate tier is not run again without cause (owner,
2026-11-19); the review reads the log's parts and opens the assertions.

1. **The answer is carried out as recorded:** `key_cover` is `off|ray` and `off` on every
   profile; the three removed settings are gone from code, flags, menu, HUD and
   `docs/flythrough.md`; no default, golden or image bar moved (6a's golden holds, 6c's
   gate).
2. **The removal reached no default path:** the kept tests that were re-pointed still
   assert their claim (the reviewer opens each); the byte assertions moved by the coarse
   rows alone.
3. **The soak tests assert on a baked store** (Phase 0b's contract: one store, a whole
   census or a failure, the far +30 constants of the ruling) and pass on lavapipe inside
   their bars.
4. **The readings are what the plan says they are:** 6b's two figures repeated; no
   asserting test was added for a candidate; `mod reading` holds every rig entry.
5. **The anchors exist and are named:** the five clips and the pictures with their
   checksums, the backlog's three entries pointing at them, and the orchestrator's copy to
   `live/anchor-0104/` made before the worktree goes.
6. **Disclosed, not repaired:** candidate 2 is a reading on one pair; the under-0.002
   figure of Phase 5a was missed on each hold's first window; the lit crowns' cause is not
   shown; `C`'s order was never given as a ranking.
7. **The ceremony's own:** the version (the second of this plan and Plan 0099 to merge
   renumbers), `CHANGELOG.md` with the removed public items (`KeyAt`, `KeyHorizon`,
   `KeyCover::Record`, `Lighting::key_at` and `key_horizon`, `KEY_WALK_WGSL`,
   `GpuWorld::column_coarse_bytes`, the two flags), the addition to ADR-0084's Outcome,
   `docs/roadmap.md`'s M10 lines that name the removed settings, the index row, the move to
   `done/`.

### Owner questions

None open. A push stays his word.

## After Phase 4: the stop, and the candidate remedies

**The plan stops for the architect after Phase 4** (or earlier, where Phase 2 anchors
nothing and Phase 3 shows nothing headless). The remedy phases are written as an amendment
from the anchored clips and the measure, with an ADR once a remedy has a rejected
alternative. If both measures failed, the amendment gates its remedies on clips and the
owner's eye alone, with no figure. The amendment also writes the plan's last dev phase (the
picks as defaults with their goldens, image bars, budget lines and documents) and the
owner's window before it.

What follows is a list of candidates from Plan 0100's code reading (`logs/0100.md`, "The
architect's code reading after the window"). **None is picked, and the list is not an
order.** A cost marked a guess has no reading behind it.

| candidate | what it would change | what it would cost | what would falsify it |
|---|---|---|---|
| **1. The sun's term at the internal rate** | Today the whole sun term (cosine times visibility) is evaluated once for each 2 x 2 internal pixels, at a hit the jitter or the camera moves. Evaluate it at every internal pixel. | Four times the key term's share of the temporal pass, which is not split out: a guess. The whole lighting reads 0.22 to 0.42 ms on the RTX and 1.61 to 2.07 ms on the RENOIR at Floor (Plan 0098 Phase 13). | A clip under the rig's `full` token (all lighting at the internal rate, its ceiling) that the owner does not rank calmer than the defaults, blind, at an anchored place. |
| **2. The cosine's step at face edges** | `max(n.l, 0)` takes the face's own normal near, so a tread at 0.707 sits beside a riser at 0. Shade the key term's cosine by a normal that changes smoothly across an edge. | The gradient normal's taps at near hits (a guess); and it changes the look of honest cubes, the default the owner picked on 2026-11-01, so it is his call by eye. | A clip with the far normal's weight at 1 everywhere (the rig's `farn` token) not ranked calmer; or the owner refusing the shaded cubes. |
| **3. The 0-or-1 horizon read 2.5 voxels off the face** | The visibility is a hard step read at a point off the face, in a different map cell for each face of one stair, and under `march` its 12 sampled cells change with the hit's place. Make it continuous across a surface, or the same for a face whatever the hit. | `key horizon walk` read +1.41 ms of temporal pass on the RENOIR at the terraces eye (Plan 0100 Phase 7); other forms unread. | Clips under `keyhorizon=walk` and `keyat=face-smooth` not ranked calmer; a clip with the visibility forced to 1 (the cosine alone; a rig token to add) that wobbles the same. |
| **4. `lit_at`'s nearest-tap fallback** | Where all four upsample weights vanish, an internal pixel takes the nearest lighting pixel's light whatever its face. Give those pixels their own term instead. | One key evaluation at the fallback pixels; their share of a frame is unread. | A map of the fallback pixels a frame that does not fall on the places the owner named, or a share too small to see there. |
| **5. FSR 2 as the only antialiasing in motion** | In motion an edge of the sun's term advances a lighting pixel at a time while the geometry under it moves smoothly, and nothing else in the frame antialiases it. Antialias the sun's edge inside the internal pixel, or give FSR 2 inputs that hold it. | Unread. Input tuning costs no time; a coverage term costs a second lookup at edge pixels (a guess). | The owner not ranking `T`'s no-upscaler clip worse, blind (then "the upscaler hides part of it" was never established); a clip rendered at the display's size with FSR 2 at scale 1 not ranked calmer. |
| **6. A contact ray for the near shadow** | A ray a few voxels long from a near face's centre that may darken what the horizon lights: it closes the hole (the terraces' wrongly lit near faces 3 229 to 1 357, the slot's 144 to 31; Plan 0100 Phases 5 and 5b, CPU). It is a remedy for the hole, not for the wobble. | At most the key ray Plan 0098 Phase 13 removed (+0.3 to +1.4 ms on the RTX, +4.0 to +9.7 ms on the RENOIR at Floor); a ray of 4 voxels should cost less: unread. It is the one exception to "a cover source only adds", so it takes an ADR that supersedes that clause of ADR-0084. | It adds shadow edges on the terraces: a clip with it that the owner ranks as wobbling more at an anchored place. |

**One reading cuts across candidates 1 to 3:** a clip with the key light off (`keyoff`). If
the owner still answers "yes, that is it" there, the sun's term is not the source at that
subject and all three fall together. The amendment asks for that clip first.

**Carried to the amendment, behind the wobble:** sun under cover as a default
(`key_cover ray`, `record`, or a new source), with the question whether the key bake runs
while no default pixel reads it; `key horizon walk` as a default; the slot's shimmer, whose
other possible sources (the sky term's source switching while blocks land, the exposure
metering a frame split between a dark tunnel and a sunlit cut) Phase 2's words for clip `S`
choose between.

**Profile rows.** None until a remedy is picked: Phases 0 to 4 add no render setting and
move no profile's value. The amendment names each new row's apply class and its value a
profile (ADR-0055).

**Plan 0100's settings.** `lighting.key_cover ray|record`, `lighting.key_at face|
face-smooth` and `lighting.key_horizon walk` were kept as controls for this plan. **Any of
them this plan does not pick is removed at its close** (ADR-0084: "a setting the follow-on
plan does not pick is removed there, not kept twice"), with its rows, flags, HUD words and
documents. Question 3.

## Data shapes

No engine type changes in Phases 0 to 4. Two text formats, both illustrative; the dev fixes
the spelling:

```text
# a clip's side file (Phase 1), one line a frame
# clip T-a  2530x1378  step 1/55  bake lockstep  census 617/617
frame eye_x eye_y eye_z yaw pitch exposure
0     496.0 -648.7 456.7 34.5 -46.2 0.01834

# a track (Phase 3), one line a frame, a mark where the key was pressed
# track v1  2530x1378  profile floor  bake thread on
frame dt_ms eye_x eye_y eye_z yaw pitch mark
```

## Budgets

- **No `docs/nfr.md` figure is touched by Phases 0 to 4.** No shader and no default changes.
  The remedy's figures (the lighting line of section 2, the frame at Floor and Portable) are
  named by the amendment and measured on the RTX at Floor and the RENOIR at Portable.
- **The gate's wall (section 7).** Phase 3 adds one GPU test under 10 s (a guess) and CPU
  tests under 1 s each. Phase 0 changes two soak tests, which are in no gate; a device wait
  every settle frame makes them slower, and the log holds by how much.
- **Disk, the dev box.** The clips' raw frames are 45 GB if kept (Phase 1's arithmetic), so
  they are not kept; the lossless files' sizes are unread and go in the log.
- **The owner's time.** Phase 2 is five to ten minutes. Phase 3, if it runs, is a minute a
  subject in a window and the same questions again.

## Risks & open questions

**Answered 2026-11-25 (owner, "go with recommended + 3a"):** question 1 **(a)**: Phases 0 to 2
and 4 start now beside Plan 0099; Phase 3 and the remedy phases start on a main that holds
Plan 0099. Question 2 **(a)**: the clips cover the RTX at Floor only. Question 3 **(a)**: a
Plan 0100 setting this plan does not pick is removed at its close (the owner chose this over
his 2026-11-12 rule that cheap unpicked modes stay settings, for these settings).

**Answered 2026-11-26 (owner):** question 4 **(a)**, close; its sub-question unanswered and
taken as recommended ("Amended 2026-11-26 (owner)").

- **The clip does not show it either** (likely for at least one subject: the rig is not the
  window). That is Phase 3's reason to exist, and a "not shown headless" there is a result:
  it puts the fault between the headless frame and the window's present, or between the
  bake's two modes.
- **The owner's word names more than one artifact.** Phase 2's third question splits them;
  the amendment may then be two remedies with two anchors, or two plans.
- **Neither measure holds.** Then every remedy is judged on blind clip pairs and nothing
  else. Slower, and honest; the plan says so in advance.
- **Plan 0099 changes when light lands in a window.** With `bake thread on` the render
  thread no longer waits for a step, so a landing may show a frame or more later than in
  lockstep. The clips settle the view before frame 1, so the held segments should not
  differ; the flying segment enters chunks whose blocks land during the move, and there the
  two modes can differ. Every clip and track names its mode for that reason.
- **Two lanes on one GPU.** The clip runs and Plan 0099's readings both take the RTX under
  the perf lock; a clip run is minutes, and the lock serialises them.
- **A soak bar fails on a baked store** (Phase 0). Then Plan 0098's rest bars were passed on
  a store of unknown state; the dev stops and the architect rules, before any bar moves.
- **Determinism, memory, GPU features:** nothing in `svet-world`, no new buffer, no adapter
  feature. Phase 3's track holds frame times read from the window's clock in the app; the
  headless replay reads them from the file, so the replay is reproducible.

### Questions for the owner

1. **Does this plan's lane start now, beside Plan 0099?** (a) Phases 0 to 2 and 4 now,
   beside Plan 0099 (they are rig and clip work under `apps/flythrough/tests/` and `live/`,
   plus one comment line); Phase 3 and the remedy phases start on a main that holds Plan
   0099. (b) The whole plan waits for Plan 0099's close. (c) The whole plan now, Phase 3
   included, merging Plan 0099 in as it lands. **Recommended: (a).** The viewing is the
   slowest step and needs nothing of Plan 0099; Phase 3 edits the same app directories as
   Plan 0099's flag, and a remedy is judged in a window that by then runs `bake thread on`.
2. **Do the clips also cover the Radeon at Portable?** (a) The RTX at Floor only, the five
   clips as written. (b) Add one canopy clip rendered on the RENOIR at Portable at 1280 x
   720 ("wobble is hard", your note 5). **Recommended: (a).** Portable took nothing from
   Plan 0100's window, the anchor is about what the wobble is and not which card shows
   more of it, and a sixth clip lengthens the viewing.
3. **Plan 0100's settings this plan does not pick: removed at its close, or kept?** (a)
   Removed, as ADR-0084 says. (b) Those that cost nothing when off stay settings (your rule
   of 2026-11-12), and only `key horizon walk`, which uploads its coarse rows when off, is
   removed if unpicked. **Recommended: (a).** They were kept as this plan's controls, not
   as modes anyone runs, and each is a row the settings walk, the menu and the documents
   carry.

## What this plan does NOT do

- **Pick or build a remedy.** Not before the stop; the candidates' table is not a design.
- **The gate-sized rig for the bake's key ray at its cap.** It stays in the backlog ("Plan
  0100's leftovers") for Plan 0101, which works in that file.
- **Retune a bar of Plan 0098**, or any golden. Phase 0 reports; the architect rules.
- **Touch `crates/svet-render/src/bake/queue/` or the fly-through's frame path** in Phases
  0 to 2 and 4 (Plan 0099's ground).
- **Distant trees under FSR 2** (the backlog's "Distant trees that do not wobble", Plan
  0033's re-read), unless Phase 2's words put the owner's wobble there.
- **A penumbra of the disc's true width, entities' and field surfaces' shadow edges, the
  sky term under cover** (the backlog's "The sun's shadow near under the calm defaults").
- **A Windows run** (owner, 2026-11-15).
- **A quiet re-read of the gate tier** (the backlog, "Plan 0100's leftovers").

## Implementation log

> Written by `dev`: one row per phase as its commit lands, then the close block. **The phases
> above are the contract. This section records what happened.** Observations, never conclusions:
> no pass list, no self-assessment. Deviations and unmet done-whens are **always** disclosed.
> Keep it shorter than `## Implementation phases`. Readings, gate counts, attempts, sweeps and
> resume notes go to the plan's readings log, `docs/plans/logs/0104.md` (started from
> `templates/readings-log.md`), and this section links it once it exists.

| phase | owner | state | commit |
|---|---|---|---|
| 0: the wobble rigs read a baked store | dev | done: the shared settle (`support/baked.rs`); both soak tests pass on a whole census on the RTX and lavapipe (the forest one 12 538 s on lavapipe); W1 unexplained; readings in [`logs/0104.md`](logs/0104.md) | 565bdee2, 0bc94eb4, e42f5e8a |
| 1: the owner's move as clips | dev | done: six files under `live/anchor/`, readings in [`logs/0104.md`](logs/0104.md) | 31d18c4e |
| 2: the owner watches the clips | human | done: T, C, S anchored; T and C orders confirmed blind; answers in [`logs/0104.md`](logs/0104.md) | f8dd1fac, a183ddd6 |
| 0b: the forest soak test asserts on one baked store ("Amended 2026-11-25") | dev | done: one store, two rest protocols; settle at the submit's cap; half frame on lavapipe; 113 to 146 s RTX, 250 to 255 s lavapipe (410 s loaded); far +30 bars 5.52 / 4.88 (1080p), 8.99 / 8.30 (960x540); readings in [`logs/0104.md`](logs/0104.md) | 127e7672 |
| 3: the owner's own flight, recorded and rendered again (only for a subject not shown) | dev | does not run: every subject anchored ("Amended 2026-11-25"; row marked by the architect) | |
| 4: a measure that ranks what the owner ranked | dev | done: the store's tally (equal with and without lever 2), the W1 split by source (exit (a), (b)), `S`'s change named; step 3: the tracks rendered again with world units (`keys.rs`), both candidates scored on his files (`measure.rs`): candidate 2 holds (a) at `T a` and `S` but fails (b) at T (0.01507 against 0.01515), candidate 1 fails (a) at `C a` and (b) at C (0.04399 against 0.04858); candidate 7: no source 0 sample in any clip, its drop rule fires; `S`'s control: outlines change under the pinned exposure (light, read as a reading); maps and `index.md` under `live/p4/`; readings in [`logs/0104.md`](logs/0104.md) | 13605f9e (wip), 780ec1ec |
| 5: two readings that separate the causes ("Amended 2026-11-26") | dev | done: 5a, held units in the holds: `T b`'s holds 0.00035 to 0.00317 (were 0.00719 to 0.01141), clip totals `T a` 0.01507 against `T b` 0.01123, **holds** by the refutation rule, the under-0.002 figure missed on each hold's first window and both extras; sunlit-leaf row `C a` 0.02480 against `C b` 0.02476; 5b, `S` with no upscaler pinned at 397: outlines 398 / 440 0.007 levels, **the upscaler**; `live/p4/index.md` rewritten; readings in [`logs/0104.md`](logs/0104.md) | committed with this row |
| the stop: the architect's amendment | - | written, with no remedy phase: the owner closed the plan ("Amended 2026-11-26 (owner)"; row marked by the architect) | |
| 6a: Plan 0100's unpicked settings removed | dev | not started | |
| 6b: the rig cut to what stays, the anchors named, the pins | dev | not started | |
| 6c: main merged in, and the tip's gate in parts | dev | not started | |

### Notes

_(Deviations from the plan, with the commit, stated without justification. Done-whens not
satisfiable as stated, and what was done instead. Followups noticed and not acted on. One line
each. Empty is fine.)_

- Phase 1: Phase 0's settle does not exist yet; the clips use their own (a device wait every
  frame, then a census of hit chunks against `BakeQueue::settled`), whole on every clip.
- Phase 1: no canopy start within 30 m holds 10 / 5 / 20 % (800 read); the nearest to holding
  them is used (sunlit ground 5.3 %, leaf 2.4 %, shadowed 68.4 %); its fly is 1.85 s, cut by the
  clearance rule, and no 15-degree turn of the level leg fits 4 s. The slot's fly is 3.51 s, the
  same way. Where no turn fits, the heading stays as it was.
- Phase 1: the level leg's clearance is the Phase 8b rule (no solid within 8 voxels of the eye);
  a heading turn moves the eye along the turned heading and leaves the camera's yaw.
- Phase 1: `libx264rgb` stores `gbrp` (planar RGB) from the `rgb24` input; frame 1 decodes back
  byte-identical on all five clips.
- Phase 0b: the RTX census at the settle's end reads 59/59 (53 coarse), 61/61 (55) and 60/60
  (54) on three runs with lever 2, and 61/61 (55) on one without it, the samples drawn equal
  (346 782 647) on all four: the done-when's "equals Phase 0's 60/60 with 54 coarse" is met on
  one run of three. On lavapipe (960x540) it reads 58/58 (54) on every run with lever 2 and
  59/59 (55) without, equal samples (236 349 415). Lever 2 is kept; the architect rules.
- Phase 0b: the second rest protocol keeps the exposure pinned by the first (read once after
  the settle) and runs its 32 frames at the spawn; the test runs no read frames before the
  settle (at least 120 unread frames instead).
- Phase 5: 5a's scoring ran on the CPU while 5b's render held the lock (5b's load 2.87 to 8.44,
  labelled loaded); "mostly sunlit leaf" is over half of a unit's samples summed over the window.
- Phase 5: the T maps on one scale are new files (`map-T-{a,b}-c2-shared.png`); Phase 4's page
  is kept as `live/p4/index-phase4.md`.
- Phase 1: `play.sh` adds `--input-conf` (`[` half speed, `]` normal, Enter next) and a
  screenshot directory to the plan's `mpv` line.

### Close triggers

_(Facts for the architect. No recommendations, and no suggested version bump.)_

- **What shipped:**
- **Public API changed:**
- **Budget readings:**
- **Gate at the tip:**
- **Outstanding `human` phases:**

## Followups

_(Empty at the draft.)_
