# 0059: Loaded light: a per-chunk store of face values, a reference with no budget first, then cuts by eye

> **Status:** in-progress: Phase 8c done 2026-11-08 (exploration mode; coarse first, the near cube at 4096); Phase 8d (human) ready, `live/session8c.sh`; Phase 8b done 2026-11-08 ("coarse first + bigger initial area"; the amendment of 2026-11-08, 9364b50); Phase 8a done 2026-11-08 (exploration mode; the queue's throughput); Phase 8b (human) ready, `live/session8.sh`; Phase 8 done 2026-11-08 (exploration mode; the live queue); Phase 7b done 2026-11-08 (bricks 8/4096 everywhere); Phase 7a done 2026-11-08 (exploration mode); Phase 7 done 2026-11-08 (the brick level is the direction, its look not enough; the amendment of 2026-11-08); Phase 6 done 2026-11-08 (exploration mode); Phase 5b done 2026-11-08 ("works perfectly"); Phase 5a done 2026-11-08, built for the owner's eye, bars not read; Phase 5b (human) ready, `live/session5c.sh`; Phase 5 done 2026-11-08 (512 stands; the glass to Phase 5a, ADR-0047); Phase 4a done 2026-11-07 under the amendment of its glass bar; Phase 5 (human) ready, `live/session5.sh`; Phase 4 done 2026-11-06 under the amendment of its stops (Phase 3 done 2026-11-06 with
> the verdict yes; Part 2 re-read by the amendment of 2026-11-06; owner's go 2026-11-05; amended 2026-11-05 to look first, cut later; amended 2026-11-05 (owner) to a
> bounded piece and the stop's submits after Phase 1's stop rule)
> **Created:** 2026-11-05
> **Milestone:** M2 in [`docs/roadmap.md`](../roadmap.md) (the emissive light behind "dark scenes,
> emissive voxels as the light")
> **Related ADRs:**
> [ADR-0046](../adrs/0046-static-emissive-light-is-baked-per-chunk-when-a-chunk-loads-or-changes.md)
> (proposed; this plan builds it and its close accepts it), which replaces, on the owner's verdict
> in Phase 13, the table, fill, preload and guard of
> [ADR-0043](../adrs/0043-emissive-light-from-a-world-space-per-face-table-and-a-lamp-bounce-probe-channel.md),
> and refines
> [ADR-0025](../adrs/0025-light-candidates-from-a-power-over-distance-table-of-near-emitter-chunks.md)
> and
> [ADR-0045](../adrs/0045-the-per-face-table-stores-irradiance-at-unit-emission-and-the-read-applies-the-scale-now.md).
> **Source:** the owner's windows of 2026-11-05 (`docs/plans/logs/0057.md`, Phases 6 and 8)
> and this design's interview of the same day.
> **Starts after:** [Plan
> 0057](done/0057-steady-light-the-per-face-table-as-the-emissive-store-and-a-lamp-bounce-channel.md)'s
> close. Before Plan 0058, whose emission-scale phase needs a refresh (Followups).
>
> **Amended 2026-11-05 (architect, for the owner), the order.** The owner, after Plan 0057's second
> window: "what if we go differently - no matter performance, but build something that would look
> really polished, then we will see where we can cut performance?", and on its scope: "agreed, only
> light. antializing next". The first thing built is now a reference with no budget, and the owner
> judges it before any budget work starts. The old Phases 1 to 8 are Phases 4 to 14, re-cut around
> that; everything after Phase 3 is provisional and the architect re-reads it at the verdict.

> **Amended 2026-11-05 (architect, for the owner), glass at rest.** The owner asked that Phase 3's window look at glass at
> rest. A translucent hit has no face, so it keeps the fallback, and on 299b5c8 a fallback whose accumulation holds
> bit-identical still reads 51.10 composite patches a step at the patch pose, with every hit a fallback (`docs/backlog.md`,
> "A held fallback still moves on screen"). The reference's two worlds hold no glass: its only row is the castle's, and
> `--reference` refuses the streamed world the castle lives in. So Phase 2 gains a key that places a pane, and Phase 3 a step.

> **Amended 2026-11-05 (architect, for the owner), an early look.** The owner asked to see the light sooner, so Phase 1b puts
> Phase 1's bake in a window before the whole-world work starts. It is a first look, not Phase 3's verdict.

> **Amended 2026-11-05 (owner), a bounded piece and the stop's submits.** Phase 1's stop rule fired (`logs/0059.md`,
> Phase 1). The kept arm (32 candidates, clamp off) meets the `G_noise` bars at all four views at 65 536 samples, but the
> census read the level at 21 316 662 faces (surface share 0.990, not 0.5) and a sample costs 36 to 64 ns in its own
> dispatch (not 4.6), so a whole level projects at about 15 h on the RTX. The stop's own path ran at 310 ns a sample,
> because a chunk's last faces took one waited submit a round. The owner's answers, verbatim: on the reference, "Bounded
> piece, bars kept"; on Phase 1b, "Fix submits, radius 1". So the bars, `BAKE_SE`, `N_MIN`, `N_MAX` and the kept arm
> stand. The reference becomes a baked piece of the world around each of the window's poses, not a whole world (TL;DR,
> Context, Decision, Phases 1b to 3). A new dev Phase 1a makes the stop's submits efficient before 1b, and 1b runs at
> `--bake-radius 1`. Part 2 is unchanged, and its numbers (4.6 ns, the share of 0.5) are re-read at the verdict as before.
>
> **Phase 1's departures, ruled (architect).** Each is accepted as the Notes state it. (1) The deepest count is 65 536,
> and the reference is the mean of a view's four 65 536 runs: the bars compare seed with seed and never read the
> reference, and the clamp's bias is a paired difference against it. (2) The clamp arms reuse the unclamped arms'
> samples, which gives a paired difference, tighter than separate runs. (3) The zero share is read on the views' faces,
> since no phase bakes a whole world now. (4) The stop was read at the vein only: Phase 1a reads it there again, and
> Phase 2 reads it at every view. (5) The pool is clamped to 220 MiB: a radius-2 piece takes under 10 MB, and even the
> whole level's 177 MB fits. (6) The running sums, the stop and the packing run on the CPU in f64, which Phase 1a may
> change under its rule. (7) `--light baked` is refused without `--level` or `--source showcase`, and Phase 8 lifts that.

> **Amended 2026-11-06 (owner), Part 2 re-read at the verdict.** Phase 3's verdict was yes (`logs/0059.md`, Phase 3). The
> owner's answers to the architect's interview the same day, verbatim. On edges and the fast turn in the reference: "Barely
> noticed". On load time: "Playable in seconds, light refines". On digging: "No freeze, settles fast". On the order: "Cuts
> first, then AA". So:
> - **No antialiasing plan comes next.** It goes back to `docs/backlog.md` as polish, and Part 2 runs now. Phase 10 no
>   longer waits for it: it reads native and FSR 2 at 1.5 (the fly-through's default) against the reference.
> - **Two of the owner's targets become bars,** set in the phases that build them. **A fresh start** (Phase 8): the first
>   frame within 3 s of the world's load on the RTX, with every pixel whose hit is within 64 voxels at a face cell. The
>   rest bakes in the background behind the fade, with no visible pop. 3 s is the architect's reading of "seconds" (a
>   proposal). The arithmetic: a radius-1 ring is about 1.9 x 10^5 faces (Phase 1b), and at 512 samples and 13.5 ns
>   (Phase 1a) that is about 1.3 s of bake. **A dig** (Phase 9): no hitch, meaning no frame on the dig frame or the 30
>   after it over 33 ms, and the bake's slots in each of those frames at most `bake_slice_ms` plus the edit cap. The
>   hole's faces settle within 2 levels (p90) of a fresh bake within 0.5 s of wall time. The arithmetic: an arm's-length
>   ring of about 11 000 faces (Phase 2's reading) at 512 samples is about 5.6 x 10^6 samples, about 76 ms of GPU time,
>   which the slice and the edit cap together (about 2.4 x 10^5 samples a frame at 13.5 ns) clear in about 23 frames.
>   A `B` press (about 3.3 x 10^5 faces) then settles over about 10 s. That is read, not barred.
> - **The first draft's guessed rates move to Phase 1a's 13.5 ns.** `BAKE_EDIT_SAMPLES` falls from 400 000 to 133 000
>   (1.8 ms), and Phase 4's cost stop moves from 8 ns to 20 ns a sample. Both are re-read in their phases.
> - **The glass pane's white blob** (Phase 3's rerun, `live/session3b-glass-blob.png`, the HUD at manual +4 EV) is read
>   in Phase 4 with the lamp off and on. If it is the lamp's spot at 60 cm, the log says so and the entry is closed; if
>   not, it goes to `docs/backlog.md` beside the held fallback. No cut fixes it.
> - The owner did not answer Phase 3's (e) and (f). Phase 5 asks them again beside the filter.

> **Amended 2026-11-06 (owner), Phase 4's stops.** Two of Phase 4's stops fired (`logs/0059.md`, Phase 4, at 8df5c6c). The
> owner's answers:
> - **No seed form met the offer's bars at 512 samples.** The shared forms pass `G_local`, but their `G_seam` p90 is 3.0 to
>   5.1 and their `G_patch` p90 4.4 to 9.9, against independent's 0.7. Independent and lattice miss `G_local` p90 by 0.02 to 0.32.
>   The owner picked "Independent 1024". **The arm offered at Phase 5 is independent seeds at `N` 1024 through the filter**
>   (`G_local` p90 1.46 to 1.61, `G_seam` p90 1.46 to 1.66), with independent 512 filtered beside it as the cheaper step down.
>   The shared and lattice forms are dropped. Later phases' arithmetic at 512 doubles where it costs samples: Phase 8's
>   radius-1 ring reads about 2.6 s of bake at 13.5 ns, inside its 3 s bar but with less room, and Phase 9's arm's-length
>   ring takes about 46 frames to settle, not 23. Each phase re-reads its own.
> - **The RTX read 23.5 ns a sample at 300 000 at the hall**, against the 20 ns stop (the vein 15.5; at 1 000 000 a dispatch,
>   7.6 to 11.8). The owner picked "Go on, Phase 8 judges". The 23.5 is recorded as a reading, not a stop. Phase 8's
>   3 s first-frame bar is the time bar, and its dispatch size is chosen there.
> - **The glass blob is the composite, not the light.** With the lamp off, 44 % of the pane's pixels read over 250 at
>   exposure 8. Before the pane, 0.8 % of the same pixels did. The accumulated lighting at those pixels is unchanged.
>   The owner picked "Fix inside 0059", so the first amendment's "no cut fixes it" is withdrawn. **Phase 4a** fixes it
>   before the Phase 5 window, and Phase 5 looks at the pane.

> **Amended 2026-11-07 (owner), Phase 4a's glass bar.** Phase 4a's fix is at a2cb48a: translucent layers honour the lamp
> switch, and with the lamp off 10 of the pane's pixels read over 250 (`logs/0059.md`, Phase 4a). Done-when 2's model assumed
> one layer, but the pane is 2 voxels thick and its pixels enter 2 to 4 layers. The owner's answers:
> - **"k-layer model".** Done-when 2 is read against `(1 - a)^k` of what is behind plus each layer's own term, with `k` the
>   layers the walk counts. It is the same check at the pane's real thickness.
> - **Glass's own light: "Judge at Phase 5".** With the lamp off, glass has no store cell and no fallback, so a pane reads dark
>   over a dimmed scene. Phase 5 asks (h). A fault there comes to the architect, who then chooses between a store cell for
>   translucent faces and a fallback rule.
> - **The lamp's spot on glass at 60 cm: "Judge at Phase 5".** It is unchanged by the fix (0.436 of the pane over 250). Phase 5 asks (i).
> - **The stated test times stay** ("Keep, note the readings"). The filter test and the glass test read 3.55 s and 3.93 s
>   quiet, and 5.5 to 6.0 s on a series' first run or loaded. The readings are logged, and "under 5 s" is the quiet steady figure.

> **Amended 2026-11-08 (owner), Phase 5's window and glass.** The owner's words at the window (`live/session5*.sh`):
> "512 looks mostly fine, if it is cheaper"; "light table still needs time to settle". On the glass, which a rerun placed
> on the third try: lamp off, "looks very dark and consists of individual visible voxels"; lamp on, "looks like a white
> non transparent pane" (`live/session5b-glass-lamp-on.png`). So:
> - **Independent 512, filtered, stands.** Phase 6 onward cut from 512, and the first amendment's arithmetic at 512
>   holds again (Phase 8's radius-1 ring about 1.3 s of bake, Phase 9's arm's-length ring about 23 frames).
> - **(h) and (i) are faults with one cause.** The per-layer composite shades each glass voxel layer as a diffuse wall.
>   The owner picked "Both": a filter by path length plus a small baked reflected share, recorded as ADR-0047 (proposed).
>   The fix runs "Inside 0059, before Phase 6", as **Phase 5a** (dev), with **Phase 5b** (human) to look at it.
> - (a), (b), (c), (e), (f) and (g) were not answered by name. "Mostly fine" stands for (a) to (c). The rest are not
>   asked again.
> - **Amended 2026-11-08 (architect), Phase 5a's stop before code** (`logs/0059.md`, Phase 5a, a972410). Dev found five open
>   points. The answers, in Phase 5a and in ADR-0047, which is still proposed:
>   - `face_light` is computed in the hit pass, with the store bound read-only.
>   - `T` goes to the composite as rgb9e5 in a second target.
>   - The reflection is untinted.
>   - What is behind glass reads its store cell.
>   - Done-when 6 compares opaque faces by key in chunks holding glass.
>   - Done-when 7 is new.
>
>   These are the architect's calls, under the owner's standing rule that technical questions take the recommended option.

> **Amended 2026-11-08 (owner), exploration until the pick.** The owner: "we are moving very slowly with lighting atm...
> maybe it is more effective to skip tests while we decide what to do with lighting, and once result is more or less
> stable, fix/write tests on top". The architect's proposal, and the owner's word on it, "i agree". From Phase 5a through
> Phase 13's pick, these rules **override every remaining phase's Done when and stop rules**:
> - **The owner's eye is the gate.** A dev phase is done when its change is built, behind a key or flag, named in the
>   banner, and ready for the next owner window. Its numeric bars become readings, taken only when cheap, and a miss
>   is reported without stopping. Named lavapipe tests are written only where one costs little, and they do not gate.
> - **A dev stop is for a design choice the plan does not make, or for code that cannot work** (a device limit, a crash).
>   It is never for a bar.
> - **Two figures are kept as readings at every window,** the owner's targets: the first frame after a fresh start, against
>   3 s on the RTX, and the longest frame around a dig, against 33 ms.
> - **The checks per step are build, clippy, fmt and the quick tier,** with no GPU. The gpu tier runs once before the
>   merge into main, and at the close.
> - **Measure to diagnose.** A headless reading is taken when the owner's eye names a fault whose cause is not obvious,
>   as Phase 4's glass reading found the lamp switch.
> - **Bake small.** Windows use the radius-1 pieces at the vein and the patch. The whole showcase is re-baked only when
>   a window needs the hall.
> - **Tests come after the pick.** Phase 12 (budgets, the default) and the close write the tests and goldens against the
>   mechanism that stands, with the bars this plan names as their first draft.

> **Amended 2026-11-08 (owner), Phase 7's window: the brick level's resolution.** The owner's words
> (`live/session7.sh`; screenshots `live/session7-far-bricks.png`, `live/session7-far-table.png`, far from the spawn, chunk
> [18, 15, 3]), verbatim:
> 1. "I dont see a difference in ring or bricks - and view is rather blocky, quality is not enough (this is place far
>    away from spawn) ... light table are much closer to the look that should be (after settling)".
> 2. "digging with bricks, far away from spawn feels exactly as it should feel - instant settle and right light".
> 3. "more or less same. I think bricks looks the best, but it feels like resolution is too low maybe".
> 4. "hall looks the best with bricks".
>
> Far from the spawn, the rings and the bricks are the same store, the brick level, so (1) compares the brick level with
> the table. **The brick level is the direction:** a dig re-bakes in 22 to 39 ms, and the owner calls that settle right.
> Its look is not enough. The screenshot shows blotches the size of a brick, and two sources fit it:
> - **Noise:** 64 samples a brick direction, about 3 a face. A dig of about 12 000 faces drew 3.6 to 5.1 x 10^4 samples.
> - **Resolution:** one value per 8 voxels, bilinear in the face's plane.
>
> No reading has split them, so **Phase 7a** puts both axes under the key and the owner's eye picks. Phase 8 on waits for
> that pick. Ring choice (d) is moot until then.

> **Amended 2026-11-08 (owner), Phase 7b's pick: bricks 8/4096 everywhere; Part 2 re-read.** The owner's words at
> `live/session7b.sh`, verbatim:
> 1. "I believe that 8/4096 looks almost fine, and 5/512 is maybe acceptable" (read as 4/512).
> 2. "digging on 8/4096 feels fine".
> 3. "8/4096 is fine, others are too noisy".
> 4. "again, best at 8/4096".
>
> Asked whether bricks replace per-face light everywhere or only away from the camera, the owner chose "Bricks
> everywhere". So **the light is one store: an 8-voxel cell, 4096 samples a brick direction, the bilinear read of Phase 6,
> at every distance.** Phase 7a's readings on the RTX: a dig re-bakes in 72 to 88 ms, the hall bakes from nothing in
> 10.3 s and the level in 81.8 s, the store is 1.75 MB at the hall and 15.4 MB for the level, and the brick read costs
> about 2 ms a frame over per-face cells at rest (none on the Cezanne). Exploration mode holds to Phase 13. The remaining
> phases are re-read as follows; where a phase's text below disagrees, this block wins:
> - **The face cells, their rings, the seed forms and the samples cut are retired from Part 2.**
>   - The reference keeps its per-face path as the comparison oracle until Phase 14 decides whether it stays.
>   - The cut flags stay until Phase 14.
> - **Phase 8, the queue:** brick jobs only, at 8/4096. The chunk is the unit, ordered by distance and then view.
>   - **The CPU listing moves to the stream workers.** Phase 6 found it is most of the 14 to 82 s: 3168 chunks and the
>     brick rounds' bookkeeping.
>   - **No tiers, no tier-ups, no ring crossings.** A re-bake fades over `FADE` frames. A first value is written directly.
>   - **Eviction:** a chunk leaving the mirror frees its records.
>   - **Readings for the owner's fresh start, against 3 s:** the first frame, with the camera's chunk and its 26
>     neighbours baked before it. The arithmetic: about 26 ms a chunk at 4096, so 27 chunks is about 0.7 s. Beside it,
>     the seconds until the queue empties, on the RTX and on the Cezanne.
> - **Phase 9, the dig:** ring 1 is the touched bricks at 8/4096, spread over frames within the slice plus the edit cap.
>   - Read: the longest frame around a dig, against 33 ms, and the frames until the hole settles.
>   - The owner judged an unsplit 72 to 88 ms re-bake "fine", so the split is for the Cezanne and `B` presses. If it costs
>     more than a phase, it is reported, not built.
>   - Rings 2 and 3 (reach and break-through) stay as written, at brick level.
> - **Phases 10 and 11, the resolution cut:** the brick read does not change with the lighting resolution. So Phase 10 only
>   reads native against FSR 2 at 1.5 (frame passes and the fast yaw). Phase 11's window folds into Phase 13.
> - **Phase 12, budgets:** read as written. The temporal pass's "within 0.1 ms of the table's" becomes a reading: Phase 7a
>   read the brick read about 2 ms over per-face cells on the RTX, and slightly under the table. The Cezanne's lighting
>   line stays against 6.0 ms.
> - **Phase 13 adds** a fresh start and a walk on the Cezanne, beside the RTX.
> - **Phase 14 on "baked"** also removes the face-cell store, the rings, the seed forms, the cut flags and Phase 4's
>   readings code. It keeps a reference path only if Phase 12's tests need it as an oracle.
> - **ADR-0046** (proposed) records per-face cells over per-brick values. It is rewritten to the brick level at the close,
>   with Phase 7a's readings. The Data shapes and Budgets sections below are refreshed then too.

## TL;DR

Build the light as it should look before asking what it costs. **The reference** bakes every face
of every chunk in a bounded piece of the world around each of the window's poses, before the first
frame, until each face's own noise is under half a display level, however long that takes. (A
whole level would take about 15 h on the RTX, by Phase 1's census.) It renders at native resolution with full-resolution
lighting and no FSR 2. A pixel reads its face's value and draws nothing, so moving fast cannot
outrun the light and nothing moves at rest. A dug hole is re-baked in the frame it appears, at a
hitch nobody budgets. **The owner judges that in the window (Phase 3) before anything else is
built.** After the verdict come the cuts (antialiasing is parked, the owner's answer of 2026-11-06), one at a time:
samples, reach, resolution, each read against the reference and each shown to the owner beside it.
The queue, the rings and the budgets of the first draft are what the cuts are expected to need, and
they are built only once a cut asks for them. The table stays the default until Phase 12, and is
deleted only on the owner's verdict in Phase 13. The lamp and its bounce stay as Plan 0057 built them.

## Context & problem

`project-context.md` (Rendering, P0): dark scenes, emissive voxels as the light, stable at arm's
length, denoising that survives digging. The owner's rules (`LIGHT-FEEDBACK.md`, "What the owner
wants"): nothing changes while nothing moves; the light settles quickly after a change; the cost
matches what is on screen; a stable image comes first. **For this reference the owner has set
performance aside until after the verdict.**

**The owner, first window, 2026-11-05 (verbatim, `docs/plans/logs/0057.md`, Phase 6):** "there
is no flicker nether when i turn or go forward/backward. But there is visible visual artifacts on
places that I was not looking at when scene loaded initially, dark patches on some voxels"; "when I
move through caves, I see dark patches everywhere - it settles slowly"; "distand caves are too bad";
"preload doesnt really change anything - or it should be much much bigger. But it feels like this is
wrong way". And of Plan 0056's window: "once light settles it is not flickering anymore".

**The owner, second window, on the table's fix pass (verbatim, Phase 8):** the hall: "fast turn still
has artifacts, but better"; the old binary: "old is very bad, a lot of bark spots everywhere"; the
vein: "quite a few of dark patches when moving fast, but it seems they are gone after stabilization";
the patch: "maybe a little better"; the distant caves: "yess same problem with dark patches on distant
caves when moving fast"; `B` at the vein: "same, maybe a bit faster"; the hall with preload and `B`:
"mostly same, distant parts are with dark patches".

ADR-0046's Context holds the readings that decide the direction. In short: the store idea worked and
the view-driven fill did not.

**Why the sample count is the reference's first question.** The first draft priced grain as `82 /
sqrt(N)` display levels rms a face, whatever its brightness, so 4096 samples reads 1.3. That holds in
the hall and not beside emitters. Plan 0057's end readings at frame 4096, with every face at the
4096 cap (`logs/0057.md`, Phase 7, `G_noise` p50 / p90, blend off, "before" rows):

| Row | `G_noise` at 4096 samples | p90 at most 1 level needs, by `1 / sqrt(N)` |
|---|---|---|
| hall, native | 0.91 / 2.92 | about 35 000 samples |
| vein, native | 3.18 / 13.78 | about 780 000 samples |

Its fresh runs at the vein differ face by face by a constant (max 7 levels native), and done-when 5's
first frame missed at any pool with the blend on or off: dev's reading, untested, is estimator variance
beside the emitters. **No fixed count fits both**: 780 000 samples on every face is about 11 hours, so
the reference stops each face by its own spread (below), and Phase 1 reads whether that is enough
before the whole world is baked. The light pass's sampler also clamps a sample's luminance at `2 PI
lum(Le)` (`FIREFLY_CLAMP`, `light.wgsl`), which biases faces beside an emitter dark and exists to spare a
history from a firefly. A bake has no history, so Phase 1 reads it off.

**The reference's arithmetic** (Phase 1's readings, RTX 3080 Laptop at 5bb8c11, `logs/0059.md`; the
first draft's 4.6 ns a sample and surface share of 0.5 are refuted). At the kept arm a sample costs 36
to 64 ns in its own dispatch. Its shadow rays run 141 to 161 voxels on the level and 275 in the
showcase. The stop spends about 60 954 samples a face at the vein. The census, CPU, whole worlds:

| World | Chunks, surface share | Faces (p50 / p99 / max a surface chunk) | Store | At 60 954 samples a face |
|---|---|---|---|---|
| level, `LEVEL_CHUNKS` 20 x 20 x 8 | 3200, 0.990 | 21 316 662 (6 948 / 14 275 / 18 026) | 177 MB | about 15 h at 42 ns |
| showcase, `SHOWCASE_CHUNKS` 14 x 12 x 7 | 1176, 0.378 | 2 005 240 (4 047 / 13 117 / 14 663) | 17 MB | about 2.0 h at 60 ns |

**So the reference is a piece:** the `(2r + 1)^3` chunks within `--bake-radius r` of a pose's chunk.
The rest of the world is mirrored whole and left unbaked. At the level's p50, r = 1 is 27 chunks,
about 1.9 x 10^5 faces and about 8 minutes at 42 ns. r = 2 is 125 chunks, about 8.7 x 10^5 faces
and about 37 minutes. These are guesses from the census and the vein's stop mean, and an unlit face
stops sooner, at `N_MIN`. A piece is baked once per tree and pose, and written to a file the window
loads (Phase 2).

**The arithmetic the cuts rest on** (the first draft's, RTX 3080 Laptop and AMD Cezanne, derived from
Plan 0057's readings; every row a guess until read, **and none of it binds the reference**; Phase 1
refuted its 4.6 ns and its surface share of 0.5, above, and Part 2's rows are re-read at the verdict). `S` =
samples a second, a frame at 60 fps.

Cost of one surface chunk at the vein's density (6 900 faces, 187 surface bricks), 4.6 ns and about
50 ns a sample:

| Tier | Ring | Samples a chunk | RTX | Cezanne |
|---|---|---|---|---|
| brick level, 64 per brick direction, 2.5 directions a brick | 12 chunks | about 30 000 | 0.14 ms | 1.5 ms |
| face, 32 | 4 chunks | about 224 000 | 1.0 ms | 11 ms |
| face, 128 | 2 chunks | about 896 000 | 4.1 ms | 45 ms |
| face, 512 | 1 chunk | about 3 600 000 | 16.5 ms | 180 ms |

The loader's rate. Chunks entering a ring of radius `r` chunks at `v` chunks a second: `pi r^2 v`,
halved for a surface share of 0.5. Walking is the fly-through's 3 m/s (30 voxels/s, 0.47 chunks/s) and
Shift is the rig's 2 voxels a frame (120 voxels/s, 1.875 chunks/s):

| Ring | Surface chunks/s walking / Shift | Samples/s walking / Shift |
|---|---|---|
| 12, brick | 106 / 424 | 3.2 M / 12.7 M |
| 4, 32 | 11.8 / 47 | 2.65 M / 10.6 M |
| 2, 128 | 2.95 / 11.8 | 2.65 M / 10.6 M |
| 1, 512 | 0.74 / 2.9 | 2.65 M / 10.6 M |
| **sum** | | **11.1 M / 44 M** |

**On the RTX,** walking is 51 ms of GPU a second, **0.85 ms a frame**, under a 1.5 ms slice. Shift is
3.4 ms a frame over it: the near-first queue lets the far brick level lag, and a tier lags to the tier
under it, never to dark. **On the Cezanne** the full set is 9.3 ms a frame at 60 fps. Today it pays 4.2
to 6.9 ms in the light pass walking at 1280 x 720, so it gets a 4 ms slice with `N` and the brick
samples a quarter: at about 30 fps, 2.4 M samples/s against 2.8 M/s wanted at that quarter, a close
fit. **The light pass's emissive sampling leaves** (0.75 to 1.33 ms on the RTX in Plan 0057's readings
and window), so the net line moves little.

**The dig under a budget** (a 1 m carve, radius 10 voxels): about 3 000 faces with the survivors in its
bricks. 32 samples: 96 000, 0.44 ms RTX, 4.8 ms Cezanne, in the frame. Refined to 512 at arm's length:
1.5 M, 7 ms, about five 1.5 ms slices.

**The fresh start under a budget** at the vein's density: face rings 7.5 M + 15 M + 30 M samples (2
chunks at 512, 17 at 128, 134 at 32, nested rings not subtracted) and the brick level within 6 chunks
13.6 M (452 chunks): **about 66 M samples, 0.3 s RTX, 3.3 s Cezanne, about 0.8 s at its quarter of the
samples.** The brick level from 6 to 12 chunks is another 95 M, in slices: about 290 frames at 1.5 ms,
about 5 s at 60 fps, nearest first.

**The grain under a budget** by samples, independent seeds, `82 / sqrt(N)` levels rms per face (Plan
0057's derivation, which the vein's row above overshoots), and through the 3 x 3 tent at 0.375: 32:
14.5, 5.4; 128: 7.2, 2.7; 512: 3.6, 1.36. The p90 of a normal is 1.645 rms: 8.9, 4.5 and 2.2 through the
tent. Shared seeds aim below these and have no reading yet.

**The memory under a budget:** about 62 KB a near chunk (2 KB directory, 187 x 24 B brick values, 6 900 x
8 B cells) and 6.5 KB a far one. At the rings' residency, with face cells retained to 6 chunks: 452 x 62
KB + 3 167 x 6.5 KB, about 49 MB. A 128 MiB pool leaves retention room; `nfr.md` section 3's other line
is 1.5 GB.

## Decision

Build ADR-0046 from the far end: **the look first, the cost after.**

**The reference (Phases 1 to 3).** One store, one job, one read: every face a cell, every chunk of a
bounded piece around the pose baked before the first frame, each face to its own spread, native resolution, full-resolution
lighting, no upscaler. Nothing else is built ahead of it. **What leaves the front of the plan:** the
spike's seed arms and filter, its stop rule and cost bars, the budgets table, the brick level, the
rings, the queue, the fade and the worker hook. All of them answer "what does this cost", which the
owner has put after "does this look right". **What stays ahead of it:** one reading, the sample count
(Phase 1), because the reference cannot be called one until its own noise is under the eye's line, and
Plan 0057's vein row says a fixed count will not get there.

**The verdict (Phase 3)** gates everything below it. A "no" goes to the architect with the owner's
words, and the plan may end there: the table is untouched and stays the default.

**Antialiasing comes next after the verdict, in its own plan** (`docs/backlog.md`; the owner: "antializing
next"). The reference has no upscaler partly so that fast-turn artifacts can be attributed: what
survives with no upscaler and fully baked light is not the light store. Phases 4 onward wait for that
plan's window, and the resolution cut needs its answer in any case.

**The cuts (Phases 4 to 11), one at a time, each with the owner watching.** Samples, then reach, then
the machinery a walked and dug world needs once the bake leaves the pre-roll, then resolution. Each cut
is a setting read against the reference and shown beside it, and the owner can refuse it. Numbers in
these phases come from the first draft and are re-read at the verdict.

**The default and the deletion (Phases 12 to 14):** as first drafted. The table is deleted only on the
owner's verdict.

**Digging in the reference.** The owner's requirement that a dug hole settle quickly is in it: the box
and the bricks within 32 voxels (ADR-0046's rings 1 and 2) re-bake in the frame the edit reaches the
mirror, to the reference's stop, ahead of the read, so the hole is final on its first frame. The cost is
a hitch, printed, not budgeted: at Phase 1's rate, a 1 m dig at the vein takes about half a minute (a guess:
about 11 000 faces at the stop's mean and 42 ns). **Deliberately out of the first look:** ring 3. Beyond 32 voxels a face
keeps its value after a dig, so light through a new opening does not reach a far room, and a far wall
lit by a dug-away emitter stays lit. The HUD says so, the runner says so, and the owner does not judge
that here. Re-baking every face in reach (about 1.6 million) is about 4.6 minutes a dig even at `N_MIN` (at Phase 1's 42 ns), and it would
make every dig a wait for a fault that is the budgeted queue's to answer (Phase 9).

**The lamp and the bounce stay as built**: the composite adds the store's value to the lamp's part and
the gather, and the bake never sees them.

**Of Plan 0057's Phase 7, which its Followups ask this plan to state:** the table's size and age-out are
moot under a store, and go with the table; the blend is kept in form as the bake's filter, a cut in
Phase 4 and absent from the reference; the widening goes, since the brick level is what would show where
a wall is young, if the reach cut brings it.

We rejected: the budget-first order this plan first had, cutting down from the table, and a still
offline reference. Their reasons, and those for the view-driven table alone, a larger radius preload,
screen-space history, per-brick irradiance with occlusion, a CPU or a split bake, the count alone,
emitter-out rays, re-baking whole chunks at an edit, keeping the guard, the table as a not-ready cache,
and fixed-point shading, are ADR-0046's.

## Architecture diagram

The reference. A cut adds a level to the pool (Phase 6) or moves the job onto a queue (Phase 8).

```mermaid
flowchart LR
    subgraph vxe-world
        S[bounded world, mirrored whole]
        ED[edit result: touched box]
    end
    subgraph vxe-render
        S --> LI[list a chunk's faces]
        LI --> J[bake job: rounds of samples at unit emission, a candidate table per chunk, stop by each face's spread, rgb9e5]
        ED --> R[rings 1 and 2: re-list and re-bake in the frame]
        R --> J
        J --> P[(pool: chunk directory, face cells)]
        F[(store file)] <--> P
        P --> TP[temporal pass: face cell x scale now x exposure]
        L[light pass: lamp only, and the fallback's sample] --> TP
        TP --> AT[a-trous: lamp part only] --> C[composite, no upscaler]
        PU[lamp channel: as Plan 0057] --> C
    end
```

## Implementation phases

Each phase ships as its own commit. `dev` implements the dev phases in order and stops at each `human`
phase for the owner's window. The architect reviews once at the end, in a fresh session, **and re-reads
Phases 4 onward by an amendment at Phase 3's verdict**. Lane rule 9c applies: exploratory runs on the
RTX 3080 Laptop by `VXE_TEST_ADAPTER=rtx`, labelled; asserted readings on lavapipe. **Tables and
attempts go to `docs/plans/logs/0059.md`**, from the readings-log template.

**The gates.** Per phase: `node scripts/test-changed.mjs --profile quick`, then `--profile gpu` under the
perf lock, clippy, fmt. At the tip: `cargo nextest run --workspace --profile gate` under the perf lock,
doctests, clippy, fmt, deny, and the doc and tier gates. A new test file that opens a device joins
`.config/nextest.toml`'s gpu filter. `soak` and `full` only if the owner asks.

**Test times.** On lavapipe a hand-built box (`cache_box`) runs 2 to 3 s and the level loads in about 5 s.
A bake's times below assume a box of under 2 000 faces and test-set sample counts. A test over 30 s is
born on the soak list and named in its phase. **A test's stated time is a quiet machine's** (one-minute load
under about 2, the test alone or in the tier). It is not a gate bar, because the gate's line is `docs/nfr.md`
section 7's 30 s. Ruled at the 2026-11-05 amendment: `a_baked_view_is_final_on_its_first_frame` read 3.7 s alone
and 5.8 s in the gpu tier, against "under 5 s". `a_moving_camera_reads_what_a_still_one_reads` read 6.9 s alone
and 13.2 s in the tier, against "under 10 s". Both runs were at load 12.4, so both tests stay in the gpu tier
and off the soak list, and no stated time moves. The plan's tip gate reads them again on a quiet machine with
`node scripts/test-times.mjs`. A quiet reading over the stated time goes to the architect, which corrects the
time by an owner-visible amendment. A reading over 30 s moves the test to the soak list.

**A long bake.** A bake holds the GPU at full load for minutes, on a laptop whose RTX has fallen off the
bus under load (2026-10-16). It runs under the perf lock and only with no other GPU work, in submits of
at most 10 M samples (about 0.4 s on the RTX at Phase 1's 42 ns) with a wait between, and it prints
progress. Nothing bakes a whole world in a test, and no phase bakes a whole level.

## Part 1: the reference

### Phase 1: The reference bake, and the count it needs
- **Owner skill:** dev
- **What:** the store, the job and the read: a still view lit from frame 1 through `--light baked`, on a
  world small enough to wait for, and the reading that sets the stop. The table stays the default and no
  golden moves.
- **The list.** A function of a chunk's opaque masks and its six neighbours' borders gives each surface
  brick's sorted face keys. It runs inline on the render thread for now.
- **The pool.** One storage buffer under `Light::Baked` only (`Lighting::bake_pool_bytes`, 256 MiB by
  default, clamped to the device's storage binding as `table_cells_within` does; `--light table` allocates
  a minimal one, so `frame_bytes` at the default does not move). A window-indexed table gives a chunk's
  block; the block holds a 512-entry directory (a `u32` per brick: run offset and face count, 0 for no
  surface), then each surface brick's run of cells (8 B each, sorted by key). **No brick level and no
  `BRICK_FACES_MAX`:** a run may hold every face a brick can expose, 1536 in a checkerboard, so the count
  takes 11 bits. The light pipeline's storage bindings are full (Plan 0057's resume notes), so the job and
  the read bind the pool where they have room, or it shares a binding with the table's buffer; dev says
  which in the log.
- **The bake job.** Independent seeds, keyed by face key and sample index. Unit emission. The candidate
  table per job (ADR-0025's weights about the job chunk's box, every emitter chunk within `light_radius` of
  it, up to its 1024 entries) and the window `(1 - (d / L)^4)^2` on each sample, `L` being `light_radius` in
  voxels from the face, at each scene's `light_radius` as the fly-through runs it (4 chunks on the level, 19
  in the showcase). Rounds of `BAKE_ROUND` 1024 samples a face, a face's sums and its luminance's second moment
  kept in a working buffer (`BAKE_BATCH_FACES`, 65 536 faces a batch; dev states its layout). No filter, no
  shared seeds. **The stop:** a face ends when its standard error through the display transform at exposure 8
  (`rigs.rs`'s `tone_bytes`, the runner's fixed exposure) is at most `BAKE_SE` 0.5 levels, not before `N_MIN`
  4096 and not after `N_MAX` 2^20. Then `rgb9e5`, and the dominant material as the emitter of the face's
  largest single sample. The stop depends only on the face's own samples, so a result does not depend on
  batching.
- **The read.** With `Lighting::light` at `Baked`, the temporal pass reads a cacheable hit's cell by chunk,
  directory and a binary search of the run; else 0, counted in `store_missing` for the HUD. It multiplies by
  the dominant material's scale and the exposure. The light pass draws no emissive sample and casts no emissive
  shadow ray for a cube face. A no-face hit keeps the fallback. `--light baked|table` shows in the banner and
  HUD with `store <chunks> <MB> missing <pixels>`.
- **Pre-roll.** `Frame::bake_until(extent)`, batches submitted and waited on, progress through a callback. The
  fly-through calls it before its first frame under `--light baked`, with a stdout line. Under this phase a chunk
  outside the extent is simply not baked.
- **The count reading** (`bake_count_readings`, ignored, RTX, `apps/flythrough/tests/bake.rs`). All faces within
  96 voxels at `VEIN_POSE`, its `back` view, the patch and the hall (the census's 98 025, 47 242 and 61 066 at
  the vein, patch and hall; 262 144 samples on 98 025 faces is 2.6 x 10^10, about 2 minutes), against a
  reference of that scene at the deepest count run:
  - **Fixed counts,** `N` of 4096, 16 384, 65 536 and 262 144, two seed sets, `params.candidates` 8 and 32,
    the firefly clamp on and off: `G_noise` and `G_plane`, p50 and p90, by the distance from the face to its
    nearest emitter (under 4, 4 to 16, 16 to 64, over 64 voxels) and by decile of brightness; the mean bias by
    decile against the deepest run; ns a sample in a separate dispatch with the shadow rays' mean length.
  - **The stop,** at the kept arm: the samples spent, the faces at `N_MIN` and at `N_MAX`, and the same
    `G_noise`.
  - **The census,** CPU: faces per surface chunk (p50, p99, max), surface share, and faces whose reference reads
    0, for the whole level and the whole showcase, and the bytes the store takes.
  - **The kept arm:** the fewest samples whose `G_noise` is at most **p50 0.5 and p90 1 level at all four
    views** (a standard error of 0.5 reads about p50 0.34, p90 0.82), the clamp off unless its bias is under a
    level in every decile.
- **The stop rule.** Dev stops for an architect amendment, with the table, if no arm meets the bars at `N_MAX`, or
  if the kept arm projects a whole level over 60 minutes on the RTX. Everything short of that is the reference
  simply taking its time.
- **Files touched:** `crates/vxe-render/src/bake.rs` (new), `shaders/bake.wgsl` (new), `shaders/temporal.wgsl`,
  `light.wgsl`, `march.wgsl`, `marcher.rs`, `gpu_world.rs`, `lib.rs`, `apps/flythrough/src/main.rs`, `hud.rs`,
  `apps/flythrough/tests/bake.rs` (new), `.config/nextest.toml`, `docs/plans/logs/0059.md` (new).
- **Done when:**
  1. `every_exposed_face_has_one_key` (no GPU, under 1 s): a seeded mixed chunk with seeded neighbours lists
     exactly the faces a brute-force scan finds toward a non-opaque voxel, across borders, each once, sorted.
  2. `a_bake_does_not_depend_on_its_slicing_or_order` (lavapipe, under 10 s): `cache_box` baked at test counts
     in one batch, in batches of 7 faces, and with its jobs reversed reads bit-identical cells.
  3. `a_wall_reads_its_analytic_light` (lavapipe, under 5 s): an unoccluded wall under one emitter, at a distance
     under `L / 4` (the window then within 0.8 %, arithmetic: `2 (1/4)^4`), a fixed 4096 samples, reads within 3 %
     of the closed form times the window on every face, with the clamp off, and at twice `light_radius` from the
     emitter reads 0.
  4. `a_baked_view_is_final_on_its_first_frame` (lavapipe, under 5 s, scale 1, no jitter, lamp off): after
     `bake_until`, frame 1 and frame 64 are bit-identical, frame 1's emissive equals the cell's value times the
     exposure within one `f16` step, the light pass reports 0 emissive samples, and `store_missing` reads 0.
  5. `the_stop_spends_where_the_noise_is` (lavapipe, under 10 s; `N_MIN` 64, `BAKE_ROUND` 64, `N_MAX` 4096): on a box
     with an emitter, a face beside it, a face far from it and a face no emitter reaches: the unlit face holds
     exactly 64 samples, the face beside the emitter holds more than the far one, and every face ends with its
     standard error at most `BAKE_SE` or at `N_MAX`.
  6. `a_moving_camera_reads_what_a_still_one_reads` (lavapipe, under 10 s, scale 1, no jitter, lamp off): after a
     bake, a frame reached by a 90 degree yaw in 2 frames is bit-identical to a fresh `Frame` at that pose, on every
     pixel with a face. **This is the owner's "moving fast" as a property.** With the table in the same arm the test
     fails first.
  7. **The count reading's table in the log,** with the kept arm, or stop, in one line, and the cost rows named with
     machine, commit and load.
  8. `the_banner_names_the_light` (`hud.rs`, no GPU, under 1 s). With `--light table` the goldens and `frame_bytes`
     pass untouched.
  9. `the_bake_radius_flag_sets_the_extent` (`main.rs`, no GPU, under 1 s): `--bake-radius N`, under `--light baked` only, bakes
     the chunks within `N` of the eye's chunk and is refused without it; unset, the pre-roll takes the whole extent. The runner
     `live/session1b.sh` is written.

### Phase 1a: The stop's submits
- **Owner skill:** dev
- **What:** the stop's path made about as fast as a fixed count's, so that a piece bakes at the dispatch rate. Phase 1's stop
  ran at 310 ns a sample, against 42 ns in a fixed run at the same view, because a chunk's last faces each took one waited submit
  a round. This is a dev efficiency change, and it moves no bar, constant or arm.
  - **Batches across chunks.** A submit is filled with the live faces of as many jobs as fit under `SUBMIT_SAMPLES`, rather than
    with one chunk's tail. Each face still samples its own job's candidate table. Dev states in the log how: tables for several
    jobs in one buffer indexed from the face, or the jobs grouped by table.
  - **Several rounds a submit.** A submit runs `k` rounds of each live face before its readback, `k` being dev's choice and
    stated. The stop is still decided round by round from each round's sums. A round past a face's stop inside a submit is
    thrown away and counted as waste.
  - **The rule it keeps.** A face's stop and its cell depend only on its own samples. The round it stops at, its samples and its
    cell are what one face alone at one round a submit would give.
- **Files touched:** `crates/vxe-render/src/bake.rs`, `shaders/bake.wgsl`, `apps/flythrough/tests/bake.rs`,
  `docs/plans/logs/0059.md`; the runner `live/session1b.sh` (untracked).
- **Done when:**
  1. `a_bake_does_not_depend_on_its_slicing_or_order` (lavapipe, under 10 s) also bakes `cache_box` to a test stop
     (`the_stop_spends_where_the_noise_is`'s constants) four ways: at one round a submit, at several, with all its jobs in one
     submit, and with each job alone. The four read bit-identical cells and bit-identical samples a face.
  2. **The stop's rate on the RTX** (`bake_count_readings`'s stop part, seed 0, the vein's faces within 96 voxels, exclusive
     perf lock, about 5 minutes): the wall time over the samples kept is at most 1.25 times the rate of a fixed 16 384-sample
     c32 run of the same faces in the same session (Phase 1 read 42.1 ns at 65 536, so about 53 ns). The waste share (samples
     thrown away over samples kept) is read beside it. If the stop's arithmetic is unchanged, the samples spent and the faces at
     `N_MIN` and at `N_MAX` equal Phase 1's seed-0 row exactly (5.9696 x 10^9, 3698 and 460). If dev changes the arithmetic, the
     log says how, and the stop's `G_noise` at the vein, with both seeds, meets the bars again.
  3. If the rate misses 1.25 times, the log records the miss and its cause. Phase 1b still runs if its `--bake-radius 1` launch
     at the vein projects under 20 minutes at the measured rate. Otherwise the miss goes to the architect.
  4. `live/session1b.sh` runs every stop at `--bake-radius 1`, and its first lines print the projected minutes of a launch at
     the measured rate.

### Phase 1b: The owner's first look
- **Owner skill:** human
- **What:** a short window on Phase 1a's tree, on the RTX, with stdout visible and a ping when ready. It runs at the settings
  Phase 2's `--reference` will name, set by hand: `--light baked --internal-scale 1 --upscaler off --lighting full --far-normal 1,2`,
  no jitter, at Phase 1's kept arm, with `--bake-radius 1` (the owner, 2026-11-05: "Fix submits, radius 1"). That bakes at least 64
  voxels around the eye in every direction, and up to 128. At the level it is 27 chunks, about 1.9 x 10^5 faces at the census's p50,
  so about 8 minutes a launch at the stop's 60 954 samples a face and 42 ns. That figure is a guess, and the runner prints the
  projection at Phase 1a's measured rate. There is no bake file yet, so each launch waits for its bake and prints its progress. Stops: the vein, the patch and the
  hall, each standing, then turning and walking a little, the lamp off and on, and the mode key against `--light table` if Phase 1
  has one. **The runner says first:** this is a small piece of the world, so beyond it faces read dark and `store_missing` counts
  them (that is the unbaked edge, not the light); silhouettes are not antialiased; there is no dig yet; the verdict is Phase 3's.
- **Files touched:** this plan's log.
- **Done when:** the log records the owner's words on: (a) the flat, settled look of a face at rest; (b) grain, above all beside the
  vein; (c) dark patches inside the baked piece while turning; (d) **whether Phase 2 should go on.** Only "no" stops the lane: it
  goes to the architect with the words and Phase 1's table. A remark on the look does not stop it, and Phase 3 still asks every question.

### Phase 2: The reference: a piece to walk and a hole to dig
- **Owner skill:** dev
- **What:** the bake of a bounded piece around each of the window's poses, the file that spares the wait, the dig, and the
  settings the window runs under.
- **The piece** (amended 2026-11-05, the owner: "Bounded piece, bars kept"). `--reference` bakes the chunks within
  `--bake-radius` of the pose's chunk, with a default of 2 under `--reference` (125 chunks, about 37 minutes at the vein at
  Phase 1's rate, a guess). An explicit radius overrides the default. The world is still mirrored whole, and a piece's job samples
  every emitter within `light_radius` of its chunk, whether it lies inside the piece or not. So a piece's cell is the cell a
  whole-world bake gives that face, and `light_radius` limits the far light at a piece's edge exactly as it does anywhere else. A
  chunk outside the piece has no block: its hits read 0, count in `store_missing`, and are the **unbaked edge**, not the light.
  The HUD's store row names the piece (`piece r<radius> at <chunk>`). The window's files are the vein, the patch and the hall,
  the hall's being in the showcase. Dev may serve two poses from one file if its piece covers both poses' radius. **The whole
  showcase** (the owner's "and/or"): after the hall's piece is baked, dev projects the whole showcase from that piece's samples a
  face and its rate. If the projection is at most 60 minutes, the hall's file is the whole showcase. If it is over, the hall's
  piece stands and the log says so. The owner kept this 60-minute rule over a one-time bake of up to 3 hours (2026-11-05).
- **The stop rule, restated for the piece.** Dev projects each file at Phase 1a's rate and the census's faces before baking it.
  A file that projects over 60 minutes on the RTX goes to the architect before it is baked. So does a miss of the `G_noise`
  bar below.
- **`--reference`** (fly-through): `--light baked`, `--internal-scale 1 --upscaler off`, `--lighting full`, `--far-normal 1,2` (the owner's pick in Plan 0056's window; the fly-through's default stays `[2, 6]`
  until the far-normal plan), no
  jitter, the clamp and candidates of the kept arm. The banner names it. It takes `--level` and `--source showcase`,
  and refuses a streamed world with a message naming this plan's Phase 8: a streamed world has no "every chunk".
  A free key (dev's choice; the banner and the HUD name the mode) swaps the temporal pass's read between the store
  and the table in place, so the owner compares one pose.
- **The store file.** `--bake-file PATH` writes the pool after a bake and loads it instead of baking when its header
  matches: the world's seed and extent, the piece's centre chunk and radius, `light_radius`, the arm and stop, and the
  `bake.wgsl` source's hash. It holds the piece's blocks only. Otherwise
  it refuses with a message naming the field. Files under `live/bake/` (gitignored). This is the testbed's, for a
  window and for Phase 4's comparison; a product's bake on disk stays out of this plan.
- **The dig.** On `Frame::invalidate`'s box, in the frame the mirror receives the edit and ahead of the temporal pass: the
  bricks of the box dilated by one voxel and those within 32 voxels of it are re-listed, their blocks rebuilt, and their
  faces baked to the same stop with `N_MAX` 65 536, by the same submits and waits, inside the frame. The emitter ranges
  and each job's candidate table are refreshed first, as `sync` gives them. A frame's several boxes are merged by brick.
  Nothing beyond that ring moves. The ring is clipped to the piece: its bricks in a chunk outside the piece gain no block, and
  a dig wholly outside the piece re-bakes nothing and prints `dig outside the baked piece`.
  **The hitch is printed per dig on stdout** (faces, samples, ms) and shown on the HUD.
  The HUD's row says `far light stale after a dig` once one has happened.
- **A scale change** moves nothing: the read multiplies by the dominant material's scale now.
- **The readings** (`reference_readings`, ignored, RTX, exclusive perf lock, the window's 2530 x 1378 native, level at the
  vein and patch, showcase at the hall, each from its file; no bars but the first):
  - **`G_noise` at Phase 1's four views, on each view's faces within 96 voxels of its eye that lie inside its file's piece: the
    file's cells against a second seed's stop bake of the same faces, p50 at most 0.5 and p90 at most 1 level.** A miss goes to
    the architect.
  - Each file's pre-roll: minutes, samples, samples a face, faces at `N_MIN` and at `N_MAX`, ns a sample, and the file's bytes.
    Beside it, for Part 2 and with no bar, the whole level and the whole showcase projected at that rate.
  - A strip beside the table's frame 4096 of the same tree: the share within 1, 2 and 4 levels, `G_plane`, and the back view's
    dark share of hits beyond 64 voxels (the table's frame 1: 0.4389 under FSR 2, frame 64: 0.0180), over the hits inside the
    piece, with the share of hits outside it beside.
  - The rig's fast yaw (`the_fast_yaw_excess_by_table_read`'s row) at the reference with the lamp off and on, beside the table's
    at native and at FSR 2. The rig's flight at 30 and 120 voxels/s, on the part of its path whose eye stays inside the piece:
    `store_missing` per frame as the piece's edge, and the same counts. **What remains here is not the light store, and the
    antialiasing plan starts from it.**
  - The frame's passes at rest and in flight, so the owner knows the frame rate the window will show. If the mean frame is over
    33 ms the runner says so and offers the owner a smaller window, since a low rate could read as a motion fault.
  - The dig: a 5-voxel dig, a dig at 100 voxels and each of the four `B` presses at the vein (78 to 87 carves): faces, samples, ms.
    The runner states each step's measured hitch before it. A `B` press over 5 minutes leaves Phase 3's `B` step at one press,
    the lightest, unless the owner asks for more. A dig keeps the reference's stop: the owner chose this over a fixed 4096
    samples for dug faces and over dropping `B` (2026-11-05).
  - Glass at rest: a pane placed by the key below at the hall, 600 still frames, the lamp off and then on: the pixels of the
    accumulation that changed from first to last, and the composite's patches a step on the tiles the pane fills. No bar.
- **The glass pane.** Under `--reference` only, a free key (dev's choice; the banner names the panes placed) stamps a pane of the
  castle's glass row, 2 voxels thick and about 40 x 40, 6 voxels ahead of the eye, on air voxels only, through the world's edit
  ops. A pane that would reach outside the piece is refused, and the banner says so. It goes through the dig's re-bake, so no
  bake file changes. An unused glass row in the reference's material table changes
  no frame until the key is pressed. No key removes a pane.
- **Files touched:** `crates/vxe-render/src/bake.rs`, `shaders/bake.wgsl`, `shaders/temporal.wgsl`, `marcher.rs`, `gpu_world.rs`,
  `lib.rs`, `apps/flythrough/src/main.rs`, `hud.rs`, `apps/flythrough/tests/bake.rs`, `.config/nextest.toml`, `.gitignore`,
  `docs/plans/logs/0059.md`.
- **Done when:**
  1. `a_dug_hole_is_final_on_its_first_frame` (lavapipe, under 15 s, test counts, scale 1, lamp off; a lit wall, a radius 3 carve):
     on the first frame the hole is on screen, every cell in the ring is bit-identical to a whole fresh bake of the edited world (the test's `N_MAX` the same for both), and
     that frame and frame +64 are bit-identical.
  2. `a_dug_emitter_leaves_the_near_ring_at_once` (lavapipe, under 15 s): carving the box's one emitter brings every cell within the
     ring to exactly 0 on that frame, bit-identical to a fresh bake.
  3. `the_far_light_stays_as_the_reference_states` (lavapipe, under 15 s): a wall between an emitter and a far room, the room beyond 32
     voxels of the carve: after a window is carved, the far wall's cells are bit-identical to before and the HUD string names it. This test
     defends a stated limit, and Phase 9 replaces it.
  4. `a_scale_change_moves_no_bake` (lavapipe, under 10 s): `Frame::set_emission_scales` at 0.5 halves a lit wall's displayed irradiance on
     that frame, within one `rgb9e5` step before the display, with the cells unchanged; a material at 0 at bake time reads its light when
     raised.
  5. `the_lamp_adds_over_the_bake` (lavapipe, under 10 s): with the lamp on, a frame's emissive part equals the lamp-off frame's and the
     cells are unchanged; off again, the frame is bit-identical to the first lamp-off frame after the lamp's history empties (the test states the frames).
  6. `a_store_file_loads_the_same_light` (lavapipe, under 10 s): a written file loaded into a new `Frame` gives every cell and a frame
     bit-identical, and a file with another seed, piece, `light_radius` or stop is refused with the field's name.
  7. `the_reference_flag_sets_its_look_and_refuses_a_streamed_world` (with its default bake radius of 2) and the mode key's banner
     (`hud.rs` and `main.rs`, no GPU, under 1 s).
  8. The readings above in the log, the `G_noise` bar met, and the runner `live/session3.sh` written with its files baked.
  9. `the_glass_key_places_a_pane_under_the_reference` (`main.rs` or `hud.rs`, no GPU, under 1 s): the key's edit returns the
     pane's voxel count in the glass row and changes no solid voxel, the banner names the pane, and outside `--reference` the
     key does nothing.
  10. `a_piece_reads_what_the_whole_world_reads` (lavapipe, under 15 s, test counts): take a box of at least two chunks whose
      emitter lies outside a one-chunk piece. The piece's cells are bit-identical to a whole bake's, the chunk outside has no
      block, and its hit pixels count in `store_missing`. A carve that straddles the piece's edge re-bakes only cells inside it,
      bit-identical to a fresh piece bake of the edited world, and creates no block outside.

### Phase 3: The owner's verdict on the reference
- **Owner skill:** human
- **What:** the window, on Phase 2's tree, from the files, on the RTX; the orchestrator runs it with stdout visible and pings when ready. The
  owner's steps of Plan 0057's second window, with the mode key in each: the hall, the vein, the patch, the distant caves down the
  passage, moving fast, turning fast, standing; the lamp on and off; a dig at arm's length; then `B` at the vein; then, at the hall, a glass
  pane placed by Phase 2's key, standing still with the lamp off and on. **The runner says first:**
  silhouettes are not antialiased (that is next), a dig hitches (nothing is budgeted, and each step's measured hitch is stated before it),
  and beyond 32 voxels a dig does not reach. Each pose is a baked piece of the world, launched from its file, and moving between poses is a
  relaunch. Beyond the piece, faces read dark and `store_missing` counts them: that is the unbaked edge, not the light. The loading wait is
  the file's, and the real bake's minutes are printed beside it.
- **Files touched:** this plan's log.
- **Done when:** the log records the owner's words, with the strips beside, on: (a) dark patches while moving fast and in the distant caves inside the piece,
  the second window's complaint; (b) anything moving at rest, Plan 0057's unanswered (a); (c) the grain at rest, its unanswered (d);
  (d) the fast turn: **what survives here is not the light store**, and goes to the antialiasing plan; (e) a face's flat value at a
  voxel step and at a shadow edge, with no filter, against the owner's "nearest"; (f) the lamp and its bounce over the bake; (g) the dug
  hole at arm's length, and the hitch; (h) `B` at the vein; (i) glass at rest: does the pane or what shows through it move while nothing moves? A "yes" takes
  `docs/backlog.md`'s "A held fallback still moves on screen" to the architect with the words and Phase 2's reading, and does
  not by itself decide (j); (j) **the verdict: is this the look to cut down from?** A "no" names the step and goes
  to the architect with the words, and nothing below runs. **Nothing in Part 2 starts before a "yes".** A "yes" hands to the architect, who drafts
  the antialiasing plan from `docs/backlog.md` and re-reads Part 2.

## Part 2: the cuts (after the verdict; re-read 2026-11-06 around the owner's load and dig targets)

**The rule of every cut.** A setting, one at a time. The headless read is the cut's distance from the reference, on the same poses, as the share
of pixels within 1, 2 and 4 levels, `G_plane`, and `G_noise`, beside what it saves. The owner sees the cut and the reference at one pose, from
two store files, by a key. **A bar a dev phase reads does not overrule the owner's eye.**

### Phase 4: The samples cut, the shared seeds and the filter (read against the reference)
- **Owner skill:** dev
- **What:** what Phase 1 of the first draft read, now on the reference's own job and against the reference instead of against arithmetic.
- **The grain.** At the vein's `back` view, the patch and the hall, all faces within 96 voxels, `N` of 32, 128, 512 and 2048, filter on and off, seed
  arms: **independent** per face; **chunk-side**, one sequence per chunk and side; **global-side**, one sequence per side for every chunk;
  **lattice**, one sequence offset per face by a low-discrepancy step of its plane coordinates. A second seed set for every arm. The figures, through
  the display transform at the pose's fixed exposure (`rigs.rs`'s `tone_bytes`):
  - **`G_local`**, p50 and p90 over coplanar neighbour pairs `(f, n)`: `|(D_f - D_n)_A - (D_f - D_n)_B| / sqrt 2`, A and B the two seed sets. This is the
    grain the eye sees, with the true gradient cancelled, and it is what shared seeds should lower.
  - **`G_seam`**, the same over pairs that cross a chunk border.
  - **`G_patch`**, p90 of `|mean_A - mean_B| / sqrt 2` over 8 x 8 coplanar patches: the slow error that shared seeds leave.
  - **The dark share**, faces whose reference is over 1 level and which read 0.
  - Mean bias by decile against the reference, with the deciles' spread beside it.
  - The filter's harm: at a flat wall's shadow edge and a crease in the level, the values across it, in levels.
- **The filter.** The tent: 3 x 3, the kernel of Plan 0057's Part B (same axis and sign, the coplanar voxel first, then a voxel toward the air, then one into
  the rock, stepped neighbours at half weight, no material test), across the job's cells, a brick's run and a neighbour chunk's stored values at border faces.
- **The cost.** ns a sample in a separate dispatch of 1 000, 10 000, 100 000, 300 000 and 1 000 000 samples, on the RTX and the Cezanne, at the vein and the
  hall, with the shadow rays' mean length (Plan 0057 read a floor of about 0.4 ms for a separate pass at 192 x 108, and the 4.6 ns was read inside the light
  pass near the camera). One read of a CPU sampler over `World::raycast` on one worker thread, ns a sample, for ADR-0046's Alternative F. Whether the kept
  arm's spread stop still earns its second moment at these counts.
- **The glass blob.** At the hall with Phase 2's pane, at the runner's exposure and at manual +4 EV, the lamp off and then on: the pane's pixels over
  250 in the display and their centroid against the lamp cone's axis. The log names what the blob is, and Notes route it as the amendment of 2026-11-06 says.
- **What goes to the owner.** The arm that is the cheapest to meet, at all three poses with the filter on: `G_local` p90 at most 2 levels and p50 at most 1 at
  512; p90 at most 5 at 128 and at most 10 at 32; and `G_seam` p90 at most 2 at 512. Ties go to chunk-side. Beside it, the next arm up in samples, and
  `N` 1024 independent through the filter (the first draft's fallback). **Dev stops for an architect amendment, with the tables, if:** no arm meets it; the RTX
  reads over 20 ns a sample at 300 000 at any pose (Phase 1a read 13.5 at the vein's stop); or the CPU reads under 100 ns a sample.
- **Files touched:** `crates/vxe-render/src/bake.rs`, `shaders/bake.wgsl`, `apps/flythrough/src/main.rs`, `hud.rs`, `apps/flythrough/tests/bake.rs`,
  `.config/nextest.toml`, `docs/plans/logs/0059.md`.
- **Done when:**
  1. The log holds the grain figures for every arm, `N` and pose against the reference, the cost rows on both adapters, and one line: the arm offered, or stop.
  2. `the_seed_forms_differ_as_named` (lavapipe, `cache_box`, under 5 s) asserts that on a wall lit through a slit, with `N` 16: under chunk-side, two coplanar
     faces of a chunk that see the same emitters read closer to each other than under independent seeds, by `G_local` measured on the box, the independent arm
     being the failure case.
  3. `the_filter_stops_at_a_crease_and_softens_a_shadow_by_one_face` (lavapipe, under 5 s; Plan 0057's arithmetic): an unlit wall's row along an L crease reads
     exactly 0, and on a flat wall half under a slab the face two from the edge on the dark side reads exactly 0 and the face beside the edge between 0.15 and 0.35
     of the lit face beside it.
  4. Store files for the offered arm at the window's poses, and the mode key cycles the reference and the cut, its name in the banner and HUD.

### Phase 4a: The glass composite (amended 2026-11-06, owner)
- **Owner skill:** dev
- **What:** the translucent row's composite brightens what is behind it.
  - At the hall's shot, the pane is 0.6 opacity, albedo 0.25 0.35 0.85, with no emission. The pixels behind it read luma
    p50 24 before the pane and 146 with it, while the lighting accumulated there is the same (Phase 4's glass reading).
  - A blue glass at 0.6 cannot brighten what it covers above its own lit face. Measure first:
    - At the same pixels, read each part the composite sums for a translucent hit: the glass face's own light (store or
      fallback, and which of the two), its weight, the transmitted hit's light and its weight, and any term outside both.
    - Read them before the display, at the hall's shot and at the owner's screenshot pose (eye in chunk [2, 5, 3]).
  - The log names the part that carries the excess. Then fix that part in the composite or in the face's light lookup.
  - If the fix needs a choice between mechanisms (for example, a store cell for translucent faces against a fallback
    rule), dev stops with the readings for an architect amendment.
  - No other material's composite changes.
- **Files touched:** the shader that composites translucent hits under `crates/vxe-render/src/shaders/`, `crates/vxe-render/src/`
  as the reading finds, `apps/flythrough/tests/bake.rs`, `docs/plans/logs/0059.md`.
- **Done when:**
  1. The log holds the parts table at both poses, names the part, and records the fix.
  2. At the hall's shot, lamp off, exposure 8: the pane's pixels over 250 are at most twice the share before the pane
     (0.8 %). Each pixel's composite is within 2 levels at p90 of the k-layer model, built from the reading's own parts:
     `(1 - a)^k * behind + sum over layers i of a * (1 - a)^(i - 1) * albedo * light_at_layer_i`, with `a` = 0.6 and `k` the
     layers the walk entered at that pixel (amended 2026-11-07, owner: the pane is 2 voxels thick and walks 2 to 4 layers).
  3. `a_glass_pane_does_not_brighten_what_it_covers` (lavapipe, `cache_box`, under 5 s) asserts that a 0.6 pane of a dark
     albedo over a lit wall reads no brighter, per pixel, than the larger of the wall alone and the pane's own lit face.
     The brightening before the fix is the failure case.
  4. The gpu tier's goldens that hold no translucent material are unchanged.

### Phase 5: The owner's look at the samples cut
- **Owner skill:** human
- **What:** a short window, standing and turning, the reference and the offered arm at one pose by the key, at the vein, the patch and the hall. Plan 0057's unanswered
  (g), the blend's look, is asked here, since the filter is where it returns.
- **Files touched:** this plan's log.
- **Done when:** the log records the owner's words on: (a) the grain against the reference; (b) a slow error across a wall, and seams at chunk borders; (c) the filter's
  softening, shadows and a pit's lit rim; (d) **which arm stands, or how many samples to go back up.** A refusal of every arm comes to the architect.
  Phase 3's unanswered (e) and (f) are asked again here: a face's flat value at a voxel step and a shadow edge in the reference, and the lamp and its bounce over the bake.
  The arms are independent 1024 and 512, filtered (the amendment of 2026-11-06, Phase 4's stops). (h) The glass pane at the
  hall with the lamp off: no white blob after Phase 4a, and the pane reads dark, since glass has no light of its own. Is that
  right, or should glass get baked light? (i) With the lamp on, the lamp's spot on the pane at 60 cm saturates about 44 % of it:
  is that the lamp, or a fault? A fault in (h) or (i) comes to the architect (the amendment of 2026-11-07).

### Phase 5a: Glass as a filter (amended 2026-11-08, owner; ADR-0047)
- **Owner skill:** dev
- **What:** `translucent_march` stops compositing each layer as a diffuse wall. The pixel becomes
  `T(L) * behind + R * face_light`, as ADR-0047 says:
  - `L` is the path length through the translucent row's voxels, from the walk's entry and exit distances, up to the
    march's layer cap measured in length rather than layers.
  - `T(L) = tint^(L / d)` per channel, with `tint = 0.8 * (0.5 + 0.5 * albedo / max(albedo))`, read from the row's albedo,
    with no new row field. Glass's tint is then (0.52, 0.56, 0.80), and through 2 voxels (0.27, 0.31, 0.64), whose
    luminance is 0.33.
  - `R` is Schlick's Fresnel with `F0` = 0.04 at the entry face.
  - `face_light` is the entry face's untinted light, `E / PI` (a dielectric reflects white). `E` is the entry face's
    store cell at the scale the opaque read applies, plus the lamp's analytic term while the lamp is on. The lamp
    term needs no shadow ray, because the lamp sits at the eye and the eye's ray reached the face. The lamp's bounce is
    not added on glass: `R` is a few percent, and the probe group stays off the hit pass. The bake writes cells for
    translucent faces, both sides of a pane.
  - **Where it is computed (amended 2026-11-08, architect, at 5a's stop):** in `translucent_march`, with the store
    bound read-only to the hit pass (the temporal pass keeps its read_write binding). The march writes `R * face_light`
    into the layer target's rgb.
  - **How `T` reaches the composite:** a second target holding `T` per channel as rgb9e5 in an `r32uint`, 4 B a pixel
    (about 14 MB at 2530 x 1378). The composite reads `layer.rgb + T * c` for the native and the FSR 2 path alike.
    The layer target's `w` is no longer read.
  - **What is behind glass reads its store cell:** `cacheable(rec)` stops excluding a hit with translucent layers in
    front, since the store is per face and the glass in front does not change the face's value. The backlog's
    "A held fallback still moves on screen" stays parked: this removes the fallback behind glass only.

  The march's source hash changes, so every store file under `live/bake/` is re-baked: the reference and 512 at the vein,
  the patch and the hall. Opaque faces' blocks are compared before and after, and must be bit-identical.
- **Files touched:** `crates/vxe-render/src/shaders/march.wgsl`, `composite.wgsl`, `temporal.wgsl` (only if `cacheable`
  lives there), `crates/vxe-render/src/marcher.rs`, `crates/vxe-render/src/bake.rs`, `shaders/bake.wgsl`,
  `crates/vxe-render/src/lib.rs` and `gpu_world.rs` (the storage-buffer count), `apps/flythrough/tests/bake.rs`, the
  `translucent.png` golden, `docs/plans/logs/0059.md`.
  - **The storage-buffer count goes from 11 to 12** (amended 2026-11-08, architect, at 5a's second stop, 6e100d8). The
    store's read-only binding on the hit pass is counted in `OTHER_STORAGE_BUFFERS`, and lib.rs's limit test asserts 12.
    Every named adapter offers about 10^6 per stage (lavapipe 1 015 808, the RTX 1 048 576, RADV Renoir 8 388 606), so it
    costs no adapter. The rejected option, freeing a slot by packing the split walk's arguments into its list, changes
    the walk for a count no adapter is short of.
  - Phase 4a's `cut_glass_parts` reading assumes the per-layer model (`layer.w` as `(1 - a)^k`). It is marked in the
    log as historical and not kept up to date.
- **Done when:**
  1. **No voxel grid.** At the hall's shot with Phase 4a's pane, lamp off, exposure 8, read the pane's pixels whose
     behind is flat, meaning neighbours within 1 level before the pane. Their neighbour differences, minus those of the
     same pixels before the pane, read at most 1 level at p90.
  2. **No white-out.** The same shot with the lamp on: the pane's pixels over 250 are at most twice the share of the
     same pixels over 250 with the lamp on and no pane.
  3. **Not dark.** Lamp off, at near-normal view, the pane's mean luminance before the display is at least 0.3 of its
     behind's mean. The arithmetic is 0.33 for 2 voxels of glass, so the bar leaves a small margin for paths longer
     than 2 voxels.
  4. `a_glass_pane_filters_by_path_length` (lavapipe, `cache_box`, under 5 s) asserts three things: a pane 1 voxel
     and a pane 2 voxels thick over the same lit wall read `T` and `T^2` of it, within 2 levels; a 2-voxel pane at
     30 degrees reads its longer path's `T(L)`; and with no light behind, a pane in a lit box reads `R * E / PI`
     from its store cell's `E`, within 10 %. Before the fix, the per-layer composite fails the first assertion.
  5. `a_glass_pane_does_not_brighten_what_it_covers` still passes. `translucent.png` is re-blessed, its change stated in
     the log; every other golden is byte-for-byte unchanged.
  6. Every opaque face's cell reads bit-identical by its key, before and after the fix, in every chunk of the re-baked files.
     A chunk with no translucent voxel is bit-identical block for block. A chunk holding glass is compared by key, because
     its face runs gain the glass faces. `live/session5c.sh` is written: glass at the hall, lamp off then on, F through
     the reference and 512.
  7. **What is behind reads its cell.** At the hall's shot, lamp off, the pane's pixels' behind light (before `T`) is
     within 1 level at p90 of the same pixels' light with no pane.

### Phase 5b: The owner's look at the glass
- **Owner skill:** human
- **What:** the glass at the hall, `live/session5c.sh`, lamp off, then on.
- **Files touched:** this plan's log.
- **Done when:** the log records the owner's words on the glass with the lamp off and on. A refusal comes to the architect.

### Phase 6: The reach cut, the brick level and the rings
- **Owner skill:** dev
- **What:** samples by distance, and a coarse level beyond a ring. **The brick level:** a surface brick holds six direction values (24 B), each the mean of that direction's faces.
  A brick-level job draws from its own samples (64 per brick direction, a guess). A face with no cell reads its brick's direction value, interpolated across the same-side values of
  neighbouring bricks, multiplied by the brick's first dominant material's scale. A brick over `BRICK_FACES_MAX` (512, set by Phase 1's census) stays at the brick level. **The rings**, a
  `Lighting` setting: 12 brick, 4 / 2 / 1 face at 32, 128, 512 (or the samples Phase 5 kept), still from a pre-roll, with no queue yet. The bake's ring is cut against the reference too:
  whether the world past `r_full - light_radius` needs anything, and at what share of the view.
- **Files touched:** `crates/vxe-render/src/bake.rs`, `shaders/bake.wgsl`, `shaders/temporal.wgsl`, `marcher.rs`, `apps/flythrough/src/main.rs`, `hud.rs`, `apps/flythrough/tests/bake.rs`,
  `docs/plans/logs/0059.md`.
- **Done when:**
  1. `a_face_with_no_cell_reads_its_brick` (lavapipe, under 5 s): a lit wall with its cells dropped by a test hook reads within 4 levels of the mean of the same faces' cells, the wall's
     pixels non-zero.
  2. A strip at the vein's `back` view and the hall against the reference, by ring: the share within 1, 2 and 4 levels, `G_plane`, the dark shares, the fresh start's seconds on both
     adapters at the rings' samples, and the store's bytes against the reference's.
  3. Store files for the rings as offered, under the mode key.

### Phase 7: The owner's look at the reach cut
- **Owner skill:** human
- **What:** the first draft's still-bake look: the reference and the rings at one pose by the key, standing and turning, then flown across a ring on the pre-rolled store.
- **Files touched:** this plan's log.
- **Done when:** the log records the owner's words on: (a) the brick level beyond 4 chunks against face cells ("Lego" or not), and whether the ring should move; (b) the far caves against the
  reference; (c) the fresh start's wait at the rings' samples; (d) **which ring stands.** A refusal comes to the architect.

### Phase 7a: The brick level's samples and cell (exploration; amended 2026-11-08, owner)
- **Owner skill:** dev
- **What:** the brick level everywhere, under the mode key, in variants along two axes. The first draft's ring structure is
  set aside until the pick.
  - **Samples a brick direction:** 64 (Phase 6), 512 and 4096. The arithmetic for a dig of about 700 bricks: 6 x 700 x
    4096 = 1.7 x 10^7 samples, about 0.23 s at 13.5 ns; at 512, about 29 ms.
  - **Cell:** 8 voxels (the brick, Phase 6) and 4 voxels (eight sub-cells a brick, 8 x the record's values). The same read:
    bilinear in the face's plane across the neighbouring cells of the same side.

  Offer at most six cuts. Start with 8/512, 8/4096, 4/512 and 4/4096, and add 4/64 if it's cheap: a cell of 4 at 64 is
  the noise floor's test. The banner names each cut as `bricks <cell>/<samples>`. For each cut, read the dig's re-bake ms,
  the bake from nothing at the hall and at the level, and the store bytes. They are readings; nothing gates.
- **Files touched:** `crates/vxe-render/src/bake.rs`, `shaders/bake.wgsl`, `shaders/temporal.wgsl`, `apps/flythrough/src/main.rs`,
  `hud.rs`, `apps/flythrough/tests/bake.rs`, `docs/plans/logs/0059.md`.
- **Done when:** the cuts are built and baked at the vein, the patch and the hall, under the key, with the readings in the
  log, and `live/session7b.sh` is written. Its steps are the vein far from the spawn (the screenshots' chunk [18, 15, 3],
  as a pose or a start position), a dig there, the patch, and the hall. The table and the reference stay in the cycle.

### Phase 7b: The owner's pick of the brick level
- **Owner skill:** human
- **What:** `live/session7b.sh`.
- **Done when:** the log records which cell and sample count stand, or that none does. None standing comes to the architect.

### Phase 8: The queue, the rings and the reach gate
- **Owner skill:** dev
- **What:** the bake leaves the pre-roll, so a walked and a streamed world stay lit: the scheduler of ADR-0046, nothing yet for edits. This lifts `--light baked`'s refusal of a streamed world.
- **The list on the workers.** The face list moves to the stream workers through ADR-0017's hook, declared in `vxe-world` and implemented in `vxe-render`, the result opaque to the world, with the inline path if a prepared result is missing; `every_exposed_face_has_one_key` gains the equality of the two.
- **The queue,** on the render thread: a chunk bakes only when every chunk within `light_radius` of
  it is mirrored or known empty. Its order: edits (Phase 9), then distance to the camera, ties by
  forward direction then by coordinate. Each chunk has a tier from its ring (Phase 7's, the first draft's 12 brick, 4 / 2 / 1
  face at 32, 128, 512), and a job replaces a lower tier from
  scratch. Tier-ups are enqueued when the camera crosses a ring, and a chunk leaving a ring keeps
  what it holds.
- **The slice.** `Lighting::bake_slice_ms` (1.5 by default; 4 with `BakeProfile::Low`, where `N` and
  the brick samples are a quarter, on `--adapter` Radeon by default): samples this frame are the
  last frames' ns a sample into the slice, from the bake's timestamp. Without timestamps, or in a
  test, a fixed `bake_samples_per_frame`. An idle queue dispatches nothing.
- **The fade.** A job's values are written to the transient buffer's final form and copied over
  `FADE` 16 frames by a lerp in the pool, in the job's dispatch; a first value for a face and
  anything under `bake_until` is written directly. The move from brick level to face cell uses the
  same fade, by a per-chunk counter the read mixes with.
- **Eviction.** The pool's free list under pressure drops the farthest chunk's cells to its brick
  level, then a chunk beyond 12 altogether; a bake for a nearer chunk always finds room. A chunk
  leaving the mirror frees its block. Nothing else drops values, so walking back finds them.
- **Emitter ranges** (ADR-0025): ranges are held within bake radius plus `light_radius`, up to the
  reservation, by the existing nearest-first writes, and every job's table is written from ranges
  that exist: a job waits rather than baking against a missing range. The extraction is re-read
  against its budget.
- **Readings:** the rig's flight (`settle.rs`, Plan 0057's `light_table` paths) at 30 and 120
  voxels/s at the vein, the patch, the hall and the streamed cave, per second: jobs, samples, queue
  depth by ring, the slice's ms; per frame, the share of pixels at a face cell of each tier, the
  brick level and no value, by ring.
- **Files touched:** `crates/vxe-render/src/bake.rs`, `shaders/bake.wgsl`, `shaders/temporal.wgsl`,
  `crates/vxe-world/src/stream.rs` (the hook), `gpu_world.rs`, `marcher.rs`, `lib.rs`, `apps/flythrough/src/main.rs`, `hud.rs`,
  `apps/flythrough/tests/bake.rs`, `settle.rs`, `support/settle.rs`, `upscale.rs`,
  `docs/plans/logs/0059.md`.
- **Done when:**
  1. `a_chunk_waits_for_its_reach` (lavapipe, under 10 s): a chunk whose reach holds an emitter in a
     chunk not yet mirrored is not baked; after the chunk arrives it is, and its wall reads the
     emitter's light within 3 % of a bake where both were present from the start, and the value is
     bit-identical to it.
  2. `the_queue_runs_near_first_and_within_its_samples` (no GPU where the scheduler allows, under 1
     s): over a scripted ring of chunks, jobs emit by distance, each frame's samples stay at or
     under `bake_samples_per_frame` but for one batch's overrun, and an idle queue emits nothing.
  3. `a_tier_up_fades_and_ends_still` (lavapipe, under 10 s): moving a camera across a ring re-bakes
     at the higher tier, each face moves at most `|new - old| / FADE` a frame plus one `rgb9e5`
     step, and 32 frames after the last job two frames read bit-identical.
  4. `a_full_pool_drops_the_farthest_first` (lavapipe, under 10 s): a pool of a test size over-asked
     keeps the near chunk's cells, drops the farthest's to the brick level, and reports both in the
     HUD's counters.
  5. `walking_back_finds_its_light` (lavapipe, under 15 s): a camera moved away and back over a pool
     with room causes 0 new jobs for chunks it left within the pool's retention.
  6. **On the RTX, readings against bars:** at 30 voxels/s over 180 voxels at the vein and the patch
     (Plan 0057's `flight_path`), in every frame, **no pixel with a hit within 256 voxels has no
     value**, **the share of pixels whose hit is within 64 voxels and at a face cell of at least 128
     samples is at least 0.99**, and **the dark share (over 8 levels darker than the settled frame)
     of hits beyond 64 voxels is at most 0.01** (Plan 0057's rows, frame 1: 0.4389 under FSR 2,
     frame 64: 0.0180). The slice's mean is at most 1.5 ms. At 120 voxels/s the same rows are read,
     not barred. A miss comes to the architect with the queue's depth.
  8. **The owner's fresh start, on the RTX, a bar** (amendment of 2026-11-06): at the vein, the patch and the hall from a
     cold start with no store file, **the first frame within 3 s of the world's load**, with every pixel whose hit is within
     64 voxels at a face cell. The seconds until the queue first empties are read beside it, as is the same start on the
     Cezanne, which is not barred. A miss comes to the architect with the samples spent before the first frame.
  7. `frame_bytes` in `upscale.rs` asserts the base's figure plus `bake_pool_bytes` plus the working
     buffer (1 MiB) plus the chunk table's, the arithmetic in its comment, under 5 s, while `--light
     table` allocates a minimal pool.

### Phase 8a: The queue's throughput (exploration; amended 2026-11-08, owner)
- **Owner skill:** dev
- **Why:** Phase 8 met the fresh start on the RTX (first frame 1.9 to 2.5 s), but the live queue fills at 4 to 6 x 10^6
  samples a second on the RTX and about 10^6 on the Cezanne (`logs/0059.md`, Phase 8). The level would take an estimated
  10 to 15 minutes, and a 30 voxels/s flight shows up to 0.46 of pixels with no value. Phase 8 names two causes: one
  invocation draws an entry's samples serially, and each frame waits for the GPU to go idle before its slice. The owner
  picked "Fix throughput, then look".
- **What:**
  - **Spread an entry's samples across a workgroup in `bake.wgsl`.** Each lane draws a share, and the sums reduce in
    workgroup memory. The estimator is unchanged: the same samples, the same seeds by sample index, so a value differs
    from Phase 8's only by the order of float sums.
  - **Submit the slice without waiting.** The slice's samples are sized from the last read-back timestamps, one or
    two frames late. A job's results are read back when ready, not in the frame that submitted them.
  - The source hash changes, so store files re-bake.
- **Files touched:** `crates/vxe-render/src/bake.rs`, `shaders/bake.wgsl`, `gpu_world.rs`, `marcher.rs`,
  `apps/flythrough/src/main.rs`, `docs/plans/logs/0059.md`.
- **Readings** (exploration; nothing gates):
  - samples a second on both adapters, against Phase 8's;
  - the fresh start's first frame and the seconds until the queue empties, at the vein, the patch and the hall;
  - the vein flight's share of pixels with no value, per second;
  - the frame's GPU ms while the queue runs, against at rest;
  - a hall bake's values against Phase 7a's 8/4096 file: the share within 1 level, which should be about all of it.
- **Done when:** built, read, and `live/session8.sh` written: a fresh start at the vein and the hall with `--light queue`,
  a flight at the vein, and a dig at each, with the first frame and the queue's time to empty printed after each run.

### Phase 8b: The owner's look at the live queue
- **Owner skill:** human
- **What:** `live/session8.sh`.
- **Done when:** the log records the owner's words on the fresh start, the world filling in, the flight, and the digs.

### Phase 8c: Coarse first, a larger near bake (exploration; amended 2026-11-08, owner)
- **Owner skill:** dev
- **Why:** at Phase 8b (`live/session8.sh`; screenshot `live/session8-hall-dark.png`), the owner's words, verbatim:
  1. "it starts fine, but far parts of cave is loading and quite slowly, it would be better to postpone the initial load
     but load more".
  2. "load is unfortunately too slow even for this slow drift".
  3. "dig is instant".
  4. "hall loads slowly with the same problem of darkness ... better to wait more initially. dig is instant. overall
     quality of lighting are good though and I think this is the right way".

  Asked how the world should fill, the owner picked "coarse first + bigger initial area".
- **What:** before the first frame, at full GPU (the pre-roll path, not the 1.5 ms slice):
  1. Every chunk the mirror holds gets a **coarse pass**: 8-voxel cells at 256 samples a direction.
  2. Every chunk within 2 chunks of the camera (Chebyshev, 125 chunks) gets the **full 8/4096**.

  Then the queue refines the rest to 8/4096 within the slice, nearest first. A refined value fades over `FADE` frames
  from the coarse one. A chunk the mirror gains later takes a coarse pass at the head of the queue, ahead of
  refinement. So nothing reads no value once its chunk is mirrored.

  The arithmetic, from Phase 8a's 7 ns a sample at full GPU on the RTX (a guess for the pre-roll, which may run faster):
  - the level's coarse pass: 3.84 x 10^9 / 16 = 2.4 x 10^8 samples, about 1.7 s;
  - the near 125 chunks at 4096: about 0.9 to 1.5 s.

  So the first frame is about 3 to 4 s at the level, against the owner's 3 s target, which the owner traded for "wait
  more initially". The coarse sample count (256) and the near radius (2) are flags (`--coarse N`, `--near R`), named in the
  banner.
- **Files touched:** `crates/vxe-render/src/bake.rs`, `shaders/bake.wgsl` if needed, `marcher.rs`, `apps/flythrough/src/main.rs`,
  `hud.rs`, `docs/plans/logs/0059.md`.
- **Readings** (nothing gates):
  - the first frame, split into its coarse and near parts;
  - the time until everything is at 4096;
  - the no-value share, which should be 0 for mirrored chunks;
  - the vein flight's share of pixels still coarse.

  Read at the vein, the patch, the hall and the streamed cave, on the RTX and on the Cezanne.
- **Done when:** built, read, and `live/session8c.sh` written with Phase 8b's steps.

### Phase 8d: The owner's look at coarse-first
- **Owner skill:** human
- **What:** `live/session8c.sh`.
- **Done when:** the log records the owner's words on the start's wait, any darkness, and the coarse light against the refined one.

### Phase 9: The dig under a budget, the three rings
- **Owner skill:** dev
- **What:** the three rings of ADR-0046, driven by the box `Frame::invalidate` receives. They replace the reference's in-frame re-bake of Phase 2, and ring 3 is what the reference left out; `the_far_light_stays_as_the_reference_states` goes. The owner's eye on it is Phase 13 (e).
- **Ring 1,** the box dilated by one voxel. In the frame the mirror receives the edit, ahead of the
  temporal pass: a brick's run is relisted (surviving keys keep their value until replaced) and
  baked at 32 samples within a per-frame cap of `BAKE_EDIT_SAMPLES` (133 000 RTX, about 1.8 ms at Phase 1a's
  13.5 ns, on top of the slice in that frame only, a tenth of it on `BakeProfile::Low`; a guess). What the cap
  postpones is first in the queue and reads the brick's old direction value. Then the ring's tier at
  edit priority. A face with no value takes its first value directly, without the fade.
- **Ring 2,** bricks within 32 voxels of the box: at the ring's tier, at edit priority, fading in.
- **Ring 3,** bricks within `light_radius` of the box: at low priority, at the ring's tier (or brick
  level), only if the touched bricks' direction values read above 0 before or after the edit, or the
  box's material counts include an emitter.
- **Dedup and coalescing.** Per chunk a 512-bit dirty mask and a generation: a re-dirtied brick
  restarts its job, and a job for a stale generation throws its result away. `MAX_EDITS` no longer
  matters to this store. A collapse's `drop_region` and `remove_region` arrive as boxes through the
  same path: one per tick.
- **A scale change** moves nothing: a test defends it. A material swap that adds or removes an
  emitter opens ring 3 and updates the emitter lists as today.
- **Files touched:** `crates/vxe-render/src/bake.rs`, `shaders/bake.wgsl`, `gpu_world.rs`,
  `marcher.rs`, `apps/flythrough/tests/bake.rs`, `settle.rs`, `support/settle.rs`, `strip.rs`,
  `docs/plans/logs/0059.md`.
- **Done when:**
  1. `a_dug_hole_is_lit_on_its_first_frame` (lavapipe, under 15 s, a test-set samples per frame,
     scale 1, lamp off; a lit wall, a radius 3 carve): on the first frame the hole is on screen,
     every pixel whose settled value is over 1 level reads over 0, and the hole's faces' p90
     distance to their settled values is at most 10 levels (32 through the tent: 8.9); by frame +16,
     at most 3 levels (512: 2.2).
  2. `a_postponed_face_reads_its_bricks_old_light` (lavapipe, under 10 s, the cap at 1 sample): the
     new faces read their brick's direction value from before the edit, within 4 levels of it, and
     no pixel in the box reads 0 where the value was over 1 level.
  3. `a_break_through_reaches_the_far_wall` (lavapipe, under 15 s): a wall between an emitter and a
     far room: after carving a window in it, the far wall facing the window reads within 10 % of a
     fresh bake within the frames its samples need (the test states them), and the plug put back
     returns it to within 1 level of 0.
  4. `a_dark_carve_opens_no_reach` (lavapipe, under 10 s): a carve in rock no emitter reaches queues
     rings 1 and 2 only.
  5. `an_emitter_dug_away_darkens_what_it_lit` (lavapipe, under 15 s): removing the box's one
     emitter brings its wall under 1 level within the frames ring 3 needs at the test's samples.
  6. `a_scale_change_moves_no_bake` (Phase 2's) also asserts the queue stays empty.
  7. `a_falling_slab_bakes_only_what_it_touched` (lavapipe, under 15 s): a `drop_region` over 8
     ticks, the bricks baked are within the rings of its boxes, no brick is baked twice for one
     generation, and 32 frames after the last tick two frames read bit-identical.
  8. **The owner's test on the RTX, readings against bars** (the rig's dig at `VEIN_POSE`, Plan
     0057's 5-voxel dig and the dig at 100 voxels, scale 1, lamp off): **`Q90` (Plan 0057's
     stop-moving figure) at most 16 frames at arm's length and at most 30 at 100 voxels**, against
     that plan's bars of 60 and 120; **`S90` and the travel after the stop** beside it; the frame's
     lighting ms on the dig frame and the next 16. The B strip (Plan 0057 Part 0's four presses, the
     fly-through's bench): the carved region's share within 2 levels of +256 at +1, +4, +16, +64,
     against that plan's before rows (0.8317 and 0.9814 at +1 and +4 native at the vein), and the
     whole frame's jobs and seconds to an empty queue. Read, not barred.
  9. **The owner's "no freeze, settles fast", on the RTX, bars** (amendment of 2026-11-06; the same two digs at the window's
     size): **no frame from the dig frame to dig + 30 over 33 ms**, with the bake's slots in each at most `bake_slice_ms`
     plus the edit cap; and **the hole's faces at p90 within 2 levels of a fresh bake within 0.5 s of wall time** (the
     arithmetic is in the amendment: about 23 frames). A `B` press: the same frame bar, and the seconds to settle read,
     not barred (about 10 s by the arithmetic). A miss comes to the architect with the per-frame samples.

### Phase 10: The resolution cut
- **Owner skill:** dev
- **What:** lighting and internal resolution against the reference, read at native and at FSR 2 at 1.5 (the fly-through's default; the antialiasing plan is parked, amendment of 2026-11-06); what is cut here is
  `--lighting half` under the store, and whether a half-resolution read needs the own-face read of Plan 0057's unbuilt Part 3 at face edges. Each setting is read against the reference at
  the same poses, standing and on the rig's fast yaw.
- **Files touched:** `crates/vxe-render/src/shaders/temporal.wgsl` and the composite if the own-face read is built, `apps/flythrough/src/main.rs`, `hud.rs`, `apps/flythrough/tests/bake.rs`,
  `docs/plans/logs/0059.md`.
- **Done when:**
  1. The log holds, per setting, the share of pixels within 1, 2 and 4 levels of the reference at one display size, the frame's passes, and the fast yaw's excess.
  2. The settings are offered under the mode key, the mode named in the banner and HUD.

### Phase 11: The owner's look at the resolution cut
- **Owner skill:** human
- **What:** a short window: the reference and each offered setting, at the hall and the vein, standing, turning and moving fast.
- **Files touched:** this plan's log.
- **Done when:** the log records the owner's words on: (a) face edges and silhouettes; (b) the fast turn; (c) **which setting stands.** A refusal comes to the architect.

### Phase 12: Budgets, the default and the end strip
- **Owner skill:** dev
- **What:** both adapters read at the cuts the owner kept, and `--light baked` made the default, goldens re-blessed once on
  lavapipe with their pixel difference in the log. `--light table` stays for the window.
- **Files touched:** `crates/vxe-render/src/lib.rs`, `apps/flythrough/src/main.rs`,
  `apps/flythrough/examples/hero.rs`, the golden baselines, `apps/flythrough/tests/*` that assert
  the default, `docs/plans/logs/0059.md`, `docs/nfr/02-frame-budget.md`, `docs/nfr/03-memory.md`,
  `docs/nfr.md`.
- **Done when:**
  1. The Budgets rows below are read with machine, commit and load. **The bars on the RTX at the
     window's size:** the slice's mean at most 1.5 ms walking; at rest with an empty queue the
     bake's slot at most 0.02 ms and two frames bit-identical at scale 1; the temporal pass within
     0.1 ms of the table's; the lighting line (light, gradient, probes, gather, bake) at or under
     the table's in the same run at rest and walking. **On the Cezanne:** the lighting line under
     `docs/nfr.md`'s 6.0 ms, or the reading names what is over.
  2. The still image holds: Plan 0057's still bars read at baked (the hall native one-frame mean at
     most 0.005, no pixel over 8 levels), and `the_fast_yaw_excess_by_table_read`'s row beside the
     table's.
  3. The dark shares and `G_plane` from Phase 2's strip, and the share within 1, 2 and 4 levels of the reference, are re-read on the tip with the window's
     path, and the adapter difference is read once: the vein strip's cells on the RTX against the
     Cezanne, the mean and p99 in levels (the target is under 1 level mean, a reading).
  4. Residency p95 is re-read under motion (owed since `docs/nfr.md` section 5's note of 2026-11-04)
     at the fly-through's default, and the worker hook's list time per chunk beside it. Any
     regression over 500 ms comes to the architect.

### Phase 13: The owner's window
- **Owner skill:** human
- **What:** the orchestrator's runner on Phase 12's tip, with `--light table`, the reference's files and stdout visible, and
  the strips at hand; a ping when ready. Vein, patch and hall from a fresh start, then a walk in and
  out of a place not yet seen, then a dig at each, then the fast turn.
- **Files touched:** this plan's log.
- **Done when:** the log records the owner's words, with the HUD in view, on: (a) "already there":
  at a fresh start and on walking into a place not yet seen; (b) dark patches and the distant caves;
  (c) grain at rest, on a turn and while walking; (d) anything moving at rest, and any tier change
  seen while walking or standing; (e) the dug hole at arm's length and at range, the `B` presses,
  and a dug break-through; (f) the brick level at range, the ring's place, seams at chunk borders;
  (g) the lamp and bounce over the bake; (h) the fresh start's wait on both adapters; (i) **the
  verdict: baked or table.** A "no" names the step and goes to the architect.

### Phase 14: The deletion
- **Owner skill:** dev
- **What:** the path that lost goes. **On "baked":** the table, its fill, the preload, the guard's
  re-shade of cells, the sweep, the age-out, Plan 0057's table size and blend pass, `--light`, the
  cell's test rigs, and every test that asserted them, named in the log. `frame_bytes` falls by the
  table. The gradient pass's lamp lambda stays. **On "table":** the store and bake, the hook,
  `--light` and this plan's tests go, the reference's readings stay in the log, and ADR-0046 is marked
  superseded by an architect amendment.
- **Files touched:** `crates/vxe-render/src/marcher.rs`, `bake.rs`, `lib.rs`, `shaders/`
  (`cache_fill.wgsl`, `cache_clear.wgsl`, `cache_sweep.wgsl`, `gradient.wgsl`, `light.wgsl`,
  `temporal.wgsl`, `march.wgsl`), `apps/flythrough/src/main.rs`, `hud.rs`,
  `apps/flythrough/tests/*`, `upscale.rs`, `look.rs`, `.config/nextest.toml`,
  `docs/plans/logs/0059.md`.
- **Done when:**
  1. `cargo build` has no dead `Lighting` field and the flags that went are refused with a message
     naming this plan's number (`hud.rs`, no GPU, under 1 s).
  2. `frame_bytes` asserts the base less the table plus the store, and the log prints the figure
     against `docs/nfr.md` section 3's 1.5 GB.
  3. The gate's test count in the log names each test removed and each re-pointed. The still-image
     and dig bars of Phases 9 and 12 are re-read on the tip, within their bars.

## Data shapes

Illustrative; dev records the final layout in the log. The reference holds the directory and the cells; the direction values arrive with the reach cut (Phase 6).

```rust
// A chunk's block in the pool. Offsets are in u32 words from the block's start.
struct ChunkLightBlock {
    directory: [u32; 512], // (run_offset << 11) | face_count; 0 = no surface in the brick
    // per surface brick, at run_offset:
    //   direction_values: [u32; 6]  rgb9e5, side order +x -x +y -y +z -z (Phase 6 on)
    //   cells: [FaceCell; face_count], sorted by key; absent at the brick level
}

#[repr(C)]
struct FaceCell {
    rgb: u32,  // rgb9e5, irradiance at unit emission, albedo divided out
    tail: u32, // key: 12 (voxel 9, side 3) | flags: 4 | dominant emitter material: 16
}
```

**The stored error:** `rgb9e5` steps by about 0.2 to 0.4 % of a value, at most about 0.36 of a
display level at level 200 through the 2.2 encode, arithmetic: under `STILL_LEVEL`.

## Budgets

**The reference has none.** Phases 1 and 2 read their figures so the cuts have something to cut from, and no bar here applies before Phase 12. `frame_bytes` under `--light table`, the default until then, does not move. The table below is the first draft's and is re-read at the verdict.

| `docs/nfr.md` figure | Today | Expected (a guess until read) | How the close measures it |
|---|---|---|---|
| The reference (read, no bar) | none | A radius-2 piece: about 8.7 x 10^5 faces, about 7 MB in the pool and about 37 minutes a file on the RTX (guesses from Phase 1's census and rate; the whole level read 21.3 M faces and 177 MB, about 15 h); a 1 m dig's hitch of about half a minute at the vein | Phase 2's `reference_readings` |
| §2 lighting line 6.0 ms | RTX in the window: light 1.33 to 3.18, gradient 0.00 to 0.17, preload 0.00 to 0.04; Cezanne walking at 1280 x 720: light 4.2 to 6.9 | RTX: slice mean at most 1.5 walking, lamp pass about 0.11 (Plan 0051), gradient as lamp only. Cezanne: 4 ms slice at a quarter's samples plus about 0.3 | `cost` rows in `settle.rs` on both adapters at rest, walking and Shift, quiet where the machine allows; Phase 12 done-when 1; `docs/nfr/02-frame-budget.md` |
| §2 denoise line 3.0 ms (the temporal pass's read) | The table's read at the window's size | Within 0.1 ms of it: a directory read and at most about six in a run of 34 faces | The same rows |
| §2 streaming line 1.0 ms (a recorded miss) | p50 about 5 ms over a flight | Adds under about 25 KB a frame of copies walking (1.4 MB/s: face chunks 12/s x 62 KB, brick chunks 106/s x 6.5 KB), and the free list. Listing on workers | The autopilot's streaming CPU, the worker hook's list time per chunk; Phase 12 done-when 4 |
| §3 everything else in VRAM, 1.5 GB | About 274 MB with the table at 2^22 (Plan 0057's Budgets) | Between phases + 134 217 728 B pool + 1 048 576 B working + the chunk table; at Phase 14 - 201 326 592 B (the table, at the size Plan 0057 closes with): about 208 MB. Use inside the pool about 49 MB at the vein's density | `frame_bytes` in `upscale.rs` asserted; HUD `store`; `docs/nfr/03-memory.md` |
| §5 residency p95, 500 ms | Last read at Plan 0023, 188 to 403 ms; owed | No regression: the bake never delays a chunk's mirror | Phase 12 done-when 4 |
| Light sample buffer | 16 B x 1 048 576, peak 18 522 within 4 chunks | Up to about 997 740 at 16 chunks (ADR-0025's streamed reading) | Phase 1's census; Phase 8's readings |

## Risks & open questions

- **The reference may not look right.** That is what it is for. A "no" at Phase 3 costs the two phases before it and leaves the table in place, against the first draft's eight phases.
- **The count may not reach the eye's line at any wait.** Plan 0057's vein row extrapolates to about 780 000 samples a face beside the emitters, and `1 / sqrt(N)` is an extrapolation. Phase 1's arms read `params.candidates` and the clamp, its stop spends only where the noise is, and its stop rule brings a miss to the architect. A heavy-tailed sample can also make a face's own spread read small before its rare bright samples arrive, so the `G_noise` bars across two seed sets are the check, not the stop's own estimate.
- **Dev's reading of the vein's first frame was not tested** (Plan 0057's done-when 5, estimator variance beside the emitters). If the reference's vein still reads dark or noisy at a low `G_noise`, the cause is elsewhere, and Phase 1's rows by distance to the nearest emitter are where that shows.
- **Turning the clamp off changes the look beside emitters.** It should, since the clamp biased them dark, but the owner has never seen those faces without it. Phase 3 (e) covers it, and the setting is one word.
- **A long bake on this laptop.** See "A long bake" above. A piece's bake of up to an hour at full GPU load is the longest sustained load this box has had from this repo. It is run once per tree, alone, under the lock, with the owner told, and the file spares repeats.
- **The window's frame rate.** Native at the owner's 2530 x 1378 with full-resolution lighting marches about 2.25 times the pixels of today's 1687 x 919 and runs the lamp's passes at about 9 times the lighting pixels of today's 843 x 459. A low rate could read as a motion fault. Phase 2 reads it and the runner says so.
- **A dig's hitch at `B`.** 78 to 87 carves at the vein, each about 11 000 faces in its ring at the guessed count, at most about 3.6 x 10^9 samples at `N_MIN` before overlap: at Phase 1's 42 ns about 2.5 minutes frozen at `N_MIN`, and up to about 35 minutes at the stop's mean, before overlap (arithmetic on a guess; Phase 2 reads each press, and a press over 5 minutes leaves the step at one press). The bake's waits keep the window's events pumped so the compositor does not call it hung. Stated before the step, and accepted for this look.
- **What the owner sees after a dig is not the whole truth.** Ring 3 is out and the runner says so; the far light is stale until Phase 9.
- **The cuts may fall short of the frame.** The reference says nothing about whether a cut version meets 60 fps on the Cezanne. The first draft's arithmetic is the only evidence so far, and it now carries the vein's correction: `82 / sqrt(N)` understates the grain beside emitters, so the same samples read worse there than the first draft said. Phase 4 reads it, and the owner chooses.
- **Shared seeds might not beat their seam.** Phase 4; independent seeds at 1024 are the fallback and Phase 8's slice then needs a re-read.
- **The 4.6 ns may not hold.** It was read inside the light pass near the camera, with a camera-centred table and short rays. A far job's rays may be longer, and a separate dispatch has a floor. Phase 1 reads it for the reference and Phase 4 for the cuts. **Phase 1: it did not hold** (36 to 64 ns at 32 candidates, 141 to 275 voxels of shadow ray).
- **The brick level may look blocky.** Interpolation helps, and the owner sees it in Phase 7 before the queue exists, when moving the ring is a one-line change. The cost: ring 6 at 32 samples is 2.25 times ring 4's.
- **Faces lit through a gap cost as many samples as bright ones,** and so do the unlit, a share Phase 1 reads. Under the cuts, an early out on a brick whose 32-sample tier found nothing would save its later tiers. It is a Followup, because it can miss a face lit on one sample in 32 about 36 % of the time at 32 samples (`(1 - 1/32)^32`).
- **Fading vs nothing moves.** A tier change moves a still face for 16 frames. Tier changes happen when the camera crosses a ring and so only after motion, with one exception: a queue still behind when the camera stops. The queue empties in seconds on the RTX, and the owner judges it in Phase 13 (d).
- **GPU feature limits.** The pool is one binding: 256 MiB is over `wgpu::Limits::default`'s `max_storage_buffer_binding_size`, and `limits_within` already asks for more for the table. Lavapipe stops at 128 MiB, and the tests' worlds fit it. No new feature is needed. `rgb9e5` is arithmetic in the shader, not a storage format.
- **Threading.** Before Phase 8 everything runs on the render thread and blocks it, which is the reference's stated price. From Phase 8 the hook runs on workers and reads a chunk's and six neighbours' masks through the existing per-chunk locks, read-only; the render thread stays the only owner of the pool and the queue (ADR-0017). A prepared list stale by an edit is thrown away by the chunk's generation.
- **Long jobs on the slow adapter.** The slice caps dispatch sizes, so the driver's watchdog sees small dispatches. The reference does not run on the Cezanne; the first read there is Phase 4's.
- **Determinism.** Asserted within an adapter, read across. A bake's input includes emitter ranges and their order, which must be sorted by coordinate; the slicing test defends it. `vxe-world` gains a hook type at Phase 8 and nothing that reads the result.
- **Plan 0058.** Its emission-scale phase reads a cell's kept emitter. This plan's Phase 2 already makes the scale work through the dominant material. See Followups.

## What this plan does NOT do

- Bounce or ambient occlusion in the reference: "agreed, only light". The lamp's bounce stays live, and nothing is baked for it.
- Antialiasing. Its own plan, after Phase 3's verdict (`docs/backlog.md`). This plan's reference only shows what it has to fix.
- Dynamic or moving emitters in the store. They are ADR-0044's list, Plan 0058.
- Bounce from emitters. ADR-0043's lamp channel stays lamp-only; the per-source bounce is `docs/backlog.md`'s.
- A bake to disk as a product feature, or across runs for a consumer. `--bake-file` is the testbed's. Per-frame refinement past a stop: a still face stops at it.
- A per-emitter delta on an emission add. An edit opens ring 3.
- LOD or far-field light beyond the bake radius, and sky light (Plan 0032).
- Change the exposure or the fly-through's default far normal (Plan 0056's owed plan). The reference runs at `[1, 2]`, the owner's pick, by flag.
- Move the rings by adapter speed beyond the one `BakeProfile::Low`. The settings surface is `docs/backlog.md`'s.
- Entities shadowing the bake (M6).

## Implementation log

> Written by `dev`: one row per phase as its commit lands, then the close block. **The phases above
> are the contract.
> This section records what happened.** Observations, never conclusions: no pass list, no
> self-assessment. Deviations and
> unmet done-whens are **always** disclosed. Keep it shorter than `## Implementation phases`.
> Readings, gate counts,
> attempts, sweeps and resume notes go to the plan's readings log, `docs/plans/logs/0059.md`,
> started from
> `templates/readings-log.md`.

| phase | owner | state | commit |
|---|---|---|---|
| 1: The reference bake, and the count it needs | dev | stopped at its stop rule: the kept arm (32 candidates, clamp off, 65 536) meets the `G_noise` bars and projects a whole level at about 15 h on the RTX; readings in [`logs/0059.md`](logs/0059.md) | 70305ba |
| 1a: The stop's submits | dev | done: one dispatch a submit across jobs, rounds a submit by the stop's expectation; the stop reads 13.51 ns a sample kept against the fixed run's 14.23 on the RTX; the runner at `--bake-radius 1` | c7e1bb9 |
| 1b: The owner's first look | human | done 2026-11-05/06 on 9394ad5's release binary, RTX, `live/session1b.sh`: (a), (b) and (c) "all good" at the vein, the patch and the hall; (d) no "no", Phase 2 goes on; a remark: the load of the small piece is too long (words and bakes in the log) | ab7855f |
| 2: The reference: a piece to walk and a hole to dig | dev | done: the piece, the store file, the dig's re-bake, `F` and the pane; `G_noise` met at all four views; the files baked (the whole showcase as the hall's); the readings and `live/session3.sh` in [`logs/0059.md`](logs/0059.md) | 9a55841, ba46603, 595e568 |
| 3: The owner's verdict on the reference | human | done 2026-11-06 (`live/session3.sh`, then `live/session3b.sh` for glass and `B`): (j) yes, the look to cut down from; a dig froze 3-5 s, `B` about 2 min; new: a white blob on the glass pane; words and mapping in the log | 12446af |
| 4: The samples cut, the shared seeds and the filter | dev | done under the amendment of 2026-11-06 (d029596): the stop rule fired (no arm meets the offer's bars; the RTX 23.5 ns at 300 000 at the hall), and the owner picked independent 1024 filtered, 512 beside it; the cut files wait for 4a's re-bake; tables in [`logs/0059.md`](logs/0059.md) | 8df5c6c (code), b663570, the cut files with 4a's close |
| 4a: The glass composite | dev | done under the amendment of 2026-11-07 (0511c86): the part is the glass layer's headlamp light with the lamp off; fixed (`translucent_march` reads `LIGHTING_LAMP`); over 250 at 0.0000 against 0.0077 before the pane; the k-layer model reads p90 0.00 levels (lavapipe); [`logs/0059.md`](logs/0059.md) | a2cb48a (fix), 72d44f9 |
| 5: The owner's look at the samples cut | human | done 2026-11-07/08 on 72d44f9's release binary, RTX, `live/session5.sh` then `live/session5b.sh` (glass, placed on the third try): "512 looks mostly fine, if it is cheaper"; glass lamp off "looks very dark and consists of individual visible voxels", lamp on "looks like a white non transparent pane"; (h) and (i) to the architect (the amendment of 2026-11-08); [`logs/0059.md`](logs/0059.md) | 3ac2832 |
| 5a: Glass as a filter | dev | done: built for the owner's eye; bars not read (owner, 2026-11-08). The count at 12 (a939ba2's amendment); the two glass tests pass; `translucent.png` re-blessed; the gpu tier's other goldens unchanged, and 4 tests outside the files assert the replaced model (left red); the hall and vein files re-baked, the patch not; [`logs/0059.md`](logs/0059.md) | a972410 (first stop), 6e100d8 (wip), b3be3c1 |
| 5b: The owner's look at the glass | human | done 2026-11-08 on the 5a binary, RTX, `live/session5c.sh`: "works perfectly"; first frame 2711 ms; the pane's re-bake reference 2.5 s, 512 0.38 s; [`logs/0059.md`](logs/0059.md) | 9083198 |
| 6: The reach cut, the brick level and the rings | dev | done under exploration mode: brick records (28 B) before each brick's run, a face with no cell reads its brick bilinear; `--cut rings:12/4/2/1` and `bricks:12` under F; files at the vein, patch and hall; fresh rings bake 20.3 s vein, 4.0 s hall (target 3 s); the test not written, `G_plane` not read; [`logs/0059.md`](logs/0059.md) | 7c105c5 |
| 7: The owner's look at the reach cut | human | done 2026-11-08 on 7c105c5's release binary, RTX, `live/session7.sh`: the owner's four statements in the amendment of 2026-11-08 and the log; first frames 2458 to 3188 ms (files); 106 digs on bricks 12 re-baked in 9 to 39 ms; [`logs/0059.md`](logs/0059.md) | 8fd29cb |
| 7a: The brick level's samples and cell | dev | done under exploration mode: `--cut bricks:B/CELL/SAMPLES`, cuts 8/64, 8/512, 8/4096, 4/64, 4/512, 4/4096 under F with the reference, 512 and the table; files at the far spot (`--pose far`), the vein, the patch and the hall; digs 15 to 25 ms at 64, 49 to 63 at 512, 69 to 90 at 4096 (target 33); from nothing at the level 14, 51, 82 s; the RTX temporal pass about 3.0 ms on bricks against 0.94; no test; [`logs/0059.md`](logs/0059.md) | ffbf032 |
| 7b: The owner's pick of the brick level | human | done 2026-11-08 on ffbf032's release binary, RTX, `live/session7b.sh`: 8/4096 stands, "Bricks everywhere" (the amendment of 2026-11-08, b991868); first frames 1065 to 4003 ms (files); digs on 8/4096 83 to 106 ms, the hall's 60 to 178; [`logs/0059.md`](logs/0059.md) | 34ad3b4 |
| 8: The queue, the rings and the reach gate | dev | done under exploration mode, as re-read at 7b: `--light queue` (and `--light baked` streamed) fills the brick level at 8/4096 from nothing, nearest then by view, behind the reach and emitter-range gates, a 1.5 ms slice a frame; the listing's decode on the stream workers (ADR-0017's hook); RTX first frames 2500 (vein), 2415 (patch), 1893 ms (hall), loaded; the hall empty at 112 s; the vein flight's no-value share up to 0.456; [`logs/0059.md`](logs/0059.md) | d45df55 |
| 8a: The queue's throughput | dev | done under exploration mode: an entry's samples over a 64-lane workgroup, reduced in workgroup memory (same samples and seeds); the slice submitted without a wait, two in flight, sized by its own GPU timestamps and read back when mapped; the pre-roll overlaps two submits; rounds 64; `--queue-file`; RTX 1.3 to 1.6 x 10^7 samples a 60 frames (Phase 8: 4 to 7 x 10^6), first frames 2313 (vein), 2139 (patch), 1199 ms (hall), empty after 130, 142, 22 s; the vein flight's no-value share up to 0.069; the hall's store against 7a's 8/4096 file: 4 of 179 046 words differ, by 0.101 levels at most; Cezanne 1.3 to 1.7 x 10^6; no file re-baked but the hall's queue store; `live/session8.sh`; [`logs/0059.md`](logs/0059.md) | 3610e2f |
| 8b: The owner's look at the live queue | human | done 2026-11-08 on 3610e2f's release binary, RTX, `live/session8.sh`: the owner's four statements and the pick "coarse first + bigger initial area" in the amendment of 2026-11-08 (9364b50) and the log; first frames 2362 to 3842 ms; the hall empty at 116.7 s; digs 11 to 26 ms; screenshot `live/session8-hall-dark.png`; [`logs/0059.md`](logs/0059.md) | 52de2bf |
| 8c: Coarse first, a larger near bake | dev | done under exploration mode: before the first frame the chunks within `--near` 2 (Chebyshev) at 8/4096, then a coarse pass at `--coarse` 256 over the full chunks mirrored then (5 s box past the near ones), flagged in the chunk table; the slice refines coarse chunks nearest first, fading from the coarse value; a chunk with no block opens first; RTX first frames 7137 (vein), 7110 (patch), 1839 (hall), 11 547 ms (cave); all at 4096 after 157.0 s (vein), 20.4 s (hall); no value 0.0000 at every line; the flight's coarse share up to 0.0676 on its path; Cezanne 13 524 (vein, boxed), 6690 ms (hall); [`logs/0059.md`](logs/0059.md) | 03c73c2 |
| 8d: The owner's look at coarse-first | human | done 2026-11-08 on 03c73c2's release binary, RTX, `live/session8c.sh`: "longer start is acceptable"; a dig far from the spawn changes the light around the hole "drastically"; the flight's change "acceptable"; "aliasing got much worse in hall"; first frames 3131 to 10 104 ms; digs 13 to 39 ms at 2.3 to 5.8 x 10^4 samples; [`logs/0059.md`](logs/0059.md) | committed with 8e's row |
| 8e: The dig on the live queue (the orchestrator's lane prompt after 8d) | dev | done under exploration mode: under the queue a dig re-bakes at the queue's settings (8/4096) in place of the reference's default (64 a brick direction); `store_brick_value` by `select` masks (`shader_portability` passes); `queue_dig_readings` (RTX, far pose, 462 ring bricks): the queue's arm equal words 1.0000 against a fresh 8/4096 bake in 15 ms, the old arm p90 8.95 levels touched, 6.96 untouched; the 34 s HUD frame a 35.6 s gap between frames in the log; `live/session8e.sh` at native, far normal 1..2; the owner's window 2026-11-09: "the quality is finally there", "dig is fine", the hall crashed in `advance_fades` after a dig (fixed: a fade ends when a dig relists its chunk; the hall again on 57efe3c: "all seems good", 14 digs, no panic; `B` "lags a bit but seems fine", 230 to 701 ms a press); [`logs/0059.md`](logs/0059.md) | d8d9b75 (wip), 3ac5662, 57efe3c, committed with this row |
| 9: The dig under a budget, the three rings | dev | not started | |
| 10: The resolution cut | dev | not started | |
| 11: The owner's look at the resolution cut | human | not started | |
| 12: Budgets, the default and the end strip | dev | not started | |
| 13: The owner's window | human | not started | |
| 14: The deletion | dev | not started | |

### Notes

_(Deviations from the plan, with the commit, stated without justification. Done-whens not
satisfiable as stated, and what
was done instead. Followups noticed and not acted on. One line each. Empty is fine.)_

- Phase 5a (owner's process change, 2026-11-08): done-whens 1, 2, 3, 6 and 7 not read as bars, the full gpu tier not gating, done-when 4's failing case shown before the change; observations in `logs/0059.md`, Phase 5a.
- Phase 5a: 4 gpu-tier tests outside the Files touched assert the per-layer composite or the fallback behind glass and fail (`settle::a_hit_with_no_face_takes_the_fallback`, `settle::a_still_fallback_pixel_holds_still`, `vxe-render::translucent` x2); not changed; their new claims are the architect's.
- Phase 5a: with the lamp on at the hall, 0.307 of the pane's pixels read over 250 against 0.0077 without the pane (done-when 2's figure, observed, not a bar).
- Phase 5a: the patch's store files and every `independent 1024` file keep the old source hash.

- Phase 1 (this row's commit): the stop rule fired (done-when 7: the kept arm projects a whole level over 60 minutes); Phase 1b is not run.
- Phase 1: the running sums, the stop and the `rgb9e5` packing live on the CPU in f64; the working buffer holds one round's sums (layout in the log).
- Phase 1: the pool's binding is clamped to what `limits_within` requests, 220 MiB (`TABLE_BINDING_BYTES`), not 256 MiB; lavapipe's 128 MiB.
- Phase 1: `--light baked` is refused without `--level` or `--source showcase`; `--bake-radius` without `--light baked`.
- Phase 1: under `Light::Baked`, `Frame::set_lighting` turns off the table's blend, fill, preload and age-out, and the encode skips the guard and the chunk sweep.
- Phase 1: the count reading's deepest count is 65 536 (not 262 144), the reference is the mean of a view's four 65 536 runs, the clamp arms share the unclamped arms' samples, the zero share is read on the views' faces, and the stop was read at the vein only.
- Phase 1: the runner's comparison against the table is a separate `--light table` launch (no mode key in Phase 1).
- Phase 1, noticed: the stop's wall rate (310 ns a sample against 42 in fixed runs) is set by one waited submit a round for a chunk's last faces.
- Phase 1a (this row's commit): the bake's module is march.wgsl and light.wgsl with two lines replaced in `bake.rs` (`bake_source`): the candidate table read (`table_slot`) and the LOD walk's camera chunk (`lod_level`) come from the entry's job slot. The two files are not edited.
- Phase 1a: a face takes its rounds to `N_MIN` in one submit, then one; past that, rounds up to the count its standard error expects (`1 / sqrt(n)`) fill a submit's room once no job is left to open. The stop's arithmetic is unchanged.
- Phase 1a: the slicing test's four ways run on `cache_box`, where every face ends at its first round at the test stop, and again on a two-chunk slab under a small emitter at the chunk border, where faces end at 9 counts.
- Phase 1a: the count reading's stop part takes `VXE_BAKE_SEEDS` and `VXE_BAKE_FIXED`; the census part prints each view's piece at `VXE_BAKE_RADIUS`.
- Phase 1a, noticed: Phase 1's log reads `the_stop_spends_where_the_noise_is`'s face beside the emitter at se 1.673; 70305ba's tree prints 1.168, as this tree does.
- Phase 1a, noticed: in `apps/flythrough/src/main.rs`, `start_camera`'s doc comment sits above `bake_extent`.
- Phase 1b, the owner's remark (for the architect at Phase 3): the comparison against the table is "striking", but the load of even the small piece takes too long; Part 2's cuts.
- Phase 2 (9a55841): the dig's re-bake runs inside the frame and does not pump window events, so a long hitch (a `B` press, about 2 minutes) may draw a compositor "not responding".
- Phase 2 (9a55841): the glass key's helpers live in `apps/flythrough/src/main.rs`; the plan's files do not name the fly-through's `lib.rs`.
- Phase 2 (9a55841): a `--bake-file` refused by its key bakes and rewrites the file (the message names the field).
- Phase 2: `.gitignore` is unchanged; `live/` is already ignored.
- Phase 2: the whole showcase's pre-roll (24.5 min) was read loaded (load to 29); not retaken, it has no bar.
- Phase 2: the strip's `G_plane` is the store's cells only (the noise part's); the strip reads display luma against the table's frame 4096.
- Phase 2 (this row's commit): the flight reading flies the settle rig's steady flight at the vein and the patch and the hall's view line; the far dig is the rig's (from the flight's start, the hit nearest 100 voxels); a first run of each read the pose's own view and is in the log.
- Phase 2: the yaw reading's table row is native from empty (192 + 64 frames before the yaw); the FSR 2 row is Plan 0057's, not re-read.
- Phase 2: the glass reading's pane fills the view at the hall (3 486 320 of 3 486 340 pixels changed by it), so its counts are the whole frame's.
- Phase 2: the strip's last views and the hall's flight lines were read at a load up to 4.8 (a browser outside this lane), labelled loaded.
- Phase 2: no `B` press is over 5 minutes (1.9 to 2.5); Phase 3's `B` step offers up to four presses, each hitch stated.
- Phase 3: the first window exercised neither the pane nor `B` (no pane or bench line in its logs); a rerun (`live/session3b.sh`) took both, one press each, and a second pane press placed 0 voxels.
- Phase 3, for the architect: a big white blob on the glass pane when looked at directly (`live/session3b-glass-blob.png`, untracked); not the parked held-fallback item.
- Phase 3, not answered: (d) the fast turn on the store, (e) the voxel step and shadow edge, (f) the lamp and its bounce by name; (a) to (c) read from "everything looks good".
- Phase 4 (8df5c6c): two of the stop rule's three stops fired: no arm meets the offer's bars, and the RTX reads over 20 ns a sample at 300 000 at the hall; the owner's answers are the amendment of 2026-11-06 (d029596).
- Phase 4: the lattice form is a Cranley-Patterson rotation of the global-side sequence by the R2 lattice point of the face's plane coordinates; the filter runs on the CPU after the bake, and a dig re-filters around its ring.
- Phase 4: the grain's counts come from one 2048-sample run's snapshots (32 to 2048), and the filter is applied to the f32 means, not to packed cells.
- Phase 4: the cost reading times the dispatch by GPU timestamps beside the wall, cold and after a warm-up bake; entries of 32 and 512 samples.
- Phase 4, the glass blob (for `docs/backlog.md`, beside "A held fallback still moves on screen"): not the lamp's spot. 44 % of the pane's pixels read over 250 with the lamp off, 407 px off the lamp's axis; the store, the table and the bounce read alike; the accumulated lighting there is unchanged by the pane, so the brightness comes from the composite of the translucent glass row.
- Phase 4, noticed: on the patch the filter lifts a one-face-wide dark row from 0 to 39 to 50 levels (the harm rows).
- Phase 4 (b663570): `the_filter_stops_at_a_crease_and_softens_a_shadow_by_one_face` read 5.99 s solo at load 1.36 (a series' first run), 4.2 to 4.3 s at load 2.2, 3.65 s in the gpu tier, and 3.55 s twice at load 0.6 (2026-11-07); the test is unchanged.
- Phase 4: done-when 4's cut files are baked after Phase 4a, whose fix edits march.wgsl (`bake::source_hash()` covers it); the glass blob went to Phase 4a, not to `docs/backlog.md`, as the amendment of 2026-11-06 says.
- Phase 4a (this row's commit): done-when 2's second half cannot be read as written: the pane is 2 voxels thick and its pixels enter 2 to 4 layers (transmittance p50 0.0256), and with the lamp off the glass has no light of its own, so the pixel is `0.4^k * behind`; `|pixel - ((1 - a) behind + a albedo light_at_glass)|` reads p90 19.69 levels against 2. Stopped for the architect; read against the k-layer model after the amendment of 2026-11-07 (this row's commit).
- Phase 4a: the glass face has no light but the headlamp's (no store cell, no fallback); with the lamp off the pane reads black over a dimmed scene, and with the lamp on the lamp's spot on the glass is unchanged (0.4358 over 250 at the shot).
- Phase 4a: `a_glass_pane_does_not_brighten_what_it_covers` moved to one rig and a pane of the view's size (the claim and assertions unchanged); it reads 5.50 s solo quiet as a series' first run, 3.93 s the next, 5.73 s in the gpu tier loaded.
- Phase 4a: the hall's reference file was re-baked under the new source hash for the readings; its blocks are bit-identical to the file before the fix. The vein's and the patch's files, the cut files, the Phase 5 binary and `live/session5.sh` wait for 4a's close.
- Phase 4, done-when 4 (after 4a, 2026-11-07): the cut files (independent 1024 and 512, filtered) at the vein, the patch and the hall, baked at 72d44f9's binary, with the reference files re-baked under the new source hash (blocks bit-identical); `live/session5.sh` written. Times in the log.
- Phase 4a (72d44f9): the k-layer model's `k` is counted by a CPU voxel walk of the pane's box (`layers_on_ray`), not read from the frame; the layers' own terms are the glass term read, their sum.
- Phase 5 (3ac2832): the first two glass runs placed no pane (no `pane` line; their F8 and F9 lines printed, so the `1` key did not reach the window); the third try placed it. (a) to (c), (e) to (g) not answered by name.
- Phase 5a (this row's commit): stopped before code; the facts are in the log's Phase 5a section.
- Phase 7a (this row's commit): `apps/flythrough/src/lib.rs` (`FAR_POSE`, `START_POSES`, for `--pose far`) is outside the phase's Files touched; `--show NAME` added to `main.rs` for the owner's GPU-time reading; the rings left out of the F cycle (still built); the samples are a brick direction's at either cell; the cell-4 record and the read across cells are dev's choices (log, Phase 7a).
- Phase 7a: the far spot's yaw and pitch are dev's guess (the flight's heading, +10 up); its reference is a radius-1 piece and leaves 1 447 956 of 3 486 340 pixels of the view with no value.
- Phase 8 (this row's commit): built as the amendment of 2026-11-08 re-reads it, not as its phase block: no tiers, rings, tier-ups or pool-pressure eviction; done-whens 1 to 5 and 7 not written, 6's bars not read, 8's figures read loaded (exploration mode).
- Phase 8: the fade is a CPU lerp of each record written to the pool every frame, not a lerp in the job's dispatch; nothing reaches it in this phase (the queue opens only chunks without a block; a dig keeps its in-frame re-bake, written directly).
- Phase 8: the slice is one waited submit a frame after a wait for the GPU to go idle, so while a job is open a frame pays its GPU time plus the slice; the slices take rounds of 4 samples and the pre-roll rounds of 32 (the bake shader draws an entry's samples in one invocation).
- Phase 8: `apps/flythrough/src/lib.rs` (`VEIN_FLIGHT_POSE` for `--pose vein-flight`) and `crates/vxe-world/src/lib.rs` and `tests/stream.rs` (the hook's export and test) are outside the phase's Files touched; `--glide V` added for the flight reading.
- Phase 8: the store's chunk table follows a moving window by a CPU rewrite of the table, no shader change; the light-sample ranges are held to `r_full + light_radius + 1` on the streamed world.
- Phase 8c (this row's commit): a coarse job draws a brick direction's samples on `ceil(N / 64)` of its faces spread through it (4 at 256), not on every face; the record is their mean.
- Phase 8c: the pre-roll's coarse pass covers the full chunks mirrored when the near chunks settle, and ends 5 s past them at the latest (the streamed cave mirrors 17 077 full chunks); a chunk it leaves takes its coarse pass at the head of the live queue. It ended by the box on the cave (RTX) and the vein (Cezanne).
- Phase 8c: a chunk with no block that the mirror gains within `--near` of the camera takes its first pass at 4096, not coarse.
- Phase 8c: `shaders/temporal.wgsl` (the coarse flag's mask and the coarse-read counter) and `apps/flythrough/src/hud.rs`'s test fixture are changed; `temporal.wgsl` is outside the phase's Files touched.
- Phase 8c: the first frame at the vein and the patch is 7.1 s on the RTX (the amendment's arithmetic 3 to 4 s): the coarse pass reads about 4.2 s with the GPU at 26 to 40 % (the main thread's job bookkeeping; the log's attempts).
- Phase 8c, noticed: `vxe-render::shader_portability`'s `no_shader_stores_through_a_runtime_vector_index` (gpu tier) fails on `store_brick_value`'s stores, in the tree since Phase 6 (7c105c5).
- Phase 8, noticed: at the hall the near bake leaves 0.97 of the view's pixels with no value (the hall's walls are beyond the 27 chunks), and the share falls to 0.019 by frame 7200; on the Cezanne the slice's cap sits at its floor.

### Close triggers

_(Facts for the architect. No recommendations, and no suggested version bump.)_

- **What shipped:** feature / fix-only / docs-chore-only
- **Public API changed:** types, traits, functions, shader interfaces, config (list them, or none)
- **Budget readings:** each `nfr.md` figure touched, with its measured value, machine and commit
- **Gate at the tip:** the commands run (build, nextest, doctests, clippy, fmt, deny), exit codes,
  test counts
- **Outstanding `human` phases:** which, or none

## Followups

- **The antialiasing plan**, parked by the owner on 2026-11-06 ("Barely noticed" at native): `docs/backlog.md` keeps it, with Phase 2's fast-yaw rows, together with the far-normal plan Plan 0056 owes.
- **Plan 0058's refresh** (architect, at this plan's Phase 14 on "baked"): its emission-scale phase
  and ADR-0045's kept emitter become the dominant material of this plan. Its analytic lights stand.
- **ADR-0043's Outcome and supersession,** at this plan's close, on "baked": the table, fill,
  preload and guard, with the lamp channel, the nearest read and the empty screen history kept.
- **A brick-level early out** for faces unlit at a low tier, if Phase 1's unlit share is large and
  Phase 8's slice is short.
- **A cone from the box for ring 3,** if the owner sees far re-bakes starve near ones while digging
  repeatedly.
- **The light pass's lamp-only reduction,** and whether the gradient pass needs its emissive parts
  at all under a store: dev removes what a deletion leaves dead in Phase 14; anything larger is Plan
  0044's.
