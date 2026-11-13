# Architecture review, 2026-11-12: performance, maintainability and the consumer's API

> **Written:** overnight 2026-11-11 to 2026-11-12, on main at `56008fa` (v0.48.0), in the
> `docs-review` worktree. **Read-only:** no cargo, no GPU, no test run (Plan 0067's lane held the
> machine for quiet readings). Every figure below is a reading already in the tree (named by
> file) or a count made with `grep`, `wc` and `node scripts/{loc,velocity,test-times}.mjs`. A
> figure this review could not read says so and names the plan phase that reads it.
> **Asked for by the owner:** "performance, both development and render, maintainability and
> clear API".

The one-paragraph verdict: the engine's GPU frame is where the plans say it is (6.3 ms on the
RTX in every tester scene), but the **render thread's CPU is not**: the cave and castle paths run
at wall p95 36.6 and 58.1 ms with the GPU at 6.3 ms, and most of that is the mirror's `sync`, which
no approved plan owns (Plan 0067 owns the bake queue's step beside it, and has a stop rule that
the tester's readings already trip). On the development side the gate is a serial lavapipe
queue (177 GPU tests = 610 s = the whole gate), a fresh lane pays a 7.5 to 9.4 minute cold
build, and CI compiles the workspace in about six configurations. On maintainability Plan 0045
covers the right files; it misses three test files and a stale premise (the reservoir code it
calls gone is live). On the API, a consumer cannot find out how to render a frame without
reading the testbed's `main.rs`: `Frame` has 98 public methods, 52 of them used by nothing in
`crates/`, and the crate doc has no example. Three plan drafts follow (0070, 0071, 0072), the
rest are amendment notes and backlog entries.

How to read the table: **L** is leverage (1 highest), **owner** is where the fix belongs: an
existing plan (with an amendment note), a new plan drafted in this review, or the backlog.

| L | Finding | The number | Owner |
|---|---|---|---|
| 1 | The render thread's `sync` in flight is the frame's lag on the RTX | cave wall p95 36.6 ms, castle 58.1 ms, GPU 6.3 ms; `update + poll + sync` p99 39.9 / 40.7 ms | **new Plan 0070**, ADR-0056 |
| 2 | The gate is a serial lavapipe queue; a fresh lane's first build is the lane's biggest cost | 177 GPU tests sum to 610 s = the gate's wall; cold test build 7.5 to 9.4 min per lane | **new Plan 0072** |
| 3 | A consumer has no path to a frame; `Frame` is a 98-method object with dead ends | 98 pub fns, 52 unused in `crates/`, 10 unused anywhere; no crate example; 12 steps to one frame | **new Plan 0071**, ADR-0057 |
| 4 | The Radeon is GPU-bound on the hit pass, 7 to 8x the RTX where the light pass is 2x | 52.3 ms at the window: hit 26.2, FSR 2 18.5, a-trous 4.4 | Plan 0069 Phase 4 + backlog's walk-speed spike (note) |
| 5 | Plan 0045's premise and reach | reservoir code live at `light.wgsl:270-330`; `hud.rs` 2458 and `showcase.rs` 1570 named only conditionally; 3 test files 1636 to 1814 lines unnamed | Plan 0045 (amendment notes) |
| 6 | Settings with no setter and setters with no caller | 5 `Upscaler` fields set only in `Default`; 5 `Lighting`, 4 `BakeSettings`, 5 `LightParams` fields set only by tests; `Lighting::lamp_bounce` defaults differ between library and app | Plan 0069 (amendment notes) |
| 7 | CI compiles the workspace about six times per push | Windows 55 min cold / 17 warm; 10 compile-shaped steps | Plan 0072 Phase 5 |
| 8 | The testbed's reading harness lives in `tests/` behind environment variables | about 75 distinct `VXE_*` names, 40 in `gpu_strip.rs`; `VXE_STEADY` drives a shim for a flag the binary refuses | Plan 0045 Phase 8 (amendment) |
| 9 | Shader compile: every pass prepends `march.wgsl`, no pipeline cache | 2496 lines x 13 pipelines; 1396 ms cold on the RTX | Plan 0045 Phase 4; backlog (pipeline cache) |
| 10 | Names: one thing, several words | `GpuWorld` / `MirrorConfig` / `sync` / "upload"; `*Params`, `*Settings`, `*Config`, `*Tuning`, `Lighting`; `Host::raycast(Fixed3) -> Option` vs `World::raycast([i64; 3]) -> Result` | Plan 0071 Phase 4 |
| 11 | Comment and manifest drift | `world_budget.rs` 26 "contour" lines describing ADR-0011's removed layout; `Cargo.toml` descriptions "placeholder" for `vxe-world` and `vxe-render`; CLAUDE.md gives `vxe-render` entities it does not have | backlog (dev chore) |
| 12 | Velocity halved while plan size grew | 542 commits over the last 14 active days against 616 over 8; 19 plans closed against 36; 0.50x commits per active day | for the owner, no plan |

## 1. Performance

### 1.1 Render: the frame's CPU in flight (L1)

**Evidence.** Plan 0068's bench runs, release build of `12ffee5`, RTX 3080 Laptop, 2530 x 1378
window, FSR 2 at 1.5, one run per scene, exclusive perf lock
([`docs/plans/logs/0068.md`](../plans/logs/0068.md), Phase 2):

| scene | wall p50 / p95 / p99 ms | CPU p50 / p95 ms | GPU p50 / p95 ms | frames over 33 ms |
|---|---|---|---|---|
| cave (streamed) | 14.3 / 36.6 / 65.3 | 13.8 / 36.1 | 6.35 / 7.41 | 228 of 4098 |
| showcase (bounded) | 10.4 / 12.8 / 13.9 | 1.8 / 3.1 | 6.28 / 7.31 | 0 |
| castle (streamed, stamped) | 13.8 / 58.1 / 69.6 | 13.4 / 57.6 | 6.27 / 7.98 | 133 of 1657 |

The fly-through's exit line for the same runs: `update + poll + sync` p99 39.9 ms (cave) and
40.7 ms (castle), 0.023 ms (showcase). The same shape at 1280 x 720 in the launcher's run
(Phase 4): cave p95 34.0 ms, castle 55.7 ms, GPU 2.2 ms. So the two streamed scenes are
CPU-bound by 5 to 9x their GPU time, and the bounded scene is not.

Two CPU costs sit on the render thread in flight, and only one has an owner:

- **The bake queue's step**: 10.3 ms p50 in the fast cave flight, single steps to 56 ms
  (Plan 0067's TL;DR, from Plan 0060's window). **Owned by Plan 0067**, in flight now.
- **The mirror's `sync`**: streaming CPU per frame p50 3.4 to 4.2 ms against the 1.0 ms line,
  p99 45.9 to 83.6 ms, "the sync stage is most of it in every arm (3.19 to 3.90 ms p50)"
  (`docs/nfr/02-frame-budget.md`, Plan 0059's quiet re-read). The `update + poll + sync` p99 of
  40 ms above is this. **Owned by nobody.** Plan 0064 tried a time budget on it and read
  residency p95 3.0 to 13.7 s against 0.30 s, so the budget stays off
  (`docs/backlog.md`, "Slot words packed on the workers"): a budget cannot help while a chunk
  costs about 0.3 ms of render-thread time, because the work is the same whether it is spread
  or not. The roadmap has owed an "upload-copy ADR" since Plan 0023's close (2026-10-21).

What the sync does per chunk, read in `crates/vxe-render/src/gpu_world.rs`: `select`
(:2105) picks pending chunks, `need` (:1976) computes the chunk's 512 solid masks and its
surface bands under the chunk's write lock, `upload` (:2225) packs the slot words brick by
brick, scans `Full` bricks for emissive samples (`SyncStats::light_us`) and copies into the
staging belt. Every step but the belt copy and the record allocation is a pure function of the
chunk's content and the material table. The stream workers already run such a function per
loaded chunk: `vxe_world::ChunkPrep` (`crates/vxe-world/src/stream.rs:1034-1036`), used by
`bake.rs:309` (`FacePrep`) for the face masks at a mean of 177.8 us a chunk.

**Plan 0067's stop rule already fires.** Its Phase 1 says: "If `update + poll + sync` p99 reads
over 16.6 ms in rig (b) with Plan 0064 merged, the lane stops for an architect amendment before
Phase 2" (`docs/plans/0067-...md:374-375`). The tester's reading above is 39.9 ms at the RTX
window. Its Budgets table also bars "the frame's CPU ... p99 at most 16.6 ms in flight", a
figure the step alone cannot reach while the sync's p99 is 40 ms.

**Cost today.** The owner's own words after Plan 0060's and 0064's windows: "we are not there -
either lagging too much, or the picture is unacceptable or both". Every frame over 33 ms in the
table above is a visible hitch at the window, 5.6 % of the cave's frames and 8.0 % of the
castle's, on the fast GPU.

**Fix.** [Plan 0070](../plans/0070-the-sync-off-the-frame-slot-words-packed-on-the-workers.md)
with [ADR-0056](../adrs/0056-the-mirrors-slot-image-is-packed-on-the-stream-workers-and-the-render-thread-only-allocates-and-copies.md):
the workers pack a loaded chunk's slot image (words, sizes, emissive samples, face masks) in one
`ChunkPrep`, the render thread allocates records and copies bytes, and the edit path re-packs
only the dirty bricks inline. The copy is bounded by the mirror's bytes (0.87 GB live over about
17 000 full chunks at `r_full` 16, so about 50 KB a chunk on average: tens of microseconds of
memcpy, not 300). It is the amendment Plan 0067's stop rule asks for.

**Amendment note for Plan 0067 (owner):** when its Phase 1 reads the stop rule tripped, do not
stop the lane: its Phase 2 target (the step) is independent of the sync, and Plan 0070 takes the
sync. Restate 0067's frame-CPU p99 bar as "the step's share of the frame CPU", or read it after
Plan 0070 merges.

**Amendment note for Plan 0044 (owner):** its Phase 4, "the sync sizes only what it uploads",
moves `need()`'s deferral check and reuses its masks on the render thread. If Plan 0070 moves
the packing to the workers, Phase 4's mechanism is gone; strike it into 0070 (0070's Phase 3
keeps its done-when, `sized_chunks = 1`, as a property of the prepared path).

### 1.2 Render: the slow adapter (L4)

**Evidence.** Plan 0060's window on the RADV RENOIR at the window's size: frame GPU 52.3 ms, hit
26.2 ms, FSR 2 18.5 ms (1707 x 960 to 2560 x 1440), a-trous 4.4 ms; fps 15.1 (Plan 0067, Context).
At 1080p from 720p the hall's hit reads 45.7 ms on the RENOIR against 4.66 ms on the RTX (9.8x),
while the light pass reads about 2x and FSR 2 7.3x (`docs/nfr/02-frame-budget.md`, Plan 0065 close).
The 2026-10-31 ledger's conclusion stands: "the Radeon misses in every scenario before any system
is added".

**Owner today.** Plan 0069's Portable profile (720p output, scale 1.5, residency 12/18) projects
the RENOIR at about 8 ms at the vein and 19 ms at the hall, inside its 33.3 ms column, from pixel
scaling alone. That projection assumes the hit pass scales with pixels, which the 9.8x ratio
against the RTX's 2x light ratio says it may not: a walk that is bandwidth- or latency-bound on an
iGPU with DDR4 does not shrink linearly. The backlog's "slow adapter's walk-speed spike" (owed
since Plan 0054, in no plan) is the reading that would say.

**Fix, as an amendment note for Plan 0069 (owner):** its Phase 4 (exploration, both adapters)
should read the hit pass alone at Portable's internal size on the RENOIR before the profile's
values are set, and if the hit reads over 10 ms there (a third of Portable's column), the
walk-speed spike runs before Phase 6, not after M10. No new plan: Phase 4 is already the slot.
The spike's content (steps-view histogram, `spirv-opt` against naga, subgroups, `shader-f16`) is
in the backlog's two entries, unchanged.

### 1.3 Render: memory

**Evidence.** Peak RSS in Plan 0068's runs: cave 1.40 GB, castle 1.70 GB, showcase 0.77 GB
(release, RTX). Mirror live bytes 0.87 GB at `r_full` 16 (Plan 0028), the bake pool 256 MiB
default, the store's use 15.4 MB at the level (`docs/nfr/03-memory.md`). All inside `docs/nfr.md`
section 3's 4 GB RAM and 3 GB VRAM. Not a finding. The castle's 344 MB of prefabs at about 92 %
air (Plan 0042) is a known followup in that plan; nothing here changes it.

### 1.4 Render: shader compile and startup (L9)

**Evidence.** Every shader is `include_str!` (24 sites). 13 pipeline sources are
`format!("{MARCH_WGSL}\n{source}")`, so each prepends the 2496-line `march.wgsl`
(`crates/vxe-render/src/marcher.rs:1451` to `:4370`, `fsr2.rs:661`, `bake.rs:1130`); Plan 0045
already notes `sharpen.wgsl` compiling all of it. No `wgpu::PipelineCache` (every descriptor
passes `cache: None`). Cold creation of the eleven pipelines read 1396 ms on the RTX, 458 ms on
RADV, 299 ms on lavapipe (`docs/nfr.md` section 7, Plan 0046). Time to first frame reads 7137 ms
at the vein under the baked light (backlog, "Budget lines over with no owner").

**Owner.** Plan 0045 Phase 4 splits `march.wgsl` into modules a pass picks; that is the right
fix for compile time and this review adds nothing to it except a done-when suggestion: read
`pipeline_times` before and after Phase 4 on lavapipe, since compile time is the one figure a
module split moves directly. A pipeline cache is a backlog entry (below): it is Vulkan-only in
wgpu 30, so it needs a reading before it is a plan.

### 1.5 Development: the gate (L2)

**Evidence** (`node scripts/test-times.mjs` on `live/plan-0068/merge-gate.log`, the gate at the
merge of main into Plan 0068, 2026-11-11, 809 tests, 610.2 s wall):

- 177 `gpu_*` tests sum to **610.2 s serial, which is the gate's wall time**; the `lavapipe`
  test group is `max-threads = 1` (`.config/nextest.toml:44-45`).
- 632 CPU tests sum to 272.7 s and run in parallel beside them, so they cost nothing at the wall.
- 11 tests over the 10 s line, 2.6 min of it: `castle_vector` 22.1 s and `showcase_vector`
  19.6 s (CPU, kept in the gate by the owner), `the_castle_path_flies_in_air` 20.3 s (CPU, new in
  Plan 0068), `a_baked_view_is_final_on_its_first_frame` 17.5 s.

So the gate's wall is the serial sum of GPU test time, and nothing else. Cutting CPU tests
changes nothing; the levers are the GPU tests' pixels and frames, and the group's thread count.
lavapipe is itself multi-threaded (its own JIT workers), which is why the group is 1, but on a
16-thread box with the GPU tests at 64 x 36 to 320 x 180 two at once may still be faster. That
is a reading, not a fact, and Plan 0072's Phase 4 takes it.

**Cost today.** 610 s per tip gate, per close review, and per merge of main into a lane (three to
five gates per plan). The owner's figure for this review was "about 610-650 s".

### 1.6 Development: the build (L2, L7)

**Evidence.**

- **Each lane pays a cold build.** Every worktree has its own `target/`
  (`.claude/worktrees/plan-0067/target/` exists beside the main checkout's); there is no
  `.cargo/config.toml`, no shared target dir, no compilation cache. The first `quick` tier
  of a fresh lane reads `Finished test profile in` **9m25s** (`live/plan-0061/p1-quick.log`),
  **9m13s** (`plan-0068/p1-quick.log`), **7m32s** (`plan-0060/p1-quick.log`); later phases
  12 to 60 s. `scripts/test-changed.mjs` runs `cargo nextest run --workspace -E 'rdeps(...)'`
  (`:184-194`), so it builds every workspace test target, `wasmtime` and the guests included,
  however narrow the filter.
- **The per-phase gate is not narrowed where it costs most.** The dev skill's phase gate runs
  `cargo clippy --workspace --all-targets -- -D warnings` after every phase
  (`.claude/skills/dev/SKILL.md:67-70`): a check build of every crate's tests, benches and
  examples for a one-crate change.
- **The dev profile is nearly a release build with debug info.** `[profile.dev.package."*"]
  opt-level = 2` plus `vxe-world`, `vxe-sim`, `vxe-audio` and `flythrough` at `opt-level = 2`
  (`Cargo.toml:118-137`), with `debug` left at its default (full), no `split-debuginfo`, no
  `[profile.test]`. The comment's reason is sound (the cave at 52 s unoptimised against 1.7 s);
  the debug-info level is the unread knob.
- **The incremental build figure is stale.** `docs/nfr.md` section 7 bars "incremental build of
  a one-crate change under 10 s" and holds one reading, Plan 0011's 1.4 s (2026-10-17, a `pub
  const` change in `vxe-world`). Plan 0043's context noted 28 s for the same change
  (`docs/plans/done/0043-...md:41-42`), never carried to `nfr.md`. The tree has grown 3x since.
- **CI compiles the workspace about six times.** `.github/workflows/ci.yml`: `cargo build
  --workspace` (:54), `pipeline_times --release` (twice on Windows, :66-73), `nextest
  --workspace` (:81-90), `nextest -p flythrough --features wasm` (:113), `cargo test --doc`
  (:116), `clippy --workspace --all-targets` (:117), `clippy -p flythrough --features wasm`
  (:118), two wasm32 clippy runs (:122-131). The backlog's cold-cache entry (2026-11-11) read
  Windows at 55 min against 17 and already lists the fold as an option "none taken".
- Rust 1.97.1 (`rust-toolchain.toml`), so `lld` is already the default linker on Linux; no
  linker finding.

**Fix.** [Plan 0072](../plans/done/0072-development-speed-the-build-graph-a-narrowed-gate-and-one-compile-per-ci-profile.md):
read first (`cargo build --timings` cold, incremental and fresh-lane; the gate by group), then
the dev profile's debug-info level, a narrowed per-phase gate (`-p` packages for nextest and
clippy, not `--workspace`), the lavapipe group at 2, one compile per profile in CI, and a
fresh-lane warm start measured (a seeded `target/` or a pinned `sccache`; an ADR only if the
cache is adopted).

### 1.7 Development: the readings workflow and velocity (L12)

**Evidence** (`node scripts/velocity.mjs`): "the 14 days before 2026-10-28: 616 commits over 8
active days, 36 plans closed; the last 14 days: 542 commits over 14 active days, 19 plans closed;
commits per active day: 0.50x the window before". The architect skill says a window at half the
one before is a finding for the owner, not for a plan. The plans of the second window are
larger (0059 at 14 phases, 0067 at 802 lines), look-driven, and each ends in an owner window,
so part of the halving is the shape of the work, not its speed. What the workflow adds per plan,
from the lane rules: a cold lane build (1.6), three to five gates (1.5), readings on two adapters
under one GPU lock, and a close review that re-runs the gate. Plan 0072's Phases 2 to 4 are the
part of that this review can shorten; the rest is the owner's choice of plan size.

## 2. Maintainability

### 2.1 The mega files and Plan 0045's reach (L5)

**Evidence** (`node scripts/loc.mjs`): Rust code 45 299, Rust test 51 565 (test : code
**1.14**), WGSL 5 422, markdown 73 784. Largest: `apps/flythrough/src/main.rs` **9698**,
`bake.rs` 4642, `marcher.rs` 4390, `gpu_world.rs` 3589, `tests/gpu_bake.rs` 3215,
`tests/gpu_mirror.rs` 2704, `march.wgsl` 2496 (100 functions), `hud.rs` 2458,
`tests/support/settle.rs` 2342, `tests/gpu_look.rs` 2203.

`main.rs` in detail: a 258-line module doc; a **2555-line in-file `mod tests`** (62 tests, lines
7144-9698); `parse_args_from` **660 lines** (:846-1505), a hand-rolled `match` over **111 flag
arms, 16 of them refusals** of flags Plans 0057 and 0059 removed, and **no `--help`**; `Args`
90 fields, `Run` 41 fields, `impl Run` in two blocks of 641 and 1951 lines; `Run::draw` 339
lines; 168 `println!`; `mirror.sync` called from six functions (`bench_sync`, `init`,
`queue_pre_roll`, `init_stream`, `stream_step`, `draw`). In `vxe-render`: `Frame` 75 fields,
`GpuWorld` 61 (Plan 0045 targets 20 and 15).

**What Plan 0045 covers.** Phases 2 to 8 name every file over 1500 lines but three, and its
Phase 7 moves `main.rs` into library modules with a flag table and a generated `--help` (done
when `main.rs` is under 200 lines). Its caps (1200 source, 2000 test, 150 per function) and the
two gates (`check-file-size.mjs`, clippy `too_many_lines`) are the right mechanism. Seven
functions in `main.rs` are over the 150-line cap today.

**What it misses, as amendment notes for the owner:**

1. **A premise is stale.** The plan's header says the reservoir is "gone, and struck" (Plan
   0057). `crates/vxe-render/src/shaders/light.wgsl:270-330` still defines `struct Reservoir`,
   `merge`, `draw_candidates` and `resolve_emissive`, called at `light.wgsl:104` and `:330` and
   `bake.wgsl:136`. They are the one-sample emissive estimator ADR-0018 kept, not the screen
   reservoirs 0057 removed. Phase 4's module cut (`common`, `mirror`, `walk`, `hitrec`) should
   list where they go; Phase 0's inventory should name them.
2. **`hud.rs` (2458) and `showcase.rs` (1570) are over the source cap today**, and the plan
   names them only "if Phase 0 reads them over the cap". Name them unconditionally in Phase 7.
3. **Three test files between 1636 and 1814 lines** (`gpu_golden.rs`, `gpu_strip.rs`,
   `gpu_fsr2.rs`) are under the 2000 cap and unnamed. Fine under the cap; but `gpu_strip.rs` is
   not a test file (see 2.3) and should leave `tests/`.
4. **Fixture duplicates Phase 1 does not name:** `fn table()` (a `MaterialTable`) defined 9
   times (`gpu_bake.rs:56`, `gpu_queue.rs:24`, `gpu_translucent.rs:49`, `gpu_walk.rs:48`,
   `occlusion.rs:31`, `vxe-sim/tests/support:25`, `brick_kernel.rs:52`, `edits.rs:29`,
   `census.rs:8`); `fn now()` 6 times across benches, examples and `main.rs:392`; a GPU
   `fn device()` in 5 `vxe-render` test files plus `gpu_level_mirror.rs:39` (Plan 0044 Phase 8's
   helper covers the device; the table does not have a home). Phase 1 names `SplitMix64` (4),
   `hash_result` (2, byte-identical bodies) and `carved_world` (3; `gpu_golden.rs:55`
   duplicates the support module it already imports); add the table and `now`.
5. **A done-when for Phase 2's narrowing:** `vxe-render` has 680 pub items today (412 fns, 191
   consts; 13 `pub mod`). State the count the curated re-export list leaves, so the narrowing
   is a number, not an intent.

### 2.2 Settings with no setter, setters with no caller (L6)

**Evidence** (grep over `crates/` and `apps/`, definition sites in `crates/vxe-render/src`):

- `Upscaler::{width, clip_gamma, change_rel, change_abs, change_frames}` are written only in
  `Default` (`marcher.rs:695-701`); `alpha` and `alpha_moving` only by `gpu_upscale.rs`. These
  are the `own` upscaler's knobs; the removal plan takes them.
- Set only by tests, never by the app: `Lighting::{tone_map, probe_freeze, lamp_reach,
  probe_move_blend, bake_pool_bytes}`; `LightParams::{offset_right, offset_down, intensity,
  outer, skips}` (`skips` and `masks` documented "Off only for tests", `light.rs:19-27`);
  `BakeSettings::{submit_rounds, jobs_alone, reversed, keep_faces}`;
  `AutoExposure::compensation_ev`.
- `Frame`: of 98 pub fns, **52 are used by nothing under `crates/`**; 10 are used nowhere at
  all (`metering`, `set_probe_axis`, `sharpness`, `upscaler`, `set_bake_slice`, `digs_pending`,
  `pending_digs`, `probe_emission_scale`, `emission_scales`, `transmittance`). `GpuWorld`:
  `band_units`, `field_items`, `rewrite_materials`, `spare_slots`, `update_field_timed`,
  `light_samples`, `light_span`, `light_range` have no caller outside the file.
- `Params::flags` uses bits 0 to 10 and 19; bits 11 to 18 are holes. `Params::lighting` has
  holes at 2, 4, 7, 10, 11 (`marcher.rs:105-161`). `FLAG_*` is defined twice, in Rust and as
  literals in `march.wgsl:284-293` (Plan 0044 Phase 8's layout test catches a drift there).
- **`Lighting::lamp_bounce` defaults to `false` in the library (`marcher.rs:858`) and `true` in
  the fly-through (`main.rs:895`)**; `vxe-render/tests/gpu_frame.rs:86` calls the library's
  value "the default". A consumer who takes `Lighting::default()` gets a different image from
  the one the owner judged.

**Owner.** Plan 0069 builds `RenderSettings` from these fields and classifies each as `Live`,
`Settle` or `Rebuild`. **Amendment notes:** (a) its Phase 1 table should carry a fourth class,
**`Delete`**, and the list above is its first draft, so a field with no setter and no caller
leaves instead of becoming a documented setting; (b) `Floor` must equal the fly-through's
defaults, and `Lighting::default()` (and every settings struct's `Default`) must equal `Floor`,
with a test that says so, so the library's default is the look the owner picked; (c) its
setter gate ("a `pub fn set_*` lands without a field or an input listing") should count the
22 existing setters, not only new ones.

### 2.3 The reading harness in `tests/` (L8)

**Evidence.** About 95 `std::env::var` sites and **about 75 distinct variable names** across
the fly-through's tests: `gpu_strip.rs` alone reads **40** `VXE_STRIP_*` names through
`env()`/`env_or()`/`on_off()` (:104 and 1417-1656); `gpu_settle.rs` 16; `VXE_BLESS` at 11
sites; `VXE_STEADY` (`gpu_settle.rs:254, 892`) drives a `Steady` shim
(`tests/support/settle.rs:134-170`) whose every value maps to the default, for a `--steady`
flag the binary has refused since Plan 0057. Production `src/` reads seven variables in all,
none in `vxe-render` or `vxe-world`.

**Cost.** A reading tool's switches are undiscoverable and untested (no test exercises
`VXE_STRIP_PAN`), and a stale shim keeps a dead mechanism's vocabulary in the tree.

**Owner, as an amendment note for Plan 0045 Phase 8:** `gpu_strip.rs` and the `mod reading`
harnesses that take more than two switches become examples or `src/bin` tools with flags
(`apps/flythrough/examples/strip.rs`), sharing Phase 7's flag table; the `Steady` shim and
`VXE_STEADY` are deleted; a gate line in `check-test-tiers.mjs` caps `env::var` reads per
test file (a proposal: 4; `VXE_BLESS`, `VXE_REQUIRE_SOFTWARE_ADAPTER` and `VXE_TEST_ADAPTER`
are the ones that belong).

### 2.4 Comment, manifest and index drift (L11)

- `apps/flythrough/examples/world_budget.rs`: 26 "contour" lines, "Contour adds the attribute
  units" (:35), "an upper bound on the attribute pass's planes" (:41), "Bricks by the attribute
  pass's rounds" (:656): the ADR-0011 attribute-page arithmetic Plan 0054 removed.
- `apps/flythrough/tests/gpu_lights.rs:610` "on the emissive table's path" (Plan 0059 deleted
  the table); `gpu_lights.rs:614, 687` "Reservoir reuse's bar"; `crates/vxe-render/src/exposure.rs:21`
  "reservoirs"; `gpu_upscale.rs:635` "two `rgba32uint` reservoir textures".
- `Cargo.toml` of `vxe-world` and `vxe-render`: `description = "placeholder; see docs/roadmap.md"`.
  No crate sets `publish = false`, so a `cargo publish` of a placeholder is one command away.
- `CLAUDE.md:39-40` gives `vxe-render` "voxel-model entities"; the crate has no entity code
  (Plan 0062 is approved, not started). The line also omits the bake store and FSR 2.
- 15 `diag(...)` lines in `main.rs` (:400-410, :6339) marked "DIAGNOSTIC ... out with the WARP
  preload fix"; the preload is deleted, the backlog's WARP entry keeps them until one more push.
- `docs/nfr.md` section 7's incremental-build line (1.6 above).

**Owner.** A dev chore in the backlog (below), one commit, no plan. The `CLAUDE.md` line is
the owner's (this review writes only under `docs/`).

### 2.5 Where the renderer is tested

**Evidence.** `apps/flythrough/tests/` holds 18 `gpu_*` files, 20 711 lines; `crates/vxe-render/tests/`
8 files, 5115 lines. Plan 0045 says, under what it does not do: "Move the fly-through's render
tests into `vxe-render`. They need the cave scene builders." That is true of the goldens and
the look tests; it is not true of the queue, the bake, the FSR 2 history and the frame-wait
tests, which need a world, not a cave. A renderer that is only testable through its testbed is
also a renderer a consumer cannot read a test of. Plan 0071's Phase 2 gives `vxe-render` a
world-only rig and writes its new tests there; moving existing ones stays a followup.

## 3. The consumer's API

The first consumer's P0 list (`~/Work/voxel-roguelite/docs/engine-requirements.md`) maps to an
engine API in every row: bounded volumes (`World::generate`), 16-bit materials, the four carve
shapes with per-material results (`EditOp`, `EditResult`), the indestructible flag (per
material, as the roadmap decided), the material table's payload, HRTF playback and occlusion,
the coarse grid (0.8 m, accepted), raycast and the cylinder controller, `Logic`/`Host` and the
in-memory channel, `Fixed3` and `World::replay`. The gaps are not missing mechanisms; they are
how a consumer finds and holds them.

### 3.1 No path to a frame (L3)

**Evidence.**

- `crates/vxe-render/src/lib.rs:1-6` is six lines of crate doc and no example; `vxe-app` and
  `vxe-script-wasm` have none either (`vxe-world` and `vxe-sim` have one each). The root
  `README.md` has no Rust snippet. The seven `examples/` are tools (`pipeline_times`,
  `hrir_pack`, `voxtext`, `hero`, `world_budget`, `castle_gen`), none a consumer path.
- To render one frame a consumer follows `main.rs:4033-4120` and `:5760-5830`: request an
  adapter and device themselves (the library has no device helper; the tests carry one at
  `apps/flythrough/tests/support/mod.rs:52`), call `vxe_render::limits_for`, build a `World`
  and a `Streamer`, `GpuWorld::with_config(MirrorConfig::new(Window::...))`, `Frame::new`,
  `Presenter::new`, then per frame `apply_events`, `set_focus`, `sync`, `invalidate`, `params`,
  `encode`, `advance`, `presenter.encode`, `submit`: twelve steps across three objects, none
  written down outside the testbed.
- `Frame` has **98 pub fns** (22 `set_*`, 19 bake and store, 14 raw `&wgpu::Texture` getters,
  `bake_until` with 7 arguments, `encode` with 6). `Upscaler` is taken by `Frame::set_upscaler`
  but not re-exported (reachable only as `vxe_render::marcher::Upscaler`). `Params` (38 pub
  fields) and `View::_pad`/`_pad2` are public. 13 `pub mod`, 191 pub consts (GPU bit layouts).
- Testbed vocabulary in library docs: "The lower-case name the fly-through prints and parses"
  on `name()`/`from_name()` (`marcher.rs:198, 600`, `fsr2.rs:111`, `exposure.rs:74`); "Off by
  default here; the fly-through turns it on" (`marcher.rs:812`); "the streamed cave's 17 000
  candidates" (`bake.rs:3000`). No item name carries a scene word; the leak is in the docs and
  in the defaults (2.2).
- No crate has `missing_docs` or `unreachable_pub`; the count of pub fn/struct/enum/trait with
  no doc line directly above: `vxe-world` 63, `vxe-render` 55, `vxe-audio` 21.

**Cost.** The consumer game is the reason the engine exists; its first playable slice needs M1 to
M5, all of which are in the tree, and today it would start by copying 1800 lines of `main.rs`.

**Fix.** [Plan 0071](../plans/0071-the-consumers-path-a-render-example-a-renderer-tested-alone-names-and-errors.md)
with [ADR-0057](../adrs/0057-the-render-api-is-three-objects-and-a-settings-struct-with-a-compiled-example-as-the-contract.md):
a compiled, CI-run `examples/one_frame.rs` in `vxe-render` that renders a world to a PNG headless
in under 60 lines is the contract; the crate docs and README quote it; `vxe-render` gets a
world-only test rig and a `test-support` device helper (Plan 0044 Phase 8's) so new renderer
tests live in the renderer; names and errors are made consistent with re-exports kept for one
minor; `missing_docs` goes warn, then deny. It runs after Plans 0045 (the narrowed surface) and
0069 (`RenderSettings` replaces most setters), because both change the surface it documents.

### 3.2 Names (L10)

**Evidence** (`crates/`):

- The GPU copy of the world is `GpuWorld`, configured by `MirrorConfig`, failing with
  `MirrorError`, filled by `sync` (returns `SyncStats`), described as "the mirror" 141 times and
  "upload" 57 times.
- Config suffixes: `LightParams, LookParams, LodParams, ControllerParams, SupportParams,
  WalkableParams, PlayParams` / `BakeSettings, Fsr2Settings, MixSettings, ReverbSettings` /
  `MirrorConfig, StreamConfig, PersistConfig, ShellConfig, WasmConfig` / `SurfaceTuning` /
  `Lighting`. The lamp is split between `LightParams` (per call) and `Lighting` (on the frame).
- `Host::raycast(origin: Fixed3, dir: Fixed3, max: i64) -> Option<Hit>` (`vxe-net/src/logic.rs:88`)
  against `World::raycast(origin: Fixed3, dir: [i64; 3], max_t: i64) -> Result<Option<Hit>, EditError>`
  (`raycast.rs:91`); `Host::unsupported(&[CellCoord])` against `find_unsupported(&[CellChange])`
  (`support.rs:209`); `WalkableMap` addresses cells as `(ChunkCoord, u16)` (`walkable.rs:93`).
- `tick` (`Server`, `Logic`, `CharacterController`, `OpLog`) against `Frame::step` and
  `AutoExposure::step`, with `Frame::advance` for the next frame. `HudColour` against `TextColor`.
- The voxel scale: `VOXELS_PER_METRE = 10.0` is a private constant in `vxe-render/src/light.rs:5`,
  while `vxe-audio` takes `metres_per_voxel` as an argument (`Mixer::new`, 23 uses in
  `acoustics.rs`). The roadmap decided "voxel size one scalar per world"; the world does not
  carry it.
- `SpillError` and `StreamError` have no `Display` or `Error` impl; the other 14 error types do.
  `Streamer::update(&mut self, _world: &World, ...)` takes a parameter it does not use
  (`stream.rs:376`).

**Owner.** Plan 0071 Phases 3 to 5.

### 3.3 Requirement gaps worth a sentence in the consumer guide

Not defects, but things the consumer will ask: `Material::hardness` is stored and read by
nothing in the engine (the game's rule, as the roadmap says of fall damage); a runtime emission
change is a render-side scale (`Frame::set_emission_scale`) or a replace-material op, since
`MaterialTable` has no setter (`GpuWorld::rewrite_materials` exists with no caller); region
change notifications are return values (`CoarseGrid::update -> Vec<CellChange>`,
`Client::poll -> Vec<Event>`), not a subscription. Plan 0071's Phase 1 example and Phase 6 guide
say each of these in one line.

## 4. What this review does not do

- Measure anything. The constraint was absolute; every "reads" above is a reading already in
  the tree and every "would" is a plan phase.
- Redraw Plan 0045's seams, 0069's profile table or 0067's rigs. The amendment notes above are
  for the owner to accept or refuse; no approved plan was edited.
- Draft the `own` removal plan (owed after Plan 0069, backlog), the walk-speed spike, or the
  documentation site (backlog). None of them ranked above the three drafted.

## 5. Owner questions

1. **Plan 0070's slot.** Right after Plan 0067 and before 0045 (recommended: it is 0067's own
   stop-rule amendment, it serves "we are not there", and 0045 re-cuts `gpu_world.rs`'s seams
   at its Phase 0 anyway), or after 0045 on the split tree.
2. **Plan 0044 Phase 4.** Strike it into Plan 0070 (recommended) or keep both.
3. **Plan 0071's slot and the renames.** After 0045 and 0069 (recommended), renaming with
   deprecated re-exports kept for one minor (recommended; 0.x, "any minor may break" per the
   CHANGELOG).
4. **Plan 0072's cache.** Allow a pinned compilation cache (`sccache` by `cargo install
   --locked`) if Phase 1 reads the fresh-lane build at over 5 min and the cache halves it
   (recommended: read first, adopt on the number), or keep per-lane cold builds.
5. **The lavapipe group at 2 threads.** Measured in Plan 0072 Phase 4, kept if the gate falls
   by a quarter with no flaky test over three runs (recommended), or left at 1.
6. **`docs/reviews/`.** Keep standalone reviews here (this file is the first) and add the row to
   `CLAUDE.md`'s tree (the owner's edit), or fold future ones into the backlog.
