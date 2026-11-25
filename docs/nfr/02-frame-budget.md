# NFR section 2: frame budget, the readings

> The budget and its table are in `../nfr.md` section 2; this file holds every reading
> taken against it, in the order recorded, each naming its machine, scene and commit.
> New readings go here. A reading that replaces a proposal also updates the budget line
> in `nfr.md`. A bare `§N` in this text is a section of `nfr.md`.

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
872 640 B live, 259 203 264 B allocated in every run. The attribute pass's initial fill with
ADR-0038's plane code (Plan 0050's close review, 2026-10-31, the same two binaries, 1-minute
load 1.65 to 2.91, three alternated rounds, medians; 383 608 bricks): RTX 3080 Laptop GPU 17.837
ms against 17.814 (+0.1 %), Radeon RENOIR 95.627 against 95.531 (+0.1 %), equal to its noise.

**The default surface is honest cubes, the field an opt-in look (Plan 0054's close readings,
ADR-0041; main `28948a5`'s release binary, its default the snap planes, against the release
binary of `86611cc`'s tree, whose default is `face` with no attribute pass; one quiet round
each, every run under the exclusive perf lock and the memory cap, started at a 1-minute load of
1.75 to 1.99; `flythrough --level --seed 1 --headless --frames 300 --render 1920x1080 --adapter
NVIDIA|AMD [--look field --crisp none|rock|all]`, 1080p from 1280 x 720, release, each run's
frame line; logs `renders/plan-0054-live/p4c/close/` (local)).** Hit / light / GPU frame, ms:

| run | RTX 3080 Laptop | Radeon RENOIR |
|---|---|---|
| main `28948a5`, default (snap planes) | 0.612 / 0.921 / 4.516 | 4.288 / 7.045 / 28.801 |
| `86611cc`, default (`face`) | 0.548 / 0.849 / 4.222 | 4.064 / 6.732 / 27.166 |
| `86611cc`, `--look field --crisp none` | 3.827 / 0.891 / 7.511 | 30.734 / 7.108 / 54.522 |
| `86611cc`, `--look field --crisp rock` | 0.570 / 0.855 / 4.263 | 4.308 / 6.966 / 28.015 |
| `86611cc`, `--look field --crisp all` | 0.572 / 0.865 / 4.294 | 4.302 / 6.928 / 27.973 |

The default's hit plus light reads 1.397 ms against main's 1.533 on the RTX and 10.796 against
11.333 on the Radeon. The level's initial fill: main's attribute pass 17.832 ms GPU (RTX) and
95.743 ms (Radeon); the default runs no attribute or field pass. The streamed flight
(`flythrough --autopilot --headless --seed 1 --adapter NVIDIA`, one round each, started at a
load of 1.75 to 1.90, the flight itself raising it to 5.6 to 8.5): main's default streaming CPU
p50 13.435 / p99 60.044 ms over 1366 frames, attribute pass GPU p50 1.154 / p99 1.771 ms, march
GPU p50 8.628 / p99 19.951 ms; the default at `86611cc` streaming CPU p50 0.239 / p99 64.251 ms
over 3148 frames, march GPU p50 3.031 / p99 5.767 ms, no attribute or field pass. RADV shader
stats at `86611cc` (load 3.1 to 4.1, not a timing reading): two compute shaders at 128 VGPRs
(the field look's hit pipeline and its resume dispatch), two at 84, the rest at 64 and below, no
spill; RADV prints no pipeline names, so the attribution is by elimination.

**The opt-in field look, readings of an option and not budgets the default draws on (Plan 0054,
same binaries, conditions and logs).** `--look field --crisp none` (every level material
smooth): hit plus light 4.718 ms on the RTX against the 5.00 ms figure it was built against
(Plan 0054 owner answer 3), 37.842 ms on the Radeon. Its field pass at the level's initial fill:
12.130 ms GPU (RTX) and 61.049 ms (Radeon) with no crisp row, 102.702 and 100.118 ms under
`--crisp rock`, 74.293 and 72.518 ms under `--crisp all`; over the RTX flight, p50 0.634 / p99
1.388 ms, with streaming CPU p50 66.030 / p99 206.603 ms over 494 frames and march GPU p50 8.153
/ p99 39.488 ms. The thin rig (`--thin-rig`, its wood crisp, 1080p): `face` 0.630 / 0.104 / 2.402
ms (RTX) and 4.884 / 0.945 / 16.659 (Radeon); `--look field` 4.821 / 0.126 / 6.667 ms with a
smooth share of 0.8476 (RTX) and 39.543 / 1.012 / 52.030 (Radeon); its field pass 0.064 ms (RTX)
and 0.188 ms (Radeon) for 1186 band slots. The owner's window (Plan 0054 owner answer 13, RTX
3080 Laptop, 2530 x 1378, `renders/plan-0054-live/session3-seam.log`): GPU frame 12.0 ms on the seam rig against
`face`'s 3.8, 7.8 ms on the thin rig against 3.4.

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

### Frame ledger (2026-10-31)

*Noted 2026-11-24 (Plan 0102):* the scenario tables below were read under `own`, the engine's
own upscaler, which Plan 0102 removed (ADR-0086); FSR 2 is the only upscaler and a frame
without it renders at its output size. The tables are kept as history; the live figures are
the render profiles' rows ("The render profiles" below) and the re-based paragraph below. At
Plan 0102's base (`be97152a`, Portable's 853 x 480 to 1280 x 720, loaded) `own`'s upscale and
sharpen passes read 0.380 ms on the RTX 3080 Laptop and 1.790 ms on the RADV RENOIR against
FSR 2's 0.640 and 4.077 ms ([Plan 0102's readings log](../plans/logs/0102.md), Phase 1).

The frame the owner judges Plan 0054's field surface against at its Phase 4d: today's measured
passes in three surface scenarios, beside a slice reserved for every planned system not yet in
the frame, held against the 16.6 ms split above. It is a ledger, not a bar. Phase 4b's bar (RTX
hit plus light at most 5 ms at the level's spawn, Plan 0054 owner answer 3) stands unchanged
(owner, 2026-10-31: "do not tighten yet. maybe there is still a chance to have some smoothness,
maybe only for certain types of materials"). Plain cubes everywhere, Teardown-style, are the
owner's accepted fallback if the field cannot pay. A reading replaces a guess here as each plan
closes. Plan 0054 closed on scenario A (owner answer 13, 2026-11-01: `face` the default, the
field an opt-in look, its Phase 4d dropped); the close's readings are in the paragraphs "The
default surface is honest cubes" above and "The `face` default carries no attribute" in §3.

*Noted 2026-11-03:* the figures below read `own` at 1.5, but the default is now FSR 2 at 1.5 with the far normal at `[2, 6]`; the readings that exist are in [Plan 0055's readings log](../plans/logs/0055.md) and [its close](../plans/done/0055-clarity-pass-no-detail-fsr2-default-and-a-far-normal-spike.md), and re-basing this ledger is owed.

*Re-based 2026-11-11 (Plan 0065 close, 03b00ab):* the default frame (FSR 2 at 1.5 to 2530 x 1378, half lighting tier, far normal `[2, 6]`, the baked store, lamp on, at rest) reads 4.097 ms GPU at the vein and 8.569 ms at the hall on the RTX 3080 Laptop, 30.450 and 72.922 ms on the RADV RENOIR; FSR 2's passes are 2.144 to 2.170 ms (RTX) and 15.715 to 15.720 ms (RENOIR) of it, the hall's hit pass 4.659 and 45.657 ms. Every pass, both lighting tiers, lamp off and walking: [Plan 0065's readings log](../plans/logs/0065.md), Phase 7, and the paragraph "The baked light's wasted work removed" below. The scenario tables that follow are the 2026-10-31 frame and stay as history.

**Conditions.** The level's spawn, `flythrough --level --seed 1 --headless --render 1920x1080
--frames 300 --adapter NVIDIA|AMD [--look L]`: 1080p output from 1280 x 720 internal (`own` at
1.5), half lighting tier, coarse shadow walk, lamp on, release, exclusive perf lock and memory
cap, three alternated rounds started at a 1-minute load under 2, the median over the rounds of
each run's frame line (itself the median of the last 30 frames). Sources, untracked logs in
`renders/plan-0054-live/` (local):

- cubes on the RTX: `face` at main `28948a5`, Plan 0054 Phase 4a's matrix (`renders/plan-0054-live/p4a/runs/`),
  read while that matrix was still running; Phase 4a's log table supersedes it.
- cubes on the Radeon: **no `face` reading at this tree.** The default `contour-sharp` (the cube
  walk with contour planes and snap) at `811b37e` (`renders/plan-0054-live/cost/level-base-*.log`, Plan 0054 Phase
  1's base) stands in as an upper bound: on the RTX at `28948a5` `face`'s hit reads 7.5 % under
  the default's (0.567 against 0.613 ms).
- the field: Plan 0054 Phase 3a's cost runs at `17045d8` (`renders/plan-0054-live/p3a/cost/`, its done-when 3).
  `field-cubes` is the field with the crisp test, the nearest reading of Phase 4c's default (no
  level material is crisp, so it shows the crisp test's cost and no cube); `field` is without it.

The split's lines group the passes: primary rays = hit; lighting = gradient + light + probes +
gather; denoise = temporal + a-trous; upscale and post = composite + upscale + sharpen +
exposure. The CPU is the render thread's frame CPU, which runs beside the GPU; at rest at the
spawn its `update + poll + sync` reads p50 0.03 to 0.09 ms and p99 0.05 to 0.16 ms in every run
above, and under motion it is §2's recorded streaming miss. **The ledger keeps the whole 1.0 ms
streaming line reserved and holds the GPU frame against the other 15.6 ms.**

**1. Measured, per pass (ms).** RTX = NVIDIA GeForce RTX 3080 Laptop GPU, Radeon = AMD Radeon
Graphics (RADV RENOIR), the slow adapter (§1).

| line (budget) | pass | RTX `face` | RTX `field-cubes` | RTX `field` | Radeon default | Radeon `field-cubes` | Radeon `field` |
|---|---|---|---|---|---|---|---|
| primary rays (3.0) | hit | 0.567 | 5.497 | 4.936 | 4.296 | 42.101 | 40.146 |
| lighting (6.0) | gradient | 0.092 | 0.096 | 0.099 | 0.486 | 0.498 | 0.497 |
| | light | 0.905 | 0.950 | 0.975 | 7.021 | 7.218 | 7.216 |
| | probes | 1.207 | 1.475 | 1.501 | 8.229 | 8.614 | 8.587 |
| | gather | 0.110 | 0.108 | 0.110 | 0.609 | 0.610 | 0.609 |
| denoise (3.0) | temporal | 0.080 | 0.085 | 0.084 | 0.623 | 0.619 | 0.622 |
| | a-trous | 0.538 | 0.495 | 0.502 | 2.347 | 2.350 | 2.346 |
| upscale, post (1.5) | composite | 0.180 | 0.193 | 0.196 | 1.323 | 1.327 | 1.323 |
| | upscale | 0.593 | 0.497 | 0.504 | 2.979 | 2.997 | 2.983 |
| | sharpen | 0.076 | 0.079 | 0.077 | 0.649 | 0.650 | 0.650 |
| | exposure | 0.035 | 0.036 | 0.036 | 0.029 | 0.029 | 0.029 |
| **GPU frame** | | **4.388** | **9.501** | **9.010** | **28.688** | **67.007** | **65.052** |
| lighting line | | 2.314 | 2.629 | 2.685 | 16.345 | 16.940 | 16.909 |
| denoise line | | 0.618 | 0.580 | 0.586 | 2.970 | 2.969 | 2.968 |
| post line | | 0.884 | 0.805 | 0.813 | 4.980 | 5.003 | 4.985 |
| CPU (beside the GPU) | frame CPU | 0.546 | 0.846 | 0.847 | 0.781 | 0.935 | 0.824 |

Read off it: on the RTX every line but primary rays is inside its budget in every look, and the
field's cost is its hit pass (8.7x to 9.7x `face`'s) plus about 0.3 ms more probe update than
`face`'s. On the
Radeon the frame is over 16.6 ms before the surface is chosen: its lighting line alone reads
2.7x the 6.0 ms budget and its post line 3.3x the 1.5 ms one.

**The owner's window (a scaling note).** Session 1's frame line reads the window at 2530 x 1378
from 1687 x 919 (`own` at 1.5), 1.68x the internal pixels of the conditions above. Measured
there on the RTX, Phase 4a's matrix (`--render 2530x1378`, shaded, lamp on; in progress, two
or three rounds), GPU frame in ms:

| pose | `face` (`28948a5`) | default (`28948a5`) | `field` (`17045d8`) | `field-cubes` (`17045d8`) |
|---|---|---|---|---|
| spawn | 6.639 | 6.816 | 13.873 | 14.393 |
| vein | 6.403 | 6.846 | 11.743 | 11.906 |
| patch | 6.243 | 6.528 | 13.396 | 13.191 |

The window reads about 1.5x the 1080p frame (`face` 6.639 / 4.388 = 1.51, `field-cubes` at the
spawn 14.393 / 9.647 in the same matrix = 1.49). **Projection, not read:** the Radeon at the
window about 1.5x its rows above (default about 43 ms, `field-cubes` about 100 ms), and every
per-pixel slice in table 2 about 1.5x on either adapter.

**2. Reserved slices for the systems not yet in the frame**, at the conditions above. RTX
figures are the plans' own where a plan exists. Where a plan gives no slow-adapter figure the
Radeon column is 6.5x the RTX's, today's measured ratio of the default frame at `811b37e` (28.688 /
4.413 = 6.5x; the field's hit reads 7.7x), a guess. "Budgeted" means a plan's Budgets table states the
figure, itself a guess until that plan's close reads it; "guess" means no plan exists yet.

| system | RTX ms | Radeon ms | status | source |
|---|---|---|---|---|
| light quality: the per-face table, its guard and fill, the lamp channel | 0.1 to 0.3 | 0.4 to 1.7 | guess | Plan 0057 (ADR-0043; its Budgets guess the default at `b --indirect off` plus 0.3 ms RTX, 2.0 ms Radeon, lamp on) reads it at its close; Plan 0048's bar 6 is moot under ADR-0043 and Plan 0049's guesses were dropped. Plan 0051 read its opt-in arms (loaded, indicative, c9ce8c9, spawn, 1080p from 720p): at rest on the RTX `a` probes 0.016 ms and light + gradient + temporal 0.424 ms (`off` 1.058, 0.973); `b --indirect off` light 0.122 ms, no probes, frame 3.255 ms against 5.224; in flight the light pass falls only 0.14 to 0.16 ms under `b`. No default moved, so nothing is counted until the verdict plan reads its mechanism |
| liquids | 0.9 to 1.2 | 5.9 to 7.8 (6.5x) | budgeted | Plan 0034, ADR-0029: a half-screen lake in `Trace`, refraction walk 0.4 plus the liquid pass 0.5 to 0.8; under 0.5 in `Sky`; lava adds none |
| voxel-model entities (creatures), with their shadows and motion | 0.5 to 2.2 | 4.6 to 23.0 | measured | Plan 0062 Phase 5 (c872cb1, `--level --seed 1`, spawn, 1920 x 1080, FSR 2 at 1.5, quiet): the frame's growth at 4 to 16 walkers, still to walking, their eye lights in the list, RTX 3080 Laptop and RADV RENOIR; most of it the light pass's shadow rays through the tree. 64 walkers: 3.5 to 4.5 RTX, 31 to 33 Radeon. Accepted by the owner as measured (2026-11-16) |
| entity shadows and motion vectors | in the row above | in the row above | measured | Plan 0062 read them in one frame with the entities; probe and bake rays see no entity (ADR-0050) |
| entity marks: an outline and a tint in the composite | 0.02 to 0.03 | 0.07 to 0.10 | measured | Plan 0077 Phase 6 (ADR-0074; the Phase 6 tree at 9abcd694, `gpu_entity_library`'s `reading::the_composite_cost_with_marks`, four library walkers in a rock box, 1920 x 1080 from FSR 2 at 1.5, loaded at a 1-minute load of 7 to 55, indicative): the composite's median with the mark test against the same tree with `mark_at` returning nothing, RTX 3080 Laptop and RADV RENOIR; 0, 1 or 4 marked read alike. lavapipe has no timestamp queries |
| volumetrics: dust, fog, smoke | 0.3 to 0.6 | 2.0 to 3.9 (6.5x) | guess | M8 (`roadmap.md`): a coarse volume marched at half resolution with a fixed step count; scaled from Plan 0035's cloud march |
| effects: debris particles | 0.1 to 0.3 | 0.7 to 2.0 (6.5x) | guess | M8, no plan |
| **cave frame subtotal** (the first consumer's) | **1.92 to 4.63** | **13.67 to 38.5** | | the seven rows above (the entity rows measured at 4 to 16 walkers, Plan 0062; the marks, Plan 0077) |
| sun and sky | 0.87 to 0.92 | 7.8 to 9.6 | measured | Plan 0032 Phase 7 (a68f2659 plus its tree, loaded, indicative; the table under "The sun and the sky on the outdoor hills" below), ADR-0026, ADR-0027: the light pass's growth over `--sky off` plus the three sky passes, at noon, midnight and inside the cave mouth, one sky ray, 1080p from FSR 2 at 1.5. RTX 3080 Laptop and RADV RENOIR; most of it the key ray and the sky ray in the light pass. Read before Plan 0086 (ADR-0070), which moves the sky ray's cost |
| far field | 0.3 to 1.0 | 2.0 to 6.5 (6.5x) | budgeted | Plan 0033, ADR-0028: outdoor horizon view, primary rays; the sun ray's growth is unprojected |
| clouds and aerial perspective | 0.5 to 1.0 | 3.6 to 7.7 | budgeted | Plan 0035, ADR-0030; it proposes a 1.0 ms sub-line on the floor |
| weather | 0.25 to 0.8 | 0.75 to 8.0 | budgeted | Plan 0036, ADR-0031: fall map, precipitation, composite terms |
| night sky | 0.25 to 0.65 | 2.0 to 5.2 | budgeted | Plan 0037, ADR-0032: aurora and the composite's stars and bodies; it proposes a 0.3 ms cap on the floor |
| surface detail layer | 0.3 to 0.8 | 2.0 to 5.0 | budgeted | Plan 0081, ADR-0063 (row added 2026-11-17 at the M10 re-read): the hit pass's shell march with `grass` on half the screen at the `[6, 12]` fade, probes off; the slow adapter at about six times the RTX's hit pass, which is why the Portable profile is off. Plan 0086's baked sky visibility (ADR-0070) has no row: its close replaces the "sun and sky" figures |
| **outdoor frame total** (cave rows plus M10) | **4.39 to 9.80** | **31.87 to 80.5** | | all rows (sun and sky measured, Plan 0032's close; the surface detail row added 2026-11-17) |

Game work is the consumer's CPU, outside this GPU ledger; today's frame CPU (table 1) runs
beside the GPU.

**3. The ledger: three surface scenarios.** (A) plain cubes everywhere (`face`); (B) the field
for every material that is not crisp (Plan 0054's Phase 4c default, read as `field-cubes` at
the level, where no material is crisp); (C) smoothness for some material types only, the field's
cost scaling with the smooth share `s` of the on-screen surface: GPU frame = A + `s` x (B - A),
a **projection** that assumes Plan 0054 Phase 4c's amended per-brick rule (a brick with no
smooth voxel near walks as cubes) and a cost linear in the smooth pixels; rays that cross smooth
band bricks before a crisp hit pay more than it says. The floor (§1's baseline, not on hand) is
projected at 2x to 3x the RTX, the factor §2's Plan 0005 paragraph uses. "Left" is 15.6 ms
minus the GPU frame: what remains for table 2.

| scenario | RTX GPU frame | RTX left | Radeon GPU frame | Radeon left | floor, projected (2x to 3x RTX) | floor left |
|---|---|---|---|---|---|---|
| A, cubes | 4.39 | 11.2 | 28.7 (default, upper bound) | -13.1 | 8.8 to 13.2 | 6.8 to 2.4 |
| B, field but crisp | 9.50 | 6.1 | 67.0 | -51.4 | 19.0 to 28.5 | -3.4 to -12.9 |
| C, `s` = 0.25 (projection) | 5.67 | 9.9 | 38.3 | -22.7 | 11.3 to 17.0 | 4.3 to -1.4 |
| C, `s` = 0.5 (projection) | 6.94 | 8.7 | 47.8 | -32.2 | 13.9 to 20.8 | 1.7 to -5.2 |
| C, `s` = 0.75 (projection) | 8.22 | 7.4 | 57.4 | -41.8 | 16.4 to 24.7 | -0.8 to -9.1 |

**4. What each scenario leaves, and the field's cost at which it still fits** (RTX, dev
reference, 1080p; the cave frame needs 1.9 to 3.7 ms of table 2, the outdoor frame 4.05 to
9.05 ms):

- **A** leaves 11.2 ms: the cave frame and the outdoor frame both fit at every guess. On the
  floor's projection it leaves 6.8 to 2.4 ms: the cave frame (3.8 to 11.1 ms at the floor's
  factor) fits only toward the 2x end with the low guesses.
- **B** leaves 6.1 ms: the cave frame fits with 2.4 to 4.2 ms to spare; the outdoor frame fits
  only at its low guesses. B's passes other than the hit read 4.00 ms (9.501 - 5.497), so B
  fits while the field's hit is at most 15.6 - 4.00 - the systems: **at most 7.9 ms for the
  cave frame at its high guesses** (today 5.50), **at most 2.55 to 7.55 ms for the outdoor
  frame**. Under Phase 4b's bar (hit plus light at most 5 ms, so a hit near 4.05 ms) B's frame
  is about 8.05 ms and leaves 7.55 ms: the cave frame at every guess, the outdoor frame up to its
  low-middle guesses. On the floor's projection B fits nothing at today's cost: at the 2x end
  with the cave's low guesses the field's hit would need to read about 1.9 ms or less on the RTX.
- **C** leaves 11.2 - `s` x 5.11 ms (5.11 = B - A today). It fits while the field's extra over
  cubes is at most (11.2 - the systems) / `s`: for the outdoor frame at its high guesses
  (9.05 ms) that is 8.6 ms at `s` = 0.25 (today's 5.11 fits), 4.3 ms at 0.5 and 2.9 ms at 0.75;
  for the cave frame at its high guesses it fits at every `s` at today's cost. On the floor's 2x
  end with the cave's low guesses the budget is 15.6 / 2 = 7.8 ms of RTX time, 5.9 ms after the
  systems: A fits, and the field's extra may be 1.5 / `s` ms, 6.0 at `s` = 0.25 (today's cost
  fits), 3.0 at 0.5. At the 3x end nothing fits, cubes included.
- **The Radeon misses in every scenario before any system is added** (A over by 13.1 ms; its
  lighting line alone 16.3 ms). That is a profile question (§1's portable profile: internal
  resolution, lighting tier), not a surface one; the field's 7.7x hit ratio there is the walk-speed
  spike's (Plan 0054 owner answer 3).
- **At the owner's window** the RTX's B leaves 1.2 to 3.7 ms by pose (the window table) against
  the cave frame's projected 2.9 to 5.6 ms there (table 2 x 1.5, a projection): B misses at the
  window at the spawn and the patch; A leaves 9.0 to 9.4 ms and fits.

**Procedural detail removed (Plan 0055 Phase 1, RTX 3080 Laptop GPU, showcase start, headless,
2530 x 1378 from 1687 x 919, `--upscaler fsr2`, median of 30 of 300 frames, perf lock, 1-minute
load 5.3 to 6.9, three runs alternated with the c09aa5b binary at `--rock-detail 4,0`):** hit
2.793, 2.815 and 2.846 ms against 2.800, 2.828 and 2.826; GPU frame 10.388, 10.439 and 10.530 ms
against 10.436, 10.531 and 10.525; composite 0.259 to 0.260 ms against 0.273.

**FSR 2 at 1.5 the fly-through's default (Plan 0055 Phase 2, tree on 4ec2b34, `flythrough
--headless --frames 300`, GPU frame median of the last 30, perf lock, 1-minute load 0.59 to
1.43).** Frame in ms, FSR 2 against ours. RTX 3080 Laptop GPU at 2530 x 1378 from 1687 x 919:
showcase start 10.348 and 10.399 against 8.632 and 8.652, `--level --pose vein` 7.400 and 7.520
against 5.426 and 5.449; FSR 2's passes read 2.932 to 2.958 ms, over §2's 1.5 ms upscale line,
ours 0.928 to 1.301 ms (upscale plus sharpen). RADV RENOIR at 1920 x 1080 from 1280 x 720:
showcase start 48.360 against 41.677, vein 32.865 against 26.124; FSR 2's passes 9.967 to 9.983
ms, ours 3.655 to 3.864 ms.

**The fly-through's defaults with the far normal at `[2, 6]` (Plan 0055 Phase 5, tree on e9f0d55,
`--headless --frames 300`, median of the last 30, perf lock, 1-minute load 1.16 to 1.49).** RTX 3080
Laptop GPU at 2530 x 1378 from 1687 x 919: showcase start frame 11.799 and 11.936 ms (hit 4.264,
4.306), `--level --pose vein` 7.482 and 7.519 ms (hit 0.632, 0.637). RADV RENOIR at 1920 x 1080 from
1280 x 720, FSR 2: showcase start 55.063 and 54.998 ms (hit 21.078) against 48.468 and 48.725 (hit
14.47) with `--look face` and 51.023 and 51.034 (hit 17.007) at `[1, 2]`; vein 33.107 and 33.206 ms
(hit 3.05 to 3.08) against 33.131 and 34.344 (hit 3.04 to 3.26). The hall shot's crops and the other
settings are in `docs/plans/logs/0055.md`.

**The aliasing spike's arms and candidates (Plan 0056 Phase 3, tree on 879c452 plus the phase,
`flythrough --headless --frames 300`, GPU frame median of the last 30, exclusive perf lock, three
alternated rounds; 1-minute load 1.8 to 2.8 in round 2 but for the RADV vein's `own`, native,
scale-1 and cap runs (3.7 to 11.3), up to 26 in rounds 1 and 3. Most round-2 runs and every
run of rounds 1 and 3 read over 2, so every figure here is loaded (indicative)).** Frame in ms, round 2, showcase start then `--level --pose vein`. RTX 3080
Laptop GPU at 2530 x 1378 from 1687 x 919: main's 38cd29a binary 14.996 and 8.125; the default
(FSR 2 at 1.5, far normal `[2, 6]`) 12.340 and 8.013; `[1, 2]` 11.260 and 7.938 (hit 3.369
against 4.397); `--look face` 10.828 and 7.803 (hit 2.932); `--sharpen 0` 12.255 and 7.755; `own`
10.329 and 5.827 (upscale plus sharpen 1.025, 0.984); native 19.662 and 9.204 (hit 10.233);
FSR 2 at scale 1 with `[3, 9]` 25.662 and 15.126 (FSR 2's passes 5.063, 5.344); `--emitter-cap 1
--dither on` 12.896 and 8.003 (FSR 2's passes 3.024 against 3.125). RADV RENOIR at 1920 x 1080
from 1280 x 720: main 54.949 and 33.060; default 54.954 and 32.842; `[1, 2]` 50.795 and 32.888;
`face` 48.394 and 33.273; `--sharpen 0` 54.057 and 32.780; `own` 48.101 and 30.154; native
88.231 and 49.668; FSR 2 at 1 with `[3, 9]` 105.626 and 73.771 (FSR 2's passes 16.130 at the
start); cap and dither 54.961 and 43.231 (the vein's round 2 at load 11.3; rounds 1 and 3 33.071,
33.084). `frame_bytes` with every knob off equals main's: 483 147 556 B at 2530 x 1378 and
309 377 468 B at 1080p; FSR 2 at 1 reads 808 972 340 and 503 318 268 B. All three rounds and the
per-pass lines are in `docs/plans/logs/0056.md`.

**The table's fix pass (Plan 0057 Phase 7, 2de29c6, `preload_level_readings`, 1687 x 919, half-resolution lighting, cost only, exclusive perf lock, load 1.5 to 8.5, loaded in the base's runs; base 4cc0fa1's renderer).** Medians in ms, patch then vein. RTX 3080 Laptop GPU, walking: light 0.7265, 1.1694 (base 0.7447, 1.1320), preload 0.0558, 0.0510, temporal 0.1056, 0.1083, gradient 0.1238, 0.1507, blend 0.1345, 0.1926. At rest, every face in view capped: light 0.0518, 0.0501, temporal 0.1067, 0.1049, gradient 0.0089, 0.0092, blend 0.0076, 0.0081. Radeon (RADV RENOIR), walking: light 5.9693, 9.8796 (base 5.9214, 9.6856), preload 0.0872, 0.0683, temporal 0.8684, 0.9105, gradient 0.6146, 0.8524, blend 0.6398, 1.3835; at rest light 0.3273, 0.3216, temporal 0.8687, 0.8608, blend 0.0041, 0.0041. **Radeon lighting walking is over the 6.0 ms line at the vein, in the base and on the tip.** The walk's slice frames were not read on their own. Against the plan's bar of at most 0.25 ms added on the RTX: the blend's 0.13 and 0.19 ms walking and 0.008 ms at rest are under it, read beside loaded base rows. Per-pass rows are in `docs/plans/logs/0057.md`, Phase 7.

**The loaded light's brick store against the table (Plan 0059 Phase 12, 393aa68 plus the pre-roll's tick fix, release, exclusive perf lock).** The lighting line here is gradient, preload, light, fill, blend, temporal, probes and gather (the amendment of 2026-11-09 adds the temporal pass). At rest (`rest_readings`, 120 timed frames after 1024 still frames, lamp off, far normal 1..2, load about 1): RTX 3080 Laptop GPU at 2530 x 1378, baked against table, native: vein 3.86 against 1.36 ms (temporal 3.52 against 0.92), hall 3.80 against 6.06 (temporal 3.41 against 1.12); FSR 2 at 1.5: vein 1.69 against 0.64 (temporal 1.54 against 0.39), hall 1.72 against 4.60 (temporal 1.55 against 0.48). RADV RENOIR at 1280 x 720, native: vein 3.22 against 3.04, hall 3.10 against 34.00 (light 20.26, blend 10.55 on the table); FSR 2 at 1.5: vein 1.37 against 1.35, hall 1.42 against 25.48. Walking (the autopilot of section 5's protocol, `--seed 1`, p50 per pass over the flight, loaded by the flight's own workers to 8 to 15): RTX at 2530 x 1378, FSR 2 at 1.5, queue against table: light 0.321 against 0.534, temporal 0.092 against 0.076, probes 0.191 against 0.165, gather 0.172 against 0.135, preload 0.000 against 0.061; native: light 0.771 against 0.863, temporal 0.267 against 0.155; march GPU p50 5.496 against 4.855 ms (FSR 2) and 7.219 against 5.019 (native). RADV RENOIR at 1280 x 720, FSR 2 at 1.5: light 0.697 against 2.603, temporal 0.145 against 0.145; march p50 8.536 against 11.004 ms. The queue's slice walking on the RTX: p50 weighted by submits 1.50 ms in both flights, max 10.82 (FSR 2) and 6.29 ms (native). At the hall with an empty queue: 0 bake submits a second. **The à-trous pass alone reads 2.96 to 6.95 ms on the RTX and 4.26 to 10.85 on the RENOIR in either light, over the 3.0 ms denoise line.** Rows in `docs/plans/logs/0059.md`, Phase 12.

**The loaded light's streaming CPU re-read quiet (Plan 0059's fix pass, a4fbbd8 and 4b2de1e, release, exclusive perf lock, 2026-11-09).** The autopilot of section 5's protocol, `--seed 1`, RTX 3080 Laptop GPU at 2530 x 1378, FSR 2 at 1.5, the table (4b2de1e) and the live queue (4b2de1e, a4fbbd8's parent and a4fbbd8) alternated over two rounds, each run started at a 1-minute load of 1.41 to 1.49 (the flight's workers raise it to 6 to 9). Streaming CPU per frame p50: table 3.551 and 4.160 ms, queue 3.420 to 4.036 ms; p99 45.9 to 83.6 ms in both lights; the sync stage is most of it in every arm (3.19 to 3.90 ms p50). Phase 12's 10.0 and 19.8 ms were runs started at load 5.2 and 12.0. The queue's own step, outside that figure, reads 1.118 and 1.259 ms p50 before Plan 0059's fix pass and 1.029 and 1.019 after (p99 17 to 19 ms). The workers' face masks: mean 177.8 us a chunk, max 5.7 ms over 90 702 chunks. Residency p95 314 to 354 ms in both lights. The vein piece's brick records on the RTX against RADV RENOIR: equal words 0.9998, |levels| p99 0.000, max 0.204. Rows in `docs/plans/logs/0059.md`, Fix pass.

**The window's pacing, the frame's waits and the streaming CPU (Plan 0064 close, a9298c6, release, exclusive perf lock, 2026-11-11).** Windowed (Phase 1, 24 runs, RTX 3080 Laptop GPU rendering, AMD iGPU compositing under Hyprland, panel 2560 x 1440 at 165 Hz, `Immediate` granted for `AutoNoVsync`, load 0.9 to 3.6): wall p50 with a frame callback before each present (`--pace compositor`, the pacing before the plan) 13.0 to 17.2 ms, without one (`--pace free`, the default since) 10.6 to 13.5 ms; compositor wall p99 18.2 to 19.3 ms (three refreshes) in every cell. Windowed at the tip (2552989, the code of a9298c6; the owner's session, 8 runs, HUD shown, two rounds, load 1.2 to 1.8, same machine and present mode), against Phase 1's HUD-shown cells: `free` wall p50 level 11.24 to 11.27 against 11.68 to 11.78 ms, showcase 12.19 to 12.27 against 13.36 to 13.48; `compositor` level 14.93 to 15.01 against 13.75 to 14.14, showcase 18.02 to 18.18 against 16.77 to 17.23; frame CPU p50 3.8 to 4.5 ms in every run against 1.5 to 2.4; no blocking GPU read in any run. The RADV RENOIR's window is unread. Headless, the autopilot of section 5's protocol, `--seed 1`, 1280 x 720, Phase 1's start 37f6845 against the tip, three alternated rounds, loaded by the flights' own workers to 5 to 13: RTX frame CPU p50 8.7 to 9.8 against 8.8 to 9.9 ms, p99 71 to 105 against 69 to 105; streaming CPU p50 0.2 to 3.0 against 0.2 to 3.4 ms (two states), p99 50 to 84 against 45 to 81; residency p95 302 to 341 against 301 to 338 ms. RADV RENOIR frame CPU p50 6.86 to 7.02 against 6.70 to 6.85 ms, p99 57 to 61 against 58 to 59; streaming p50 2.66 to 2.71 against 2.61 to 2.62, p99 43.3 to 44.2 against 43.3 to 43.8; residency p95 304 to 310 against 302 to 304 ms. The frame path's blocking GPU reads: 608 over 300 lavapipe frames before, 0 after. A sync time budget of 2, 4 or 8 ms (`--sync-ms`, off by default) put residency p95 at 3.0 to 13.7 s on the RTX against 0.26 to 0.30 s without one; bounding selection too and keeping deferred chunks' sizes (Phase 4b, 00731f9, three rounds, loaded 7.6 to 11.6) read 6.6 to 6.9 s at 2 ms, 3.8 to 4.1 s at 4 and 2.7 to 2.8 s at 8 against 0.30 s at none, streaming p99 31.5 to 31.6, 38.9 to 41.9 and 92.4 to 125.4 ms against 44.5 to 48.6, so the default stays none. The window's frame CPU rise (1.5 to 2.4 ms p50 before, 3.8 to 4.5 at the tip) is the live queue's step at rest, Plan 0060's (close review): at the tip the queue empties within 5 to 10 s and its step reads 2.84 to 3.56 ms p50 on every per-second line after that, against 0.43 to 1.21 ms in Phase 1's runs, where the queue kept baking; Plan 0067 owns it. Rows in `docs/plans/logs/0064.md`, Phases 1, 4, 4b and 5.

**The baked light's wasted work removed (Plan 0065 close, base 486a53e against the tip 03b00ab, `reading::pass_readings`, release, exclusive perf lock, 2026-11-11).** 1687 x 919 to 2530 x 1378, FSR 2 at 1.5, far normal `[2, 6]`, the brick store baked first, mean of 120 timed frames after 128 still ones; vein and hall, half and full lighting tier, lamp off and on, at rest and walking. RTX 3080 Laptop GPU (load 0.95 / 0.49 base, 0.96 / 0.76 tip): temporal 0.319 to 0.563 against 0.069 to 0.121 ms at the half tier and 1.270 to 2.509 against 0.248 to 0.436 at the full; a-trous with the lamp off 0.759 to 0.790 against 0.260 to 0.299 (half) and 2.991 to 3.236 against 0.986 to 1.011 (full), with the lamp on unchanged at the half tier (0.757 to 0.796 against 0.783 to 0.812) and 3.134 to 7.953 against 3.088 to 3.321 at the full; FSR 2's passes 2.855 to 4.725 against 2.103 to 2.424 (fsr2 depth clip 0.669 to 1.148 against 0.306 to 0.435, the tone pass not dispatched with sharpening on), **still over the 1.5 ms upscale line**; hit at the hall 4.129 to 4.323 against 4.127 to 4.955 at `[2, 6]`, and at `[1, 2]` 2.981 to 3.577 against 2.945 to 3.167 (476fa67, the tip's hit pass); frame total 4.960 to 21.394 against 3.414 to 12.650. RADV RENOIR (load 0.13 to 1.28 base, 0.76 / 1.92 tip): temporal 0.936 to 1.015 against 0.550 to 0.944 (half), 3.943 to 4.278 against 2.221 to 4.099 (full); a-trous lamp off 4.020 to 4.092 against 2.087 to 2.113 (half) and 19.089 to 19.335 against 8.552 to 8.642 (full), lamp on unchanged (19.047 to 19.348 against 19.126 to 19.389 at the full tier, **over the 3.0 ms denoise line**); FSR 2's passes 17.147 to 17.495 against 15.691 to 16.058; hit at the hall 44.021 to 46.424 against 43.190 to 45.657. `frame_bytes` unchanged. The queue's step moved to Plan 0067. Rows in `docs/plans/logs/0065.md`, Phases 1 to 5 and 7.

**The store read by the far normal and the filters weighed by face (Plan 0061 Phases 2 and 5, the release binary of c327f6a's code, `flythrough --headless --hud --frames 300`, three alternated rounds, exclusive perf lock, 2026-11-11; 1-minute load at the lock 0.64 to 9.32, 80 of 96 runs at 2 or above, so loaded (indicative)).** FSR 2 at 1.5, half light, medians of the rounds' last-30 medians. RTX 3080 Laptop GPU at 2530 x 1378: the hall at far normal `[2, 6]`, GPU frame 7.241 ms with the store read by face and `rec.n` weights (the library's defaults), 7.379 at the fly-through's defaults since Plan 0061 (store read `normal`, weights `face`), temporal 0.077 against 0.163 ms; the vein 4.663 against 4.795, a-trous 0.833 against 0.903. RADV RENOIR at 1920 x 1080: the hall 40.948 against 41.159 ms, temporal 0.367 against 0.578; the vein 19.766 against 20.214, **a-trous 2.701 against 3.187, over the 3.0 ms denoise line with the face weights**. At `[1, 2]` the read adds 0.01 ms (RTX) and 0.06 ms (RADV) to the hall's temporal pass. Every arm and pass is in `docs/plans/logs/0061.md`, Phase 2, done-when 6.

**The live queue's step, the bake budget and the missing light (Plan 0067 Phase 7, the tip 98c10a0's code with main merged against Phase 1's binary 87bf0d2 = the start commit 4bc8100 plus timers, release, headless, exclusive perf lock, quiet starts at a 1-minute load of at most 2, 2026-11-12).** RTX 3080 Laptop GPU at 2530 x 1378 (FSR 2 at 1.5), RADV RENOIR at 1280 x 720. The patch at rest (`--level --pose patch`): the start commit's queue opens and withdraws the same job every step (step p50 3.262 ms RTX, 3.794 RENOIR; frame CPU p50 4.426 and 4.622); the tip drains in 155.9 s on the RTX with 0 withdrawn, drained step p50 0.075 ms, frame CPU p50 1.680, p99 4.916; the RENOIR is still refining at 200 s (step p50 0.283, frame CPU 1.118). The cave glide at 150: RTX regime B, step p50 14.716 before and 12.980 after, p99 29.468 and 28.767; RENOIR regime A, 0.447 and 0.541. The autopilot of section 5's protocol (`--seed 1`): regime A after 2 to 3 B lines on both binaries; A's step p50 0.93 to 1.29 ms before (3 runs) and 1.23 to 1.54 after (4 runs); step p99 18.5 to 23.2 and 16.5 to 21.4; frame CPU p50 10.6 to 36.8 ms on both binaries, swinging with `update + poll + sync` (p50 3.5 to 24.9), so no frame-CPU difference is read; residency p95 316 to 366 and 318 to 379 ms. Pinned: `--coarse 0` (A) non-opening p50 0.966 and 1.060 ms, opening 6.655 and 3.487, p99 13.718 and 10.319; `--near 0` (B) opening p50 10.706, then 7.982 under `black` and 9.180 to 9.404 under the default `chain`, whose first-block fallback fades (`fades` 1.05 to 1.10 ms p50) and their summaries (0.61 to 0.63) the B step pays. The temporal pass under `chain`: 0.126 ms (rest) and 0.182 (glide) on the RTX, 0.265 and 0.322 on the RENOIR. The store at the level: 15.5 MB, summaries 88 704 B, parent table 19 964 B; in the glide 10.2 MB, 54 600 B and 627 956 B. The owner's window (2f3450f, windowed, HUD, free pacing): the patch at rest wall p50 10.041 ms, p99 14.357, frame CPU 2.555, march 4.555; the streamed cave wall p99 51.6 to 78.4 ms with the sync's p99 36.8 to 48.3; the RENOIR at the window's size wall p50 46.9 to 51.0 ms, GPU-bound. Rows in `docs/plans/logs/0067.md`, Phases 6 and 7.

**The sync's stages cut on the render thread (Plan 0070 Phase 5, ef0b55c against bdca60a, release, exclusive perf lock, 2026-11-13, clocksource `hpet`).** The RTX 3080 Laptop host's kernel had marked the TSC unstable (`hpet`, about 1.7 us a clock read); figures on another clocksource are not compared with these. The tester's cave and castle benches headless (RTX 3080 Laptop GPU at 2530 x 1378, RADV RENOIR at 1280 x 720), three rounds alternated, each run started at a 1-minute load of 2 or less: `update + poll + sync` p99 cave 22.8 to 24.6 ms against 42.7 to 45.4 at the base, castle 24.4 to 28.2 against 44.6 to 47.2, over Plan 0067's 16.6 ms in both; sync p50 15 to 19 us against 180 to 184; the cave sync's `alloc` p99 6.9 to 7.4 ms against 26.0 to 27.2 (the emitter rescan set as dense bits) and `light_place` p50 2 to 3 us against 165 to 167 (the light walks on change); the tip's largest p99 stage is `light_note` (8.2 to 8.4 ms, `exposed_emitters`). The autopilot of section 5's protocol (`--seed 1`, 1280 x 720, RTX, three rounds, loaded by the flights' workers to 6.8 to 8.9): streaming CPU per frame p50 0.038 to 0.039 ms against 0.826 to 6.964, p99 26.4 to 27.9 against 47.6 to 74.1; residency p95 212 to 213 ms against 311 to 333. The fly-through `B` line's sync 2.05 to 2.06 ms against 2.59 to 2.62. The owner's window (Phase 6, windowed, HUD on, both adapters, runs started at loads of 0.1 to 7.7): "overall TIP is slightly better for fps, picture is same"; `update + poll + sync` p99 by hand 21.1 to 28.6 ms at the tip against 41.1 to 161.0 at the base; in the RTX's `--glide 300` cave flight the per-second `light place` p99 read over 20 ms in 19 of the tip's 30 lines (max 846 ms) and 12 of the base's 73 (max 330 ms), never seen headless. Rows in `docs/plans/logs/0070.md`, Phases 2 to 6.

**The sync's light stages (Plan 0074 Phase 4, d20bf31 against its Phase 1 binary, release, exclusive perf lock, 2026-11-14, clocksource `hpet`, lamp off).** The tester's cave and castle benches headless (RTX 3080 Laptop GPU at 2530 x 1378, RADV RENOIR at 1280 x 720), three rounds alternated, each run started at a 1-minute load of 2 or less: `update + poll + sync` p99 cave 18.3 to 23.7 ms (RTX) and 18.0 to 18.7 (Radeon) against 23.5 to 26.2 at the base, castle 20.4 to 22.7 against 24.5 to 25.8, over Plan 0067's 16.6 ms in both; `light_note` p99 2.5 to 2.6 ms against 8.3 to 8.5 (exposed emitters by masks, the chunk's lists looked up per record; 68.0 to 70.5 % under), with one Radeon castle round at 4.2 ms; the tip's largest p99 stage is `alloc` (7.3 to 8.8 ms). The windowed `light place` spikes were not reproduced headless: glide 1000 at the headless step reads no line over 20 ms at the tip or the base (max 19.4 and 17.1 ms), no eviction in any run, and in Phase 1's thread-CPU reading build the place stage's CPU time stays about 3 ms while its wall above that is off-CPU. The glide's `update + poll + sync` p99 54.5 to 55.2 ms against 62.4 to 64.6. The autopilot of section 5's protocol (`--seed 1`, 1280 x 720, RTX, loaded by the flights' workers to 6.9 to 8.6): streaming CPU per frame p50 0.038 to 0.039 ms at both; residency p95 188 to 212 ms against 210 to 211. The fly-through `B` line's sync 1.89 to 1.92 ms against 2.02 to 2.11. The owner's window (Phase 5, windowed, RTX, HUD on, runs started at loads of 1.1 to 7.9): "no difference" in the picture, the hitches "mostly same"; by hand, in the castle and on the tester's cave path no line's `light place` over 20 ms and no eviction; in the `--glide 300` cave flight the light buffer fills and from about 4 s in every second's slowest sync is `light place` (up to 1186 ms at the tip and 1050 ms at the base, over up to 3697 evictions and 99.9 M scan steps, r 0.997 to 0.9996 against the scan steps), the owner's "staggers at 30 to 40 s, fps drops to 15", while `alloc` stays at 5.2 to 7.6 ms. Rows in `docs/plans/logs/0074.md`, Phases 1, 3, 4 and 5.

**The list of analytic lights (Plan 0058, the Phase 3 cost table at a616204 and Phase 5g at 71b4478, release, headless, `--level --seed 1`, 1920 x 1080, FSR 2 at 1.5, the frame line's median of 30 timed frames after 900, exclusive perf lock, each run started at a 1-minute load of 2 or less except one 8-light arm at 9.4 that was rerun at 15.3, one round each against the plan's three).** RTX 3080 Laptop GPU, lamp off, at the spawn: the light pass reads 0.031 ms with no light, then 0.142, 0.328, 0.730, 1.430 and 2.699 ms at 1, 2, 4, 8 and 16 list lights. That is 0.167 to 0.175 ms per light in reach, against the plan's projection of 0.08 to 0.12. At the vein it reads 0.031 ms with none, 0.556 at 8 and 1.050 at 16. The GPU frame is 2.181 ms with none and 5.314 ms with sixteen (5.406 with the lamp and fifteen). RADV RENOIR at the spawn: the light pass reads 0.221 ms with none and 24.790 ms at sixteen, 1.54 ms per light, and the GPU frame 17.072 against 43.882 ms. At the vein it reads 8.974 ms at sixteen and the frame 26.900. **The RENOIR's light pass at sixteen is over section 2's 6.0 ms lighting line**. The plan claims no line, and Plan 0069's Portable profile caps the count (4, a guess). Denoise: any shaded light switches off Plan 0065's lamp-off skips, so with one light or more the a-trous at 4 iterations reads 0.537 to 0.562 ms (0.164 with none) on the RTX, 0.296 to 0.310 at 2 and 0.176 to 0.186 at 1. The RENOIR reads 3.220 to 3.243 ms at 4 with any light or the lamp, over the 3.0 ms denoise line. Temporal: 0.088 to 0.096 ms on the RTX against 0.055. Probes and gather with the lamp's bounce on: 0.006 and 0.126 to 0.161 ms with 0, 1 or 4 flagged lights at 900 frames, which is the frozen channel (0.138 ms probes at 300 frames). The steps view counts exactly one shadow ray per light in reach per casting pixel. The sync's stages are unmoved: the list never reaches the mirror. Rows in `docs/plans/logs/0058.md`, Phase 3 and Phase 5g.

**The frame's bind groups kept and the march's discarded reads removed (Plan 0044 Phases 3 and 5, base 371fa7a, Phase 3 2056310, tip 777abc0, release, headless, `flythrough --autopilot --headless --seed 1`, the defaults: 1280 x 720 output, FSR 2 from 853 x 480, half lighting, streamed level; the three binaries alternated over two rounds, exclusive perf lock, each run started at a 1-minute load of 2.00 or less, 1.63 to 2.36 at the lock, 2026-11-15/16).** p50 in ms, base then tip, per round. RTX 3080 Laptop GPU: march GPU 1.574, 2.397 against 1.478, 1.436 (the base's rounds swing with the flight's frame count, 3434 and 2457 frames; the tip's read 4082 and 3969); hit pass 0.084, 0.127 against 0.081, 0.084; light pass 0.109, 0.191 against 0.089, 0.090. RADV RENOIR: march GPU 9.047, 8.799 against 8.941, 8.972; hit pass 0.906, 0.946 against 0.925, 0.852; light pass 0.644, 0.639 against 0.640, 0.641; the Phase 3 binary alone read 9.427 and 9.195 there, inside the flight-to-flight swing. Encode CPU (`gpu_frame.rs` `reading::encode_cpu_time_at_the_default_sizes`, 853 x 480 to 1280 x 720 under FSR 2, 300 encodes, loaded 3.7 to 9.5): p50 0.20 to 0.03 ms on the RTX and 0.22 to 0.06 ms on the RENOIR. The hit and light passes are far under the 3.0 and 6.0 ms lines here; the upscale line stays over under FSR 2 (above). Rows in `docs/plans/logs/0044.md`, Phase 3 and Close.

**The sun and the sky on the outdoor hills (Plan 0032 Phase 7, a68f2659 plus Phase 7's tree, release, headless, `flythrough --source outdoor --headless --seed 1 --render 1920x1080 --frames 300` at main's defaults: FSR 2 at 1.5, the far normal 2..6, the coarse shadow walk, half lighting, one sky ray; median of the last 30 frames, three runs each, the median of the three; exclusive perf lock).** NVIDIA GeForce RTX 3080 Laptop GPU (driver 615.78.08, the default adapter, dev reference) and AMD Radeon Graphics, RADV RENOIR (Mesa 26.2.5, `--adapter AMD`, the slow adapter). The base rows are the level's and the cave's spawns with no sky at 356b32a1 (Plan 0069's tip, this branch's base). Every run's 1-minute load was 3.2 to 36.6 (the flights' own workers), so each reading is loaded (indicative). Every streamed run but one AMD noon and one AMD noon sky-off run reached full residency after its frame 270 (the ready time plus 270 median frames), so those are unsettled; the level is not streamed. Times in ms; "sky" sums the three sky passes.

| adapter | run | GPU frame | sky | light | temporal | atrous | composite | exposure |
|---|---|---|---|---|---|---|---|---|
| RTX 3080 Laptop | outside, 12:00 | 5.593 | 0.079 | 1.040 | 0.086 | 0.415 | 0.145 | 0.032 |
| RTX 3080 Laptop | outside, 12:00, `--sky off` | 4.851 | 0 | 0.203 | 0.088 | 0.431 | 0.142 | 0.033 |
| RTX 3080 Laptop | outside, 00:00 | 5.463 | 0.077 | 1.010 | 0.085 | 0.407 | 0.145 | 0.032 |
| RTX 3080 Laptop | outside, 00:00, `--sky off` | 5.030 | 0 | 0.211 | 0.091 | 0.446 | 0.144 | 0.034 |
| RTX 3080 Laptop | inside, 12:00 | 5.053 | 0.084 | 0.893 | 0.084 | 0.470 | 0.147 | 0.034 |
| RTX 3080 Laptop | inside, 12:00, `--sky off` | 4.217 | 0 | 0.111 | 0.083 | 0.478 | 0.145 | 0.034 |
| RTX 3080 Laptop | base, level | 3.310 | 0 | 0.099 | 0.098 | 0.623 | 0.164 | 0.036 |
| RTX 3080 Laptop | base, cave | 5.940 | 0 | 0.261 | 0.153 | 1.041 | 0.231 | 0.053 |
| RADV RENOIR | outside, 12:00 | 45.810 | 0.316 | 10.515 | 0.640 | 2.542 | 1.320 | 0.029 |
| RADV RENOIR | outside, 12:00, `--sky off` | 37.292 | 0 | 1.223 | 0.685 | 2.692 | 1.453 | 0.029 |
| RADV RENOIR | outside, 00:00 | 45.716 | 0.315 | 10.512 | 0.626 | 2.539 | 1.313 | 0.029 |
| RADV RENOIR | outside, 00:00, `--sky off` | 36.032 | 0 | 1.206 | 0.631 | 2.538 | 1.305 | 0.029 |
| RADV RENOIR | inside, 12:00 | 37.519 | 0.315 | 8.224 | 0.641 | 2.630 | 1.398 | 0.029 |
| RADV RENOIR | inside, 12:00, `--sky off` | 29.423 | 0 | 0.768 | 0.627 | 2.604 | 1.331 | 0.029 |
| RADV RENOIR | base, level | 20.802 | 0 | 0.621 | 0.718 | 3.298 | 1.433 | 0.029 |
| RADV RENOIR | base, cave | 22.927 | 0 | 0.860 | 0.712 | 3.306 | 1.437 | 0.029 |

The default look is `face`, so the noon run is also the `--look face` reading. The runs and the
per-run figures are in `docs/plans/logs/0032.md`, Phase 7.

**The forest at a low sun (Plan 0032 Phase 8, a66a9463 plus Phase 8's tree, release, headless, `flythrough --source outdoor --headless --seed 1 --render 1920x1080 --frames 300 --spawn forest --sun 10,180 --trees T` at main's defaults, median of the last 30 frames, three runs each and their median; exclusive perf lock; every run loaded by its own workers (start load 3.6 to 15.8) and unsettled at its frame 270).** NVIDIA GeForce RTX 3080 Laptop GPU (driver 615.78.08, dev reference) and AMD Radeon Graphics, RADV RENOIR (Mesa 26.2.5, `--adapter AMD`, the slow adapter). Times in ms; against §2's lighting line (6.0 ms in the HUD) and 16.6 ms a frame.

| adapter | trees | light | GPU frame | light vs 6.0 | frame vs 16.6 | key ray steps mean / p99 (`--view steps`) |
|---|---|---|---|---|---|---|
| RTX 3080 Laptop | none | 1.334 | 5.640 | under | under | 22.72 / 57 |
| RTX 3080 Laptop | forest, fill 128 | 1.392 | 5.425 | under | under | 19.21 / 79 |
| RTX 3080 Laptop | forest, fill 64 | 1.318 | 5.201 | under | under | 19.28 / 89 |
| RTX 3080 Laptop | forest, fill 128, `--shadow-walk full` | 1.667 | 5.649 | under | under | 24.12 / 92 |
| RADV RENOIR | none | 10.641 | 38.096 | over | over | 23.23 / 61 |
| RADV RENOIR | forest, fill 128 | 12.481 | 39.307 | over | over | 19.24 / 80 |
| RADV RENOIR | forest, fill 64 | 12.521 | 39.624 | over | over | 19.31 / 92 |
| RADV RENOIR | forest, fill 128, `--shadow-walk full` | 15.135 | 41.984 | over | over | 24.17 / 95 |

Ratios of the light pass: forest/open at fill 128 1.04 (RTX) and 1.17 (RADV), at fill 64 0.99 and 1.18; fill 64/128 0.95 and 1.00; full/coarse at fill 128 1.20 and 1.21. The `--look face` configuration is the default look. Per-run figures in `docs/plans/logs/0032.md`, Phase 8.

**The outdoor tester benchmark (Plan 0088 Phase 2, 2026-11-17, release `flythrough` at fa8394ea, before Plan 0086's outdoor light; the tester's exact outdoor bench flags `--source outdoor --spawn outside --trees sparse --sun 10,180 --lamp off --tester --path outdoor --path-mode bench --render 1280x720 --profile floor` plus `--headless --adapter NVIDIA|RENOIR --report DIR`; one run per adapter, exclusive perf lock; started at a 1-minute load of 19.5 and 25.6 with two other lanes building, so both are loaded (indicative)).** The 73.4 s path at the headless clock's 1/60 s, 4283 measured frames after the 2 s warm-up; `run.json`'s percentiles in ms. Every figure: [Plan 0088's readings log](../plans/logs/0088.md), Phase 2.

| adapter | wall p50 / p95 / p99 | CPU p50 / p95 / p99 | GPU p50 / p95 / p99 | over 33 ms | load |
|---|---|---|---|---|---|
| RTX 3080 Laptop (driver 615.78.08) | 20.4 / 57.0 / 114.0 | 14.2 / 48.6 / 102.3 | 4.61 / 10.90 / 13.00 | 837 | 40.1 s |
| RADV RENOIR (Mesa 26.2.5) | 29.5 / 54.7 / 77.5 | 6.3 / 31.2 / 54.8 | 22.34 / 25.14 / 26.33 | 1331 | 20.1 s |

At 1280 x 720 in the Floor profile (853 x 480 internal, FSR 2) the RTX's GPU frame p50 sits under Plan 0032's 1080p readings above (5.1 to 5.6 ms) and RADV's under its 37.5 to 45.8; the wall and CPU tails on the RTX are the loaded CPU's (the bake queue's step and the sync, section 5).

**The render profiles (Plan 0069 Phase 7, 2026-11-17, release `flythrough` at the code of
9390b033, `--headless --hud --frames 600 --adapter NVIDIA|Radeon --profile P`, the vein
`--level --seed 1 --pose vein`, the hall `--source showcase --shot hall`, the castle `--castle
--castle-shot gate --seed 1`, the frame line's `march GPU p50` (FSR 2's passes included, the
bake slice not), the median of three alternated rounds, one exclusive perf lock per run; most
rounds read a 1-minute load over 2, so every figure is loaded (indicative)).** Every pass and
round: [Plan 0069's readings log](../plans/logs/0069.md), Phase 7. Frame GPU in ms, vein /
hall / castle; the line each profile is held to is the owner's pick at Plan 0069's window.

- *Floor, RTX 3080 Laptop* (1920 x 1080 from 1280 x 720, FSR 2): 2.92 / 4.25 / 4.19; FSR 2
  1.32 to 1.34; hit 0.37 / 2.44 / 1.49. Inside the 16.6 ms line.
- *Floor, RADV RENOIR*: 19.81 / 34.82 / 30.71; FSR 2 9.38 to 9.52; hit 3.20 / 21.62 / 13.51.
  Over the 16.6 ms line, as expected below the floor's class.
- *Portable, RTX 3080 Laptop* (1280 x 720 from 853 x 480, FSR 2, 2 translucent layers, light
  cap 4, no lamp bounce, residency 12, 18, 1): 1.21 / 1.99 / 1.84; FSR 2 0.60 to 0.61.
  The `own` arm: 1.10 / 1.63 / 1.48.
- *Portable, RADV RENOIR*: 8.14 / 16.30 / 13.88; FSR 2 4.06 to 4.08; hit 1.44 / 10.52 / 6.74.
  Inside the 33.3 ms column. The `own` arm: 5.77 / 13.80 / 11.68.
- *Performance, RTX 3080 Laptop* (2530 x 1378 from 1265 x 689, FSR 2): 3.39 / 4.72 / 4.63;
  FSR 2 1.80 to 1.84. No gate.
- *Performance, RADV RENOIR*: 22.59 / 36.80 / 34.18; FSR 2 12.43 to 12.59. No gate.
- *High, RTX 3080 Laptop* (1920 x 1080 at internal scale 1, FSR 2): 5.31 / 8.25 / 7.77; FSR 2
  2.01 to 2.04; hit 0.82 / 5.25 / 3.27. No gate.
- *High, RADV RENOIR*: 38.64 / 68.46 / 60.07; FSR 2 15.34 to 15.82. Not a profile for this
  adapter (owner, Plan 0069's window: "no"). No gate.

**Still outdoor light (Plan 0086, ADR-0070; the close review's readings of 2026-11-19, the plan's tip f8cbcb2d, release `flythrough --headless --frames 1500 --render 1920x1080 --bake-budget frame:40` at the defaults (FSR 2 at 1.5, half lighting, `leaf shadow voxel`, `penumbra disc`, `sky normal smoothed`, `sky anchor 4`, `key history 256`), the live queue drained (0 jobs, coarse 0 at the end), the frame line's median of the last 30 frames, exclusive perf lock; another lane's tests ran beside the runs, so the 1-minute loads at the starts read 3.2 to 13.4: loaded, indicative).** Times in ms. RTX 3080 Laptop (`--adapter NVIDIA`): the hills at noon GPU frame 6.665, hit 2.328, light 1.570, sky irradiance 0.039, temporal 0.477, a-trous 0.552; the forest at 15:00 (`--spawn forest --trees forest`) 6.795, hit 1.209, light 2.529, temporal 0.551, a-trous 0.679. RADV RENOIR (`--adapter AMD`): the hills 53.276, hit 21.868, **light 8.025**, temporal 3.461, a-trous 3.552; the forest 43.719, hit 10.733, **light 12.034**, temporal 3.742, a-trous 3.827. Against Plan 0032 Phase 7's hills at noon with the ray on everywhere (RENOIR light 10.5, RTX 0.9 to 1.4, loaded, not drained): the RENOIR's outdoor light pass at rest is still over the 4.0 ms lighting line, by 2 x on the hills and 3 x in the forest, and its denoise (7.0 to 7.6) over the 2.0 ms line; the RTX is within every Floor line. The plan's own readings along the way (`docs/plans/logs/0086.md`): the tunnel's stairs at rest (Phase 24c, `74f64334`, RTX) light 0.724 at 1080p and 1.13 at 1440p, temporal 0.37 / 0.60, a-trous 0.49 / 0.82, GPU frame 4.53 / 7.55 against a wall of 13.4 / 16.6 ms whose CPU half is the queue's step on the main thread (6.3 ms p50 against the 1.0 ms streaming line; 13.5 at the plan's base), the RENOIR there light 3.88 + sky 0.34, hit 13.99, frame 34.5; the hills and the forest at 1440p drained (the cost table of 2026-11-19, RTX) light 2.80 to 2.94 at noon, 4.06 to 4.16 at 17:00, 4.51 to 4.74 in the forest at 15:00, the temporal pass +0.11 to +0.24 from the fix of the dig's view-wide flash (`00805204`); the a-trous tap across the cast flag +0.008 RTX, +0.214 RENOIR (Phase 21 call 4); the ring's sky rays on a dig's frame +0.6 RTX, +1.28 RENOIR (Phase 24a). The bake under a sky lengthens the pre-roll on the RTX's hills from 7.3 s (Phase 4, 447 steps) to 14.4 s (Phase 9, the column wait off; 21.2 s with it on), and the queue's step with the channel reads 5.7 to 7.6 ms p50 while baking (Phase 5). `--leaf-shadow absorb` reads 17.4 to 18.1 ms of light pass on the RTX in the forest and 87.7 to 88.3 on the RENOIR (Phase 14; a setting, off). The frame targets at 1080p read 536 150 828 B on both adapters (the pair and the samples target +20 B a lighting pixel; `frame_bytes_hold_the_visibility_pair`).

**The light's transitions (Plan 0094, ADR-0076 rules 3 and 5; the readings log
`docs/plans/logs/0094.md`, Phase 5 bar 5, loaded, indicative):** the no-record prior's arm A
(the default: a face with no record casts the sky ray seeded from the column) during a stamped
castle's bake, the light pass every frame over the drain at the rig's 320 x 180 lighting grid:
RTX 3080 Laptop 0.276 ms mean (p95 0.389), RADV RENOIR 1.169 ms mean (p95 1.625; 1.622 over the
first 100 frames), against arm B's 0.261 and 0.574; A falls to B once the store holds every
face (about frame 2200); 0 rays at rest on the hills. **1080p's half tier is not read**: a
linear scale of the RENOIR's 1.62 ms gives about 14.6 ms for the bake's length (a guess, over
the 4.0 ms lighting line), owed to Plan 0096, which reads the RENOIR's outdoor light pass. The exposure's rate cap and the
ceiling's step are one clamp each in the exposure pass (0 ms by construction, not read); the
no-ray fade's temporal and composite cost is not read separately. The exposure's meter tile
grows 8 to 16 B and its uniform 96 to 112 B: +115 216 B at 1280 x 720 internal. **The
exposure's floor walk (Plan 0097, ADR-0081):** the render thread's CPU reads the record under
the eye once a frame (`sky_over_floor`); over the canopy walk between window W2's two poses
(1130 frames at 15 voxels a second, the store baked along it) it reads 0.226 us mean and
0.501 us max a frame, the RTX 3080 Laptop's host, release, `80cd7134`, exclusive perf lock,
1-minute load 10.2 (loaded, indicative; `docs/plans/logs/0097.md`, Phase 4). Plan 0097 Phase
1's 600 eyes on the shade pose's store read 1.694 us mean, 10.477 us max (`ea15d52f`, loaded).
The GPU side is one `select`, one `max` and one `mix` in the exposure pass (not timed). Highlight
protection (Plan 0097 Phase 3a) bins the tiles' maxima into a 128-bin workgroup histogram in
the same pass (not timed), grows the uniform 112 to 128 B and adds a 16 B state: +32 B, so the
exposure's growth since Plan 0094 is +115 248 B at 1280 x 720 internal (the frame inventories,
lavapipe).

**The key light with no screen history (Plan 0095, ADR-0079; 2026-11-20, release `flythrough`
at `b46cb3eb`'s render code, `--headless --source outdoor --frames 1500 --render 1920x1080`,
the forest `--spawn forest --trees forest --time 15:00` and the hills `--time 12:00`, undrained
(the queue at the default budget) and drained (`--bake-budget frame:40`, 0 jobs at the end),
the HUD's last light row, exclusive perf lock; the RTX runs started at a 1-minute load of 3.7
to 7.1 (loaded, indicative), the RENOIR's at 0.5 to 4.2; every row in
`docs/plans/logs/0095.md`, Phase 10).** The fly-through's defaults since the plan (the far reads
the baked record, else the column map's horizon toward the sun; the approach penumbra near, on
every adapter and profile, the owner's pick of 2026-11-20) against the before (`--key-far ray
--penumbra disc`), light pass in ms, undrained / drained:

| scene | RTX 3080 Laptop, picks | RTX, before | RADV RENOIR, picks | RENOIR, before |
|---|---|---|---|---|
| forest 15:00 | 3.04 / 3.43 | 1.19 / 1.35 | **34.16 / 33.95** | 12.04 / 12.03 |
| hills 12:00 | 1.11 / 1.28 | 0.84 / 0.94 | **11.18 / 11.18** | 8.01 / 8.01 |

**The RENOIR's light pass is over the 4.0 ms lighting line, named: the approach penumbra's
cost** (+22.1 ms in the forest, +3.2 on the hills); Plan 0096 Phase 6 (the slow adapter's light
pass, lever by lever) takes it as a named lever. The RTX is inside the line. The pre-roll is a
check, not a claim: the coarse pass ends at 7.4 to 7.6 s (picks) against 7.2 to 7.4 (before) in
the RTX's forest and at 6.0 s in both arms on its hills, at 10.2 to 11.6 against 10.3 to 10.5 s
in the RENOIR's forest and 7.2 to 7.4 s on its hills; the first frame follows 0.9 to 1.5 s
later. The key re-bake under the day cycle (Phase 8's readings at `a92aca70`, `--day-cycle`,
1200 s day, a key step every 10 s, no drain, 3600 frames): the RTX re-keys every marked block
2.3 to 5.7 s after a step (forest 48.9, hills 99.5 key jobs a second), the RENOIR in 9.1 to
49.5 s in the forest and 7.1 to 14.6 s on the hills (3.0 and 4.7 a second).
The dig's frame (Plan 0086 Phase 24a's rig `flicker::reading::flicker_dig_split` at 17:00,
release, the tip's render code at `85fb3de7` against main `e6582b9e`, exclusive perf lock; RTX
3080 Laptop at a 1-minute load of 3.9 / 2.5, RADV RENOIR at 2.2 / 0.6): the light pass rises at
the dig's frame by +0.16 ms on the RTX (main +0.18) and +1.31 ms on the RENOIR (main +1.12), to
0.40 and 3.31 ms; the queue's step at the dig's frame 17.5 against 16.4 ms (RTX) and 2.80
against 2.84 ms (RENOIR); the ring's samples a step on the RENOIR 424 442 against 514 367 (main),
the same 1 971 264 on the RTX, each ring brick direction drawing `KEY_BAKE_RAYS` (64) key rays
more (Plan 0095's readings log, Phase 10).

**The bake queue's drain, step and the slow adapter's light pass (Plan 0096 Phase 9,
2026-11-22/23, release `flythrough` at `91003891`'s tree, headless, `--source outdoor`, the
forest `--spawn forest --trees forest --time 15:00` and the hills `--time 12:00` from the spawn;
RTX 3080 Laptop at `--profile floor --render 1920x1080`, RADV RENOIR at `--profile portable
--render 1280x720` (internal 853 x 480); exclusive perf lock per run, the 1-minute load at the
lock under 2 (quiet) unless said; every row in `docs/plans/logs/0096.md`, Phase 9).** The drain
sweep from the first world frame, quiet rows (in view landed / residency empty / wall p50 while
in view waits / step B p50, p99): RTX forest at drain 1.5, 4.0, 8.0 ms: 36.5 / 49.1 s / 26 ms /
15.5, 27.9; 29.5 / 42.6 / 39 / 14.8, 44.7; 25.1 / 41.4 / 37 / 14.1, 50.6. RTX hills: 12.2 /
37.1 / 30 / 12.9, 25.9; 9.1 / 36.7 / 38 / 12.6, 39.6; 8.3 / 37.6 / 37 / 12.9, 29.6. RENOIR
forest at 3.0, 6.0, 12.0: 50.4 / not empty at 220 s / 40 / 1.65, 8.13; 27.1 (37.6 on one of four
runs) / not empty / 46 / 1.04, 9.55; 18.3 / not empty / 56 / 0.98, 14.67. RENOIR hills: 28.0 /
167.7 / 31 / 1.60, 7.59; 16.7 / 152.5 / 37 / 1.22, 8.51; 10.1 / 146.2 / 48 / 0.92, 13.32. With
no drain value (the budget alone) the RENOIR lands its view in 184.2 s (forest) and 95.7 s
(hills). Picked by the owner's rule: Floor 1.5, Portable 6.0. The tester cave bench (RTX 1080p):
`update + poll + sync` p99 14.82 ms, `alloc` p99 6.42; the 1/16 light-buffer glide (loaded,
2.33): `light place` p50 / p99 6.84 / 8.84 ms. The light pass at rest (`PassTimer`, 900
frames; light / frame ms), RENOIR at 720p: forest approach 17.38 / 32.70, disc 5.13 / 20.20,
disc with the key cap and 2 a-trous 5.04 / 19.36; hills approach 4.81 / 24.20, disc 2.42 /
21.86, both levers 2.34 / 21.20; drained within 0.15 ms of undrained. RTX at 1080p: forest
approach 2.98 / 7.31, disc 1.09 / 5.36, both 0.98 / 4.98; hills 1.08 / 6.06, 0.66 / 5.98, 0.54
/ 5.70. Against the lines: section 2's table "The bake queue's drain and step".

**The owner's window at 2530 x 1378 (Plan 0096 Phase 8, 2026-11-22, release `1844a99b`,
windowed, internal 1687 x 919 at FSR 2's 1.5, 3.79 times the Portable profile's pixels; not
against the Portable line, which is defined at 720p).** RTX Floor, forest: frame CPU p50 28.0
ms against a march GPU of 11.6, wall p50 / p99 28.3 / 59.1, the step in regime B 28.0 / 51.1,
about 30 fps, in view not landed in 17.9 s; RTX hills at 17:00: in view landed 16.1 s after
the first world frame, the residency empty at 27.5 s, wall p50 13.3 ms, 68 to 77 fps from 44 s;
RTX forest at the profile's tiers: in view landed 22.3 s after the first world frame. RENOIR
Portable (before Phase 9's 720p in a window): forest wall p50 106.8 ms, GPU 85.5 (hit 25.51,
light 22.47, FSR 2 16.37, temporal 12.00, a-trous 6.18); hills at 17:00 97.5 ms, GPU 84.65,
in view landed 35.2 s after the first world frame; the submits drew 8.8 to 9.7 ms of a granted
6.0. The RTX cave glide: `light place` p50 / p99 2.16 / 6.19 ms, slowest sync 21.0 ms, the
owner's "no stagger". Every run in `docs/plans/logs/0096.md`, Phase 8.

**The owner's window W2 (2026-11-23, Plan 0098's session 98b, runs 6, 7 and 7b on Plan 0096's
release at its Phase 9, windowed at 2530 x 1378; single HUD readings from the owner's stills,
not medians, so indicative).** RENOIR, Portable, forest at 15:00, the profile at its hint in a
window (**the march and the light pass at 853 x 480, FSR 2 to 1280 x 720, that stretched to
the window**): 23.7 fps, frame 42 ms, GPU 33 ms (hit 8.30, light 13.54, temporal 3.99, FSR 2
4.49), 244 chunks in view waiting, the drain projected at 9.8 s; a later still of the same run
at rest reads 25.2 fps, wall p50 39.2 ms, GPU 34.7 (light 19.35). Against W96's 8 to 9 fps at
the window's own size, and against the Portable line of 33.3 ms: over it, outdoors, by the
light pass and the hit. The owner: "frames around 25, softer acceptable". RTX, Floor, forest,
a fresh start at `--bake-tiers 4096,4096,4096`: 49.6 fps with the bake step at 11.78 ms, 3 099
waiting, the drain projected at 233.8 s (the cost of 4 096 in every band; the owner on the
profile's tiers against it: "shadows are fine").

**The calm light by default (Plan 0098 Phase 4, 2026-11-23, ADR-0082: no light history, key far
column stepped at the band's half, far normal 4..8, the disc penumbra; release `flythrough`,
headless, `--source outdoor --render 1920x1080 --frames 1500`, the Floor profile (FSR 2 at 1.5)
on both adapters, the frame line's median of 30; the forest `--spawn forest --trees forest
--time 15:00`, the low sun `--spawn forest --trees forest --sun 10,180`, the hills `--time
12:00`; exclusive perf lock per run, nice 19, the 1-minute load at the lock 0.9 to 3.5; every row
in `docs/plans/logs/0098.md`, Phase 4).** Light / temporal / a-trous / GPU frame in ms; before
is the branch with Plan 0096 merged (`311c5e6a`) at its old defaults (light history on, key far
baked blended, far normal 2..6, the approach penumbra) and with `--calm` (the five picks), after
is the new defaults with no flag:

| scene | RTX before | RTX before `--calm` | RTX after | RENOIR before | RENOIR before `--calm` | RENOIR after |
|---|---|---|---|---|---|---|
| forest 15:00 | 3.16 / 0.93 / 0.65 / 7.66 | 1.08 / 0.74 / 0.75 / 5.43 | 2.16 / 1.57 / 1.51 / 10.39 | **33.93** / 6.73 / 3.69 / 67.44 | **11.30** / 6.18 / 4.58 / 45.19 | **11.36** / 6.26 / 5.28 / 51.67 |
| forest `--sun 10,180` | 2.37 / 0.89 / 0.62 / 6.74 | 0.58 / 0.78 / 0.78 / 5.00 | 1.27 / 1.71 / 1.57 / 10.20 | **27.69** / 6.71 / 3.69 / 61.12 | **6.17** / 6.15 / 4.58 / 40.04 | **6.21** / 6.23 / 5.28 / 43.85 |
| hills 12:00 | 1.14 / 0.76 / 0.53 / 6.31 | 0.66 / 0.85 / 0.65 / 6.35 | 1.07 / 1.38 / 1.03 / 10.02 | **11.17** / 5.95 / 2.93 / 53.31 | **6.02** / 6.27 / 3.53 / 49.98 | **6.13** / 6.44 / 4.41 / 60.67 |

The RTX's after column ran in a slower clock state than its before columns: read back to back
at 15:48 (load 2.7), the forest under the old binary's `--calm` reads 2.45 / 1.78 / 1.64 / 11.70
and the new defaults 2.32 / 1.69 / 1.61 / 11.10, so on the RTX the defaults cost what `--calm`
did, and the before columns are the faster state's. On the RADV RENOIR (no such swing) the new
defaults' light pass reads as `--calm`'s, 11.4, 6.2 and 6.1 ms against the old defaults' 33.9,
27.7 and 11.2: the approach penumbra's cost is gone from the default (Plan 0095's table above),
and the RENOIR at 1080p is still over the 4.0 ms lighting line by 1.5 to 2.8 times. The temporal
pass holds (5.9 to 6.4 ms RENOIR) and the a-trous rises with no history (3.7 to 5.3 ms RENOIR).

**The unshown key ray not cast (Plan 0098 Phase 13, 2026-11-23, ADR-0082): this row replaces the
after columns above as the defaults' figures.** Under the defaults the light pass casts no key ray
at a cube face wherever the light history's setting is off, the penumbra is not approach and the
sky did not jump (the cast sample was shown nowhere). The same protocol as above, on the RTX
3080 Laptop and the RADV RENOIR (`--adapter`), exclusive perf lock per run, nice 19, the 1-minute
load at the lock 0.9 to 6.6; before is `4c61a3e8` (the Phase 4 defaults), after is the same tree
with the skip (Plan 0098 Phase 13's commit); each read twice, interleaved (before, after, before,
after, 19:42 to 20:09). Light / temporal / a-trous / GPU frame in ms, the first run, and the
second run's light in brackets:

| scene | RTX before | RTX after | RENOIR before | RENOIR after |
|---|---|---|---|---|
| forest 15:00 | 1.07 (1.64) / 0.78 / 0.78 / 5.50 | **0.22 (0.27)** / 1.04 / 0.96 / 5.77 | 11.30 (11.31) / 6.18 / 4.58 / 45.13 | **1.61 (1.61)** / 6.18 / 4.58 / 35.43 |
| forest `--sun 10,180` | 0.89 (0.96) / 1.20 / 1.15 / 7.27 | **0.26 (0.27)** / 1.14 / 1.20 / 6.71 | 6.17 (6.22) / 6.15 / 4.58 / 39.93 | **1.61 (1.61)** / 6.15 / 4.57 / 35.36 |
| hills 12:00 | 0.73 (0.68) / 0.93 / 0.72 / 6.95 | **0.40 (0.42)** / 0.85 / 0.69 / 6.23 | 6.02 (6.13) / 6.27 / 3.53 / 49.93 | **2.07 (2.07)** / 6.28 / 3.53 / 46.00 |

On the RADV RENOIR the light pass falls 7.0, 3.8 and 2.9 times and is under the 4.0 ms lighting
line in all three scenes; the RTX's GPU frame swings by its clock state run to run (5.5 to 8.0 ms
before), so its frame column is not a gain.

**The dig hold's cost (Plan 0098 Phase 17, 2026-11-24, ADR-0082's Outcome; `--dig-hold on`, the
held record in the store).** The dig rig's timers (`dig_line::reading::dig_timeline`, a release
`gpu_outdoor` test binary at `39354892`, exclusive perf lock, each run started at a 1-minute load
under 2.1, mains power; sequences A, three digs down one ray, and B, six digs eight frames
apart): the RTX 3080 Laptop at the owner's pose of window 98d (the forest at `--sun 10,180`,
960 x 540 from 640 x 360) and the RADV RENOIR at Portable (the forest at 15:00, 1280 x 720 from
853 x 480). The largest value over each dig's frame and the three after, against the frames
before the first dig; temporal pass / GPU frame in ms:

| adapter, sequence | rest | `on` | `off` |
|---|---|---|---|
| RTX, A | 0.26 / 1.26 | 0.27 / 2.8 | 0.26 / 2.0 |
| RTX, B | 0.27 / 1.30 | 0.27 / 1.6 | 0.26 / 1.5 |
| RENOIR Portable, A | 2.89 / 14.20 | 2.53 / 17.5 | 1.77 / 16.8 |
| RENOIR Portable, B | 2.89 / 13.98 | 2.69 / 17.8 | 2.08 / 17.2 |

Met: the temporal pass while a dig is held is at most its rest value plus 0.1 ms on the RTX and
plus 0.7 ms on the RENOIR, and the RENOIR's GPU frame at most `off`'s plus 1.0 ms (the bars of
Plan 0098's "Amendment at window 98d"). The arm it replaced (12 near rays a pixel while a dig is
held, removed at `d6c58a04`) read 4.45 to 9.71 ms of temporal pass on the RENOIR and put its
Portable frame over the 33.3 ms line at window 98d (temporal 19.63 ms, GPU 36.5 ms). The bake
step's main-thread time on a dig's frame is in `04-edits.md` ("An edit's light").
