# Plans index

The one-minute "what's in flight" view. Read this first each session instead of re-deriving
state from `git log`. Completed plans move to `done/`.

**Next free number: 0098** (0096 is the performance plan below; 0097 the exposure's anchor by
the sky around the camera; ADRs are a separate sequence, see `../adrs/README.md` for its next
free number). Plan 0095, the motion wobble (a calm key light in motion), closed 2026-11-21,
v0.69.0.

## Active roster

<!-- roster:begin cap=320 -->
| Plan | Title | Status | Live constraint |
|------|-------|--------|-----------------|
| [0033](0033-far-field-clipmap-to-a-kilometre.md) | The far field: a clipmap of three coarse chunk levels to about 1 km | approved | M10, ADR-0028 (proposed, revised). Re-read 2026-11-17: absent is sky; questions answered. After 0081. |
| [0034](0034-water-surface-waves-reflection-refraction-underwater-fog.md) | Liquid surfaces: analytic waves, traced reflection and refraction, absorption under the surface | approved | M10, ADR-0029 (proposed). Re-read 2026-11-17 (the reflected hit reads 0086's baked sky). After Plan 0033. |
| [0035](0035-aerial-perspective-and-volumetric-clouds-with-cloud-shadows.md) | Aerial perspective over the near and far field, and volumetric clouds that shadow the ground | approved | M10, ADR-0030 (proposed). Re-read 2026-11-17: `E_sky` reads `cloud_sky`; questions answered. After 0034. |
| [0036](0036-weather-rain-snow-wind-wet-surfaces-snow-cover-lightning.md) | Weather: rain, snow, wind, wet surfaces and snow cover, lightning | approved | M10, ADR-0031 (proposed). Re-read 2026-11-17, questions answered. After Plan 0037. |
| [0037](0037-night-sky-stars-moon-phases-sky-bodies-and-aurora.md) | The night sky: stars, the moon's phases, sky bodies, an aurora that lights the ground, a night tint | approved | M10, ADR-0032 (proposed). Re-read 2026-11-17: the night's light through `E_sky` (Phase 7 rewritten). After Plan 0035, before 0036. |
| [0071](0071-the-consumers-path-a-render-example-a-renderer-tested-alone-names-and-errors.md) | The consumer's path: a render example as the contract, a renderer tested alone, one name per thing, errors | approved | Cross-cutting, ADR-0057. From the 2026-11-12 review. After 0045, 0069 and 0044's Phase 8. |
| [0073](0073-a-settings-menu-and-file-for-the-fly-through-and-the-tester.md) | A settings menu and a settings file for the fly-through and the tester | approved | ADR-0058. Apps only. After 0069, 0071 and 0080; amended 2026-11-16 (owner). |
| [0076](0076-a-key-help-overlay-for-the-fly-through-from-the-key-table.md) | A key help overlay for the fly-through: `F1` (readings move to `Enter`), written from the key tables, modes live, 80 x 45 | superseded | Apps only, no ADR. Folded into 0080 (owner, 2026-11-16). |
| [0081](0081-a-surface-detail-layer-blades-caps-and-pebbles-marched-above-styled-faces-swayed-by-wind.md) | A surface detail layer: blades, caps and pebbles marched above styled faces, lit as hits, faded by size on screen | approved | M10, ADR-0063 (proposed). Re-read 2026-11-17. After 0086 and 0080, before 0033. |
| [0082](0082-squash-and-stretch-a-scale-per-part-in-the-pose-and-the-clips.md) | Squash and stretch: a scale per axis per part in the pose and the clips, a slime in the library with a bounce | approved | M6, ADR-0064 (proposed). Approved 2026-11-16. After 0063, the clip chore, 0078 and 0079 P1-3. |
| [0084](0084-staged-structures-stages-from-a-vox-a-stamp-with-a-write-policy-and-a-region-a-reveal-and-a-demolish.md) | Staged structures: stages from a `.vox`'s layers, a stamp with a write policy and a region, a reveal over ticks, a demolish | approved | M7, ADR-0067. Approved 2026-11-16. After Plan 0083 closes. |
| [0085](0085-entity-variants-holding-clips-and-an-entity-stamped-into-the-world.md) | Entity variants (hide, swap, remap) from the `.rig`, a clip that holds, a posed entity stamped into the world | approved | M6, ADR-0068, ADR-0069 (proposed). Approved 2026-11-16. After 0063, the chore, 0078 and 0082. |
| [0087](0087-the-rename-to-svet-crates-env-vars-wit-package-archives-docs-and-the-repository.md) | The rename to `svet`: crates, `SVET_*`, the WIT package, the tester archive, docs, the repository, a minor bump | approved | ADR-0071. Alone, last: after 0086 merges (the rest have). Pre-flight 2026-11-19. |
| [0089](0089-deterministic-simulation-testing-seeded-scenarios-a-simulated-channel-a-mirror-oracle-and-gates.md) | Deterministic simulation testing: seeded scenarios, a shrinker, a simulated channel, a mirror oracle, determinism gates | approved | ADR-0073. After 0087 (owner). Pre-flight 2026-11-19, q7 answered. |
| [0090](0090-the-pop-counter-scenes-on-lavapipe-count-the-pixels-that-break-the-light-rules-a-ratchet.md) | The pop counter: fixed scenes on lavapipe count the pixels that break each light rule, by source; a baseline that only falls | approved | Cross-cutting, ADR-0076 (proposed). After 0086 closes, before 0081. |
| [0091](0091-the-frame-profiler-scopes-across-threads-a-gpu-lane-a-dump-for-agents-and-a-hud-ticker.md) | The frame profiler: scopes across the threads, a GPU lane, a dump the agent ranks, budget scopes, a HUD ticker | approved | M7, ADR-0077. After 0087 and 0089, before 0090 (owner). |
| [0092](0092-wgsl-hot-reload-in-the-fly-through-an-mtime-poll-pipelines-rebuilt-and-the-old-ones-kept-on-an-error.md) | WGSL hot reload in the fly-through: an mtime poll, pipelines rebuilt from disk, the old kept on an error | approved | M7, no ADR. After 0091 in its lane, before 0090 (owner). |
| [0093](0093-the-readme-rewrite-with-reproducible-screenshots-a-shot-manifest-a-shoot-script-and-an-image-gate.md) | The README rewrite with reproducible screenshots: a shot manifest, a shoot script, an image gate, the owner's picks | approved | M7 docs, no ADR. After 0087 merges, beside 0089, before 0091. |
| [0096](0096-the-bake-queues-drain-tiers-a-drain-slice-the-step-and-the-slow-adapters-light-pass.md) | Performance: the bake queue's drain, the step off its depth, the RENOIR light pass, `alloc` and the eviction scan | in progress, amended 11-22 | M10, ADR-0080 (proposed). Phase 9 left. Before 0087. |
| [0097](0097-the-exposures-anchor-by-the-sky-around-the-camera-the-floor-record-under-the-eye-a-time-constant-and-the-canopys-shade.md) | The exposure's anchor by the sky around the camera: the floor record under the eye, a time constant | approved 11-20 | M10, ADR-0081 (proposed). Lane waits on 0095's merge. |
<!-- roster:end -->

## Sequence

The order the active plans run in (owner, 2026-11-10, set at the review of every active plan;
amended 2026-11-11: Plan 0045 moves ahead of Plan 0058 and the rest of Plan 0044).
Each starts on main after the one before it closes, unless its line says otherwise.

1. **Plan 0060**, the live queue: closed 2026-11-11 (v0.45.0). Plan 0061's Phase 1 (readings only) and Plan
   0064's Phases 1 and 2 (the pacing spike and the owner's pick) ran beside it.
   - **Plan 0064**, the window's frame: closed 2026-11-11 (v0.46.0).
   - **Plan 0065**, the baked light's waste: closed 2026-11-11 (v0.47.0).
   - On 2026-11-10 the look-driven plans (0058, 0061, 0032 to 0037) marked their phases
     before the window as exploration: image tests are written after the pick, in each plan's
     last dev phase.
2. **Plan 0061**, antialiasing: closed 2026-11-11 (v0.48.0).
   - **Plan 0067, the frame's CPU**: closed 2026-11-12 (v0.49.0). Approved 2026-11-11 (owner go
     with the defaults on its open points), after 0061 and ahead of 0058 (owner, 2026-11-11, after Plan 0060's and 0064's
     windows: "we are not there - either lagging too much, or the picture is unacceptable";
     "address performance next after this set of plans"). Its Phase 1 is readings only and may
     run beside 0061's Phase 2 once 0064 and 0065 have merged. It takes Plan 0065's Phase 6 (the
     queue's step) and the backlog's per-second bake budget and missing-light fallback (a
     neighbour's or parent's light, owner 2026-11-11; ADR-0052 and ADR-0053 accepted the same
     day); the Radeon's hit and FSR 2 passes (26 and 18.5 ms at the window) stay with
     the profiles plan (step 6); 0065 closed with them over their lines.
3. **Plan 0045**, the structure pass: closed 2026-11-13 (v0.50.0); later plans read old paths
   through its "Where things moved". Moved ahead of 0058 and 0044's render phases (owner
   2026-11-11). It starts once Plans 0061, 0067 and 0068 have merged, while no lane edits the
   files it splits. Its Phase 0 re-measures and the architect confirms the seams before a file
   moves. Plan 0044's render device helper (its Phase 8) now follows it.
4. **Plan 0058**, dynamic lights: closed 2026-11-15 (v0.54.0; ADR-0044 accepted). Plan 0069's
   per-profile light cap is a field at or under its `MAX_LIGHTS` (16).
5. **Plan 0044**, the hot-path pass: closed 2026-11-16 (v0.55.0); Phase 2 ran as Plan 0064's
   Phase 3, Phase 4 as Plan 0070.
6. **Plan 0069, render settings and profiles**: closed 2026-11-17 (v0.60.0; ADR-0055 accepted with
   an Outcome), then the owed plan removing the `own` upscaler (not drafted; owner, 2026-11-11:
   Portable takes FSR 2, so the removal goes ahead).
7. **M6, entities: Plan 0062, then Plan 0063** (owner, 2026-11-10: ahead of M10,
   because the first consumer is a cave game). Plan 0062 closed 2026-11-16 (v0.56.0; ADR-0050
   accepted with an Outcome); Plan 0063 closed 2026-11-16 (v0.57.0; ADR-0051 accepted).
   - **Plan 0068**, the tester build: closed 2026-11-15 (its code shipped in v0.54.0, the first
     tagged release; ADR-0054 accepted).
8. **M10 in plan order: 0032, 0033, 0034, 0035, 0036, 0037.** 0032 builds on Plan 0058's direct
   target, and each later plan relies on 0032's "Interface for Plans 0033 to 0037" section.
   **Reordered 2026-11-16 (owner):** Plan 0069 starts once Plans 0062 and 0063 have merged, and
   Plan 0032 follows it directly, both beside Plans 0080, 0078, 0077 and 0079. Plan 0071, Plan
   0073 and the `own` removal no longer hold M10; they follow 0069 in their own order.
   Plan 0032 closed 2026-11-17 (v0.61.0; ADR-0026 and ADR-0027 accepted).
   **Added 2026-11-17 (architect, from the owner's picks at Plan 0032's window):** Plan 0086
   (the flicker's fixes, ADR-0070) starts when Plan 0032 closes and merges, recommended ahead
   of Plan 0081 (the still image first); the owner's go and the order are owed.
   **Added 2026-11-18 (architect, draft):** Plan 0090 (the pop counter, ADR-0076) takes its
   baseline on the tree with Plan 0086 merged, so it starts when 0086 closes, recommended ahead
   of Plan 0081 so the detail layer is the first plan measured against the ratchet; awaits go.
   **Re-read 2026-11-17 (architect, the owner's "start the re-read"):** Plans 0081, 0033, 0034,
   0035, 0036 and 0037 each carry an "Amended 2026-11-17 (re-read)" section against main
   3b3986d0: the Plan 0045 paths, the menu entries in place of keys (ADR-0061), the settings
   rows and profile picks (ADR-0055), the 10 s soak line, the files at their caps
   (`hud/modes.rs`, `frame/encode.rs`, `gpu_golden.rs`), absent is sky (0033, ADR-0028
   revised), and Plan 0086's baked sky visibility (`E_sky`) in 0033 to 0037. The six owner
   questions were answered 2026-11-17 with the recommended option each: no far sky ray, solid
   far canopies, the cloud shadow after the history, the clouds kept and scaled by profile, snow
   on the layer a followup, and Plan 0037 before Plan 0036. The run order after 0086 and 0080
   merge: 0081, 0033, 0034, 0035, 0037, 0036.

**Plans from the architecture review of 2026-11-12** (`../reviews/2026-11-12-architecture-review.md`),
approved by the owner on 2026-11-12 ("45 after 67, 70 after 45. all draft plans can be
approved"): **Plan 0070** (the sync off the frame), closed 2026-11-13 (v0.52.0), taking
Plan 0044's Phase 4; **Plan 0072** (development speed), closed 2026-11-13 (v0.51.0);
**Plan 0071** (the consumer's path) after Plans 0045, 0069 and 0044's Phase 8 (no longer before M10: owner, 2026-11-16).

**Plan 0073** (a settings menu and file for the fly-through and the tester, approved 2026-11-12 with every recommendation): after Plans 0069 and 0071, and after Plan 0080, whose menu its settings pages live in (amended 2026-11-16, the owner's go).

**Plan 0080** (the fly-through's menu: one overlay component, a palette, the minimal key set; drafted and approved 2026-11-16 with ADR-0061 from the owner's "keep using keys to minimal"; **closed 2026-11-17, v0.62.0**, ADR-0061 accepted with an Outcome): after Plans 0062 and 0075 merge (both edit `toggles.rs`), **before Plans 0076, 0077 and 0073**, which are amended onto its component (notes of 2026-11-16, approved by the owner); it does not wait for Plans 0069 or 0071. Recommended there: Plan 0076 is folded into it (its Keys page), superseded 2026-11-16.

**Plan 0074** (the sync's light stages): closed 2026-11-14 (v0.53.0). The next sync plans are the backlog's head: the light placement's eviction scan, then `alloc`.

**Plan 0075** (a loading screen): closed 2026-11-16 (v0.58.0). Plan 0069 rebases its `Run::init` edits onto the stepped start-up; the streamed worlds' loading stutter is the plan's followup.

**Plan 0076** (a key help overlay, drafted and approved 2026-11-16 from the owner's request after the Plan 0062 window; `F1` becomes help, the readings move to `Enter`): after Plans 0062 and 0075 merge, since both edit `toggles.rs`. Superseded 2026-11-16 (owner): folded into Plan 0080, whose Keys page is its panel and whose Phases 1 and 3 carry its work.

**Plan 0077** (an entity picker over the entities library, drafted and approved 2026-11-16; amended 2026-11-18 after the first window into a chooser with icons, a belt of slots and a placement mode with a marked ghost, ADR-0074 and ADR-0075): **closed 2026-11-18, v0.64.0**, both ADRs accepted with an Outcome. Plan 0079 Phase 4 and Plan 0083 Phase 6 extend `Run::aim_feet` and the Entities page; their re-reads are owed when their lanes reach them.

**Plan 0078** (the entity pool frees models): closed 2026-11-16 (v0.59.0), ADR-0060 accepted. Plan 0077 is re-read against it before its lane: its reload calls `free_model` and `EntitySet::replace`, and its HUD pool figure is 0078's.

**Plan 0081** (a surface detail layer: a style table per material id on the render side, a procedural shell marched in the hit pass where a ray meets a styled face, lit as a hit, faded by the voxel's size on screen, swayed by a wind the game sets; drafted 2026-11-16 from the owner's "outdoors needs trees, grass and water" and "maybe this new layer can be used for something else as well", approved 2026-11-16 with every recommendation; ADR-0063 proposed): after Plan 0032 closes and merges, before Plan 0033; it does not wait for Plan 0036, whose wind it takes when 0036 lands. Plan 0032 was amended the same day: its trees are prefabs stamped at generation time.

**Plan 0079** (entities aware of the floor and the walls: a sphere surface mover beside the cylinder controller in `vxe-sim`, the solid-count gradient as the smoothed normal in `vxe-world`, `Quat::from_frame` in `vxe-entity`, a `body` and a clip `stride` in the `.rig`, a seeded wander in the fly-through; drafted 2026-11-16 from the owner's request, approved 2026-11-16 with every recommendation; ADR-0062 proposed): **closed 2026-11-18, v0.65.0**, ADR-0062 accepted with an Outcome; every phase done, two course hashes pinned on both arms; the 64-spider frame CPU (+3.07 ms against a +1.5 ms guess) and the pathfinding plan are backlog entries. Phases 1 to 3 (`crates/` and the two library rigs) as a side lane once Plan 0063 and the clip-recorder chore merge, beside 0077; Phase 4 (the fly-through) after Plan 0077 merges, with the architect's re-read. **Re-read 2026-11-18** against 0077 as landed: the wander is the menu entry (`F11` is unbound), placing is 0077's belt and click, `belt::stand` branches on the body and the ghost of a surface body stands on the wall, the mover state is `Placed.mover`, the built-in walker keeps its circle, `hud_feed.rs` splits first; owner questions 9 to 11 (a key, the ghost on the wall, the words) took the recommendations (no key, the ghost on the wall, `wander`); the Phase 5 window asked for longer wander legs and an `Esc` confirmation, both in Phase 6.

**Plan 0082** (squash and stretch: a scale per axis per part in `PartPose` and in a clip's keys, `key 10 body scale 1.2 0.8 1.2` in the `.rig`, a slime with a bounce in the entities library; drafted 2026-11-16 from the owner's "jelly-like entities, slimes" question, approved 2026-11-16 with every recommendation; ADR-0064 proposed, a parent's scale moves its children's pivots and not their shapes): after Plan 0063, the clip-recorder chore, Plan 0078 and Plan 0079's Phases 1 to 3 merge (the owner's recommendation agreed: "squash-and-stretch plan after 0079"); it may run beside 0079's Phase 4 and 0077.

**Plan 0083** (anything from a `.vox`: every palette entry a material row, the material table as a text
file in the `.rig` grammar hot-reloaded in the window, prefabs stored as bricks, a stamp past the
residency sphere kept as a placement the world finishes as chunks arrive, layers, hidden nodes,
mirrors and an integer scale read from the scene graph, a Structures page of Plan 0080's menu;
drafted 2026-11-16 from the owner's "import nearly anything that can be created as a .vox file",
approved 2026-11-16 with every recommendation; ADR-0065 and ADR-0066 proposed): Phases 1 to 5 (`crates/` and `--vox`) once Plan 0063
and the clip-recorder chore merge, beside Plans 0078 and 0080; Phase 6 after Plans 0080 and 0077
merge, with the architect's re-read. It holds nothing of M10.

**Plan 0084** (staged structures: a stamp with a write policy and a region, stages from a `.vox`'s
layers, a pure reveal planner, a demolish that the support query follows; drafted 2026-11-16 from
the owner's "a castle being built... part of the map", approved 2026-11-16 with every recommendation; ADR-0067 proposed): after Plan
0083 closes.

**Plan 0085** (entity variants: `variant` blocks in the `.rig` that hide a part, swap its volume for another `.vox` node or remap a material, picked per instance by one index and honoured by the picture and the headless query; `Playback::ended` on a one-shot clip that already holds; a posed entity baked onto the world grid in integers, clipped to air on the server and stamped through op kind 2; the spider gains `damaged` and `dead`; drafted 2026-11-16 from the owner's "healthy, damaged, dead: three entities or one with modes?", approved 2026-11-16 with every recommendation; ADR-0068 and ADR-0069 proposed): after Plan 0063, the clip-recorder chore and Plan 0078 merge, and after Plan 0082 (recommended; it edits the same files), beside Plans 0077 and 0079's Phase 4. Per-entity voxel damage is parked in `docs/backlog.md`.

**Plan 0087** (the rename to `svet`, ADR-0071 accepted 2026-11-17 on the owner's "for renaming, I think svet is the best": `crates/svet-*`, `SVET_*` with no aliases, `svet:logic`, `svet-tester` and its archive, the documents and the harness, the owner's GitHub rename, a minor bump, a final grep; accepted ADRs, closed plans, logs, reviews and past CHANGELOG entries keep `vxe`; drafted 2026-11-17, approved 2026-11-17 on the owner's "Approve rename"): it touches every crate directory, so it runs **alone, when no lane is open**: after Plans 0069, 0032 and 0080 (and any other lane `git worktree list` shows then) merge, and after Plans 0077, 0079, 0083 and 0086 merge too (owner, 2026-11-17: "86 finish before").

**Plan 0089** (deterministic simulation testing: a dev-only crate `vxe-dst` that draws a scenario
from one seed, checks invariants after every step, shrinks a failing scenario by delta debugging
and writes it as a text file and a JSON line; a message-level `SimNet` channel in `vxe-net` with
seeded drops, delays, duplicates, partitions and clogs and the owned client's `Lost` rule; a GPU
mirror oracle against a fresh upload; a `SpillIo` seam with a simulated disk; `iter_over_hash_type`
and a `check-determinism.mjs` gate; drafted 2026-11-17 from the owner's "yes draft it" on the
article "Deterministic Simulation Testing"; ADR-0073 proposed): approved 2026-11-17 with the
recommended answers except the timing: every phase runs after Plan 0087 (the rename) merges
(owner). No new dependency.

**Plan 0091** (the frame profiler: `profiling` facade scopes in `vxe-render` and the apps, puffin
behind the fly-through's `profiler` feature, `--profiler-dump` writing a Chrome trace, a text tree
and `run.json`'s `scopes_top`, `scripts/profile-top.mjs` for agents, `PassTimer`'s passes as a
`gpu` lane, budget scopes, allocation counting, a HUD ticker with a frozen frame; drafted
2026-11-19 from the Bonsai second pass and approved the same day, ADR-0077 accepted): the owner placed it
**after Plans 0087 and 0089 and before Plan 0090** (2026-11-19). It touches the fly-through's
HUD, report and run loop, so it does not run beside a lane that edits them.

**Plan 0092** (WGSL hot reload in the fly-through: `Frame::reload_shaders` in `vxe-render`, an
mtime poll under `--watch-shaders`, the old pipelines kept and the error on the window on a
compile failure; drafted and approved 2026-11-19, no ADR, no dependency): the owner placed it
on the architect's recommendation, **right after Plan 0091 merges, before Plan 0090**, in the same lane, because
it is small, shares the HUD's menu and banner files with 0091, and shortens the window loop
that Plans 0090, 0081 and the M10 look plans run on.

**Plan 0093** (the README rewrite with reproducible screenshots: `--still-at`, `--still-dir` and
`--camera` on the fly-through, a manifest `docs/images/shots.json` and `scripts/shoot-images.mjs`
that re-renders every image under the perf lock, `scripts/check-images.mjs` holding 300 KB a
still, 700 KB a loop and 5 MB for the README in CI, fragment links validated, contact sheets the
owner picks from, the README after Bonsai's shape with the layout, checks and process moved to
`docs/development.md`; drafted 2026-11-19 from the Bonsai second pass, section 2, and the owner's
"more screenshots through the doc"; approved the same day, all recommended): **after Plan 0087 merges**
(0087 rewrites the README, every doc and the script names), **beside Plan 0089** (only `ci.yml`
in common) and **before Plan 0091** (which edits the HUD and `run/frame.rs` that its Phase 1
touches). 0093's canopy shot may wait for Plan 0094.

**Plan 0094** (the light's transitions: the key mean's age at a still camera, the no-ray sky
prior and one fade for every no-ray value change, ADR-0076 rule 5's exposure rate cap, the
dig's hole band in rule 3's form, the castle's fill-in, and the tunnel far-end and seal bars
Plan 0086 struck; plus the two items the owner routed here at 0086's slowdown, the light pass's
compile cost (the default-off arms out of the default pipeline, a pipeline cache for the GPU
tests, about 600 s of the gate) and three regressions against 0086's Phase 5 bars; drafted
2026-11-19 from Plan 0086's follow-up scope, its 86c and 86d
verdicts and its readings log, while 0086 is still open on its branch; no ADR, ADR-0076 rules
2, 3 and 5 are its targets; proposed, awaits the go): **starts after Plan 0086 closes and
merges**. The architect recommends it as the next lane, **before Plan 0087** (the owner's
still-image priority; its rigs, hooks and `VXE_*` names are 0086's and the rename would owe a
re-read; Plan 0090's first baseline then lands on the fixed tree); the alternative is after
0087 beside 0093 and 0089. Owner question 1 of the plan. **2026-11-20:** its owner window
(94a) is done; the forest and the castle read fixed, the dig and the forest at 15:00 not; the
plan closes on a short fix pass (Phase 7a: the exposure's rate 3.75 EV/s, the arms' pipelines
built in the loading stage, the pool off, six tests restated), Phase 8 narrowed, and a short
second look (Phase 9). ADR-0076 gained rule 6 (the undrained image is the judged image). The
motion wobble the owner named after the window is a separate lane's. **Closed 2026-11-20, v0.68.0**
(windows 94b and 94c: the rate pinned at 3.75 EV/s, the ceiling by the sky in view, the night
at `max`).

**Plan 0096** (performance: the bake queue's drain as a budget, ADR-0080 proposed: a record by
rule for a chunk with no face or buried with no emitter in reach, a sample tier by the chunk's
size on screen through the far-normal band, a slice that drains what is in view and falls to the
line once landed; the queue's step off its depth; the RENOIR's outdoor light pass lever by
lever; the sync's `alloc` and the light placement's eviction scan from the backlog; drafted
2026-11-20 from the owner's "address performance next" (2026-11-11) and window 94a's "if we have
to wait for a minute for light to settle, something is wrong with the light", ADR-0076 rule 6;
awaits the go): **after Plan 0095 (the motion wobble; closed 2026-11-21, v0.69.0) merges**,
which followed Plan 0094; then Plan 0087 (owner's order, 2026-11-20: 0094,
0095, 0096, 0087, 0089 and 0093, 0091, 0092, 0090, M10). Its Phase 1 is readings only, on both
adapters, on the undrained image.

**Plan 0097** (the exposure's anchor by the sky around the camera, ADR-0081 proposed: Plan 0094
Phase 7b anchors the ceiling by the share of sky in view, and at window W2 (2026-11-20, the
forest at `--sun 10,180`) one canopy closing over the last sky pixel released the ceiling by
about 5 EV, the owner's "when I reach really shadowed spot - too bright"; the anchor now holds
by the larger of the sky in view and the `+z` sky visibility of the baked floor record under
the eye, smoothed by a time constant, so a tunnel or cave still releases and the night stays at
+2 EV; four phases, a reading before the threshold, the mechanism, the owner's window with the
owed open hills at midnight, the picks as tests): **approved 2026-11-20 by the owner's picks at
W2 (all as recommended); its lane starts after Plan 0095 closes and merges**, beside or after
Plan 0096 as the owner orders then. The slope's step lines seen at W2 went to the backlog.

Every active plan but Plan 0096 is approved (Plans 0091, 0092 and 0093 on 2026-11-19; Plans 0087 and 0089 on 2026-11-17; owner, 2026-11-10; Plans 0070 to 0072 on 2026-11-12; Plan 0069 on 2026-11-11; Plans 0064 and 0065 the same day, after the
static performance review). Approval accepts the design; a plan's lane
starts only when the plans its "Starts after" line names have closed and merged, and where that
line asks for it, after the architect has re-read the plan against the new tip. The approved
plans were amended on 2026-11-10;
their figures for wall time and frame cost are estimates until a phase measures them.

## Recently closed

<!-- roster:begin cap=320 -->
| Plan | Title | Closed |
|------|-------|--------|
| [0095](done/0095-a-calm-key-light-in-motion-the-column-maps-horizon-at-the-far-the-baked-record-where-it-lands-a-deterministic-penumbra-near-no-pre-roll.md) | A calm key light: the far's record else the column horizon, the approach penumbra near, key re-bakes; RENOIR over the line | 2026-11-21, v0.69.0 |
| [0094](done/0094-light-transitions-the-key-means-age-the-no-ray-prior-and-its-fades-the-hole-band-and-the-castle-fill-in.md) | Light transitions: the still tap, the no-record prior and a 16-frame fade, the exposure at 3.75 EV/s, the ceiling by the sky in view and at most `max` | 2026-11-20, v0.68.0 |
| [0086](done/0086-still-outdoor-light-baked-sky-visibility-a-long-history-solid-canopy-shadows-and-stair-history.md) | Still outdoor light: a sky visibility baked per brick, the column map, the sky and key rays on a long history, canopies as bricks; ADR-0070 accepted; the leftovers to Plan 0094 | 2026-11-19, v0.67.0 |
| [0083](done/0083-anything-from-a-vox-palette-materials-a-material-file-and-large-scenes-stamped-into-the-world.md) | Anything from a `.vox`: palette materials, a material file, brick prefabs, placements past residency, structures in the belt with a coarse ghost; ADR-0065, 0066, 0077 | 2026-11-19, v0.66.0 |
| [0079](done/0079-entities-aware-of-floor-and-walls-a-surface-mover-the-smoothed-normal-and-a-wandering-demo.md) | Entities aware of the floor and the walls: a sphere surface mover, the solid-count gradient, a `.rig` body and stride, a seeded wander; 64 spiders +3.07 ms CPU (backlog) | 2026-11-18, v0.65.0 |
| [0077](done/0077-an-entity-picker-in-the-fly-through-choose-a-model-from-the-library-and-place-it.md) | An entity picker: the entities library, a chooser with icons, a belt of slots, a placement mode with a marked ghost | 2026-11-18, v0.64.0 |
| [0088](done/0088-the-outdoor-scene-in-the-tester-a-path-over-the-hills-through-the-trees-and-into-the-cave.md) | The outdoor scene in the tester: a computed path over the hills, between the trees and down the tunnel; a fourth menu row | 2026-11-17, v0.63.0 |
| [0080](done/0080-the-fly-throughs-menu-one-overlay-a-palette-and-the-minimal-key-set.md) | The fly-through's menu: one key table, a palette, the Keys page, the minimal key set and the pad; numbers edited, grey reasons | 2026-11-17, v0.62.0 |
| [0032](done/0032-sun-and-sky-hillaire-atmosphere-time-of-day-and-an-outdoor-testbed.md) | Sun and sky: Hillaire LUTs, a time of day, the sun or moon and one sky ray into the direct target, absent is sky, an outdoor testbed | 2026-11-17, v0.61.0 |
| [0069](done/0069-render-settings-one-surface-an-apply-class-per-field-and-named-quality-profiles.md) | Render settings: one struct, an apply class per field, a setter gate; profiles Floor, Portable, Performance, High | 2026-11-17, v0.60.0 |
| [0078](done/0078-the-entity-pool-frees-models-by-handle-stable-instance-slots-and-a-reset.md) | The entity pool frees models by handle into a coalescing free list; stable instance slots with a generation; ghost leaves; a reset | 2026-11-16, v0.59.0 |
| [0075](done/0075-a-loading-screen-steppable-start-up-a-progress-signal-and-a-default-loading-frame.md) | A loading screen: start-up stepped behind a turning cube and a bar, pipelines on a worker, a splash after the device; streamed stutter kept | 2026-11-16, v0.58.0 |
| [0063](done/0063-rigs-and-clips-keyframe-animation-a-model-viewer-and-text-authoring.md) | Rigs and clips: Q14 clips by tick, a `.rig` beside the `.vox`, the fly-through as viewer, `.voxtext`; between 0.50 ms at 64x16 parts; crossfade and error row to backlog | 2026-11-16, v0.57.0 |
| [0062](done/0062-entities-voxel-models-on-rigid-parts-traced-lit-shadowed-and-queried.md) | Entities: posed voxel models traced with the world, list-lit and shadowed, baked world light by default, a headless ray query | 2026-11-16, v0.56.0 |
| [0044](done/0044-hot-path-and-defect-pass.md) | The hot-path and defect pass: audio frees nothing, bind groups kept by identity (encode CPU 0.20 to 0.03 ms), material read at the hit; goldens unchanged | 2026-11-16, v0.55.0 |
| [0068](done/0068-a-tester-build-for-linux-and-windows-scenes-paths-and-a-report.md) | A tester build: Linux and Windows archives, three scene paths, tour and free flight, a report zip; Windows image 2-3x darker (backlog) | 2026-11-15, shipped in v0.54.0, no bump |
| [0058](done/0058-dynamic-lights-a-list-of-analytic-lights-dropped-carried-and-bounced.md) | Dynamic lights: sixteen exact lights, one shadow ray each, the direct part in rgb, a bounce flag; RTX 0.17 ms a light, RENOIR 1.5 | 2026-11-15, v0.54.0 |
| [0074](done/0074-the-syncs-light-stages-placement-spikes-attributed-and-exposed-emitters-by-masks.md) | The sync's light stages: `light_note` p99 8.3 to 2.5 ms by masks; placement index struck, then the window put the stagger in the eviction scan (backlog) | 2026-11-14, v0.53.0 |
| [0070](done/0070-the-sync-off-the-frame-slot-words-packed-on-the-workers.md) | The sync in place: rescan set dense, light walks on change; sync p50 180 to 17 us, `update + poll + sync` p99 45 to 24 ms, over 16.6; worker packing deferred | 2026-11-13, v0.52.0 |
| [0072](done/0072-development-speed-the-build-graph-a-narrowed-gate-and-one-compile-per-ci-profile.md) | Development speed: deps at line tables (cold build 455 to 403 s), `-p` gate and `--clippy`, reflink-seeded lanes (137 s); lavapipe stays at 1; CI readings owed | 2026-11-13, v0.51.0 |
| [0045](done/0045-structure-pass-split-the-mega-files-share-the-fixtures.md) | The structure pass: GpuWorld 11 fields, Frame 15, main.rs 36 lines; vxe-render's root 100 names; file-size and fn-length gates; benches at the start's | 2026-11-13, v0.50.0 |
| [0067](done/0067-the-frames-cpu-a-cheap-queue-step-a-bake-budget-by-time-and-no-black-for-missing-light.md) | The frame's CPU: a still queue drains at 0.075 ms a step; missing light reads neighbour, then parent (`chain`); bake stays frame:1.5 | 2026-11-12, v0.49.0 |
| [0061](done/0061-antialiasing-far-bands-under-the-baked-light-and-the-edges-left.md) | Antialiasing: the far store read by the shading normal and filters weighed by face; the owner: "no aliazing", a bit soft | 2026-11-11, v0.48.0 |
| [0065](done/0065-the-baked-lights-waste-atomics-a-filter-of-zeros-and-a-scan-per-frame.md) | The baked light's waste: temporal 0.32 to 0.07 ms, lamp-off a-trous a third, FSR 2 0.8 ms less; images byte-identical | 2026-11-11, v0.47.0 |
| [0064](done/0064-the-windows-frame-pacing-workers-and-readbacks-that-wait.md) | The window's frame: free pacing the default, below the compositor's; no frame-path GPU waits; the sync's time budget kept off | 2026-11-11, v0.46.0 |
| [0060](done/0060-the-live-queue-by-the-view-and-withdrawing-passed-work.md) | The live queue: a view score with the class as a tier, passed jobs withdrawn; the owner kept the withdrawal, saw no change from the order | 2026-11-11, v0.45.0 |
| [0066](done/0066-test-cost-a-10-s-soak-line-and-named-gpu-binaries-per-phase.md) | Test cost: the soak line at 10 s on lavapipe, 26 tests to soak, gate 505 s; `--binaries` per phase; CPU vectors stay in the gate | 2026-11-11, no bump |
| [0059](done/0059-loaded-light-a-per-chunk-store-baked-at-load-and-at-edit.md) | Loaded light: the owner picked the bake; a brick store at 8/4096 filled by a live queue and re-baked by digs; the table deleted | 2026-11-09, v0.44.0 |
| [0057](done/0057-steady-light-the-per-face-table-as-the-emissive-store-and-a-lamp-bounce-channel.md) | Steady light: the table holds still, the lamp channel stands; the owner moved to a look-first bake, vein bars left to it | 2026-11-05, v0.43.0 |
| [0056](done/0056-aliasing-spike-what-moves-on-the-final-image.md) | The aliasing spike: FSR 2's jumps are jitter on silhouettes, the dots light on creases; FSR 2 at 1.5 stays, far normal `[1, 2]` picked by eye (next plan) | 2026-11-04, v0.42.0 |
| [0051](done/0051-steady-light-spike-converge-and-freeze-and-a-world-space-emissive-cache.md) | The steady-light spike: A, B and C read behind settings; the owner picked B with indirect off, nearest read, the probes to a lamp-bounce channel | 2026-11-04, v0.41.0 |
| [0048](done/0048-probe-rays-direct-light-without-the-clamp-bias.md) | Dropped by the owner: moot under ADR-0043, the probe rays gather no emissive light; nothing was built | 2026-11-04, abandoned |
| [0055](done/0055-clarity-pass-no-detail-fsr2-default-and-a-far-normal-spike.md) | The clarity pass: procedural detail gone, FSR 2 at 1.5 the default, the far normal on at `[2, 6]` (provisional) | 2026-11-03, v0.40.0 |
| [0049](done/0049-light-settles-fast-and-young-pixels-hold-no-dots.md) | Abandoned by the owner: its Phase 1 readings kept in the plan, its rig under tag `archive/plan-0049`; Plan 0051 takes the work | 2026-11-03, abandoned |
| [0054](done/0054-field-surface-trilinear-zero-set-of-the-27-fraction.md) | Honest cubes the default, the planes gone (hit plus light 1.397 ms, mirror 60 % smaller at the level); the field surface built and kept opt-in | 2026-11-03, v0.39.0 |
| [0053](done/0053-one-voxel-features-on-the-contour-surface-majority-fit-and-a-normal-field.md) | One-voxel features: a box metric kept; the majority fit and the normal field failed the eye and are gone; ADR-0040 rejected | 2026-11-03, v0.39.0 |
| [0050](done/0050-watertight-contour-surface-crack-spike-and-closing-rule.md) | A watertight contour surface: a crack spike, then snap closes the wedges and triangles at +1.3 % (RTX) and +0.4 % (Radeon); the one-voxel boxes go to Plan 0053 | 2026-10-31, v0.38.0 |
| [0052](done/0052-second-review-defects-and-the-waste-the-first-pass-missed.md) | The second review: five defects fixed, fuel charged before the walk, exit 1 on failure, the idle 54.7 MB probe volume and the losing lab modes gone | 2026-10-30, v0.37.0 |
| [0047](done/0047-emissive-light-holds-still-long-guarded-history.md) | Emissive light holds still: an f32 history of 256 frames, an emissive-only guard and hashed probe slices; 1.5 patches a step from 77; motion faults to Plan 0049 | 2026-10-29, v0.36.0 |
| [0038](done/0038-showcase-cavern-and-hero-shots.md) | A showcase cavern lit by four ores, lamp off: a 72 m hall, three named shots, half dark with crystals leading (owner, Phase 6a); the hero re-shot from `hall` | 2026-10-28, v0.35.0 |
| [0040](done/0040-eye-adaptation-asymmetric-auto-exposure-and-pre-exposure.md) | Eye adaptation: asymmetric two-stage auto exposure metered on the GPU, pre-exposed HDR targets; owner chose 0.15 / 2 / 10 s and max 16 | 2026-10-28, v0.34.0 |
| [0041](done/0041-fsr2-port-to-wgsl.md) | FSR 2.2.1 in WGSL beside our upscaler: still, ghost and lamp bars held, slow pans and 1.2 ms (1.9 ms) missed; owner kept it opt-in, `own` at 1.5 the default | 2026-10-27, v0.33.0 |
| [0039](done/0039-contour-look-sharpness-measured-and-tuned.md) | The contour look at arm's length: a sharpness harness, five candidates; contour-sharp (weighted fit, lamp detail after lighting) the default; mirror bytes unchanged | 2026-10-27, v0.32.0 |
| [0046](done/0046-dx12-shaders-through-bundled-static-dxc.md) | DX12 shaders through bundled static DXC: WARP's eleven pipelines 76.6 s cold under FXC, 2.5 s under DXC; the attribute pass's WARP execution stays slow | 2026-10-22, v0.31.1 |
| [0029](done/0029-indirect-light-probe-volume.md) | Indirect light from a probe volume that scrolls with the camera: bounce kept at 1.6 m, update plus gather 1.57 ms, a light change refreshes it in 16 frames | 2026-10-22, v0.31.0 |
| [0042](done/0042-generated-gothic-castle-stamped-at-generation.md) | A generated gothic castle: a `.vox` writer, 21 tiles stamped into a carved hall; 344 MB of prefabs, about 92 % air, +0.3 GB RSS | 2026-10-22, v0.30.0 |
| [0043](done/0043-test-tiers-quick-gpu-full.md) | Test tiers: quick (CPU, no lock) per phase, gpu under the lock for render phases, full at close; 470 + 124 + 5 = 599, a gate on the gpu list | 2026-10-22, v0.29.1 |
| [0028](done/0028-size-classed-mirror-slots-and-surface-only-attributes.md) | Size-classed mirror slots: the level 232 to 63 MB, the default at `r_full` 16 under 0.9 GB, attribute pass p99 0.75 ms; the default's `sync` 3.1 ms open | 2026-10-22, v0.29.0 |
| [0030](done/0030-temporal-upscaler.md) | A temporal upscaler of our own: jittered rays, hit-position reprojection, RCAS; every bar met, shipped opt-in as a performance setting, the default stays Plan 0024's frame | 2026-10-22, v0.28.0 |
| [0024](done/0024-direct-light-hit-buffer-emissive-samples-and-svgf-denoiser.md) | Direct light: a hit buffer, emissive light samples, one shadow ray, an SVGF denoiser; half tier, coarse walk, reuse, a 32-frame emissive history; extraction p99 missed | 2026-10-22, v0.27.0 |
| [0027](done/0027-mirror-uploads-through-a-staging-belt.md) | Mirror uploads through a staging belt: `write_buffer` gone, plain `sync` p50 halved to 1.4 to 1.7 ms; the belt's share and the 1.0 ms line still missed | 2026-10-21, v0.26.0 |
| [0031](done/0031-edit-cost-third-pass.md) | Edit cost, third pass: a stage profile, shape words from the spans and one span solve per op; the `B` line at 1.0 ms, the line back to 2.0 ms | 2026-10-21, v0.25.0 |
| [0020](done/0020-cavern-reverb-directional-echoes-and-gliding-taps.md) | Cavern reverb: a late tail sized by the room the listener sees, echoes from their walls, taps that glide, and the player's own voice in the cave | 2026-10-21, v0.24.0 |
| [0023](done/0023-render-thread-cpu-attribute-hitch-and-streaming-line.md) | The render thread's mirror CPU: the attribute hitch cut with a narrowed shell and a capped pass, an incremental stream queue; the streaming line still missed | 2026-10-21, v0.23.0 |
| [0022](done/0022-a-falling-region-crushes-sub-threshold-voxels.md) | A falling region crushes the sub-threshold voxels under it and piles them as rubble, and the logic tells a wedge from a refusal and a landing | 2026-10-20, v0.22.0 |
| [0026](done/0026-gamepad-flight-generic-axes-in-the-shell-gilrs-in-the-testbed.md) | Gamepad flight for demos: generic axes and pad buttons in the shell's input, gilrs in the testbed only | 2026-10-20, v0.21.0 |
| [0025](done/0025-cutaway-view-see-the-world-from-outside-its-rock.md) | The cutaway view: a camera in rock or outside the world sees through the hull to the first cavern's walls | 2026-10-20, v0.20.0 |
| [0013](done/0013-contour-default-shadow-origin-and-streamed-footprint.md) | Contour as the fly-through's default: the shadow-ray origin, a streamed footprint under 3 GB, and a weaker rock detail | 2026-10-20, v0.19.0 |
| [0021](done/0021-packed-cpu-brick-census-kernel-and-readers.md) | The packed CPU brick: a census and kernel bench, then masks, planes and a palette under every reader | 2026-10-20, v0.18.0 |
| [0017](done/0017-mirror-budget-reservation-page-kinds-and-pressure.md) | The mirror reserves its budget: fixed buffers, empty pages that change kind, and a pressure signal the streamer follows | 2026-10-20, v0.17.0 |
| [0019](done/0019-metered-host-calls-and-shape-range-checks.md) | Host calls charged by their work, and a world that refuses out-of-range shapes and queries | 2026-10-20, v0.16.0 |
| [0018](done/0018-server-routed-stamp-drop-and-remove.md) | Stamp, drop and remove as op kinds 2 to 4 through the server, the log and the logic host | 2026-10-20, v0.15.0 |
| [0015](done/0015-traversal-hierarchy-chunk-bits-brick-masks-and-translucent-class.md) | The traversal hierarchy: chunk bits, full-resolution brick masks, a one-read-per-level walk, and the translucent class | 2026-10-19, v0.14.0 |
| [0016](done/0016-post-overnight-fix-pass-depenetration-restart-and-nits.md) | The post-overnight fix pass: depenetration, a logic that restarts after a trap, and the review nits | 2026-10-18, v0.13.0 |
| [0006](done/0006-spatial-audio-hrtf-mixer-and-occlusion.md) | Spatial audio: a headless HRTF mixer on the device callback, occlusion and early reflections through the material table | 2026-10-17, v0.12.0 |
| [0011](done/0011-wasm-host-for-the-logic-component.md) | A WebAssembly host for the logic component, proven by native-versus-component equality | 2026-10-17, v0.11.0 |
| [0007](done/0007-op-log-byte-channel-shared-and-owned-clients.md) | Server-owned op log, a byte channel, the logic boundary, shared and owned clients, and replay from seed plus log | 2026-10-17, v0.10.0 |
| [0010](done/0010-stats-hud-bitmap-font-overlay.md) | A toggleable stats HUD: a bitmap-font text grid and a frame-time strip in one overlay pass | 2026-10-17, v0.9.0 |
| [0009](done/0009-support-query-and-collapse-ops.md) | A support query on the coarse grid, and deterministic drop and rubble ops for collapse | 2026-10-17, v0.8.0 |
| [0014](done/0014-edit-cost-brick-array-pool-and-cheaper-spans.md) | Edit cost, second pass: a brick-array pool, cheaper sphere spans and an analytic capsule row; 2 ms missed | 2026-10-17, v0.7.0 |
| [0008](done/0008-vox-import-and-prefab-stamping.md) | MagicaVoxel import with a material map, and prefab stamping with 24 rotations at generation time and at runtime | 2026-10-17, v0.6.0 |
| [0012](done/0012-comment-hygiene-and-index-row-gates.md) | Two repository gates copied from Ritmolux: comment hygiene in sources, and a byte cap on index rows | 2026-10-17, v0.11.1 |
| [0004](done/0004-cylinder-sweeps-character-controller-and-coarse-grid.md) | Cylinder sweeps, a character controller, and a per-brick coarse grid with change notifications | 2026-10-17, v0.5.0 |
| [0003](done/0003-edit-cost-span-fills-and-per-op-tables.md) | Edit cost under 2 ms: row spans, a per-op target table and per-brick tallies | 2026-10-17, v0.4.1 |
| [0005](done/0005-surface-look-spike.md) | The surface-look spike: smoothed normals, per-voxel contour planes and procedural detail, measured on screen | 2026-10-16, v0.4.0 |
| [0002](done/0002-streaming-residency-lod-ring-and-mirror-narrowing.md) | Streaming residency, an LOD-only ring, and a narrowed, sharded mirror | 2026-10-16, v0.3.0 |
| [0001](done/0001-bounded-brick-map-edits-and-headlamp-marcher.md) | Bounded brick map, deterministic edits, and a headlamp ray marcher | 2026-10-16, v0.2.0 |
<!-- roster:end -->

## Conventions

- One plan per milestone slice, numbered when drafted, never reserved ahead.
- A row carries at most two sentences of live constraint. What landed and why belongs in the
  plan file.
- A closed plan leaves this table; `done/` records it.
