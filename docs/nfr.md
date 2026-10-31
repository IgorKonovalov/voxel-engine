# Non-functional requirements

The budgets the engine is held to. Every "fast", "large" and "stable" claim elsewhere means one
of the figures below. Each figure is a **proposal until a measurement replaces it**; a
measurement names the commit, the machine, the resolution and the scene. A plan that touches
storage or the frame states which figures it affects and measures them at its close.

Written 2026-10-16. Readings are added under the figure they measure, each naming its source.

## 1. Hardware

| | Baseline (the floor) | Dev reference | Slow adapter (on hand) | Portable profile (a quality profile, not a gate) |
|---|---|---|---|---|
| GPU | 2019 mid-range desktop: GTX 1660 / RX 5600 class, 6 GB VRAM, Vulkan 1.2, no RT hardware | NVIDIA RTX 3080 Laptop, 8 GB, driver 610 | the dev box's AMD Cezanne iGPU (Radeon Vega, RADV on Mesa, Vulkan 1.4): about 2 TFLOPS, DDR4 shared with the CPU, below the floor's class | Switch-2 class: about 1.7 to 3 TFLOPS, 68 to 102 GB/s, published figures; no device on hand |
| CPU | 4 cores / 8 threads | 16 threads (AMD, Cezanne) | the same | 8 ARM cores class |
| RAM | 16 GB | 30 GB | the same, shared with the GPU | 9 GB shared by CPU and GPU |
| OS | Linux (Vulkan), Windows (Vulkan or DX12 through wgpu) | Arch Linux, Wayland | the same | none targeted; the profile is held on a PC of that class |

A perf figure is labelled by the adapter it was read on. **The baseline card is not acquired
(owner, 2026-10-21; `roadmap.md`, "Baseline card")**: the floor's figures stay projections,
bracketed by two readings on the dev box, the NVIDIA card above the floor and the slow adapter
below it. From Plan 0028 on, every render plan's close takes its GPU readings on both adapters
(the fly-through's `--adapter` flag, Plan 0028 Phase 4). A figure read on the NVIDIA card alone
is labelled "dev reference" as before.

The portable profile (owner, 2026-10-17; `roadmap.md`, "Floor, profiles and scene scale") is
a named setting of the quality knobs (internal resolution, full-detail radius, translucent
layers, lighting tier) at 720p output and 30 to 60 fps. No machine of that class is on hand;
the slow adapter above is the nearest thing to it and its readings stand in, labelled as the
slow adapter's, beside the storage reads per ray that Plan 0015 counts, against about three
uncached 64-byte lines per ray at 720p and 60 fps with three rays a pixel (arithmetic from
the published bandwidth).

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

**The upscaler at 1080p output (dev reference, Plan 0030 Phase 5, render code at 11a5bc9,
the release `flythrough --headless --level --render 1920x1080 --frames 300` from the spawn
pose, median of the last 30 frames, three runs each, exclusive perf lock, 1-minute load 4.7 to
10.4 from other lanes' builds: loaded, indicative):** on the NVIDIA GeForce RTX 3080 Laptop GPU (Vulkan, driver
615.78.08), scale 1.5 (1280 x 720 internal): frame 3.053 to 3.103 ms, hit 0.600 to 0.610,
light 0.878 to 0.900, temporal 0.070 to 0.074, a-trous 0.541 to 0.549, composite 0.142 to
0.145, upscale 0.729 to 0.748, sharpen 0.076 to 0.081; scale 1 (jitter on, the upscaler
accumulating at 1080p): frame 5.980 to 6.038, hit 1.375 to 1.389, light 2.049 to 2.070,
temporal 0.139 to 0.140, a-trous 1.204 to 1.218, composite 0.324 to 0.327, upscale 0.811 to
0.820, sharpen 0.078 to 0.079; scale 1 with the jitter off (no upscaler, one run): 5.210. On the
slow adapter, the AMD Radeon Graphics of the Ryzen 9 5900HS (RADV RENOIR, Mesa 26.2.5,
selected with `VK_DRIVER_FILES=/usr/share/vulkan/icd.d/radeon_icd.json
__EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json`): scale 1.5 frame
20.149 to 20.876 ms (hit 4.074 to 4.158, light 6.690 to 6.892, temporal 0.495 to 0.571,
a-trous 2.323 to 2.408, composite 1.116 to 1.265, upscale 4.786 to 4.872, sharpen 0.647 to
0.731); scale 1 frame 38.658 to 40.955 (hit 8.950 to 9.164, light 14.966 to 15.527, upscale
5.171 to 5.549, sharpen 0.640 to 0.748); scale 1 with the jitter off 32.704 (one run). The
upscale line holds on the dev box (upscale, sharpen and composite 0.95 to 0.97 ms at scale
1.5); on the iGPU the upscale pass alone reads 4.8 ms.

**The upscaler after its rework (Plan 0030 Phase 8, render code at 12c6c57: the 3 x 3
Gaussian, the Catmull-Rom history fetch, the light-change clip; same command, pose, machines
and adapter selection as Phase 5, exclusive perf lock, 1-minute load 3.9 to 4.5 on the NVIDIA
runs and 2.3 to 3.6 on the RADV runs, loaded, indicative):** NVIDIA, scale 1.5: frame 2.727 to
2.780 ms, hit 0.570 to 0.578, light 0.832 to 0.850, temporal 0.069, a-trous 0.508 to 0.521,
composite 0.136 to 0.138, upscale 0.544 to 0.556, sharpen 0.075 to 0.076; scale 1 (jitter
on): frame 5.294 to 5.546, hit 1.257 to 1.317, light 1.857 to 1.952, temporal 0.137 to 0.138,
a-trous 1.099 to 1.150, composite 0.297 to 0.309, upscale 0.573 to 0.596, sharpen 0.075 to
0.076. RADV, scale 1.5: frame 18.797 to 18.865 ms (hit 4.078 to 4.126, light 6.729 to 6.755,
temporal 0.492 to 0.503, a-trous 2.332 to 2.343, composite 1.126 to 1.134, upscale 3.216 to
3.232, sharpen 0.645 to 0.652); scale 1 frame 38.011 to 38.246 (hit 8.934 to 8.945, light
15.000 to 15.116, upscale 4.145 to 4.191, sharpen 0.646 to 0.653). Upscale, sharpen and
composite read 0.755 to 0.770 ms on the dev box at scale 1.5; on the iGPU the upscale pass
alone reads 3.2 ms.

Plan 0030's close kept Plan 0024's frame (scale 1, jitter off, no upscaler) as the default and
read the above as the opt-in performance setting. Since Plan 0041's close (2026-10-27,
ADR-0034's Outcome) the fly-through's default is our upscaler at scale 1.5, jittered, the
owner's pick after the windowed comparison with the FSR 2 port read below; the library's
`Frame` still defaults to Plan 0024's frame.

**FSR 2.2.1 ported to WGSL beside our upscaler and Plan 0024's frame (Plan 0041 Phase 6, the
release binary of 6e64744's tree plus the Phase 6 banner change, 2026-10-27; `flythrough
--level --seed 1 --headless --render 1920x1080 --frames 300 --upscaler MODE --internal-scale S
--adapter NAME` from the spawn pose, the fly-through's defaults (indirect light on, lamp on,
half-resolution lighting, contour look with detail, sharpening 0.5), median of the last 30
frames, three runs each, exclusive perf lock, memory cap; 1-minute load 2.4 to 6.5 inside the
lock at each start: loaded, indicative).** FSR's nine passes are timed one by one (input, spd,
spd tail, reconstruct, depth clip, lock, accumulate, rcas, tone) and summed here. The FP32
path, no wave operations.

| adapter, mode | frame ms | hit ms | light + temporal + a-trous ms | probes ms | composite ms | upscale + sharpen ms | FSR 2's nine passes ms |
|---|---|---|---|---|---|---|---|
| RTX 3080 Laptop, `off` 1 | 6.789 to 6.878 | 1.390 to 1.403 | 3.427 to 3.460 | 1.417 to 1.435 | 0.322 to 0.325 | | |
| RTX 3080 Laptop, `own` 1.5 | 4.389 to 4.514 | 0.618 to 0.633 | | 1.349 to 1.365 | 0.145 to 0.150 | 0.648 to 0.669 | |
| RTX 3080 Laptop, `fsr2` 1 | 10.077 to 10.167 | 1.425 to 1.444 | 3.461 to 3.510 | 1.452 to 1.486 | 0.347 to 0.348 | | 3.115 to 3.127 |
| RTX 3080 Laptop, `fsr2` 1.5 | 5.709 to 5.805 | 0.626 to 0.634 | 1.571 to 1.593 | 1.331 to 1.389 | 0.156 to 0.157 | | 1.894 to 1.913 |
| Radeon (Cezanne, RADV), `off` 1 | 43.733 to 43.938 | 9.327 to 9.387 | 22.025 to 22.208 | 7.953 to 8.027 | 2.804 to 2.866 | | |
| Radeon (Cezanne, RADV), `own` 1.5 | 28.120 to 28.780 | 4.264 to 4.317 | | 7.926 to 8.223 | 1.230 to 1.309 | 3.885 to 4.016 | |
| Radeon (Cezanne, RADV), `fsr2` 1 | 61.327 to 61.384 | 9.318 to 9.357 | 22.077 to 22.110 | 7.890 to 8.168 | 3.441 to 3.476 | | 16.393 to 16.446 |
| Radeon (Cezanne, RADV), `fsr2` 1.5 | 34.652 to 34.998 | 4.274 to 4.284 | 9.915 to 9.930 | 7.800 to 8.166 | 1.507 to 1.521 | | 10.045 to 10.085 |

At scale 1.5 on the dev box FSR 2's passes read 1.894 to 1.913 ms against the plan's 1.2 ms
bar (the 1.5 ms upscale line less the composite and the HUD): a recorded miss, judged at
Plan 0041 Phase 7, where the owner kept FSR 2 opt-in and made `own` at 1.5 the default, whose
upscale, sharpen and composite are under the line. The largest are accumulate (0.728 to 0.743 ms) and depth clip (0.600 to
0.601). With the composite, 2.05 to 2.07 ms against the 1.5 ms line; our upscaler's
upscale, sharpen and composite read 0.79 to 0.82 ms in the same session. The hit pass at 1.5
is 44.6 to 45.6 % of scale 1's, light + temporal + a-trous 45.4 to 46.5 %. The probe update
does not scale with the internal size (1.33 to 1.39 ms at 1.5 against 1.42 to 1.44 at 1). On
the iGPU FSR 2's passes at 1.5 read 10.0 to 10.1 ms (accumulate 4.24 to 4.26) against our
upscale and sharpen's 3.9 to 4.0 ms; Plan 0030 Phase 8 read that iGPU frame at 18.797 to
18.865 ms and its upscale at 3.216 to 3.232 ms before the probe volume landed (about 8 ms of
probe update and 0.6 ms of gather in every mode here). The adapter is chosen with
`--adapter` (a substring of the adapter's name): the NVIDIA GeForce RTX 3080 Laptop GPU
(Vulkan, driver 615.78.08) and the AMD Radeon Graphics (RADV RENOIR) of the Ryzen 9 5900HS.

**The hit pass, split from the shading (dev reference, Plan 0024 Phase 1, same machine,
exclusive perf lock, 1-minute load 7.3 to 8.0 from other work on the box, not quiet; the
default look, `flythrough --level --headless --seed 1 --render 1920x1080 --frames 300`, three
runs of each binary alternated):** the hit pass reads 1.235 to 1.236 ms against 1.733 to 1.753
ms for the single-pass march (ae58f0c) in the same session; the shadow ray is now the light
pass's, 0.712 to 0.718 ms, and the composite reads 0.255 ms, so the frame's three passes sum to
2.202 to 2.210 ms. On the dev box's AMD iGPU (RADV, `RADV_DEBUG=shaderstats`) the hit kernel
uses 112 SGPRs and 64 VGPRs with no spills, 4 waves per SIMD; the single-pass march used 112
SGPRs (9 spilled) and 84 VGPRs, 3 waves per SIMD.

**The hit pass over size-classed slots (dev reference, Plan 0028 Phase 2, RTX 3080 Laptop GPU,
exclusive perf lock, 1-minute load 4.5 to 8.7 from the other lanes' work, not quiet; three runs
of each binary alternated against the pre-plan binary at 1d1e67c; the frame line's `hit`):** the
level's spawn (`flythrough --level --headless --seed 1 --render 1920x1080 --frames 300`) reads
1.274, 1.314 and 1.274 ms with `--look plain` against 1.273, 1.274 and 1.264 ms, and 1.404,
1.420 and 1.419 ms in the default look against 1.346, 1.404 and 1.346 ms (that phase's contour
still kept 16 attribute units per brick); the streamed spawn (`--frames 3000 --look plain`,
full residency at `r_full` 16) 1.701, 1.710 and 1.714 ms against 1.708, 1.731 and 1.705 ms. A
hit in a one-material brick reads one header word where the walk read a pool byte and a record
palette word; a plane costs one more word per plane.

**The settled streamed spawn at `r_full` 16, both adapters (Plan 0028 Phase 4, exclusive perf
lock; `flythrough --headless --seed 1 --render 1920x1080 --frames 3000 [--look plain]`, median
of the last 30 frames, three runs each; no bar).** Dev reference (RTX 3080 Laptop, load 3.27
to 7.90, not quiet): the hit pass reads 1.767, 1.764 and 1.780 ms in the default look (frame
5.705 to 5.739 ms) and 1.539, 1.571 and 1.558 ms plain (frame 5.277 to 5.381 ms). Slow adapter
(AMD Radeon Graphics, RADV RENOIR, selected with `--adapter AMD`, load 1.05 to 2.96): the
default look's hit 13.337, 13.318 and 13.354 ms (frame 44.550 to 44.706 ms, the light pass
22.344 to 22.370 ms), plain 11.961 ms in one run (frame 41.229 ms); the level's spawn
(`--level ... --frames 300 --adapter AMD`, load 2.01 to 3.62) hit 9.302, 9.296 and 9.297 ms,
frame 33.757 to 33.774 ms. These are the first slow-adapter readings.

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

**The attribute pass over surface-only attributes (dev reference, Plan 0028 Phase 3, RTX 3080
Laptop GPU, exclusive perf lock, 1-minute load 9.3 to 9.8; `flythrough --autopilot --headless
--seed 1`, the default look, `r_full` 16 held by every run):** GPU p50 3.054 ms, p99 5.770 ms,
max 6.058 ms with `attr_cap` 8192 (work list p50 8192, p99 10 204; clear items p50 5482), and
p50 1.533 ms, p99 5.751 ms with 4096 (a measurement build of the cap), against Plan 0013's 1.0
ms p99 proposal: a recorded miss. The carry-over age reads p50 520 ms, p99 7625 ms at 8192 and
p50 13 360 ms, p99 27 229 ms at 4096. The pre-plan binary (1d1e67c), whose flight lowered
`r_full` to 12, read p99 4.820 ms with clear items p50 0 and age p99 69 ms. A first version of
the pass that filled its occupancy box voxel by voxel read p99 7.636 and 7.781 ms; filling it
from the 27 bricks' masks in workgroup memory took 2 ms off. The pass's cost is per brick (the
box fill, the workgroup, the clear items), not per voxel computed.

**The amended pass (dev reference, Plan 0028 Phase 3, RTX 3080 Laptop GPU, exclusive perf
lock, quiet: 1-minute load 0.93 to 1.98; the same flight):** four bricks a workgroup, no clear
items, a LOD-only neighbour read through its level-1 mip brick's masks. With `attr_cap` 8192,
GPU p99 0.711, 0.753 and 0.754 ms over three runs (p50 0.445 to 0.485 ms) and carry-over age
p99 25.1 to 25.5 ms: under the 1.0 ms proposal and the 250 ms line. With 4096 (a measurement
build, load 3.88), p99 0.563 ms and age p99 2306.4 ms. On the slow adapter (AMD Radeon
Graphics, RADV RENOIR, `--adapter AMD`, load 7.27, no bar), 8192: p50 2.010 ms, p99 2.522 ms,
age p99 5295.6 ms. The per-attempt readings are in Plan 0028's log.

**Face against contour at the level's spawn, both adapters (dev reference, Plan 0039 Phase 1,
the binary of 99e7bee, exclusive perf lock, 1-minute load 4.2 to 18.8 from the other lanes:
loaded, indicative; `flythrough --level --headless --seed 1 --render 1920x1080 --frames 300
--look face|contour --adapter NVIDIA|AMD`, median of the last 30 frames, three runs of each look
alternated; the attribute pass from the level's initial fill and two `flythrough --autopilot
--headless --seed 1 --look face|contour` flights each).** NVIDIA GeForce RTX 3080 Laptop GPU:
face hit 1.290 to 1.375 ms, light 2.074 to 2.226, composite 0.316 to 0.338; contour hit 1.391 to
1.470, light 2.147 to 2.276, composite 0.317 to 0.336; initial fill 16.4 to 16.5 ms for 383 608
bricks in both looks (one contour run 11.8); autopilot attribute pass GPU p50 0.488 and 1.510
ms, p99 0.740 and 2.483 ms (face), p50 0.482 and 0.811, p99 1.281 and 1.790 ms (contour). AMD
Radeon Graphics (RADV RENOIR, `--adapter AMD`): face hit 8.749 to 8.798 ms, light 14.993 to
15.039, composite 2.542 to 2.629; contour hit 9.307 to 9.310, light 15.585 to 15.616,
composite 2.543 to 2.561; initial fill 94.2 to 94.8 ms; autopilot p50 2.010 to 2.025 ms, p99
2.514 and 2.965 ms (face), 2.530 and 2.542 ms (contour). The NVIDIA flights' spread follows the
load (face p99 2.483 ms at load 14.8, 0.740 at 8.4).

**The contour presets at the level's spawn, both adapters (dev reference, Plan 0039 Phase 3,
the binary of 426982e, exclusive perf lock, 1-minute load 11.5 to 44.6 from the other lanes:
loaded, indicative; `flythrough --level --headless --seed 1 --render 1920x1080 --frames 300
--look contour|contour-sharp|contour-grain --adapter NVIDIA|AMD`, the median over three
alternated runs of each run's median of the last 30 frames; the attribute pass from the
level's initial fill and two `flythrough --autopilot --headless --seed 1` flights each).**
NVIDIA GeForce RTX 3080 Laptop GPU: contour hit 1.378 ms, light 2.139, composite 0.320,
initial fill 17.0 ms, autopilot p50 1.552 and 1.308, p99 2.944 and 2.756 ms; contour-sharp hit
1.408, light 2.174, composite 0.334, fill 17.8, p50 1.928 and 1.384, p99 2.756 and 2.522;
contour-grain hit 1.408, light 2.160, composite 0.383, fill 17.0, p50 1.234 and 1.730, p99 2.376
and 2.957. AMD Radeon Graphics (RADV RENOIR): contour hit 9.908, light 17.545, composite 3.424,
fill 97.1, p50 2.080 and 2.080, p99 2.709 and 2.697; contour-sharp hit 9.898, light 16.929,
composite 3.483, fill 96.3, p50 2.075 and 2.070, p99 2.645 and 2.614; contour-grain hit 9.831,
light 17.172, composite 3.600, fill 96.3, p50 2.063 and 2.062, p99 2.655 and 2.586. Mirror
157 872 640 B live, 259 203 264 B allocated in every run. The volume-matched offset alone
(not in a preset) filled in 46.6 ms on NVIDIA, 61.8 ms with the weighted fit (Plan 0039's
log).

**The default look, contour-sharp, against the pre-plan default, both adapters (dev reference,
Plan 0039's fix pass: the merged binary of 4d68e91 against main's binary of dedd83c, whose
default is `contour`, both with no `--look`, so with main's probe GI on by default; alternated
in one session; exclusive perf lock; the level runs at 1-minute loads 1.9 to 3.0 (NVIDIA) and
7.1 to 14.9 (AMD): loaded, indicative; commands as in Phase 3's line, three level runs of each
binary; the attribute pass retaken quiet: three alternated pairs of autopilot flights per
adapter, each started at a 1-minute load of 1.73 to 1.98).** NVIDIA GeForce RTX
3080 Laptop GPU: hit 1.392 ms against 1.402 (-0.7 %), light + composite 2.505 against 2.486
(+0.8 %), initial fill 17.8 against 16.4 ms; quiet autopilot attribute pass p99 0.872, 1.054
and 0.851 ms against 0.945, 0.871 and 0.737 (-7.7, +21.0 and +15.5 %; medians 0.872 against
0.871, +0.1 %; main's own three span 0.737 to 0.945). AMD Radeon Graphics (RADV RENOIR): hit
9.338 against 9.330 (+0.1 %), light + composite 18.488 against 18.454 (+0.2 %), initial fill
266.0 against 262.5 ms (loaded); quiet autopilot p99 23.952, 22.728 and 22.399 ms against
21.765, 21.126 and 20.964 (+10.0, +7.6 and +6.8 %), p50 2.07 against 2.04. Quiet, the AMD
attribute pass's p50 is Phase 3's again (about 2.05 ms; loaded it read about 20) while its p99
stays near 22 ms against Phase 3's 2.7, with the same work-list sizes; the pairing, not the
absolute value, is the reading. The tail is the machine's state, not the code: Phase 3's own binary
of 426982e, re-run at the close review (2026-10-27 14:32, load 3.41), read p50 2.093 and p99
23.314 ms.

**The crack-closing rule at the level's spawn, both adapters (dev reference, Plan 0050 Phases 3, 4
and 6; each run under the exclusive perf lock; `flythrough --level --seed 1 --headless --render
1920x1080 --frames 300 --adapter NVIDIA|AMD`, 1080p from 1280 x 720, release, the median over three
alternated rounds of each run's median of the last 30 frames).** The tree's reading (Phase 6,
2026-10-30, 45bc227's tree with snap the only rule, against the binary of 00ca974 before the plan;
1-minute load 1.48 at the start, 1.5 to 3.4 during, loaded): NVIDIA GeForce RTX 3080 Laptop GPU hit
0.616 ms against 0.608 (+1.3 %), light 0.925 against 0.923; AMD Radeon Graphics (RADV RENOIR) hit
4.298 against 4.279 (+0.4 %), light 7.175 against 7.266. While the rules stayed selectable (Phases
3 and 4, 2026-10-29, the release binary of e9cae0e, `[--closing RULE] [--shadow-push on]`, load 1.1
to 3.0): RTX hit 0.598 ms before the plan, 0.658 with snap and 0.662 with `old` (+10.0 %, +10.7 %),
light 0.904 before and 1.529 with snap; Radeon hit 4.264, 5.981 with snap and 5.978 with `old`
(+40.3 %, +40.2 %), light 7.012 before and 8.893 with snap. In that series the light pass read
1.439 (RTX) and 9.165 (Radeon) with the shadow push. The hit and light shaders held 64 and 84 VGPRs
before the plan and with the other rules deleted (a scratch build of e9cae0e's tree), 128 each in
e9cae0e, where the blend rule's code sat in every walking shader (RADV shader stats). Mirror 157
872 640 B live, 259 203 264 B allocated in every run.

**Mirror attribute pass (contour look), render-thread CPU: p99 under 4 ms on the dev reference,
deferred work visible as face-mode normals for at most 250 ms** (budget, Plan 0023, ADR-0017;
dev reference only). **Reading: a recorded miss (Plan 0023 Phase 6 quiet re-read, the Phase 6
binary at a4889e4's code, RTX 3080 Laptop GPU, Ryzen 9 5900HS, exclusive perf lock, start loads
1.56 to 1.83 with nothing above 5 % CPU, `flythrough --autopilot --headless --seed 1`
alternated with `--look plain`):** `update_attrs` p99 5.528 and 6.024 ms, max 14.596 and
13.727 ms (138 to 180 ms before); carry-over age max 0 ms, since no flight's work list (max
7381 and 7606 bricks) reached the 8192 cap. The default flew 9775 and 9563 frames against
plain's 4111 and 4159 (before: 35 to 47 % fewer), with `r_full` lowered to 12 under mirror
pressure. The pass's GPU time is not this line: p50 1.25 to 1.28 ms, p99 2.82 to 3.14 ms, max
3.20 to 3.46 ms, still over Plan 0013's 1.0 ms p99 proposal, unowned. In the owner's windowed
flight (2026-10-21) the hitch was "mostly gone" and new rock showed no face-mode lag.

**Streaming line after Plan 0023 (same runs): still a recorded miss.** `update + poll + sync`
p50 4.896 and 5.020 ms in plain (0.006 ms in the default runs, which mirror about 40 000 chunks
at `r_full` 12 against plain's 73 000), p99 12.3 to 24.3 ms, max 22.2 to 60.3 ms against the
1.0 ms line (before: p50 4.5 to 5.0, p99 33 to 65, max 41 to 108 ms). The incremental queue
cut `update`'s max from 37.8 to under 3.2 ms. What remains is `sync`, led by
`Queue::write_buffer` (41.8 % of its samples) ahead of narrowing (about 38 %); ADR-0017's
Outcome names the next step.

**The light pass by shadow walk (dev reference, Plan 0024 Phase 4, RTX 3080 Laptop GPU,
exclusive perf lock, 1-minute load 6.3 to 8.1, not quiet; the default look with the ore
emissive and the lamp on, one run each, median of the last 30 frames):** the level's spawn
(`--level --frames 300`) reads 6.640 ms with the full walk and 5.329 ms with the coarse walk;
the streamed spawn (`--frames 3000`) 12.200 and 9.168 ms, against the 6.0 ms lighting line.
The time is the emissive shadow ray's: candidates drawn uniformly from the whole buffer (the
level's 417 388 samples) keep far emitters, so the ray is long. The steps view's lamp shadow
ray (`--view steps --frames 300`, every hit's ray toward the lamp, lit or not) reads at the
level 7.34 steps, 16.18 loads (p99 27) and 0.05 pool reads per ray in either walk; at the
streamed spawn 11.19 steps, 24.02 loads (p99 127), 0.16 pool reads full, and 10.64 steps,
23.02 loads (p99 93), 0.09 pool reads coarse. A lamp shadow ray is short (the lamp sits
beside the eye), so its loads barely move.

**Light candidates from the near-emitter table and bounded extraction (dev reference,
indicative, Plan 0024 Phase 7, RTX 3080 Laptop GPU, exclusive perf lock, NOT quiet: 1-minute
load 2.9 to 8.7; light radius 4, the ore emissive, the lamp on, 8 candidates; one run each;
the quiet-box readings at the defaults are in "Quiet-box readings at the lighting defaults"
below):** the light pass reads at the level's spawn 9.420 ms (full
walk) and 8.293 ms (coarse) with the table in a 16 KiB uniform, and 7.448 ms (full) with the
table moved into the light-sample buffer; the streamed spawn (`--frames 3000`) 13.249 ms (full)
and 11.311 ms (coarse) with the uniform table. With one candidate the level reads 4.644 ms
(4.178 ms with the lamp off), so the emissive shadow ray is about 3.5 ms and each further
candidate about 0.4 ms. Over the autopilot's flight (`--autopilot --headless --seed 1`) the
light-sample peak is 18 522 (15 915 at the end), with 0 ranges evicted, 0 refused and 0 ms of
wait inside the radius; extraction in `sync` reads p50 0.160 ms, p99 2.422 ms, max 6.408 ms
(load 3.5) and p50 0.142, p99 3.628, max 9.992 ms (load 4.6), against 13.3 and 48.9 ms at
Phase 2.

**The light pass by sampler (dev reference, indicative, Plan 0024 Phase 8, RTX 3080 Laptop
GPU, exclusive perf lock, 1-minute load 1.3 to 3.2 but the owner using the machine, so not
quiet; ore emissive, lamp on, 8 candidates, full shadow walk, one run each):** at the
streamed spawn (`--frames 3000`) the flat list reads 11.618 ms and reservoir reuse 14.183 ms;
at the level's spawn (`--level --frames 300`) 8.330 and 10.905 ms. Reuse adds about 2.6 ms:
its six re-targeted reservoirs each look their emitter's material up through the walk.

**The gradient pass (dev reference, indicative, Plan 0024 Phase 9, RTX 3080 Laptop GPU,
exclusive perf lock, 1-minute load 2.1 to 2.3 with the owner using the machine, so not
quiet; `--history gradient`, the default look, ore emissive, lamp on, one run each):** the
gradient pass (one re-shaded pixel, two shadow rays, per 3 x 3 stratum) reads 0.755 ms at the
level's spawn and 1.277 ms at the streamed spawn at 1080p; the a-trous pass reads 5.795 and
5.566 ms in that mode, its ages kept shorter.

**The temporal pass with two histories (dev reference, indicative, Plan 0024 Phase 13, RTX
3080 Laptop GPU, exclusive perf lock, 1-minute load 4.0 to 6.8, not quiet; the default look,
ore emissive, lamp on, one run each):** the temporal pass (the emissive part on a 32-frame
history, the lamp's on 0.2) reads 0.508 and 0.505 ms at full resolution and 0.137 and
0.135 ms at half (level, streamed spawn) at 1080p, against Phase 13's 1.0 and 0.3 ms bars; the
quiet readings are in "Quiet-box readings at the lighting defaults" above. `Frame::frame_bytes`
is unchanged at 308 736 000 B.

**Quiet-box readings at the lighting defaults (dev reference, Plan 0024 Phase 14, the release
binary of e9a8e6a's tree, RTX 3080 Laptop GPU, Ryzen 9 5900HS, 2026-10-22, exclusive perf
lock, each run started at a 1-minute load of 0.83 to 1.98 with no other process above 8 % CPU;
1920 x 1080, the default look, the ore emissive, the lamp on, full shadow walk, reservoir
reuse, edit-box history, a 32-frame emissive history, the filter on; median of the last 30
frames, three runs; `--level --frames 300` and the streamed spawn `--frames 3000`):**

| pass (ms) | level, half (default) | streamed, half (default) | level, full | streamed, full |
|---|---|---|---|---|
| light | 2.335, 2.441, 2.430 | 3.315, 3.355, 3.340 | 10.528, 10.915, 11.028 | 14.413, 14.526, 14.436 |
| temporal | 0.135, 0.137, 0.136 | 0.134, 0.134, 0.145 | 0.557, 0.559, 0.558 | 0.553, 0.553, 0.554 |
| light + temporal | 2.470, 2.578, 2.566 | 3.449, 3.489, 3.485 | 11.085, 11.474, 11.586 | 14.966, 15.079, 14.990 |
| a-trous (variance + 4) | 1.141, 1.197, 1.192 | 1.102, 1.119, 1.114 | 4.983, 5.147, 5.269 | 4.960, 4.960, 4.960 |
| hit | 1.343, 1.402, 1.390 | 1.906, 1.928, 1.916 | 1.377, 1.438, 1.457 | 2.034, 1.993, 2.015 |
| composite | 0.316, 0.329, 0.326 | 0.304, 0.308, 0.312 | 0.278, 0.282, 0.279 | 0.256, 0.278, 0.278 |

At the default (half) tier the light pass is under its 4.0 ms bar, light plus temporal under the
6.0 ms lighting line and the a-trous under the 3.0 ms denoise line at both spawns; the temporal
pass is under 0.3 ms at half and 1.0 ms at full. Full resolution misses the light bar and both
lines. The loads at the streamed full runs' starts: 1.31, 0.83, 1.48 (two earlier runs started
at 2.30 and 2.36 were re-read). These replace the flat-list readings of d367d8f (light at half
1.845 to 1.873 level, 2.897 to 2.915 ms streamed; reuse adds about 0.5 ms at half).

Light extraction over the autopilot's flight at the defaults (`--autopilot --headless --seed
1`, 1280 x 720, 6868 frames, load 0.90 at the start): p50 0.150, p99 2.210, max 6.123 ms,
against Phase 7's p99 1.0 and max 3.0 ms (missed, as d367d8f's p99 2.255 and max 6.693 ms);
longest wait inside the radius 0 ms; peak 18 522 samples, 0 evicted, 0 refused.
`Frame::frame_bytes` at 1080p is unchanged: 195 379 200 B at half, 308 736 000 B at full.

**The coarse shadow walk as the default (dev reference, Plan 0024 after Phase 15, the release
binary of the plan's closing tree, RTX 3080 Laptop GPU, 2026-10-22, exclusive perf lock, each
run started at a 1-minute load of 1.77 to 1.98 with other sessions' background load; 1080p,
the defaults: half lighting, coarse shadow walk, reservoir reuse, edit-box history; median of
the last 30 frames, three runs):**

| pass (ms) | level | streamed |
|---|---|---|
| light | 1.972, 1.996, 1.994 | 2.810, 2.650, 2.686 |
| temporal | 0.135, 0.136, 0.135 | 0.135, 0.134, 0.135 |
| light + temporal | 2.107, 2.132, 2.129 | 2.945, 2.784, 2.821 |
| a-trous (variance + 4) | 1.140, 1.150, 1.154 | 1.147, 1.093, 1.103 |

Under the 4.0 ms light bar, the 6.0 ms lighting line and the 3.0 ms denoise line at both
spawns; the coarse walk takes 0.4 to 0.6 ms off the full walk's light pass at half.

**Both lighting tiers at 1080p output (dev reference, indicative, Plan 0024 Phase 10, RTX
3080 Laptop GPU, exclusive perf lock, 1-minute load 2.2 to 6.1 with the owner using the
machine, so not quiet; defaults otherwise: full shadow walk, the flat list, edit-box
history, the filter on, the ore emissive, the lamp on; one run each, median of the last 30
frames):**

| pass (ms) | level, full | level, half | streamed, full | streamed, half |
|---|---|---|---|---|
| hit | 1.263 | 1.332 | 1.833 | 1.861 |
| light | 7.519 | 1.876 | 11.535 | 2.878 |
| temporal | 0.509 | 0.137 | 0.551 | 0.135 |
| a-trous (variance + 4) | 4.726 | 1.168 | 4.538 | 1.106 |
| composite | 0.255 | 0.310 | 0.277 | 0.295 |
| frame | 14.261 | 4.818 | 18.745 | 6.280 |

Against the lines: lighting (light + temporal) 2.0 and 3.0 ms at half resolution under 6.0,
8.0 and 12.1 ms at full over it; denoise (a-trous) 1.1 to 1.2 ms at half under 3.0, 4.5 to
4.7 ms at full over it. The quiet-box readings are in "Quiet-box readings at the lighting
defaults" above.

**The probe volume's two passes (dev reference, Plan 0029 Phase 4, the release binary of
83ae737's tree, 2026-10-22, exclusive perf lock; 1920 x 1080, the defaults with indirect light
on (16-voxel spacing, 4096 probes x 64 rays a frame), lamp on, ore emissive; median of the last
30 frames, three runs; the streamed spawn `--frames 3000`, the level `--level --frames 300`;
1-minute load at each run's start in brackets). The slow adapter is selected with
`VK_DRIVER_FILES=/usr/share/vulkan/icd.d/radeon_icd.json`, which leaves the Vulkan loader only
RADV:**

| adapter, spawn, lighting | probes (update) ms | gather ms | probes + gather ms | light + temporal + probes + gather ms |
|---|---|---|---|---|
| RTX 3080 Laptop, streamed, half [6.2, 4.2, 2.8] | 1.356, 1.390, 1.392 | 0.269, 0.271, 0.271 | 1.625, 1.661, 1.663 | 4.417, 4.454, 4.483 |
| RTX 3080 Laptop, streamed, full [2.3, 1.7, 2.1] | 1.607, 1.589, 1.568 | 1.154, 1.141, 1.129 | 2.761, 2.730, 2.697 | 15.064, 14.952, 14.891 |
| RTX 3080 Laptop, level, half [1.1, 2.4, 2.3] | 1.350, 1.339, 1.351 | 0.239, 0.234, 0.237 | 1.589, 1.573, 1.588 | 3.711, 3.661, 3.698 |
| RTX 3080 Laptop, level, full [2.1, 2.0, 1.8] | 1.599, 1.592, 1.606 | 1.052, 1.053, 1.066 | 2.651, 2.645, 2.672 | 12.104, 12.095, 12.211 |
| Radeon (Cezanne, RADV), streamed, half [1.6, 0.3, 0.7] | 7.832, 7.764, 7.760 | 1.546, 1.546, 1.546 | 9.378, 9.310, 9.306 | 31.188, 31.126, 31.109 |
| Radeon (Cezanne, RADV), level, half [0.3, 0.4, 0.4] | 7.081, 7.080, 7.088 | 1.341, 1.341, 1.342 | 8.422, 8.421, 8.430 | 24.393, 24.398, 24.413 |
| Radeon (Cezanne, RADV), streamed, full [0.4, 0.1, 0.1] | 7.746, 7.781, 7.747 | 6.015, 6.016, 6.015 | 13.761, 13.797, 13.762 | 99.649, 99.749, 99.613 |

Bars (the owner, 2026-10-22, at these readings): the two passes at most 1.7 ms on the dev box
at the streamed spawn, read 1.63 to 1.66 ms; the lighting line (light, temporal and the two
passes) is read at the default half tier, at most 6.0 ms, read 4.4 to 4.5 ms streamed. The
full tier is a quality mode with no bar: 14.9 to 15.1 ms streamed, of which light plus
temporal is 12.2 to 12.3. The slow adapter carries no bar.

**The slow adapter at the probe settings (the release binary of db7b057's tree, 2026-10-22,
exclusive perf lock, AMD Radeon Cezanne on RADV selected as above, 1920 x 1080, half lighting,
the streamed spawn `--frames 3000`, median of the last 30 frames, three runs; 1-minute load
7.94 at the start of the 32-ray runs, so they are loaded (indicative), and 0.86 at the start
of the one-in-16 runs):** `Lighting::probe_rays`
and `Lighting::probe_slices` (flags `--probe-rays N`, `--probe-slices N`; defaults 64 rays and
one probe in 8 a frame) trade the update pass's cost.

| setting | probes (update) ms | gather ms |
|---|---|---|
| `--probe-rays 32`, one in 8 | 6.249, 6.222, 6.202 | 1.546 to 1.547 |
| 64 rays, `--probe-slices 16` | 4.162, 4.234, 4.167 | 1.546 to 1.547 |
| 64 rays, one in 8 (the table above, 68b3bdb) | 7.832, 7.764, 7.760 | 1.546 |

**The light-change window (Plan 0029 Phase 8, the release binary of the tree at b51538a plus
Phase 8's code, 2026-10-22, exclusive perf lock, 1920 x 1080, half lighting, indirect on at 64
rays and one in 8, the streamed spawn `--frames 3000`, one run each; 1-minute load at each
run's start in brackets).** A light change opens a window of 16 frames that refresh one probe
in 4 at hysteresis 0. `--lamp-toggle 64` switches the lamp every 64 frames and gives the median
of the last 30 frames outside and inside the windows; "plain" is the run with no toggle,
median of the last 30 frames. The bars are the dev box's: outside a window at most 1.7 ms,
inside at most 3.2 ms. The slow adapter carries no bar.

| adapter | plain probes + gather ms | toggled, outside a window ms | toggled, inside a window ms |
|---|---|---|---|
| RTX 3080 Laptop [1.26, 1.76] | 1.465 (1.226 + 0.239) | 1.515 | 2.403 |
| Radeon (Cezanne, RADV) [1.40, 1.18] | 9.479 (7.933 + 1.546) | 9.298 | 16.476 |

The retake after the merge of Plans 0028 and 0030 (Plan 0029 Phase 10, the release binary of
the tree at a8b20c2 plus Phase 10's code, 2026-10-22, exclusive perf lock, the same flags,
`--lamp-toggle 64`, one run each, the adapter chosen with `--adapter`): RTX 3080 Laptop at a
1-minute load of 1.88 (quiet), outside a window 1.567 ms, inside 2.701 ms; Radeon (Cezanne,
RADV) at 2.62 (loaded), outside 10.177 ms, inside 18.037 ms.

Readings taken during tuning on the same box, 1000 frames,
1-minute load 8 to 14: the update pass read 1.525 ms before the blend skipped inside probes'
texels and the visibility rays under 0.83 in cosine (1.274 after); without the probe rays'
emissive sample it read 0.989 ms and with the blend alone 0.367 ms, so the emissive sample
and its shadow ray are about a third of the pass. Over the autopilot's flight (`--autopilot
--headless --seed 1 --render 1920x1080`, 4429 frames, load 1.57): update p50 1.104, p99
1.370 ms; gather p50 0.107, p99 0.294 ms.

**Denoise: the temporal pass and four a-trous iterations (dev reference, Plan 0024 Phase 6,
RTX 3080 Laptop GPU, exclusive perf lock, 1-minute load 4.2 to 4.6; the default look with the
ore emissive, median of the last 30 frames, 1920 x 1080 full-resolution lighting):** the
a-trous pass (the variance estimate and four 5 x 5 iterations) reads 4.662, 4.712 and 4.726 ms
at the level's spawn (`--level --frames 300`, three runs) and 4.558 ms at the streamed spawn
(`--frames 3000`), over the 3.0 ms denoise line; the temporal pass reads 0.506 to 0.548 ms.
The plan's projection was 1.2 to 2.0 ms for the iterations. Replacing the normal weight's
`pow(x, 128)` by seven squarings moved the reading by less than run-to-run spread (4.673 ms),
so the passes are bound by their texture loads (24 bytes of hit record per tap).

**Light extraction inside `sync` (dev reference, Plan 0024 Phase 2, RTX 3080 Laptop GPU,
Ryzen 9 5900HS, exclusive perf lock, 1-minute load 8.3 to 9.0 from other work, not quiet; the
ore emissive, `SyncStats::light_us`, the render thread's time scanning written `Full` bricks
for exposed emissive voxels and rewriting the chunks' sample ranges):** the streamed spawn's
first sync (`flythrough --headless --seed 1`) extracted 4545 samples from 3527 bricks in 0.718
and 3.253 ms (two runs). Over the autopilot's flight (`flythrough --autopilot --headless --seed
1`, 1885 frames) it read p50 0.602 ms, p99 13.327 ms, max 48.906 ms per frame, on top of the
streaming line's `update + poll + sync` (p50 8.534 ms, p99 94.288 ms in the same run); the
mirror then held 997 740 samples of the 1 048 576 reserved. The level's one upload of 383 608
bricks extracted 417 388 samples in 25.901 ms (0.175 ms with `--emissive-ore off`). The
plan's projection was under 0.1 ms per sync; the readings are over it.

**Streaming line after Plan 0027 (the staging belt, ADR-0022): still a recorded miss of the
1.0 ms line; the plan's own p50 bars are met.** Dev reference, RTX 3080 Laptop GPU, Ryzen 9
5900HS, exclusive perf lock, quiet box (each run started at a 1-minute load under 2 with no
other process above 7 % CPU), `flythrough --autopilot --headless --seed 1 [--look plain]`, the
pre-plan binary (ae58f0c) alternated with the belt binary (843c5a3), three runs each. Plain:
`sync` p50 1.397 to 1.662 ms (pre-plan 3.278 to 3.687 ms; bar 3.0 ms), `update + poll + sync`
p50 1.715 to 1.994 ms (pre-plan 3.325 to 3.783 ms; bar 3.2 ms), `sync` p99 5.9 to 6.3 ms and
max 8.8 to 9.6 ms (pre-plan 8.7 to 12.4 and 15.1 to 35.6 ms); residency p95 258 to 262 ms.
Default: `update_attrs` p99 3.61 to 3.73 ms, carry-over age max 0 ms, residency p95 129 to
131 ms, and the return hash matches in every run. The belt holds 1 to 2 chunks of 4 MiB
(4 to 8 MiB of mapped RAM, §3). A frame-pointer profile of one quiet plain flight puts
`Queue::write_buffer` at no samples and splits `sync` into narrowing (`upload_record`) 43.2 %,
the belt 25.4 %, palette scans 9.9 %, flushes 6.4 %, the fit check 4.8 %, the rest under 4 %
each. **The belt's share is a recorded miss against Plan 0027's under-5 % bar (owner,
2026-10-21):** 20.5 % of `sync` lands at the first locked instruction after the copy into the
mapped chunk and 2.3 % in recording the copies, so it is the cost of writing the bytes into
mapped memory, not of allocating it. Followup, for Plan 0028 or after it: stage the narrowed
voxels straight into the belt slice, or look into the memory type of the belt's chunks.
The first windowed reading of the `frames:` summary (owner, b4b524d, default look,
2560x1440, the overlay shown for 4326 of 5245 frames, so the march timer's readback
serialized CPU and GPU, the mirror at its 3 GiB budget): frame CPU p50 10.223 ms, p99
53.354 ms; `update + poll + sync` p50 4.432 ms, p99 36.548 ms; march GPU p50 6.571 ms; about
30 fps. It is the baseline Plan 0028's windowed flight compares with.

**Streaming after size-classed slots (dev reference, Plan 0028 Phase 4, RTX 3080 Laptop GPU,
exclusive perf lock, 1-minute load 5.90 to 9.53, not quiet; `flythrough --autopilot --headless
--seed 1 --look plain`, two runs alternated with 1d1e67c):** `sync` p50 0.168 and 0.171 ms
against 4.931 and 4.977 ms, `update + poll + sync` p50 0.172 and 0.175 ms against 5.471 and
5.493 ms, under the 1.0 ms line; `sync` p99 29.474 and 29.598 ms against 13.353 and 14.403 ms.
The default look's flight (load 1.49) reads `sync` p50 3.086 ms, p99 29.199 ms, the chunk-local
surface masks computed in it; 1d1e67c's default flight settled at `r_full` 13 (p50 0.186 ms).
On the slow adapter (`--adapter AMD`, load 0.69) the default flight reads `sync` p50 16.001 ms
over 1275 frames.

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

**The showcase cavern at its shots (Plan 0038 close, 2026-10-28, b9f4b08, `flythrough --source
showcase --shot NAME --headless --render 1920x1080 --frames 300 --adapter NVIDIA|AMD`, median
of 30 frames, three runs each, the shipped defaults: lamp off, indirect on at probes 16, our
upscaler at scale 1.5 (1280x720 internal), the contour-sharp look, manual exposure 1.5;
29 806 light samples; exclusive perf lock, 1-minute load 2.2 to 5.9, so loaded and
indicative):** readings, no line. RTX 3080 Laptop GPU: `hall` 5.848 to 5.906 ms (hit 1.96 to
1.97, light 1.29 to 1.30, atrous 0.48, probes 1.15 to 1.16, upscale 0.54), `chasm` 5.627 to
5.645 ms (hit 1.38 to 1.39, light 1.39 to 1.40, probes 1.35 to 1.36), `crystals` 6.549 to
6.613 ms (hit 2.51 to 2.53, light 1.39 to 1.40, probes 1.17 to 1.18). The Radeon iGPU on RADV
(`AMD Radeon Graphics (RADV RENOIR)`): `hall` 44.144 to 44.184 ms (hit 15.8, light 11.6,
probes 8.0, upscale 3.2), `chasm` 42.675 to 42.843 ms (hit 11.0, light 12.3, probes 10.5 to
10.6), `crystals` 49.436 to 49.456 ms (hit 20.4, light 12.5, probes 7.9 to 8.0). Phase 5's
readings at 654d3c6 (no upscaler, 99 326 samples before Phase 6a thinned the ore) read 10.7
to 13.5 ms and 75.7 to 100.3 ms; most of the fall is the 1.5 internal scale, which the merged
default brought. The long rays across the hall keep the hit pass the largest at `crystals`.

**Eye adaptation (Plan 0040 close, 2026-10-28; ADR-0033): the meter in the composite and the
exposure pass, under "Upscale, tone map, post".** `flythrough --level --headless --seed 1
--render 1920x1080 --frames 3000 --upscaler off --adapter NVIDIA|AMD` (1080p internal), the
spawn pose, the default look (`contour-sharp`), per-pass timestamps, median of the last 30 of
3000 frames, release; main 05a3a28 (no meter) against the plan's tip 8a3fff3, exclusive perf
lock, **quiet** (1-minute load 1.71, 1.45, 1.27 and 0.18 at the four runs; a repeat on NVIDIA,
tip then base, at 0.41 and 0.43). NVIDIA GeForce RTX 3080 Laptop GPU (dev reference): composite
0.359 to 0.429 ms (+0.070; repeat 0.360 to 0.413, +0.053), the exposure pass 0.075 ms (0.073);
light 2.078 to 2.134, temporal 0.134 to 0.136, a-trous 1.132 to 1.165 ms (repeat 2.092 to 2.075,
0.135 to 0.135, 1.094 to 1.081: within the spread, as hit's 1.363 to 1.395, which the plan does
not touch). AMD Radeon Graphics (RADV RENOIR, the slow adapter, by `--adapter AMD`): composite
2.869 to 2.975 ms (+0.106 ms), the exposure pass 0.060 ms; light 15.435 to 15.478, temporal
1.133 to 1.136, a-trous 5.196 to 5.212 ms. Meter plus pass: about 0.13 to 0.15 ms on the dev
box and 0.17 ms on the iGPU, above the plan's guesses (0.03 to 0.08 ms) and inside the 1.5 ms
line. Phase 4's loaded readings (dedd83c against e9879cc) are superseded.

**Static emissive light on a long guarded history and probe slices spread by a hash (Plan 0047
Phase 6, 2026-10-28, the close's readings at v0.36.0; ADR-0036, ADR-0023).** `flythrough --level --headless --render 1920x1080
--frames 300 --adapter NVIDIA|AMD --lamp on|off`, the spawn pose, the defaults (`own` 1.5, half
lighting, 64 rays one in 8, emissive history 256, guard on, reuse cap 20), per-pass timestamps,
median of the last 30 frames, release, exclusive perf lock, memory cap, on AC (power "Not
charging" at every run's start and end). The base is the release binary of 93a2c72's tree
(slices by `slot mod slices`, emissive history 32, the guard in); the tip is 67872f8's tree plus
the `--reuse-m-cap` flag. Base and tip runs interleaved, three each. No `VEIN_POSE` run: the
fly-through has no pose flag. Probe update ms per run, base then tip:

| adapter, lamp, load at the runs' starts | probes (update) base | probes (update) tip | median, tip against base | gather |
|---|---|---|---|---|
| Radeon (Cezanne, RADV), off, **quiet** (0.79 to 0.99) | 7.277, 7.334, 7.303 | 7.606, 7.635, 7.610 | 7.610 against 7.303, +4.20 % | 0.574 to 0.575 / 0.572 to 0.573 |
| RTX 3080 Laptop, on (0.96 to 2.17) | 1.171, 1.175, 1.201 | 1.207, 1.189, 1.206 | 1.206 against 1.175, +2.6 % | 0.098 to 0.099 / 0.097 to 0.098 |
| RTX 3080 Laptop, off (1.54 to 1.79) | 1.111, 1.102, 1.105 | 1.120, 1.109, 1.115 | 1.115 against 1.105, +0.9 % | 0.098 to 0.099 / 0.098 |
| Radeon (Cezanne, RADV), on (0.92 to 1.54) | 7.871, 7.969, 7.854 | 8.155, 8.146, 8.140 | 8.146 against 7.871, +3.5 % | 0.599 to 0.600 / 0.600 |
| Radeon (Cezanne, RADV), off (1.06 to 2.32) | 7.635, 7.396, 7.340 | 7.711, 7.652, 7.655 | 7.655 against 7.396, +3.5 % | 0.574 to 0.575 / 0.572 to 0.573 |

The bar (Plan 0047 Phase 4 done-when 6): the update at most 5 % over its base on each adapter,
the Radeon's lamp-off figure read quiet; read +4.20 %. The tip's lighting passes, RTX 3080 Laptop
lamp on: gradient 0.083 to 0.084 ms (bar 0.15), light 0.816 to 0.826, temporal 0.080 to 0.081
(light + gradient + temporal 0.98 to 0.99 ms, bar 6.0), a-trous 0.483 to 0.488 ms (bar 3.0);
lamp off: 0.084 to 0.085, 0.758 to 0.759, 0.080 to 0.081, a-trous 0.486 to 0.488. Radeon, lamp
on: gradient 0.454 to 0.460, light 7.004 to 7.132, temporal 0.618 to 0.626, a-trous 2.371 to
2.398; lamp off (quiet): 0.461 to 0.462, 6.511 to 6.531, 0.612 to 0.622, 2.371 to 2.377
(readings, no bar). The frame's CPU time (the hashed slices' update list is filtered over all
32 768 slots on the CPU each frame), RTX: base 0.439 to 0.461 ms, tip 0.523 to 0.592 ms.
`Frame::frame_bytes` reads 184 956 992 B on both: the slice hash adds 0 B.

**The lab modes removed (Plan 0052 Phase 9, the close's reading at v0.37.0).** The upscaler's
losing reconstructions, fetches, blends and still clips, their `UpTuning` selectors, and the
uniform candidate draw are gone. `flythrough --level --headless --render 1920x1080 --frames 300
--adapter NVIDIA|AMD` at the defaults (`own` 1.5, 1280 x 720 internal, half lighting, the
level's lamp on), per-pass timestamps, median of 30 frames, release, exclusive perf lock, base
`c7ed5a5` (before the phase) and tip `891655a` interleaved, three runs each, 1-minute load 1.28
to 2.45 at the runs' starts. Milliseconds, base runs / tip runs:

| adapter | hit | light | upscale |
|---|---|---|---|
| RTX 3080 Laptop | 0.586, 0.602, 0.594 / 0.592, 0.589, 0.587 | 0.884, 0.913, 0.897 / 0.888, 0.883, 0.881 | 0.540, 0.555, 0.549 / 0.496, 0.494, 0.491 |
| Radeon (Cezanne, RADV) | 4.277, 4.262, 4.339 / 4.265, 4.300, 4.286 | 7.150, 7.024, 7.046 / 7.019, 7.039, 7.029 | 3.273, 3.239, 3.282 / 2.995, 3.009, 3.015 |

Hit and light inside the base's spread on both adapters; the upscale pass about 10 % lower on
the RTX and 8 % lower on the Radeon. **Drift, not this plan's:** on this command line the RTX
light pass has read 0.88 to 0.91 ms since before Plan 0052 (the `c7ed5a5` base), 7 to 8 % above
Plan 0047 Phase 6's lamp-on 0.816 to 0.826 ms above; the Radeon reads inside 0047's spread. The
bisect between 0047's close and `c7ed5a5` is owed (Plan 0052's Followups). No hit-pass figure
for this command line existed before this reading.

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

**The contour-sharp default leaves the mirror's bytes unchanged (Plan 0039's fix pass, the
merged binary of 4d68e91 against main's binary of dedd83c, both adapters):** the level reads
157 872 640 B live and 259 203 264 B allocated under both, and the streamed flight's full
residency (17 077 full and 40 700 LOD-only chunks) 871 679 852 B live and 3 221 014 080 B
allocated under both; the flights' peaks differ with the flight (882 016 044 to 996 021 932
B live across both binaries).

**The frame's targets (Plan 0024 Phase 11, `Frame::frame_bytes` printed by `flythrough
--autopilot --headless --seed 1 --render 1920x1080`):** 308 736 000 B with full-resolution
lighting and 195 379 200 B at half resolution, against the 1.5 GB line; the light-sample buffer
adds 16 MiB plus its 16 KiB candidate table (2^20 samples of 16 B), the blue noise 16 KiB and
the emission scale 256 KiB. Per pixel at full resolution: the hit buffer twice (48 B), the
translucent layers 8 B, the irradiance 8 B, the accumulation and moments twice (32 B), the
filter's pair 16 B, the reservoirs twice (32 B), the output 4 B, and the gradient strata 8 B
per 9 pixels. The plan's projection was about 276 MB.

**The probe volume (Plan 0029 Phase 4, 83ae737's tree, the same autopilot command):**
`Frame::frame_bytes` reads 254 216 224 B at 1080p with half-resolution lighting, 58 837 024 B
above Plan 0024's 195 379 200: the atlases 50 331 648 B, the state 131 072 B, the update list,
ray buffer and uniform 4 227 104 B (the volume, 54 689 824 B, printed as `autopilot: probe
volume`), and the gather's lighting-grid target 4 147 200 B (8 B per lighting pixel). The
plan's bar (amended 2026-10-22, owner) counts all of it: 58 837 024 B and no more, against the
1.5 GB line.

**The frame's targets with the upscaler (Plan 0030 Phase 5, `Frame::frame_bytes`, 1080p
output):** 124 621 440 B at scale 1.5 (1280 x 720 internal): the 86 835 840 B of Plan 0024's
frame at 1280 x 720, plus the composite's `rgba8` target grown to the output size (4 608 000
B) and two output-size `rgba16float` histories (33 177 600 B); 228 556 800 B at scale 1 with
the jitter on (195 379 200 B of the frame plus the histories); 195 379 200 B at scale 1 with
the jitter off, as before. Plan 0030 Phase 8 (12c6c57): unchanged at 124 621 440 B at scale
1.5; the light-change clip adds no target (the upscale pass's settings are a 96 B uniform).

**The frame's targets under FSR 2 (Plan 0041 Phase 6, 6e64744's tree, `Frame::frame_bytes`,
1080p output, the probe volume included as the fly-through runs it):** our upscaler at 1.5
reads 181 154 464 B (Plan 0030's 124 621 440 B plus the probe volume and its gather target,
56 533 024 B at that lighting size); FSR 2 at 1.5 reads 305 574 892 B, which is ours less
the two output-size histories (33 177 600 B, not allocated in FSR mode) plus FSR 2's
inventory, 157 598 028 B: 66 355 200 B at the render size (72 B a pixel: the rgba32float
colour input, depth, motion, the masks, the reconstructed previous depth, the dilated depth,
two dilated motions, the lock luma, the prepared colour and the dilated masks), 91 238 400 B
at the output size (44 B a pixel: the rgba16float upscaled colour twice, which RCAS also
writes into, the lock status twice, the luma history twice and the new locks) and 4 428 B
for the luminance pyramid's two stored mips, the exposure texels and the 1 x 1 debug weight.
`frame_bytes_under_fsr2_swap_the_histories_for_fsr2s_inventory` asserts the sum. FSR 2 at
scale 1 reads 494 763 692 B; Plan 0024's frame with the probe volume 254 216 224 B.

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
  Since Plan 0027 (ADR-0022, closed 2026-10-21) the mirror also holds its staging belt's
  mapped chunks in host RAM: 1 to 2 chunks of 4 MiB in flight (§2's quiet runs), at most
  `BELT_KEEP_CHUNKS` (3, 12 MiB) between syncs, a transient burst of about 56 chunks during
  the bounded level's unbudgeted first sync, released once its copies complete.

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

**Size-classed slots, projected (arithmetic, not a reading; Plan 0028 Phase 1, `world_budget
--chunks 20x20x8 --census` and `--stream R,24 --surface`, cave seed 1, dev box).** A `Full`
brick's slot is 64-byte units (mask, header, 0/1/2/4 plane units or 16 of raw ids: 2, 3, 4, 6
or 18 units) in bins of 2, 3, 4, 6, 8, 12, 16, 20, 24 and 36 units; a record is 2112 B. With
contour attributes a full chunk's brick adds a surface-mask unit and `ceil(n / 32)` units for
its `n` chunk-local surface voxels (an air face neighbour inside the chunk, or the chunk's
face). The level: pool 56 075 200 B, records 6 758 400 B, 62 846 720 B with table and rows
(232 436 992 B live today). Streamed, at full residency (57 777 records):

| residency | plain look | contour look | surface voxels (chunk-local / exact plane) | attribute units per brick |
|---|---|---|---|---|
| 12/24 | 409 969 196 B | 563 102 380 B | 37 518 244 / 23 899 197 (1.570) | 3.07 |
| 14/24 | 451 317 548 B | 700 607 596 B | 61 043 072 / 39 015 306 (1.565) | 3.08 |
| 16/24 | 502 928 492 B | 871 679 852 B | 90 331 692 / 57 703 350 (1.565) | 3.08 |

**Size-classed slots, read (dev reference, Plan 0028 Phase 2, RTX 3080 Laptop GPU, exclusive perf
lock, 1-minute load 5.2 to 8.8).** The level (`flythrough --level --headless --seed 1 --frames 1
--look plain`, and `apps/flythrough/tests/level_mirror.rs` on lavapipe) holds 62 846 720 B live
(56 075 200 B of 2-, 3- and 4-unit slots, 3200 records of 2112 B, the table, the rows) under an
88 285 376 B reservation, against 232 436 992 B live before; its first sync uploads 63 027 696 B
in 405 writes in 342 ms against 500 ms for the pre-plan binary (1d1e67c, 232 617 968 B in 435
writes) in the same session, lights included (65 to 67 ms in both). The streamed plain spawn at
full residency (16/24, `--frames 3000`) holds 502 928 492 B live against 1 786 641 644 B, as
projected.

**Surface-only contour attributes, read (dev reference, Plan 0028 Phase 3, RTX 3080 Laptop GPU,
exclusive perf lock, 1-minute load 9.5 to 10.2).** The level in the default look (`flythrough
--level --headless --seed 1 --frames 3 --look contour`) holds 157 872 640 B live (680 491 136 B
before) under a 259 203 264 B reservation; its first sync uploads 158 053 616 B in 826 writes in
687 and 690 ms against 652 and 659 ms for 1d1e67c (232 617 968 B), the chunk-local surface
masks computed on the render thread; the attribute pass's initial fill of its 383 608 bricks
reads 34.7 and 47.2 ms against 55.2 ms. The default autopilot flight (`--autopilot --headless
--seed 1`, same load) keeps `r_full` 16 with no radius change and no deferral, peak pressure
0.6447 (records; units 0.3027), peak 1 003 260 588 B live, where 1d1e67c lowered `r_full` to 12
at pressure 0.911 with 2 864 318 452 B live.

**The streamed default at `r_full` 16 (dev reference, Plan 0028 Phase 4, RTX 3080 Laptop GPU,
exclusive perf lock, 1-minute load 1.49 to 9.53 from the other lanes' work; `flythrough
--autopilot --headless --seed 1`, paired with 1d1e67c).** The default look reaches full
residency at 16/24 (17 077 full, 40 700 LOD-only chunks) with `r_full` 16: 871 679 852 B live
there, peak 941 911 020 B over the flight, peak pressure 0.6514 (records; units 0.2701), no
radius change, no sync with a deferral, return hash matching; 1d1e67c settled at `r_full` 13
with 2 864 309 692 B live, pressure 0.911 and four radius changes. Plain: 502 928 492 B at
full residency, peak 532 183 724 B, against 1 786 641 644 and 1 890 756 140 B. Fragmentation at
full residency (the fly-through's `bins:` line): 11 908 spare units of 11 705 061 live (0.10 %)
in the default look over six bins (2, 3, 4, 6, 8 and 12 units), 3817 of 5 943 321 (0.06 %) in
plain over two; the same figures on the slow adapter's runs.

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
plain` holds 16 (102 m). Since Plan 0028 (surface-only attributes, below) the default holds
`r_full` 16 (102 m) at 0.87 GB live with pressure 0.65: the "about 100 m" figure is met in both
looks on the dev reference.

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

**The showcase cavern beside the level (Plan 0038 close, 2026-10-28, b9f4b08, `world_budget
--source showcase --census` and `--chunks 20,20,8 --census`, seed 1, release, the dev
reference; arithmetic from the generated worlds, no timing):** the showcase's 1176 chunks hold
71 046 `Full` bricks (26 187 of 1 material, 44 859 of 2) against the level's 383 608 in 3200
chunks: CPU bytes 17 620 080 against 60 807 552, mirror bytes 14 453 792 against 62 846 720,
and the fly-through's bounded budget (`level_mirror_budget(world, chunks, contour)`, dig
headroom included) 134 604 544 B against 259 203 328 B. Live in the fly-through: 33 003 744 B
of mirror in 134 604 480 B allocated. Light samples 29 806 of the 1 048 576 reserved (amber
3792, crystal 13 497, amethyst 1552, ember 10 965; the fullest chunk 2880 of its 8192 cap), so
no new VRAM. Phase 5's census at 654d3c6, before Phase 6a moved the ore, read 71 282 `Full`
bricks and 99 326 samples.


**Stats HUD (arithmetic, Plan 0010; ADR-0009).** The overlay holds about 130 KiB of VRAM
under "everything else": a 128 KiB cell buffer at the 256 x 128 cap layout (`u32` per cell),
a 1 KiB sample buffer (256 `f32`), the font and a small parameter block, under the 200 KiB ADR-0009 states.

**Eye adaptation (Plan 0040 close, 2026-10-28; ADR-0033).** `Frame::frame_bytes` on the level
at 1920x1080 internal (same quiet runs as §2's eye-adaptation reading, both adapters): 254 216 224
B at main 05a3a28, 254 476 352 B at 8a3fff3, +260 128 B: the meter's 240 x 135 tiles (259 200 B),
the 32-byte state and its uniform copy, the pass's 96-byte parameters and three 256-byte
read-back buffers. The plan projected +0.26 MB.

**The emissive accumulation in `f32` (Plan 0047 Phase 2, the close's reading at v0.36.0; ADR-0036).** The temporal pass's
accumulation pair is `rgba32float`, 16 B more per lighting pixel. `Frame::frame_bytes` printed by
`flythrough --autopilot --headless --seed 1 --render 1920x1080` (RTX 3080 Laptop, exclusive perf
lock, 1-minute load 7.7, loaded): 184 956 992 B at the default (`own` 1.5, half: 640 x 360
lighting), +3 686 400 B over the merged base (the FSR 2 entry's 181 154 464 B plus the eye
adaptation's meter at 1280 x 720, 116 128 B); 262 770 752 B at scale 1, half (960 x 540
lighting), +8 294 400 B over the eye adaptation's 254 476 352 B. The plan's bar is 16 B per
lighting pixel at most.

**The probe volume on demand (Plan 0052 Phase 4, the close's reading at v0.37.0).** A `Frame`
builds its `ProbeVolume` only on the first encode with indirect light on (or on
`probes()`, `probes_mut()` or a spacing change), so the default frame (indirect off) no longer
holds it. The volume is 54 689 824 B: 12 615 680 + 1 056 768 words of atlases, state and scratch
(54 689 792 B) and its 32-byte uniform. **Reading** (`crates/vxe-render/tests/frame.rs`,
lavapipe, a 64 x 64 frame, exclusive perf lock, 1-minute load 2.2, `1fdac4d`): `frame_bytes`
412 008 B with indirect off, before an encode and after one; 55 101 832 B after an encode with
it on, +54 689 824 B, equal to `ProbeVolume::bytes`; the test asserts the figure. The
fly-through turns indirect light on by default, so its frame is unchanged: `flythrough
--autopilot --headless --seed 1 --render 1920x1080 --adapter NVIDIA` (release, RTX 3080 Laptop,
exclusive perf lock, 1-minute load 9.3 to 17.8, loaded; the close review on `200607c`) prints
frame targets 184 956 992 B, the 0047 figure above, of which the probe volume is 54 689 824 B
(atlases 50 331 648 B, state 131 072 B, update list, rays and uniform 4 227 104 B, spacing 16).
With `--indirect off` it would hold 130 267 168 B (arithmetic, not read). The saving is every
engine `Frame` at `Lighting::default()`, the render tests' host RAM among them.

**Prefabs (arithmetic, Plan 0008; no reading needed).** A `Prefab` holds 2 B of material plus
one mask bit per voxel: a 16³ prefab is 8 KiB + 512 B, a 64³ one 512 KiB + 32 KiB. A parsed
`.vox` model is capped at 256³ per axis (`MAX_MODEL_EDGE`), so the largest `to_prefab`
allocation is about 50 MiB including its index buffer. `StampedSource` holds one rotated copy
of the prefab per placement, so its RAM grows with the placement count, not the prefab count.
**Reading (Plan 0042 Phase 4, 1474e59, `--castle`, release, RTX 3080 Laptop GPU, Ryzen 9
5900HS, exclusive perf lock, 1-minute load 2.4 to 13.0, so loaded and indicative):** the
generated castle keeps 21 of its 27 tiles (600 x 600 x 500 voxels split at 256): its prefabs
hold 343 566 464 B (the sum of `2n + 8 ceil(n / 64)` over the kept tiles), of which the
castle's listed voxels are 13 154 879 (about 8 % of the kept tiles' 161 678 336 voxels; the
rest is the air in each tile's box). The encoded `.vox` is 52 623 529 B. Process peak RSS (`VmHWM`,
streamed seed 1, headless, `--frames 3000`, 1280 x 720) is 1 242 440 to 1 331 404 kB
with the castle against 949 184 to 986 124 kB without it, three runs each: about 260 to 350
MiB more, the prefabs' 328 MiB plus the build's transients. Chunk generation over the 990
chunks the castle meets (`examples/castle_gen`, single thread, three runs, load 9.1 to 6.3):
mean 423.5 to 427.9 us and p99 2258 to 2380 us per chunk with the castle, against 186.2 to
188.6 us and 1650 to 1710 us for the hall alone.

## 4. Edits

- **Cost:** 100 sphere carves of radius 1 m (exactly 4169 voxels each at 10 cm, the lattice
  count on voxel centres; the continuous volume is about 4189) in one frame cost at most 2.0 ms
  of CPU on the dev reference (raised to 2.2 ms by the owner on 2026-10-20 after the quiet-box
  re-read below, and returned to 2.0 ms on 2026-10-21 after Plan 0031's quiet-box reading;
  **met on the dev box**, no baseline-GPU reading) including result counting and dirty marking, and their upload (up to about 6400 bricks,
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
  **Quiet-box reading at the Plan 0031 tip (dev reference, AMD Ryzen 9 5900HS, on AC with the
  performance profile and governor, 8dad5ca, exclusive perf lock, 1-minute load 0.48 to 1.98
  and no other process above 7 % CPU before every run, three runs alternated with 2796062's
  tree, criterion medians):** `worst` 0.766, 0.774 and 0.774 ms against 1.318, 1.316 and
  1.337 ms; `worst_warm` 0.765, 0.775 and 0.776 ms against 1.316, 1.319 and 1.338 ms;
  `mixed` 0.622, 0.628 and 0.630 ms against 1.044, 1.047 and 1.055 ms; `tool` 3.585, 3.578
  and 3.590 ms against 4.282, 4.194 and 4.217 ms; `tool_warm` 3.585, 3.586 and 3.590 ms
  against 4.284, 4.195 and 4.219 ms. The fly-through `B` line (`--level --seed 1 --headless
  --bench 1 --frames 1`) reads **1.00, 0.99 and 0.99 ms** against 1.49, 1.49 and 1.54 ms for
  2796062 in the same window, the same carve counts on both. 2796062 read 1.79 ms `worst`
  and a 2.06 to 2.09 ms `B` line at the re-read above; the gap to its 1.32 and 1.49 to 1.54
  ms here is machine state, not code. The planar kernel is unchanged; the op path builds each
  brick's shape words straight from row spans solved once per op (Plan 0031). On this reading
  the owner returned the line to **2.0 ms on the dev box** (2026-10-21): the `B` line meets it
  with about 1 ms to spare. No baseline-GPU reading.
  **Span arena (Plan 0052 Phase 5, c130daf; bar amended by the owner 2026-10-30: `tool` and
  `tool_warm` within 2 % of the base, `tool_long` not above it).** `OpRows` keeps row spans in
  one arena of slabs, one per brick row it reaches, instead of a table over the op's whole
  bounds: a diagonal capsule of 999 voxels at radius 2 writes 13 828 span entries against the
  338 724 rows of its bounds (the arena is reserved for the bounds, about 5.4 MB of address
  space, and only the reached rows are written). **Readings** (dev reference, AMD Ryzen 9
  5900HS, `cargo bench -p vxe-world --bench edits -- "^tool"`, exclusive perf lock, criterion
  medians; base binary built from `a392e47`'s `edit.rs` with the current bench file): quiet run,
  tip then base, 1-minute load 1.47 to 1.97: tip `tool` 4.8885 ms, `tool_warm` 4.8714 ms,
  `tool_long` 5.8762 ms; base 4.8221, 4.8096 and 5.8850 ms. `tool` +1.4 %, `tool_warm` +1.3 %
  (their intervals do not overlap), `tool_long` -0.1 %: within the amended bar. The base's
  bounds-wide table is the waste removed; a short capsule pays the arena's indexing for it.
  `tool_long` is new in this plan (eight radius-2 capsules along the space diagonals, about 430
  voxels long).
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
  **Yielding drop and rubble pile (proposals, Plan 0022, ADR-0016):** `drop_region_through` of
  the 9-cell slab through a 32-voxel sliver under the slab's 100 µs; of the 4096-cell cube at
  yield 64 under the cube's 20 ms; `pile_rubble` of 32 on the 9-cell slab under the removal's
  200 µs (a pile is the removal's second half). **Readings (dev reference, AMD Ryzen 9 5900HS,
  Linux, same bench, criterion medians, exclusive perf lock, at 7e42bbf, 1-minute load 2.2 to
  6.8, three runs):** slab through the sliver 91.5 to 94.7 µs; yielding cube 7.4 to 10.6 ms;
  pile 35.4 to 37.9 µs. In the same lock hold, paired with the pre-plan build (ec99160, load 4.5
  to 9.1), the plan build reads within noise of or under it on every older arm:
  `find_unsupported` 337 to 395 µs against 393; slab by 32 78 to 82 µs against 84 to 89; by 10
  57 µs against 62 to 68; `remove_region` 154 to 160 µs against 153 to 161; cube by 64 7.4 to
  8.7 ms against 10.2 to 11.0. Every reading is under its proposal; the proposals stand as
  budgets.
- **Logs (Plan 0007, Plan 0018):** an op log entry is 47 bytes for a sphere carve (65 for a box
  fill, 75 for a capsule replace), 34 for a stamp (whatever the prefab's size: it names the
  prefab by content id, ADR-0014), `21 + 12n` for a drop of `n` cells and `31 + 12n` for a
  removal (49 173 bytes for a 4 096-cell drop), `23 + 12n` for a drop through sub-threshold
  cells (kind 5, Plan 0022: 131 bytes for 9 cells, 49 175 for 4 096) and `27 + 12n` for a
  rubble pile (kind 6: 135 bytes for 9 cells); a boundary entry 13, an input entry 14 bytes
  plus its payload. A log file adds a header of 60 fixed bytes, a 4-byte prefab count, and
  per prefab its stamps name an 8-byte id and its content (`12 + 2n` bytes for `n` voxels plus
  8 per 64 voxels of mask: 8 716 for the 16³ vault), once per prefab however often it is
  stamped. A falling region re-sends its cells every tick it drops (129 bytes a tick for a
  9-cell slab, 131 as kind 5), and a fall that crushed anything adds one kind-6 pile entry at
  its end. All asserted in `crates/vxe-world/tests/log.rs`.
- **Replay throughput:** no budget; a reading only, bound by the edit cost above (the ops
  dominate, not the decode). **Reading (dev reference, AMD Ryzen 9 5900HS, Plan
  0007 Phase 6, exclusive perf lock, 1-minute load 1.45):** `cargo bench -p vxe-world --bench
  replay`, decode plus replay of a 470 060-byte log file of 10 000 radius-1 m carves on a
  solid 4³-chunk world: 279.07 ms (CI 278.48 to 279.70 ms), about 35 800 ops per second.
  **Quiet-box reading at the Plan 0031 tip (8dad5ca, same machine and protocol as §4's Plan
  0031 reading, three runs alternated with 2796062):** 61.29, 61.29 and 61.20 ms (about
  163 000 ops per second) against 103.16, 102.64 and 103.19 ms.
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
  **Quiet-box reading at the Plan 0031 tip (8dad5ca, same machine and protocol as §4's Plan
  0031 reading, three runs alternated with 2796062):** through `EchoLogic` 0.9284, 0.9280 and
  0.9264 ms; direct carves with grid updates 0.8581, 0.8647 and 0.8573 ms; direct carves alone
  0.7809, 0.7801 and 0.7843 ms. The overhead is **70.3, 63.3 and 69.1 µs per 100 inputs**
  against 64.7, 63.5 and 70.4 µs for 2796062 in the same window (1.4913 / 1.4266 / 1.3377,
  1.4821 / 1.4186 / 1.3326 and 1.4871 / 1.4167 / 1.3384 ms); the grid updates 77.2, 84.6 and
  73.0 µs against 88.9, 86.0 and 78.3 µs.
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
- **After Plan 0023 (same machine and protocol, quiet re-read, 2026-10-21):** plain p95 403
  and 385 ms over the flight, under the 500 ms proposal (the plan's 381 ms bar, set at one
  pre-plan reading, missed by 22 and 4 ms; the pre-plan binary read 383 ms in the same
  session). The default reads 188 and 191 ms. In the owner's windowed default flight the mirror
  sat at its 3 GiB budget with `r_full` 13 and deferral streaks of up to 2015 syncs, which
  ADR-0012's mirror-slot plan owns.

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
  drops and a removal, hashing every result and the world after each step; Plan 0022 adds a
  slab dropped through a sliver and its rubble pile) pins `0xd223_d848_c8fc_ca0f`, blessed on
  Linux; the Windows arm confirms it on the first CI run after the owner pushes `main`. The
  course's first two parts still hash to the earlier `0x3178_0a15_61c1_3041`. The ops are
  integer-only, so a mismatch is a defect, fixed forward.
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
  **Reading (Plan 0042 Phase 4, 1474e59, `--castle`, streaming seed 1 headless, release, RTX
  3080 Laptop GPU, exclusive perf lock, 1-minute load 2.4 to 13.0, loaded):** the castle's
  build runs before the first chunk, single-threaded but for the tiles: generate 86 to 138 ms
  (tiles on the machine's threads), encode 30 to 44 ms, parse 43 to 64 ms, map (palette to
  prefabs) 405 to 492 ms, place (`StampedSource::new`, a rotated copy per prefab) 37 to 82 ms,
  about 0.7 s in all over 18 runs. First frame 1087 to 2638 ms with the castle (1105 to 1303
  ms at the gate shot) against 510 to 559 ms without it; full residency (17 077 full, 40 700
  LOD-only chunks) at 8735 to 11 200 ms with it against 9343 to 9396 ms without. The census
  at full residency: 1 547 872 `Full` bricks with the castle (its materials in 295 bricks of
  3 to 4 materials and 407 with glass) against 1 640 292 without, the hall replacing cave
  surface; mirror 841 816 812 B live against 871 679 852 B.
  **Reading (Plan 0038 Phase 5, 654d3c6, `flythrough --source showcase --headless --frames
  60`, release, RTX 3080 Laptop GPU, three warm runs, exclusive perf lock, 1-minute load 8.9,
  loaded):** generation of the 1176 chunks 281 to 300 ms (71 282 `Full` bricks), device ready
  443 to 470 ms, the first sync 83 to 86 ms for 33 554 128 B, first frame 664 to 700 ms. The
  first sync's light-sample extraction (`SyncStats::light_us`) 8.297 to 8.812 ms for 16 471
  samples from 21 484 bricks, the sync's sample budget (16 384 plus the last chunk); the rest of
  the 99 326 follow over six more syncs (29 806 in two syncs since Phase 6a; the close's first
  sync read 4.269 ms for 16 564 samples). Headless, so no window or surface.
  **Reading (Plan 0038 Phase 6, e4b43bb, `flythrough --source showcase`, windowed, release, RTX
  3080 Laptop GPU, exclusive perf lock per run):** cold (the dGPU runtime-suspended before
  each) 2077, 2047 and 2081 ms at load 1.24, 0.68 and 7.56 (the last loaded); warm 671, 660
  and 701 ms at load 7.12, 6.71 and 6.17 (loaded). Under the 3 s line. The session's first run
  (load 1.50, not one of the six) read 4015 ms, over the line; it was not reproduced in the
  next three (689 to 742 ms) and its cause was not read.
 of a one-crate change under 10 s on the dev reference; the
  everyday loop does not compile crates a plan does not touch (`default-members`).
  **Reading (Plan 0011 Phase 4, dev reference, AMD Ryzen 9 5900HS, `CARGO_BUILD_JOBS=3`,
  exclusive perf lock, 1-minute load 1.99 to 1.91):** default-members `cargo build` after a
  one-line change (a `pub const`) to `crates/vxe-world/src/lib.rs`, alternated three times:
  1.40 to 1.49 s on the tree before Plan 0011 (066d90e), 1.46 to 1.48 s at its tip (70b40ec
  plus the bench). The default members compile no `wasmtime`: `cargo tree -e normal,build`
  of them and `cargo tree -p flythrough -e normal` list none; `flythrough --features wasm`
  brings it in. Under the 10 s budget, and unchanged at the Plan 0011 close: the default build
  still compiles no `wasmtime` or `tokio`.
- The test gate (Plan 0043): a dev phase runs `cargo nextest run --workspace --profile quick`
  (CPU only, no perf lock), adds `--profile gpu` under the lock when it touches render paths,
  and a close or a merge runs `--profile full`, CI its `ci` twin (the full tests, the GPU group unthrottled) (`.config/nextest.toml`). Proposal: quick
  under 60 s on the dev reference, so a phase gate holds no lock and blocks no other lane.
  **Reading (Plan 0043 close review, 5877b8a, 599 tests, dev reference, Ryzen 9 5900HS and RTX
  3080 Laptop GPU on lavapipe, built tree, `--test-threads 2`, other lanes running, so all
  loaded):** quick 470 tests in 45.4 s, no lock, 1-minute load 6.40 to 8.80; gpu 124 tests in
  201.5 s, exclusive perf lock, load 9.51 to 2.53; full 599 tests in 327.1 s, exclusive perf
  lock, load 7.90 to 3.71. The dev lane read 44.8, 205.2 and 362.7 s at 8059d41, also loaded.
  Before the tiers every phase gate was a full run under the lock, 4 to 7 minutes in the
  lanes' logs at 87e4b67. The CI Linux job time against run 39808788735's 981 s is owed by the
  first CI run after the merge into main (the Phase 3 amendment).
  **Reading (Plan 0052 close review, 200607c with main 7ef72e6 merged, 745 tests, dev
  reference on lavapipe, `CARGO_BUILD_JOBS=4`, `--test-threads 2`, exclusive perf lock, a 6 GiB
  memory cap, built tree):** full 745 run, 744 passed, 1 failed (main's
  `showcase_hall_golden_is_lit_by_its_four_ores`, owed to Plan 0050 Phase 6), 12 skipped,
  **6040.6 s**, 1-minute load 3.94 at the start and 6.84 at the end with another project's
  processes running (loaded). Plan 0040's close read 690 tests in 1440 s at load 3.5; the slow
  list (Plan 0047's long emissive runs and the 1600-frame indirect golden among them) is most of
  the difference, but the run printed no per-test times, so the split is not read.
  **The two control renders (Plan 0052 Phase 6).** The indirect golden's control pass renders
  64 frames (was 1600) and the showcase golden's dark pass 8 (was 64). Quiet readings, each test
  alone, lavapipe, exclusive perf lock, load at each start 1.48 to 12.18 (lavapipe's own threads
  from the run before): `indirect_golden_shows_the_vein_s_bounce_light` 151.57 s at the base,
  115.70 s at the tip (-24 %); `showcase_hall_golden_is_lit_by_its_four_ores` 17.90 s and
  14.56 s (-19 %). The showcase golden left the slow list. Re-read at the close review alone at
  load 6.15 to 14.11 (loaded): indirect 149.0 s, showcase 16.5 s.
- Pipeline creation, one compute pipeline per pass the renderer builds (attrib, the march hit
  pass and the nine march-prefixed passes), read by
  `cargo run -p vxe-render --release --example pipeline_times` (cold is the first creation on
  a fresh device, min the fastest of three). No budget; part of time to first frame.
  **Reading (Plan 0046 Phase 1, on ce62f0c, Linux on Vulkan, driver shader caches off with
  `MESA_SHADER_CACHE_DISABLE=true` and `__GL_SHADER_DISK_CACHE=0`, adapter by
  `WGPU_ADAPTER_NAME`, exclusive perf lock, 1-minute load 10.4 to 11.1, so loaded):** sum of
  the eleven cold creations 1396.1 ms on the RTX 3080 Laptop GPU (light 522.6, march 312.3,
  gradient 265.1, attrib 75.3), 458.4 ms on the Radeon iGPU on RADV (light 152.7, march 90.9,
  gradient 70.8, attrib 54.2), 299.0 ms on lavapipe (light 96.3, march 53.9, attrib 32.4).
  Microsoft's Linux `dxc` 1.9.2609 on naga's Shader Model 6.0 HLSL of the same passes,
  `-HV 2018 -Ges` as wgpu passes them, wall time per process, at load 8.95: attrib 437 ms,
  march 283, light 517, gradient 390, upscale 111, the other six 46 to 81.
  **Reading (Plan 0046 close, Windows DX12 on WARP, the `windows-latest` CI arm, a software
  rasterizer, run 39843578678 on f3ed4a3):** sum of the eleven cold creations 76 623.6 ms
  under FXC (`WGPU_DX12_COMPILER=fxc`; attrib 42 412.2, light 18 050.1, march 10 214.0,
  gradient 4876.8) and 2507.9 ms under the default static DXC (light 744.6, attrib 596.0,
  gradient 469.0, march 452.5); the full table is in Plan 0046's close review. Debug
  `flythrough.exe` 37 744 128 B with static DXC linked; `machdxcompiler.lib` 113 589 902 B, an
  upper bound on what the feature adds (no before-size was read). The compile is not WARP's
  whole startup cost: with DXC the headless fly-through's `flythrough::shutdown` trace test on
  the default look still failed at 121.055 s against its 120 s bound, and each `mirror`
  attribute test still passed nextest's 90 s slow marker (15.3 s for the three at 578c8d3,
  152.8 to 158.2 s under FXC after Plan 0028), so WARP's cost is the attribute pass's
  execution; the test runs `--look plain` there. The Windows `cargo build --workspace` step
  delta is not read: the merge run's cache key changed with the lockfile.

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

**Paired mixer readings (ADR-0020, 2026-10-21).** A plan's *relative* bound on `cargo bench -p
vxe-audio --bench mixer` (within N percent of the parent's) is read with both builds compiled
with `RUSTFLAGS="-C llvm-args=-align-loops=64"` and `CARGO_TARGET_DIR=target/aligned`. Each build
gets three alternating runs in one exclusive perf-lock hold, with the 1-minute load and the top
processes stated, and each case's median of the three criterion medians is compared. The
default-flag pair may be logged beside it, but it does not decide the bound: the placement of
the FIR loop alone moved the static case by 5.8 percent in Plan 0020 Phase 13. The *absolute*
mixer figures below are read on the default build, the code that ships, under the quiet-box
protocol above. Other benches keep default-flag paired readings unless their plan adopts this.

- **Mixer callback, 32 HRTF voices, one 256-frame block:** 1.0 ms on the baseline CPU
  (proposal; 32 x 2 x 128 x 256 = 2.1 million multiply-adds plus filters and taps).
  **Reading (`cargo bench -p vxe-audio --bench mixer`, 32 voices of looping noise, each
  low-passed and carrying 16 taps):** 0.3619 ms (0.3615 to 0.3623) static; 0.6667 ms (0.6659
  to 0.6673) with the listener yawing 90 degrees per block, every voice crossfading its HRIR.
  **Silent voices (Plan 0052 Phase 5, 14f3841).** The voice smoothers (own gain, send, gain,
  distance gain) settle on their target within `1e-7` at each block edge and the voice
  low-pass state flushes under reverb's `FLUSH_BELOW`, so a voice at gain 0 no longer decays
  into subnormal floats. **Reading** (`cargo bench -p vxe-audio --bench mixer`,
  `mixer_block_silent`, dev box, exclusive perf lock, load 1.36 to 2.96, default build, not the
  ADR-0020 paired protocol): 64 voices at gain 0 after 10 s 1.7909 ms at the base (47 % over 64
  fresh voices' 1.2177 ms), 1.1721 ms at the tip, inside the fresh case's spread (1.1951 to
  1.2164 ms) and under it.
- **HRTF ceiling (arithmetic, Plan 0052; no budget).** The HRIR convolution is the direct form:
  2 ears x 128 taps per sample, 64 x 2 x 128 x 48 000 = 0.79 G multiply-adds a second at 64
  voices (about 1 G with the filters and taps), twice
  that while sources move and every voice crossfades two HRIRs. That is the mixer's ceiling on
  voice count; a partitioned FFT convolution is the step when a consumer needs more voices.
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
- **Plan 0020 (the late tail, sixteen direction buses, the room fan, gliding taps; closed
  2026-10-21, v0.24.0).** Proposals: mixer block, 32 HRTF voices, static at most 0.6 ms and
  turning at most 1.0 ms; `Acoustics::update`, 32 sources with a 16-ray fan slice, at most
  1.1 ms. **Readings at the close (recorded misses, accepted by the owner 2026-10-21; a
  quiet-box re-read is a followup, then a mixer optimisation if it stays over):** the shipped
  code (Phase 13, c85bac5), default build, dev box, release, exclusive perf lock, median of
  three runs at load average 4.6 to 9.3 with other processes near 100 % CPU: mixer static
  0.6897 ms, turning 1.1441 ms; with the direct delay on, 0.8011 and 1.2577 ms. The same code
  aligned (ADR-0020, load 3.2 to 5.5): 0.657 and 1.110 ms. `Acoustics::update` at Phase 9's
  rays (100 m range, load 13.6 to 9.8): 0.7370 ms. Audio RAM printed by the mixer bench: echo
  lines 116 224 B per voice (7 438 336 B for the 64-voice pool, 600 ms), two tail networks
  3 616 768 B, the pre-delay line 39 424 B, buses 24 512 B, tap slots 8 192 B. **Earlier
  readings, dev box,
  release, exclusive perf lock, vxe-audio and both benches as at d7eb1f8, 2026-10-20; the box
  was not quiet (1-minute load average 1.87 to 2.98, and other processes above 7 % CPU: one at
  86 %, two at 37 %), so they read high against the protocol above:** mixer static 0.5078 ms
  (0.5004 to 0.5162), turning 0.8786 ms (0.8700 to 0.8887) (`cargo bench -p vxe-audio --bench
  mixer`, 32 voices with 16 taps each into the buses, the 12 m room's tail running);
  `Acoustics::update` 0.8388 ms (0.8327 to 0.8463) (`cargo bench -p flythrough --bench
  acoustics`, the fan's first slice in every call; the fan's rays stop at the first solid
  voxel). Audio RAM printed by the mixer bench: the tail's lines 323 584 B, the buses 24 512 B,
  tap state 128 B per voice (8 192 B for the 64-voice pool); a streamed voice's ring 19 200 B.
  Room estimates the fly-through prints headlessly (`--room N --headless --frames 600`, no
  device): 12 m mfp 7.999 m and T60 2.257 s, 36 m T60 6.770 s, 4 m T60 0.752 s; the seed-1
  streaming spawn mfp 16.082 m, T60 4.537 s, energy -7.5 dB.

## 10. Not goals (for now)

- Binary size.
- Mobile, web or console targets as such. The portable profile of §1 is a PC-class setting
  that a Switch-2-class device could run; a console port, its SDK and its certification are
  not planned, and Switch 1 is out of reach for a ray marcher of this shape (owner,
  2026-10-17).
- Worlds wider than 2^24 voxels (about 1.7 km at 10 cm) from the camera. The march does its
  world math in `f32`, exact to 2^24 voxels; a following window past that needs
  camera-relative march coordinates, a future plan's work (Plan 0052's Followups).
- Frame rates above 60 fps as a gate; they are welcome, not required. A fast machine
  reaches 120 fps through a performance profile, not through a budget row here.
- VR on the baseline. The VR-readiness decision in `roadmap.md` keeps the renderer's doors
  open; a VR plan adds its own profile here (two views, 90 Hz, a named card) when scheduled.
