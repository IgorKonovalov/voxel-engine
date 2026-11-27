# NFR section 7: startup and build, the readings

> The budgets are in `../nfr.md` section 7, one bullet per figure; this file holds the readings
> taken against them that are too long for the bullet, each naming its machine, commit and load.
> Earlier readings of the test gate stay inline in `nfr.md`. A bare `§N` in this text is a
> section of `nfr.md`.

## The test gate's wall time (the gate-time chore, 2026-11-24)

The chore on branch `chore-gate-time`, cut from main at `193cae15` (v0.72.0), owner's go of
2026-11-24 ("the gate-time cut first"), against the 19 min line of §7.

**Machine and conditions, every run below.** The dev reference (AMD Ryzen 9 5900HS, 16
threads, 30 GiB), lavapipe (Mesa 26.2.5, LLVM 23.1.3), `cargo nextest run --workspace --profile
gate --status-level pass --final-status-level fail`, the gate's binaries built before the run
(147 s, 12 build jobs, outside the lock), exclusive perf lock, 16G memory cap, no test-thread
cap, on AC power. A run starts once the 1-minute load is under 2 (the load at its start and end
are given). **The CPU ran capped at its 3.3 GHz base clock** (`scaling_max_freq` 3 301 000 kHz
on all 16 threads against `cpuinfo_max_freq` 4 683 471, amd-pstate-epp, boost flag 1): asusd
switched to its quiet profile when the AC dropped at 2026-11-23 15:14, and the cap stayed after
the AC came back. The readings below are at that cap, so they read about 1.42 times (4.68 / 3.30)
slower in compute-bound work than the same box at full clock.

### Attribution at the base (`193cae15`)

| | Reading |
|---|---|
| Wall (nextest summary) | **2145.0 s** (35.7 min); load 1.83 at the start, 3.05 at the end |
| Tests | 1361 run, 1361 passed, 242 skipped |
| The lavapipe group (334 tests in `gpu_*` binaries, `max-threads = 1`) | 2141.2 s serial |
| CPU tests (1027, beside the group) | 387.0 s serial |
| Tests over the 10 s line | 72 (62 GPU, 10 CPU), 17.1 min of the serial time |
| A GPU test's time | median 6.0 s, p25 3.9, p75 8.5, p90 12.4 |

- **What bounds the wall:** the lavapipe group's serial sum. 2141.2 s of GPU tests at one at a
  time against a 2145.0 s wall; the CPU tests finish beside it. Build time is outside the
  figure (the binaries were built first).
- **Why one at a time wasted the box:** the 1-minute load sat near 3 through the run on 16
  threads. Most of a GPU test's time is pipeline creation (each test process builds its frame's
  compute pipelines, which lavapipe compiles through LLVM on the calling thread), not
  rasterising, so one lavapipe process does not spread over the cores. Device creation and
  shader compilation per test process are the fixed cost; the engine's pipeline cache (its own
  backlog entry) is the lever on that cost and is not this chore's.
- **Box state against growth.** Against Plan 0096's lane gate of 2026-11-23 05:40 (`501bcd95`,
  same boot, `p9-gate3.log` in its archive), the tests both runs share (1 s or more) read a
  median 1.44 times longer on the GPU and 1.25 on the CPU (`castle_vector` 16.7 to 20.0 s,
  `showcase_vector` 11.8 to 15.3, `the_castle_path_flies_in_air` 12.3 to 17.4). Between those
  commits `svet-world`, `svet-sim`, `determinism.rs` and `path.rs` are unchanged and the
  manifests differ only in version numbers, so the CPU tests' growth is the box. The clock cap
  above (set at 15:14 that day, after that run) is 1.42 times, which accounts for the GPU
  tests' 1.44 within about 1 %; the CPU tests scale less than the clock (memory-bound work).
  Against Plan 0098's close log (`merge-gate.log`, loaded) the same tests read 0.99 (GPU) and
  0.98 (CPU): this quiet base and that loaded run agree. So the 2153 s at Plan 0098's close is
  the clock cap, not Plan 0098's shaders (an inference from the ratios; no run of main before
  Plan 0098 at the cap was taken).

### The lavapipe group's thread count

| `max-threads` | Commit | Wall | GPU serial sum | Median GPU test against N = 1 | Tests over 2x their N = 1 time | Load start / end |
|---|---|---|---|---|---|---|
| 1 (base) | `193cae15` | 2145.0 s | 2141.2 s | | | 1.83 / 3.05 |
| 2 | `193cae15` | 1190.3 s | 2369.2 s | 1.10 | 1: `gpu_entity_pool` `a_reload_keeps_the_picture_steady` 9.2 to 23.4 s | 1.76 / 4.59 |
| 3 | `a1bba8d1` (scripts only) | 889.7 s | 2651.3 s | 1.26 | 1: `gpu_bake` `a_baked_view_is_final_on_its_first_frame` 6.7 to 21.8 s | 1.99 / 4.61 |

Every run 1361 of 1361 passed. Plan 0072 Phase 4 read the same shape on 2026-11-12 (656 s at
1, 355 to 368 at 2, 259 to 262 at 3). The owner's go on two threads stands from 2026-11-22
(`docs/reviews/2026-11-22-backlog-review.md`, answer 3); `.config/nextest.toml` takes 2
(`909a2552`). Three is an owner question (the chore's report).

### Tests shortened (tier unchanged)

Each test below runs arms that share nothing but the device (each its own mirror and frame, or
its own fly-through process) one after another; they now run on scoped threads
(`support::joined` re-raises a thread's panic), so their pipeline compilation overlaps. No
assertion, scene, frame count or golden changed (`ad790c40`). Quiet means alone, one test
thread, load under 2 at the start, at the cap.

| Test | Base gate (N = 1) | Alone, quiet, after | Quiet after gate (N = 2) |
|---|---|---|---|
| `gpu_golden` `uniform_chunks_render_identically_from_their_mips` | 24.0 s | 10.3 s | 11.8 s |
| `gpu_loading` `a_sliced_pre_roll_bakes_what_an_unsliced_one_bakes` | 22.5 | 14.0 | 14.1 |
| `gpu_indirect` `the_channel_holds_the_lamps_bounce_only` | 19.8 | 7.7 | 8.7 |
| `gpu_dynamic_lights` `a_red_light_near_a_wall_draws_no_rings_under_fsr2` | 19.8 | 13.1 | 13.7 |
| `gpu_golden` `hit_buffer_views_match_the_single_pass_march` | 17.0 | 8.4 | 8.7 |
| `gpu_golden` `fsr2_golden_matches_and_rcas_is_bypassed_at_zero` | 17.0 | 12.9 | 12.1 |
| `gpu_golden` `upscaled_golden_sharpens_within_the_neighbourhood` | 15.8 | 12.5 | 11.6 |
| `gpu_entities` `the_probes_and_the_bake_see_no_entity` | 16.4 | not read alone | 10.5 |

The soak test `a_saturated_light_near_a_wall_draws_no_rings` shares the rings helper and
overlaps the same way.

### Tests whose tier changed

**None.** No test took the soak mark or the sole mark in this chore. The GPU tests over the line
at the cap that are over it at full clock too by the ratio (over 14.2 s at the cap) are the
gate's only test of their mechanism: most are the deliberate gate variants of soak tests
(`a_red_light_near_a_wall_draws_no_rings_under_fsr2`, `fsr2_marks_a_dropped_list_light`), the
three loading-screen runs are one fly-through process each, and the rest step one rig whose
state carries from arm to arm (a second bake on the same store, a frame swapped mid-run), so
overlapping their arms would change what they test. With the wall under the line they stay in
the gate and are listed here:

| Binary | Test | Base gate (N = 1) | Quiet after gate (N = 2) |
|---|---|---|---|
| `flythrough::gpu_fsr2` | `fsr2_marks_a_dropped_list_light` | 18.3 | 15.6 |
| `flythrough::gpu_loading` | `a_benchmark_warms_up_from_the_first_world_frame` | 17.7 | 21.3 |
| `flythrough::gpu_dynamic_lights` | `a_pulsing_emitter_marks_only_where_its_light_lands` | 17.4 | 18.8 |
| `flythrough::gpu_loading` | `a_source_switch_shows_the_loading_frames_before_the_new_world` | 17.0 | 18.1 |
| `flythrough::gpu_bake` | `a_bake_with_no_key_light_leaves_the_other_words` | 16.4 | 15.7 |
| `flythrough::gpu_entities` | `a_moving_entity_leaves_no_ghost_and_no_stale_shadow` | 16.1 | 16.6 |
| `flythrough::gpu_entities` | `history_does_not_cross_between_instances` | 14.5 | 13.5 |
| `flythrough::gpu_golden` | `dark_cave_golden_is_lit_by_its_ore_alone` | 14.4 | 15.9 |
| `flythrough::gpu_bake_tiers` | `a_lod_only_chunk_in_a_partial_reach_casts_from_its_mip` | 14.3 | 13.5 |
| `flythrough::gpu_queue_reach` | `a_break_through_reaches_the_far_wall` | 14.3 | 13.1 |

A test's time at two threads includes its partner's contention (median 1.10 times its N = 1
time), so `scripts/test-times.mjs` over a two-thread log counts more tests over the line than
the same tree at one (76 against 72 here); the line is read on a test's N = 1 time.

**The sole mark** (owner, 2026-11-24): a test over the line that is the gate's only test of its
mechanism and cannot be shortened may leave the gate as `#[ignore = "sole: <mechanism>"]`; the
soak and full tiers run it, and `node scripts/test-changed.mjs` runs it whenever its package or
a dependency changed (`a1bba8d1`). The list is derived from the marks:
`node scripts/test-changed.mjs --list-sole` (empty at this chore's tip). CI's push run does not
run a sole test, so **the weekly soak is its only other net**; a red weekly soak is then the
only signal from CI for those tests.

### After (`909a2552`)

**1133.2 s** (18.9 min, under the 19 min line by 7 s), 1361 run, 1361 passed, 242 skipped;
load 1.98 at the start and 4.14 at the end; the lavapipe group 2251.6 s serial at two threads,
the CPU tests 372.7 s; at the 3.3 GHz cap. The same tree's first run started at a load of 10.3
(the lock was held by a soak run while it queued): 1201.3 s, 1361 passed, loaded, indicative.

### The weekly soak run of 2026-11-23 (run 45165663078, main `123964e7`), read again

Red with 5 of 111 failing, 9978 s. Read on this tree (`909a2552`) under the perf lock, one
test thread, quiet at the start:

| Test | CI (`123964e7`, GPU tests in parallel) | Here (`909a2552`, lavapipe) |
|---|---|---|
| `gpu_wobble` `the_wobble_table_over_the_arm_set` | failed after 6575.7 s: `stop +0 near 19.30 % against the lighting-off 0.04 % + 2` | not run: a reading in `mod reading` with no assertion since Plan 0098 (`c46fb41a`) |
| `gpu_fsr2` `a_still_camera_under_our_upscaler_is_nearer_the_linear_supersampled_render_than_a_blit` | `ours 3.256 not below the blit's 2.738` | passed, 34.3 s |
| `gpu_upscale` `a_still_camera_upscales_nearer_the_supersampled_render_than_the_bilinear_blit` | `upscaled 3.341 not below bilinear 2.774` | passed, 32.8 s |
| `gpu_shutdown` `the_headless_flythrough_exits_through_every_teardown_stage_within_bound` | the run over its 120 s bound (stdout stops after `structures:`) | passed, 55.5 s |
| `gpu_outdoor` `the_underground_eye_reads_no_sky` | `surface stored 157.49023807374985` | **failed, the same value**, 224.2 s |

`the_underground_eye_reads_no_sky` is a real failure, deterministic across CI's and this box's
lavapipe: the surface reference's stored part reads 157.49 against the bar's 149.7 within 5 %
(+5.2 %), a reading taken at `2fe4598c`. The near-eye visibility bar before it passed; the
parts after the reference in the test's loop were not reached. Not fixed here: the reference moving is an engine
change to attribute, not a tier fault. The two still-camera upscaler failures do not reproduce
on this tree; whether main since `123964e7` or CI's older LLVM 21 lavapipe explains them, the
next weekly run shows. The shutdown bound is CI's load (the `ci-soak` profile runs the GPU tests
in parallel, and this test's fly-through process competes with them).

### The `gpu_wobble` soak tests on one baked store (Plan 0104 Phase 0b, `127e7672`)

Quoted from [`plans/logs/0104.md`](../plans/logs/0104.md) ("Phase 0b"), not read again. The
RTX 3080 Laptop (driver 615.78.08) and lavapipe (llvmpipe Mesa 26.2.5, LLVM 23.1.3) on the RTX
laptop machine, each test alone under the exclusive perf lock, nice 19, from a frozen release
copy of the binary at `127e7672`; the 1-minute load at the lock.

| Test | Adapter, frame | Load | Wall | Settle | Reads (fly 180 / fly 20) |
|---|---|---|---|---|---|
| `the_defaults_hold_still_at_rest_and_the_far_holds` | RTX, 1920x1080 | 1.42 (22:06:26) | 145 s | 30.1 s | 60.8 / 45.0 s |
| `the_defaults_hold_still_at_rest_and_the_far_holds` | lavapipe, 960x540 | 0.87 (22:09:30) | 253 s | 141.4 s | 60.6 / 46.9 s |
| `the_far_moves_no_worse_than_the_before` | RTX | 1.29 (22:08:51) | 5 s | 0.2 to 0.3 s each | |

The forest test before the shared settle (Plan 0104 Phase 0, six arms at 1920x1080) read 771 s
on the RTX and 12 538 s on lavapipe. Its bar on lavapipe is 600 s. The other test was not read
on lavapipe in Phase 0b.

## Time to the first frame and the in-view wait after it, by the pre-roll's default (Plan 0108 Phase 4, 2026-11-27)

The defaults since Plan 0108 Phase 4 (the owner's pick at that plan's Phase 2): in a window the
light pre-roll ends at the near chunks (`--pre-roll near`) over a streamed source (`cave`,
`outdoor`) and runs in full (`full`, the near chunks and then the coarse pass in its 5 s box)
over a bounded one (`level`, `showcase`); `--headless` keeps `full`. `--pipeline-threads`
defaults to 4 on 16 or more logical CPUs, else 2 (the cold figures below).

**Warm, headless** (release, pipelines warm, `--bake-thread on`, the tree at Plan 0108 Phase 4
before its commit on `e6bba08b`, one start an arm, `--pre-roll` given so the headless run takes
the window's arm; dev host, Ryzen 9 5900HS, 16 logical CPUs; exclusive perf lock, 1-minute load
3.0 to 9.2: **loaded**, indicative). RTX: `--adapter RTX` (Floor, 1280 x 720), 4000 frames.
RENOIR (Radeon iGPU, RADV): `--adapter RENOIR --profile portable --render 1280x720`, 1000
frames. "In view lit" is the first `bake queue:` line (once a second) whose `bake queue view:`
line reads no in-view chunk without a block, minus `first frame after`; it read the same as
"in view final" (no blank and no coarse block in view) in every run that landed.

| adapter | scene | arm | first frame after ms | pre-roll steps, ms of steps | in view lit after the first frame | from the start |
|---|---|---|---|---|---|---|
| RTX 3080 Laptop | forest (`--source outdoor --spawn forest`) | `full` | 6336 | 277, 5678 | 4.9 s | 11.2 s |
| RTX 3080 Laptop | forest | `near` (the window's default) | 1246 | 61, 695 | 6.6 s | 7.8 s |
| RTX 3080 Laptop | cave (`--source cave`) | `full` | 8749 | 305, 8260 | 10.3 s | 19.0 s |
| RTX 3080 Laptop | cave | `near` (the window's default) | 4479 | 205, 3980 | 14.5 s | 19.0 s |
| RENOIR | forest | `full` | 6566 | 178, 5918 | 6.7 s | 13.3 s |
| RENOIR | forest | `near` (the window's default) | 1417 | 25, 886 | 10.1 s | 11.5 s |
| RENOIR | cave | `full` | 9499 | 197, 8905 | not by the run's end (28.1 s from the start: 721 in view without a block) | |
| RENOIR | cave | `near` (the window's default) | 4461 | 103, 3929 | not by the run's end (23.0 s: 1237 in view without a block) | |

Plan 0108 Phase 1's medians of three (the same host, loaded, `--frames 8`): forest `full`
6430 ms RTX and 6639 RENOIR, `near` 1445 and 1560; the showcase's hall (bounded, `full` stays
its window default) 1646 and 3505. Raw logs: `live/p4/` and `live/p1/` in the plan-0108 lane's
worktree (untracked); readings in `docs/plans/logs/0108.md`, Phases 1 and 4.

**Warm, in a window** (Plan 0108 Phase 2, the owner's sitting, 2026-11-27, release `e6721462`,
the owner moving; `docs/plans/logs/0108.md`, Phase 2): RTX forest `full` 8980 ms (the warm-up
9018), `near` 1882; the tunnel (`--spawn inside`) `near` 3924; the hall `near` 1514 RTX and 3683
RENOIR. The window's `full` start came about 2.5 s after the headless figure with the same
pre-roll (not attributed).

**Cold** (Plan 0108 Phase 3, release, `--headless --source outdoor --frames 3`, `full`, each
start on an emptied driver cache in a scratch directory, loaded; `docs/plans/logs/0108.md`,
Phase 3): first frame after, means of two, at `--pipeline-threads` 1 / 2 / 4: RTX 58 836 /
40 196 / 37 308 ms; RENOIR 16 886 / 13 455 / 12 425 ms. The pipeline span 49.0 / 31.0 / 28.0 s
(RTX) and 8.4 / 6.3 / 5.1 s (RENOIR).

**Against the 3 s proposal** (`../nfr.md` section 7, left a proposal): the window's warm default
reads under 3 s on the forest on both adapters headless (1.2 and 1.4 s) and in the window on the
RTX (1.9 s), and on the hall on the RTX (1.6 s headless); it reads over it on the cave (4.5 s on
both adapters headless), the tunnel (3.9 s, window), the hall on the RENOIR (3.5 s headless) and
every cold start (12.4 to 37.3 s at the default thread count).
