# Non-functional requirements

The budgets the engine is held to. Every "fast", "large" and "stable" claim elsewhere means one
of the figures below. Each figure is a **proposal until a measurement replaces it**; a
measurement names the commit, the machine, the resolution and the scene. A plan that touches
storage or the frame states which figures it affects and measures them at its close.

Written 2026-10-16. Readings are added under the figure they measure, each naming its source.

## 1. Hardware

| | Baseline (the floor) | Dev reference | Portable profile (a quality profile, not a gate) |
|---|---|---|---|
| GPU | 2019 mid-range desktop: GTX 1660 / RX 5600 class, 6 GB VRAM, Vulkan 1.2, no RT hardware | NVIDIA RTX 3080 Laptop, 8 GB, driver 610 | Switch-2 class: about 1.7 to 3 TFLOPS, 68 to 102 GB/s, published figures; no device on hand |
| CPU | 4 cores / 8 threads | 16 threads (AMD, Cezanne) | 8 ARM cores class |
| RAM | 16 GB | 30 GB | 9 GB shared by CPU and GPU |
| OS | Linux (Vulkan), Windows (Vulkan or DX12 through wgpu) | Arch Linux, Wayland | none targeted; the profile is held on a PC of that class |

A perf figure measured only on the dev reference is labelled as such. The baseline card is to be
acquired or borrowed before M2 closes; until then the baseline numbers are projections.

The portable profile (owner, 2026-10-17; `roadmap.md`, "Floor, profiles and scene scale") is
a named setting of the quality knobs (internal resolution, full-detail radius, translucent
layers, lighting tier) at 720p output and 30 to 60 fps. It has no reading until a machine of
that class is on hand; until then its figure is the storage reads per ray that Plan 0015
counts, against about three uncached 64-byte lines per ray at 720p and 60 fps with three rays
a pixel (arithmetic from the published bandwidth).

## 2. Frame budget

Target: **60 fps at 1080p output** on the baseline, with the upscaler allowed (internal
resolution between 720p and 1080p). Proposed split of the 16.6 ms, to be rebalanced by
measurement:

| Stage | Budget |
|---|---|
| Streaming, upload, light extraction (CPU-side and copies) | 1.0 ms |
| Primary rays (DDA, full detail + LOD) | 3.0 ms |
| Lighting (path trace 1 spp, next-event estimation, shadow rays) | 6.0 ms |
| Denoise (temporal + spatial) | 3.0 ms |
| Upscale, tone map, post | 1.5 ms |
| Headroom (entities, volumetrics, game work) | 2.1 ms |

**Primary rays, dev reference (not the baseline):** 1.59 ms with brick occupancy masks, 1.99 to
2.01 ms without them. Timestamp queries around the march pass, median of the last 30 of 300
frames, `flythrough --seed 1 --render 1920x1080 --frames 300` from the spawn pose, three runs
each. The pass includes the headlamp's one shadow ray per lit pixel, so it covers more than the
primary-ray line. NVIDIA GeForce RTX 3080 Laptop GPU, Vulkan, driver 610.57.04 (Plan 0001
Phase 8). No baseline reading yet (§1).

**Streaming line, first observation (dev reference, Plan 0001 close, 1d61aba):** the
fly-through's `B` key (100 radius-1 m carves near the camera, then one `GpuWorld::sync`)
uploaded 1.2 to 2.0 MB in 3.9 to 12.9 ms of render-thread CPU over about 20 runs, against
the 1.0 ms line. Part of that is `sync` visiting every chunk; Plan 0002 Phase 3 adds a
dirty-chunk set and a byte budget. Same session, `F1` read the march pass at 3.8 to 8.1 ms at
2530 × 1378 away from the spawn pose: views differ by more than 2× from the spawn reading
above. RTX 3080 Laptop GPU, Ryzen 9 5900HS.

**Streaming line under motion: a recorded miss (dev reference, Plan 0002 Phase 8 at 05334d7
and the close review at ffc73e8, headless `flythrough --autopilot`, unbounded cave seed 1,
r_full 16, r_lod 24, h 1, 15 workers, 2 MiB sync budget, RTX 3080 Laptop GPU, Ryzen 9
5900HS):** render-thread `update + poll + sync` per frame over a 60 s flight at 10 m/s, four
runs: p50 6.94 to 7.13 ms, p99 51.8 to 60.7 ms, max 83.1 to 119.8 ms, against the 1.0 ms line.
The line stands (owner's ruling at the Plan 0002 close, 2026-10-16); a streaming-CPU plan is
the next work against it. Most frames spend it in `sync` narrowing and uploading
about 2 MiB; the p99 frames are the ones whose focus chunk changed (queue rebuild, residency
sweep and window shift over about 58 000 chunks). At rest after full residency a frame's CPU
reads 0.35 ms.

**Primary rays in the streamed world: a recorded miss (dev reference, Plan 0002 Phase 8,
05334d7, reproduced at ffc73e8):** the march pass from the unbounded cave's spawn at
1920 x 1080 reads 6.69 ms at rest after full residency (headless, timestamps, median of the
last 30 of 300 frames), against the 3.0 ms line and against 1.95 ms from the bounded level's
spawn at the same commit. The line stands (owner's ruling, 2026-10-16); a march-pass diagnosis
spike runs before M2's lighting. The autopilot's flight reading (p50 0.46 ms) is not evidence
for the line: its +x path flies through rock. Met at Plan 0015 (2026-10-19): the
traversal-hierarchy paragraph below reads the same view at 1.93 to 1.98 ms.

**Primary rays per surface-look mode (dev reference, Plan 0005 Phase 5, 37b22fa, headless
`flythrough --level --seed 1 --render 1920x1080 --frames 300 --look MODE [--detail]` from the
spawn pose, median of the last 30 of 300 frames, three runs each, NVIDIA GeForce RTX 3080
Laptop GPU; taken while the machine's 1-minute load read between 3 and 5):** face 1.876 to
1.877 ms (1.826 to 1.859 ms without `--look`, so without attribute pages), face with detail
1.976 to 1.995 ms, `Trilinear8` 2.874 to 2.907 ms (with detail 3.020 to 3.051), `Gradient27`
4.564 to 4.612 ms (with detail 4.583 to 4.675), `Contour` 1.927 to 1.944 ms (with detail
2.068 to 2.069). The march pass includes the headlamp's shadow ray. ADR-0011 chooses `Contour`
with detail; on the baseline (about two to three times slower) the level's contour reading
projects to roughly 3.9 to 5.8 ms against the 3.0 ms line, a projection, not a reading.

**Primary rays with the traversal hierarchy (dev reference, Plan 0015 Phase 6, the march of
d3d2f1c, headless, timestamp queries, median of the last 30 frames, three runs each, NVIDIA
GeForce RTX 3080 Laptop GPU, Ryzen 9 5900HS, 1-minute load 7 to 11 from the runs' own
streaming workers and other work on the box):** the streamed cave's spawn
(`flythrough --seed 1 --render 1920x1080 --frames 3000`, full residency after about 12 s)
reads 1.932 to 1.981 ms over six runs, against 6.809 to 6.852 ms for the same view at
52d6a7b before the hierarchy and the 3.0 ms line: the line is met on the dev reference. The
level's spawn (`--level --seed 1 --render 1920x1080 --frames 300`) reads 1.450 to 1.463 ms
against 1.822 ms. Both passes include the headlamp's shadow ray. The autopilot's flight
(`--autopilot --headless --render 1920x1080`, 2830 frames) reads p50 1.569 ms, p99 21.174 ms,
max 32.958 ms; its +x path flies through rock, so its p50 is not evidence for the line, and
its p99 frames are the ones whose chunks are still arriving. The steps view
(`--view steps`, no shadow ray) reads mean 10.98 steps, p99 53, block-max mean 11.67 and
25.99 storage loads per ray at the streamed spawn, against 15.55 steps and p99 84 before; at
the level's spawn 7.49 steps, p99 11, 18.79 loads, against 10.48 and 15. Per phase the
streamed spawn read 6.97 ms with the chunk and group skips alone (106998d), 2.50 ms with the
opaque masks and the plane-order exit (4cafd06), 1.86 ms with the walk's held state
(e564f44) and 1.95 ms with the translucent class (d3d2f1c); the level 1.65, 1.69, 1.34 and
1.45 ms. No baseline reading (§1).

**The default look: contour with detail (dev reference, diagnostic, Plan 0013 Phase 7, the
fly-through's default since that phase, rock detail 4, 0.1; headless, timestamps, median of the
last 30 of 300 frames, three runs each, NVIDIA GeForce RTX 3080 Laptop GPU, Ryzen 9 5900HS,
exclusive perf lock; diagnostic because the 1-minute load read 11.1 to 13.4 from other work on
the box):** the level's spawn (`flythrough --level --headless --seed 1 --render 1920x1080
--frames 300`) reads 1.705 to 1.780 ms; the streamed spawn (`flythrough --headless --seed 1
--render 1920x1080 --frames 300`) reads 3.502 to 3.970 ms, over the 3.0 ms line. Frame 300 is
mid-fill, not full residency: the mirror then holds about 1.87 GB live of the settled 2.86 GB.
`--look plain` keeps the face readings above reproducible.

**The default look at the settled streamed spawn (dev reference, diagnostic, the Plan 0013 close
review, 91a3967, same machine and lock, one run each at a 1-minute load of 12 to 19).** With
`--frames 3000`, as in Plan 0015's reading, residency settles at r_full 12 to 13. The default
reads 2.276 ms and `--look plain` reads 2.134 ms, so contour with detail costs about 0.14 ms
there and is under the 3.0 ms line. With `--frames 300` in the same session, the default reads
3.590 and 3.894 ms against plain's 2.298 ms, and the render thread's median frame CPU is
11.0 to 11.9 ms against 3.3 to 4.5 ms. So the 3.5 to 4.0 ms readings are the fill, not the
settled view.

**The default look on a quiet box (dev reference, 2796062, same machine, exclusive perf lock,
1-minute load 1.67 to 1.98 at each start, three runs each):** the level's spawn reads 1.432,
1.438 and 1.472 ms; the streamed spawn at frame 300 reads 2.864, 2.865 and 2.862 ms, still
mid-fill (1 640 292 full bricks, 1 867 584 964 B live; render-thread frame CPU 7.8 to 8.0 ms).
Both are under the 3.0 ms line; the loaded readings above overstated them by 0.3 and 0.6 to
1.1 ms.

**The attribute pass under motion (dev reference, diagnostic, Plan 0013 Phase 7, same machine
and lock, `flythrough --autopilot --headless --seed 1`, default contour, two runs at load 3.6
to 11.1):** per frame of the flight, the work list reads p50 3584 to 4446, p99 5468 to 5469,
max 7948 to 8051 bricks (1769 and 1776 of 2949 and 2688 frames with work); the pass's GPU time
over the frames with work p50 2.045 to 2.155 ms, p99 4.303 to 4.312 ms, max 4.760 to 4.815 ms,
against the plan's 1.0 ms p99 proposal; the render thread's CPU in `update_attrs` (the work
list, `attr_work`, plus recording the pass) p50 3.689 to 3.834 ms, p99 138.7 to 146.4 ms, max
233.3 to 277.5 ms, on top of the streaming line's `update + poll + sync` (p50 3.7 to 4.0 ms).
Both are over their lines; no plan owns them yet. The close review paired the default with
`--look plain` (91a3967, load 12.2 to 13.6). The default read `update_attrs` CPU p50 3.786 ms,
p99 167.477 ms and max 297.460 ms, with streaming CPU p99 43.2 ms, over 2730 frames. Plain
had no attribute pass and read streaming CPU p99 40.7 ms over 3521 frames in the same 60 s
flight. The attribute work list is a render-thread hitch of 100 ms or more on about 1 % of
the frames in motion, and the default flies about a fifth fewer frames. It is owed before the
lighting plan, beside the streaming-CPU plan.
**Quiet-box re-read (dev reference, 2796062, same machine, exclusive perf lock, each run started
at a 1-minute load of 1.7 to 1.9, the 5-minute load 2.0 to 3.5 from the previous flight; runs
alternated default, plain, default, plain):** the default flew 2211 and 2695 frames against
plain's 4123 and 4155, 35 to 47 % fewer. `update_attrs` CPU p50 4.600 and 4.700 ms, p99
179.598 and 138.104 ms, max 390.376 and 156.825 ms; the work list p50 4773 and 4462, p99 6777
and 5509, max 11 477 and 7555 bricks (1717 and 1802 frames with work); the pass's GPU p50
2.990 and 2.416 ms, p99 5.572 and 4.463 ms, max 9.074 and 5.049 ms. The hitch is not a load
artifact. The followup stays owed before the lighting plan (owner, 2026-10-20).

**Stats HUD (dev reference, Plan 0010, f91726f, headless `flythrough [--level] --seed 1
--render 1920x1080 --frames 300 [--hud]`, three runs each, NVIDIA GeForce RTX 3080 Laptop GPU,
Ryzen 9 5900HS):** a tool, not a frame stage, so it has no line in the table. The HUD pass
reads 0.038 ms at 1080p and scale 2 (median of the last 30 frames), against the plan's 0.1 ms
proposal. With the HUD shown on `--level`, the march reads +0.06 % (1.723 vs 1.722 ms) and the
frame mean +4.0 % (2.34 vs 2.25 ms), which includes the per-frame timer readback. `World::stats`
reads 14.85 ms on the streamed cave at full residency, so the fly-through samples it once per
240 frames while the HUD is shown. The streamed world's readings are logged in the plan, not
gated.

**The cutaway view from outside the level (dev reference, diagnostic, Plan 0025 Phase 2, the
ignored test `cutaway_steps_from_outside_the_level_at_1080p`, `--level --seed 1`, camera 40 m
outside the -x face at the centre looking +x, face normals, 1920x1080, NVIDIA GeForce RTX 3080
Laptop GPU, exclusive perf lock, 1-minute load 8.7, not quiet):** a debug view, off whenever
the camera is in air, so it has no line in the table. The steps view reads a mean of 49.53
steps per ray, p99 170, with 2434 of 2 073 600 rays at the view's 255 clamp; with the cutaway
off the same camera reads a mean of 0.91, p99 1. From 20 m out the cutaway read a mean of 58.74,
p99 201 (load 21.3). A ray through the hull steps uniform bricks eight voxels at a time and the
voxels of full bricks one at a time.

No frame may stall on paging: work attributable to streaming on the render thread is capped at
the streaming line above; the rest runs on background threads.

## 3. Memory

| | Budget |
|---|---|
| World in VRAM at the full-detail radius (about 100 m) plus LOD to about 1 km: live bytes against the reserved budget | 3 GB |
| World in RAM (resident chunks, edit persistence for paged-out chunks) | 4 GB |
| Everything else in VRAM (frame targets, history, entities, upscaler) | 1.5 GB |

The world figure rests on the uniform-brick assumption in `roadmap.md` (under 10 % of bricks
non-uniform at full detail). When exceeded, far chunks drop to LOD-only residency rather than
the frame dropping.

**Readings (dev reference, 2026-10-16, Plan 0001 Phase 4 commit, `flythrough` cave seed 1,
`cargo run --release -p flythrough --example world_budget`):**

| Extent (chunks) | `Full` fraction | CPU bytes | Mirror bytes (no LOD) |
|---|---|---|---|
| 20 × 20 × 8 (the level, 128 × 128 × 51 m) | 23.41 % | 399.6 MiB | 383.8 MiB |
| 32 × 32 × 32 (204.8 m cube) | 20.79 % | 3661.6 MiB | 3496.2 MiB |

With occupancy masks (Plan 0001 Phase 8), the level-size mirror reads 402 449 984 B from the
fly-through's first sync. Both measured `Full` fractions are above the 10 % assumption above; the cave is about 29 %
air with caverns on a 6.4 m lattice.

With 8-bit palette indices (Plan 0002 Phase 4, ADR-0004) the level-size mirror reads
207 681 088 B: pool 196 407 296 B (512 B for each of the 383 608 `Full` bricks, no wide chunk),
masks 8 B per brick, records 2560 B per chunk (512 entries and a 256-entry `u16` palette),
table 4 B per chunk, from `apps/flythrough/tests/level_mirror.rs` on the dev box.

With the traversal hierarchy (Plan 0015 Phases 2 and 3) the same level-size mirror reads
232 436 992 B: pool 196 407 296 B as above, masks 72 B per `Full` brick (27 619 776 B: a
64-byte opaque mask per slot in groups of eight, plus two translucent words), records 2624 B
per chunk (16 words of brick bits, 512 entries, the palette), table 4 B per chunk, from the
same test on the dev box (lavapipe).

**Streamed residency (dev reference, Plan 0002 Phase 8, unbounded cave seed 1 around its spawn,
hysteresis 1, `cargo run --release -p flythrough --example world_budget -- --stream
R_FULL,R_LOD`, 16 workers, Ryzen 9 5900HS).** Mirror bytes at 8-bit palette indices (ADR-0004):
512 + 8 B per `Full` brick or mip-brick, 2560 B per record for full and LOD-only chunks alike,
plus the `W^3` chunk table. (Since Plan 0015 Phase 3 a slot carries 512 + 72 B and a record
2624 B; the example counts the new figures, the table below keeps the readings taken at the
old ones, and the paragraph after it holds Plan 0015's.) CPU bytes: full chunks' slot arrays and `Full` bricks, LOD-only
chunks' mip-brick slots and `Full` mip-bricks. In every row about 18.5 to 18.8 % of a full
chunk's bricks and about 39 % of the ring's mip-bricks are `Full`.

| r_full | r_lod | Full chunks | LOD-only chunks | CPU MiB | Mirror MiB (full) | Mirror MiB (LOD) | Mirror MiB (total) |
|---|---|---|---|---|---|---|---|
| 10 | 14 | 4 169 | 7 344 | 634.2 | 205.7 | 123.4 | 329.2 |
| 10 | 18 | 4 169 | 20 236 | 1 006.2 | 205.7 | 336.0 | 541.9 |
| 12 | 16 | 7 153 | 9 924 | 1 012.7 | 355.9 | 165.5 | 521.5 |
| 12 | 20 | 7 153 | 26 248 | 1 486.2 | 355.9 | 436.0 | 792.2 |
| 16 | 20 | 17 077 | 16 324 | 2 210.0 | 855.1 | 270.5 | 1 126.0 |
| 16 | 24 | 17 077 | 40 700 | 2 915.9 | 855.1 | 673.9 | 1 529.5 |

**With the traversal hierarchy (dev reference, Plan 0015 Phase 6, d3d2f1c, headless, NVIDIA
GeForce RTX 3080 Laptop GPU, Ryzen 9 5900HS):** mirror bytes at 512 + 72 B per `Full` brick
or mip-brick and 2624 B per record. The level (`--level --seed 1`) holds 232 436 992 B live
and 238 303 552 B allocated (207 886 080 and 213 662 016 B at 106998d, before the masks grew).
The streamed defaults at rest after full residency (`--seed 1 --frames 3000`, 17 077 full and
40 700 LOD-only chunks) hold 1 786 641 644 B live and 1 865 490 412 B allocated
(1 607 517 932 and 1 679 368 172 B at 106998d). The autopilot's flight
(`--autopilot --headless --render 1920x1080`) peaks at 1 889 760 188 B live and
2 714 672 620 B allocated, against Plan 0002's 1 672 823 572 B live and 2 431 425 836 B
allocated: about 217 MB more live and 283 MB more allocated, under the 3 GB line; the
residency latency wanted-to-mirrored reads p95 493 ms, max 1804 ms over 72 849 chunks
(p95 472 ms, max 2362 ms at e564f44), against the 500 ms proposal.

**Under motion (dev reference, Plan 0002 fix pass, 076b1c4, headless `flythrough --autopilot`,
unbounded cave seed 1, r_full 16, r_lod 24, h 1, 15 workers, RTX 3080 Laptop GPU, NVIDIA
610.57.04, Ryzen 9 5900HS, two runs):**

- **World VRAM:** the mirror peaked at 1 672 823 572 and 1 692 743 692 B by live accounting
  (`GpuWorld::mirror_bytes`) and at 2 431 425 836 B allocated (`GpuWorld::allocated_bytes`,
  both runs), against the 3 GB line, then read on allocated bytes (buffers grew by half
  again and never shrank). At full residency, 1 603 820 012 B live and 1 675 304 748 B
  allocated. Since Plan 0017 (ADR-0013) the line is read on live bytes against the reserved
  budget; the readings below replace these allocated figures.
- **World RAM:** resident chunks account for 3 242 091 312 and 3 239 741 232 B at peak (slot
  arrays, `Full` bricks and mips), and persisted RAM for 233 616 B. The process peak RSS
  (`VmHWM`, the whole fly-through, dev reference) reads 3 869 564 928 and 3 873 890 304 B,
  about 3.87 GB against the 4 GB line. RAM is the tightest margin at these defaults; they are
  revisited on the baseline card at M2.
  Since Plan 0021 (ADR-0012's packed brick, closed 2026-10-20) the reading is 0.44 GB of
  resident chunks and about 0.97 to 1.01 GB `VmHWM` on the dev box ("The packed brick" below);
  it replaces these figures, and the 4 GB line holds with room.

**Attribute pages (dev reference, Plan 0005, RTX 3080 Laptop GPU).** The contour attribute
(`u16` per voxel) lives in attribute pages of the pool: 1040 B more live per `Full` brick of a
full chunk, and a page holds 1364 bricks in 4096 slots. At the level (`flythrough --level
--headless --seed 1 --frames 1`, 6d77f12): 207 681 088 B live and 213 334 144 B allocated
without attribute pages; 606 633 408 B live and 613 759 104 B allocated with them
(`--look contour`), a live rise of exactly 1040 x 383 608 B. 128 B more with 64-byte material
rows at 37b22fa. Streamed (`flythrough --autopilot --headless --look contour`, 37b22fa, r_full
16, r_lod 24, h 1, 15 workers): at full residency 3 309 723 820 B live and 4 103 413 164 B
allocated, over the 3 GB line, against Plan 0002's 1 675 304 748 B allocated in face mode; the
run then stopped in its outbound flight on `wgpu error: Out of Memory` on the 8 GiB card, so
no peak was read. A recorded miss: ADR-0011 keeps contour planes, and fitting this footprint
under the 3 GB line is owed to the surface follow-up plan (Plan 0005 follow-up 2).
Since Plan 0015's masks an attribute-page brick costs 1168 B more live (two 512 B attribute
slots and their two 72 B mask entries). At the level (`flythrough --level --headless --seed 1
--look contour --frames 3`, Plan 0013 Phase 5, RTX 3080 Laptop GPU, exclusive perf lock,
1-minute load 3.47): 680 491 136 B live, the face level's 232 436 992 B plus 1168 x 383 608 =
448 054 144 B, under a 797 779 456 B reservation. The layout is unchanged; surface-only
attribute storage is ADR-0012's mirror-slot plan.

**The reserved budget (dev reference, Plan 0017 Phase 4, ef37395, headless `flythrough
--autopilot --headless`, unbounded cave seed 1, h 1, 15 workers, RTX 3080 Laptop GPU, Ryzen 9
5900HS, exclusive perf lock, 1-minute load 10.6 to 23.2 from other builds on the box, two runs
each).** The mirror reserves its pool, masks and records once at startup (`--mirror-budget`,
default 3072 MiB) and never reallocates; `allocated_bytes` is the budget rounded down to whole
pages and records. Records are reserved for the residency's ball over 0.7 (93 239 at 16/24,
244 659 136 B; 72 690 at 14/22) instead of the window cube. Pressure is the larger of pool-slot
and record occupancy; the fly-through lowers `r_full` a chunk after 30 frames above 0.9 and
restores it after 300 frames under 0.7, down to 6. A sync writes the nearest chunks that
fit and defers the rest, which stay queued (ADR-0013 Outcome). No run stopped on
out-of-memory; every return hash matched. The face rows, every column, are at ef37395; the
contour rows, every column including full residency, were re-taken at b474737 (the partial
sync with its per-chunk check), at load 13.6 to 26.0.

| run | split (slots, records) | full residency live | flight peak live / budget | peak pressure (slots, records) | radius changes, final r_full | deferrals: syncs (empty), chunks, longest streak | reserve, first frame |
|---|---|---|---|---|---|---|---|
| 16/24 face | 5 095 424, 93 239 | 1 786 641 644 B, 0.620 | 0.5844, 0.5667 | 0.6473 (0.5803, 0.6473); 0.6200 (0.5643, 0.6200) | none, 16 | none | 83.1, 87.1 ms; 2669 (cold), 554 ms |
| 16/24 contour | 5 095 424, 93 239 | 2 864 309 692 B at r_full 13, 0.911 (both runs) | 0.8894, 0.8894 | 0.9114 (0.9114, 0.6197); 0.9114 (0.9114, 0.6197) | 4 each (16 to 12), ends 12 | 1 (0), 1605, 1; none | 100.6, 96.2 ms; 2330 (cold), 857 ms |
| 14/22 face | 5 185 536, 72 690 | 1 312 177 076 B, 0.612 | 0.4339, 0.4179 | 0.6469 (0.4203, 0.6469); 0.6118 (0.4063, 0.6118) | none, 14 | none | 82.2, 97.9 ms; 564, 1105 ms |
| 14/22 contour | 5 185 536, 72 690 | 2 607 896 708 B, 0.822 (both runs) | 0.8747, 0.9198 | 0.9020 (0.9020, 0.6118); 0.9442 (0.9442, 0.6327) | none, ends 14; 1 (14 to 13), ends 13 | none; 15 (0), 19 801, 3 | 105.4, 197.3 ms; 673, 1030 ms |

Contour fits the 3 GB line at both radii only by the rule: at 16/24 it settles at r_full 12
(11 513 full, 46 264 LOD-only chunks at full residency, r_full 13), at 14/22 it stays at 14 or
drops to 13 in the flight. With attribute pages the pool runs out of pages while slots are
spare in plain pages that each hold a live mip-brick. Those syncs upload what fits and defer
the rest; no sync was empty and every deferral streak cleared within 3 syncs (the bar is 90).
The one 16/24 deferral came at pressure 0.796, under the 0.9 trigger, and cleared in its own
sync with no radius change; the 14/22 deferrals came at 0.855 to 0.944, after the decrease to
13 (tick 796, first deferral at tick 807). Two of the four runs deferred nothing. A recorded
finding, not a miss of the line: live bytes peak at 2.96 GB under 3 GB. At 52cf84d, before the
per-chunk check, the same runs read longest streaks of 1, 3, 2 and 6 syncs. These readings stand
at the close tip (16e17a7, v0.17.0): the later fixes change only the wide path (more than 256
materials) and syncs that write nothing, neither of which these runs reach. Deferring under
the 0.9 trigger with attribute pages is page fragmentation, Plan 0017's open followup.

**The default look (dev reference, Plan 0013 Phase 7, `flythrough --autopilot --headless
--seed 1` with no `--look`, so contour with detail and attribute pages, r_full 16, r_lod 24, h 1,
the 3072 MiB default reservation, same machine, exclusive perf lock, 1-minute load 3.6 to
11.1, two runs).** Reserved 3 220 387 776 B (5 095 424 slots, 93 239 records). Both runs:
`r_full` 16 to 15 to 14 to 13 at pressure 0.960, 0.982, 0.918 (ticks 469, 499, 664), full
residency at r_full 13 with 2 864 309 692 B live, pressure 0.911, then 13 to 12 at 0.911
(tick 905) and back to 13 at 0.669 and 0.651 (ticks 2444, 2088), ending at 13; lowest 12.
Peak live 2 864 318 452 B, peak live / budget 0.8894, peak pressure 0.9114 (slots 0.9114,
records 0.6513 and 0.6495). Run 1 deferred nothing; run 2 deferred 5032 chunks over 10 syncs
at pressure 0.755 to 0.763, none empty, longest streak 3. Both returned `return: 2 chunks,
hash match` with no wgpu error. The default look therefore holds full detail to the radius
the rule settles at, 12 to 13 chunks (76.8 to 83.2 m), against this section's "about 100 m",
until ADR-0012's mirror-slot plan stores the attribute for surface voxels only; `--look
plain` holds 16 (102 m).

**The pressure rule's gate (Plan 0017 Phase 3, same machine and flags, r_full 16, r_lod 24,
no `--look`, `--mirror-budget 1930`: 3 043 328 slots and 93 239 records, 2 021 964 224 B,
reserved in 63.0 and 70.7 ms).** Run 1 (06d1f17, load 24.1): `r_full` 16 to 15 to 14 at
pressure 0.920 right after full residency, back to 15 at 0.676 in the flight; run 2 (ef37395,
load 18.3): 16 to 15 at 0.920, back to 16 at 0.688. Both: 0 syncs past the reservation, peak
pressure 0.9197 (slots 0.9197, records 0.6227 in run 2), peak live / budget 0.8836, return hash
match, `r_full` never under 14. Run 3 (b474737, partial sync with its per-chunk check, load 19.8 to
14.9): 16 to 15 to 14 at 0.920, back to 15 at 0.604; 0 deferred chunks, peak pressure 0.9197
(slots 0.9197, records 0.6488), peak live / budget 0.8836, hash match. At 3072 MiB the same
flight makes no change (peak pressure 0.6363 and 0.6473, 0.6200).

**Brick-array pool (Plan 0014, retired by Plan 0021).** The pool added at most 8 MiB per world (8192 arrays x 1 KiB); the fly-through held a 4096-array reserve (2326 KiB pooled after the `B` bench, 5850218, dev reference). Plan 0021 retired it: a `Full` brick's box is 72 to 352 B and is allocated on the edit path (11.2 ns per expansion; below).

**The packed brick (dev reference, Plan 0021 Phase 8, ADR-0012, Ryzen 9 5900HS, RTX 3080
Laptop GPU, 8440aa9 with Phase 8's two printouts, exclusive perf lock, 1-minute load 8.4 to
14.2 from other lanes' builds).** A `Full` brick is a mask, an ascending palette and bit
planes in one of five classes (72, 136, 200, 352 or 1024 B by material count), behind the
same 16-byte slot. The autopilot (`flythrough --autopilot --headless --render 1920x1080`,
streamed defaults, alternated with main at 5ca4151 in the same window): **resident chunks peak
at 441 596 664 B and the process `VmHWM` at 965 971 968 B**, against 3 242 091 312 B and
3 886 219 264 B for main (Plan 0002's 3.24 GB and 3.87 GB), under the 4 GB line; a second
Phase 8 run read 441 555 208 B and 1 009 381 376 B. `world_budget` CPU bytes: the level
60 807 552 B (58.0 MiB, against 399.6 MiB in the table above), the streamed defaults
415 231 624 B (396.0 MiB: full 274 067 464, LOD 141 164 160, against 2915.9 MiB). The census
at rest (`--census`): the level's 383 608 `Full` bricks hold 1 material (275 466), 2 (107 325)
or 3 to 4 (817); the streamed defaults' 1 640 292 `Full` bricks and 1 158 516 `Full`
mip-bricks hold 1 or 2. Mirror bytes are unchanged by construction: the level reads
232 436 992 B live.


**Stats HUD (arithmetic, Plan 0010; ADR-0009).** The overlay holds about 130 KiB of VRAM
under "everything else": a 128 KiB cell buffer at the 256 x 128 cap layout (`u32` per cell),
a 1 KiB sample buffer (256 `f32`), the font and a small parameter block, under the 200 KiB ADR-0009 states.

**Prefabs (arithmetic, Plan 0008; no reading needed).** A `Prefab` holds 2 B of material plus
one mask bit per voxel: a 16³ prefab is 8 KiB + 512 B, a 64³ one 512 KiB + 32 KiB. A parsed
`.vox` model is capped at 256³ per axis (`MAX_MODEL_EDGE`), so the largest `to_prefab`
allocation is about 50 MiB including its index buffer. `StampedSource` holds one rotated copy
of the prefab per placement, so its RAM grows with the placement count, not the prefab count.

## 4. Edits

- **Cost:** 100 sphere carves of radius 1 m (exactly 4169 voxels each at 10 cm, the lattice
  count on voxel centres; the continuous volume is about 4189) in one frame cost at most 2.2 ms
  of CPU on the dev reference (raised from 2 ms by the owner on 2026-10-20 after the quiet-box
  re-read below; one more optimization pass is planned) including result counting and dirty marking, and their upload (up to about 6400 bricks,
  6.4 MB, if every touched brick is non-uniform) fits in the streaming line of §2.
  **Readings (dev reference, AMD Ryzen 9 5900HS, Plan 0001):** `cargo bench -p vxe-world
  --bench edits` (100 radius-10 carves on a fresh solid world, every carve expanding uniform
  bricks, the worst case): 4.92 ms (CI 4.87 to 4.98 ms), Phase 4 commit. In the fly-through
  (`B` key, carves into an already dug cave, 1d61aba): 3.07 to 4.63 ms over about 20 runs.
  Both miss the 2 ms budget; the budget stands (owner's call at the Plan 0001 close) and the
  edit cost gets its own plan after Plan 0002.
  **Reading (dev reference, AMD Ryzen 9 5900HS, Plan 0003, 96afb82):** the same `worst` case
  reads 3.656 ms (CI 3.641 to 3.672 ms) with the dev box under outside load (load average
  about 10), against 4.548 to 4.641 ms for the Plan 0001 edit path run alternately in the same
  window; the fly-through `B` line (`--level --seed 1`) reads 3.33 to 3.41 ms against 4.42 to
  4.46 ms. **Quiet-box reading at the Plan 0003 close** (a958ec1, same `vxe-world` tree,
  exclusive perf lock, 1-minute load under 3.5 and falling, nothing else running): `worst`
  3.556 to 3.566 ms (CI 3.552 to 3.572 ms over two runs), `mixed` 2.586 to 2.596 ms, `tool`
  8.119 to 8.120 ms; the fly-through `B` line 3.30 to 3.34 ms. It misses the 2 ms budget; the
  budget stands (owner's call at the Plan 0003 close) and Plan 0014 is the second pass.
  **Quiet-box reading at the Plan 0014 tip (dev reference, AMD Ryzen 9 5900HS, 5850218,
  exclusive perf lock, 1-minute load 1.93 before and 1.50 after, paired and alternated with
  the Plan 0003 tip 2129bfb):** `worst_warm` (the `worst` case after
  `World::reserve_brick_arrays`) 3.034 to 3.054 ms (CI 3.031 to 3.057 ms over two runs);
  `worst` 3.189 to 3.264 ms (CI 3.187 to 3.269 ms), against 3.549 ms for the Plan 0003 tip
  in the same window; `mixed` 2.326 to 2.355 ms; `tool` 6.777 to 6.798 ms, `tool_warm` 6.660
  to 6.688 ms. The fly-through `B` line (`--level --seed 1 --headless --bench 1`) reads 2.98
  to 3.04 ms against 3.31 to 3.32 ms, three runs each. The warm figure assumes the reserve
  runs off the edit path: the caller tops the pool up between frames (the fly-through does,
  after each frame's edits), so a frame's expansions take pooled arrays instead of
  allocating. Both readings miss the 2 ms budget; the 2 ms figure is unchanged, and the
  remaining cost is the row write bounded by the brick representation (Plan 0014's close). The decision awaits the owner (a representation ADR versus a measured cap).
  **Reading at the Plan 0021 tip (dev reference, AMD Ryzen 9 5900HS, ADR-0012's planar edit
  kernel, exclusive perf lock, not quiet):** `worst_warm` 1.339 ms against 2.811 ms for the
  Plan 0021 Phase 1 tip 992fdaa alternated in the same window (load 17.6; 1.485 against
  2.864 ms at load 24.1); `worst` 1.356 against 3.527 ms. At the Phase 8 tree (8440aa9, load
  10.4) `worst_warm` 1.516 ms and `worst` 1.538 ms; with the pool gone both cases build the
  same world. The fly-through `B` line (`--level --seed 1 --headless --bench 1 --frames 1`)
  reads 1.52 ms against Plan 0014's 2.98 to 3.04 ms. **Under the 2 ms line on the dev box**; no
  baseline-GPU reading. The owner held the 2 ms figure on the dev box (2026-10-20), so the
  line is **met on the dev box**. The allocation share the retired pool covered: 2495
  expansions of 11.2 ns, 27.9 us, 1.8 % of the 1.516 ms `worst_warm`, under the 5 % that would
  have called for per-class pools.
  **Quiet-box re-read (dev reference, AMD Ryzen 9 5900HS, on AC with the performance profile
  and governor, 2796062, exclusive perf lock, 1-minute load 1.81 at the start):** `worst`
  1.793 ms, `worst_warm` 1.794 ms, `mixed` 1.417 ms, `tool` 5.823 ms, `tool_warm` 5.821 ms.
  Paired with 8440aa9 (the Plan 0021 Phase 8 tree; `vxe-world` is unchanged since),
  alternated twice at load 1.76 to 2.27: `worst_warm` 1.785 and 1.789 ms on main against
  1.783 and 1.783 ms, so the gap to the 1.34 to 1.52 ms above is machine state, not code, and
  is unexplained. The fly-through `B` line reads **2.06, 2.07 and 2.09 ms**, over 2 ms and
  under **2.2 ms, the line the owner set on 2026-10-20**. One more optimization pass is
  planned; no baseline-GPU reading.
- **Results** are returned synchronously with the edit, per material, exact.
- **Stamp cost (proposals, Plan 0008):** a 16³ prefab stamp (the shelled box `Prefab::vault`)
  into solid rock costs under 100 µs, and a 64³ all-written prefab under 5 ms, both including
  the `EditResult` tally. **Readings (dev reference, AMD Ryzen 9 5900HS, `cargo bench -p
  vxe-world --bench stamp`, exclusive perf lock, box under outside load of about 2 to 5,
  commits 83ab811 and 39d3700):** `vault16` 22.6 to 22.8 µs; `prefab64_mixed` 2.02 to 2.07 ms;
  `prefab64_uniform` 1.455 ms. Both under their proposals; the proposals stand as budgets.
- **Collapse cost (proposals, Plan 0009):** `find_unsupported` after a radius-10 carve at the
  centre of a solid 4³-chunk world (every flood runs to the cap) under 1 ms; `drop_region` of
  the 9-cell test slab under 100 µs; `remove_region` of that slab with rubble 1/4 under
  200 µs; `drop_region` of a 4096-cell cube (2 097 152 voxels) under 20 ms (a guess at the
  proposal). **Readings (dev reference, AMD Ryzen 9 5900HS, Linux, `cargo bench -p vxe-world
  --bench collapse`, criterion medians, exclusive perf lock; at b0d5565 with 1-minute load
  4.57 falling to 2.18, and at 09556ed after the brick-array pool merge with load 11.75 falling
  to 4.18):** `find_unsupported` 327 to 396 µs; slab drop by 32 72 µs and by 10 (not a
  multiple of the brick edge) 49 to 50 µs; `remove_region` 119 to 127 µs; cube drop by 64
  6.8 to 6.9 ms and by 60 8.1 to 8.6 ms; cube drop by 8 5.15 ms (no proposal). In the
  fly-through, the query after a 0.5 m dig on the seed-1 level reads 492 to 619 µs over six
  digs (release build, 25b483d; no proposal). Every reading is under its proposal; the
  proposals stand as budgets.
- **Logs (Plan 0007, Plan 0018):** an op log entry is 47 bytes for a sphere carve (65 for a box
  fill, 75 for a capsule replace), 34 for a stamp (whatever the prefab's size: it names the
  prefab by content id, ADR-0014), `21 + 12n` for a drop of `n` cells and `31 + 12n` for a
  removal (49 173 bytes for a 4 096-cell drop); a boundary entry 13, an input entry 14 bytes
  plus its payload. A log file adds a header of 60 fixed bytes, a 4-byte prefab count, and
  per prefab its stamps name an 8-byte id and its content (`12 + 2n` bytes for `n` voxels plus
  8 per 64 voxels of mask: 8 716 for the 16³ vault), once per prefab however often it is
  stamped. A falling region re-sends its cells every tick it drops (129 bytes a tick for a
  9-cell slab). All asserted in `crates/vxe-world/tests/log.rs`.
- **Replay throughput:** no budget; a reading only, bound by the edit cost above (the ops
  dominate, not the decode). **Reading (dev reference, AMD Ryzen 9 5900HS, Plan
  0007 Phase 6, exclusive perf lock, 1-minute load 1.45):** `cargo bench -p vxe-world --bench
  replay`, decode plus replay of a 470 060-byte log file of 10 000 radius-1 m carves on a
  solid 4³-chunk world: 279.07 ms (CI 278.48 to 279.70 ms), about 35 800 ops per second.
- **Tick overhead:** 100 inputs through one server tick and one shared-client poll cost at
  most 100 µs more than applying the same 100 ops directly (budget since the Plan 0007 close,
  backed by the reading below).
  **Reading (dev reference, AMD Ryzen 9 5900HS, Plan 0007 Phase 6, exclusive perf lock,
  1-minute load 1.66):** `cargo bench -p vxe-net --bench tick`: 3.4706 ms (CI 3.4663 to
  3.4756 ms) through `EchoLogic` against 3.3859 ms (CI 3.3782 to 3.3990 ms) direct, an
  overhead of 84.7 µs per 100 inputs.
  **Reading with the server's coarse grid (dev reference, AMD Ryzen 9 5900HS, Linux, Plan
  0018 Phase 5, a9014d0 plus the phase's bench arm, exclusive perf lock but not quiet: other
  worktrees building and testing, 1-minute load 14.46 falling to 6.50):** the server now
  updates its coarse grid after every applied op, which is edit work, so the overhead is read
  against the direct carves each followed by the same grid update. Three rounds, arms
  alternated, criterion medians: through `EchoLogic` 4.3895, 3.9923 and 3.8381 ms; direct
  carves with grid updates 4.3400, 3.9843 and 4.0753 ms; direct carves alone 2.8636, 2.4967
  and 2.8504 ms. Overhead per 100 inputs: 49.5 µs, 8.0 µs and -237.2 µs (the last round's
  noise exceeds the difference). Under the 100 µs line in every round; the readings are noisy
  and want a quiet-box re-read.
  **Close-review re-read (Plan 0018, c1ea275, same machine, exclusive perf lock, still not
  quiet: 1-minute load 20.82 at the start and 13.60 at the end):** one round, through
  `EchoLogic` 3.7783 ms (CI 3.7630 to 3.7943 ms), direct carves with grid updates 3.7716 ms (CI
  3.7528 to 3.7914 ms), direct carves alone 2.4188 ms. The overhead is 6.7 µs per 100 inputs,
  inside the overlapping intervals and consistent with the line.
  **Quiet-box reading (2796062, same machine, exclusive perf lock, 1-minute load 1.36 at the
  start and 1.29 at the end):** through `EchoLogic` 2.0012 ms (CI 1.9990 to 2.0037 ms), direct
  carves with grid updates 1.9072 ms (CI 1.9055 to 1.9089 ms), direct carves alone 1.7929 ms.
  The overhead is **94.0 µs per 100 inputs**, under the 100 µs line with 6 µs to spare; the
  loaded readings above understated it.
- **Component boundary (budgets since the Plan 0011 close):** a host call from a logic
  component (the `apply` import of a radius-1 m carve: the op's wire bytes out, the tagged
  answer back) costs at most 2 µs more than a native `Host::apply`; the fly-through's `Bench`
  tick (100 carves) through the component costs under 2× native `DigLogic`. The per-call
  proposal was 1 µs, and the readings below missed it at 1.37 to 1.42 µs; the 2 µs budget was
  adopted at the close (an orchestrator call, overturnable by the owner). The empty component
  tick (1.1 to 1.25 µs) is a reading, not a budget. Host-call work is charged to fuel at the
  configured rates (Plan 0019): before any host work, `apply` and `unsupported` are charged
  `WorkRates::per_byte` (default 1) for each byte of their list argument (the op bytes; 12 per
  changed cell); after each import returns, the tick's fuel loses `per_call` (100), plus
  `per_voxel` (10) for each voxel an `apply`'s op bounds hold and 64 per chunk it touched,
  `per_step` (10) for each step of a `raycast`'s walk and for each cell the `unsupported`
  query looked at, and `per_voxel` for each voxel a `count_in_box` counted. A refused or
  undecodable call is charged its bytes and `per_call` only, and a charge past the fuel left
  is the same `Fuel` failure as an instruction overrun. The server's support query reads a
  `changed` list longer than one in-range edit can change (`(edit_axis_max / 8 + 1)^3`,
  2 146 689 cells at the defaults) as no regions. **Asserted:** a `count_in_box` over
  `1 024^3` resident voxels (`10 x 2^30` fuel) fails the tick on fuel at the default `10^10`;
  100 rays of `max_t = 2^22` through an empty world are charged exactly `100 x (100 + 10 s)`;
  an undecodable 4 096-byte `apply` costs `100 + 4 096`; `unsupported` over `K` cells costs
  `100 + 12 K + 10 s`. A cheap query's per-call cost is not read yet.
  **Readings (dev reference, AMD Ryzen 9 5900HS, Linux, `wasmtime` 49.0.3, Plan 0011 Phase 4
  at 07a3be1 (70b40ec plus the bench), exclusive perf lock, 1-minute load 1.87 before and 1.66
  after two runs):** `cargo bench -p vxe-script-wasm --bench boundary`, each tick on a fresh
  `solid3_world()` through a host straight on the world: 100 carves through the echo
  component 3.1181 and 3.1225 ms against native `EchoLogic` 2.9765 and 2.9858 ms, so 1.42 and
  1.37 µs per host call, the tick's entry and the inputs' lowering included (an empty
  component tick reads 1.12 and 1.25 µs). The per-call figure misses the 1 µs proposal. The
  `Bench` tick through the `flythrough-logic` component 3.1984 and 3.2065 ms against native
  `DigLogic` 3.1033 and 3.1025 ms: 1.031× and 1.034×, under the 2× proposal. Re-run after the
  close review's fix pass (host imports under `catch_unwind`): 1.38 µs per call, empty tick
  1.125 µs, `Bench` 1.033×.
  **Restart after a failure (Plan 0016 Phase 2):** a failed call is followed on the next tick
  by a fresh instance and `init`, at most `WasmConfig::max_restarts` times (default 8) per
  session, then the logic stops. One restart of the `flythrough-logic` component
  (`WasmLogic::try_init`: store, instantiation from the pre-linked component, `init`) read
  a median of 26.9 and 29.3 µs over 50 restarts each (min 24.5, max 109.0 µs): a one-off
  release-build timing loop, not a committed bench, AMD Ryzen 9 5900HS, Linux, not quiet
  (load average 7.4 to 9.4), at 1d1b3aa plus Phase 2's working tree.
  **Re-read with host-call charging (Plan 0019 Phase 2, AMD Ryzen 9 5900HS, Linux, `wasmtime`
  49.0.3, exclusive perf lock, not quiet), paired with its parent:** parent c04bfd1 (1-minute
  load 3.94 to 4.66): 100 carves through the echo component 2.4928 ms against native
  `EchoLogic` 2.3589 ms, 1.339 µs per host call, empty component tick 1.092 µs, `Bench`
  1.028×. With charging, 3445f30 (load 7.15 to 4.58): 2.4593 ms against 2.3306 ms, 1.287 µs
  per host call, empty tick 0.974 µs, `Bench` 2.5510 against 2.4014 ms, 1.062×. The 2 µs
  line holds; the charge's fuel read and write is inside the run-to-run noise.
  **Re-read with the per-byte charge (Plan 0019 close review, AMD Ryzen 9 5900HS, Linux,
  exclusive perf lock, not quiet, paired):** parent 40d2b91 (1-minute load 8.17 to 8.26):
  2.5902 ms against native 2.5167 ms, 0.74 µs per host call, empty tick 1.269 µs, `Bench`
  2.9585 against 2.5515 ms, 1.160×. With `per_byte`, adfca8b (load 8.26 to 6.79): 2.7319 ms
  against 2.7332 ms, under 0.1 µs per host call (the native arm read slower than the
  component), empty tick 1.463 µs, `Bench` 2.6337 against 2.5813 ms, 1.020×. The load moved
  the native arm by 0.22 ms between runs, more than the whole host-call cost, so this pair
  resolves no difference the per-byte charge makes; the 2 µs line and the 2× `Bench` line hold
  in both.
  **Quiet-box reading (2796062, same machine, exclusive perf lock, 1-minute load 1.29 at the
  start and 1.41 at the end, one run, not paired with a parent):** 100 carves through the echo
  component 1.8005 ms against native `EchoLogic` 1.6445 ms, **1.56 µs per host call**; empty
  component tick 1.007 µs; `Bench` 1.8380 against 1.7269 ms, **1.064×**. Both lines hold.
- **Parameter range (Plan 0019):** every edit, ray, box query and region op is checked
  against the world's `WorldLimits` before any arithmetic and refused with
  `EditError::OutOfRange` outside it, changing nothing. Defaults: coordinates and radii within
  `±2^30` fixed units (±4 M voxels); a capsule's `b - a` within `2^30` per axis (the bound
  that keeps its `i128` products exact); an edit's voxel bounds at most 1 024 per axis (at
  most 4 913 chunk coordinates walked); a ray's reach `|dir[k]| x max_t / 256` at most `2^22`
  fixed units per axis (at most 258 cell steps along an axis through empty chunks, 772 on a
  diagonal); a box query at most 1 024 voxels per axis and `2^30` voxels; a drop or removal
  at most 4 096 cells. A world built `with_limits` takes its own; a server and an owned client
  hold the same limits or refuse different ops.
- **Determinism:** the same op list on the same seed yields bit-identical voxels and results on
  Linux and Windows. CI asserts a fixed vector's hash on both arms. **Asserted:**
  `determinism_vector` passed on the Linux and Windows CI arms at 9565c5a and 1d61aba
  (Plan 0001), with the hash blessed on Linux.

## 5. Streaming

- Chunk generation and LOD building run on background threads; the render thread only swaps
  pointers and issues uploads.
- Moving the focus point at 10 m/s keeps the full-detail radius resident; a chunk entering the
  radius is resident within 500 ms (proposal). At a 100 m radius that is about 1200 chunks per
  second, roughly 300 M voxels per second across the baseline's four cores, so generation
  throughput per core is a measured figure from Plan 0002 on and the radius default follows it.
  **Reading (dev reference, AMD Ryzen 9 5900HS, 16 threads, 2026-10-16, Plan 0001 Phase 4
  commit):** the `flythrough` cave generates at 1.03e8 voxels per second per thread at
  20 × 20 × 8 chunks (0.509 s) and 1.12e8 at 32 × 32 × 32 (4.785 s), bounded and synchronous,
  from `world_budget`.
- **Residency latency (dev reference, Plan 0002 Phase 8, 05334d7, and the close review at
  ffc73e8, headless autopilot as below):** wanted-to-mirrored p95 431 to 481 ms over four
  runs, under the 500 ms proposal; max 1678 to 3669 ms, over it. The proposal stands as a p95
  figure; the tail is the streaming-CPU plan's.
- An edited chunk that is evicted and later reloaded reproduces the edit.
  **Asserted** headlessly by `crates/vxe-world/tests/persist.rs` (RAM, spill file, racing
  eviction) and under motion by `flythrough --autopilot` (`return: 2 chunks, hash match`
  after flying 300 m out and back), Plan 0002 Phase 8.
- **Reading under motion (dev reference, Plan 0002 Phase 8, headless `flythrough
  --autopilot`, unbounded cave seed 1, r_full 16, r_lod 24, 15 workers, RTX 3080 Laptop GPU,
  Ryzen 9 5900HS):** at 10 m/s, 0 frames out of 2775 and 2772 had a chunk wanted full within
  14 chunks of the camera's not yet resident and mirrored. Wanted-to-mirrored latency of the
  73 000 chunks loaded full: p95 462 and 455 ms, max 1779 and 2359 ms. Generation under
  streaming: 6.95e7 voxels/s/worker over the flight (both tiers, 15 workers beside the render
  thread), against 1.23e8 to 1.34e8 headless in `world_budget --stream`.
- **With the packed brick (dev reference, Plan 0021 Phase 8, 8440aa9 with Phase 8's two
  printouts, Ryzen 9 5900HS, RTX 3080 Laptop GPU, exclusive perf lock, not quiet, alternated
  with main at 5ca4151):** generation encodes each `Full` brick. `world_budget --chunks
  20,20,8` reads 1.254e8 voxels/s/thread (load 10.1) and `--stream 16,24` 1.344e8
  voxels/s/worker (load 10.1), beside Plan 0001's 1.03e8 and 1.12e8. The autopilot at load
  8.4 to 13.2: residency latency wanted-to-mirrored p95 611 ms, max 2449 ms over 72 614 chunks
  against main's 1031 ms and 6523 ms in the same window, both over the 500 ms proposal at this
  load (Plan 0015's quiet reading was 493 ms; the quiet-box re-read is below); streaming CPU per frame p50 5.352 ms, p99
  64.449 ms against 7.575 and 80.785 ms; march GPU p50 1.745 ms against 1.576 ms (Plan 0015's
  spawn march 1.95 ms). A second Phase 8 run (load 10.2 to 14.2) read p95 658 ms, max 2456 ms;
  CPU p50 5.227, p99 63.362 ms; march p50 1.636 ms. The per-chunk mip build, which decodes
  each `Full` brick, reads 0.202 ms per chunk on one thread over the level's 3200 chunks (no
  proposal).
- **Quiet-box re-read under motion (dev reference, 2796062, Ryzen 9 5900HS, RTX 3080 Laptop
  GPU, exclusive perf lock, `flythrough --autopilot --headless --seed 1`, each run started at
  a 1-minute load of 1.7 to 1.9, alternated with `--look plain`):** with `--look plain` (face
  mode, `r_full` 16 throughout) the residency latency p95 reads **381 and 381 ms**, max 574
  and 518 ms over 73 193 and 73 124 chunks, under the 500 ms proposal; streaming CPU per frame
  p50 4.993 and 4.966 ms, p99 32.840 and 32.640 ms. With the default contour look, p95 reads
  904 and 270 ms, max 6014 and 2017 ms over 45 537 and 46 470 chunks; streaming CPU p50 4.740
  and 4.505 ms, p99 65.300 and 35.261 ms. The contour run at 904 ms had 196 syncs with a
  deferral (1531 chunks waiting for space at one tick) against 3 in the other, so the default
  look's latency follows mirror pressure (r_full 16 to 12, ending 13; peak live 0.8894 of the
  budget in both), the footprint ADR-0012's mirror-slot plan owns. The streaming line (1.0 ms)
  stays a recorded miss.

## 6. Determinism and headless operation

- `vxe-world` builds and tests without a GPU, a window or a display server.
- No wall-clock reads in `vxe-world` or in server-side code; `clippy.toml` enforces it.
- Everything the engine generates takes an explicit seed. There is no engine-owned global RNG.
- Render tests run on the software adapter (lavapipe) in CI; golden baselines are blessed there
  and real-vendor readings are recorded separately, never substituted.
- Chunk mips join the determinism vector (`apps/flythrough/tests/determinism.rs`, Plan 0002):
  the mips hash `0x6cc1_4fda_4b7f_5188` is blessed on Linux. The Windows arm confirms it on
  the first `main` CI run after Plan 0002's merge; a mismatch is a defect in an integer path,
  fixed forward.
- Generation with prefab placements (`StampedSource`, Plan 0008) is independent of the thread
  count: `generate_with_threads` at 1 and 7 threads gives the same hash, and
  `crates/vxe-world/tests/stamp.rs` pins it at `0x7ee7_eeb7_0956_d069`, blessed on Linux; the
  stamp is integer-only, so the Windows arm must reproduce it.
- The collapse course (`crates/vxe-world/tests/collapse.rs`, Plan 0009: a query, four per-tick
  drops and a removal, hashing every result and the world after each step) pins
  `0x3178_0a15_61c1_3041`, blessed on Linux. The Windows arm confirms it on the first CI run
  after a push of `main`; the ops are integer-only, so a mismatch is a defect, fixed forward.
- The op log (Plan 0007): the determinism vector's 24 ops encoded as an op log (1446 bytes)
  hash to `0x5ac8_737d_db0f_ccec` (`apps/flythrough/tests/determinism.rs`), and decoding and
  replaying that log reproduces the vector hash; `vxe-net`'s re-simulation tests hold a seeded
  logic to equal op-log bytes. **Asserted** on Linux, blessed at d77ef85; the Windows arm
  confirms it on the first CI run after a push of `main`.
- Native versus component (Plan 0011): one logic linked natively and hosted as a WebAssembly
  component gives byte-equal op logs and equal world hashes from the same inputs. The echo
  logic pins `0x8f7d_4016_15d9_5c4d` (`crates/vxe-script-wasm/tests/equality.rs`), and the
  fly-through's dig logic is equal on the 20-dig replay fixture, the `Bench` input and a saved
  `--logic wasm` session replayed with `--logic native` (`0x86ca427a47e8908c`). **Asserted** on
  Linux; the Windows arm confirms it on the first CI run after a push of `main`.

## 7. Startup and build

- Time to first frame in `apps/flythrough` under 3 s on the dev reference (proposal).
  **Reading (Plan 0001, 112cfa0, level-size bounded cave seed 1, 3200 chunks, RTX 3080 Laptop
  GPU, Ryzen 9 5900HS, Hyprland):** 2226 to 2301 ms cold (dGPU suspended before launch),
  1063 to 1187 ms warm; device creation dominates cold start (1838 to 1903 ms), the mirror
  upload is 349 to 357 ms for 402 MB. 2282 ms on the owner's interactive run at 1d61aba.
  **Reading (Plan 0002 Phase 8, streaming unbounded cave seed 1, headless on the RTX 3080
  Laptop GPU, so no window or surface):** the first frame waits for the 33 chunks within
  squared distance 4 of the spawn chunk: first frame at 1491 and 457 ms (device ready at 1268
  and 417 ms); full residency (17 077 full, 40 700 LOD-only chunks) at 12 798 and 12 685 ms,
  and at 12 134 and 11 697 ms at the fix-pass commit (076b1c4). The close review's cold run
  (ffc73e8) read device ready 1968 ms and first frame 2075 ms. These runs create no window or
  surface, so a windowed first frame adds that cost.
  **Reading (Plan 0017, ef37395 and 3b9ea25, streaming seed 1 headless on the RTX 3080 Laptop
  GPU, exclusive perf lock):** the mirror's one reservation at startup takes 63.0 to 109.9 ms
  for 1930 to 3072 MiB (Phase 1's guess was under 50 ms; a probe put `create_buffer` near
  18 ms per GiB, so the time follows the budget). First frame 516 to 1105 ms warm under
  1-minute load 10.6 to 23.2 from other builds (2135 and 2669 ms with a cold device); Phase 1
  read 414 to 513 ms at load 9.6 to 11.0 beside 367 and 375 ms on the base commit d11d0cb.
  No quiet-box reading: every lane was building during these runs.
  **Level-size first sync after narrowing (Plan 0002 Phases 5 and 8, RTX 3080 Laptop GPU):**
  497 to 517 ms, up from Plan 0001's 349 to 357 ms for the `u16` mirror; the palette scan
  before indices are written. Accepted at the Plan 0002 close (owner, 2026-10-16); TTFF stays
  under 3 s.
- An incremental `cargo build` of a one-crate change under 10 s on the dev reference; the
  everyday loop does not compile crates a plan does not touch (`default-members`).
  **Reading (Plan 0011 Phase 4, dev reference, AMD Ryzen 9 5900HS, `CARGO_BUILD_JOBS=3`,
  exclusive perf lock, 1-minute load 1.99 to 1.91):** default-members `cargo build` after a
  one-line change (a `pub const`) to `crates/vxe-world/src/lib.rs`, alternated three times:
  1.40 to 1.49 s on the tree before Plan 0011 (066d90e), 1.46 to 1.48 s at its tip (70b40ec
  plus the bench). The default members compile no `wasmtime`: `cargo tree -e normal,build`
  of them and `cargo tree -p flythrough -e normal` list none; `flythrough --features wasm`
  brings it in. Under the 10 s budget, and unchanged at the Plan 0011 close: the default build
  still compiles no `wasmtime` or `tokio`.

## 8. Collision and coarse grids

Added at Plan 0004's close (2026-10-17). `vxe-sim` sweeps and the character controller,
`vxe-world`'s coarse grid and walkable map. All of it is integer and joins §6: the controller
course hash `0x0c80_8031_d14f_27eb` (`crates/vxe-sim/tests/course.rs`) is blessed on Linux and
the Windows arm confirms it on the first `main` CI run after Plan 0004's merge.

| Figure | Budget | Measured by |
|---|---|---|
| Controller tick, one body | 20 µs | `cargo bench -p vxe-sim --bench controller` |
| Sweep, one body, one contact | 26.3 µs (the Plan 0004 close reading, accepted by the owner 2026-10-17; the proposal was 5 µs); a sweep profile is scheduled only when a scene sweeps more than about 100 bodies a tick | same bench, `1000 sweeps of the F2 wall case` |
| Coarse grid update after a radius-1 m carve | 50 µs | `cargo bench -p vxe-world --bench coarse` |
| Walkable update after that change | 20 µs | same bench |
| Coarse grid RAM | 4 B per brick, 2048 B per chunk | `world_budget --chunks` |

**Readings (dev reference, quiet box, AMD Ryzen 9 5900HS laptop, Linux, commit b94622a (the
merge of `main` into Plan 0004's branch; `vxe-sim` identical to cf3bbc6), `cargo bench`,
criterion median (low, high), exclusive perf lock, load average 1.7 to 2.3 at each start):**

- **Controller tick:** `60 ticks of the F6 staircase, all on terrain` 156.01 µs (155.84,
  156.21), **2.60 µs per tick**; `300 ticks of the F6 course` 522.16 µs, 1.74 µs per tick;
  `1000 ticks of the F6 course` 772.89 µs, 0.77 µs per tick (most of those ticks are free fall
  past the fixture's extent). Under the 20 µs line. The worst case, a 64-iteration grazing
  corner sweep inside a tick, is not benched.
- **Controller tick with depenetration (Plan 0016 Phase 1, an overlap test at every tick's
  start):** `60 ticks of the F6 staircase, all on terrain` 108.17, 110.59 and 109.61 µs
  against 108.56, 108.35 and 111.60 µs for the same bench at e23f309 (no overlap test),
  alternated on the same box: AMD Ryzen 9 5900HS laptop, Linux, not quiet (load average 13.4
  falling to 5.2, other processes running), no perf lock. About **1.8 µs per tick**; the two
  arms do not separate. The 156.01 µs above was not reproduced on this day by either arm.
- **Sweep:** `1000 sweeps of the F2 wall case` 26.303 ms (26.295, 26.312), and 26.264 ms on
  a second run: **26.3 µs per sweep**, against the 5 µs proposal. The owner accepted the
  measured figure as the line at 17.5 µs (fe50d6d); the walk-check fix pass (closes-on filter,
  overlap guard, rounded-up step length; ADR-0005 Outcome) raised it to 26.3 µs. **The
  owner accepted 26.3 µs as the line (2026-10-17)**, with a sweep profile scheduled only
  when a scene sweeps more than about 100 bodies a tick.
- **Coarse grid update** after a radius-10 (1 m) carve on the solid world: 100 updates
  1.3544 ms (1.3528, 1.3560), **13.5 µs per update**. **Walkable update** from those changes:
  807.17 µs (805.83, 808.64), **8.1 µs per update**. Both under their lines. Plan 0003's
  `mixed` world was not on the branch; the bench runs on the solid world.
- **Coarse grid update on the server path (Plan 0018 Phase 5):** the server updates its grid
  over each applied op's aabb, in op order on the server thread. `cargo bench -p vxe-net
  --bench tick` (100 radius-1 m carves on `solid3_world()` each followed by
  `CoarseGrid::update_aabb`, against the carves alone), AMD Ryzen 9 5900HS, Linux, a9014d0
  plus the phase's bench arm, exclusive perf lock, not quiet (1-minute load 14.46 falling to
  6.50), three alternated rounds: 1.4764, 1.4876 and 1.2249 ms per 100, **12.2 to 14.9 µs per
  update**, under the 50 µs line and beside the 13.5 µs above.
- **Coarse grid RAM:** 6 553 600 B at the level (3200 chunks, `flythrough --level --seed 1`,
  fe50d6d). At 32 × 32 × 32 chunks it is 67 108 864 B by the same 2048 B per chunk
  (arithmetic, not read). Grid build at the level, one thread (`world_budget --chunks 20x20x8
  --seed 1`): 156.3 ms at the close review; grid plus walkable map in the fly-through 231 ms.

## 9. Audio

`vxe-audio` (Plan 0006 closed 2026-10-17, ADR-0006): 48 kHz stereo `f32`, 256-frame blocks
(5.33 ms), mixed on the device callback. The baseline CPU is not on hand, so every reading is
dev reference: Ryzen 9 5900HS laptop, release, the quiet-box protocol of Plan 0006's close
(exclusive machine lock, 1-minute load average under 2, no other process above 7 % CPU),
44a1582's tree unless stated. Readings taken under a load average of 9 to 21 are in the
plan's Close triggers as superseded figures.

- **Mixer callback, 32 HRTF voices, one 256-frame block:** 1.0 ms on the baseline CPU
  (proposal; 32 x 2 x 128 x 256 = 2.1 million multiply-adds plus filters and taps).
  **Reading (`cargo bench -p vxe-audio --bench mixer`, 32 voices of looping noise, each
  low-passed and carrying 16 taps):** 0.3619 ms (0.3615 to 0.3623) static; 0.6667 ms (0.6659
  to 0.6673) with the listener yawing 90 degrees per block, every voice crossfading its HRIR.
- **`Acoustics::update`, 32 sources:** about 1.0 ms per call, accepted by the owner
  2026-10-17 (D2b) over the plan's 0.5 ms proposal, which had no arithmetic behind it: about
  240 rays of about 100 voxel steps at about 40 ns, each step a chunk-map lookup and lock.
  **Reading (`cargo bench -p flythrough --bench acoustics`, seed-1 level-size cave, listener
  at the spawn, sources 2 to 12 m out, every source due, 512 taps sent):** 1.0294 ms (1.0278
  to 1.0313). Speed-ups (a chunk handle cached across steps, a probe worker thread) are
  followups.
- **Audio RAM:** the pack plus 200 ms delay lines plus scratch (proposal). **Reading:** pack
  3 194 204 B in memory (3 044 420 B on disk); delay line 39 424 B per voice, 2 523 136 B for
  the 64-voice pool allocated at `Mixer::new`.
- **Startup:** pack load under 10 ms (proposal). **Reading:** 13.5 to 13.6 ms in the release
  fly-through over three runs (`--seed 1 --audio --headless --frames 10`, a null-ALSA
  device); 7.1 to 8.9 ms in a release test build. Accepted as the reading (D2c): it runs once.
- **Fly-through `F1`:** the callback times itself (`OutputStats::busy_max_ms`) and `F1` prints
  the longest callback over the last second, the voices, the active taps and the last
  acoustics update's time. **Reading by the owner, 2026-10-17, `main` at 7e8b3d2 with 8
  voices playing:** callback max 0.429 ms, 128 active taps.
- **Determinism:** `World::trace_materials` is integer and its 500-ray hash is pinned in
  `crates/vxe-world/tests/queries.rs` on both arms; nothing else in `vxe-audio` is
  deterministic by contract (§6).
- **Plan 0020 proposals (the late tail, sixteen direction buses, the room fan; dev box, to be
  read at its close):** mixer block, 32 HRTF voices, static, at most 0.6 ms (today 0.3619 ms;
  the buses are 1 048 576 multiply-adds per block, about half the 32-voice FIR load) and
  turning at most 1.0 ms (today 0.6667 ms); `Acoustics::update`, 32 sources, at most 1.1 ms
  with a 16-ray fan slice in the call (today 1.0294 ms); audio RAM plus 323 584 B for the tail's
  lines, about 24.5 KB for the buses, 8 KB of tap state and 19.2 KB per streamed voice.

## 10. Not goals (for now)

- Binary size.
- Mobile, web or console targets as such. The portable profile of §1 is a PC-class setting
  that a Switch-2-class device could run; a console port, its SDK and its certification are
  not planned, and Switch 1 is out of reach for a ray marcher of this shape (owner,
  2026-10-17).
- Frame rates above 60 fps as a gate; they are welcome, not required. A fast machine
  reaches 120 fps through a performance profile, not through a budget row here.
- VR on the baseline. The VR-readiness decision in `roadmap.md` keeps the renderer's doors
  open; a VR plan adds its own profile here (two views, 90 Hz, a named card) when scheduled.
