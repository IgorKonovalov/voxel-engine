# Backlog

Ideas, parked questions and owed follow-ups that no plan, ADR or roadmap milestone carries yet.
This is the owner's parking lot: anything said in a session that is "later" lands here, dated,
so it is visible in the tree instead of in an agent's memory.

Rules:

- **One entry per idea**, dated when it was parked, with the context it came from, what it would
  take, and the condition for picking it up. Keep an entry under about ten lines; the long form
  goes in the plan or ADR that eventually takes it.
- **An entry leaves when a plan or ADR takes it.** Replace its body with one line naming the plan
  or ADR, or delete it when the owner drops the idea. The roadmap still owns the milestones; an
  idea that becomes a milestone item moves there.
- **Anyone edits this file**, by hand, in the session the idea comes up. The architect reads it at
  every plan interview and close review and asks which entries are ripe.

## Next up

### Done: test tiers by marks, not by copied lists

Queued 2026-11-08 (owner); top of the backlog since 2026-11-09. Starts as a dev chore once Plan 0059 has merged (owner, 2026-11-09): the rename touches the test files and `.config/nextest.toml` that lane also edits. The WARP lane it first waited on is done; Windows runs the CPU tests only. `.config/nextest.toml` spells the GPU list (15 binaries) and the soak list (about 50 test names) out in every profile, six copies kept in step by `scripts/check-test-tiers.mjs`, because nextest has no filter aliases. The replacement marks the tests themselves:
- **Soak:** `#[ignore = "soak"]` on each long series; the soak tier runs `--run-ignored only`, full runs `--run-ignored all`, and `soak.yml`, the docs and `test-changed.mjs` take the flag.
- **GPU:** every test file that opens a device is named `tests/gpu_*.rs`, and the profiles filter on `binary(/^gpu_/)`. `check-test-tiers` shrinks to the one rule that a file which opens a device carries the prefix.

The profiles then read `not binary(/^gpu_/)` (quick), `binary(/^gpu_/)` (gpu), no filter (gate), and the quick tier's filter again for ci-windows, the CPU tests only. The rename changes the binary IDs, for example `flythrough::golden` becomes `flythrough::gpu_golden`.

**Done 2026-11-10** (GPU half 50b2193, soak half 4a15e60). 25 files renamed to `tests/gpu_*.rs`; the 41 long series are `#[ignore = "soak"]`; the 34 hand-run measurements that were already `#[ignore]`d moved into a `mod reading` per file (owner, 2026-11-10), and soak and full leave out `test(/(^|::)reading::/)`. `check-test-tiers` checks the prefix, that every `#[ignore]` is soak or a reading, and the profiles' forms. Tier membership unchanged in every profile.


### The live queue's order and cancellation, after Plan 0059

Taken 2026-11-10 by [Plan 0060](plans/done/0060-the-live-queue-by-the-view-and-withdrawing-passed-work.md) and [ADR-0048](adrs/0048-the-live-queue-orders-by-the-view-and-withdraws-the-work-a-flight-has-passed.md); the hall's aliasing goes to the antialiasing plan below.


## Engine mechanisms

### Chunk-scale world dynamics: a cellular material step and an event scheduler

Parked 2026-11-03 and 2026-11-04. The owner wants Noita or Teardown style dynamics (sand, flowing
liquids, fire, reactions) and "large world events" that run over many chunks. Today the storage
layer is ready and the simulation layer is absent: 64³ chunks behind per-chunk locks, two-tier
streaming residency, every op returning its touched chunks, the coarse grid updated from that
result, and a server tick that only changes the world when an op is applied. Nothing revisits
chunks over time, there is no active set or dirty-chunk scheduler, and collapse (ADR-0008) is
pull-based and per edit.

Feasibility read (2026-11-03): powders, liquids, fire and material-pair reactions fit as one
deterministic op kind, "step N with seed S", through the op log, over resident chunks only, with
a brick checkerboard for order and a per-chunk dirty bitmap as the active set; cost is set by
active voxels, not resident ones, and the GPU mirror's re-upload of every active brick may
starve before the CPU does. Rigid bodies and rotating pieces conflict with ADR-0008 and stay on
the cosmetic client-side layer. Gas as voxels conflicts with M8's separate coarse volume.

What it takes: an ADR that generalises M10's "flowing liquids" item into the cellular step plus a
per-tick chunk scheduler (active set, per-tick budget, resident chunks only; rejected: GPU-side
authoritative simulation, a rigid-body engine), then a plan with a headless proof (a sand column
released over a pit lands as the same pile with the same hash on both CI arms) and a per-tick
reading against a new `nfr.md` line.

Pick up when the lighting base is settled: Plans 0056, 0057 and 0058 closed.

### Per-source indirect light

Parked 2026-10-22 (Plan 0029 live session 11), reaffirmed 2026-11-04 (Plan 0051 verdict). The
owner wants bounce light only from some sources: "fire on a stick or fire in a dungeon" feed the
probes, the headlamp and simple lights are direct only. ADR-0043 shrinks the probe volume to a
lamp-bounce channel and keeps its logic for exactly this later redesign; ADR-0044 gives lights a
fixed list. The missing piece is a per-light and per-emissive-material "feeds the bounce channel"
flag and the channel's redesign. Plan 0058's Phase 3 (redrafted 2026-11-10) builds the per-light
flag; what remains after its close is the per-emissive-material flag and the channel's redesign.
Plan 0032 (redesigned 2026-11-10) ships no sunlight bounce: sky light is a sky ray, so a cave
mouth gets no bounced sun until this entry is taken. Plan 0037's night light likewise has no
bounce and no probe feed.

### More than eight lights: a world-space light grid

Parked 2026-11-10 (owner asked for the idea in detail while Plans 0058 and 0062 were drafted). ADR-0044 shades eight list lights, each pixel looping over all of them. Plan 0062 allows 64 entities, and each glowing creature or projectile that lights the walls takes a slot, so the cap of eight is the first limit a real scene meets.
- **The mechanism:** a grid of cells around the camera (a guess: 16-voxel cells, 64 a side) holding, per cell, a bit mask of the lights whose reach touches it. A pixel, a shadow ray's origin or a probe ray's hit reads its cell and shades only those lights, still exactly, one shadow ray each. The grid is in world space, not screen tiles, because the bounce channel and the upscalers' change mask need it at points that are not pixels.
- **What it buys:** 64 or more lights in the world for the cost of the few that overlap at each point. **What it does not buy:** many lights overlapping one spot still cost one shadow ray each; a per-pixel cap past that is a pick, and a random pick is noise (ADR-0044 B, rejected).
- **The cheaper first step:** keep eight slots and let the engine choose the eight that matter most to the view each frame, with hysteresis. No shader change; lights can pop when the choice changes.
- **Pick up when:** Plan 0058's readings give the cost per light in reach, and a scene with entities wants more than eight. It needs an ADR that amends ADR-0044's cap.

### Per-face light cells as a near-camera enhancement over the brick level

Parked 2026-11-08 at Plan 0059's Phase 7b pick ("Bricks everywhere": 8-voxel cell, 4096 samples a direction). The owner
asked whether per-face cells should stay as a graceful enhancement: brick light by default, with face cells fading in
near the camera. The answer for now is no.
- At Phase 7b, 512-sample face cells did not look better than 8/4096 bricks; the 64- and 512-sample bricks were "too
  noisy".
- A near ring of about 1.9 x 10^5 faces at 512 adds about 1.3 s to a fresh start and roughly doubles a dig's re-bake.
- The frame headroom is kept for dynamic lights (Plan 0058).

Keeping the option open is cheap. The store puts a brick's record before its face run, and the read prefers a face cell
when one exists. Plan 0059 Phase 6 (7c105c5) has the whole face path working; Phase 14 deletes it. Pick up when
close-range light reads too soft: shadow edges smeared over 8 voxels, or light bleeding round thin walls.

### Baked or emissive light on far-field cells

Parked 2026-11-10 (plan review). Plan 0033 gives a far-field hit no store value, so distant
emitters do not light far terrain. A coarse store block per clip brick is ADR-0028's
Alternative F, picked up only if a reading or the owner's flight asks.

### Remove `own`, the engine's own upscaler

Parked 2026-11-10 (owner, plan review). FSR 2 at 1.5 is the default (Plan 0056) and `own` is
opt-in. Removing it deletes `Upscaler`'s pass, `upscale.wgsl`, the `own` arms of `gpu_upscale.rs`
and its goldens, and re-bases the frame ledger's `own` line. That is a behaviour change, so it is
its own small plan, not Plan 0045's, which only moves the code it finds. Accepted ADR-0024 and
ADR-0034 still name Plan 0045 for it; the removal plan's ADR note corrects that. Draft after
Plan 0061's picks; it runs after Plan 0069 (owner, 2026-11-11) and before M10.
Noted 2026-11-10 (static performance review): `own` is the only upscaler on record under or near
section 2's 1.5 ms upscale line (0.93 to 1.3 ms at the owner's window against FSR 2's 2.9 ms) and
reads 6 to 7 ms less on the Radeon. Removing it before the profile plan arbitrates removes the
slow adapter's cheaper arm; the removal plan should say what the portable profile uses instead.
Answered 2026-11-11 (owner, Plan 0069's answer 2): Portable uses FSR 2 at 720p output, and the
removal plan goes ahead after Plan 0069.

### The lamp channel's faults

Parked 2026-11-05 at Plan 0057's close. `the_lamp_furnace_reads_its_albedo_times_the_direct_mean` takes
its `D` from the shader's output, so it does not defend the albedo: `D` wants an independent reading
of the lamp's irradiance at the rays' hits. Inside probes are traced while the lamp moves
(`probe_update.wgsl`'s `probe_idle` returns on `MOVED_BIT` before its inside check): cost only, unread on
the RTX. `a_lamp_step_moves_the_probes_in_reach_only` checks one probe. Plan 0058's Phase 3 (redrafted 2026-11-10) takes the furnace's `D` and the one-probe step test; the inside-probe trace is a reading there.

### Far light on a slow adapter: a bake budget per second, and no black for missing light

Read 2026-11-11 in Plan 0060's Phase 4 window (`docs/plans/logs/0060.md`): the far caves stay coarse on the RTX and black on the Radeon, because the bake's slice is per frame and a brick with no light value renders black. Taken by [Plan 0067](plans/0067-the-frames-cpu-a-cheap-queue-step-a-bake-budget-by-time-and-no-black-for-missing-light.md) (approved 2026-11-11) and ADR-0052 and ADR-0053.

### A dug chunk's settled light differs from a one-shot bake

Read 2026-11-10 in Plan 0060 Phase 3 (`docs/plans/logs/0060.md`, `a_dig_is_never_withdrawn`). In that test's rig, after the dig settles, chunk [2, 0, 0] differs from a one-shot bake of the edited world. Chunks 0, 1 and 3 to 7 match, and so does the dug chunk the test now compares. The difference reads the same under the score order with withdrawal on, under `distance` with withdrawal off, and at the default `edit_ms` (the lane's `live/p3-digexp.log` and `p3-digexp2.log`, untracked). So it is not the withdrawal and not the order. It predates Plan 0060 by that reading, but it was not read at the base. A guess, not isolated: a neighbour chunk whose light reads the dug chunk is not re-baked after the dig, or is re-baked before the dig's own job lands. What it takes: a headless test that digs, settles and compares every chunk with a one-shot bake, read at main first, then a fix in the live queue's dirty-neighbour rule. Added at the owner's call (2026-11-11).

### The near shell's cost in a flight, and the streaming edge

Parked 2026-11-10 at Plan 0060's Phase 1 stop rules, owed to the backlog at its close (owner's amendment). In the streamed cave at `--glide 300` the near shell (Chebyshev 2, baked at the full count) takes 98 to 99 % of the slice, and about 0.70 of the in-view chunks still waiting have their reach not ready (the mirror's edge), so neither the order nor the withdrawal can light them. What it would take: a shell baked coarse first in a flight, or a smaller shell at speed; and a streamer that loads the light radius ahead of the view. Pick up with Plan 0067's bake budget by time, which reads the same flight (`docs/plans/logs/0060.md`, Phase 1).

### The hall's headless render went dark: `showcase_hall_golden_is_lit_by_its_four_ores`

Read 2026-11-11 at Plan 0060's close review: the soak test failed with 98.6927 % of pixels over 2/255, the same on the base a8b1a2c. Re-blessed 2026-11-11 at Plan 0061's close (fix pass 6287e88): the new golden pins today's render, which Plan 0061's defaults do not move (the library's read and weights give the same pixels), but it is not a correct hall. Mean sRGB luma fell from 11.97 (Plan 0057's image, 2de29c6) to 2.73; the walls are black and only the ores' faces are lit; ember covers 0 pixels. The test still fails its hue assert (`[336, 0, 209, 121]`) and would fail its luma band (11..40). The owner's window at Plan 0061 saw a lit hall, so the likely cause is the headless rig (`gpu_golden.rs`'s `showcase_hall`: its bake or settle state under the baked light since Plan 0059), not the look; unverified. What it takes: bisect the rig's render between 2de29c6 and today, fix the rig, re-bless after a look, and keep the hue and luma asserts as they are. The weekly soak fails on it until then.

### Automatic extremity bounds from declarative noise layers

Parked 2026-11-12 (owner, from the video "500 million voxels/sec - Voxel engine optimizations", youtube.com/watch?v=ru_oz09Zo-s). Today world generation is the consumer's: `WorldSource` is a trait, and the flythrough's cave (`apps/flythrough/src/cave.rs`) already aligns its noise lattices with bricks (one corner set per brick) and proves a brick uniform from its eight corners. The idea the engine lacks: when terrain is described as data (named noise layers, each with a range, mapping, scale and upsample factor), the generator derives each column's or brick's guaranteed solid-below and air-above bounds from the layer config itself, so a new biome skips noise evaluation outside its band with no hand-written proof; layers whose range is unbounded (domain warps) switch the bound off. Same video, also not taken: noise shared by name across biomes and computed once per position. Rejected for storage: RLE columns (slow random access and edits, which collision, acoustics and runtime digging need). Pick up only if the engine ever ships a configurable terrain generator; the requirement would arrive through `project-context.md`.

### Ambient occlusion driving a per-material colour ramp

Parked 2026-11-12 (owner, from the video "the voxel rendering tech nobody talks about", youtube.com/watch?v=1gxcNUUc37Y). Instead of darkening albedo by AO, AO picks a colour along a per-material ramp: snow white to cyan to deep blue in crevices, sand yellow to red, grass bright to deep green. The video's form: per material `light`/`mid`/`dark` colours plus `t_mid`/`t_high` thresholds, `mix(mix(dark, mid, smoothstep(0, t_mid, ao)), bright, smoothstep(t_high, 1, ao))`; AO is per-corner neighbour occupancy shaped by `pow(ao, 1.5)`; fixed face weights (up 1.0, down 0.1, x 0.5, z 0.9) and side faces capped at 0.7 of the ramp; each colour hue-rotated by low simplex noise over world xz for variety.
- **Our gap:** `vxe-render` has no AO term; ambient comes from the baked brick light (Plan 0059). `Material` carries one `albedo`.
- **The cheap input:** at a ray hit the voxel and face are known, so the face's 8 neighbours give corner AO, interpolated at the hit point. Analytic, no noise, no history, so it keeps the image stable.
- **Open look question:** drive the ramp by local corner AO, by the baked sky or ambient light, or both, so a cave reads tinted rather than only dark. Decide by eye in the window.
- **What it takes:** a look spike (corner AO at the hit, an optional ramp beside `albedo`, the hue jitter) with an owner window; an ADR if the material row changes shape.

Pick up after the current lighting plans close (0058 and those ahead of it).

## Performance: the static review of 2026-11-10

Parked 2026-11-10. The owner asked why the window reads 30 to 50 fps with little on screen; a
read-only review of the tree at `50b637a` followed, with no run (the owner asked for none), so
every figure below is an estimate. The window's pacing, the readbacks, the workers and the
sync's time went to [Plan 0064](plans/done/0064-the-windows-frame-pacing-workers-and-readbacks-that-wait.md);
the baked light's shader waste and the queue's scan to
[Plan 0065](plans/done/0065-the-baked-lights-waste-atomics-a-filter-of-zeros-and-a-scan-per-frame.md)
(both approved 2026-11-10). What neither takes:

### Budget lines over with no owner

- **Upscale, 1.5 ms:** FSR 2 reads 2.1 to 2.4 ms on the RTX and 15.7 to 16.1 ms on the Radeon
  at 2530 x 1378 after Plan 0065 (closed 2026-11-11); every plan "reads its delta and does not
  claim the line". Either the line moves or a profile owns it: the render-profile plan's call.
- **Denoise, 3.0 ms, on the Radeon with the face weights:** at the vein the a-trous pass reads
  3.19 ms with Plan 0061's default face weights against 2.70 ms without (RADV RENOIR, 1920 x
  1080, loaded; `docs/plans/logs/0061.md`, Phase 2 done-when 6). Routed at Plan 0061's close to
  Plan 0067 (the frame's cost) and Plan 0069 (a per-profile default for the weights is its
  question).
- **The bake's slice and the queue's step have no line.** 1.5 ms of GPU a frame while the queue
  works (max 10.8 ms on record) and 1.0 ms of CPU sit outside section 2's split. Give them a
  line at the next `docs/nfr.md` revision. Plan 0060's close left it: the step reads one of two
  states (about 1 and 9 ms p50), and Plan 0067 pins the state and reads the step.
- **Streaming CPU p50, 1.0 ms:** 3.4 to 4.2 ms on record. Plan 0064 (closed 2026-11-11) left
  it where it was: its sync time budget cost residency and stays off, and the upload's copy
  into mapped memory stays (see "Slot words packed on the workers").
- **The frame ledger's split**: its totals were re-based at Plan 0065's close (2026-11-11,
  `docs/nfr/02-frame-budget.md`, "Frame ledger"), but its scenario tables still read `own`, the
  table light and the probes, with no row for the list lights or the bake's slice. Rewrite them
  with the render-profile plan, which sets the lines.
- **Time to first frame, 3 s:** 7137 ms at the vein under the baked light; Plan 0060 bars only
  its delta.

Pick up: at the render-profile plan's interview, or at the next close that touches the line.
Plan 0069 (approved 2026-11-11, its answer 4) proposes the upscale, bake-slice and split lines
under its "Budgets", sets them at its close from the readings and rewrites the ledger then.

### The a-trous under the baked light: fewer iterations, or none

Plan 0065 (closed 2026-11-11) skips the filter only with the lamp off, where its input is zero;
with the lamp on it filters as before. There its input is an analytic light with a binary shadow, no Monte Carlo noise; four iterations may be one or two,
and with the lamp off and the `face` look the whole pass could be skipped on the CPU side. Both
change shadow softness or the rare fallback pixels, so they are the owner's eye. Pick up after
Plan 0058, which widens the filter to rgb.

### A beam prepass, or a distance value per brick

The primary walk skips by occupancy only: chunk, 2 x 2 x 2 brick groups, uniform bricks, the
in-brick mask. There is no coarse depth prepass and no distance field. A 1/8-resolution beam
pass over the chunk, group and brick levels would start each ray near its hit; a 3-bit
Chebyshev distance per brick, mirror-side only, would lengthen the fixed 16-voxel group skip.
Both pay in halls and outdoors and little in veins. Read the steps view's histogram first; pick
up before Plan 0033's far field, whose rays are the long ones.

### Narrower frame targets and fewer stores

`hit_pos` is `rgba32float` and every screen pass taps it; a ray length in `r32float` would cut
that traffic but moves every golden. FSR 2's colour input is `rgba32float` though clamped inside
half-float range. The hit pass writes the translucent layer and transmittance on every pixel,
translucent or not. FSR 2's input pass could fold into the composite. Each is 0.1 ms or so on
the RTX, more on the Radeon where bandwidth is the limit. Pick up with the slow adapter's
walk-speed spike below.

### Uniform branches as pipeline constants

`FLAG_SKIPS`, `FLAG_MASKS`, `FLAG_CUTAWAY`, `FLAG_FIELD_SPLIT`, `coarse_span`, `forced_level`,
the view-mode chains in the composite and `LIGHTING_GRADIENT` are tested per step or per pixel
from a uniform and are fixed for a pipeline's life in production. `override` constants would
let the compiler drop them. Small each; fits Plan 0044's Phase 5 or the shader toolchain spike.

### Slot words packed on the workers

The mirror packs and copies a chunk's bricks on the render thread; `docs/roadmap.md` names an
"upload-copy ADR" that was never drafted. The workers already prepare face masks. Moving the
packing there needs the mirror's slot format across a thread boundary, so it is an ADR and a
plan. Read 2026-11-11 at Plan 0064's close: its time budget (`--sync-ms`, off by default) does
leave residency short, p95 2.7 to 6.9 s at 2 to 8 ms against 0.30 s without one, at about
0.3 ms of render-thread time a chunk (RTX, `docs/plans/logs/0064.md`, Phase 4b). A budget
pays only once a chunk's upload is cheaper; this entry is that cheaper upload, and the
budget's mechanism and flag are in the tree for it. Pick up after Plan 0067, with the
upload-copy ADR.
Drafted 2026-11-12 (the architecture review) as [Plan 0070](plans/0070-the-sync-off-the-frame-slot-words-packed-on-the-workers.md)
and ADR-0056, awaiting the owner's go; this entry leaves when the plan is approved.

### The slow adapter's walk-speed spike

Owed since Plan 0054 (ADR-0034 and ADR-0041 name it) and in no plan or backlog entry until now:
why the Radeon's hit pass reads 7 to 8 times the RTX's when the light pass under the store reads
about twice. Run with the shader toolchain spike, when no lane holds the GPU.
Noted 2026-11-12 (the architecture review): the slot is Plan 0069's Phase 4, which reads both
adapters anyway; the review's amendment note asks it to read the hit pass alone at Portable's
internal size on the RENOIR and to run this spike before Phase 6 if the hit reads over 10 ms there.

### The dev box presents across two GPUs

The panel hangs off the AMD iGPU, Hyprland composites there, and the RTX has no connected
output, so a windowed frame crosses GPUs before it shows. Windowed readings should say so, and
`references/project-context.md`'s machine facts should carry it. An external display on the
RTX's port would give a windowed reading without the crossing.

### Costs that multiply across the approved plans

Plans 0058, 0032, 0034 and 0062 each add shadow rays per lighting pixel that the others
multiply (lights by entities, a sky ray and a sun ray per lit pixel, liquid reflections), and
no plan budgets the product. Plans 0058, 0061 and 0062 carry dated review notes for their own
part. Plans 0035, 0036 and 0037 project more on the RTX than their floor shares allow by their
own figures. The render-profile plan is where the product gets a cap.

### The window's frame rate out of focus

Parked 2026-11-11 at Plan 0064's close (the owner's words in its window): the frame rate moves a
lot with the window's workspace in or out of focus. Expected for `--pace compositor`, since
Hyprland throttles frame callbacks for a hidden window; whether it also moves `--pace free`,
the default, is not isolated, and Plan 0064's discarded session (`live/session64b.log`) was
spoiled by it. What it takes: one windowed pair per pacing, focused and on another workspace,
read with the `frames:` line's wall time. Pick up at the next windowed perf session (Plan
0067's window), or if the owner reports the game slowing when tabbed back.

### The frame path's GPU waits in the gate

Parked 2026-11-11 at Plan 0064's close review. `a_shown_hud_and_the_queue_line_wait_on_the_gpu_nowhere`
(`apps/flythrough/tests/gpu_frame_waits.rs`, 300 frames, 12.3 s on lavapipe) is soak-marked,
so the fly-through's wiring (the frame calls the ring's reads, not the waiting ones) runs in
no gate; the gate holds only the ring's own test. A variant of about 120 frames with the
queue line due at least once, or a CPU test on the call sites, would bring it under the 10 s
line. Pick up as a dev chore with the next plan that touches the frame's read-backs.

## Tooling and process

### A pipeline cache for the renderer's shaders

Parked 2026-11-12 (the architecture review, section 1.4). Every pipeline creates its own shader
module and 13 of them prepend the 2496-line `march.wgsl`; no descriptor passes a
`wgpu::PipelineCache` (`cache: None` everywhere). Cold creation of the eleven pipelines read
1396 ms on the RTX, 458 ms on RADV, 299 ms on lavapipe (Plan 0046). wgpu 30's `PipelineCache`
is Vulkan-only and keyed by the driver, so it helps a second start on the same machine, not the
first; the driver's own disk cache already does part of that. What it takes: `pipeline_times`
read with and without a cache file across two starts, on both adapters, after Plan 0045's
Phase 4 has split the shaders (which changes what is compiled). Pick up at 0045's close.

### Comment, manifest and index drift found by the 2026-11-12 review

Parked 2026-11-12; a dev chore of one commit, no plan. `apps/flythrough/examples/world_budget.rs`
describes ADR-0011's attribute pages and planes (26 "contour" lines; :35, :41, :656), removed in
Plan 0054. `apps/flythrough/tests/gpu_lights.rs:610` "the emissive table's path" (deleted in
Plan 0059), `:614` and `:687` "Reservoir reuse's bar", `crates/vxe-render/src/exposure.rs:21`
"reservoirs", `apps/flythrough/tests/gpu_upscale.rs:635` "reservoir textures" (Plan 0057).
`crates/vxe-world/Cargo.toml` and `crates/vxe-render/Cargo.toml` carry
`description = "placeholder; see docs/roadmap.md"`, and no crate sets `publish = false`.
`vxe-render/tests/gpu_frame.rs:86` calls `lamp_bounce` off "the default", which is the
library's default and not the fly-through's (Plan 0069's amendment note takes the default
itself). `CLAUDE.md:39-40` gives `vxe-render` "voxel-model entities" it does not have yet
(Plan 0062) and omits the bake store and FSR 2: the owner's edit. Pick up with the next chore
lane; it touches no file a plan lane edits except `gpu_lights.rs` (Plan 0058's).

### Amendment notes from the 2026-11-12 review, awaiting the owner

Parked 2026-11-12. The review (`docs/reviews/2026-11-12-architecture-review.md`) proposes
amendments to approved plans that only the owner accepts; none was written into a plan. In
brief: **Plan 0067**, do not stop at the sync stop rule, Plan 0070 takes the sync (section 1.1);
**Plan 0044**, Phase 4 struck into 0070 (1.1); **Plan 0069**, a `Delete` class for the fields
with no setter and no caller, `Floor` equal to the fly-through's defaults and every `Default`
equal to `Floor`, the setter gate over the 22 existing setters, and the RENOIR hit pass read
alone in Phase 4 (1.2, 2.2); **Plan 0045**, the reservoir code is live, `hud.rs` and
`showcase.rs` named unconditionally, `table()` and `now()` in Phase 1, `gpu_strip.rs` out of
`tests/`, a pub-item count as Phase 2's done-when (2.1, 2.3). Each leaves when the owner
accepts or refuses it.

### A documentation site

Parked 2026-11-11 (owner: "no plan needed yet, will address this later"). A public site in the
manner of Ritmolux's (Starlight, rustdoc beside it, mermaid built in CI, a Pages workflow that
checks every PR and deploys from main). Two steps: first the existing `docs/` (ADRs, plans,
roadmap, budgets) plus rustdoc, with CI as a link and render guard; a getting-started guide and
a gallery only once the look is picked (after Plans 0067 and 0058) and Plan 0045 has moved the
files. Starlight's npm tree falls outside `cargo deny`, and mdBook is the lighter alternative,
so it takes an ADR. Pick up when the owner asks.

### CI's cold cache after a workspace change

Read 2026-11-11 (owner asked why Windows took 55 min on run 43079051216, 486a53e, against
about 17 on 43011459965 and 42879171018). The rust-cache key changed (`...-e0aa3e4d` to
`...-6cbf2b65`) and the job restored the old entry by its restore-key prefix only. Version bumps
alone did not change the key (43011459965 hit fully after 0.44 to 0.45); the likely trigger is
the new `crates/vxe-entity` manifest and the root members list (inference: the logs show the
manifests rust-cache hashed, not the hash). Although `Cargo.lock`'s dependency set was the same,
nearly every registry crate recompiled: Windows `cargo build` 14.8 min against about 1.3 (252
"Compiling" lines against 48 for the whole fast job), `clippy --workspace` 6 min against 25 s,
the gate 14.5 against 8.6; Ubuntu's build 5.7 min against 0.6. Why a prefix restore rebuilt the
dependencies is not known. The new key's entry is saved, so the next push should hit it fully
(unconfirmed until a run after 486a53e). Options, none taken: (1) accept one slow run per new
workspace member, or warm the cache with a `workflow_dispatch` run; (2) one diagnostic run with
`CARGO_LOG=cargo::core::compiler::fingerprint=info` to read why `proc-macro2` and the like are
dirty after a prefix restore (a rust-cache quirk could be fixed by its version or the key's
inputs); (3) cut the steady state whatever the cache does: drop `cargo build --workspace` if
nextest builds the same targets (check bins and examples first), fold the three clippy steps,
or lint on Ubuntu only. Pick up when a second cold run shows up or the owner wants CI faster.
Options (2) and (3) are [Plan 0072](plans/0072-development-speed-the-build-graph-a-narrowed-gate-and-one-compile-per-ci-profile.md)'s
Phase 5 (drafted 2026-11-12, awaiting go); this entry leaves when it is approved.

### Shader toolchain spike

Parked 2026-10-31. Compare naga's SPIR-V against `spirv-opt -O` on the hit shader (VGPRs,
milliseconds, both adapters), then try wgpu subgroups and `shader-f16` on the walk. Rust versus
C for the CPU side was asked and answered no (GPU about 9 of 10.6 ms, CPU about 1 ms). Run when
no lane holds the GPU.

### A held fallback still moves on screen

Parked 2026-11-05 from the ruling on `the_vein_wall_patches_are_counted`. With the table's read off at the patch pose on 299b5c8, the accumulation is bit-identical over the window (0 pixels changed), yet the filter's output reads 90.95 patches a step and the composite 51.10. The lamp is off and the gather is not added, so the filter is the only part left that moves. By reading the code (not isolated): `temporal.wgsl` keeps blending each frame's noisy sample into the moments after the emissive part holds, and `atrous.wgsl` weights its taps by that variance. `a_still_fallback_pixel_holds_still` reads the accumulation, so it does not see this. Plan 0059 keeps the fallback for no-face hits, so glass and the field look may shimmer at rest. That patch-pose run makes every hit a fallback, so it is an extreme case; a translucent surface has not been read. What it takes: a spike that reads a translucent wall's composite at rest, then a fix that holds the moments with the mean, or passes a held part around the filter as the table's part does. Plan 0059's Phase 2 reads a glass pane at rest and its Phase 3 window looks at it (owner, 2026-11-05); pick up on that answer.

### The gate tier is over its wall-time budget

Read 2026-11-05 on 7cd77ad (`docs/nfr.md` section 7): 1313.4 s, about 22 minutes, loaded, against the 12 min proposal, with the two tests then over the 30 s line already on the soak list. `flythrough::settle` is the largest binary at 338 s. Ruled 2026-11-05 on the four tests over the line in that run: `the_tone_pass_reproduces_the_composites_bytes_on_every_golden_pose` split (the every-pose test on the soak list; the primary pose alone stays in the gate at 9.2 s), `castle_gate_golden` went back under with its warm-up removed (25.7 s cold, 7.4 s warm, alone), and `smoothed_normals_cut_the_normal_error_and_move_no_hit` and `a_face_hidden_from_every_eye_is_settled_before_it_is_seen` stay (23.9 s and 24.1 s alone). Those two move only if a gate run started on a quiet machine reads them over the line. Moving all four would save at most about 95 s of serial time, so this is not how the budget gets met. Still owed: either cut the slowest `settle` and `golden` tests or re-set the proposal. Much of `settle`'s time goes with the table at Plan 0059's deletion, so re-read then. `the_lamp_toggle_opens_the_light_window` read 98.0 s on CI's last red run, on an older tree, and reads 18.0 s here, so it stays in the gate; CI's next run shows whether it is still over the 60 s line. Pick up as a dev chore when the owner next wants the gate faster or CI green. Read 2026-11-12 (the architecture review, on `live/plan-0068/merge-gate.log`): 809 tests, 610 s; the 177 lavapipe tests sum to 610 s serial, the whole wall, the 632 CPU tests to 273 s beside them, so the lever is the lavapipe group's thread count or the GPU tests' pixels, not the CPU tests; [Plan 0072](plans/0072-development-speed-the-build-graph-a-narrowed-gate-and-one-compile-per-ci-profile.md) Phase 4 measures the group at 2 and 3 (drafted, awaiting go). Read 2026-11-09 at Plan 0059's close (`docs/nfr.md` section 7): 850.2 s with `settle` back in the gate and none over the 30 s line, still over the 12 min proposal; the re-read of the proposal stays owed. Read 2026-11-08 on CI's Linux arm, Mesa 26.2.6, the GPU tests in parallel: the same tests move 25 to 40 % between runs (`a_face_hidden_from_every_eye_is_settled_before_it_is_seen` 55.6 s on run 42470428595 and 77.6 s on 42477325818; `the_lamp_toggle_opens_the_light_window` 53.2 then 70.5; `fsr2_adds_little_flicker_on_a_still_vein` 49.8 then 62.8), with several more between 45 and 58 s, so moving tests one push at a time does not converge. Ruled 2026-11-08 (owner): CI's budget step warns per test over 60 s instead of failing the push, until the light work settles; a test that stays over moves to the soak list by hand. Still owed: the architect re-reads `docs/nfr.md` section 7's 60 s line (it says a test over the line moves to the soak list; CI no longer enforces it), at Plan 0059's deletion or when the owner next wants the gate faster. Also ruled 2026-11-08 (owner), from run 42477325818 (2972 s summed over 727 gate tests, 27 tests at 30 s or more holding 41 % of it): the whole `flythrough::settle` binary (34 tests, 746 s) moves to the soak list until Plan 0059's deletion phase decides the table, and CI's Windows arm runs every CPU test but only a GPU smoke set (the `ci-windows` profile: the shutdown and walk binaries and one golden), since WARP skips the image bars and runs a test 10 to 20 times slower than lavapipe. Both are tier moves, not deletions.

### `vxe-render::exposure` hangs on WARP since Plan 0057 Phase 7

Read 2026-11-06 from CI run 42062512872 on 7914635. Windows' gate step ran over 2 h until the owner cancelled it. The previous run (41886647253, 9dbb933, before Phase 7) took 28 to 81 s for each of these tests on WARP. On 7914635: `auto_exposure_finds_the_key` 45 s (2 frames), `limits_compensation_and_dt` 94 s, `light_adaptation_is_fast` 120 s, `dark_adaptation_runs_in_two_stages` 412 s (602 frames), `a_flash_leaves_the_eye_glared` 4001 s. Still running at the cancel, with their elapsed times: `a_wall_split_in_half_meters_the_mean_of_its_halves` about 11 800 s, `history_survives_an_exposure_step` about 11 760 s, `no_target_overflows_at_the_glare` about 11 160 s, and `the_readout_arrives_within_four_frames_without_waiting` about 7 800 s. On local lavapipe at 7914635 all 11 take 3.8 to 7.2 s, as on CI's Ubuntu before. A guess, not isolated: a four-frame test stuck for hours is a GPU wait that never returns on WARP (`device.poll(wait_indefinitely)` or a blocking readback), and the rest pay a WARP-only cost that grows with the frame count, about 0.6 s a frame extra. The suspect is the full 201 MB table at `DEFAULT_TABLE_CELLS`, which WARP's binding limit allows while lavapipe's 128 MiB halves it. Ruled 2026-11-06 (owner): the ci profile kills a test at 600 s, so Windows fails red by name instead of hanging, and stays red until a dev chore profiles Phase 7's passes on WARP through CI diagnostic runs, after Plan 0059 Phase 3. The same run (42222147229) read it again: `flythrough::shutdown the_headless_flythrough_exits_through_every_teardown_stage_within_bound`, a 3-frame headless run at 64 x 36, was killed by its own 120 s bound with an empty stderr. Plan 0059's Phase 3 is past, so the chore is unblocked; the owner started it on 2026-11-08 in the same lane as the Mesa fix. Read 2026-11-08 in that lane: the empty stderr does not place the hang. The test discarded the run's stdout, where the adapter, `device ready`, the mirror, `near residency` and `first frame` lines go, and the fly-through logs nothing at info before its teardown, so a run stuck anywhere from the adapter request to frame 2 leaves the same empty stderr. `a_mirror_that_cannot_sync_exits_1_after_the_teardown` passed in 17.8 s in that run, so on WARP the adapter, the device and the frame's creation return. The lane's diagnostics, for one push: the test now prints the run's stdout with its stderr on a kill; the headless fly-through logs timestamped `diag:` lines through start-up (adapter with its binding limit, device, frame with its table cells and bytes, each streaming sync, init) and each frame (start, submitted, the GPU wait and its return); and a Windows CI step runs the debug fly-through on WARP six times, killed past 180 s each: as the test runs it, 35 frames (the median GPU time per pass), 35 frames with a 2^16-cell table (`VXE_DIAG_TABLE_CELLS`), with `--preload off`, with `--cutaway on` (no fill or preload), and on the fixed `--level`. Locally on lavapipe the 3-frame run reaches `init done` at 5.4 s and exits at 7.9 s (loaded, load 5.6), and a 35-frame run with the small table reads the per-pass line (light 81.4 ms of a 112.7 ms GPU frame at 64 x 36), so each variant prints what the push needs.

Read 2026-11-08 from CI run 42470428595 on 22970c8, the diagnostic step: WARP (`Microsoft Basic Render Driver`, Dx12, static DXC, driver 10.0.26100.33648), the debug fly-through at 64 x 36, each run killed past 180 s. In every run the device is ready at 0.2 to 0.5 s (6.6 s for `--level`) and the frame, its pipelines created, at 13.0 to 15.5 s.

| Run | Outcome | Frame 0 on the GPU |
|---|---|---|
| 3 frames, as the shutdown test runs it | killed at 180 s | submitted at 17.7 s, the wait never returned |
| 35 frames | killed at 180 s | submitted at 17.6 s, never returned |
| 35 frames, a 2^16-cell table | killed at 180 s | submitted at 16.8 s, never returned |
| 35 frames, `--preload off` | exited 0 at 62 s | 17.3 s (17.4 to 34.7 s); each later frame about 0.4 s |
| 35 frames, `--cutaway on` (no fill or preload) | exited 0 at 58 s | 14.1 s (17.3 to 31.4 s) |
| 35 frames, `--level` | killed at 180 s | submitted at 23.1 s, never returned |

The preload-off run's per-pass GPU medians over 30 frames: frame 57.7 ms, hit 13.5, gradient 0.28, light 5.2, fill 0.02, blend 2.1, temporal 0.37, atrous 2.5, probes 22.6, gather 0.58, composite 0.83, the FSR 2 passes 9.6 in all (accumulate 5.7, depth clip 1.5), exposure 0.08; march GPU p50 57.1 ms, p99 81.6 ms. Cutaway's frame is 27.4 ms (hit 13.3, light 0.1, probes 0). The gate step ran 429 of 730 tests before fail-fast cancelled it: the shutdown test failed at 122.5 s, and `vxe-render::exposure a_wall_split_in_half_meters_the_mean_of_its_halves` was killed at 600.2 s, while `auto_exposure_finds_the_key` passed in 65.7 s and `a_flash_leaves_the_eye_glared` in 71.5 s. The exposure tests split the same way as the runs. `BoxRig` sets `candidates: 0`, which turns the preload off (`preload_want` in marcher.rs), and those tests finish. The split wall and `ceiling_frame` (the tests stuck for hours on 7914635) keep the default candidates, so the preload is on, and they never finish. So on WARP, frame 0 never finishes whenever the preload runs, at any table size. Most fly-through GPU tests skip on Windows, since their device is Vulkan-only.

Read 2026-11-08 in the chore-ci-preload lane, with no root cause found. Frame 0's preload work is small: the args pass (one workgroup), the sweep (8 x B = 1024 workgroups of 64 lanes) and `shade_preload` with its light rows only, because the fill and probe lists stay empty until frame 1. So one huge dispatch is not what WARP stalls on. A read of `cache_fill.wgsl`, light.wgsl's `cache_claim`, `cache_find` and `cache_seen`, march.wgsl's walk helpers, naga's HLSL of the preload's entry points, and wgpu 30's DX12 indirect path (the `num_workgroups` root constants after its indirect validation) found:
- no unbounded loop: the one retry loop, `cache_claim`'s compare-exchange, exits in naga's HLSL, which sets `exchanged` to `old == 0`;
- no barrier or `workgroupUniformLoad` behind a branch that is not workgroup-uniform;
- no index out of range;
- no dispatch that is unbounded.

Mesa 26.0.8 does not place the fault either. With `shade_px` removed from `shade_preload`, frame 0 completes and frame 1 dies in `preload_sweep`. With `--preload off` it dies in `shade`. The 26.0.x compiler bug hits any large entry point of this module, not a preload construct, so it is a separate fault from WARP's, which finishes with the preload off. On lavapipe 26.2.5 (LLVM 23, loaded), pipeline creation reads `shade` 148 ms, `shade_preload` 290 ms, `preload_sweep` 58 ms and `preload_args` 16 ms, so no compile blows up there. Still unknown: which of frame 0's three preload dispatches never finishes on WARP, and why. The next step is the owner's call:
- (a) a diagnostic push that names the dispatch. The headless fly-through's frame 0 would submit the args pass, the sweep and the light pass separately, with a GPU wait and a `diag:` line after each. Or variants could skip the sweep, or run `shade` in place of `shade_preload` with the args pass and the sweep kept.
- (b) cap or split the preload's dispatches. The readings above argue against it.
- (c) a WARP-only path. It branches on the backend, so it needs an ADR.
- (d) leave Windows red on these tests in the meantime.

Read 2026-11-08 from CI run 42477325818 on 7e8e15a, option (a): the headless fly-through on WARP, 3 frames at 64 x 36, frame 0's preload and light dispatches each submitted alone with a GPU wait after each, killed past 180 s.

| Run | Outcome | Frame 0 |
|---|---|---|
| split | killed at 180 s | the passes before the preload 9.5 s, the args pass 0.03 s, the sweep 0.9 s, then `shade_preload` never finishes |
| split, sweep skipped | exited 0 at 45 s | `shade_preload` 14.5 s |
| split, `shade` in place of `shade_preload` | exited 0 at 42 s | the sweep, then `shade` 7.7 s (its pipeline built lazily within that time) |
| no split, `shade` | exited 0 at 32 s | |
| `--preload off` | exited 0 at 32 s | |

So the hang needs both the sweep and `shade_preload`. Read the same day in the chore-ci-preload-fix lane on lavapipe (Mesa 26.2.5, LLVM 23.1.3), exclusive perf lock, with a temporary readback of `PreloadStats` and `TableStats` after each headless frame at the same start pose. It rules out work per dispatch as frame 0's cause:
- Frame 0's `shade_preload` runs no fill or probe workgroup. Its input lists are the last frame's outputs, empty on frame 0 by the parity swap; the readback reads `list` 0 and `probes` 0. The sweep visits 128 bricks and scans 0 slabs (every slab stamped done by the brick early-out); it lists nothing and claims nothing (the sweep never claims; the probes do, a frame later). So after the sweep, frame 0's `shade_preload` does the same work as without it: the light rows only. No word the sweep writes on frame 0 (the slab stamps, its counters) is read by `shade_px`.
- The split on lavapipe (load 12.5, loaded): `shade_preload` 17 ms after the sweep, 13 ms with the sweep skipped, `shade` 11 ms after the sweep; within noise.
- From frame 2 on the preload's work is real but spread one sample a lane: the fill gives each drawing entry FILL_LANES lanes of one sample, the probes a lane a sample, so no thread loops over a cell's samples. Over 35 frames: the probes sampled 44 to 4605 faces a frame (8 samples each), the fill drew 2048 cells (the S_PRE cap, 16 samples each) out of an input list that grew to 21 589 entries, 32 656 to 69 592 preload samples a frame against 264 lighting pixels. The median light pass (`shade_preload`) read 73.4 ms of a 99.2 ms GPU frame (load 0.95, quiet); with `--preload off` the light pass reads 1.8 ms of 31.7 ms (load 4.9 to 8.0, loaded). The cap bounds the fill in samples (`draw_cells` x 16) but not the probes (up to PRELOAD_PROBE_LIST_MAX x 8 = 131 072 samples a frame) nor the dispatch's workgroups (one per 4 input entries, drawing or not, up to 8192). That is a cost on every adapter, not WARP's frame-0 hang.

Still unknown: what the sweep's dispatch changes for the next `shade_preload` on WARP, and whether that `shade_preload` is hung or only slow past 155 s. Ruled 2026-11-08 (owner): one more diagnostic push from the same lane, every run split, the meaning of each outcome printed in the step's log: a control with the sweep skipped (180 s kill); (b) `VXE_DIAG_PRELOAD=empty-sweep`, the sweep's dispatch recorded indirect from a buffer of zero workgroups, the dispatch without its writes (180 s); (c) `direct-sweep`, the sweep dispatched directly with the workgroups the args pass would write (180 s); (a) the run as before with a 600 s kill, hung or slow. Worst case 19 min in the step's 25. Locally on lavapipe both new modes exit 0 over 35 frames: `empty-sweep` reads 0 preload samples (the lists never fill), `direct-sweep` 32 656 samples and a 67.5 ms light pass, as the indirect sweep does. The diagnostic code stays in until the cause is placed. The preload's cost from frame 2 on is a separate finding, parked for Plan 0059 below.

Read 2026-11-08 from CI run 42483023036 on f45523f, the step as ruled above, WARP (driver 10.0.26100.33648), every run split:

| Run | Outcome | Frame 0 |
|---|---|---|
| control, sweep skipped | exited 0 at 40 s | the passes before the preload 7.8 s, the args pass 0.03 s, `shade_preload` 11.8 s |
| (b) `empty-sweep` | killed at 180 s | the args pass 0.02 s, the sweep (zero workgroups) 0.7 s, then `shade_preload` never finishes |
| (c) `direct-sweep` | killed at 180 s | the args pass 0.02 s, the sweep (1024 workgroups) 0.7 s, then the same |
| (a) as before, 600 s kill | killed at 600 s | the sweep 0.7 s, then no `shade_preload` line in ten minutes |

So it is a hang, not a slow pass, and recording the sweep's dispatch at all, even of zero workgroups and so without its writes, in an earlier submission waited for, is enough. The sweep's stage reads 0.7 s whatever its work, against the args pass's 0.02 s: a fixed cost per dispatch of that pipeline (WARP compiling it at first use, a guess). Read the same day in the chore-ci-warp-args lane, on paper, wgpu `=30.0.1` (`dx12`, `static-dxc`). Every buffer is bound whole (`as_entire_binding`):

| Dispatch | Group 0 | Group 2 | Group 3 | Dispatched |
|---|---|---|---|---|
| `preload_args` | none | table (3) and counters (4), read-write | the preload uniform (1); `preload_dispatch` (2), read-write | directly, 1 x 1 |
| `preload_sweep` | the world, read-only | table (3) and counters (4), read-write | the preload uniform (1) | indirect from `preload_dispatch` at 0 |
| `shade_preload` | the world, read-only | the hit textures (0, 1), irradiance storage texture (2), table (3) and counters (4) read-write, edits uniform (5) | the preload uniform (1) | indirect from `preload_dispatch` at 12; reads `@builtin(num_workgroups)` |

The suspect the lane started from does not hold: `preload_dispatch`, the only indirect args buffer, is bound read-write by the args pass alone, never by the sweep, and in (b) and (c) the sweep does not even read it. No buffer is read-only in one of these passes and read-write in the other, and no two bindings cover different ranges of one buffer. What the sweep and `shade_preload` share read-write is the table and the counters, which the args pass binds read-write too, so their state between the args pass and `shade_preload` is the same with the sweep as without. On DX12, wgpu 30 gives every pipeline layout three root constants for the special builtins and, with indirect validation on, a command signature that sets them; each indirect dispatch first runs wgpu's validation pass, which copies the three words into one device-wide buffer, and dispatches from that. (c)'s sweep is direct and skips that path, so the sweep's indirect path is not it either. wgpu's issues name nothing that matches. The nearest are #10632 (DX12 drops the UAV barrier between two writes to a write-only or atomic storage texture, WARP included: a hazard inside a submission, which cannot hang submissions waited for one by one) and #8368 (device removals on WARP in wgpu's own tests). So the paper reading is not conclusive, and no fix is prepared. One more diagnostic push from the lane, every run split and killed past 180 s, worst case 15 min of the step's 25 (`VXE_DIAG_PRELOAD` now takes comma-separated switches):
- control: `skip-sweep,read-args`, the words read back where the light pass is recorded, as the reference;
- (1) `read-args`: `preload_dispatch`'s six words and the counters' `pre_fill_groups` and `pre_groups`, read back after the sweep and before `shade_preload` is recorded, as a `diag: frame 0 split: args read back: ...` line. The copy moves the words' buffer to a copy source before the indirect read, so an exit 0 here would itself place the buffer's state;
- (2) `direct-shade`: the same read, then `shade_preload` dispatched directly with the x and y read back;
- (3) `scratch-sweep`: the empty sweep with its group 2 bound to scratch buffers no other pass binds, so it shares no read-write buffer with `shade_preload`;
- (a) the normal split, to show the hang still reproduces in that build.

Locally on lavapipe (Mesa 26.2.5, LLVM 23.1.3), exclusive perf lock, load 1.4 to 6.8 (loaded from the third run on), every mode exits 0 over 3 frames, `shade_preload` 11 to 17 ms after the sweep. Each read reads the sweep's words [1024, 1, 1], `shade_preload`'s [3, 2, 1] (3 columns and the 2 rows of the lighting grid, no preload rows) and 0 fill and 0 preload groups, with the sweep skipped or not. Those are the words WARP should read in (1) if its args pass and the sweep are right.

Read 2026-11-08 from CI run 42489194343 on f23c5b3 (WARP, 3 frames, split, each killed past 180 s):

| Run | Result | Read back |
|---|---|---|
| control: `skip-sweep,read-args` | **killed** after the read-back line | sweep [1024, 1, 1], `shade_preload` [3, 2, 1], 0 fill, 0 preload groups |
| (1) `read-args` | killed after the read-back line | the same words |
| (2) `direct-shade` | killed after the read-back line | the same words |
| (3) `scratch-sweep` | **exits 0 in 59 s**; `shade_preload` 18.9 s after the sweep | none |
| (a) the normal split | killed after "preload sweep done" | none |

- **The words are right.** WARP reads exactly lavapipe's words, so the hang is not a garbage dispatch.
- **The read-back alone hangs it.** The control was the sweep-skipped run that exits 0 in runs 42477325818 and 42483023036, but with the read-back added it was killed. So a separate submission that only copies `preload_dispatch` between the args pass and `shade_preload` is enough, with no sweep at all.
- **(2) says nothing about the indirect path,** because it carried the same read-back.
- **(3) is clean.** With its table and counters on scratch buffers, the empty sweep leaves `shade_preload` running.
- **Reading:** on WARP, any submission between the args pass and `shade_preload` that touches a buffer `shade_preload` then uses (the table and counters bound read-write, or `preload_dispatch` copied) makes `shade_preload` hang. Both the in-frame path and the split hang, so it is not only a cross-submission effect. This is a guess until a debug layer reads it.
- **Next candidates, the owner's pick:** the D3D12 debug layer with GPU-based validation on the WARP runs, which names a bad state or hazard if wgpu emits one; wgpu newer than 30.0.1 on WARP; and (2) without the read-back.

Pushed 2026-11-08 from the chore-ci-warp-debug lane, the owner's pick: the debug layer on the WARP runs. The fly-through's headless instance already reads wgpu's `WGPU_VALIDATION` and `WGPU_GPU_BASED_VALIDATION` (`with_env`). The debug build has asked for `VALIDATION` all along, but wgpu-hal skips the layer without a word when the Graphics Tools feature (`d3d12SDKLayers.dll`) is missing, and no D3D12 line ever reached the earlier logs, so the layer probably never loaded. The step now checks for the DLL, installs the feature with DISM if it is missing (killed past 480 s, not fatal), and skips the runs if it is still absent. The layer's output is logged at trace and printed deduplicated, with the `diag:` lines and a new `diag: instance flags` line. Three runs, every one split: (a) the normal split with the layer and GBV (180 s); (b) `scratch-sweep` with the same, which should exit 0 and so gives the baseline lines (300 s); (c) the normal split with the layer and no GBV (180 s). The read-args and direct-shade runs are dropped. Worst case 19 min of the step's 25. Locally on lavapipe all three exit 0 in 10 to 23 s (load 27, loaded), and with no Vulkan validation layer installed they print no layer lines.

Read 2026-11-09 from CI run 42573270474 on e5d6646 (WARP, driver 10.0.26100.33648). The image already has `d3d12SDKLayers.dll`, so DISM did not run. Each run's `diag: instance flags` line shows the request it made. The gate's shutdown test still failed at 122 s, without the layer.

| Run | Flags | Outcome | Layer lines |
|---|---|---|---|
| (a) normal split, GBV | DEBUG, VALIDATION, GPU_BASED_VALIDATION | crashed at 23 s, 0xC0000005 (access violation), last line `frame 0: start` | none |
| (b) scratch sweep, GBV | the same | crashed at 23 s, 0xC0000374 (heap corruption), last line `frame 0: start` | none |
| (c) normal split, no GBV | DEBUG, VALIDATION | **exited 0 after 62 s**: preload sweep done at 30.7 s, `shade_preload` done at 50.2 s, all 3 frames | none |

- **Reading:** GBV crashes WARP before the first stage of frame 0, including on the scratch baseline that exits 0 without it, so (a) and (b) say nothing about the hang. With the debug layer on and GBV off, the split that hung past 600 s in every earlier run completes, and the layer reports no misuse of states or barriers. The layer's own work at record and submit time clears the hang. That points at a race inside WARP across submissions on buffers `shade_preload` uses, not at a bad state that wgpu records. crates.io's newest wgpu is still 30.0.1, so a newer wgpu is not available to try.
- **Next candidates, the owner's pick:** run the Windows GPU smoke set with `WGPU_VALIDATION=1` (the layer that cleared the hang), which also keeps validation running on Windows; take the GPU smoke set off WARP and keep Windows on CPU tests until the preload's fate in Plan 0059; or a backend-neutral change that keeps other submissions off the preload's buffers between its args pass and `shade_preload`, which the scratch sweep showed runs.
- **Ruled 2026-11-09 (owner): no GPU tests on WARP.** The `ci-windows` profile runs the quick tier's filter, the CPU tests only, and the debug-layer step is gone from `ci.yml`. Windows still creates every pipeline on DX12 (the pipeline-times step). The env-gated switches in the code (`VXE_DIAG_PRELOAD`, `VXE_DIAG_SPLIT_FRAME0`, the `diag:` lines) stay until Plan 0059 settles the preload. If the path stays, this entry is how to put a GPU smoke set back on Windows; if it goes, they go with it and this entry closes.
- **Plan 0059 closed 2026-11-09 on "baked":** the preload, its sweep and `shade_preload` are deleted, and with them `VXE_DIAG_PRELOAD` and `VXE_DIAG_SPLIT_FRAME0`; the `diag:` lines stay (`apps/flythrough/src/main.rs`). Next, the owner's call: one push that runs the GPU smoke set on WARP again. Clean, the `diag:` lines go and this entry closes; still hung, the hang was not the preload's.

### llvmpipe SIGSEGV on CI's Ubuntu since Plan 0057 Phase 7

Read 2026-11-06 from the same run. Three gate tests died with SIGSEGV on lavapipe: `flythrough::castle_golden castle_gate_golden` (57.5 s, 20.8 s on the run before), `flythrough::fsr2 a_sideways_step_keeps_fsr2s_history` (20.8 s) and `a_still_camera_has_no_disocclusion` (27.7 s). The driver was the same on both runs: `mesa-vulkan-drivers 26.0.8-1ubuntu0.3`, LLVM 21.1.8, image ubuntu-26.04. Every test in that run that rendered a shaded frame crashed. The GPU tests that passed render none (`frame_bytes_under_fsr2_swap_the_histories_for_fsr2s_inventory`) or render cutaway, where the fill and preload do not run. Memory is not it: each test peaks at 0.73 to 0.92 GB and four at once at 3.7 GB locally, against the runner's 16 GB. It does not reproduce on local lavapipe (Mesa 26.2.5, LLVM 23): the ci profile at four threads passed 729 of 729. A guess: a Mesa 26.0.8 llvmpipe crash on a Phase 7 shader, not concurrency. Read 2026-11-08 from CI run 42222147229 on 7c73b37, the GPU tests one at a time: `castle_gate_golden` still died with SIGSEGV at 60 s on the same driver, so concurrency is ruled out and the driver crash stands; the ci profile's `@global` line is back. The owner chose (2026-11-08) a chore lane: a newer Mesa on the runner first, a local Mesa 26.0.x bisect of the pass if that is not available. Read 2026-11-08 in that lane (Launchpad, the day's indexes): the kisak-mesa PPA publishes `26.2.6~kisak1~r` for resolute (26.04) amd64, with its libdrm `2.4.134-3~kisak~r`, and the runner's archive has 26.0.8-1ubuntu0.3 at most; oibaf's PPA publishes no mesa at all. kisak's resolute build links Ubuntu's `libllvm21`, so it would have moved the runner to Mesa 26.2.6 on the same LLVM 21.1.8, not to the local pair (Mesa 26.2.5, LLVM 23). Ruled 2026-11-08 (owner): no kisak-mesa PPA and no other package source whose version cannot be pinned; the owner does not trust packages that can't be pinned, and the PPA indexes its latest build only. The runner keeps its stock Mesa 26.0.8. The LLVM 21 suspicion is accepted, and the pass is to be found on a local build of Mesa 26.0.8 from the release tarball against LLVM 21. Read 2026-11-08 in that lane, on local builds under `~/build/mesa-26.0.8`: the tarball `mesa-26.0.8.tar.xz` from mesa.freedesktop.org, sha256 caf1c0061a68e88dfa74967a7e780c0e85d65b6c4e334cd69095a5dc54ad78bc as published in its release notes, signature good from Eric Engestrom's key 57551DE15B968F6341C248F68D8E31AFC32428A6; LLVM 21.1.8 from Arch's extra `llvm21` and `llvm21-libs` 21.1.8-1, signatures and sync-db sha256 checked, extracted rather than installed. Lavapipe only, meson 1.9.1 with `-Dbuildtype=debugoptimized -Db_ndebug=true -Dvulkan-drivers=swrast -Dgallium-drivers=llvmpipe -Dplatforms= -Dopengl=false -Dglx=disabled -Degl=disabled -Dgbm=disabled -Dgles1=disabled -Dgles2=disabled -Dllvm=enabled -Dshared-llvm=enabled -Dvideo-codecs= -Dvalgrind=disabled -Dlibunwind=disabled -Dbuild-tests=false`, 804 steps in 53 s at `ninja -j12` on the dev box. It reproduces: `castle_gate_golden` dies with SIGSEGV at 24.6 s (adapter `Mesa 26.0.8 (LLVM 21.1.8)`), and so do `a_sideways_step_keeps_fsr2s_history` and `a_still_camera_has_no_disocclusion`. The core's crashing thread is a llvmpipe compute worker in JIT code with no symbols (`mov (%rbx),%rax` with rbx 0, a null pointer loaded from the shader's stack frame); its registers hold the table's mask 0x1fffff, the last cell's byte offset 0x5ffffd0 and the blend header's dirty word at 0x6000004, so the shader was addressing the per-face table. A local dispatch trace in lavapipe (a patch kept beside the build, not in the repo) names the dispatch: with the preload on, the last dispatch before the crash is the indirect 8 x 8 one, `shade_preload` (cache_fill.wgsl, the light pass with the preload's fill and probes); with `--preload off` it is the plain light pass `shade` (light.wgsl). Both run `shade_px`. Not Phase 7 alone: a bisect of the fly-through's 3-frame headless run on this build passes on 9dbb933 (main, CI's last green) and on 2bf4568 (plan 0057 Phase 4 part 2), and crashes on their merge a337f76 and every later point read (dc2a470, ef8cf7d, ca2b0e9, 4de5c4e, 41cda0c). The merge's only change to march.wgsl is Params' 16-byte tail (`emitter_cap`, `dither`), so the crash follows the shader's build, not a single feature, which is what a compiler bug looks like. Not LLVM 21: the same Mesa 26.0.8 tarball built against Arch's LLVM 23.1.3 crashes the same test, and Mesa 26.2.6 (sha256 b9436b3e1a8be76fbd4823dc884f90596ab7c6ac5e1867a305e66ffccfb31a92, same signer, same options) built against the same LLVM 21.1.8 passes all three tests. The accepted LLVM 21 suspicion is refuted: the crash is Mesa 26.0.x's own, fixed by 26.2. The owner's options, no shader changed: CI builds the pinned 26.2.6 tarball against the runner's LLVM 21 (55 s here at 12 jobs; the runner has 4 cores, and the build could be cached by its hash), or waits for the runner's image to ship Mesa 26.2. Ruled 2026-11-08 (owner): (a). CI's Linux arm builds lavapipe from the 26.2.6 release tarball, checked by its sha256 (no signature check, which would need an unpinned keyserver fetch), against the runner's own `llvm-21-dev`, with the options above and meson 1.9.1, ninja 1.13.0, mako 1.3.10, packaging 25.0 and pyyaml 6.0.3 from PyPI. It caches the install by the hash, the options and the runner image, points `VK_DRIVER_FILES` and `VK_ICD_FILENAMES` at it, and fails the job unless vulkaninfo reports Mesa 26.2.6. The workflow's exact steps, run in an `ubuntu:26.04` container capped at 4 CPUs, built it in 102 s and the loader reported `Mesa 26.2.6 (LLVM 21.1.8)`. The build comes out once the runner image ships Mesa 26.2 or later. Read 2026-11-09 from Soak run 42602069520 on 8092b6c, the first weekly soak: `soak.yml` still used the runner's Mesa 26.0.8, and 67 of its 79 tests died with SIGSEGV. The build moved into a composite action, `.github/actions/lavapipe`, which both workflows call, under the same cache key.

## Later apps

### Web demo as a separate app

Decided 2026-10-16: a WebGPU and wasm demo for blog interactivity is a later, for-fun item and
its own app under `apps/`. The fly-through and the native paths never change or lose performance
for it; any crate support is target-gated. Not before the engine has steady lighting, coloured
materials, a sky and a way to build interesting architecture.

