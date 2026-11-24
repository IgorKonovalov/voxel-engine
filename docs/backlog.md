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

### The record by rule at every opening, and the bake's key ray at its cap (2026-11-23)

From [Plan 0096](plans/done/0096-the-bake-queues-drain-tiers-a-drain-slice-the-step-and-the-slow-adapters-light-pass.md) Phase 10 (its section "Phase 10 after its reading"; readings in `docs/plans/logs/0096.md`, "Phase 10a"). A small plan of its own, to run ahead of Plan 0099's Phase 1 (ruled at Plan 0096's close, 2026-11-23, v0.71.0). **Drafted 2026-11-24 as [Plan 0101](plans/0101-the-record-by-rule-at-every-opening-an-audit-on-the-real-scenes-first-then-the-three-exclusions-lifted.md) with [ADR-0085](adrs/0085-the-zero-record-is-ruled-on-the-mirror-as-it-stands-exact-by-the-re-list-not-by-the-columns-finality.md) (proposed); awaits the owner's go.** The draft carries the rule alone (the key ray is Plan 0100's Phase 2), puts the audit on the real scenes ahead of any behaviour change, and recommends running after Plan 0099 closes instead of ahead of its Phase 1 (its question 1); this entry leaves when the owner answers.

- **The reading:** ADR-0080 rule 1's zero record is applied only to a first bake with a full reach under final columns, so it answered 266, 241, 3 and 6 chunks on the four runs; of the jobs opened before the in-view landing it would have answered 11.4 % (RTX forest), 20.0 % (RTX hills), 12.0 % (RENOIR forest) and 10.1 % (RENOIR hills), and 5.1, 8.9, 5.1 and 6.5 % of the samples, mostly chunks with a partial reach. The win is bounded by those shares: about 1.4 s of the RENOIR forest's 27.3 and under 4 s of the RTX's 36.5 (arithmetic).
- **First, the bake's key ray:** a key ray that reaches the job's radius without rock counts lit (`bake.wgsl`), so a floor under a closed cave taller than the radius would bake sunlit where the column clause says no sky. **Confirmed** by Plan 0096 Phase 10c (`7b24002f`, lavapipe: the job's record reads key 255 on 64 of 64 floor faces under a closed roof, the rule's 0). The fix is the column clause at the key ray's cap; it changes record bytes and what ADR-0079's bake writes, and ADR-0079 is accepted, so **it takes an ADR superseding that clause, drafted with this plan**. An image question: the owner's word on its place is owed (this plan's first phase, recommended, or a fix pass on Plan 0098). **2026-11-23:** [Plan 0100](plans/0100-a-sun-fixed-in-the-world-the-key-shadow-a-face-at-a-time-the-coarse-tier-held-still-and-sun-under-cover.md) (draft) proposes to take the key ray's fix as its Phase 2 under ADR-0084, which supersedes that clause; this small plan then keeps the rule at every opening alone and starts from that commit. **Answered 2026-11-24 (owner, Plan 0100's question 1):** the fix goes to Plan 0100 as its Phase 2.
- **Then the rule at every opening:** Plan 0096's Phase 10b as written, cases (1), (2) and (3), with fact (i) restated (every change of a top is a mirror change the re-list reads; a full chunk arriving over its LOD can lower a top) and the audit on the real scenes (every ruled chunk also baked and compared, 0 differ). ADR-0080's rule 1 sentence on the column map's finality is superseded there.

### The plan after 0098: a sun fixed in the world (2026-11-23)

Taken by [Plan 0100](plans/0100-a-sun-fixed-in-the-world-the-key-shadow-a-face-at-a-time-the-coarse-tier-held-still-and-sun-under-cover.md) and ADR-0084 (proposed), approved 2026-11-24 (owner). Plan 0098 has closed and merged (v0.72.0), so its lane is open, beside Plan 0099's. Its three subjects: the noon terraces' key shadow under the jitter, the coarse tier's wobble on near trunks and leaves, the sun under cover.

### The bake step off the main thread (2026-11-22)

Taken by [Plan 0099](plans/0099-the-bake-step-off-the-main-thread-a-bake-thread-write-lists-and-the-in-view-drain.md) and ADR-0083, approved 2026-11-22; its lane after Plan 0087.

### The sun's shadow near under the calm defaults: soft, true under cover, no screen history (2026-11-23)

From [Plan 0098](plans/done/0098-a-calm-light-by-default-no-screen-history-smooth-sources-to-the-mid-range-and-the-fsr-2-turn.md)'s "Amendment at Phase 4's findings" and its window W3 (2026-11-23). The requirement the owner set at W3 (sun patches under canopies and through openings that are steady, the same at a distance as near, present before the surface is seen and after a turn) is **taken by [Plan 0100](plans/0100-a-sun-fixed-in-the-world-the-key-shadow-a-face-at-a-time-the-coarse-tier-held-still-and-sun-under-cover.md)** as its third subject, with ADR-0084 (proposed); light history on was rejected as the way. What stays here, none of it read, for a plan of its own when the owner asks for a penumbra or a consumer renders with no bake:

- **A penumbra of the disc's true width.** Under the defaults the key light's visibility on a world face is the column map's horizon at every distance: a hard edge but for the horizon's ramp. A true ray near (`penumbra approach`) seams against the column where they disagree (W1), and a running mean is the history ADR-0082 removes.
- **A shadow's edge on an entity or a field surface:** the raw disc sample, one ray a frame (inferred, unread).
- **The sky term under cover with no store bound**, or before a bake lands: such a face has no sky light (`outdoor_forest_afternoon.png`). The sky ray cast where no record is fresh is never shown with the history off, but the dig hold reads it, so skipping it needs the hold's box as its exception.

The W3 verdict's words and stills are in Plan 0098's "Amendment at W3" and its `live/window98c/`.

### A dig's landing: the slight colour change, and the dig delayed until its light is ready (owner, 2026-11-24)

From [Plan 0098](plans/done/0098-a-calm-light-by-default-no-screen-history-smooth-sources-to-the-mid-range-and-the-fsr-2-turn.md)'s window 98e. Under `--dig-hold on` the owner accepts the dig on the RTX and on the Radeon at Portable ("so much better, not ideal but acceptable"; "change of color is there, but slight"; the Radeon "almost good", "same slight color change"). What is left is the held estimate's distance from the baked record, shown as one 4-frame fade when ring 1 lands one or two frames after the dig, and again smaller when the chunk's second job lands. Parked, not needed now (owner). Pick up if the owner asks for it or a consumer's digging reads as a flicker.

- **The measured form of it:** the soak image test's sequence B against the queue's full landing (the form before the owner's pick of the groups at ring 1's landing) reads 3.01 % of the hole's pixels over 4 levels above and 4.72 % below their settled value in the two frames before the next dig; under 32 levels nothing (0.33 / 0.18 %). The histogram is in `docs/plans/logs/0098.md` ("Step 1: the histogram").
- **Way 1, a held value nearer the landed one:** the estimate is a value a cell and direction from 12 short rays and the removed surface's record (`bake/held.rs`); more rays, the brick's own emitters, or the chunk's coarse answer as a second term. Cheap to try on the rig; the error table of Plan 0098's Phase 14 is the instrument.
- **Way 2, the owner's delay (staged records):** the old picture is held until the dig's light is ready, a lag of 2 to 3 frames on the RTX and about 6 on the Radeon. Feasible only with staged mirror records (a dug chunk written to a fresh record and fresh slots, the table's flip delayed, the bake reading the staged chunks, the column map carrying the staged tops for the bake alone); a two-phase commit in `gpu_world` under ADR-0013 and ADR-0022, so a plan with its own ADR. The design and the five rejected lines are in Plan 0098's "Amendment at window 98d" and ADR-0082's Outcome.
- **Beside it, two switches with no fade:** a new dig into a chunk whose dig fade still runs ends that fade with a step (`take_boxes`, `bake/queue/digs.rs`), and a stale ring record steps to its re-bake's in one frame (read under 4 levels in Plan 0098's Phase 8).

### Distant trees that do not wobble: a stable far representation (owner, 2026-11-22)

What is left of the wobble after Plan 0098's calm picks is distant trees while moving: sparse
leaves at 15 to 60 m flip between jitter positions under FSR 2 (the plan's readings log; a
denser canopy wobbles less at window W1, and the owner rejected changing models: "we don't know
what models will be used in the end and can't simplify models"). The owner's direction: do not
re-decide distant things every frame; "just not change them, or make this change less frequent.
Maybe use rasterised objects when they are in distance, and voxels only for something that is
close and we can reliably calculate it". Three ways, none measured:

1. **A cached or rasterized stand-in at distance** (impostors, or a far band rendered to a
   texture and refreshed rarely). Calm because the texture is filtered and nothing changes
   between refreshes. Costs: invalidation on edits and on the sun's move, the swap to voxels on
   approach, a raster path (ADR-0001). At 15 to 60 m a walk changes the view fast, so it fits
   best beyond that band. **Owner, 2026-11-22: folded into
   [Plan 0033](plans/0033-far-field-clipmap-to-a-kilometre.md)**, which picks the far
   representation anyway; its next re-read takes this in as an arm.
2. **Partial coverage in the far voxel data**: a canopy cell answers a share of leaf, the pixel
   blends it with what is behind; independent of where in the pixel the ray went; any model;
   edits ride the LOD mips. Risk: soft trees (the owner disliked the solid canopy's look).
3. **No jitter in the far band**: far rays through the pixel centre. The cheapest test of the
   direction (with the upscaler off he saw no wobble on a turn, "not right" as a picture). It is
   Plan 0098 Phase 5's `far jitter` arm, not built there because the hit pass has no previous
   hit to read; it wants a small phase of its own.

Order proposed by the orchestrator, not yet the owner's: 3 first, then 2 if 3 reads calm but
ugly; 1 with Plan 0033.
**W2, 2026-11-23:** under the calm picks the owner calls the distant wobble "not too bad";
`far light wide` is "not calmer" (it stays a setting, off). No arm of Plan 0098 is picked, so
this entry is where the far trees stand. Plan 0100's second
subject (the coarse tier's wobble on near trunks and leaves) is the near half; this entry is
the far half.

### Windows draws the whole image two to three times darker than Linux

Noted 2026-11-15 from the owner's Windows run (same laptop, v0.54.0, RTX on Vulkan, the window
on the AMD iGPU's display): every scene, the overall brightness down by about 2 to 3x, not only
the dark tones. A decode without its encode (`pow(x, 2.2)`: 0.5 to 0.22) would fit; by a code
read the present pass matches the surface format, but the format picked is not logged. Also
suspected: the panel's backlight on Windows (AMD Vari-Bright, power plan). What it takes: log
the surface format and colour space at start (and into `system.json`/`run.json`), compare a
Windows and a Linux screenshot of the same pose by pixel values, and swap `fs_srgb`'s
`pow 2.2` for the exact sRGB decode. Pick up before the next tester archive. **Code side done 2026-11-16** (chore-surface-format: the format and colour space logged and in `run.json`, 8-bit UNORM preferred, the exact decode): by code read the Windows run used `Bgra8Unorm` and a straight copy, so no code path explains a uniform 2-3x drop. Left: a Windows screenshot and a Linux one of the same pose compared by pixel values (equal bytes: the display or OS, e.g. HDR's SDR brightness, Vari-Bright).

### `--adapter NAME` that matches nothing falls back silently

Noted 2026-11-17 by the Plan 0032 lane (Phase 6 readings): `--adapter radeon` and `--adapter rtx`
match no adapter name on the dev box and the fly-through runs on the RTX without a word; the
iGPU needs `--adapter AMD`. A reading taken that way is labelled with the wrong machine. Fix as a
chore: refuse (or print the adapters and exit) when no adapter matches, and accept the short
names the tests' `VXE_TEST_ADAPTER` takes.

### A headless run at `--residency 3,4` hangs before its first frame

Noted 2026-11-17 by the Plan 0032 lane (`docs/plans/logs/0032.md`, Phase 3): a headless
fly-through with `--residency 3,4` hangs after the live queue starts and before frame 1, on the
plain cave as well as the outdoor source, so it predates that plan; `6,8` and `8,12` run. Not
yet bisected. Take it as a chore when a small residency is next needed (a test rig or the
Portable profile's floor), starting from which stage waits.

### The profile row's `restart for:` names live fields

Noted 2026-11-17 at Plan 0080's close review: after a toggle, the render profile row lists live
fields as waiting for a restart (window session 80c, the lamp flipped off, then `F4`:
`profile: performance, restart for: lighting.lamp, build.record_chunks`). `Run::cycle_profile`
(`apps/flythrough/src/run/profile.rs`) takes the pending list from
`RenderSettings::changed_fields` against the live settings, toggles included, so any `Live` or
`Settle` field the owner changed reads as restart-only, and `build.record_chunks` (which the
fly-through sizes itself) shows on every cycle. Plan 0069's behaviour. Fix as a chore: filter
the pending list to the `Rebuild` class and to fields the fly-through does not set itself, with
a test that flips the lamp and cycles. Pick up with Plan 0073's settings pages, which show the
same classes.

### `StampedSource` shares its prefabs across placements

Noted 2026-11-17 by the Plan 0032 lane (Phase 8, `docs/plans/logs/0032.md`): `vxe_world::StampedSource`
keeps its own rotated copy of the prefab for every placement, so a forest of about 3,850 trees
would hold 3,850 copies (estimated 1 to 2 GB, not measured). The lane wrote a fly-through
`ForestSource` over 64 shared prefabs instead. Engine follow-up: placements reference a shared
prefab and rotation (an `Arc` and an orientation), so a dense generation-time scatter costs its
distinct prefabs, not its placements. Pick up before the next scatter (Plan 0032 closed 2026-11-17 without it).

### The menu with a gamepad, in a window

Noted 2026-11-17 (owner): Plan 0080 closes without its window's pad step (no pad was at hand;
`docs/plans/logs/0080.md`, Phase 7). The headless tests `the_pad_walks_the_menu` and
`the_pad_edits_a_number` cover the mapping. Owed: one windowed run with a pad connected, Start
through the tree, South into the dig radius's edit mode, the D-pad stepping and repeating, East
back, Select for the stats; at the next window session that has a pad.

### The gate tier's tests over the 10 s line, and its wall time

Noted 2026-11-17 (Plan 0032's fix pass): a loaded gate run over flythrough, vxe-render and
vxe-tester showed 22 tests over the 10 s line, all in flythrough GPU binaries (gpu_golden,
gpu_fsr2, gpu_queue, gpu_loading and others); the log is archived at
`~/Work/vxe-live-archive/plan-0032/m1-gate.log`. On lavapipe a `Frame::new` costs 2.4 to 2.8 s
and a 16-frame mean about 0.08 s, so sharing one Frame across a test's arms cuts more than
fewer frames (0032's m1 fix). The whole gate tier read 937 to 1214 s at the last closes, over
the 12-minute proposal in `docs/nfr.md` section 7. Owed: a test-time chore that reads
`scripts/test-times.mjs` on a quiet gate run and trims or soaks each test over the line.

Plan 0083's close (2026-11-19, `docs/nfr.md` section 7): 1263 s loaded, 29 tests over the line.
Five are the plan's, each generating the castle's world in a test build (21.6 to 26.4 s):
`the_spider_builds_on_every_source_castle` and `the_castle_world_maps_the_structures` read only
the castle's material table, which `setup::world_table(&args, &setup::materials(&args))` gives
without the build (`table_of_run` in `run/structure_tests.rs` joins `Pending::StreamBuilding`
instead); `the_built_castle_has_an_icon`, `the_ghost_is_a_coarse_copy_in_its_frame` and
`the_castle_loads_through_the_scene` need the built castle and are soak marks or one shared
build. About 100 s of serial time.

Plan 0086's close (2026-11-19, `docs/nfr.md` section 7): 1792.6 s quiet, 61 tests over the
line, none of them new; the wall is the light pass's pipeline compile on lavapipe (+2.2 to
2.6 s on every test that builds a frame, the penumbra spike's default-off branch the largest
step), accepted by the owner and taken by Plan 0094 with the pipeline-cache entry below. Two
smaller items from the same close: `gpu_world::columns::tests::the_first_flush_of_the_streamed_window_is_cheap`
(c682fa9d) bounds a wall clock in the gate (under 1000 ms; 189.5 ms quiet in debug on the dev
reference), which a loaded CI runner can trip, where a count of the sheet lookups per tile
would not; and two of `gpu_sun`'s sky tests the plan rewrote (Plan 0032's names,
`a_closed_box_takes_no_sky_light` 13.7 s, `the_open_floor_reads_pi_under_a_uniform_sky` 13.3 s)
read over the line under the compile and go under it once the cache lands, or take the soak
mark.

Plan 0098's close (2026-11-24, `docs/nfr.md` section 7): 2153 s loaded, 2011 s at a load of 2.2
on the same branch a day earlier, 74 tests over the line; Plan 0096's lane gate the day before
read 1437 s. Not attributed: the CPU tests the plan does not touch read 1.25 to 1.4 times
longer in the same logs, and the plan also grew `temporal.wgsl` and `pair.wgsl`, which every
test that builds a frame compiles. Owed with the chore above: a quiet gate of main before
Plan 0098's merge and of the merged tip, back to back, per-binary sums compared
(`scripts/test-times.mjs`); if the shaders are the cause, the engine's pipeline cache entry
below is the lever.
**Taken 2026-11-24 by the gate-time chore** (branch `chore-gate-time`, `docs/nfr/07-startup-and-build.md`):
the quiet base at `193cae15` read 2145.0 s, the lavapipe group's serial sum; the 1.25 to 1.44
growth is the CPU's clock capped at its 3.3 GHz base since the AC drop of 2026-11-23 (no quiet
run of main before Plan 0098 at the cap was taken; the ratios match the cap). The group at two
threads and eight tests' independent arms on threads read **1133.2 s at `909a2552`, under the
line**, no test moved tier. Left: the ten GPU tests over the line at full clock by the ratio,
listed in that file, each the gate's only test of its mechanism; the per-test fixed cost
(pipeline compilation) is the engine pipeline cache's.

### The light placement's eviction scan in a windowed glide

Taken by [Plan 0096](plans/done/0096-the-bake-queues-drain-tiers-a-drain-slice-the-step-and-the-slow-adapters-light-pass.md) Phases 7 and 7b (closed 2026-11-23, v0.71.0; the owner's glide: "no stagger"). What is left, the wanted set's collection and sort at every sync, is the first item of "Plan 0096's leftovers".

### The CPU settle outdoors: the bake queue's step on a world with no emitter, and the forest's sync p99

Taken by [Plan 0096](plans/done/0096-the-bake-queues-drain-tiers-a-drain-slice-the-step-and-the-slow-adapters-light-pass.md) (closed 2026-11-23, v0.71.0) and [ADR-0080](adrs/0080-the-live-queue-bakes-what-the-image-reads-a-record-by-rule-a-tier-by-size-on-screen-and-a-drain-slice.md). What it left, the step's per-job cost while baking (`list`, `collect`, `submit`, `fades`, `summaries`; 20.7 / 55.0 ms p50/p99 on the RTX forest, loaded), is [Plan 0099](plans/0099-the-bake-step-off-the-main-thread-a-bake-thread-write-lists-and-the-in-view-drain.md)'s.

### The sync's `alloc` stage: the sync plan after the eviction scan

Read, not met, by [Plan 0096](plans/done/0096-the-bake-queues-drain-tiers-a-drain-slice-the-step-and-the-slow-adapters-light-pass.md) (closed 2026-11-23, v0.71.0: `alloc` p99 6.42 ms against 3.0). Merged into "Slot words packed on the workers" (Performance), whose mechanism is the fix.

### The streamed benches on Windows: CPU frames of 20 to 256 ms on a desktop

Done 2026-11-16 (chore `chore-default-workers`): the 0.51.0 stutters were the build, not Windows (the tester's rerun on v0.54.0: castle wall p50 4.2 ms from 256), and the fly-through's default `--workers` is half the logical cores. No sweep between 4 and 6 workers was taken; the next tester archive reads the default. The readings (three Windows runs and one Linux tester's archive; the files under `live/tester-2026-11-15/`, `live/windows-run-2026-11-15/`, `live/tester-2026-11-16/` and `live/tester-2026-11-17/` in the main checkout) are in this file's history at `193cae15`.

### Done: test tiers by marks, not by copied lists

Done 2026-11-10 (50b2193, 4a15e60): GPU test files are `tests/gpu_*.rs`, long series `#[ignore = "soak"]`, kept by `scripts/check-test-tiers.mjs`.

### The live queue's order and cancellation, after Plan 0059

Taken 2026-11-10 by [Plan 0060](plans/done/0060-the-live-queue-by-the-view-and-withdrawing-passed-work.md) and [ADR-0048](adrs/0048-the-live-queue-orders-by-the-view-and-withdraws-the-work-a-flight-has-passed.md); the hall's aliasing goes to the antialiasing plan below.


## Engine mechanisms

### Plan 0098's leftovers (close review, 2026-11-24)

Found at the close ([Plan 0098](plans/done/0098-a-calm-light-by-default-no-screen-history-smooth-sources-to-the-mid-range-and-the-fsr-2-turn.md), v0.72.0); none blocked it.
- **The dig rig's queue never drains.** With the window's queue flags the rig holds 102 to 107
  chunks in view waiting after 30 000 frames on the RTX and 126 to 129 after 3 000 on the
  RENOIR, where the fly-through drains in 44 to 50 s: the rig's frames are submitted with no
  wait on the device, so its job loop reads back one submit in about 190 frames on the RTX
  (one in 15 on the RENOIR). "Drained" and "waiting" are the same state on the RENOIR's runs,
  and the rehearsal's first dig falls in a chunk with no light. What it takes: the rig's
  stepper polls the device a frame as the fly-through's loop does, then Plan 0098's landing
  table read again on a truly drained queue. With Plan 0099's Phase 1, which reads the same
  rig for its base.
- **Sequence B's "settled after" claim is thin** (`dig_line::hold::claims`): with the groups
  at ring 1's landing (the owner's pick) a dig of B has at most two frames after its fade and
  their reference is the later of the two. Sequence A and the "before the landing" bars carry
  the test. A form that bites: B's digs 16 frames apart, or the reference at the chunk's
  second landing.
- **No gate test renders a held record.** `gpu_lights::a_fresh_hole_holds_one_value_between_black_and_white`
  is a soak mark (11 to 13 s on lavapipe); the gate reads the held record's words
  (`bake::held::tests`, `gpu_sun::a_dig_rebakes_the_key_words_with_the_sky_ring`) and the
  queue's order, not the light pass's read of them. A smaller rig under 10 s would close it.
- **`every_profile_holds_the_calm_picks` does not read `dig_hold`** (Phase 17's profile row:
  `on` in every profile); one more field in the test's tuple.
- **`open_digs` knows a dig's fade by `frames == DIG_FADE`** (`bake/queue/digs.rs`): a value
  compare that breaks without a failing test if `FADE` and `DIG_FADE` ever meet or a fade is
  paced by elapsed frames (Plan 0099's Phase 2). A flag on `Fade` says it.
- **`--dig-hold rays` stays as the image test's bite check.** The W3 amendment's question 3
  recommended removing it at the close; the amendment at window 98d kept it for the test. The
  owner rejected its picture at W3 and 98e, so it is a setting nobody would pick: remove it
  once the image test has a bite check that needs no arm (the owner's call).
- **Followups the plan carried:** under `key far baked` (not a pick) the key record still
  steps at a landing with no history; `source fade` mixes in linear light from black, so its
  first frames are large on the display (the readings log, "What the fade does"); the sky ray
  cast where no record is fresh is never shown with the history off (the entry "The sun's
  shadow near under the calm defaults" holds it; which of the dig's paths still need the cast
  under `--dig-hold on` is to be read first: the ring's hold tests that the ray was cast).
- **Files near their caps:** `temporal.wgsl` 1169 of 1200 lines, `gpu_outdoor/flicker.rs`
  1966 of 2000, `bake/queue/mod.rs` 1060 and `bake/job_loop.rs` 1047 of 1200. Plans 0099 and
  0100 both add to them; new code goes to files of its own.

### The weekly soak's underground eye: the surface reference reads 157.49 (2026-11-24)

Read by the gate-time chore from the red weekly soak of 2026-11-23 (run 45165663078, main
`123964e7`) and again on `909a2552` on the dev reference's lavapipe:
`gpu_outdoor::the_underground_eye_reads_no_sky` fails at its surface reference, `surface stored
157.49023807374985` against the bar's 149.7 within 5 % (+5.2 %; the reference was read at
`2fe4598c`, Plan 0086), the same value on CI's and this box's lavapipe, so deterministic. The
near-eye visibility bar passes; the underground parts after the reference are not reached. A
real engine change, not a tier fault: some plan since Plan 0086 moved the stored sky light on
the open surface. What it takes: find the commit (the reading `reading::underground_eye` at
`r` 7 takes about 220 s on lavapipe), then either a fix or the reference re-read by the owner's
rule for a moved reference. Read with it: of the run's other four failures, the wobble table is
a reading since Plan 0098 (`c46fb41a`), and the two still-camera upscaler bars and the shutdown
bound pass here (`docs/nfr/07-startup-and-build.md`); the next weekly run shows whether CI's
older LLVM 21 lavapipe or its parallel GPU tests still trip them. The weekly soak is the only
CI net for soak and sole marks, so a red run is owed a reading each week.

### Plan 0096's leftovers (close review's rulings and the close, 2026-11-23)

Routed by the architect's amendment at the close review of [Plan 0096](plans/done/0096-the-bake-queues-drain-tiers-a-drain-slice-the-step-and-the-slow-adapters-light-pass.md) (its section "The close review's rulings"); none blocked the close (v0.71.0).

- **The glide's `light place` where a default buffer evicts.** Phase 7's indexes and Phase 7b's stop took the eviction scan from up to 1 186 ms a sync to 2.16 / 6.19 ms p50 / p99 in the owner's windowed cave glide (the default buffer, 158 / 1 279 chunks evicted a sync) and 6.84 / 8.84 on the 1/16 stress rig (loaded), against "under 2 ms in a glide that evicts"; the bench, which evicts nothing, reads 1.07. What is left is the wanted set's collection (58 %) and sort (40 %) at every sync (`place_lights`, `crates/vxe-render/src/gpu_world/lights.rs`). The lever: keep the wanted set ordered across syncs and re-sort on a focus move only. **Taken as a phase of the "Slot words packed on the workers" plan** (ADR-0056) when it is drafted: the same thread, bench and glide.
- **The listing census reads no bands.** `--queue-census` counts every surface chunk as near ("mid 0, far 0" on all four runs of Phase 9), so it is taken without the far-normal band; the in-view line's bands are right. Give the census the view's `Bands`, with a CPU test that a synthetic listing of 4 near, 4 mid and 4 far chunks prints those counts.
- **Faceless chunks at the residency's edge reach the listing** (16, 27, 0 and 8 on the four runs): the worker's verdict counts a neighbour not streamed as open, and the outer neighbour of an edge chunk never streams. One listing each, no job. Left as is unless a census shows the count growing with the radius faster than the sphere's surface.
- **The Portable profile's 720p is stretched to the window's shape** (the fly-through, `cli/settings.rs` `fixed_output`): a 16:9 frame scaled to 2530 x 1378 is about 3 % wide, no letterbox. The fix proposed: the hint's pixel count at the window's aspect (1 301 x 708 there), so no stretch, no bars, the same budget. With it, if the owner picks it (the plan's question 2): the fixed output as a readable row of the engine's profile, so a game that picks `Profile::Portable` is told.
- **Phase 10b** left the plan at its reading: see "The record by rule at every opening" under Next up.
- **The fixed output as a row of the engine's profile** (the close's ruling 2): `RenderSettings` carries a row true for Portable, the fly-through's `fixed_output` reads it, a CPU test reads the row per profile. With the aspect item above; the owner called the stretch itself acceptable at W2, so neither is urgent.
- **The per-phase GPU run picks its binaries from the changed files** (the harness's; awaits the owner's go). Five tests in `gpu_queue` and `gpu_queue_reach` stayed red from Plan 0096's Phase 4 to its tip's gate, because the phases that changed `bake/queue/` did not name those binaries. `test-changed.mjs` adds, to the named binaries, every `gpu_*` binary whose test file or support module names a changed module's public items (or a path map kept beside the script), and the lane rules say a phase's named binaries are a floor.
- **The RENOIR forest's one stalled drain** (37.6 s in view against 27.1 and 27.0; the drawn share fell to 0.180 once): no cause named. An open reading for Plan 0099, where the submit moves to the bake thread.

### Plan 0097's leftovers (close review, 2026-11-23)

Found at the close ([Plan 0097](plans/done/0097-the-exposures-anchor-by-the-sky-around-the-camera-the-floor-record-under-the-eye-a-time-constant-and-the-canopys-shade.md), v0.70.0); none blocked it.
- **`SkyAround` is not reset on an exposure reset or a teleport.** Its `reset` runs only when
  the floor's term is off (`frame/encode.rs`, `exposure_uniform`), so after a jump of the
  camera the floor's share smooths over `tau_around` from the old place's value while the
  ceiling jumps to a target read from it. What it takes: reset the smoother with
  `Frame::reset_exposure`, and a clause in `the_floor_share_settles_at_tau`.
- **The floor term and the lift reach the shader through soak tests alone**
  (`the_shade_under_the_canopy_keeps_the_anchor`, `the_tunnel_exposes_for_its_walls_by_day`):
  the gate reads the Rust model and the uniform's words, not `exposure.wgsl`'s
  `ceiling_target` with the floor's bit set. The first of them was broken by the lift and
  no gate saw it (fixed at the close's fix pass). A gate-size rig with a baked floor record
  would close it.
- **`tau_around` (1 s) has no word from the owner**; window 97 left its question unanswered.
- **Readings owed quiet:** the stop guard's lavapipe time (8.65 and 8.95 s, loaded; its soak
  mark comes off only on a quiet read under 10 s) and the floor walk's CPU time (0.226 us
  mean, load 10.2).
- **Owner questions open:** whether highlight protection wants an ADR of its own (ADR-0081's
  Outcome records it meanwhile), and the gate's 1498 s against the 19 min line (`docs/nfr.md`
  section 7).

### Plan 0094's leftovers (close review, 2026-11-20)

Found at the close ([Plan 0094](plans/done/0094-light-transitions-the-key-means-age-the-no-ray-prior-and-its-fades-the-hole-band-and-the-castle-fill-in.md), v0.68.0); none blocked it, each is a small fix when the area is next open. **A bar that does not see its mechanism:** `a_rare_hit_is_clamped` (`crates/vxe-render/tests/gpu_sun/history.rs`, soak) passes with the clamp off too (largest 0.1996 both ways); a clause on what the clamp changes, or a rig with more than one hit, would defend it. **Unread costs:** the no-record prior's arm A on the RENOIR at 1080p's half tier during a stamp's bake (a linear guess of 14.6 ms, `docs/nfr/02-frame-budget.md`; Plan 0096's RENOIR readings can take it) and the no-ray fade's temporal and composite cost.

### Plan 0083's leftovers (close review, 2026-11-19)

Found at the close ([Plan 0083](plans/done/0083-anything-from-a-vox-palette-materials-a-material-file-and-large-scenes-stamped-into-the-world.md), v0.66.0); none blocked it, each is a small fix pass when the area is next open. **Pending placements (`vxe-world`, ADR-0066's Outcome).** `World::stamp` takes `&Prefab`, so a placement past residency copies the prefab into a fresh `Arc` (`stamp.rs`, the `Placement` push) instead of sharing the registry's, and a rotated stamp materialises its rotation once more: bounded by the pending prefabs' bytes (the castle's 45 MB again at most), a `stamp` over the `Arc` removes it. The poll thread blits every placement an inserted chunk lists under the store's write lock (`stream/mod.rs`, `commit_residency`), up to a 64³ blit per placement while every reader waits; blit into the held chunk first, then apply only what arrived since under the lock. A placement whose remaining coordinates lie outside a finite `World::extent`, or where the camera never goes, lives for the session; drop the coordinates a finite extent can never stream at stamp time and report the pending bytes beside the count. Two stamps on one world at once (not reachable through the server, whose ops are serial) can order an overlap differently in resident and pending chunks: say in `stamp`'s doc that the caller serialises stamps. **Scenes (`vxe-assets`).** `VoxScene::load` honours a hidden layer only on the outermost layered transform of a path (`scene.rs`, the group arm): a shape on a hidden layer under a group on a visible one is hidden in MagicaVoxel and placed here; check every transform's layer and add the nested case to `hidden_layers_and_nodes_are_not_placed`. The grammar example in `materials.rs`'s crate doc and a comment in `rows.rs` name the fly-through's castle and torch; a neutral example costs nothing. **The coarse grain (`vxe-render`).** Instance word 27 and the inverse map are derived from the one f32 grain `detail / coarse` and inverted, one ulp off for coarse 7, 13, 14 and 15; write word 27 as `coarse / detail` directly. **The testbed.** The placed notice counts the ops submitted, not the tick's applied events, and `register_prefab`'s errors are dropped, so a stamp refused at the tick still reads `placed`. A click while the ghost body's worker is building the same scaled scene builds it again on the frame thread (`scene_at`); refuse with `building` or wait on the worker. `Entry::refusal` adds the 1 m margin on every axis while the air box adds it above only and `clear box` off adds none, and prints the un-margined size it did not check. `Modes::edit_axis_max` starts at the default until a slot is entered, so the chooser's belt row can show the wrong verdict on a world with another limit. `a_key_places_as_the_click_does` compares prefab ids, not `(prefab, origin, rot)`; `a_placed_structure_is_logged_stamps` pins the chapel's per-row counts as a sorted list, losing row identity. `run/structure_tests.rs` is at 1169 of 1200 lines; the next test splits it (the ghost and icon tests to a sibling). **Owed readings (the plan's Budgets table).** The live queue's settle after a runtime castle (two 1500-frame headless runs never drained the queue, with or without the castle, `docs/plans/logs/0083.md` Phase 8: a longer run or a quiet queue is needed), `VmHWM` at `--frames 3000` with and without `--castle` (the prefab bytes fell from 343 MB to 45 MB; the RSS was not re-read), the first frame with `--vox chapel` against the plain cave over three runs, the material term of `GpuWorld::mirror_bytes` with the chapel's 255 rows, and `--census` for `Wide` bricks with the castle placed. Pick up with the next plan on structures (0084) or a readings chore.

### A pipeline cache owned by the engine, for the renderer's shaders

Drafted 2026-11-24 as [Plan 0103](plans/0103-a-pipeline-cache-for-the-renderer-bytes-the-consumer-keeps.md) with [ADR-0087](adrs/0087-the-renderers-pipeline-cache-is-the-drivers-through-wgpu-as-bytes-the-consumer-keeps.md) (proposed); **parked by the owner 2026-11-24**: a reading the same day ([logs/0103.md](plans/logs/0103.md)) shows the driver's own disk cache already makes a second start warm on both adapters, so the plan pays only where that cache fails. A `ShaderCache` per device wraps the driver's cache through wgpu (Vulkan only in wgpu 30) in an engine envelope; the consumer passes bytes in and takes bytes out, the engine opens no file, and a bad cache is a cold start. The two entries merged here on 2026-11-24 (the architecture review's of 2026-11-12 and the start-up side's of 2026-11-16) and their readings are in the plan's context. What the plan does not take stays here:

- **The default-off arms built in the background after the first world frame**, not inside the loading stage (routed to Plan 0096 on 2026-11-20, which closed without it; `absorb`'s cold start read 70 s at Plan 0094's window 94b). The plan's Phase 4 reads that start with a warm cache first (its owner question 4).
- **The light pass's default-on compile cost on lavapipe**, where no cache helps (lavapipe keeps no compiled code): the fly-through's light-pass build stage 3.7 s against the base's 2.7, about 1.4 s a frame-building test of the gate. The next step is a per-module compile reading (`pipeline_times` per `PASSES` row) to name the module that grew.

### The cold first start: 36 s of pipeline creation on the RTX, 12 s on the Radeon (2026-11-24)

From the reading for Plan 0103 ([logs/0103.md](plans/logs/0103.md); loaded, indicative). A first-ever start (a tester's first run, a driver update, a cache over its limit) builds every pipeline cold: the pipeline span reads 35.5 to 36.7 s on the RTX 3080 Laptop and 11.7 to 12.4 s on the Radeon at Portable, and the first world frame about 46 s and 20 s. No cache helps a first start.

- **What it would take:** a census of which pipelines the default start needs before its first world frame; the rest built after it or while the loading screen shows; a look at why the RTX takes three times the Radeon (the NVIDIA compiler's cost per shader, or pipelines the Radeon's profile does not build). The pre-roll also runs about 2.5 s longer cold than warm on the RTX (one step 1253 ms against 50), so some shader work lands there.
- **Pick up:** before the next tester archive, or when the owner asks for a faster first start. Not drafted.

### The light pre-roll holds the first frame 6 to 7 s on every start (2026-11-24)

From the same reading: once the pipelines are warm, the first world frame comes at 6.5 to 9.5 s on the RTX and 6.8 to 7.1 s on the Radeon, of which 5.7 to 7 s is the two light pre-roll stages. `docs/nfr.md` section 7 proposes 3 s to the first frame.

- **What it would take:** read what the pre-roll buys the first frame under the calm defaults (the coarse tier first, the rest after the frame shows), against the owner's rule of no waiting for light and no unlit first picture. Plan 0099 (the bake step off the main thread) changes what a pre-roll costs; read after it.
- **Pick up:** after Plan 0099 closes. Not drafted.

### Remove `own`, the engine's own upscaler

Drafted 2026-11-24 as [Plan 0102](plans/0102-remove-own-the-engines-own-upscaler-fsr-2-is-the-only-one.md) with [ADR-0086](adrs/0086-fsr-2-is-the-only-upscaler-the-upscaler-of-our-own-is-removed.md) (proposed); awaits the owner's go. It recommends the removal: `own` is a second output path (two passes, a history target, its own settings, a test binary and a golden), not an arm that is cheap when off. Its Phase 1 reads what the Radeon loses at Portable before anything is deleted.

### The first near-lighting step stalls 130 to 210 ms on a cold driver cache

Noted 2026-11-16 from Plan 0075 Phase 7b (`docs/plans/logs/0075.md`, session 75c; RTX host). With
every pipeline built on the start-up worker, one loading gap stays at the first "lighting the near
chunks" frames, cold only (warm max 87 ms). Every `vxe-render` compute pipeline exists before it,
so the time is something else the driver does on first use. The owner noticed no hitch and
accepted it. Pick up when a start-up or first-frame plan needs the bar, with a driver trace.

### The loading frames stutter on the streamed worlds

Noted 2026-11-16 at Plan 0075's close (its Followups; `docs/plans/logs/0075.md`, Phases 5 and 6).
On the cave and the castle the loading frames run near 24 fps (interval p95 96 to 104 ms on the
RTX); the owner saw it ("like fps in 30-40") and accepted it. A pre-roll bake step runs 70 to 140
ms against the 25 ms slice, and each loading frame waits for the previous slice's GPU work. What
it takes: a bake step split under the slice (fewer samples a step, or a step bounded by time),
read on the castle. Pick up with the next start-up plan, or when a tester reports a jerky load.

### A CPU-drawn first frame before the device

Parked 2026-11-16 by the owner at Plan 0075's Phase 6b (`docs/plans/logs/0075.md`). Since 6b the window shows a static loading frame 0.3 to 0.4 s after process start on the RTX (0.2 to 0.3 s on the Radeon iGPU), drawn by the GPU right after the device. What is left before it is the instance (78 to 244 ms), the device and the surface's configure. In two runs, the first launch after a pause, the instance took about 1.6 s (a guess: the NVIDIA dGPU waking; not confirmed). On Wayland nothing shows until a buffer is attached, so a frame before the device needs a CPU-filled buffer: the `softbuffer` crate (rust-windowing, winit's org; wl_shm on Linux, GDI on Windows), or the same by hand per platform. Windows already paints a window's class background at once. Cost: a dependency and its ADR, about 100 lines in `vxe-app`'s shell, and the window's surface handed from the CPU buffer to wgpu's swapchain, read on both platforms. What it takes: a reading of how often the slow instance happens on testers' machines (the `start-up:` lines in run logs), then a small plan. Pick up if a tester reports a blank window at start.

### The baked key record's cell blend carries light across a shadow edge

Found 2026-11-22 by the tunnel-leak diagnosis (branch `diag-leak` at `4a6770db`, its log
`docs/plans/logs/diag-leak.md` on that branch; owner's window 97b: a blinking sunlit wall and a
white patch in a dark tunnel, on the builds before and after Plan 0097). With the far estimate
forced on every face, `KeyFar::Baked`'s blend between brick-direction cells (ADR-0079) puts
12 225 px bright where the truth is shadowed, against 912 for the column map's horizon, which
errs dark through the tunnel's mouth instead. Neither symptom reproduced headless, and at the
wall's distance (2 to 4 m) the far weight is 0, so how the window read the record there is
unshown. **What it takes:** first a debug view of each pixel's key source tag, to catch the lit
state live in a window; then, only if the record is shown to be the source, a blend that does
not cross a shadow edge (the own cell only, a blend gated by the cells' agreement, or a finer
record). Plan 0098 made the column's horizon the default at every distance, so the record is off
by default. **Plan 0100 reads the baked key record far where the column map reads shadow
(ADR-0084), so this is a risk its lane reads first.** The white patch seen
from the spawn is real light through a roof slot of the test world's generator (the valley
dips under the tunnel's crown over x 192 to 266), the owner's call to close.

### A mip by majority can drop a roof one or two voxels thick

Raised 2026-11-22 by the same diagnosis, untested: at LOD 1 the mip's majority rule may remove
a 1 to 2 voxel roof (the tunnel at x 266 to 290), which would let the sun into an enclosed
place at a distance. It bears on any far geometry that reads a coarser mip (Plan 0098's
candidate for the far hits that flip under the jitter): a mip read for visibility or for a
stable far hit wants "any solid" coverage, not the majority. **What it takes:** a CPU test of
the mip of a one-voxel slab over air, and a reading of the far shadow under it. Plan 0098
closed without it (no arm of its Phase 5 was picked). Pick up with the far-field work (Plan
0033's re-read), which reads coarser mips for far hits.

### How dug interiors should look: darker, less sky than the true aperture

Parked 2026-11-20 by the owner at Plan 0094's window 94a verdict ("the hole reads lit";
`live/window94a/1-forest-dig-lit-hole.png`, `4-dig-hole-lit.png`: a radius-0.5 m hole's floor
blue-grey under a shaded hill and under a canopy, its walls dark brown). Plan 0094 Phase 7a (6)
reads whether the baked visibility matches the hole's true aperture; the owner's call is that
**even a correct aperture is not the look**: a dug interior should read darker and take less sky
than the geometry gives, so a hole reads as a hole beside the ground it was dug from. A look
plan, later (the motion lane it waited for closed with Plans 0095 and 0098): candidates are the hole's walls' bounce in the baked term
(the walls' own low value folded into the floor's, which the bake's bounce cell mean may already
carry too little of), a darker soil row at the material table (a consumer's palette call, not
the engine's), or an occlusion term at concave faces read from the column map or the coarse
grid. Read first on the 94a stills' scenes (the hills at 16:00, the forest at `10,180`) with
the true aperture printed beside the stored value; the pick is the owner's eye.
**Read (Plan 0094 Phase 7a, `gpu_sun`
`reading::the_dug_floors_baked_visibility_against_its_aperture`, lavapipe): `geometry`.** The
record is right for its cell, but the cell is the 8-voxel brick: a radius-5 sphere's record
reads 0.718 where the bowl's own faces see 0.473, so a small hole's floor shows the brick's
mean with the open ground beside it, not its own sky.

### A second `Frame` on one rig does not drain its bake queue

Noted 2026-11-19 by Plan 0094 Phases 1 and 2 (`docs/plans/logs/0094.md`, bars 2 and 4 of
Phase 1, bar 1 of Phase 2): on the hills, the castle's arms (b) and (c) and the forest's turn
(twice on a fresh rig) the queue did not drain in 30 000 stepped frames, where a fresh process
drained the same scene in 1200 to 3300; the undrained arms read up to 16 % of pixels over 4 at
rest against under 1 % drained. Plan 0094 runs every arm in its own process. What it would take:
a reading of the queue's depth and the job at its head on the second frame (a job that never
lands, or a rig that re-marks what the first frame baked). A chore or the perf plan's; read
before any gate test shares a rig between two frames. Possibly the same cause as "The dig rig's queue
never drains" (Plan 0098's leftovers: a rig's frames submitted with no wait on the device);
read that one first.

### A dug chunk's settled light differs from a one-shot bake

Read 2026-11-10 in Plan 0060 Phase 3 (`docs/plans/logs/0060.md`, `a_dig_is_never_withdrawn`). In that test's rig, after the dig settles, chunk [2, 0, 0] differs from a one-shot bake of the edited world. Chunks 0, 1 and 3 to 7 match, and so does the dug chunk the test now compares. The difference reads the same under the score order with withdrawal on, under `distance` with withdrawal off, and at the default `edit_ms` (the lane's `live/p3-digexp.log` and `p3-digexp2.log`, untracked). So it is not the withdrawal and not the order. It predates Plan 0060 by that reading, but it was not read at the base. A guess, not isolated: a neighbour chunk whose light reads the dug chunk is not re-baked after the dig, or is re-baked before the dig's own job lands. What it takes: a headless test that digs, settles and compares every chunk with a one-shot bake, read at main first, then a fix in the live queue's dirty-neighbour rule. Added at the owner's call (2026-11-11).

### A `Uniform` emitter brick gives no light samples

Noted 2026-11-13 from the tree at `98e69b8` (Plan 0074's draft, its owner answer 5).
`note_lights` scans `Full` bricks only, so a brick made entirely of one emissive material
contributes no light samples, even on its faces toward air. Not seen on screen. Fixing it changes
the light, which Plan 0074's byte guard forbids. Pick up after Plan 0074 as a look check: place a
solid emissive block of 8 or more voxels a side in the cave and compare it with a hollow one.

### The bake queue ordered by what the march touched (2026-11-20)

From a devlog (voxelbee, "Adding A CACHE To My Custom VOXEL Game Engine", devlog 5): a GPU-run
cache where every voxel a ray renders moves to the front, so residency follows visibility. We
do not want its eviction (a turn would reveal unloaded data: pop-in against the stable-image
rule; the mirror's distance window, ADR-0013, stays). The part worth taking is the signal: the
march sets a bit per chunk it hit this frame, read back a frame late. Plan 0096's in-view drain
orders jobs by an estimated size on screen; the touched set would order them by actual
visibility, skipping chunks behind hills or the camera, and could order the mirror's uploads
the same way. Cost guess (not measured): one atomic-free store per hit into a bitset of the
window's chunks, one small readback. Plan 0096 closed (2026-11-23) with the in-view drain at
36.5 s against 10 on the RTX, bound by the step's CPU per job, which Plan 0099 takes. Pick up
if the drain still misses once that CPU is off the render thread.

### The near shell's cost in a flight, and the streaming edge

Parked 2026-11-10 at Plan 0060's Phase 1 stop rules, owed to the backlog at its close (owner's amendment). In the streamed cave at `--glide 300` the near shell (Chebyshev 2, baked at the full count) takes 98 to 99 % of the slice, and about 0.70 of the in-view chunks still waiting have their reach not ready (the mirror's edge), so neither the order nor the withdrawal can light them. What it would take: a shell baked coarse first in a flight, or a smaller shell at speed; and a streamer that loads the light radius ahead of the view. Plan 0067 (closed 2026-11-12) did not take it, and Plan 0096 has since changed what the shell bakes at (a sample tier by size on screen, ADR-0080), so the flight is read again on today's tree first (`docs/plans/logs/0060.md`, Phase 1, holds the old reading). Pick up with the next plan that reads a flight's light.

### The first-block fallback fade's CPU under `chain`

Parked 2026-11-12 at Plan 0067's close review (`docs/plans/logs/0067.md`, Phase 7, the opening row). Under the default missing light, `chain`, every chunk's first block fades over `FADE` frames from its fallback value, and the render thread rewrites the fading block (`advance_fades`) and its summary and two parent records (`Store::show`) every frame of the fade. In regime B, where nearly every step opens, that reads `fades` 1.05 to 1.10 ms p50 and `summaries` 0.61 to 0.63 on the RTX 3080 Laptop (`--near 0`, rig (c)), against 0.002 and 0.05 under `black`, and takes the opening step from 0.75 to 0.86 to 0.88 of the start commit's (bar 0.8); the default glide's `summaries` read 0.965 ms p50, p99 2.86. The owner saw no lag at this cost in the window. Levers, unbuilt: a shorter fade for a first block (a look change, the owner's eye), the summary and parents written once at the fade's end instead of every frame (ADR-0053 has a parent fade with its children, so this one needs an Outcome there), or the blend on the GPU (the temporal read mixes the fallback with the block by a per-chunk weight). `fades` and `summaries` are among the per-job stages Plan 0099 moves off the render thread; read them again after it, and pick up only if they still show there.

### The pre-roll never settles at small residencies

Seen 2026-11-15 by Plan 0075's lane while sizing its streamed test (`docs/plans/logs/0075.md`): with `--residency 2,3` or `4,6` and `--near 0`, the start-up pre-roll runs to its 120 s cap, with the loading screen on or off; the camera's chunk never reports its light settled. Not read further. A guess, not isolated: the pre-roll's "near chunks hold their light" condition waits on a chunk the small window never makes resident, or keeps evicting and reloading. What it takes: a headless repro at main with the pre-roll's wait condition logged per chunk, then a fix in that condition. Pick up before a consumer runs small residencies.

### The fired lights' grain: the list lights' visibility on a running mean

Parked 2026-11-18 by Plan 0086 Phase 22 (the second window's defect 3: "when I shoot light,
walls becoming grainy", 20.png, the noon tunnel under anchor 0). Phase 16 read it as Plan
0058's, not the sky's (`docs/plans/logs/0086.md`, Phase 16's table and Phase 22): with five
lights fired the lit patch's p99 change is 25 to 28 levels in flight and 5 to 6 at rest, the
same with the sky off; no sky ray returns over the lights; the light pass's rise (0.13 to 4.6
ms, loaded) is the five lights' shadow rays. In flight the list changes every frame and
`LIGHTING_LIGHTS_CHANGED` takes the direct part's sample as it is. What it would take: the
list lights' shadow visibility on a running mean of its own, as the key light's is on the
visibility pair's `g` (Plan 0086 Phase 19), reprojected and reset where the light moves. Plan
0086 closed without a ruling on it, and a running mean is a screen history, which ADR-0082
took out of the default light pass: the mechanism is chosen again under that ADR, after the
grain is read under today's defaults. Pick up when a consumer fires lights at walls in the
dark, or the owner calls the grain.

### Settle early: a coarse area answer where an estimate is not still, and a hold once it is

Parked 2026-11-19 (owner, at Plan 0094's Phase 1 and 2 readings: "light that is not still can be approximated around area and settle exit early; we don't need super precision if it becomes flicker", and "this exit early strategy can be used a lot around similar cases"). The principle: while an estimate is young or restarting, show a coarse answer pooled over its neighbourhood and narrow it as it ages; once it is still, hold it and move again only through one fade.

**Outcome (Plan 0094, closed 2026-11-20):** the principle is ADR-0076 rule 6 (a coarse stable answer while the precise one has not landed, one fade when it does, every reading on the undrained image too). Both forms built for the visibility pair left: the hold at the cap (`settle_at_cap`, `--settle`) was rejected at window 94a and its code removed (ADR-0076's Alternative F is its record), and the age-weighted pool is off by default. The first place the rule pointed at, the bake queue's drain, went to Plan 0096. The readings of both forms are in `docs/plans/logs/0094.md` (Phases 3 and 6).

**Places it might still apply, each unread:** the exposure (a hold once the meter is within 1/64 EV), the probes or lamp channel (hold a probe whose samples sit in its band), the denoiser's footprint (wide while the variance is young, none once settled), the fired lights' grain (the entry above), the entity lighting (a part's coarse grain holds its light until the part moves), the far field and LOD transitions (hold a tile's converged value across a mip switch, fade the switch). Pick up per place when its flicker is read; Plan 0090's counter names the places.

### A held fallback still moves on screen

Parked 2026-11-05 from the ruling on `the_vein_wall_patches_are_counted`. With the table's read off at the patch pose on 299b5c8, the accumulation is bit-identical over the window (0 pixels changed), yet the filter's output reads 90.95 patches a step and the composite 51.10. The lamp is off and the gather is not added, so the filter is the only part left that moves. By reading the code (not isolated): `temporal.wgsl` keeps blending each frame's noisy sample into the moments after the emissive part holds, and `atrous.wgsl` weights its taps by that variance. `a_still_fallback_pixel_holds_still` reads the accumulation, so it does not see this. Plan 0059 keeps the fallback for no-face hits, so glass and the field look may shimmer at rest. That patch-pose run makes every hit a fallback, so it is an extreme case; a translucent surface has not been read. What it takes: a spike that reads a translucent wall's composite at rest, then a fix that holds the moments with the mean, or passes a held part around the filter as the table's part does. Plan 0059 closed (2026-11-09) with no answer recorded here, and ADR-0082 has since taken the screen history out of the default light pass; read a glass pane at rest under today's defaults before any fix.

### Plan 0090 owes a stamp scene (a freshly placed castle under the sky) and a reading of the key mean's fade-in against `F`

Parked 2026-11-18, twice: from Plan 0083's Phase 7 verdict (session 83b, `--source outdoor --time 12:00`, 25.png: inside a castle placed from the belt a moment before, the walls clipped white and blue and the vault was dark and grainy, "castle flickers badly"; HUD queue depth 10 751, missing 0, scene -4.1 EV, eye +1.34 EV) and from Plan 0086's flicker amendment (the stamped chunks baked behind the carve's dig ring and every reset pixel on the new walls ran the sky ray's mean from one raw sample; `docs/plans/logs/0086.md`, the flicker section). Plan 0086's Phases 21 (a) and 21a took the fix. What is owed is an architect amendment to [Plan 0090](plans/0090-the-pop-counter-scenes-on-lavapipe-count-the-pixels-that-break-the-light-rules-a-ratchet.md) before its lane starts; its scene table has the castle's hall with no sky, not a stamp under one:

1. **A scene:** a prefab stamped on the hills at noon with the eye inside it, counting the pixels that break rules 1 to 5 over its settle, by source, so the class cannot return unseen.
2. **A reading of the key light's mean** (`key_mean`, Plan 0086 Phase 19) at an edit-cast shadow outside its edit box, which fades in over up to `sky_history` frames, against the `F` the owner calibrates in Plan 0090 Phase 5; if `F` is shorter, the key mean takes a band test as `pair_step` has for the sky. Under ADR-0082's defaults the light pass keeps no screen history, so this reading applies with light history on only; the amendment says whether it still earns a scene.

### The bake and strip examples drift from the fly-through's lighting defaults

Parked 2026-11-13 (orchestrator default while the owner was away). The hero fix (5b35aa3) gave `examples/hero.rs` the shared `flythrough::default_lighting()`. Two tools still build the library's `Lighting` and so miss the Plan 0061 store-read and light-weight defaults: `apps/flythrough/examples/bake.rs` and `apps/flythrough/examples/strip.rs`. Their readings and stills differ from what the window shows. The fix is to call `default_lighting()` and re-read anything that cites their figures. Pick it up when a plan next reads a strip or a bake figure, or when the owner asks.

### Overlapping coloured lights go white: a hue-keeping tone map

Parked 2026-11-15 from Plan 0058's re-window (step 1b, the plan-0058 worktree's `live/10.png`; orchestrator default, the owner said "not sure colours mix well" and passed (k)). Two dropped lights at 600 overlap as plain white, with cyan only on the green and blue edge: auto exposure sits at its +4 EV ceiling (the scene reads -3.3 EV), so every channel clips. The lights add correctly; the tone map's per-channel clip loses the hue. What it takes: a tone map that desaturates toward white only past a luminance shoulder (hue kept), or a lower light intensity; either moves goldens and is the owner's eye. Pick up at the render-settings profiles (Plan 0069) or the next look plan. **Extended 2026-11-20** by the review [`reviews/2026-11-20-crystal-islands-look.md`](reviews/2026-11-20-crystal-islands-look.md) (trait 2): the Crystal Islands frames clip the same way (saturated bodies, white tips, 1:00's giant crystals flat white), so the per-channel curve is not what separates the looks; this entry is a refinement ranked second of that review's engine levers, after the per-voxel variation, and the content pass (dimmer, more varied emission) comes first.

### Per-voxel albedo variation by a seeded hash, an amplitude per material

Parked 2026-11-20 from the review
[`reviews/2026-11-20-crystal-islands-look.md`](reviews/2026-11-20-crystal-islands-look.md)
(trait 4), the first of its engine levers. John Lin's Crystal Islands frames put a fixed
per-voxel speckle of two to four shades on nearly every material (foliage crowns, crystal
faces, rock), and it is most of what reads as texture there. We have one `albedo` per row and
nothing varies it (`composite.wgsl` reads `materials[rec.material * 4u].xyz`); the procedural
sources put one row on everything. **The mechanism:** at the composite, scale (or hue-shift)
the albedo by a hash of the hit voxel's integer position (ADR-0043's key `floor(p - 0.5 n)`)
times a per-material amplitude; optionally the emission too (the frames' crystal bodies are
darker than their tips). Stateless, no history, no random stream: a static pattern under FSR 2
and invisible to Plan 0090's pop counter. Cost a guess: one hash and one multiply a pixel.
**Owner, 2026-11-20:** the amplitude lives in the render settings (ADR-0063's render-side
per-material style word), not on the material row. Size S; a look spike with an owner window, the content pass it
waited for is done (2026-11-20, the showcase's palette).

### Ambient occlusion driving a per-material colour ramp

Parked 2026-11-12 (owner, from the video "the voxel rendering tech nobody talks about", youtube.com/watch?v=1gxcNUUc37Y). Instead of darkening albedo by AO, AO picks a colour along a per-material ramp: snow white to cyan to deep blue in crevices, sand yellow to red, grass bright to deep green. The video's form: per material `light`/`mid`/`dark` colours plus `t_mid`/`t_high` thresholds, `mix(mix(dark, mid, smoothstep(0, t_mid, ao)), bright, smoothstep(t_high, 1, ao))`; AO is per-corner neighbour occupancy shaped by `pow(ao, 1.5)`; fixed face weights (up 1.0, down 0.1, x 0.5, z 0.9) and side faces capped at 0.7 of the ramp; each colour hue-rotated by low simplex noise over world xz for variety.
- **Our gap:** `vxe-render` has no AO term; ambient comes from the baked brick light (Plan 0059). `Material` carries one `albedo`.
- **The cheap input:** at a ray hit the voxel and face are known, so the face's 8 neighbours give corner AO, interpolated at the hit point. Analytic, no noise, no history, so it keeps the image stable.
- **Open look question:** drive the ramp by local corner AO, by the baked sky or ambient light, or both, so a cave reads tinted rather than only dark. Decide by eye in the window.
- **What it takes:** a look spike (corner AO at the hit, an optional ramp beside `albedo`, the hue jitter) with an owner window; an ADR if the material row changes shape.

Pick up after the current lighting plans close (0058 and those ahead of it). 2026-11-20: the
hue jitter named here has its own entry now, "Per-voxel albedo variation by a seeded hash"
(from the review [`reviews/2026-11-20-crystal-islands-look.md`](reviews/2026-11-20-crystal-islands-look.md)),
whose frames show the jitter without any ramp; a spike on this entry shares its hash and its
place in the composite.

### Darker scenes, and light intensity varied by seed

Noted 2026-11-15, the owner, after the darker Windows image: "it was a nice scene, we should
make lights darker, or random in darkness as well". Two parts: the test scenes' lights
(fly-through, consumer side) set dimmer, and a seeded per-light intensity spread so some
lights are dim and some dark. The engine side is the scale over the table (ADR-0045) and the
light list's per-light intensity (ADR-0044); the randomness belongs to the scene's seed, not
the engine. Pick up after the Windows brightness entry is explained, so "darker" is judged on
a correct image. 2026-11-20: the review
[`reviews/2026-11-20-crystal-islands-look.md`](reviews/2026-11-20-crystal-islands-look.md)
(trait 3) gives it a reference: the Crystal Islands caves sit one to two EV under our key with
the emitters dimmer and more varied; its "showcase's palette" entry carries the testbed half.

### A cheaper `absorb` canopy shadow

Parked 2026-11-18 at Plan 0086's Phase 15 window (owner: "shadows are good but fps dropped
significantly - around 20"; pick C: `absorb` stays an off-by-default setting). As built (Phase
14, `walk.wgsl`'s translucent walk through flagged bricks, `exp(-sigma x)` over the leaf
voxels' path, sigma 0.15 a voxel) the forest's light pass reads 17.4 to 18.1 ms on the RTX
3080 Laptop against voxel's 4.1 to 4.5, and 87.7 to 88.3 ms on the RADV RENOIR against 18.4 to
20.3 (`docs/plans/logs/0086.md`, Phase 14 bar 4): 4.1 to 4.8 times, every shadow ray stepping
voxel by voxel inside every leaf brick. Also read there: 49 % of the canopy's brick projection
holds columns with no leaf voxel, which the per-voxel walk leaves at the open value. What it
would take: a per-brick leaf density (one byte, the brick's fill, from the mirror's masks at
upload) so the walk steps bricks and adds `sigma x fill x path` per brick, or the 2^3 coarse
masks' count as that density per coarse cell; both make the transmittance a brick-scale blur
of the voxel truth, which is also what fills the no-leaf columns. Pick up when a canopy's
continuous shadow is wanted as a default or by a consumer, with Phase 14's rig as the bar.

### Light shafts inside structures: visible sun beams through openings

Noted 2026-11-20 by the owner at Plan 0094's window 94a, inside the castle's keep: "it would be
nice in the future rays of light inside such structures". Visible beams where sunlight enters an
interior through windows, doors or breaches, with the air lit along the ray. It belongs with the
volumetrics (Plan 0035's froxel or march work; ADR-0030 names the cloud shell): a low-resolution
in-scattering march along the view ray that samples the key light's shadow, reduced and
temporally stable (the owner's stable-image rule; no grain while it converges). Cost is a guess
until measured on the slow adapter against the headroom line in `docs/nfr.md` section 2.

### Per-source indirect light

Parked 2026-10-22 (Plan 0029 live session 11), reaffirmed 2026-11-04 (Plan 0051 verdict). The
owner wants bounce light only from some sources: "fire on a stick or fire in a dungeon" feed the
probes, the headlamp and simple lights are direct only. ADR-0043 shrinks the probe volume to a
lamp-bounce channel and keeps its logic for exactly this later redesign; ADR-0044 gives lights a
fixed list. The missing piece is a per-light and per-emissive-material "feeds the bounce channel"
flag and the channel's redesign. Plan 0058 (closed 2026-11-15) built the per-light `BOUNCE` flag,
under `Lighting::lamp_bounce`; what remains is the per-emissive-material flag and the channel's redesign.
Plan 0032 (redesigned 2026-11-10) ships no sunlight bounce: sky light is a sky ray, so a cave
mouth gets no bounced sun until this entry is taken. Plan 0037's night light likewise has no
bounce and no probe feed. 2026-11-20: the review
[`reviews/2026-11-20-crystal-islands-look.md`](reviews/2026-11-20-crystal-islands-look.md)
(trait 1) reads the Crystal Islands caves as first-order light (emitter to wall), which the
baked store gives; a second bounce is not what their colour rests on, so this entry ranks
fourth of that review's engine levers.

### More than sixteen lights: a world-space light grid

Parked 2026-11-10 (owner asked for the idea in detail while Plans 0058 and 0062 were drafted). ADR-0044 shades sixteen list lights (eight until the owner's pick of 2026-11-14, at Plan 0058's window), each pixel looping over all of them. Plan 0062 allows 64 entities, and each glowing creature or projectile that lights the walls takes a slot, so the cap is still the first limit a real scene meets.
- **The mechanism:** a grid of cells around the camera (a guess: 16-voxel cells, 64 a side) holding, per cell, a bit mask of the lights whose reach touches it. A pixel, a shadow ray's origin or a probe ray's hit reads its cell and shades only those lights, still exactly, one shadow ray each. The grid is in world space, not screen tiles, because the bounce channel and the upscalers' change mask need it at points that are not pixels.
- **What it buys:** 64 or more lights in the world for the cost of the few that overlap at each point. **What it does not buy:** many lights overlapping one spot still cost one shadow ray each; a per-pixel cap past that is a pick, and a random pick is noise (ADR-0044 B, rejected).
- **The cheaper first step:** keep the slots and let the engine choose the sixteen that matter most to the view each frame, with hysteresis. No shader change; lights can pop when the choice changes.
- **Pick up when:** a scene with entities wants more than sixteen. Plan 0058 (closed 2026-11-15) read the cost per light in reach: 0.17 ms on the RTX and 1.54 ms on the RADV RENOIR at the spawn, 24.8 ms for sixteen there. It needs an ADR that amends ADR-0044's cap.

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
2026-11-20: the review [`reviews/2026-11-20-crystal-islands-look.md`](reviews/2026-11-20-crystal-islands-look.md)
(trait 1) names this as the one engine difference in how a small emitter's glow falls off on a
near wall; ranked third of its engine levers, to be judged only on a window after that review's
palette pass, not before.

### Baked or emissive light on far-field cells

Parked 2026-11-10 (plan review). Plan 0033 gives a far-field hit no store value, so distant
emitters do not light far terrain. A coarse store block per clip brick is ADR-0028's
Alternative F, picked up only if a reading or the owner's flight asks.

### A consumer's settings file in the render example

Parked 2026-11-12 (owner, at the settings-menu interview). The fly-through and the tester keep render settings in a TOML file through an app-side field table ([Plan 0073](plans/0073-a-settings-menu-and-file-for-the-fly-through-and-the-tester.md), ADR-0058); the engine stays file-free (ADR-0055). A later example could show a consumer doing the same, so a game sees how. The natural home is [Plan 0071](plans/0071-the-consumers-path-a-render-example-a-renderer-tested-alone-names-and-errors.md)'s render example, or a second example beside it. Pick up after Plan 0073 closes, if 0071 has not taken it by then.

### Rebuild the world live behind the loading screen

Parked 2026-11-15 by Plan 0075 ("What this plan does NOT do"). A settings change of Plan 0069's
`Rebuild` class could drop to the loading screen, rebuild, and step back in, instead of a restart.
What it takes: the run's stages re-entered from a live run, the world and mirror torn down in
place. Pick up if a rebuild-class setting proves common (Plan 0069 closed 2026-11-17). Sharpened
2026-11-19: it needs a drop-and-recreate test first, headless: build, drop, build again, and
assert the mirror's VRAM reservation and the process's RSS return to the first build's figures
(within a stated slack); a leak there is what makes a live rebuild worse than a restart.

### A foundation under a snapped structure

Parked 2026-11-19 from Plan 0083's Phase 7c verdict (window 83c: the castle's ghost floated
over the hills). Phase 7d snaps a structure's box to the terrain under its footprint, one column
winning (the lowest, owner question 9), so on a slope one side is sunk or floats. A foundation
that fills the footprint's columns from the base down to the terrain would remove the choice; it
is a stamp that writes into air only, which Plan 0084's write policy (ADR-0067) gives a stamp.
Pick up when 0084 lands: a `foundation on|off` beside `clear box`, the fill in the structure's
most-used row, as a third stamp of the click.

### `.vox` import: down-scaling, a traced preview, a row's class changed live

Parked 2026-11-16 at the drafting of Plans 0083 and 0084 (the owner's "import nearly anything
that can be created as a .vox file"). Three pieces those plans leave out. **Down-scaling** a model
(a majority vote per 2³, or a non-integer scale): Plan 0083 scales up by an integer only, since a
vote changes thin features and the owner judges detail by eye; pick up if a scene authored at a
finer grid than 10 cm is wanted. **A traced preview** of a structure at the crosshair before it
is placed: taken 2026-11-18 by Plan 0083's Phase 7b after the owner's window ("something
simplified at least to understand what it is"): a coarse grain per entity part and a one-part
coarse copy of the structure in its marked frame; what stays parked is a full-detail preview,
which the pool's 64-voxel parts (ADR-0050) cannot hold. **A row's class
changed live** (stone to glass without a restart): ADR-0065 fixes a row's translucent, emissive
and opaque class for the world's life because the mirror's slot packing and the bake's emitter
lists read it at the first sync; lifting it means re-packing every brick that holds the row and
re-baking its chunks. Pick up if the materials file's reload hits the `restart` banner often.

### Entity placement extras parked at Plan 0077's amendment

Parked 2026-11-18 from Plan 0077's "Amended 2026-11-18" (the reference is
[`docs/references/entity-placement-ux.md`](references/entity-placement-ux.md)). Four things the
sources show and the plan does not take: **a translucent ghost** (Valheim's look; needs
translucent entity voxels in `entity.wgsl`, composited with the world's layers), **model icons
as an engine facility** (the plan makes them in the fly-through on the CPU through
`EntitySet::raycast`; a consumer with an inventory wants the same renderer, so it moves to
`vxe-entity` or `vxe-assets` when a second user appears), **"pick" the aimed entity into the
belt** (Minecraft's middle click) and **several belts** (Satisfactory's ten). Pick up when a
consumer or the owner asks for one; the first two are small plans, the last two are rows in
Plan 0077's key table.

### The spider's mover tick costs twice the guess

Read 2026-11-18 at Plan 0079's close (`docs/nfr.md` section 8, `docs/plans/logs/0079.md` Phase 4,
loaded): 64 spiders' movers read 1.4 ms a tick in the room course and 2.8 ms on the showcase
(36 drawn), +3.07 ms of frame CPU against the plan's +1.5 ms guess, while 64 walkers read 0.35 ms.
The owner's window did not notice it. Suspects, none attributed: the pose work per tick
(`sample` of a fourteen-part clip, `blend` while fading, the `Pose` clones into `set_pose` and
`prev`, all allocating), the sphere sweep's candidate boxes where a spider dwells at a pillar's
vertical edge (up to 220 ticks), and `solid_gradient` at `half` 6 (0.92 µs alone). **What it
takes:** a profile of `Walkers::tick` at 64 spiders on the showcase, then the cut the profile
names (a pose written in place, a tick every other frame for bodies out of view, or a cheaper
dwell). **Pick up when** a scene needs more than about 30 crawlers, or before the first consumer
runs creatures on its server tick.

### Pathfinding over the coarse grid and the walkable map

Parked 2026-11-18 at Plan 0079's close (owner question 3 took the recommendation: a seeded
wander in the fly-through, no pathfinding). The engine now has the movers (`vxe-sim`'s cylinder
controller and sphere surface mover), `probe_ahead`, the coarse grid and the per-brick walkable
map (0.8 m cells); a creature that goes somewhere needs a path over them. **What it takes:** a
search over the walkable cells (and, for a crawler, over faces: the gradient and the contact
normals say which cells are climbable), integer costs so the server's path is the same on every
platform, a string-pull to waypoints the mover's heading can follow, invalidation when an edit
changes a cell (M3's region notifications), and a budget per tick for many creatures. About a
plan and an ADR (the graph: cells, faces or both). **Pick up when** the first consumer asks for a
creature that hunts or flees rather than wanders.

### Hundreds or thousands of entities

Parked 2026-11-16 by the owner ("minecraft can have thousands of entities"). Today: `MAX_ENTITIES` 64, `MAX_ENTITY_PARTS` 512 (`vxe-render` `entities.rs`), 6 instance bits in the hit record. The cost is the trace, not the caps: Plan 0062 Phase 4 (`docs/plans/logs/0062.md`) read 64 walkers at +3.4 to 4.4 ms on the RTX 3080 Laptop and +32 to 34 ms on the Radeon iGPU, growing with screen cover and shadowing lights. **Hundreds:** wider caps and hit bits, the instance tree built on the GPU, coarse far models, shadow rays testing only entities near their light (one or two plans, no ADR). **Thousands:** entities rasterized into the hit buffer beside the traced world, which reverses ADR-0050's choice: an ADR and a spike first. **The owner's direction (2026-11-16): trace only the near ones, merge the crowded ones.** Distance tiers: near entities traced in full; mid ones traced on a coarse merged model (parts folded into one volume at detail 1); far ones as one box or impostor, or dropped below a pixel. Clusters: entities sharing a spot re-voxelized together at low resolution on the GPU each frame (or every few) into one cluster volume, walked once per ray instead of once per overlapping entity. Shadow rays test the near tier only. The server's ray query stays exact per entity; the tiers are render-only. Pick up when a consumer needs more than 64 on screen. Sharpened 2026-11-19 (Bonsai's 1.1.0 and 1.3.10 notes): the CPU side needs a per-chunk entity list as the broadphase (the mover's sweeps and the entity ray query walk the chunk's list, not every instance) and an LRU eviction of models from the pool when the library outgrows it, on top of Plan 0078's free list.

### Jelly entities: see-through glowing voxels, and true soft bodies

Parked 2026-11-16 by the owner's question "jelly-like entities, slimes - will it be possible?". The part taken: Plan 0082, a scale per axis per part in the pose and the clips (squash and stretch; ADR-0064). Two parts left. **See-through glowing jelly:** an entity voxel is an opaque honest cube (Plan 0062 draws a translucent material on an entity opaque); it waits for Plan 0034's translucent voxels and then an amendment that runs the entity walk inside the translucent march. **True soft bodies:** a blob that bends round a corner, splits or merges needs the voxels themselves to move (a lattice or field over the part's volume, sampled by the ray), which the pool (one volume per model), the regular grid walk and the query's determinism all rest against; ADR-0064's Alternative C names it. What it takes: an ADR and a spike that reads the walk's cost under a deformation field on both adapters. Pick up when a consumer asks for a creature that bends, splits, merges or is seen through.

### Per-entity voxel damage: real holes carved into one instance's parts

Parked 2026-11-16 at Plan 0085's draft (the owner's "healthy, damaged, dead" question). Plan 0085 gives an entity modes without touching its voxels: a variant hides a part, swaps its volume for another `.vox` node or remaps a material, and a dead entity is stamped into the world. What it does not give is a bite taken out of one spider's abdomen where the pick struck. **What it takes:** copy-on-write volumes per instance (an edit on an entity's part copies the shared volume into the pool, under ADR-0060's allocator, and the instance's volume word names the copy), a per-instance pool budget and an eviction rule, an edit op on an entity (shape in the part's frame, carried through the composed pose in fixed point, so the server's query and the picture agree) that reports what it removed per material as a world edit does, the op in the log (an entity op kind, which needs the model and pose in the save, ADR-0069's Alternative A), and a rule for the headless query's volumes (a per-entity `Arc<Prefab>` copy beside the model's). About a plan and an ADR. **Pick up when** a consumer asks for damage that shows where it landed, not which state it reached.

### A timeline for clips, kept open

Parked 2026-11-16 at Plan 0063's window. (b): typed keys are enough "for now, but keep it open": a timeline or curve view stays out of the engine (ADR-0051, Plan 0082's not-do list); if typed keys fail on a longer clip, ADR-0051's Alternative A (glTF from Blender) is re-read first.

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

Pick up when the lighting base is settled. As written the condition is met (Plans 0056, 0057
and 0058 closed, the last on 2026-11-15), but the light is still in work (Plans 0099 and 0100),
so the owner's word starts it.

### Automatic extremity bounds from declarative noise layers

Parked 2026-11-12 (owner, from the video "500 million voxels/sec - Voxel engine optimizations", youtube.com/watch?v=ru_oz09Zo-s). Today world generation is the consumer's: `WorldSource` is a trait, and the flythrough's cave (`apps/flythrough/src/cave.rs`) already aligns its noise lattices with bricks (one corner set per brick) and proves a brick uniform from its eight corners. The idea the engine lacks: when terrain is described as data (named noise layers, each with a range, mapping, scale and upsample factor), the generator derives each column's or brick's guaranteed solid-below and air-above bounds from the layer config itself, so a new biome skips noise evaluation outside its band with no hand-written proof; layers whose range is unbounded (domain warps) switch the bound off. Same video, also not taken: noise shared by name across biomes and computed once per position. Rejected for storage: RLE columns (slow random access and edits, which collision, acoustics and runtime digging need). Pick up only if the engine ever ships a configurable terrain generator; the requirement would arrive through `project-context.md`.

### The bake queue's backlog at a still camera: minutes to drain what is in view

Taken by [Plan 0096](plans/done/0096-the-bake-queues-drain-tiers-a-drain-slice-the-step-and-the-slow-adapters-light-pass.md) (closed 2026-11-23, v0.71.0) and ADR-0080: the drain is a budget in `docs/nfr.md` section 2 and the queue lands. The RTX's 36.5 s against 10 is the step's CPU per job, [Plan 0099](plans/0099-the-bake-step-off-the-main-thread-a-bake-thread-write-lists-and-the-in-view-drain.md)'s.

### The dug hole's light: the first frames and the lit floor

**The first frames:** taken by [Plan 0098](plans/done/0098-a-calm-light-by-default-no-screen-history-smooth-sources-to-the-mid-range-and-the-fsr-2-turn.md) (closed 2026-11-24: a dig's ring 1 jumps the queue, the hole holds a record until it lands, one 4-frame fade a landing); what is left is "A dig's landing" under Next up. **The lit floor:** read `geometry` by Plan 0094 Phase 7a, so it is a look question, merged into "How dug interiors should look".

### The key term's grain at rest on a penumbra, and the a-trous's spatial variance (ii)

Superseded by ADR-0082 (Plan 0098, closed 2026-11-24): the estimator this entry read (the key light's running mean at its cap, `key_history` 256) is a screen history the defaults no longer keep, and the key shadow on a world face is the column map's horizon. The penumbra question lives in "The sun's shadow near under the calm defaults" (Next up). SVGF's spatial variance on the key term (ii) was never built and returns only if light history does.

### Thin step lines along voxel slopes

Taken 2026-11-21 by [Plan 0098](plans/done/0098-a-calm-light-by-default-no-screen-history-smooth-sources-to-the-mid-range-and-the-fsr-2-turn.md) (closed 2026-11-24): option (a) through the far normal's widened band (`4..8`), judged at its window W1. The other two options (a riser blended with its tread, the field surface for terrain) were not built.

### Narrow the far normal's band so far pixels stop casting the ray

Taken 2026-11-21 by [Plan 0098](plans/done/0098-a-calm-light-by-default-no-screen-history-smooth-sources-to-the-mid-range-and-the-fsr-2-turn.md) (closed 2026-11-24): `key far mix step` with the band widened, judged at its window W1. Under its defaults the key shadow is the column's horizon at every distance, so no far pixel casts the ray.

### The queue step's remaining `filter` cost on a moving camera

**Done** by [Plan 0096](plans/done/0096-the-bake-queues-drain-tiers-a-drain-slice-the-step-and-the-slow-adapters-light-pass.md) Phase 5 (closed 2026-11-23, v0.71.0; as built `gone` and `forget` share one walk, the others keep theirs). As taken 2026-11-20: (one walk of the mirror's change list for `gone`, `drop_reach`, `touched` and `forget`; the change count printed in Phase 1).

### The RTX's submits draw less than their per-second slice

Read by [Plan 0096](plans/done/0096-the-bake-queues-drain-tiers-a-drain-slice-the-step-and-the-slow-adapters-light-pass.md) Phase 4 (closed 2026-11-23, v0.71.0) and re-pointed, below: a lap on `plan_rounds`, `send` and the `IN_FLIGHT` wait, then the fix it names; the bar is 85 % of the granted slice drawn. Read 2026-11-22 (Plan 0096 Phase 4, `docs/plans/logs/0096.md`): the plans stop out of rounds, not at the cap or the slots; the submit is short of jobs because the opening admits at most `QUEUE_CHECKS` (16) candidates a step, each listed on the render thread. At the drain slice the submits draw 0.94 to 0.99 of it; at the per-second form (`second:150,0.25`) 0.60 to 0.69. **Re-pointed (Plan 0096 Phase 9) at [Plan 0099](plans/0099-the-bake-step-off-the-main-thread-a-bake-thread-write-lists-and-the-in-view-drain.md) (the bake step off the main thread)**, which takes the opening's cap with the listing.

### Far light on a slow adapter: a bake budget per second, and no black for missing light

Read 2026-11-11 in Plan 0060's Phase 4 window (`docs/plans/logs/0060.md`): the far caves stay coarse on the RTX and black on the Radeon, because the bake's slice is per frame and a brick with no light value renders black. Taken by [Plan 0067](plans/done/0067-the-frames-cpu-a-cheap-queue-step-a-bake-budget-by-time-and-no-black-for-missing-light.md) (approved 2026-11-11) and ADR-0052 and ADR-0053.

### The lamp channel's faults

Parked 2026-11-05 at Plan 0057's close; taken by Plan 0058's Phase 3 (the furnace test's independent `D` and the lamp-step test over more than one probe; the inside-probe trace is a reading there).

### The hall's headless render went dark: `showcase_hall_golden_is_lit_by_its_four_ores`

Done 2026-11-16 (chore `chore-showcase-golden`, b09e018): the rig, not the look. `showcase_hall` stepped frames without baking the light store; it now bakes the showcase world first, every assert passes as written, and `showcase_hall.png` was re-blessed.

### Done: the showcase's palette after the Crystal Islands review: pale rock, more and dimmer glowing rows, a darker key

Done 2026-11-20 (419a0039, 7d3a9c7a, 6abc8a17; goldens re-blessed at db103e2b): pale tinted rock, five-hued dimmer crystals and veins, the trees' three leaf rows by hash, `--exposure-comp EV`. The owner's window P1 kept compensation 0 on every source (the darker key was not taken; the flag stays).


## Performance: the static review of 2026-11-10

Parked 2026-11-10. The owner asked why the window reads 30 to 50 fps with little on screen; a
read-only review of the tree at `50b637a` followed, with no run (the owner asked for none), so
every figure below is an estimate. The window's pacing, the readbacks, the workers and the
sync's time went to [Plan 0064](plans/done/0064-the-windows-frame-pacing-workers-and-readbacks-that-wait.md);
the baked light's shader waste and the queue's scan to
[Plan 0065](plans/done/0065-the-baked-lights-waste-atomics-a-filter-of-zeros-and-a-scan-per-frame.md)
(both approved 2026-11-10). What neither takes:

### Budget lines over with no owner

Most of this entry's lines found an owner; what is left is the last three items.

- **Set at Plan 0069's close (2026-11-17):** the Floor split in `docs/nfr.md` section 2 was rebalanced: upscale (FSR 2), tone map and post 3.0 ms (from 1.5), denoise 2.0, a line for the live bake's slice. The Radeon at Portable reads FSR 2 at 4.06 to 4.08 ms against the 3.0 (`docs/nfr/02-frame-budget.md`, "The render profiles").
- **Set at Plan 0096's close (2026-11-23):** the bake's drain and the queue's step have lines, pinned in section 2 with their misses; the step's is Plan 0099's.
- **Streaming CPU p50, 1.0 ms:** 3.4 to 4.2 ms on record. Plan 0064 (closed 2026-11-11) left it where it was; see "Slot words packed on the workers".
- **The frame ledger** (`docs/nfr/02-frame-budget.md`, "Frame ledger (2026-10-31)"): its scenario tables still read `own`, the table light and the probes, with no row for the list lights or the bake's slice. Plan 0102 (the removal of `own`, a draft) only dates a note at the ledger's head; the rewrite is owed at the next close that re-reads the split.
- **Time to first frame, 3 s** (section 7, a proposal): 7137 ms at the vein under the baked light when Plan 0060 read it; Plan 0103 (the pipeline cache, a draft) re-reads it in its Phase 4.

Sharpened 2026-11-19: Plan 0091's budget scopes (a scope that carries its `nfr.md` line and counts its overruns into `run.json`) make each line executable; a line with no owner then at least has a counter.

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
Drafted 2026-11-12 (the architecture review) as [Plan 0070](plans/done/0070-the-sync-off-the-frame-slot-words-packed-on-the-workers.md)
and ADR-0056. Deferred 2026-11-13 by Plan 0070's Phase 1: the packing is about 1.5 ms of a cave
sync p99 of 44 to 47 ms (RTX host). Read again at Plan 0070's Phase 5 (`ef0b55c`, the rescan set
dense and the light walks on change, `hpet` clocksource): `pack + size` p99 is 2.8 to 3.1 ms of
a cave sync p99 of 20.9 to 22.3 ms, 13.5 to 14.2 % on both adapters; the larger stages are
`light_note` (8.2 to 8.4 ms, `note_lights` in `exposed_emitters`) and `alloc` (6.9 to 7.4 ms).
Pick up when `pack + size` reads half or more of the sync p99 left after those two are cut;
read again at Plan 0074's Phase 4 with `light_note` cut (`docs/plans/logs/0074.md`, lamp off):
the tip cave's `size` p99 2.8 to 3.8 ms and `pack` 2 us, 17.1 to 17.9 % of the sync p99 on the
RTX (16.5 to 21.1 ms) and 16.6 to 17.1 % on the Radeon (16.1 to 16.8), still under half;
the lane's drafts (`live/p2_wire.py`, `live/p2-draft/`) stay untracked in the plan-0070
worktree and leave with it. **Read 2026-11-22 at Plan 0096 Phase 7 (`8bb8857b`, RTX host, cave
bench):** with `light_note` and the eviction scan cut, `alloc` p99 is 9.4 ms of a sync p99 of
19.4 (48 %) and its profile is the per-brick packing path as a whole (about 9.5 µs a brick,
13 383 bricks at the slowest sync; `pack + size` p99 3.0 ms beside it): the rule is met in
substance, since `alloc` is the packing this entry moves. **Ripe since Plan 0096's close (2026-11-23), not drafted:** its
own plan under ADR-0056 (the Outcome extended with this reading), the lines it chases Plan 0096's
bar 3 (`alloc` p99 3.0 ms, `update + poll + sync` p99 16.6, the forest at leaf fill 128 sync p99
6). Architect's routing, owner confirm owed. The same plan takes the glide's `light place` (the
first of Plan 0096's leftovers) and closes "The sync's `alloc` stage".

### The slow adapter's walk-speed spike

Owed since Plan 0054 (ADR-0034 and ADR-0041 name it) and in no plan or backlog entry until now:
why the Radeon's hit pass reads 7 to 8 times the RTX's when the light pass under the store reads
about twice. Run with the shader toolchain spike, when no lane holds the GPU.
Noted 2026-11-12 (the architecture review): the slot is Plan 0069's Phase 4, which reads both
adapters anyway; the review's amendment note asks it to read the hit pass alone at Portable's
internal size on the RENOIR and to run this spike before Phase 6 if the hit reads over 10 ms there.
Plan 0069's Phase 4 read it (RADV RENOIR at 853 x 480: hit 10.53 ms) and the owner accepted it
with no spike before Phase 6 (amendment 3903f104, 2026-11-17). The spike itself is still owed.

### A beam prepass, or a distance value per brick

The primary walk skips by occupancy only: chunk, 2 x 2 x 2 brick groups, uniform bricks, the
in-brick mask. There is no coarse depth prepass and no distance field. A 1/8-resolution beam
pass over the chunk, group and brick levels would start each ray near its hit; a 3-bit
Chebyshev distance per brick, mirror-side only, would lengthen the fixed 16-voxel group skip.
Both pay in halls and outdoors and little in veins. Read the steps view's histogram first; pick
up before Plan 0033's far field, whose rays are the long ones.

**A second arm for the same spike: rasterized brick boxes (owner, 2026-11-22).** Draw a tight
bounding box per occupied brick with the rasterizer and march only inside the box in the
fragment shader, so the rasterizer and the depth buffer skip the empty space (Douglas Dwyer,
"Drawing MILLIONS of voxels on an integrated GPU with parallax ray marching", Voxel Devlog #4,
2022, youtube.com/watch?v=h81I8hR56vQ: 8 x 8 x 8 boxes from an octree, 36 volumes of 256 cubed
at over 60 fps on Intel UHD; he left a compute ray marcher because it was bound by memory
bandwidth on weak cards). Why it is here: the RENOIR's hit pass read 25 to 34 ms at Plan 0096's
window (2530 x 1378, Portable). What it does not touch: the shadow, sky and lamp rays (his
shadows are a rasterized shadow map) and the far trees' wobble. Known costs: a box list kept
current under edits and streaming (a second mirror); a fragment that writes depth loses the
early depth test (he writes true depth to a second target and fixes the depth buffer in a later
pass, which holds only while boxes do not overlap); a ray that misses inside a box is a
discard, and a sparse canopy stacks them, so the forest is the case to read; a raster pass is a
change to ADR-0001's all-compute stack and needs its own ADR. The spike reads both arms on the
RENOIR in the forest and the hall; the boxes are taken only if they beat the beam prepass
clearly there.

### The a-trous under the baked light: fewer iterations, or none

Plan 0065 (closed 2026-11-11) skips the filter only with the lamp off, where its input is zero;
with the lamp on it filters as before. There its input is an analytic light with a binary shadow, no Monte Carlo noise; four iterations may be one or two,
and with the lamp off and the `face` look the whole pass could be skipped on the CPU side. Both
change shadow softness or the rare fallback pixels, so they are the owner's eye. Pick up after
Plan 0058, which widens the filter to rgb.
Checked 2026-11-13 at Plan 0058's refresh: its Phase 1 moves the lamp-off skips onto "some light
is shaded" (`LIGHTING_DIRECT`), so after it the skip holds with the lamp off and an empty list;
the iterations are taken by Plan 0058 (owner, 2026-11-13, its answer 8): a 1, 2 or 4 iteration
render setting in its Phase 2, judged in its Phase 4 window. Skipping the whole pass with the
lamp off and the `face` look stays here for after it.

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
let the compiler drop them. Small each. Plan 0044's Phase 5 (closed 2026-11-16) did only the step
counters (`override COUNT_STEPS`, the pattern to copy); the rest fits the shader toolchain spike.

### Plan 0044's leftovers (close review, 2026-11-16)

Owed by nobody yet; none blocks anything.
- **The plan's four followups:** workgroup-tiled upscale and a-trous with a decoded guide texture
  (a measured plan); one blue-noise load per pixel for `rand` (changes the noise, needs the
  owner's eye); whether the residency pressure rule (`RadiusRule`, `Deferrals`) becomes an engine
  type (an architect interview); a per-update ray budget for the audio probes.
- **`a_frame_after_the_second_builds_no_bind_group` cycles six lighting variations under three
  upscalers**, not the exposure modes, the full lighting tier, the store read, the shadow walk
  or the debug views. A mode that rebuilds a group every frame there would not be caught.
  Widen the test when Plan 0069's settings surface lists the modes.
- **`frame/encode.rs` is at 1160 of the 1200-line cap (read 2026-11-24).** The next pass that adds to the encode
  splits it first.
- **The committed `castle_gate.png`, `look_gradient27.png` and `look_trilinear8.png`** differ by
  a few pixels from the dev box's lavapipe (within the 0.1 % tolerance), at the plan's base and
  tip alike; CI's lavapipe blesses them. Re-bless on CI if one ever drifts past the tolerance.
  **Read again 2026-11-22 (Plan 0097):** the same three differ from the dev box's render the
  same way on main at 123964e7 and on the plan's tip (castle_gate 3376 pixels by at most
  12/255, the two look files 196 and 228), and **`outdoor_night.png` joins them** (1827 pixels
  by at most 2/255, main's render and the tip's byte-identical to each other); all four pass
  their tests, none was re-blessed.

### The tester cave's coarse light is slow to settle

Noted 2026-11-14 in Plan 0074's window (the owner, on the tester's cave path, the tip and the
base the same): "coarse light are slow to settle". Not Plan 0074's stages (its tip and base
differ only in the sync's light note and read the same). A reading of the bake queue's coarse
pass and its settle on a moving path; no run read it. Pick up with the next look or bake-queue
plan that walks the tester's paths.

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
read with the `frames:` line's wall time. Pick up at the next windowed perf session, or if the owner reports the game slowing when tabbed back.

### The dev box presents across two GPUs

The panel hangs off the AMD iGPU, Hyprland composites there, and the RTX has no connected
output, so a windowed frame crosses GPUs before it shows. Windowed readings should say so, and
`references/project-context.md`'s machine facts should carry it. An external display on the
RTX's port would give a windowed reading without the crossing.

### The frame path's GPU waits in the gate

Parked 2026-11-11 at Plan 0064's close review. `a_shown_hud_and_the_queue_line_wait_on_the_gpu_nowhere`
(`apps/flythrough/tests/gpu_frame_waits.rs`, 300 frames, 12.3 s on lavapipe) is soak-marked,
so the fly-through's wiring (the frame calls the ring's reads, not the waiting ones) runs in
no gate; the gate holds only the ring's own test. A variant of about 120 frames with the
queue line due at least once, or a CPU test on the call sites, would bring it under the 10 s
line. Pick up as a dev chore with the next plan that touches the frame's read-backs.
**It failed** (fixed 2026-11-16 on branch chore-soak-frames: the test matches the run's line, accepts frame
0's `store reads not counted` queue line, which prints before the frame turns the count on, and
reads the age as the last item of the parenthesis, which now names the missing light and store
bytes first) (read 2026-11-14 at Plan 0074's close review, soak tier of the changed crates,
not this plan's code): the exit's queue tally prints `frames: queue step ...` lines
(`run/mod.rs`, `Drop for Run`) before the `frames: N frames;` line, and the test takes the
first line starting `frames: `. Match `frames: {FRAMES} frames;` instead.

### The sync's `light_note` stage and the windowed glide's light walks

Taken 2026-11-13 by [Plan 0074](plans/done/0074-the-syncs-light-stages-placement-spikes-attributed-and-exposed-emitters-by-masks.md) (closed 2026-11-14): `light_note` cut by masks; the placement's spikes are "The light placement's eviction scan in a windowed glide" under Next up.


## Tooling and process

### A harness chore after Plan 0098's windows: rehearsed windows, live settings, live banners

Raised 2026-11-23 (owner: "go" on the harness list after windows W2 and W3). Three windows
cost the owner about an hour each and found faults a machine could have shown first: a white
hole at a dig, a pick that changed nothing for weeks (the penumbra under the calm defaults),
and a banner row that printed a constant (the bake tier). **What it takes**, as one chore plan:
(1) a window rehearsal: before a live session opens, each run's pose is rendered headless
(`--headless --frames`, the run's own flags and a scripted dig where the run digs), stills are
saved beside the runner and an agent reads them, so an obvious fault never reaches the owner;
(2) a test that every row of the render settings changes the image under the defaults in at
least one fixture, or is listed as inert with its reason, and the banner marks an inert row;
(3) a test that every banner and HUD row is derived from live state: flip each setting and
flag, the row's text follows; (4) the gate tier's wall time, now about 36 min on the lane's
tree against the 19 min line (see "The gate tier is over its wall-time budget" below): a cut,
not another acceptance. Size M. **Pick up** before the next plan that needs an owner window,
ahead of the follow-on light plan's first window. Item (4) done 2026-11-24 on branch
`chore-gate-time` (1133.2 s quiet, `docs/nfr/07-startup-and-build.md`); (1) to (3) remain.

### Two process-spawning GPU tests skip silently without lavapipe

Found 2026-11-17 at Plan 0088's close review. `apps/flythrough/tests/gpu_path.rs`
(`a_path_run_writes_its_report`, soak) and `gpu_frame_waits.rs` print "skipped: no adapter" and
pass when the fly-through finds no adapter, without the `VXE_REQUIRE_SOFTWARE_ADAPTER=1` panic
that `gpu_loading.rs`'s first helper has. `soak.yml` and `ci.yml`'s Linux arm set the variable,
so a runner without lavapipe passes these tests green. What it takes: the same three-line check
in both, or one helper in `tests/support/` they call. A dev chore of minutes.

### Comment, manifest and index drift found by the 2026-11-12 review

Parked 2026-11-12; a dev chore of one commit, no plan. Read again 2026-11-24, all still there: `apps/flythrough/examples/world_budget.rs` describes ADR-0011's attribute pages and planes ("contour" lines), removed in Plan 0054. `apps/flythrough/tests/gpu_lights.rs` says "the emissive table's path" (deleted in Plan 0059) and "Reservoir reuse's bar", `crates/vxe-render/src/exposure.rs` "reservoirs", `apps/flythrough/tests/gpu_upscale.rs` "reservoir textures" (Plan 0057). `crates/vxe-world/Cargo.toml` and `crates/vxe-render/Cargo.toml` carry `description = "placeholder; see docs/roadmap.md"`, and no crate sets `publish = false`. `vxe-render/tests/gpu_frame.rs` calls `lamp_bounce` off "the default", which is the library's default and not the fly-through's. Pick up with the next chore lane, or with Plan 0087 (the rename), which rewrites the manifests anyway.

### The 64 private-intra-doc-link warnings in `cargo doc -p vxe-render`

Noted 2026-11-17 from Plan 0069 Phase 2 (`docs/plans/logs/0069.md`): `cargo doc -p vxe-render
--no-deps` prints 64 `private_intra_doc_links` warnings (cargo's count), none on a line that plan
wrote; its done-when was amended to "no new warnings from this plan's files". Owner 2026-11-17: fix
them in a separate chore later. What it takes: make each linked item `pub` or `pub(crate)`-doc it
with a plain code span, then a CI step or `RUSTDOCFLAGS=-D warnings` doc build so the count cannot
grow again. Ripe: Plan 0069 merged 2026-11-17.

### Five lavapipe gate tests over the 10 s line

Noted 2026-11-13 at Plan 0070's close review (the gate at `94173ac`, 641.7 s, load 1.42 at the
start and 2.27 at the end; `node scripts/test-times.mjs`). None is Plan 0070's:
`gpu_bake::a_baked_view_is_final_on_its_first_frame` 14.2 s,
`gpu_golden::uniform_chunks_render_identically_from_their_mips` 11.7 s,
`gpu_golden::fsr2_golden_matches_and_rcas_is_bypassed_at_zero` 10.7 s,
`gpu_indirect::the_channel_holds_the_lamps_bounce_only` 10.6 s and
`gpu_queue::the_score_blends_distance_and_the_view` 10.3 s; the first two read over the line at
Plan 0064's close too. The CPU tests over it (`castle_vector`, `showcase_vector`,
`the_castle_path_flies_in_air`, `random_walks_on_the_level_never_freeze_or_overlap`) stay in the
gate by the owner's rule. What it would take: mark each `#[ignore = "soak"]` or cut its fixture,
by a reading on a quiet box. Pick up as a dev chore with the next plan that touches those
binaries.
Read again 2026-11-15 at Plan 0058's close review (the merged tip a2b3717, 897 tests, 774.4 s,
load 3.0 at the start and 1.5 at the end, so slightly loaded): the five above again, plus three
of Plan 0058's own: `gpu_dynamic_lights::a_pulsing_emitter_marks_only_where_its_light_lands`
14.1 s (8.3 s in its log), `gpu_dynamic_lights::a_red_light_near_a_wall_draws_no_rings_under_fsr2`
12.5 s (11.4 s in its log) and `gpu_fsr2::fsr2_marks_a_dropped_list_light` 10.8 s. The
rings test renders three 32-frame images (no light, white, red); fewer still frames or the soak
mark brings it under. Plan 0062 runs on these binaries next and can take them.
Read 2026-11-24 (the gate-time chore, `docs/nfr/07-startup-and-build.md`): the rings test now
renders its three images on threads (19.8 to 13.7 s in the quiet gate, at the clock cap); the
others stay in the gate, the only tests of their mechanism, listed there.

### The lavapipe group at two threads (owner question)

Parked 2026-11-13 at Plan 0072's close. At `max-threads = 2` the gate read 354.6 to 368.2 s
against 656 s at 1 (six runs, 830 of 830 passing, `docs/plans/logs/0072.md` Phase 4), but
`gpu_bake`'s `a_dug_emitter_leaves_the_near_ring_at_once` read 2.5 to 2.7 times its N = 1 time
in every run, so the plan's rule kept N at 1 (the close's default; the owner was asleep). The
owner's call: take N = 2 (nearly half the gate) and accept or soak-mark that test, or keep 1.
A one-line change in `.config/nextest.toml`; pick up on the owner's answer.
**Done 2026-11-24** (owner's go 2026-11-22; the gate-time chore, `909a2552`): N = 2, the gate
2145.0 s to 1190.3 s on its own, 1361 of 1361 passing; `a_dug_emitter_leaves_the_near_ring_at_once`
no longer slows past twice; at 2 one test did (`a_reload_keeps_the_picture_steady`, 9.2 to
23.4 s), at 3 another (`a_baked_view_is_final_on_its_first_frame`, 6.7 to 21.8 s; wall 889.7 s).
Three threads is an owner question from that chore.

### `split-debuginfo` on Linux only (owner question)

Parked 2026-11-13 at Plan 0072's close. `[profile.dev] split-debuginfo = "unpacked"` cut the
`vxe-world` quick-tier rebuild further (12.2 to 12.5 s against 17.8 to 22.4 with line tables
alone; the target dir 7.6 GB against 12.2) but rustc rejects it on `x86_64-pc-windows-msvc` and
wasm32, and a profile cannot scope it to a target. The Linux-only form is `rustflags =
["-Csplit-debuginfo=unpacked"]` under `[target.x86_64-unknown-linux-gnu]` in a new
`.cargo/config.toml`; it changes every Linux build's flags (CI's too, unless scoped), so it is
the owner's call. Not added at the close (the default). Pick up on the owner's answer.

### Plan 0072's CI readings: accepted, one reading left open

Parked 2026-11-13 at Plan 0072's close. Read 2026-11-13 on the first two pushes: warm Linux
15 min 6 s and Windows 18 min 47 s (over the under-10 and under-15 min proposals), gate count on
CI unchanged (844 passed). The owner accepted these walls the same day ("record this time as
accepted, let it be"); they are the CI line in `docs/nfr.md` section 7. Left open, no longer
owed: one manual `workflow_dispatch` run with `fingerprint` set on a prefix-restored cache, to
name why `proc-macro2` and the like recompile. Pick it up only if CI time becomes a complaint.

### The gate tier is over its wall-time budget

**Met 2026-11-24** (the gate-time chore, `909a2552`): 1133.2 s quiet against the 19 min line,
at the CPU's 3.3 GHz cap (`docs/nfr.md` section 7, `docs/nfr/07-startup-and-build.md`). The
history below stays until the architect's clean-up.

Read 2026-11-05 on 7cd77ad (`docs/nfr.md` section 7): 1313.4 s, about 22 minutes, loaded, against the 12 min proposal, with the two tests then over the 30 s line already on the soak list. `flythrough::settle` is the largest binary at 338 s. Ruled 2026-11-05 on the four tests over the line in that run: `the_tone_pass_reproduces_the_composites_bytes_on_every_golden_pose` split (the every-pose test on the soak list; the primary pose alone stays in the gate at 9.2 s), `castle_gate_golden` went back under with its warm-up removed (25.7 s cold, 7.4 s warm, alone), and `smoothed_normals_cut_the_normal_error_and_move_no_hit` and `a_face_hidden_from_every_eye_is_settled_before_it_is_seen` stay (23.9 s and 24.1 s alone). Those two move only if a gate run started on a quiet machine reads them over the line. Moving all four would save at most about 95 s of serial time, so this is not how the budget gets met. Still owed: either cut the slowest `settle` and `golden` tests or re-set the proposal. Much of `settle`'s time goes with the table at Plan 0059's deletion, so re-read then. `the_lamp_toggle_opens_the_light_window` read 98.0 s on CI's last red run, on an older tree, and reads 18.0 s here, so it stays in the gate; CI's next run shows whether it is still over the 60 s line. Pick up as a dev chore when the owner next wants the gate faster or CI green. Read 2026-11-12 (the architecture review, on `live/plan-0068/merge-gate.log`): 809 tests, 610 s; the 177 lavapipe tests sum to 610 s serial, the whole wall, the 632 CPU tests to 273 s beside them, so the lever is the lavapipe group's thread count or the GPU tests' pixels, not the CPU tests; [Plan 0072](plans/done/0072-development-speed-the-build-graph-a-narrowed-gate-and-one-compile-per-ci-profile.md) Phase 4 measures the group at 2 and 3 (drafted, awaiting go). Read 2026-11-09 at Plan 0059's close (`docs/nfr.md` section 7): 850.2 s with `settle` back in the gate and none over the 30 s line, still over the 12 min proposal; the re-read of the proposal stays owed. Read 2026-11-08 on CI's Linux arm, Mesa 26.2.6, the GPU tests in parallel: the same tests move 25 to 40 % between runs (`a_face_hidden_from_every_eye_is_settled_before_it_is_seen` 55.6 s on run 42470428595 and 77.6 s on 42477325818; `the_lamp_toggle_opens_the_light_window` 53.2 then 70.5; `fsr2_adds_little_flicker_on_a_still_vein` 49.8 then 62.8), with several more between 45 and 58 s, so moving tests one push at a time does not converge. Ruled 2026-11-08 (owner): CI's budget step warns per test over 60 s instead of failing the push, until the light work settles; a test that stays over moves to the soak list by hand. Still owed: the architect re-reads `docs/nfr.md` section 7's 60 s line (it says a test over the line moves to the soak list; CI no longer enforces it), at Plan 0059's deletion or when the owner next wants the gate faster. Also ruled 2026-11-08 (owner), from run 42477325818 (2972 s summed over 727 gate tests, 27 tests at 30 s or more holding 41 % of it): the whole `flythrough::settle` binary (34 tests, 746 s) moves to the soak list until Plan 0059's deletion phase decides the table, and CI's Windows arm runs every CPU test but only a GPU smoke set (the `ci-windows` profile: the shutdown and walk binaries and one golden), since WARP skips the image bars and runs a test 10 to 20 times slower than lavapipe. Both are tier moves, not deletions. **Read 2026-11-18 at Plan 0079's close** (`docs/nfr.md` section 7): 1208.2 s on 1174 tests, loaded (load 16.9 at the lock), over the 19 min line the owner set the same day; the plan's own tests add about 17 s outside the serial lavapipe group. A quiet re-read is owed before the line is judged again.

### `vxe-render::exposure` hangs on WARP since Plan 0057 Phase 7

Read 2026-11-06 to 2026-11-09 over seven CI runs; the full record with its tables is in this file's history at `193cae15`. On WARP (`Microsoft Basic Render Driver`, DX12, wgpu `=30.0.1`) frame 0 never finished whenever the per-face table's preload ran: any submission between the preload's args pass and `shade_preload` that touched a buffer `shade_preload` then used hung it, at any table size. The words read back were right, no unbounded loop or bad barrier was found on paper, and the D3D12 debug layer without GPU-based validation cleared the hang and reported no misuse, which points at a race inside WARP, not at a state wgpu records.

**Ruled 2026-11-09 (owner): no GPU tests on WARP.** The `ci-windows` profile runs the CPU tests only (`.config/nextest.toml`); Windows still creates every pipeline on DX12 (the pipeline-times step). Plan 0059 (closed 2026-11-09) deleted the preload, its sweep and `shade_preload`; the fly-through's `diag:` lines stay (`apps/flythrough/src/app.rs`).

**Owed, the owner's call:** one push that runs a GPU smoke set on WARP again. Clean, the `diag:` lines go and this entry closes; still hung, the hang was not the preload's.

### llvmpipe SIGSEGV on CI's Ubuntu since Plan 0057 Phase 7

Done 2026-11-08 and 2026-11-09. The crash was Mesa 26.0.x's own: it reproduced on a local build of the 26.0.8 tarball against LLVM 21 and against LLVM 23, and Mesa 26.2.6 passes, so it was neither concurrency nor LLVM. Ruled 2026-11-08 (owner): no package source that cannot be pinned; CI's Linux arm and the weekly soak build lavapipe from the Mesa 26.2.6 release tarball, checked by its sha256 and cached by it, in the composite action `.github/actions/lavapipe`. **Left:** the build comes out once the runner image ships Mesa 26.2 or later. The bisect, the build options and the hashes are in this file's history at `193cae15`.

### The dev box runs at its base clock since the AC drop of 2026-11-23

Read 2026-11-24 by the gate-time chore: every core's `scaling_max_freq` is 3 301 000 kHz against
`cpuinfo_max_freq` 4 683 471 (Ryzen 9 5900HS, amd-pstate-epp, boost flag 1). asusd switched to its
quiet profile when the AC dropped at 2026-11-23 15:14 and the cap stayed after the AC came back.
Every timing read since then, the gate's among them, is about 1.42 times slower in compute-bound
work than the dev reference's figures before it (the shared tests of two gates a day apart read
1.44 on the GPU tests). What it takes: the owner restores the cap (asusctl's profile or the
sysfs value, root), and readings from 2026-11-23 15:14 until then say "at the 3.3 GHz cap".
A check in the lane rules' power check (`scaling_max_freq` against `cpuinfo_max_freq`) would
catch the next one.

**Restored 2026-11-24 evening (the owner).** What did not lift it: the profile set to Balanced
and back, a root write of `scaling_max_freq`, the boost flag off and on, the charger out and
in. What did: the driver re-registered (`passive` then `active` written to
`/sys/devices/system/cpu/amd_pstate/status`, root), then the profile set again, since the
re-register leaves the energy preference at `balance_performance`. Readings from 2026-11-23
15:14 to then are at the cap; the gate's 1133.2 s is one of them and is owed a re-read at the
full clock. The lane rules' check is still owed.

### The RTX laptop's TSC is marked unstable: hot-loop timings since 2026-11-05 are inflated

Noted 2026-11-13 (Plan 0070 Phase 1, `docs/plans/logs/0070.md`). Every boot of the RTX laptop
since 2026-11-05 logs `clocksource: Marking clocksource tsc unstable due to frequency skew` (not
those of 11-01 to 11-03) and falls back to `hpet`; one `CLOCK_MONOTONIC` read costs about
1.7 us. Readings since then that read the clock inside a hot loop (per brick, per voxel, per
sample) are inflated; Plan 0070's per-brick stage laps turned a 42 to 64 ms sync p99 into 303 ms.
Per-frame and per-chunk readings are affected little. Readings should name the clocksource
(`/sys/devices/system/clocksource/clocksource0/current_clocksource`). Whether to boot with
`tsc=reliable` or `clocksource=tsc`, or find the skew's cause (firmware, a kernel change at that
boot), is the owner's call. Pick up: at the owner's next reboot, or before any plan's budget
readings are compared with ones from before 2026-11-05. Sharpened 2026-11-19: Plan 0091's
profiler reads `Instant` (through its backend) and the GPU timestamps, never a raw `rdtsc` as
Bonsai does, so this entry stays a question of reading cost, not of correctness; per-brick laps
inside hot loops stay out of the profiler's scopes for the same reason.

### The HUD paints a bounded level's full record reservation as mirror pressure

Read 2026-11-15 in Plan 0058's Phase 5f (`docs/plans/logs/0058.md`, the owner's `live/7.png` and `9.png`). On the level the HUD shows `mirror pressure 1.000` in yellow. `GpuWorld::pressure` (ADR-0013) is the larger of the pool-unit term and the record term. `scene::level_mirror_budget` reserves exactly one record per chunk of a bounded level, so once every chunk is mirrored the record term is 1.000 by construction (3200 of 3200; the unit term reads 0.30). No brick was refused and no radius narrowed. It read the same at the plan's start commit. The yellow at `hud::PRESSURE_HIGH` (0.9) therefore warns about a reservation that is full by design. What it takes: the HUD (or `pressure`) leaves out the record term when the reservation covers every chunk of a bounded world, or shows the two terms apart. It is a testbed-and-HUD change with a unit test. Pick up with the next HUD or mirror-budget change.

### The F9 steps view drops the window to about 14 fps

Seen 2026-11-21 at the diagnosis window (`docs/plans/logs/0098.md`, scene A; RTX 3080 Laptop):
cycling F9 to the `steps` view drops the window to about 14 fps and the compositor flags it
unresponsive. Unread: whether the view's own pass is slow or it disables a cheaper path. Pick
up with any work on the debug views.

### Plan 0068's leftovers from the owner's runs

Noted 2026-11-15 at Plan 0068's close. Not seen in either release run: "run all benchmarks" from
the Linux archive (only a castle tour; the three-run zip was read twice from local release
builds) and a free flight in each scene (the Windows report holds the three benchmarks). The next
tester archive's own run covers both; no work unless it fails.

### The built-in walker and its scripted circle go with the library in the tester archive

Parked 2026-11-18 at Plan 0079's close (its Decision 5). `apps/flythrough/src/entities/walker.rs`
(the four-part walker made in code) and `Walkers::walk_pose` (its `f32` circle under the `wander`
toggle) exist only for a run with no entities library: `scripts/package-tester.sh` ships none, so
the tester archive falls back to them. **What it takes:** the archive packs
`apps/flythrough/assets/entities/`, `library::builtin` and the circle go, the `with_walker` table
rows and the tests that place the built-in walker move to the library's walker. A chore, no ADR.
**Pick up** with the next tester release that ships the library.

### Shader toolchain spike

Parked 2026-10-31. Compare naga's SPIR-V against `spirv-opt -O` on the hit shader (VGPRs,
milliseconds, both adapters), then try wgpu subgroups and `shader-f16` on the walk. Rust versus
C for the CPU side was asked and answered no (GPU about 9 of 10.6 ms, CPU about 1 ms). Run when
no lane holds the GPU. Sharpened 2026-11-19: Plan 0092's hot reload is this spike's loop made
short (edit the WGSL, see the frame and the pass's timestamp without a restart), so run the
spike after it lands; the spike's own question, a different compiler on the same WGSL, stays.

### A bugs journal: date, hours lost, tags, the lesson

Raised 2026-11-19 (Bonsai's `bugs.md`: one heading per bug with the date, the hours it cost,
whether it is closed, tags such as `concurrency`, and a paragraph on what it was and what to
remember). Our defects live in plan logs and close reviews, spread over many files, so the
same class of mistake (a held lock, an unstable clock, a stale golden) is not counted. **What
it takes:** `docs/bugs.md` with that heading form, appended at every fix pass and close review
that names a defect; `scripts/velocity.mjs` can count its headings by tag later. Size S. Low
priority. **Pick up** when the owner wants the recurring classes named, or at a review that
finds the same defect twice.

### Mouse pointing and text events for the apps' menu

Parked 2026-11-16 from Plan 0080's draft (owner question 4, recommended later; ADR-0061's
negatives). `vxe_app::Input` reports no cursor position and no text events, so the menu is keys,
wheel and clicks, and its palette types from `KeyCode` (wrong letters on a non-QWERTY layout).
What it takes: `Input::cursor` from winit's `CursorMoved`, the cursor unlocked and shown while
the menu is open (the shell's `lock_cursor` toggled at run time, with the Wayland grab quirks
the dev box has shown), a hit test per row (one division on the overlay's cell grid), and
`KeyEvent::text` passed through as bytes for the filter. Pick up when the owner asks to point
at the menu, or when a non-QWERTY layout types wrong.

### A radial or hold menu for the pad

Parked 2026-11-16 from Plan 0080's draft (owner question 5). The pad walks the same tree as the
keyboard (`Start`, the D-pad, `South`, `East`). A radial menu held on a button, or a hold-to-open
wheel for the entries used most, is a second view over the same `menu::entries()`, drawn on the
overlay's cell grid as a ring of labels. Pick up after Plan 0080's window, if the owner plays
with the pad and the tree is too slow.

### A documentation site

Parked 2026-11-11 (owner: "no plan needed yet, will address this later"). A public site in the
manner of Ritmolux's (Starlight, rustdoc beside it, mermaid built in CI, a Pages workflow that
checks every PR and deploys from main). Two steps: first the existing `docs/` (ADRs, plans,
roadmap, budgets) plus rustdoc, with CI as a link and render guard; a getting-started guide and
a gallery only once the look is picked (after Plans 0067 and 0058) and Plan 0045 has moved the
files. Starlight's npm tree falls outside `cargo deny`, and mdBook is the lighter alternative,
so it takes an ADR. Pick up when the owner asks. Sharpened 2026-11-19: a cheap first step
needs no site: four numbered guides in `docs/` the way Bonsai's `docs/00_getting_started.md`
to `03_async.md` read (get started, build, a first consumer, the threads), each under a page.

### The tester mode's key filter is structural, not tested in the frame loop

Taken by [Plan 0080](plans/done/0080-the-fly-throughs-menu-one-overlay-a-palette-and-the-minimal-key-set.md) Phase 2 (closed 2026-11-17, v0.62.0): a test drives the frame loop's input step with the filter on (`apps/flythrough/src/run/menu_tests.rs`); `Tab` and `Start` map to nothing.

### Done: the tester's benchmarks with the lamp off

Done 2026-11-13 (4ea561f): a `--path` benchmark starts the lamp off unless `--lamp` is given, and `run.json` names it. **Bench readings of the cave and castle paths from before 4ea561f ran with the lamp on and are not compared with readings after it.**


## Ideas from Bonsai

Parked 2026-11-19: the owner pointed at [Bonsai](https://github.com/scallyw4g/bonsai), a C++
voxel engine with meshed chunks, GL deferred shading and an in-engine editor, under WTFPL, active.
The source paths below are Bonsai's. What we already carry was left out: octree LOD, generation-time
stamping, AO, traced shadows, OIT, particles, the profiler, debug views, settings files, the
sky. A second pass (2026-11-19, read against this file) added the entries from "Per-job
performance tracking" on, the profiler (Plan 0091) and WGSL hot reload (Plan 0092).

**Rejected as alternatives (2026-11-19), so nobody re-opens them from Bonsai:** its 2.0 moved
generation and editing to the GPU; ours stay on the CPU in `vxe-world`, headless and
deterministic (ADR-0001, the non-negotiables), and a GPU source would break replay from seed
plus log (ADR-0007). Its 1.6.2 stores a 16-bit HSV colour per voxel; ours stores a material id
whose row carries colour, class and an opaque payload (ADR-0065), so a colour change is a row
edit, not a re-write of every voxel.

### A hybrid far field: raster the far LOD as meshes, march near

Raised 2026-11-19 in the Bonsai comparison: Bonsai rasterizes meshed chunks. For us the hit pass
reads 1.4 to 1.9 ms of a 7.8 ms GPU frame (RTX 3080, hills, the 86c window's HUD); the rest is
light, denoise and FSR 2, which raster would not remove. Meshing every 10 cm voxel would also cost
triangles, edit latency and memory beside the brick map. Where raster can win is a long view, where
the march crosses much empty space to the far LOD. **What it takes:** meshes of the far mips only
(they are not edited at that scale), drawn into the hit buffer's depth and normal before the march,
which then starts at the near band. An ADR, and measurements on both adapters. **Pick up when** a
reading names the far field's march as the frame's largest stage, or a view-distance budget
cannot be met on the baseline GPU. **If picked up (2026-11-20):** the mesher to start from is
binary greedy meshing (Davis Morley, "Greedy Meshing Voxels Fast", Handmade Seattle 2022): one
64-bit occupancy mask per column, faces found by `col & !(col << 1)` per axis, rows merged by
bit runs, quads packed in 32 bits; the talk reports meshing in tens of microseconds a 64^3
chunk, which would make re-meshing an edited far mip cheap. His "global lattice" (raster one
plane per voxel slice and look the voxel up per fragment) is rejected for us: it is a renderer
in place of the march, with its overdraw.
**Owner, 2026-11-20:** leans to this hybrid as the answer for long-distance rendering. Plan
0033 (the marched clipmap, ADR-0028) is re-read against it before its lane starts: raster the
clip levels' meshes against marching them, measured on both adapters, as an ADR either way.
**Owner, later 2026-11-20:** "agreed on far distance": the hybrid is the agreed direction for
the far field, not only a lean. The re-read runs after Plan 0087 (the rename), as the first M10
step; the meshing explainer and Morley's talk are in `references/voxel-optimizations.md`.
The key light's far pixels casting the ray inside the far normal's band ("Narrow the far
normal's band", under Engine mechanisms) was taken by Plan 0098.

### World-space debug lines: lines, boxes and a highlighted voxel over the frame

Raised 2026-11-19 (Bonsai's `src/engine/debug_draw.cpp`: lines, AABBs, a highlighted voxel,
chunk boxes). We have a cutaway view, the HUD's text grid and the entity mark (ADR-0074); no
way to draw a chunk's box, a sweep's path, a light's reach or a job's chunk in the world.
**What it takes:** a line list the app fills each frame (world-space segments with a colour),
drawn by one render pass after the composite and depth-tested against the hit buffer's
distance, so a line behind rock hides; `vxe-render` owns the pass, the app owns the list.
Size M. M7's debug views (material ids, coarse grids, flags) need it as their canvas.
**Pick up** with the first M7 debug view, or when a streaming or collision entry above needs
to see where a job or a sweep went.

### Per-job performance tracking: reserve, retire and scopes per streaming and bake job

Raised 2026-11-19 (Bonsai's `work_queue.h` `TrackPerformance` flag; its debug view draws a
job's scopes over its reserve-to-retire span). Our streaming workers (`vxe-world`
`stream/mod.rs`) and the bake queue's jobs are timed as stages and per-second lines, never as
one job from its queueing to its retirement, so a job that waits long and runs short reads the
same as one that runs long. **What it takes:** a job id and a reserve and retire frame index on
each job, recorded into the profiler's frames as a span on the job's worker lane with the
frame it was asked for and the frame it landed; a tally of jobs retired late per second on the
HUD. Size S on top of Plan 0091, which owns the lanes. **Pick up** after Plan 0091, when a
streaming or settle entry above asks why a chunk arrived late rather than why it ran slow.

### Fan-in continuations: "generate these chunks, then bake their light" as one dependency

Raised 2026-11-19 (Bonsai's `OnComplete`, `docs/03_async.md`): a job may name a continuation
that runs once every job in its group has retired, so a fan-out of chunk jobs fans in without
polling. Ours poll: the live queue re-checks each frame whether a chunk's reach is resident
before its bake may run (the "reach-bound" census). **What it takes:** a group counter on the
streaming pool's jobs and a continuation the last one fires (the bake request for the group),
scheduling only, no change to what a job computes, so determinism holds. Size S to M, in
`vxe-world`'s pool with the bake queue as the first client. **Pick up** when the settle
readings (the entries under Next up) name the poll's latency, not the bake, as the wait.

### Edit coalescing per chunk per tick: a check first

Raised 2026-11-19 (Bonsai's 1.6.1 notes, per the research pass: several edits on one chunk in a
tick give one re-mesh). Ours should already coalesce: `sync` visits the world's dirty-chunk set
(`vxe-render` `gpu_world/mod.rs`), a set, so two ops on one chunk between syncs should give one
upload. Not asserted anywhere read. **What it takes:** a headless test that applies two carves
on one chunk, syncs once, and reads one chunk upload from the sync's counters; if it reads two,
the fix is in the dirty set's handling. Size S. **Pick up** at the next sync or edit-cost plan.

### Hot-reload a logic component, then replay to the current tick

Bonsai reloads its game library when the file changes (`game_loader.cpp`). We can go further:
swap the WASM component when it is rebuilt, then replay the op and input log from the seed to
the current tick, so state stays exact (determinism is what makes this possible). **What it
takes:** a watcher in the fly-through and a re-instance in `vxe-script-wasm`, driven through
`vxe-net`'s log. Size S to M. **Pick up with** the authoring loop (ADR-0072), or sooner if
writing logic gets slow. Sharpened 2026-11-19: one mtime watcher in the fly-through should
serve every reloadable file: the WGSL (Plan 0092 builds the watcher), this component, and the
settings file (Bonsai's `settings.init` and `hotkeys.init` are parsed by struct member name,
its 1.4.2 notes), so a second reload adds a path and a handler, not a second poll.

### A two-stage world source: shape, then decorate by slope

Bonsai's generation shapes (solid or air, with colour), takes the gradient, then decorates with
the surface normal in hand: cliff rock on steep faces, grass on flat ground. Its user stages
hot-reload in a playground (`docs/terrain_gen.md`). **What it takes:** a `WorldSource` helper in
`vxe-world`, on the CPU, because Bonsai's GPU generation would break the headless world. Add a
reloadable parameters file in the fly-through for tuning. Size S to M.
**Pick up with** M10's outdoor work or a consumer's surface source. Sharpened 2026-11-19: the
part worth copying first is Bonsai's noise-to-value remap through a polyline (a table of
`(in, out)` points, integer-friendly) and the small visualizer that draws the curve while it is
edited (its 1.3.0 release notes); terrace, plateau and cliff come from that one remap.

### Edit algebra: smooth booleans, shape modifiers, more primitives

Bonsai's edits union, intersect and subtract, each also smooth with a blend radius, and every
shape takes hollow, rounding, stretch, repeat and rotation over rect, sphere, line, cylinder,
plane, torus and pyramid (`src/engine/editor.h`, `shaders/terrain/world_edit.fragmentshader`).
We have sphere, box and capsule with carve, fill and replace (`vxe-world` `shape.rs`). **What it
takes:** new shape kinds and modifiers in `vxe-world`, with the smooth minimum in fixed point (a
polynomial one), each still one logged op that reports what it changed. Size M. **Pick up when**
a consumer wants arches, rounded rooms or hollow shells as single edits.

### Layered procedural brushes as one op

A Bonsai brush stacks up to 16 layers. Each layer is a shape or a noise (Perlin, Voronoi, white,
lattice), with a blend (add, subtract, multiply) and clamps or thresholds, and is saved as a
`.brush` file (`docs/editor.md`). That gives crater, erosion and growth brushes instead of perfect
spheres. **What it takes:** an op kind in `vxe-world` that names its brush by content id, like
ADR-0014's prefabs; seeded integer noise; the brush file in `vxe-assets`. Size M to L, likely an
ADR. **Pick up after** the edit algebra above.

### Edits you can still change: named layers, hide, reorder, regenerate

Bonsai keeps a level as records of edits (region, brush, rotation, order, hidden), grouped into
named layers. Chunks re-apply the edits that overlap them when they are generated
(`editor.h`, `world.cpp`). We replay an append-only log (ADR-0007) and persist edited chunks
(ADR-0003); moving, hiding or reordering a past edit is new. **What it takes:** a level document
beside the runtime log, which regenerates the chunks an edit touched. Size L, an ADR; read it
against ADR-0072's authoring text first. **Pick up with** the editor entries under Later apps.

### Example-based generation from a `.vox` (voxel synthesis)

`src/engine/voxel_synthesis.cpp` cuts a `.vox` into 8-cubed tiles, records the six-direction
adjacencies and solves a new volume by constraint propagation, in the style of WFC. One
hand-made sample would give endless cave or room variants. **What it takes:** baking the rules in
`vxe-assets`, and a seeded solver as a `WorldSource` in `vxe-world`. Size M to L. Unverified:
whether Bonsai 2.0 still wires it into generation. **Pick up when** a consumer wants generated
interiors from samples.

### Versioned saves and logs, migrated on load

Bonsai's serialisation versions each struct and keeps the old layouts (`world_edit_0`), which it
converts forward on load (`serdes.h`). Our spill and log files refuse any other version
(`persist.rs`, `wire.rs`). **What it takes:** pure migration functions in `vxe-world`, one per
version step, each tested on fixed vectors. Size S to M. **Pick up before** the first consumer
ships a save or a recorded log that must survive an engine upgrade.

### Inspector rows derived from struct annotations

Bonsai's `poof(@do_editor_ui ...)` generates a live inspector for any struct, with ranges,
conditions and skip flags, so brushes, particles and nodes need no hand-written UI. Our field
table (ADR-0058) covers only the render settings. **What it takes:** a small proc-macro crate
that derives overlay menu rows, used by the apps. Materials, brushes, sources and poses would
then get rows for free. `crates/` gains no UI dependency. Size M. **Pick up with** the
editor-interface entry under Later apps. Sharpened 2026-11-19: the annotation set to copy is
small: a range (`@ui_value_range`), a skip (`@ui_skip`) and an escape to a hand-written row
(`@custom_ui`), as Bonsai's `render_settings.h` uses them; the derive needs no more at first.

### Bloom from HDR emission

Bonsai gathers emission above 1.0 into a luminance target and blurs it through a three-level
downsample and upsample chain (`src/engine/bloom.h`, `shaders/bloom_*`). We have no bloom.
Glowing lamps and ores in the dark suit the first consumer. **What it takes:** a pass in
`vxe-render` after the upscaler, as a render setting (ADR-0055). It must add no flicker
(ADR-0076), and the owner judges it in a window. Size S to M. **Pick up after** Plan 0090's pop
counter, which can hold it to rule 1. **Weakened 2026-11-20** by the review
[`reviews/2026-11-20-crystal-islands-look.md`](reviews/2026-11-20-crystal-islands-look.md)
(trait 10): the Crystal Islands frames show faint bloom at most, with sharp crystal edges at
2:00 and 3:40, so the look the owner liked does not rest on it; it ranks last of that review's
engine levers.

### Lit world-space sprites (cards)

Bonsai's cards are instanced quads written into the g-buffer, so they are depth-tested, lit and
occluded like geometry. They can billboard, take a tint and an alpha cutoff, and share one sprite
array (`src/engine/card.h`). Uses: markers, decals, cheap debris, and the impostors that "hundreds
of entities" names. **What it takes:** sprites traced or merged into `vxe-render`'s hit buffer,
which touches its layout. Size M. **Pick up when** far entities or decals are asked for.

### Allocation counting per frame in the apps

Taken by [Plan 0091](plans/0091-the-frame-profiler-scopes-across-threads-a-gpu-lane-a-dump-for-agents-and-a-hud-ticker.md) Phase 6 (approved 2026-11-19): a counting `#[global_allocator]` wrapper in the apps under its feature, the count on the HUD and in `run.json`.


## Later apps

### Designer authoring: level and logic text, a separate compiler, an editor over the text

Parked 2026-11-17 (owner: "letting non programmer build levels and game logic", "a layered
architecture and keep the text", "a future idea after everything that we have will be polished").
ADR-0072 (proposed) records the frame: five layers, text the truth at each, a compiler in
`tools/` that emits a logic component and baked level data, an optional editor that reads and
writes the same text, both the human and the agent workflow first class. Its shortlists (the
rule language against a Lua dialect, the compiler's home, level editing before an editor, the
engine's mechanisms: a level loader, named regions and their events, a replay-based component
swap, machine-readable run output) carry a recommendation each and six owner questions.
**Waits on the engine's own mechanisms:** destruction, physics and water (the chunk-scale
dynamics entry above, M8, M10's liquids) and the current polish. Pick up with a spike plan, as
the ADR's "What a first plan would be" describes; questions 1 to 5 are answered then.
Question 6 answered 2026-11-18 (owner, the interim rule, (a)): every new text format the engine
grows is designed as a layer-4 file (line-based, hand-parsed, diffable, positions in its errors);
new fly-through scenes stay Rust until the stack exists.

### A proper editor interface, like Unity's or Godot's

Parked 2026-11-18 at Plan 0077's Phase 10 window (owner: "as a backlog we should build a proper
interface, similar to the one that has unity or godot"). The fly-through's menu, belt and chooser
are a testbed's text overlay; the owner wants a real editor UI: docked panels (a scene tree, an
inspector of a placed thing's parameters, an asset browser with thumbnails, a viewport with
gizmos to move, turn and scale, undo). How it fits: the roadmap refuses a scene editor inside the
engine ("no ECS, no scene editor"), so this is its own app under `apps/`, not a crate mechanism,
and it is ADR-0072's layer 5: a view over the layer-4 text files, writing them back in canonical
layout, never a format of its own. Open when picked up: the UI toolkit (an immediate-mode Rust UI
such as egui over the wgpu device, or another; ADR-0009's reasons against a UI library in the
fly-through are the testbed's, and an editor app reopens them), and whether it waits for
ADR-0072's text formats (recommended: the level file first, so the editor has something to edit).
Pick up after ADR-0072's spike, or earlier as a placement-only editor over today's entities if
the owner asks.

### The placing interface, from best practices (part of the editor epic)

Parked 2026-11-19 at Plan 0083's window 83d. Owner: the placing works better after 7d (ground
snap, `F` free, `E` places, a digit swaps), "but for right interface of placing we should create a
separate backlog entry and look for best practices", as part of the editor epic above, later. The
fly-through's placement mode grew one request at a time. Those requests were a belt, a frame
ghost, then a coarse ghost, the snap, the free mode, a scale of 1 to 4 whose limits show only on
stdout ("castle: scale 1 only", "over the edit limit of 1024"), and keys added as the mouse went
missing. **What it takes:** a survey of how editors and building games place large objects
(Unity, Godot, Unreal, Minecraft's structure blocks, Teardown's editor, Valheim and Satisfactory
build modes, MagicaVoxel). Cover snapping (grid, surface, footprint), free placement, rotate and
scale gizmos with their limits shown, a preview that tells what will be cleared, undo, and
keyboard, mouse and pad parity. Then a design that this entry's plan and the editor app share.
**Pick up with** the editor-interface entry above.

### Web demo as a separate app

Decided 2026-10-16: a WebGPU and wasm demo for blog interactivity is a later, for-fun item and
its own app under `apps/`. The fly-through and the native paths never change or lose performance
for it; any crate support is target-gated. Not before the engine has steady lighting, coloured
materials, a sky and a way to build interesting architecture.

### A second consumer in mind: a user-built, multiplayer game platform

Parked 2026-11-17 (owner asked whether the engine could carry a Roblox-style game). Already fits:
runtime edits as logged deterministic ops, sandboxed logic components (fuel, memory cap,
deterministic clocks), `vxe-net`'s server tick and op logs, voxel-model entities and prefabs,
ADR-0072's authoring stack. Missing in the engine: rigid bodies (they conflict with ADR-0008 and
need an ADR), the network at scale (QUIC, interest management, dozens of players per server),
platforms beyond Linux and Windows (next entry). The platform itself (accounts, avatars,
publishing, moderation, a marketplace) is a consumer product, never `crates/`. Use: a second
consumer to test the physics, networking and authoring designs against; no plan of its own.

### Mobile and tablet targets: Android and iOS

Parked 2026-11-17 (owner, "a distant and theoretic idea"). `wgpu` and `winit` reach both, so the
port itself is small; the cost is elsewhere. **A render profile for tile GPUs** (the Radeon iGPU
already reads 22 ms of GPU on Plan 0088's outdoor bench; a phone has less and throttles): lower
internal resolution, baked light only, smaller residency. **A WGSL limits audit** against mobile
`wgpu` limits (storage buffers per stage, workgroup size, missing atomics and subgroups). **Touch
input and the app lifecycle** (suspend, surface loss). **Wasmtime on iOS** has no JIT: Pulley
(its interpreter) or ahead-of-time components. **aarch64 determinism vectors in CI.** A new
target is an ADR (ADR-0001 names Linux and Windows) and about a milestone's work; the cheap first
step is the limits audit, which says what the rest costs. Tablets before phones.
