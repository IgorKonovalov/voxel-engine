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

### The light placement's eviction scan in a windowed glide

Taken 2026-11-20 (draft, awaits the go) by [Plan 0096](plans/0096-the-bake-queues-drain-tiers-a-drain-slice-the-step-and-the-slow-adapters-light-pass.md) Phase 7: the first-fit index and a distance-ordered victim index, the bytes asserted equal, a headless glide with a shrunk light buffer as the reproduction; the figures moved into the plan's context.

### The CPU settle outdoors: the bake queue's step on a world with no emitter, and the forest's sync p99

Taken 2026-11-20 (draft, awaits the go) by [Plan 0096](plans/0096-the-bake-queues-drain-tiers-a-drain-slice-the-step-and-the-slow-adapters-light-pass.md) and [ADR-0080](adrs/0080-the-live-queue-bakes-what-the-image-reads-a-record-by-rule-a-tier-by-size-on-screen-and-a-drain-slice.md): a record by rule for a chunk with no face or buried with no emitter in reach (Phase 2), the step off the queue's depth (Phase 5), the forest's sync p99 (Phase 7); the readings quoted in the plan's context.

### The streamed benches on Windows: CPU frames of 20 to 256 ms on a desktop

Read 2026-11-15 from an outside tester's archive (v0.51.0, before Plan 0070's and 0074's sync cuts; Windows 10.0.26200, Ryzen 5 7600X 6 cores / 12 threads, 32 GB, desktop RTX 3080 on Vulkan, 1920 x 1080, upscaler off; the files kept in the main checkout at `live/tester-2026-11-15/`). `showcase`: wall p50 3.0 ms, p99 4.5, CPU p50 0.76. `cave`: wall p50 20.8 ms, p95 271, p99 402, CPU-bound (CPU p50 20.3, GPU 4.5), 217 of 1073 frames over 100 ms. `castle`: 97 frames in 31.5 s, CPU p50 256 ms, max 3295, GPU p50 30.2. Our Linux headless benches at the matching base read `update + poll + sync` p99 43 to 47 ms, so the castle is about 5x to 70x worse there, and the tester called it "massive stutters". Unknown: whether it is Windows (timer and sleep granularity, the allocator, thread scheduling on 6 cores), the CPU's core count, or the build. What it takes: the owed Plan 0068 Windows run on today's main with the sync's stage lines, then a plan. Pick up before the next tester archive goes out. **The owner's run (2026-11-15, v0.54.0, Windows 10.0.19045, Ryzen 9 5900HS 8 cores / 16 threads, RTX 3080 Laptop on Vulkan, FSR 2 at 1.5; files in `live/windows-run-2026-11-15/`) did not reproduce it:** CPU p50 cave 8.8 ms, showcase 1.4, castle 7.2; castle wall p95 28.2 ms, 87 frames over 33 ms, 2 over 100, the sync's p95 mostly `alloc`. Still open for the tester's machine; the next archive's `--workers 4` run tells starvation apart. **The tester's rerun (2026-11-16, v0.54.0, same machine; files in `live/tester-2026-11-16/`) settles it: the 0.51.0 stutters were the build, not Windows.** Default workers: cave wall p50 4.1 ms, p99 25.5, 32 frames over 33 ms, 1 over 100; showcase p50 2.8, p99 3.7; castle p50 4.2, p99 30.1, max 90, 20 over 33 ms, none over 100 (from p50 256 ms on 0.51.0). `--workers 4` on the castle: p99 17.6, max 66, 2 over 33 ms; the sync's p99 7.3 ms to 3.8 and light's 2.1 to 0.8. So more workers than physical cores (11 on 6 cores) still costs the tail. What it takes now: a default worker count capped at the physical cores minus one (or a measured sweep on a 6-core part), as a small chore; the entry closes with it. **Done 2026-11-16 (chore `chore-default-workers`):** the fly-through's default `--workers` is now half the logical cores, at least 1 (6 on the tester's 12 threads, where `--workers 4` read the best castle tail; no sweep between 4 and 6 was taken); the tester's next archive reads it.

Beside it, 2026-11-17, an outside tester's Linux archive (v0.54.0 at 5efd884d; Ubuntu 26.04,
Wayland, Ryzen 7 5700X 8 cores / 16 threads, 32 GB, desktop RTX 3060 on Vulkan, 1920 x 1080
display, benches at 1280 x 720 with FSR 2 at 1.5, lamp off; kept at `live/tester-2026-11-17/`).
GPU p50 2.3 to 2.4 ms in every scene. `showcase`: wall p50 2.7 ms, p99 3.2, max 16. `cave`: wall
p50 4.5 ms, p95 8.1, p99 23.0, max 65, 38 of 12 887 frames over 33 ms, CPU p99 22.1 with
`sync_us` p99 9.5 ms (max 20.9) and `light_us` p99 2.6 ms (max 10.3). `castle`: wall p50 4.3 ms,
p95 10.7, p99 24.5, max 95, 24 of 5 572 frames over 33 ms, CPU p99 24.2, `sync_us` p99 9.0 ms.
So on Linux the streamed scenes are CPU-spike bound (the sync and light stages, the alloc entry
below), not the Windows-scale stall; whether Windows is the OS or the older base stays open.
`holds_pool` reads false only because the RTX 3060's binding limit is 4 bytes under 2 GiB.

### Windows draws the whole image two to three times darker than Linux

Noted 2026-11-15 from the owner's Windows run (same laptop, v0.54.0, RTX on Vulkan, the window
on the AMD iGPU's display): every scene, the overall brightness down by about 2 to 3x, not only
the dark tones. A decode without its encode (`pow(x, 2.2)`: 0.5 to 0.22) would fit; by a code
read the present pass matches the surface format, but the format picked is not logged. Also
suspected: the panel's backlight on Windows (AMD Vari-Bright, power plan). What it takes: log
the surface format and colour space at start (and into `system.json`/`run.json`), compare a
Windows and a Linux screenshot of the same pose by pixel values, and swap `fs_srgb`'s
`pow 2.2` for the exact sRGB decode. Pick up before the next tester archive. **Code side done 2026-11-16** (chore-surface-format: the format and colour space logged and in `run.json`, 8-bit UNORM preferred, the exact decode): by code read the Windows run used `Bgra8Unorm` and a straight copy, so no code path explains a uniform 2-3x drop. Left: a Windows screenshot and a Linux one of the same pose compared by pixel values (equal bytes: the display or OS, e.g. HDR's SDR brightness, Vari-Bright).

### The sync's `alloc` stage: the sync plan after the eviction scan

Taken 2026-11-20 (draft, awaits the go) by [Plan 0096](plans/0096-the-bake-queues-drain-tiers-a-drain-slice-the-step-and-the-slow-adapters-light-pass.md) Phase 7 (`alloc` p99 at most 3.0 ms, `update + poll + sync` p99 at most 16.6 in the tester cave bench), after Phase 1 (6)'s profile of the stage.

### Done: test tiers by marks, not by copied lists

Done 2026-11-10 (50b2193, 4a15e60): GPU test files are `tests/gpu_*.rs`, long series `#[ignore = "soak"]`, kept by `scripts/check-test-tiers.mjs`.

### A headless run at `--residency 3,4` hangs before its first frame

Noted 2026-11-17 by the Plan 0032 lane (`docs/plans/logs/0032.md`, Phase 3): a headless
fly-through with `--residency 3,4` hangs after the live queue starts and before frame 1, on the
plain cave as well as the outdoor source, so it predates that plan; `6,8` and `8,12` run. Not
yet bisected. Take it as a chore when a small residency is next needed (a test rig or the
Portable profile's floor), starting from which stage waits.

### `--adapter NAME` that matches nothing falls back silently

Noted 2026-11-17 by the Plan 0032 lane (Phase 6 readings): `--adapter radeon` and `--adapter rtx`
match no adapter name on the dev box and the fly-through runs on the RTX without a word; the
iGPU needs `--adapter AMD`. A reading taken that way is labelled with the wrong machine. Fix as a
chore: refuse (or print the adapters and exit) when no adapter matches, and accept the short
names the tests' `VXE_TEST_ADAPTER` takes.

### `StampedSource` shares its prefabs across placements

Noted 2026-11-17 by the Plan 0032 lane (Phase 8, `docs/plans/logs/0032.md`): `vxe_world::StampedSource`
keeps its own rotated copy of the prefab for every placement, so a forest of about 3,850 trees
would hold 3,850 copies (estimated 1 to 2 GB, not measured). The lane wrote a fly-through
`ForestSource` over 64 shared prefabs instead. Engine follow-up: placements reference a shared
prefab and rotation (an `Arc` and an orientation), so a dense generation-time scatter costs its
distinct prefabs, not its placements. Pick up at Plan 0032's close or before the next scatter.

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

### The first near-lighting step stalls 130 to 210 ms on a cold driver cache

Noted 2026-11-16 from Plan 0075 Phase 7b (`docs/plans/logs/0075.md`, session 75c; RTX host). With
every pipeline built on the start-up worker, one loading gap stays at the first "lighting the near
chunks" frames, cold only (warm max 87 ms). Every `vxe-render` compute pipeline exists before it,
so the time is something else the driver does on first use. The owner noticed no hitch and
accepted it. Pick up when a start-up or first-frame plan needs the bar, with a driver trace.

### The bake queue ordered by what the march touched (2026-11-20)

From a devlog (voxelbee, "Adding A CACHE To My Custom VOXEL Game Engine", devlog 5): a GPU-run
cache where every voxel a ray renders moves to the front, so residency follows visibility. We
do not want its eviction (a turn would reveal unloaded data: pop-in against the stable-image
rule; the mirror's distance window, ADR-0013, stays). The part worth taking is the signal: the
march sets a bit per chunk it hit this frame, read back a frame late. Plan 0096's in-view drain
orders jobs by an estimated size on screen; the touched set would order them by actual
visibility, skipping chunks behind hills or the camera, and could order the mirror's uploads
the same way. Cost guess (not measured): one atomic-free store per hit into a bitset of the
window's chunks, one small readback. Pick up inside Plan 0096 if its in-view drain misses the
10 s bar.

### A wgpu pipeline cache owned by the engine

Noted 2026-11-16 from the NVIDIA GLCache overflow (over 1 GB, so nothing new was kept and every
start recompiled). `wgpu::PipelineCache` on Vulkan, saved under the game's cache directory, would
make a warm start independent of the driver's own cache size and eviction. Pick up with the next
start-up time plan, or when a tester reports long starts. Readings: Plan 0094 Phase 2a, the
RTX's light stage 16.5 to 18.1 s cold against 0.1 s warm, and its window 94a's run 6, where
`--leaf-shadow absorb` (a default-off arm, its light pipelines built when picked) cost 18.2 s
on the first world frame on a cold driver cache (`docs/plans/logs/0094.md`, Phase 7); Plan 0094
Phase 7a moves that build into the loading stage, the cost itself stays this entry's.

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

### A foundation under a snapped structure

Parked 2026-11-19 from Plan 0083's Phase 7c verdict (window 83c: the castle's ghost floated
over the hills). Phase 7d snaps a structure's box to the terrain under its footprint, one column
winning (the lowest, owner question 9), so on a slope one side is sunk or floats. A foundation
that fills the footprint's columns from the base down to the terrain would remove the choice; it
is a stamp that writes into air only, which Plan 0084's write policy (ADR-0067) gives a stamp.
Pick up when 0084 lands: a `foundation on|off` beside `clear box`, the fill in the structure's
most-used row, as a third stamp of the click.

### Plan 0083's leftovers (close review, 2026-11-19)

Found at the close ([Plan 0083](plans/done/0083-anything-from-a-vox-palette-materials-a-material-file-and-large-scenes-stamped-into-the-world.md), v0.66.0); none blocked it, each is a small fix pass when the area is next open. **Pending placements (`vxe-world`, ADR-0066's Outcome).** `World::stamp` takes `&Prefab`, so a placement past residency copies the prefab into a fresh `Arc` (`stamp.rs`, the `Placement` push) instead of sharing the registry's, and a rotated stamp materialises its rotation once more: bounded by the pending prefabs' bytes (the castle's 45 MB again at most), a `stamp` over the `Arc` removes it. The poll thread blits every placement an inserted chunk lists under the store's write lock (`stream/mod.rs`, `commit_residency`), up to a 64³ blit per placement while every reader waits; blit into the held chunk first, then apply only what arrived since under the lock. A placement whose remaining coordinates lie outside a finite `World::extent`, or where the camera never goes, lives for the session; drop the coordinates a finite extent can never stream at stamp time and report the pending bytes beside the count. Two stamps on one world at once (not reachable through the server, whose ops are serial) can order an overlap differently in resident and pending chunks: say in `stamp`'s doc that the caller serialises stamps. **Scenes (`vxe-assets`).** `VoxScene::load` honours a hidden layer only on the outermost layered transform of a path (`scene.rs`, the group arm): a shape on a hidden layer under a group on a visible one is hidden in MagicaVoxel and placed here; check every transform's layer and add the nested case to `hidden_layers_and_nodes_are_not_placed`. The grammar example in `materials.rs`'s crate doc and a comment in `rows.rs` name the fly-through's castle and torch; a neutral example costs nothing. **The coarse grain (`vxe-render`).** Instance word 27 and the inverse map are derived from the one f32 grain `detail / coarse` and inverted, one ulp off for coarse 7, 13, 14 and 15; write word 27 as `coarse / detail` directly. **The testbed.** The placed notice counts the ops submitted, not the tick's applied events, and `register_prefab`'s errors are dropped, so a stamp refused at the tick still reads `placed`. A click while the ghost body's worker is building the same scaled scene builds it again on the frame thread (`scene_at`); refuse with `building` or wait on the worker. `Entry::refusal` adds the 1 m margin on every axis while the air box adds it above only and `clear box` off adds none, and prints the un-margined size it did not check. `Modes::edit_axis_max` starts at the default until a slot is entered, so the chooser's belt row can show the wrong verdict on a world with another limit. `a_key_places_as_the_click_does` compares prefab ids, not `(prefab, origin, rot)`; `a_placed_structure_is_logged_stamps` pins the chapel's per-row counts as a sorted list, losing row identity. `run/structure_tests.rs` is at 1169 of 1200 lines; the next test splits it (the ghost and icon tests to a sibling). **Owed readings (the plan's Budgets table).** The live queue's settle after a runtime castle (two 1500-frame headless runs never drained the queue, with or without the castle, `docs/plans/logs/0083.md` Phase 8: a longer run or a quiet queue is needed), `VmHWM` at `--frames 3000` with and without `--castle` (the prefab bytes fell from 343 MB to 45 MB; the RSS was not re-read), the first frame with `--vox chapel` against the plain cave over three runs, the material term of `GpuWorld::mirror_bytes` with the chapel's 255 rows, and `--census` for `Wide` bricks with the castle placed. Pick up with the next plan on structures (0084) or a readings chore.

### Plan 0094's leftovers (close review, 2026-11-20)

Found at the close ([Plan 0094](plans/done/0094-light-transitions-the-key-means-age-the-no-ray-prior-and-its-fades-the-hole-band-and-the-castle-fill-in.md), v0.68.0); none blocked it, each is a small fix when the area is next open. **A tautology:** `the_tunnel_exposes_for_its_walls_by_day` (`apps/flythrough/tests/gpu_outdoor/ceiling.rs`, soak) asserts the noon-midnight luma clause only inside `if ratio.abs() <= 0.5`, so it can never fail; it reads +0.063 EV since Phase 8a, so a plain `assert!` is the fix. **A bar that does not see its mechanism:** `a_rare_hit_is_clamped` (`crates/vxe-render/tests/gpu_sun/history.rs`, soak) passes with the clamp off too (largest 0.1996 both ways); a clause on what the clamp changes, or a rig with more than one hit, would defend it. **Unread costs:** the no-record prior's arm A on the RENOIR at 1080p's half tier during a stamp's bake (a linear guess of 14.6 ms, `docs/nfr/02-frame-budget.md`; Plan 0096's RENOIR readings can take it) and the no-ray fade's temporal and composite cost.

### Hundreds or thousands of entities

Parked 2026-11-16 by the owner ("minecraft can have thousands of entities"). Today: `MAX_ENTITIES` 64, `MAX_ENTITY_PARTS` 512 (`vxe-render` `entities.rs`), 6 instance bits in the hit record. The cost is the trace, not the caps: Plan 0062 Phase 4 (`docs/plans/logs/0062.md`) read 64 walkers at +3.4 to 4.4 ms on the RTX 3080 Laptop and +32 to 34 ms on the Radeon iGPU, growing with screen cover and shadowing lights. **Hundreds:** wider caps and hit bits, the instance tree built on the GPU, coarse far models, shadow rays testing only entities near their light (one or two plans, no ADR). **Thousands:** entities rasterized into the hit buffer beside the traced world, which reverses ADR-0050's choice: an ADR and a spike first. **The owner's direction (2026-11-16): trace only the near ones, merge the crowded ones.** Distance tiers: near entities traced in full; mid ones traced on a coarse merged model (parts folded into one volume at detail 1); far ones as one box or impostor, or dropped below a pixel. Clusters: entities sharing a spot re-voxelized together at low resolution on the GPU each frame (or every few) into one cluster volume, walked once per ray instead of once per overlapping entity. Shadow rays test the near tier only. The server's ray query stays exact per entity; the tiers are render-only. Pick up when a consumer needs more than 64 on screen. Sharpened 2026-11-19 (Bonsai's 1.1.0 and 1.3.10 notes): the CPU side needs a per-chunk entity list as the broadphase (the mover's sweeps and the entity ray query walk the chunk's list, not every instance) and an LRU eviction of models from the pool when the library outgrows it, on top of Plan 0078's free list.

### A timeline for clips, kept open

Parked 2026-11-16 at Plan 0063's window. (b): typed keys are enough "for now, but keep it open": a timeline or curve view stays out of the engine (ADR-0051, Plan 0082's not-do list); if typed keys fail on a longer clip, ADR-0051's Alternative A (glTF from Blender) is re-read first.

### Jelly entities: see-through glowing voxels, and true soft bodies

Parked 2026-11-16 by the owner's question "jelly-like entities, slimes - will it be possible?". The part taken: Plan 0082, a scale per axis per part in the pose and the clips (squash and stretch; ADR-0064). Two parts left. **See-through glowing jelly:** an entity voxel is an opaque honest cube (Plan 0062 draws a translucent material on an entity opaque); it waits for Plan 0034's translucent voxels and then an amendment that runs the entity walk inside the translucent march. **True soft bodies:** a blob that bends round a corner, splits or merges needs the voxels themselves to move (a lattice or field over the part's volume, sampled by the ray), which the pool (one volume per model), the regular grid walk and the query's determinism all rest against; ADR-0064's Alternative C names it. What it takes: an ADR and a spike that reads the walk's cost under a deformation field on both adapters. Pick up when a consumer asks for a creature that bends, splits, merges or is seen through.

### Per-entity voxel damage: real holes carved into one instance's parts

Parked 2026-11-16 at Plan 0085's draft (the owner's "healthy, damaged, dead" question). Plan 0085 gives an entity modes without touching its voxels: a variant hides a part, swaps its volume for another `.vox` node or remaps a material, and a dead entity is stamped into the world. What it does not give is a bite taken out of one spider's abdomen where the pick struck. **What it takes:** copy-on-write volumes per instance (an edit on an entity's part copies the shared volume into the pool, under ADR-0060's allocator, and the instance's volume word names the copy), a per-instance pool budget and an eviction rule, an edit op on an entity (shape in the part's frame, carried through the composed pose in fixed point, so the server's query and the picture agree) that reports what it removed per material as a world edit does, the op in the log (an entity op kind, which needs the model and pose in the save, ADR-0069's Alternative A), and a rule for the headless query's volumes (a per-entity `Arc<Prefab>` copy beside the model's). About a plan and an ADR. **Pick up when** a consumer asks for damage that shows where it landed, not which state it reached.

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

Parked 2026-11-05 at Plan 0057's close; taken by Plan 0058's Phase 3 (the furnace test's independent `D` and the lamp-step test over more than one probe; the inside-probe trace is a reading there).

### Far light on a slow adapter: a bake budget per second, and no black for missing light

Read 2026-11-11 in Plan 0060's Phase 4 window (`docs/plans/logs/0060.md`): the far caves stay coarse on the RTX and black on the Radeon, because the bake's slice is per frame and a brick with no light value renders black. Taken by [Plan 0067](plans/done/0067-the-frames-cpu-a-cheap-queue-step-a-bake-budget-by-time-and-no-black-for-missing-light.md) (approved 2026-11-11) and ADR-0052 and ADR-0053.

### A CPU-drawn first frame before the device

Parked 2026-11-16 by the owner at Plan 0075's Phase 6b (`docs/plans/logs/0075.md`). Since 6b the window shows a static loading frame 0.3 to 0.4 s after process start on the RTX (0.2 to 0.3 s on the Radeon iGPU), drawn by the GPU right after the device. What is left before it is the instance (78 to 244 ms), the device and the surface's configure. In two runs, the first launch after a pause, the instance took about 1.6 s (a guess: the NVIDIA dGPU waking; not confirmed). On Wayland nothing shows until a buffer is attached, so a frame before the device needs a CPU-filled buffer: the `softbuffer` crate (rust-windowing, winit's org; wl_shm on Linux, GDI on Windows), or the same by hand per platform. Windows already paints a window's class background at once. Cost: a dependency and its ADR, about 100 lines in `vxe-app`'s shell, and the window's surface handed from the CPU buffer to wgpu's swapchain, read on both platforms. What it takes: a reading of how often the slow instance happens on testers' machines (the `start-up:` lines in run logs), then a small plan. Pick up if a tester reports a blank window at start.

### The loading frames stutter on the streamed worlds

Noted 2026-11-16 at Plan 0075's close (its Followups; `docs/plans/logs/0075.md`, Phases 5 and 6).
On the cave and the castle the loading frames run near 24 fps (interval p95 96 to 104 ms on the
RTX); the owner saw it ("like fps in 30-40") and accepted it. A pre-roll bake step runs 70 to 140
ms against the 25 ms slice, and each loading frame waits for the previous slice's GPU work. What
it takes: a bake step split under the slice (fewer samples a step, or a step bounded by time),
read on the castle. Pick up with the next start-up plan, or when a tester reports a jerky load.

### Rebuild the world live behind the loading screen

Parked 2026-11-15 by Plan 0075 ("What this plan does NOT do"). A settings change of Plan 0069's
`Rebuild` class could drop to the loading screen, rebuild, and step back in, instead of a restart.
What it takes: the run's stages re-entered from a live run, the world and mirror torn down in
place. Pick up after Plan 0069 lands, if a rebuild-class setting proves common. Sharpened
2026-11-19: it needs a drop-and-recreate test first, headless: build, drop, build again, and
assert the mirror's VRAM reservation and the process's RSS return to the first build's figures
(within a stated slack); a leak there is what makes a live rebuild worse than a restart.

### The pre-roll never settles at small residencies

Seen 2026-11-15 by Plan 0075's lane while sizing its streamed test (`docs/plans/logs/0075.md`): with `--residency 2,3` or `4,6` and `--near 0`, the start-up pre-roll runs to its 120 s cap, with the loading screen on or off; the camera's chunk never reports its light settled. Not read further. A guess, not isolated: the pre-roll's "near chunks hold their light" condition waits on a chunk the small window never makes resident, or keeps evicting and reloading. What it takes: a headless repro at main with the pre-roll's wait condition logged per chunk, then a fix in that condition. Pick up before a consumer runs small residencies.

### A dug chunk's settled light differs from a one-shot bake

Read 2026-11-10 in Plan 0060 Phase 3 (`docs/plans/logs/0060.md`, `a_dig_is_never_withdrawn`). In that test's rig, after the dig settles, chunk [2, 0, 0] differs from a one-shot bake of the edited world. Chunks 0, 1 and 3 to 7 match, and so does the dug chunk the test now compares. The difference reads the same under the score order with withdrawal on, under `distance` with withdrawal off, and at the default `edit_ms` (the lane's `live/p3-digexp.log` and `p3-digexp2.log`, untracked). So it is not the withdrawal and not the order. It predates Plan 0060 by that reading, but it was not read at the base. A guess, not isolated: a neighbour chunk whose light reads the dug chunk is not re-baked after the dig, or is re-baked before the dig's own job lands. What it takes: a headless test that digs, settles and compares every chunk with a one-shot bake, read at main first, then a fix in the live queue's dirty-neighbour rule. Added at the owner's call (2026-11-11).

### The near shell's cost in a flight, and the streaming edge

Parked 2026-11-10 at Plan 0060's Phase 1 stop rules, owed to the backlog at its close (owner's amendment). In the streamed cave at `--glide 300` the near shell (Chebyshev 2, baked at the full count) takes 98 to 99 % of the slice, and about 0.70 of the in-view chunks still waiting have their reach not ready (the mirror's edge), so neither the order nor the withdrawal can light them. What it would take: a shell baked coarse first in a flight, or a smaller shell at speed; and a streamer that loads the light radius ahead of the view. Pick up with Plan 0067's bake budget by time, which reads the same flight (`docs/plans/logs/0060.md`, Phase 1).

### The queue step's remaining `filter` cost on a moving camera

Taken 2026-11-20 (draft, awaits the go) by [Plan 0096](plans/0096-the-bake-queues-drain-tiers-a-drain-slice-the-step-and-the-slow-adapters-light-pass.md) Phase 5 (one walk of the mirror's change list for `gone`, `drop_reach`, `touched` and `forget`; the change count printed in Phase 1).

### The RTX's submits draw less than their per-second slice

Taken 2026-11-20 (draft, awaits the go) by [Plan 0096](plans/0096-the-bake-queues-drain-tiers-a-drain-slice-the-step-and-the-slow-adapters-light-pass.md) Phase 4: a lap on `plan_rounds`, `send` and the `IN_FLIGHT` wait, then the fix it names; the bar is 85 % of the granted slice drawn.

### The first-block fallback fade's CPU under `chain`

Parked 2026-11-12 at Plan 0067's close review (`docs/plans/logs/0067.md`, Phase 7, the opening row). Under the default missing light, `chain`, every chunk's first block fades over `FADE` frames from its fallback value, and the render thread rewrites the fading block (`advance_fades`) and its summary and two parent records (`Store::show`) every frame of the fade. In regime B, where nearly every step opens, that reads `fades` 1.05 to 1.10 ms p50 and `summaries` 0.61 to 0.63 on the RTX 3080 Laptop (`--near 0`, rig (c)), against 0.002 and 0.05 under `black`, and takes the opening step from 0.75 to 0.86 to 0.88 of the start commit's (bar 0.8); the default glide's `summaries` read 0.965 ms p50, p99 2.86. The owner saw no lag at this cost in the window. Levers, unbuilt: a shorter fade for a first block (a look change, the owner's eye), the summary and parents written once at the fade's end instead of every frame (ADR-0053 has a parent fade with its children, so this one needs an Outcome there), or the blend on the GPU (the temporal read mixes the fallback with the block by a per-chunk weight). Pick up with the next frame-CPU plan, or if a profile (Plan 0069) runs regime B by default (`full_first` off).

### The bake and strip examples drift from the fly-through's lighting defaults

Parked 2026-11-13 (orchestrator default while the owner was away). The hero fix (5b35aa3) gave `examples/hero.rs` the shared `flythrough::default_lighting()`. Two tools still build the library's `Lighting` and so miss the Plan 0061 store-read and light-weight defaults: `apps/flythrough/examples/bake.rs` and `apps/flythrough/examples/strip.rs`. Their readings and stills differ from what the window shows. The fix is to call `default_lighting()` and re-read anything that cites their figures. Pick it up when a plan next reads a strip or a bake figure, or when the owner asks.

### Overlapping coloured lights go white: a hue-keeping tone map

Parked 2026-11-15 from Plan 0058's re-window (step 1b, the plan-0058 worktree's `live/10.png`; orchestrator default, the owner said "not sure colours mix well" and passed (k)). Two dropped lights at 600 overlap as plain white, with cyan only on the green and blue edge: auto exposure sits at its +4 EV ceiling (the scene reads -3.3 EV), so every channel clips. The lights add correctly; the tone map's per-channel clip loses the hue. What it takes: a tone map that desaturates toward white only past a luminance shoulder (hue kept), or a lower light intensity; either moves goldens and is the owner's eye. Pick up at the render-settings profiles (Plan 0069) or the next look plan. **Extended 2026-11-20** by the review [`reviews/2026-11-20-crystal-islands-look.md`](reviews/2026-11-20-crystal-islands-look.md) (trait 2): the Crystal Islands frames clip the same way (saturated bodies, white tips, 1:00's giant crystals flat white), so the per-channel curve is not what separates the looks; this entry is a refinement ranked second of that review's engine levers, after the per-voxel variation, and the content pass (dimmer, more varied emission) comes first.

### The hall's headless render went dark: `showcase_hall_golden_is_lit_by_its_four_ores`

Read 2026-11-11 at Plan 0060's close review: the soak test failed with 98.6927 % of pixels over 2/255, the same on the base a8b1a2c. Re-blessed 2026-11-11 at Plan 0061's close (fix pass 6287e88): the new golden pins today's render, which Plan 0061's defaults do not move (the library's read and weights give the same pixels), but it is not a correct hall. Mean sRGB luma fell from 11.97 (Plan 0057's image, 2de29c6) to 2.73; the walls are black and only the ores' faces are lit; ember covers 0 pixels. The test still fails its hue assert (`[336, 0, 209, 121]`) and would fail its luma band (11..40). The owner's window at Plan 0061 saw a lit hall, so the likely cause is the headless rig (`gpu_golden.rs`'s `showcase_hall`: its bake or settle state under the baked light since Plan 0059), not the look; unverified. What it takes: bisect the rig's render between 2de29c6 and today, fix the rig, re-bless after a look, and keep the hue and luma asserts as they are. The weekly soak fails on it until then. **Done 2026-11-16 (chore `chore-showcase-golden`, b09e018):** the rig, not the look. `showcase_hall` synced the mirror and stepped frames but never baked the light store, which the fly-through's pre-roll and coarse pass fill before its first frame; since Plan 0059 the cube faces read only that store, so unbaked walls read no light and only the ores' own emission showed. The rig now bakes the whole showcase world (64 samples a brick direction, as the dark cave and castle goldens do) before its 64 frames. On lavapipe: mean sRGB luma 14.4/255, hue shares amber 4.31 %, ember 0.26 %, cyan 4.55 %, violet 0.36 %, 0 pixels over 2/255 with the emission off; every assert as written passes. `showcase_hall.png` re-blessed; it matches Plan 0057's image (2de29c6) by eye.

### Automatic extremity bounds from declarative noise layers

Parked 2026-11-12 (owner, from the video "500 million voxels/sec - Voxel engine optimizations", youtube.com/watch?v=ru_oz09Zo-s). Today world generation is the consumer's: `WorldSource` is a trait, and the flythrough's cave (`apps/flythrough/src/cave.rs`) already aligns its noise lattices with bricks (one corner set per brick) and proves a brick uniform from its eight corners. The idea the engine lacks: when terrain is described as data (named noise layers, each with a range, mapping, scale and upsample factor), the generator derives each column's or brick's guaranteed solid-below and air-above bounds from the layer config itself, so a new biome skips noise evaluation outside its band with no hand-written proof; layers whose range is unbounded (domain warps) switch the bound off. Same video, also not taken: noise shared by name across biomes and computed once per position. Rejected for storage: RLE columns (slow random access and edits, which collision, acoustics and runtime digging need). Pick up only if the engine ever ships a configurable terrain generator; the requirement would arrive through `project-context.md`.

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

### Plan 0090 owes a stamp scene and a reading of the key mean's fade-in against `F`

Parked 2026-11-18 by Plan 0086's flicker amendment (the architect, overnight). A freshly
placed castle on the hills at noon flickered in the Plan 0083 window (25.png): the stamped
chunks baked behind the carve's dig ring and every reset pixel on the new walls ran the sky
ray's mean from one raw sample (`docs/plans/logs/0086.md`, the flicker section). Plan 0086
Phase 21 (a) and Phase 21a take the fix; what is owed on main is (1) a scene in Plan 0090's
table, a prefab placed on the hills at noon with the eye inside it, read under rules 3, 4 and
5, so the class cannot return unseen; and (2) a reading of the key light's mean (`key_mean`,
Plan 0086 Phase 19) at an edit-cast shadow outside its edit box, which fades in over up to
`sky_history` frames, against the `F` the owner calibrates in Plan 0090 Phase 5; if `F` is
shorter, the key mean takes a band test as `pair_step` has for the sky. An architect amendment
to Plan 0090, at Plan 0086's close.

### A consumer's settings file in the render example

Parked 2026-11-12 (owner, at the settings-menu interview). The fly-through and the tester keep render settings in a TOML file through an app-side field table ([Plan 0073](plans/0073-a-settings-menu-and-file-for-the-fly-through-and-the-tester.md), ADR-0058); the engine stays file-free (ADR-0055). A later example could show a consumer doing the same, so a game sees how. The natural home is [Plan 0071](plans/0071-the-consumers-path-a-render-example-a-renderer-tested-alone-names-and-errors.md)'s render example, or a second example beside it. Pick up after Plan 0073 closes, if 0071 has not taken it by then.

### A freshly placed castle's interior under the sky, as a pop-counter scene

Parked 2026-11-18 from Plan 0083's Phase 7 verdict (session 83b, `--source outdoor --time
12:00`, 25.png): inside a castle placed from the belt a moment before, the walls clipped white
and blue and the vault was dark and grainy with sparkles ("castle flickers badly"; HUD queue
depth 10 751, missing 0, scene -4.1 EV, eye +1.34 EV). A freshly stamped large structure under
a sky is the sky ray's region at rest on a young history with the exposure moving on it: what
ADR-0076's rules 1 to 3 forbid. Plan 0083 does not fix it. Pick up in Plan 0090 Phase 4 (the
other stills) or Plan 0086's close: a fixed scene that stamps the castle's hall on the hills
and counts the pixels that break rules 1 to 3 over its settle, by source.

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

### Light shafts inside structures: visible sun beams through openings

Noted 2026-11-20 by the owner at Plan 0094's window 94a, inside the castle's keep: "it would be
nice in the future rays of light inside such structures". Visible beams where sunlight enters an
interior through windows, doors or breaches, with the air lit along the ray. It belongs with the
volumetrics (Plan 0035's froxel or march work; ADR-0030 names the cloud shell): a low-resolution
in-scattering march along the view ray that samples the key light's shadow, reduced and
temporally stable (the owner's stable-image rule; no grain while it converges). Cost is a guess
until measured on the slow adapter against the headroom line in `docs/nfr.md` section 2.

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
per-material style word), not on the material row. Size S; a look spike with an owner window, after the content pass below shows
what the palette alone gives.

### The showcase's palette after the Crystal Islands review: pale rock, more and dimmer glowing rows, a darker key

Parked 2026-11-20 from the same review (its content levers), apps only. The frames' cave look
is albedo times the emitters' irradiance, which the baked store already computes; our rock is
`0.42 0.38 0.34` against Lin's near-white limestone, so the walls cannot take the crystals'
colour. **What it takes:** in `apps/flythrough/assets/materials.txt` (re-read live while the
window is open) a rock at about 0.7 to 0.8 with a tan or pink tint, six to eight glowing rows
at a third to a half of today's emission with the ore among them, and a cave profile one to
two EV darker (`AutoExposure::compensation_ev`); two or three leaf rows picked by hash in
`trees.rs`. Judged in an owner window; the showcase's goldens move. Size XS. **Pick up** when
the owner wants a look window next; this is the cheapest test of the review's thesis and
should run before any engine lever above.

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
- **The bake's slice and the queue's step have no line.** Taken 2026-11-20 by Plan 0096 (draft): a drain and a landed value for the slice, a step line, an in-view drain budget. 1.5 ms of GPU a frame while the queue
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
Sharpened 2026-11-19: Plan 0091's budget scopes (a scope that carries its `nfr.md` line and
counts its overruns into `run.json`) make each line above executable; a line with no owner
then at least has a counter, and the tester's report says how often it was crossed.

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
- **`frame/encode.rs` is at 1190 of the 1200-line cap.** The next pass that adds to the encode
  splits it first.
- **The committed `castle_gate.png`, `look_gradient27.png` and `look_trilinear8.png`** differ by
  a few pixels from the dev box's lavapipe (within the 0.1 % tolerance), at the plan's base and
  tip alike; CI's lavapipe blesses them. Re-bless on CI if one ever drifts past the tolerance.
- **The soak test `a_shown_hud_and_the_queue_line_wait_on_the_gpu_nowhere` failed, unrelated to
  Plan 0044 (fixed 2026-11-16 on branch chore-soak-frames: the test now matches `frames: {FRAMES} frames;`,
  accepts frame 0's `store reads not counted` queue line, and reads the age as the last item of
  the line's parenthesis; it passes in 8.9 s on lavapipe).** Its `find(|l| l.starts_with("frames: "))` takes the queue step's tally line
  (`frames: queue step A: ...`, printed first since 87bf0d2, Plan 0067 Phase 1) instead of the run's
  `frames: N frames; ...` line. Fix: match `frames: {FRAMES} frames;` in the test. Read at the
  close review's soak run (whole workspace, 73 run, 71 passed; the other failure is the hall
  entry above).

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
worktree and leave with it.

### The sync's `light_note` stage and the windowed glide's light walks

Taken 2026-11-13 by [Plan 0074](plans/done/0074-the-syncs-light-stages-placement-spikes-attributed-and-exposed-emitters-by-masks.md) (closed 2026-11-14): `light_note` cut by masks; the placement's spikes are "The light placement's eviction scan in a windowed glide" under Next up.

### The tester cave's coarse light is slow to settle

Noted 2026-11-14 in Plan 0074's window (the owner, on the tester's cave path, the tip and the
base the same): "coarse light are slow to settle". Not Plan 0074's stages (its tip and base
differ only in the sync's light note and read the same). A reading of the bake queue's coarse
pass and its settle on a moving path; no run read it. Pick up with the next look or bake-queue
plan that walks the tester's paths.

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
**It failed** (fixed 2026-11-16 on branch chore-soak-frames: the test matches the run's line, accepts frame
0's `store reads not counted` queue line, which prints before the frame turns the count on, and
reads the age as the last item of the parenthesis, which now names the missing light and store
bytes first) (read 2026-11-14 at Plan 0074's close review, soak tier of the changed crates,
not this plan's code): the exit's queue tally prints `frames: queue step ...` lines
(`run/mod.rs`, `Drop for Run`) before the `frames: N frames;` line, and the test takes the
first line starting `frames: `. Match `frames: {FRAMES} frames;` instead.

## Tooling and process

### A bugs journal: date, hours lost, tags, the lesson

Raised 2026-11-19 (Bonsai's `bugs.md`: one heading per bug with the date, the hours it cost,
whether it is closed, tags such as `concurrency`, and a paragraph on what it was and what to
remember). Our defects live in plan logs and close reviews, spread over many files, so the
same class of mistake (a held lock, an unstable clock, a stale golden) is not counted. **What
it takes:** `docs/bugs.md` with that heading form, appended at every fix pass and close review
that names a defect; `scripts/velocity.mjs` can count its headings by tag later. Size S. Low
priority. **Pick up** when the owner wants the recurring classes named, or at a review that
finds the same defect twice.

### The 64 private-intra-doc-link warnings in `cargo doc -p vxe-render`

Noted 2026-11-17 from Plan 0069 Phase 2 (`docs/plans/logs/0069.md`): `cargo doc -p vxe-render
--no-deps` prints 64 `private_intra_doc_links` warnings (cargo's count), none on a line that plan
wrote; its done-when was amended to "no new warnings from this plan's files". Owner 2026-11-17: fix
them in a separate chore later. What it takes: make each linked item `pub` or `pub(crate)`-doc it
with a plain code span, then a CI step or `RUSTDOCFLAGS=-D warnings` doc build so the count cannot
grow again. Pick up after Plan 0069 merges, so the chore reads the settled `settings.rs`.

### The tester mode's key filter is structural, not tested in the frame loop

Owed 2026-11-15 from Plan 0068's close (its phase review's M2). Under `--tester` the frame loop
(`apps/flythrough/src/run/frame.rs`) skips `handle_input` and calls
`flythrough::path::tester_action` only for keys it already chose (`F1`, the left click), so the
unit test of `tester_action` cannot catch a debug key read outside `handle_input`, as `F2` is.
What it takes: route every key the frame loop reads under `--tester` through `tester_action`, or a
test that drives the loop's input handling with the filter on. Taken 2026-11-16 by
[Plan 0080](plans/done/0080-the-fly-throughs-menu-one-overlay-a-palette-and-the-minimal-key-set.md)
Phase 2 (a test drives the loop's input step with the filter on; `Tab` and `Start` map to
nothing); the entry leaves when that plan closes.

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

### Plan 0068's leftovers from the owner's runs

Noted 2026-11-15 at Plan 0068's close. Not seen in either release run: "run all benchmarks" from
the Linux archive (only a castle tour; the three-run zip was read twice from local release
builds) and a free flight in each scene (the Windows report holds the three benchmarks). The next
tester archive's own run covers both; no work unless it fails.

### The HUD paints a bounded level's full record reservation as mirror pressure

Read 2026-11-15 in Plan 0058's Phase 5f (`docs/plans/logs/0058.md`, the owner's `live/7.png` and `9.png`). On the level the HUD shows `mirror pressure 1.000` in yellow. `GpuWorld::pressure` (ADR-0013) is the larger of the pool-unit term and the record term. `scene::level_mirror_budget` reserves exactly one record per chunk of a bounded level, so once every chunk is mirrored the record term is 1.000 by construction (3200 of 3200; the unit term reads 0.30). No brick was refused and no radius narrowed. It read the same at the plan's start commit. The yellow at `hud::PRESSURE_HIGH` (0.9) therefore warns about a reservation that is full by design. What it takes: the HUD (or `pressure`) leaves out the record term when the reservation covers every chunk of a bounded world, or shows the two terms apart. It is a testbed-and-HUD change with a unit test. Pick up with the next HUD or mirror-budget change.

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

### Done: the tester's benchmarks with the lamp off

Done 2026-11-13 (4ea561f): a `--path` benchmark starts the lamp off unless `--lamp` is given, and `run.json` names it. **Bench readings of the cave and castle paths from before 4ea561f ran with the lamp on and are not compared with readings after it.**

### A `Uniform` emitter brick gives no light samples

Noted 2026-11-13 from the tree at `98e69b8` (Plan 0074's draft, its owner answer 5).
`note_lights` scans `Full` bricks only, so a brick made entirely of one emissive material
contributes no light samples, even on its faces toward air. Not seen on screen. Fixing it changes
the light, which Plan 0074's byte guard forbids. Pick up after Plan 0074 as a look check: place a
solid emissive block of 8 or more voxels a side in the cave and compare it with a hollow one.

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

### Plan 0072's CI readings: accepted, one reading left open

Parked 2026-11-13 at Plan 0072's close. Read 2026-11-13 on the first two pushes: warm Linux
15 min 6 s and Windows 18 min 47 s (over the under-10 and under-15 min proposals), gate count on
CI unchanged (844 passed). The owner accepted these walls the same day ("record this time as
accepted, let it be"); they are the CI line in `docs/nfr.md` section 7. Left open, no longer
owed: one manual `workflow_dispatch` run with `fingerprint` set on a prefix-restored cache, to
name why `proc-macro2` and the like recompile. Pick it up only if CI time becomes a complaint.

### The lavapipe group at two threads (owner question)

Parked 2026-11-13 at Plan 0072's close. At `max-threads = 2` the gate read 354.6 to 368.2 s
against 656 s at 1 (six runs, 830 of 830 passing, `docs/plans/logs/0072.md` Phase 4), but
`gpu_bake`'s `a_dug_emitter_leaves_the_near_ring_at_once` read 2.5 to 2.7 times its N = 1 time
in every run, so the plan's rule kept N at 1 (the close's default; the owner was asleep). The
owner's call: take N = 2 (nearly half the gate) and accept or soak-mark that test, or keep 1.
A one-line change in `.config/nextest.toml`; pick up on the owner's answer.

### `split-debuginfo` on Linux only (owner question)

Parked 2026-11-13 at Plan 0072's close. `[profile.dev] split-debuginfo = "unpacked"` cut the
`vxe-world` quick-tier rebuild further (12.2 to 12.5 s against 17.8 to 22.4 with line tables
alone; the target dir 7.6 GB against 12.2) but rustc rejects it on `x86_64-pc-windows-msvc` and
wasm32, and a profile cannot scope it to a target. The Linux-only form is `rustflags =
["-Csplit-debuginfo=unpacked"]` under `[target.x86_64-unknown-linux-gnu]` in a new
`.cargo/config.toml`; it changes every Linux build's flags (CI's too, unless scoped), so it is
the owner's call. Not added at the close (the default). Pick up on the owner's answer.

### A pipeline cache for the renderer's shaders

Parked 2026-11-12 (the architecture review, section 1.4). Every pipeline creates its own shader
module and (at the entry's date) 13 of them prepended the 2496-line `march.wgsl` (since Plan
0045 each pass composes only the modules its `PASSES` row names); no descriptor passes a
`wgpu::PipelineCache` (`cache: None` everywhere). Cold creation of the eleven pipelines read
1396 ms on the RTX, 458 ms on RADV, 299 ms on lavapipe (Plan 0046). wgpu 30's `PipelineCache`
is Vulkan-only and keyed by the driver, so it helps a second start on the same machine, not the
first; the driver's own disk cache already does part of that. What it takes: `pipeline_times`
read with and without a cache file across two starts, on both adapters, after Plan 0045's
Phase 4 has split the shaders (which changes what is compiled). Ready since 0045's close
(2026-11-13): `pipeline_times` reads `internals::shader_sources()`. Sharpened 2026-11-19: Plan
0092 (WGSL hot reload) rebuilds a pass's pipeline from a file on disk; a reloaded pipeline is a
new compile the cache has not seen, so the cache and the reload are read together when this is
picked up (the engine entry above is the same question from the start-up side). Read by Plan
0094 Phase 2a (2026-11-19, `docs/plans/logs/0094.md`): the test supports' `wgpu::PipelineCache`
pays 0.01 on the RTX against an empty driver cache (29.4 s cold, 0.24 s warm, 2.0 MB) and
nothing on lavapipe (0.97, 84 B: no compiled code), so Plan 0094 keeps it for hardware adapters
only. What that plan leaves here (owner, 2026-11-19): the light pass's default-on compile cost
on lavapipe, the fly-through's light-pass build stage 3.7 s against the base's 2.7 (debug) with
every default-off arm already out of the default pipelines, about 1.0 s a device of default-on
code, about 1.4 s a frame-building test of the gate; the next step is a per-module compile
reading (`pipeline_times` per `PASSES` row) to name the module that grew. **Routed to Plan
0096 (performance) 2026-11-20**, after Plan 0094's window 94b: `absorb`'s cold start on an
emptied driver cache read 70 s (the owner: "start is slow, 70 sec"; Plan 0094 Phase 7a's
table: the arms' light pipelines 39 to 44 s inside their loading stage after the base's 25 to
49 s). Plan 0096 takes two items: an engine `wgpu::PipelineCache` for hardware adapters in the
app (the test-support cache read 29.4 s cold against 0.24 s warm on the RTX, 2.0 MB), and the
default-off arms built in the background after the first world frame rather than inside the
loading stage (the runtime pick's hitch with them). This entry leaves when 0096's phase takes
them; the lavapipe compile-cost reading above stays here.

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
itself). (The `CLAUDE.md` part was fixed 2026-11-13.) Pick up with the next chore
lane; it touches no file a plan lane edits except `gpu_lights.rs` (Plan 0058's).

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

### Shader toolchain spike

Parked 2026-10-31. Compare naga's SPIR-V against `spirv-opt -O` on the hit shader (VGPRs,
milliseconds, both adapters), then try wgpu subgroups and `shader-f16` on the walk. Rust versus
C for the CPU side was asked and answered no (GPU about 9 of 10.6 ms, CPU about 1 ms). Run when
no lane holds the GPU. Sharpened 2026-11-19: Plan 0092's hot reload is this spike's loop made
short (edit the WGSL, see the frame and the pass's timestamp without a restart), so run the
spike after it lands; the spike's own question, a different compiler on the same WGSL, stays.

### A held fallback still moves on screen

Parked 2026-11-05 from the ruling on `the_vein_wall_patches_are_counted`. With the table's read off at the patch pose on 299b5c8, the accumulation is bit-identical over the window (0 pixels changed), yet the filter's output reads 90.95 patches a step and the composite 51.10. The lamp is off and the gather is not added, so the filter is the only part left that moves. By reading the code (not isolated): `temporal.wgsl` keeps blending each frame's noisy sample into the moments after the emissive part holds, and `atrous.wgsl` weights its taps by that variance. `a_still_fallback_pixel_holds_still` reads the accumulation, so it does not see this. Plan 0059 keeps the fallback for no-face hits, so glass and the field look may shimmer at rest. That patch-pose run makes every hit a fallback, so it is an extreme case; a translucent surface has not been read. What it takes: a spike that reads a translucent wall's composite at rest, then a fix that holds the moments with the mean, or passes a held part around the filter as the table's part does. Plan 0059's Phase 2 reads a glass pane at rest and its Phase 3 window looks at it (owner, 2026-11-05); pick up on that answer.

### Two process-spawning GPU tests skip silently without lavapipe

Found 2026-11-17 at Plan 0088's close review. `apps/flythrough/tests/gpu_path.rs`
(`a_path_run_writes_its_report`, soak) and `gpu_frame_waits.rs` print "skipped: no adapter" and
pass when the fly-through finds no adapter, without the `VXE_REQUIRE_SOFTWARE_ADAPTER=1` panic
that `gpu_loading.rs`'s first helper has. `soak.yml` and `ci.yml`'s Linux arm set the variable,
so a runner without lavapipe passes these tests green. What it takes: the same three-line check
in both, or one helper in `tests/support/` they call. A dev chore of minutes.

### The gate tier is over its wall-time budget

Read 2026-11-05 on 7cd77ad (`docs/nfr.md` section 7): 1313.4 s, about 22 minutes, loaded, against the 12 min proposal, with the two tests then over the 30 s line already on the soak list. `flythrough::settle` is the largest binary at 338 s. Ruled 2026-11-05 on the four tests over the line in that run: `the_tone_pass_reproduces_the_composites_bytes_on_every_golden_pose` split (the every-pose test on the soak list; the primary pose alone stays in the gate at 9.2 s), `castle_gate_golden` went back under with its warm-up removed (25.7 s cold, 7.4 s warm, alone), and `smoothed_normals_cut_the_normal_error_and_move_no_hit` and `a_face_hidden_from_every_eye_is_settled_before_it_is_seen` stay (23.9 s and 24.1 s alone). Those two move only if a gate run started on a quiet machine reads them over the line. Moving all four would save at most about 95 s of serial time, so this is not how the budget gets met. Still owed: either cut the slowest `settle` and `golden` tests or re-set the proposal. Much of `settle`'s time goes with the table at Plan 0059's deletion, so re-read then. `the_lamp_toggle_opens_the_light_window` read 98.0 s on CI's last red run, on an older tree, and reads 18.0 s here, so it stays in the gate; CI's next run shows whether it is still over the 60 s line. Pick up as a dev chore when the owner next wants the gate faster or CI green. Read 2026-11-12 (the architecture review, on `live/plan-0068/merge-gate.log`): 809 tests, 610 s; the 177 lavapipe tests sum to 610 s serial, the whole wall, the 632 CPU tests to 273 s beside them, so the lever is the lavapipe group's thread count or the GPU tests' pixels, not the CPU tests; [Plan 0072](plans/done/0072-development-speed-the-build-graph-a-narrowed-gate-and-one-compile-per-ci-profile.md) Phase 4 measures the group at 2 and 3 (drafted, awaiting go). Read 2026-11-09 at Plan 0059's close (`docs/nfr.md` section 7): 850.2 s with `settle` back in the gate and none over the 30 s line, still over the 12 min proposal; the re-read of the proposal stays owed. Read 2026-11-08 on CI's Linux arm, Mesa 26.2.6, the GPU tests in parallel: the same tests move 25 to 40 % between runs (`a_face_hidden_from_every_eye_is_settled_before_it_is_seen` 55.6 s on run 42470428595 and 77.6 s on 42477325818; `the_lamp_toggle_opens_the_light_window` 53.2 then 70.5; `fsr2_adds_little_flicker_on_a_still_vein` 49.8 then 62.8), with several more between 45 and 58 s, so moving tests one push at a time does not converge. Ruled 2026-11-08 (owner): CI's budget step warns per test over 60 s instead of failing the push, until the light work settles; a test that stays over moves to the soak list by hand. Still owed: the architect re-reads `docs/nfr.md` section 7's 60 s line (it says a test over the line moves to the soak list; CI no longer enforces it), at Plan 0059's deletion or when the owner next wants the gate faster. Also ruled 2026-11-08 (owner), from run 42477325818 (2972 s summed over 727 gate tests, 27 tests at 30 s or more holding 41 % of it): the whole `flythrough::settle` binary (34 tests, 746 s) moves to the soak list until Plan 0059's deletion phase decides the table, and CI's Windows arm runs every CPU test but only a GPU smoke set (the `ci-windows` profile: the shutdown and walk binaries and one golden), since WARP skips the image bars and runs a test 10 to 20 times slower than lavapipe. Both are tier moves, not deletions. **Read 2026-11-18 at Plan 0079's close** (`docs/nfr.md` section 7): 1208.2 s on 1174 tests, loaded (load 16.9 at the lock), over the 19 min line the owner set the same day; the plan's own tests add about 17 s outside the serial lavapipe group. A quiet re-read is owed before the line is judged again.

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
- **Plan 0059 closed 2026-11-09 on "baked":** the preload, its sweep and `shade_preload` are deleted, and with them `VXE_DIAG_PRELOAD` and `VXE_DIAG_SPLIT_FRAME0`; the `diag:` lines stay (`apps/flythrough/src/main.rs` then; `app.rs`'s `diag` since Plan 0045, whose headless runner dropped the start, device and per-frame ones for `headless:` lines). Next, the owner's call: one push that runs the GPU smoke set on WARP again. Clean, the `diag:` lines go and this entry closes; still hung, the hang was not the preload's.

### llvmpipe SIGSEGV on CI's Ubuntu since Plan 0057 Phase 7

Read 2026-11-06 from the same run. Three gate tests died with SIGSEGV on lavapipe: `flythrough::castle_golden castle_gate_golden` (57.5 s, 20.8 s on the run before), `flythrough::fsr2 a_sideways_step_keeps_fsr2s_history` (20.8 s) and `a_still_camera_has_no_disocclusion` (27.7 s). The driver was the same on both runs: `mesa-vulkan-drivers 26.0.8-1ubuntu0.3`, LLVM 21.1.8, image ubuntu-26.04. Every test in that run that rendered a shaded frame crashed. The GPU tests that passed render none (`frame_bytes_under_fsr2_swap_the_histories_for_fsr2s_inventory`) or render cutaway, where the fill and preload do not run. Memory is not it: each test peaks at 0.73 to 0.92 GB and four at once at 3.7 GB locally, against the runner's 16 GB. It does not reproduce on local lavapipe (Mesa 26.2.5, LLVM 23): the ci profile at four threads passed 729 of 729. A guess: a Mesa 26.0.8 llvmpipe crash on a Phase 7 shader, not concurrency. Read 2026-11-08 from CI run 42222147229 on 7c73b37, the GPU tests one at a time: `castle_gate_golden` still died with SIGSEGV at 60 s on the same driver, so concurrency is ruled out and the driver crash stands; the ci profile's `@global` line is back. The owner chose (2026-11-08) a chore lane: a newer Mesa on the runner first, a local Mesa 26.0.x bisect of the pass if that is not available. Read 2026-11-08 in that lane (Launchpad, the day's indexes): the kisak-mesa PPA publishes `26.2.6~kisak1~r` for resolute (26.04) amd64, with its libdrm `2.4.134-3~kisak~r`, and the runner's archive has 26.0.8-1ubuntu0.3 at most; oibaf's PPA publishes no mesa at all. kisak's resolute build links Ubuntu's `libllvm21`, so it would have moved the runner to Mesa 26.2.6 on the same LLVM 21.1.8, not to the local pair (Mesa 26.2.5, LLVM 23). Ruled 2026-11-08 (owner): no kisak-mesa PPA and no other package source whose version cannot be pinned; the owner does not trust packages that can't be pinned, and the PPA indexes its latest build only. The runner keeps its stock Mesa 26.0.8. The LLVM 21 suspicion is accepted, and the pass is to be found on a local build of Mesa 26.0.8 from the release tarball against LLVM 21. Read 2026-11-08 in that lane, on local builds under `~/build/mesa-26.0.8`: the tarball `mesa-26.0.8.tar.xz` from mesa.freedesktop.org, sha256 caf1c0061a68e88dfa74967a7e780c0e85d65b6c4e334cd69095a5dc54ad78bc as published in its release notes, signature good from Eric Engestrom's key 57551DE15B968F6341C248F68D8E31AFC32428A6; LLVM 21.1.8 from Arch's extra `llvm21` and `llvm21-libs` 21.1.8-1, signatures and sync-db sha256 checked, extracted rather than installed. Lavapipe only, meson 1.9.1 with `-Dbuildtype=debugoptimized -Db_ndebug=true -Dvulkan-drivers=swrast -Dgallium-drivers=llvmpipe -Dplatforms= -Dopengl=false -Dglx=disabled -Degl=disabled -Dgbm=disabled -Dgles1=disabled -Dgles2=disabled -Dllvm=enabled -Dshared-llvm=enabled -Dvideo-codecs= -Dvalgrind=disabled -Dlibunwind=disabled -Dbuild-tests=false`, 804 steps in 53 s at `ninja -j12` on the dev box. It reproduces: `castle_gate_golden` dies with SIGSEGV at 24.6 s (adapter `Mesa 26.0.8 (LLVM 21.1.8)`), and so do `a_sideways_step_keeps_fsr2s_history` and `a_still_camera_has_no_disocclusion`. The core's crashing thread is a llvmpipe compute worker in JIT code with no symbols (`mov (%rbx),%rax` with rbx 0, a null pointer loaded from the shader's stack frame); its registers hold the table's mask 0x1fffff, the last cell's byte offset 0x5ffffd0 and the blend header's dirty word at 0x6000004, so the shader was addressing the per-face table. A local dispatch trace in lavapipe (a patch kept beside the build, not in the repo) names the dispatch: with the preload on, the last dispatch before the crash is the indirect 8 x 8 one, `shade_preload` (cache_fill.wgsl, the light pass with the preload's fill and probes); with `--preload off` it is the plain light pass `shade` (light.wgsl). Both run `shade_px`. Not Phase 7 alone: a bisect of the fly-through's 3-frame headless run on this build passes on 9dbb933 (main, CI's last green) and on 2bf4568 (plan 0057 Phase 4 part 2), and crashes on their merge a337f76 and every later point read (dc2a470, ef8cf7d, ca2b0e9, 4de5c4e, 41cda0c). The merge's only change to march.wgsl is Params' 16-byte tail (`emitter_cap`, `dither`), so the crash follows the shader's build, not a single feature, which is what a compiler bug looks like. Not LLVM 21: the same Mesa 26.0.8 tarball built against Arch's LLVM 23.1.3 crashes the same test, and Mesa 26.2.6 (sha256 b9436b3e1a8be76fbd4823dc884f90596ab7c6ac5e1867a305e66ffccfb31a92, same signer, same options) built against the same LLVM 21.1.8 passes all three tests. The accepted LLVM 21 suspicion is refuted: the crash is Mesa 26.0.x's own, fixed by 26.2. The owner's options, no shader changed: CI builds the pinned 26.2.6 tarball against the runner's LLVM 21 (55 s here at 12 jobs; the runner has 4 cores, and the build could be cached by its hash), or waits for the runner's image to ship Mesa 26.2. Ruled 2026-11-08 (owner): (a). CI's Linux arm builds lavapipe from the 26.2.6 release tarball, checked by its sha256 (no signature check, which would need an unpinned keyserver fetch), against the runner's own `llvm-21-dev`, with the options above and meson 1.9.1, ninja 1.13.0, mako 1.3.10, packaging 25.0 and pyyaml 6.0.3 from PyPI. It caches the install by the hash, the options and the runner image, points `VK_DRIVER_FILES` and `VK_ICD_FILENAMES` at it, and fails the job unless vulkaninfo reports Mesa 26.2.6. The workflow's exact steps, run in an `ubuntu:26.04` container capped at 4 CPUs, built it in 102 s and the loader reported `Mesa 26.2.6 (LLVM 21.1.8)`. The build comes out once the runner image ships Mesa 26.2 or later. Read 2026-11-09 from Soak run 42602069520 on 8092b6c, the first weekly soak: `soak.yml` still used the runner's Mesa 26.0.8, and 67 of its 79 tests died with SIGSEGV. The build moved into a composite action, `.github/actions/lavapipe`, which both workflows call, under the same cache key.

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

### The built-in walker and its scripted circle go with the library in the tester archive

Parked 2026-11-18 at Plan 0079's close (its Decision 5). `apps/flythrough/src/entities/walker.rs`
(the four-part walker made in code) and `Walkers::walk_pose` (its `f32` circle under the `wander`
toggle) exist only for a run with no entities library: `scripts/package-tester.sh` ships none, so
the tester archive falls back to them. **What it takes:** the archive packs
`apps/flythrough/assets/entities/`, `library::builtin` and the circle go, the `with_walker` table
rows and the tests that place the built-in walker move to the library's walker. A chore, no ADR.
**Pick up** with the next tester release that ships the library.

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

### Edit coalescing per chunk per tick: a check first

Raised 2026-11-19 (Bonsai's 1.6.1 notes, per the research pass: several edits on one chunk in a
tick give one re-mesh). Ours should already coalesce: `sync` visits the world's dirty-chunk set
(`vxe-render` `gpu_world/mod.rs`), a set, so two ops on one chunk between syncs should give one
upload. Not asserted anywhere read. **What it takes:** a headless test that applies two carves
on one chunk, syncs once, and reads one chunk upload from the sync's counters; if it reads two,
the fix is in the dirty set's handling. Size S. **Pick up** at the next sync or edit-cost plan.

### Allocation counting per frame in the apps

Raised 2026-11-19 (Bonsai counts every arena push into per-thread tables and prints
`Allocations Pushes` on its status bar). Taken by Plan 0091 Phase 6 (proposed: a counting
`#[global_allocator]` wrapper in the apps under its feature, the count on the HUD and in
`run.json`); this line stays until that plan is approved, then leaves.

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

## Later apps

### Web demo as a separate app

Decided 2026-10-16: a WebGPU and wasm demo for blog interactivity is a later, for-fun item and
its own app under `apps/`. The fly-through and the native paths never change or lose performance
for it; any crate support is target-gated. Not before the engine has steady lighting, coloured
materials, a sky and a way to build interesting architecture.

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

### The fired lights' grain: the list lights' visibility on a running mean

Parked 2026-11-18 by Plan 0086 Phase 22 (the second window's defect 3: "when I shoot light,
walls becoming grainy", 20.png, the noon tunnel under anchor 0). Phase 16 read it as Plan
0058's, not the sky's (`docs/plans/logs/0086.md`, Phase 16's table and Phase 22): with five
lights fired the lit patch's p99 change is 25 to 28 levels in flight and 5 to 6 at rest, the
same with the sky off; no sky ray returns over the lights; the light pass's rise (0.13 to 4.6
ms, loaded) is the five lights' shadow rays. In flight the list changes every frame and
`LIGHTING_LIGHTS_CHANGED` takes the direct part's sample as it is. What it would take: the
list lights' shadow visibility on a running mean of its own, as the key light's is on the
visibility pair's `g` (Plan 0086 Phase 19), reprojected and reset where the light moves. Pick
up when the third window (Plan 0086 Phase 24 run 3) judges the walls under the default anchor
4 still grainy, or a consumer fires lights at walls in the dark.

### Settle early: a coarse area answer where an estimate is not still, and a hold once it is

Parked 2026-11-19 (owner, at Plan 0094's Phase 1 and 2 readings: "light that is not still can be
approximated around area and settle exit early; we don't need super precision if it becomes
flicker", and "this exit early strategy can be used a lot around similar cases"). The principle:
when an estimate is young or restarting, show a coarse answer pooled over its neighbourhood and
narrow it as it ages; once its samples sit inside its band, hold it and move again only through
one fade when the band is left. Plan 0094 takes the two forms for the visibility pair (Phase 3's
settle on both means, Phase 6's age-weighted pool), and its window judges them. **A candidate
rule for ADR-0076** ("an estimator at rest holds by construction, not by its cap"; the rejected
alternative is a filter at a converged age), to be written at Plan 0094 Phase 7's verdict.
Where else it might apply, each unread: **the exposure** (ADR-0033's two-stage already has a
dead band in rule 5's still clause; a hold once the meter is within 1/64 EV); **the probes or
lamp channel** (hold a probe whose samples sit in its band, Plan 0051's lamp channel grains at
rest); **the denoiser's footprint** (the a-trous wide while the variance is young, none once
settled, which is SVGF's intent and this plan's pool); **the fired lights' grain** (the entry
above: a running mean that settles instead of a per-frame sample); **the entity lighting**
(a part's coarse grain holds its light until the part moves); **the far field and LOD
transitions** (hold a tile's converged value across a mip switch, fade the switch); **the bake
queue's fresh chunks** (a landed chunk's summary as the coarse answer for its faces until the
pair converges, then hold). Pick up per place when its flicker is read; Plan 0090's counter
names the places.

**2026-11-19, Plan 0094 Phase 3: the settle on the pair left the plan by its stop rule**
(`docs/plans/logs/0094.md`, Phase 3's re-read). As built (`temporal.wgsl` `settle_at_cap`: the
band at the cap only, a hold after 16 in-band frames, a release after 4 consecutive frames out
of it into a 1/16 fade; `Lighting::settle`, `--settle on|off`, default off): at rest it holds
93 % of the forest's pixels and the ages match the settle off, but the trunks' pops at rest do
not fall (0.352 % on, 0.367 % off at `10,180`) and the turn's return frame reads 37.6 % over 8
on against 21.9 % off; the clock at 1x releases 40.6 pixels a frame with 1.04 % of them a
`Flash`. Unread: why the return frame doubles (the trace the amendment names, the key mean's
age per frame at a returned penumbra pixel), and whether the pops at rest are the direct
part's own blend rather than the means'.

**2026-11-20, Plan 0094 Phase 7's verdict: the hold form is rejected, the principle becomes
ADR-0076 rule 6.** The owner's eye agreed with the readings (window 94a: settle on against off
"no significant difference" at rest, "3a looks better than 3b, especially in motion"); the
settle's code is removed by the owner's call (Plan 0094 Phase 7a) and ADR-0076's Alternative F
is its record. The age-weighted pool (Phase 6) lowered none of the hole band's shares and
raised Slow 0.4 to 2.4 %, and is off by default from Phase 7a. What the owner asked for at the
window ("if we have to wait for a minute for light to settle, something is wrong with the light
and we need approximate things") is written as rule 6: a coarse stable answer while the precise
one has not landed, one fade when it does, and every reading on the undrained image too. The
places listed above stay candidates; the first one the rule points at is the bake queue's drain
(the entry below, "The bake queue's backlog at a still camera").

### A second `Frame` on one rig does not drain its bake queue

Noted 2026-11-19 by Plan 0094 Phases 1 and 2 (`docs/plans/logs/0094.md`, bars 2 and 4 of
Phase 1, bar 1 of Phase 2): on the hills, the castle's arms (b) and (c) and the forest's turn
(twice on a fresh rig) the queue did not drain in 30 000 stepped frames, where a fresh process
drained the same scene in 1200 to 3300; the undrained arms read up to 16 % of pixels over 4 at
rest against under 1 % drained. Plan 0094 runs every arm in its own process. What it would take:
a reading of the queue's depth and the job at its head on the second frame (a job that never
lands, or a rig that re-marks what the first frame baked). A chore or the perf plan's; read
before any gate test shares a rig between two frames.

### The bake queue's backlog at a still camera: minutes to drain what is in view

Noted 2026-11-20 at Plan 0094's window 94a (`docs/plans/logs/0094.md`, Phase 7; the session
log's `bake queue:` lines): in the forest and on the hills the HUD read a queue depth of 12 752
to 16 150 jobs through every run, at a still camera after full residency (17 077 full chunks at
29.9 s), with `in view 3652 waiting (1030 coarse)`, `missing 244 (fallback 232, black 12)` at
the 15:00 still, and `baked` rising by 9 to 38 chunks a second at `budget frame:1.5`, which is 7
to 30 minutes to the drain. The owner's rule from that window (ADR-0076 rule 6): the image
meanwhile is the judged image. What it would take, in order: the drain's rate as a figure in
`docs/nfr.md` (jobs a second and the seconds until every chunk in view has a record, at a still
camera, on both adapters) with the HUD showing it; the in-view jobs ahead of the rest and the
budget sized to land them within seconds (ADR-0048's order already scores the view; the step's
6.3 ms on the main thread, Plan 0086 Phase 24c, is the other half); and the coarse pass as the
rule's "coarse answer" for what is not landed (it exists: `coarse 1479 chunks` in the same
line). The perf plan's first item; a related finding is "A second `Frame` on one rig does not
drain its bake queue" above.

### The dug hole's light: the first frames and the lit floor

Noted 2026-11-20 at Plan 0094's window 94a (runs 1 and 4: "dig still flashes, hole lit";
`live/window94a/4-dig-hole-lit.png`); the owner: "can even accept that", and it does not matter
in tunnels under the headlamp. Two halves. **The first frames** ("bright spots that settle for a
few frames"): Plan 0094 Phase 2 bar 2's thirds, the column's seed at the hole's new faces (open,
1.0, at `SEED_COLUMN` 2 until the dig's job lands, one frame on the RTX, two on the RENOIR) and
the key mean's restart on the new faces (the edit box's reset, ADR-0018); Phase 6's table
(`flicker::reading::hole_band`): flash plus dip plus overshoot 14.5 to 23.7 % of the hole band
in every arm, the pool included. Both are history mechanics; the motion lane that takes the
turn's wobble inherits them (a seed that carries the pre-dig value as rule 3's `old` across the
edit box's reset is the candidate rule 6 names). **The lit floor** (blue-grey under a shaded
hill, HUD `missing 0`, so the baked visibility): Plan 0094 Phase 7a (6) reads the stored
visibility against a brute-force aperture over the dug voxels and writes the word here:
`geometry` (a radius-0.5 m sphere's floor sees about half the sky and a grey face under a blue
sky reads blue beside grass; then the look is the hole's walls' bounce and a darker soil, a look
plan's) or `overstated` (a bake finding). **Read: `geometry`** (`gpu_sun`
`reading::the_dug_floors_baked_visibility_against_its_aperture`, lavapipe, the flat floor under
`Sky::uniform(1.0)` standing in for the hills): the 10-voxel box's record 0.184 against 0.209
over its cell's faces; the radius-5 sphere's record 0.718 against 0.748, but its record cell is
the 8-voxel brick, so the bowl's own faces (aperture 0.473) share one value with the open
ground beside them: at brick resolution a small hole's floor shows the brick's mean, not its
own sky. Pick up after the motion lane, or with the next dig look.

### The key term's grain at rest on a penumbra, and the a-trous's spatial variance (ii)

Noted 2026-11-20 at Plan 0094's window 94a, run 2 (the forest at 15:00: "still some grain" on
the penumbra edges of the trunks' shadows and the sun patch; queue depth 12 752, missing 244).
Plan 0094 removed the restarts at a still camera (Phase 3: rejections 416 to 0 a frame, the
trunks under age 64 4.67 to 0.17 %) and what remains at rest is the key light's own estimator
at its cap (`key_history` 256): the forest's trunks p99 23, mean 2.37 against the 1200-frame
reference at 512 frames; the hills' over-4 share at rest 0.154 % with the key light, 0.012 %
without (Phase 1 bar 2). A penumbra is where `g` is near one half and the variance largest.
The plan's (ii), SVGF's spatial variance estimate on the key term alone at a converged age in
the a-trous (ADR-0018; a guess of +0.05 ms RTX, +0.4 ms RENOIR), was not built: its clause (the
owner's word on run 1) did not fire, run 2's grain has the missing records and the moving
exposure as the other leads, and a spatial filter on the key term interacts with the temporal
pass the motion lane owns. Pick up in that lane, with the reading first: the penumbra's
frame-to-frame p99 at rest split by the key term, the sky's young pixels and the exposure.

### How dug interiors should look: darker, less sky than the true aperture

Parked 2026-11-20 by the owner at Plan 0094's window 94a verdict ("the hole reads lit";
`live/window94a/1-forest-dig-lit-hole.png`, `4-dig-hole-lit.png`: a radius-0.5 m hole's floor
blue-grey under a shaded hill and under a canopy, its walls dark brown). Plan 0094 Phase 7a (6)
reads whether the baked visibility matches the hole's true aperture; the owner's call is that
**even a correct aperture is not the look**: a dug interior should read darker and take less sky
than the geometry gives, so a hole reads as a hole beside the ground it was dug from. A look
plan, later, after the motion lane: candidates are the hole's walls' bounce in the baked term
(the walls' own low value folded into the floor's, which the bake's bounce cell mean may already
carry too little of), a darker soil row at the material table (a consumer's palette call, not
the engine's), or an occlusion term at concave faces read from the column map or the coarse
grid. Read first on the 94a stills' scenes (the hills at 16:00, the forest at `10,180`) with
the true aperture printed beside the stored value; the pick is the owner's eye.
