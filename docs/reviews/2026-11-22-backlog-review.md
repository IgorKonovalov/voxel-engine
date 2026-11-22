# Backlog review, 2026-11-22: closed entries and inconsistencies in `docs/backlog.md`

> **Written:** 2026-11-22. It reviews [`docs/backlog.md`](../backlog.md) at main `6372a52b`.
> **Read-only:** nothing was changed, in the backlog or anywhere else; no cargo run.
> **Asked for by the owner:** (1) inconsistencies, (2) closed entries.
> **Read in full:** the backlog (1609 lines), the plans README, the ADR index. Spot-checked:
> the roadmap, `docs/nfr.md` section 7, the plan files named below, the three lane worktrees
> (plan-0096, plan-0097, plan-0098), and code names by grep.
> **What this is not:** a plan, and not an edit. Line numbers are the backlog's at `6372a52b`.

"Verified" means the file was opened or the tree grepped. "Inferred" means it was read from
context only.

One point on the backlog's header rules (lines 7-16): they give an entry two exits, a one-line
pointer when a plan or ADR takes it, or deletion when the owner drops it. They say nothing
about a "Done:" heading or about when a one-liner to a closed plan leaves. Both are judged
here as deletable, since the file is for what "no plan, ADR or roadmap milestone carries yet".

## 1. Closed or done, but still present or reading as open

| Line | Heading | Evidence | Action |
|---|---|---|---|
| 56 | The streamed benches on Windows: CPU frames of 20 to 256 ms on a desktop | Body ends "Done 2026-11-16 (chore `chore-default-workers`)"; verified `setup.rs:368` is half the logical cores. Sits first-but-three in "Next up". | Delete; move the 2026-11-17 Linux tester reading (60-69) to `docs/nfr/` if wanted. |
| 86 | Done: test tiers by marks, not by copied lists | Done 2026-11-10. | Delete. |
| 168 | The live queue's order and cancellation, after Plan 0059 | One-liner to Plan 0060, closed 2026-11-11. | Delete. |
| 375 | The lamp channel's faults | Taken by Plan 0058 Phase 3; verified in the plan (line 595); closed 2026-11-15. | Delete. |
| 379 | Far light on a slow adapter: a bake budget per second, and no black for missing light | Taken by Plan 0067, closed 2026-11-12. | Delete. |
| 438 | The hall's headless render went dark: `showcase_hall_golden_is_lit_by_its_four_ores` | "Done 2026-11-16 (chore `chore-showcase-golden`, b09e018)" inside; heading not marked. | Delete. |
| 549 | Done: the showcase's palette after the Crystal Islands review | Done 2026-11-20. Lines 293 and 547 point at it. | Delete; fix those two pointers. |
| 704 | The sync's `light_note` stage and the windowed glide's light walks | One-liner to Plan 0074, closed 2026-11-14. | Delete. |
| 788 | The tester mode's key filter is structural, not tested in the frame loop | "The entry leaves when that plan closes"; Plan 0080 closed 2026-11-17. | Delete. |
| 846 | Done: the tester's benchmarks with the lamp off | Done 2026-11-13; the bold caveat on readings before 4ea561f is still load-bearing. | Move the caveat to `docs/nfr.md`, delete. |
| 880 | Plan 0072's CI readings: accepted, one reading left open | Body says "no longer owed". | Delete. |
| 1096 | llvmpipe SIGSEGV on CI's Ubuntu since Plan 0057 Phase 7 | Resolved 2026-11-08/09; verified `.github/actions/lavapipe` exists and `ci.yml` uses it. | Collapse to one line: drop the pinned build when the runner ships Mesa 26.2. |
| 1298 | Allocation counting per frame in the apps | "Stays until that plan is approved, then leaves"; verified Plan 0091 approved 2026-11-19 and its Phase 6 is this. | Delete. |
| 655, 750 | Plan 0044's leftovers (last bullet); The frame path's GPU waits in the gate ("It failed" paragraph) | Both say fixed 2026-11-16 on `chore-soak-frames`. | Delete the fixed text from both; 750 keeps only the gate-variant ask. |
| 716 | The slow adapter's walk-speed spike | Reads as owed "before Phase 6". Verified in Plan 0069 (done file lines 48 and 427, log line 179): the owner accepted 10.53 ms on 2026-11-17, no spike before Phase 6. | Needs the owner: drop it, or merge into the beam prepass entry (612), which now carries the RENOIR's hit pass. |
| 396 | Rebuild the world live behind the loading screen | Verified in Plan 0080's log (line 787): source and trees switches already load behind the loading frames. | Rewrite to what is left: Rebuild-class settings and the drop-and-recreate test. |
| 1476 | Settle early: a coarse area answer where an estimate is not still, and a hold once it is | 45 lines of history. The hold form was rejected, the code removed (verified: no `settle_at_cap`, `--settle` refused in `cli/tests/outdoor.rs:245`), rule 6 written. | Collapse to the list of candidate places for rule 6. |
| 819 | Plan 0068's leftovers from the owner's runs | Inferred: the 2026-11-17 Linux tester archive (line 60) ran all three benches. | Delete if the owner agrees. |

## 2. Taken by a plan, with stale or wrong routing

- **Lines 50, 54, 84, 420, 424, 578** (the six "Taken 2026-11-20 (draft, awaits the go) by Plan 0096" pointers): verified Plan 0096 is approved 2026-11-20 and in progress in its lane. Action: drop "(draft, awaits the go)".
- **1533, "The bake queue's backlog at a still camera: minutes to drain what is in view"**: reads open ("the perf plan's first item"). Verified Plan 0096's context (lines 27, 52, 580) and ADR-0080 are exactly this, and Plan 0099 takes the missed in-view drain line. Action: one-liner to Plans 0096 and 0099.
- **52, "The CPU settle outdoors: the bake queue's step on a world with no emitter, and the forest's sync p99"**: main does not mention Plan 0099. The plan-0096 lane re-points the per-job cost to its new entry "The bake step off the main thread". That lane entry says "Plan owed ... a fresh architect drafts the plan", which is already stale: Plan 0099 and ADR-0083 exist and are approved. Action at the 0096 merge: make the lane entry a one-liner to Plan 0099. Both it and today's far-trees entry sit at the top of "Next up", so expect a merge conflict there.
- **208, "The bake queue ordered by what the march touched (2026-11-20)"**: its trigger is "if the in-view drain misses the 10 s bar". The lane's text says that line was missed. Verified Plan 0099 takes the line but never names the touched set. Action: needs the architect, into 0099 or explicitly parked.
- **908, "A pipeline cache for the renderer's shaders"**: says "Routed to Plan 0096 ... Plan 0096 takes two items" (an engine `wgpu::PipelineCache`, and the default-off arms built in the background). Verified Plan 0096's not-do list says the opposite, on main (line 652) and in the lane (line 980): "the engine's is the backlog's". No 0096 phase mentions a cache. Nobody owns the 70 s cold start. Action: needs the owner; re-point or plan it.
- **480, "Plan 0090 owes a stamp scene and a reading of the key mean's fade-in against `F`"** and **498, "A freshly placed castle's interior under the sky, as a pop-counter scene"**: both ask for the same scene (25.png) in Plan 0090, "at Plan 0086's close". Plan 0086 closed 2026-11-19. Verified Plan 0090 has only the hall still, no castle stamped on the hills, no `key_mean`. Action: merge the two; the architect amendment to 0090 is owed before its lane.
- **1305, "A hybrid far field: raster the far LOD as meshes, march near"** and **20, "Distant trees that do not wobble" (arm 1)**: both say Plan 0033's re-read takes them, and the owner "agreed" the hybrid on 2026-11-20. Verified Plan 0033 was last edited 2026-11-17 and has no word of hybrid, raster or stand-in. Its roster row reads "Re-read 2026-11-17 ... questions answered. After 0081". Action: put a "re-read owed" note in Plan 0033 and its roster row, then collapse both entries.
- **460, "Thin step lines along voxel slopes"** and **1331, "Narrow the far normal's band so far pixels stop casting the ray"**: full bodies kept, with "Taken 2026-11-21 by Plan 0098" appended. Action: one line each, per the header rule.
- **1354, "The baked key record's cell blend carries light across a shadow edge"**: "Pick up after Plan 0098's window W1". Verified in the lane's plan status: W1 picked `key far column`, owner confirm owed. By the entry's own rule it stays parked. Action: say so; the white-patch roof slot is still "the owner's call to close".
- **1371, "A mip by majority can drop a roof one or two voxels thick"**: "Pick up with Plan 0098's Phase 5 amendment". That amendment was written 2026-11-22 without it (verified: no "majority" or "any solid" in the lane's plan). Action: new trigger, or into 0098 Phase 5.

## 3. Contradictions

- **56, internal**: "settles it: the 0.51.0 stutters were the build, not Windows" (line 58) against "whether Windows is the OS or the older base stays open" (line 68).
- **360, "Remove `own`, the engine's own upscaler"**: "the removal plan goes ahead after Plan 0069" (owner, 2026-11-11). Plan 0069 closed 2026-11-17 and no plan was drafted; verified `own` is a field of `RenderSettings` (`settings.rs:209`). The later rule (2026-11-12, arms that cost nothing when off stay as settings) points the other way. The plans README (Sequence item 6) and roadmap line 780 still list it as owed. Needs the owner: remove (saves pipelines, goldens, gate tests) or keep and delete the entry and both mentions.
- **136, "The gate tier's tests over the 10 s line, and its wall time"** and **988, "The gate tier is over its wall-time budget"**: both still measure against "the 12 min proposal". Verified `docs/nfr.md:513` is 19 min since 2026-11-18. Neither holds the latest readings: 1588 to 1711 s at Plan 0094, 1900 s at Plan 0095, accepted by the owner as readings.
- **568, "Budget lines over with no owner"**: "Upscale, 1.5 ms" and "the bake's slice ... has no line". Verified `docs/nfr.md:48-57`: the upscale line is 3.0 ms and a bake line exists since Plan 0069's close. Action: prune to the bullets still true (streaming CPU p50, time to first frame, the ledger tables).
- **1305**: "the re-read runs ... as the first M10 step" against the README's run order "0081, 0033, ...". Also the roster shows Plan 0033 (marched clipmap, ADR-0028) as approved while the backlog calls the hybrid the agreed direction.
- **282, "Darker scenes, and light intensity varied by seed"**: sends its testbed half to the palette entry, where the owner's window P1 did not take the darker key (line 555). What is left is only the seeded per-light spread.
- **1143** (the Bonsai section intro): "added the entries from 'Per-job performance tracking' on" is false now; five non-Bonsai entries follow.
- **Plans README, outside the backlog**:
  - The Plan 0096 paragraph says "awaits the go", and line 237 says "Every active plan but Plan 0096 is approved"; the roster says approved.
  - The Plan 0099 paragraph says "draft ... awaits the owner's go and five answers"; the roster and the plan say approved 2026-11-22.
  - The Recently closed row for Plan 0083 names ADR-0077, which is the profiler's; the ADR index gives Plan 0083 ADR-0078.

## 4. Duplicates and near-duplicates

- **221, "A wgpu pipeline cache owned by the engine"** and **908, "A pipeline cache for the renderer's shaders"**: 908 itself says "the same question from the start-up side". Merge into 221; keep 908's lavapipe compile-cost reading as a bullet.
- **136 + 858 ("Five lavapipe gate tests over the 10 s line") + 988**, with **889, "The lavapipe group at two threads (owner question)"** as their one real lever. Merge into one gate-time entry; readings belong in `docs/nfr.md` section 7.
- **480 + 498**: see section 2.
- **90, "A headless run at `--residency 3,4` hangs before its first frame"** and **406, "The pre-roll never settles at small residencies"**: inferred same cause (the pre-roll's wait at a small window). Merge.
- **716 + 966 ("Shader toolchain spike") + 638 ("Narrower frame targets and fewer stores") + 647 ("Uniform branches as pipeline constants")**: a chain where each waits on the next and none has a trigger of its own. One "slow adapter's hit pass" entry.
- **The raster question, five times**: 20 (arm 1), 1305, 612 ("A beam prepass, or a distance value per brick", its 2026-11-22 brick-boxes paragraph), 266 ("Hundreds or thousands of entities", rasterized entities) and 1215 ("Lit world-space sprites (cards)"). Each says it needs its own ADR against ADR-0001's all-compute stack. Keep the entries, add one cross-linked owner question: does the engine get a raster pass, decided once.
- **1550, "The dug hole's light: the first frames and the lit floor"** and **1596, "How dug interiors should look: darker, less sky than the true aperture"**: 1550's lit-floor half is answered ("Read: `geometry`") and what follows is 1596's look. Fold that half into 1596.
- **106, "`StampedSource` shares its prefabs across placements"** and the first item of **258, "Plan 0083's leftovers"** (`World::stamp` copying the prefab into a fresh `Arc`): the same sharing fix on two paths. Cross-link.

## 5. Wrong section

Entries were appended at the end of whatever section was last.

- **Under "Later apps", seven lighting entries**: 1462 "The fired lights' grain", 1476 "Settle early", 1522 "A second `Frame` on one rig does not drain its bake queue", 1533, 1550, 1580 "The key term's grain at rest on a penumbra", 1596.
- **Under "Ideas from Bonsai", not Bonsai's**: 1331, 1347 "The F9 steps view drops the window to about 14 fps", 1354, 1371. 1305 is by now the owner's far-field direction.
- **Under "Tooling and process"**: 975 "A held fallback still moves on screen" (render), 1100 "Pathfinding over the coarse grid and the walkable map" (engine), 1113 "The built-in walker and its scripted circle" (app chore), 1123 "The spider's mover tick costs twice the guess" (performance).
- **Under "Engine mechanisms"**: 430 "The bake and strip examples drift" (tooling), 480 and 498 (tests), 494 "A consumer's settings file in the render example" (apps).
- **"Next up" is not a next-up list**: of its fifteen entries, four are one-liners to plans, one is done and one is "Done:".

Proposal: add a "Lighting and look" section. It would take about twenty entries from Engine mechanisms and the tail of Later apps.

## 6. Pick-up condition already happened, or no trigger

- 175 "Chunk-scale world dynamics": "when ... Plans 0056, 0057 and 0058 closed"; all closed by 2026-11-15, while lighting is still in flight (0098). Re-word the trigger.
- 106: "at Plan 0032's close" (2026-11-17).
- 414 "The near shell's cost in a flight, and the streaming edge": "with Plan 0067" (closed 2026-11-12). Inferred: Plan 0096's tiers overtake it; re-read at 0096's close.
- 708 "The tester cave's coarse light is slow to settle": "next bake-queue plan" is Plan 0096, which does not name it.
- 446 "Ambient occlusion driving a per-material colour ramp" and 599 "The a-trous under the baked light: fewer iterations, or none": "after Plan 0058".
- 532 "Per-voxel albedo variation by a seeded hash" and 434 "Overlapping coloured lights go white": "after the content pass", done 2026-11-20. 532 is ripe for its look spike.
- 740 "The window's frame rate out of focus": "Plan 0067's window".
- 830 "The RTX laptop's TSC is marked unstable": "at the owner's next reboot"; several since. Needs the owner.
- 850 "A `Uniform` emitter brick gives no light samples": "after Plan 0074". Verified still open (`note_lights` tests `is_full()`).
- 858: "Plan 0062 ... can take them" (closed 2026-11-16).
- 889 and 898 "`split-debuginfo` on Linux only (owner question)": unanswered since 2026-11-13. Verified `max-threads = 1`, and no split-debuginfo set.
- 940 "Comment, manifest and index drift found by the 2026-11-12 review": "next chore lane". Verified still open: both placeholder descriptions, 26 "contour" lines in `world_budget.rs`, the `gpu_lights.rs` comments.
- 779 "The 64 private-intra-doc-link warnings": "after Plan 0069 merges".
- 666 (in Plan 0044's leftovers): "widen the test when Plan 0069's settings surface lists the modes".
- 975: "pick up on that answer" (Plan 0059's glass pane, closed 2026-11-09); no answer recorded. Inferred moot if Plan 0098's no-screen-history default lands.
- 992 "`vxe-render::exposure` hangs on WARP since Plan 0057 Phase 7": "next, the owner's call: one push on WARP", open since 2026-11-09. Verified the `diag:` lines are still in `app.rs:37-46`.
- 1462: waits on "Plan 0086 Phase 24 run 3" (closed 2026-11-19).
- 1550, 1580, 1596: "after the motion lane", which is Plan 0095, closed 2026-11-21. Inferred: 1580 is overtaken by 0095's deterministic penumbra and 0098's picks.
- 71 "Windows draws the whole image two to three times darker than Linux": the code side is done; one screenshot pair is owed, and 282 waits on it.
- 1522: "a chore or the perf plan's"; verified Plan 0096 does not name it. No owner.
- 725 "The dev box presents across two GPUs" and 732 "Costs that multiply across the approved plans": no date and no trigger. 732's "the render-profile plan is where the product gets a cap" names Plan 0069, now closed.

## 7. Broken or stale references

- 1553, 1599: `live/window94a/*.png` is not in the main checkout and not under `~/Work/vxe-live-archive/`.
- 475: "a change to the far-normal band is ADR-0063's table". ADR-0063 is the surface detail layer and holds no "far normal" (grep). Likely the wrong ADR number.
- 729: `references/project-context.md` is `.claude/skills/architect/references/project-context.md`, and it does not carry the two-GPU fact (no "iGPU", "composit" or "Hyprland").
- 944-949: line numbers drifted (`gpu_lights.rs:687` is now 691, `gpu_upscale.rs:635` is 647, `exposure.rs:21` is 22, `gpu_frame.rs:86` is 57).
- 170: "the antialiasing plan below"; there is no such entry.
- 159: "the pipeline-cache entry below"; there are two.
- 701: drafts "stay untracked in the plan-0070 worktree"; that worktree is gone.
- 1275: "the settle readings (the entries under Next up)"; those entries are now one-liners.

## 8. The file's own rules and the project's doc rules

- **"Under about ten lines"**:
  - 992 runs about 100 lines with six tables.
  - 1098 is one 6,000-character paragraph; 990 and 260 are about 3,700 each; 58 is 2,400.
  - 1476-1520, 908-938, 136-166 and 20-46 each run 27 to 45 lines.
  - The WARP and llvmpipe investigations belong in a log or a review file.
- **Counts in prose**:
  - 666: "`frame/encode.rs` is at 1190 of the 1200-line cap". Verified 1083 now; delete the bullet.
  - 858: the heading says "Five", the body lists eight.
  - 779: "64" in the heading, not re-counted.
  - 143-156: "22 tests", "29 tests", "61 tests".
  - 260: "1169 of 1200" is still true today.
- **Plan-relative narration**: 718 "in no plan or backlog entry until now"; 331 "Phase 14 deletes it"; the six "(draft, awaits the go)"; 911 "(at the entry's date)".

## Counts (about 130 entries)

| Kind | Findings |
|---|---|
| Done or closed, still present or reading open | 19 |
| Taken, with stale or wrong routing | 16 |
| Contradictions (three more in the plans README) | 7 |
| Duplicate groups | 8 |
| Misfiled entries (plus "Next up" as a whole) | 19 |
| Trigger already happened, or none | 27 |
| Broken or stale references | 8 |
| Over the length rule (worst cases) | 9 |
| Stale counts and narration | 8 |

## The five that matter most

1. **The far field's direction lives only in the backlog (1305, 20).** The owner agreed the hybrid on 2026-11-20 and folded the stand-in arm into Plan 0033 today. Plan 0033 is still the approved marched clipmap with no re-read owed on it or its roster row, and the run order conflicts. The raster-pass question appears in five entries with no single ADR decision.
2. **The engine pipeline cache has no owner (908 against Plan 0096).** The backlog says 0096 takes it; 0096 says it is the backlog's. The owner's "start is slow, 70 sec" sits between them, and 221 duplicates 908.
3. **Gate time is three stale entries.** 136, 858 and 988 still argue with a 12 min line while the gate reads 1588 to 1900 s against 19 min. The one lever that halves it (889, lavapipe at two threads) has waited on the owner since 2026-11-13.
4. **"Remove `own`" (360)** is past its trigger, undrafted, at odds with the keep-as-setting rule, and listed as owed in the plans README and the roadmap. It needs one owner answer and edits in three files.
5. **Bake-queue bookkeeping at the 0096 merge.** Six stale "(draft, awaits the go)" pointers, 1533 not marked taken, the touched-set trigger (208) met with no route into Plan 0099, the lane's "bake step off the main thread" entry already stale, and a merge conflict at the top of "Next up".

Runner-up: Plan 0090's owed amendment (480 and 498) should land before that lane starts.
