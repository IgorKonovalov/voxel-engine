# The motion wobble spike: readings log

Branch `spike-motion-wobble` (from Plan 0094's tip 37783825). The owner's complaint (2026-11-20,
after window 94a): "a wobble of everything after turn or when moving, it is clearly seen on far
trees for example when I'm slowly fly, it just can't settle." The owner's direction (the same
day): "almost good picture without wobble is better than perfect picture with wobble.
Especially when we are looking at the far distance. the further the object - more we can
approximate its lighting and we don't really need to recalculate it over and over in case of
motion - only changing parts."

## The rig

`apps/flythrough/tests/gpu_wobble.rs` (throwaway on this branch): the forest of seed 1 at
`LEAF_FILL`, the chunks within 8 of the forest spawn's in x and y (2 down, 3 up; 17 x 17 x 6
chunks, 0.3 s to generate), mirrored whole. The camera at the spawn (2 m up), yaw +90 degrees
from the spawn's heading (west), pitch -4: trunks at every distance, far canopies, little sky.
The frame is the fly-through's default but for size: FSR 2 at 1.5 from 1280 x 720 to
1920 x 1080, sharpen 0.5, lighting half, far normal `[2, 6]`, store read normal, weights face,
voxel leaves, the live queue at the fly-through's settings (coarse 8 samples, near 2), the sun
fixed at elevation 40 azimuth 200, the outdoor auto exposure, dt 1/60.

The protocol per arm: the queue stepped until it drains or `drain` (2000) frames; 240 settle
frames; a **still** strip of 33 frames; a **fly** of 180 frames forward at 1 m/s (1/6 voxel a
frame); a **stop**, held `hold` (300) frames; a **turn** of 60 degrees over 40 frames
(90 deg/s); a **hold** of `hold` frames. Every frame is read back. Display pixels are classed
by their lighting pixel's hit distance: near (under 6 m), mid (6 to 15 m), far (over 15 m),
sky. Per frame pair and class: the share of pixels whose display luma changes by over 4
levels (`T_STILL`), and the mean |dY| in levels. In a hold, each frame is also read against
the hold's last frame ("to-final"): the far class's over-4 share and mean |dY|, the slow part
of the wobble. `VXE_WOBBLE` picks the arm's tokens (the file's `Arm::parse`), `VXE_TEST_ADAPTER`
the adapter. Frames, `<segment>-diff.png` (mean |dY| over the segment, times 8) and the logs
are under the untracked `live/wobble/`.

## Readings

(Readings follow, newest last. Adapter, perf lock and load noted per batch.)

### Batch 1 (2026-11-20, RTX 3080 Laptop, exclusive perf lock, load 2 to 3.3: loaded, indicative)

The queue did not drain in 2000 frames on any arm (the forest's reach); the segments run with
it stepping, as the fly-through does. Classes at the still: sky 1.6 %, near 65.3 %, mid
16.1 %, far 17.0 % of the display.

| arm | still far pair over 4 / mean dY | fly far pair over 4 / mean dY | stop frame 0 vs final, far over 4 / mean dY | stop +120 | hold (after the turn) frame 0 vs final, far | hold +120 | hold +240 |
|---|---|---|---|---|---|---|---|
| default (shaded) | 0.33 % / 0.19 | 14.2 % / 3.11 | 35.0 % / 9.7 | 11.8 % / 2.0 | 59.0 % / 14.0 | 44.7 % / 5.5 | 22.4 % / 3.1 |
| albedo (no lighting: geometry, far normal, FSR 2, jitter) | 0.01 % / 0.01 | 1.34 % / 0.18 | 0.56 % / 0.10 | 0.00 % / 0.01 | 1.08 % / 0.18 | 0.00 % / 0.01 | 0.01 % / 0.02 |

- **The geometry and upscaler path is not the wobble.** In the albedo view the far class's
  frame-to-frame change while flying is a tenth of the shaded one, and after the stop or the
  turn the far image is within 1 % of its final state at once. FSR 2's own motion residue is
  the 1.3 % (its scale-1.5 slow-pan reading of ADR-0034 in a new form), under the eye's line.
- **The shaded image drifts for seconds after motion stops.** Per pair the far class is under
  1 % over 4 levels (so the pair-based rule 1 reading would call it still), but the frame
  against the final one keeps closing for 300 frames: a monotonic sub-threshold drift, the
  thing the eye reads as "it just can't settle". After the 60-degree turn 45 % of the far
  pixels are still over 4 levels from their final value 2 s later, 22 % after 4 s.
- The difference image of the stop's first frame against its final (`live/wobble/default/`,
  stop-000 against stop-final, times 6) puts the drift on the far canopies (every leaf pixel),
  the far trunks, and the penumbrae of the near trunks' ground shadows: the key light's
  visibility mean (`key_mean`, `pair.wgsl`: a plain average up to `key_history` 256 frames,
  restarted on any history rejection) and the sky pair, converging as 1/n from one binary
  sample. The near ground's lit treads and risers drift too (near 14.8 % at the stop's frame 0).

### Batches 1 to 4: the bisect (2026-11-20, RTX 3080 Laptop, exclusive perf lock, load 2 to 4: loaded, indicative)

Each row is one arm on the default frame with one thing changed. "stop" and "hold" columns:
the far class's share over 4 levels / mean |dY| against the hold's final frame, at frames 0,
60, 120 and 240 after the camera stops. The queue did not drain on any arm, 30 000 frames
included (`drain-30000`, 191 s: the forest's reach keeps chunks waiting).

| arm | still far pair over 4 / dY | fly far pair over 4 / dY | stop +0 | +60 | +120 | +240 | hold +0 | +60 | +120 | +240 |
|---|---|---|---|---|---|---|---|---|---|---|
| default | 0.33 % / 0.19 | 14.2 % / 3.11 | 35.0 % / 9.7 | 25.3 % / 4.8 | 11.8 % / 2.0 | 4.8 % / 0.9 | 59.0 % / 14.0 | 50.5 % / 6.8 | 44.7 % / 5.5 | 22.4 % / 3.1 |
| albedo (no lighting) | 0.01 / 0.01 | 1.34 / 0.18 | 0.56 / 0.10 | 0.04 / 0.03 | 0.00 / 0.01 | 0.02 / 0.02 | 1.08 / 0.18 | 0.06 / 0.04 | 0.00 / 0.01 | 0.01 / 0.02 |
| pin (exposure pinned) | 0.35 / 0.19 | 14.3 / 3.13 | 35.0 / 10.0 | 25.2 / 4.8 | 14.4 / 2.3 | 5.7 / 1.1 | 72.8 / 19.6 | 63.0 / 12.1 | 13.4 / 2.1 | 5.1 / 0.8 |
| nqueue (queue frozen after the pre-roll) | 0.33 / 0.19 | 14.3 / 3.11 | 36.3 / 9.4 | 22.2 / 4.0 | 12.0 / 2.2 | 6.4 / 1.3 | 52.3 / 12.8 | 29.5 / 4.5 | 15.1 / 2.5 | 7.7 / 1.7 |
| pin,nqueue | 0.35 / 0.19 | 14.3 / 3.13 | 36.8 / 9.7 | 22.5 / 4.1 | 12.6 / 2.2 | 6.1 / 1.3 | 52.4 / 13.0 | 29.3 / 4.5 | 15.0 / 2.5 | 7.6 / 1.6 |
| drain=30000 (undrained still) | 0.34 / 0.19 | 13.1 / 3.01 | 31.4 / 9.8 | 21.3 / 4.1 | 11.6 / 1.7 | 3.8 / 0.7 | 34.4 / 9.7 | 16.3 / 3.0 | 8.2 / 1.3 | 4.3 / 0.8 |
| off1 (no upscaler, scale 1, no jitter) | 0.12 / 0.18 | 13.3 / 4.61 | 41.8 / 8.7 | 21.9 / 4.1 | 12.9 / 2.0 | 8.3 / 1.2 | 64.4 / 15.2 | 33.8 / 5.9 | 21.1 / 2.5 | 20.0 / 2.3 |
| own (our upscaler at 1.5) | 2.76 / 0.73 | 18.6 / 4.46 | 42.9 / 11.2 | 21.7 / 4.1 | 19.4 / 3.8 | 9.7 / 2.0 | 72.4 / 20.8 | 59.4 / 11.7 | 57.2 / 11.2 | 6.0 / 1.5 |
| keyoff (no key light) | 0.13 / 0.12 | 8.4 / 1.67 | 33.9 / 5.9 | 22.1 / 3.9 | 8.8 / 1.6 | 4.3 / 0.9 | 55.2 / 9.6 | 48.6 / 5.8 | 46.2 / 5.5 | 25.6 / 3.4 |
| keyoff,sky0 (no per-pixel light sample at all; exposure auto) | 0.08 / 0.02 | 2.4 / 0.62 | 27.2 / 5.4 | 16.0 / 2.4 | 11.4 / 1.7 | 5.4 / 1.0 | 55.2 / 7.1 | 52.3 / 6.4 | 51.9 / 6.3 | 10.7 / 1.6 |
| kh=16 (key history 16) | 0.38 / 0.20 | 14.5 / 3.15 | 34.5 / 9.1 | 23.1 / 4.1 | 9.9 / 1.6 | 6.3 / 1.2 | 72.9 / 19.0 | 62.5 / 11.5 | 14.2 / 2.2 | 5.4 / 0.8 |
| kh=16,sh=16 | 0.60 / 0.26 | 13.7 / 3.07 | 36.5 / 9.2 | 21.2 / 3.8 | 9.1 / 1.6 | 6.3 / 1.1 | 62.8 / 13.7 | 49.8 / 6.2 | 44.3 / 5.4 | 13.4 / 1.9 |
| knj (key ray at the disc's centre, no jitter) | 0.34 / 0.19 | 13.8 / 3.05 | 34.4 / 9.8 | 24.3 / 4.8 | 11.5 / 2.0 | 5.3 / 1.0 | 64.9 / 18.3 | 42.7 / 7.8 | 26.3 / 3.4 | 19.4 / 2.4 |
| knj,sky0 | 0.53 / 0.18 | 10.1 / 2.46 | 32.8 / 10.1 | 25.4 / 5.4 | 11.1 / 1.8 | 4.0 / 0.7 | 64.4 / 14.0 | 33.3 / 4.8 | 24.6 / 3.1 | 19.2 / 2.4 |
| nofilter (a-trous off) | 0.29 / 0.19 | 15.9 / 3.49 | 36.1 / 9.4 | 27.0 / 5.0 | 14.7 / 2.3 | 6.2 / 1.1 | 61.9 / 16.9 | 48.9 / 7.0 | 39.2 / 4.8 | 23.6 / 3.1 |
| anyhist (history test keeps any material and normal) | 0.32 / 0.19 | 7.7 / 1.84 | 37.7 / 11.6 | 28.9 / 6.5 | 16.4 / 3.1 | 6.8 / 1.1 | 58.1 / 15.6 | 52.0 / 7.9 | 47.3 / 6.2 | 24.7 / 3.2 |

The hold columns past +60 carry the queue's landings where the queue runs (a step when a
chunk's record lands in view: `pin`'s 63 % at +60 to 13 % at +120), so the `nqueue` rows are
the cleaner read of the screen-space part; the `drain-30000` row (the queue closest to
drained) is the cleaner read of what remains with few landings.

**Where the far pixels that still differ from the final frame come from** (`drain-30000`,
the diagnostic at the sampled frames, far class only; "no value" is a face with no baked
record whose sky ray is cast):

- stop +0: 31 % of the far pixels over 4; of them 69 % `no value` (the sky ray cast, the
  pair converging) and 31 % `record`; by +60 the `no value` share is 4 % (records landed) and
  the rest are `record` pixels of age under 256 (57 %) or at the cap (32 %).
- hold +0 (after the turn): 34 % of the far pixels over 4, all `record`, no ray cast; ages
  under 64 for 85 % of them (the restart at the turn: 90 % of all far pixels are under 64 at
  +0). By +120, 76 % of the still-differing pixels are under 256 (the key mean still
  converging), 23 % at or over 256.

**Attribution:**

1. **Not the geometry or upscaler path.** `albedo` is a tenth of the fly change and has no
   drift; `off1` and `own` keep the drift (and `own` adds its own still flicker, 2.8 %).
2. **Not the exposure, not the a-trous, not the half-resolution lighting by itself**
   (`pin`, `nofilter` within the run-to-run noise of the default).
3. **The key light's per-pixel visibility mean is the largest single term.** `keyoff` takes
   the fly's far change from 14.2 to 8.4 % and the stop's drift mean |dY| from 9.7 to 5.9
   levels; with the sky rays off too (`keyoff,sky0`) the fly is near the albedo's (2.4 %)
   and the remaining drift is on the canopies under auto exposure (the diagnostic arm with the
   exposure pinned and the queue frozen is queued). `knj` (no disc jitter: a hard shadow)
   does not remove it: the mean restarts whatever the sample, and under FSR 2's jitter a
   far thin feature's hit moves between lit and shadowed voxels frame to frame, so even the
   centre ray's sample varies per pixel.
4. **The restart is the mechanism, the cap only its length.** `kh=16` makes the hold's
   drift shorter (14 % at +120 against 45 %) and the still noisier (0.38 against 0.33 % over
   4; the mean's variance at cap 16 is 16 times cap 256's). A short cap trades drift for
   grain; the owner rejected grain at 94a.
5. **The history test's material and normal clauses are half of the motion flicker**
   (`anyhist`: fly 7.7 against 14.2 %) and none of the drift: a far thin feature fails the
   position test too, and a restart from one sample follows either way.
6. **The queue's landings are the second term at a stop** (two thirds of the stop's
   differing far pixels at +0 were faces waiting for a record, in a scene whose queue never
   drains): ADR-0076 rule 6's case, Plan 0094's lane.

### The world-hashed key sample, first read (RTX, loaded)

`tag=hashfar` (the key disc point hashed from the hit's voxel and face for hits past 150
voxels, `sky_light.wgsl` `KEY_WORLD_HASH` / `KEY_FAR_T`, snapshot `live/wobble/bin/hash-far`),
the queue at 2000 undrained frames like the default: fly far 14.3 % / 3.15 (default 14.2 /
3.11), stop +0 35.2 % / 10.3, hold +0 65 % / 18.4, +120 27 % / 3.4 (default 45 % / 5.5, `pin,nqueue`
15 % / 2.5). **No gain in this read**, but in this run most of the far pixels still differing
were `no value` faces whose sky ray is cast (the queue at 2000), not key-light pixels, so the
apples-to-apples read is `tag=hashfar,pin,nqueue` against `pin,nqueue`, which was running at the
checkpoint. The likely reason the hash alone cannot settle a far pixel: under FSR 2's jitter the
hit moves between a leaf's voxels frame to frame, each with its own hashed point, and the key
mean restarts from whichever sample came first.

### Batch 5: the pending arms (2026-11-20, RTX 3080 Laptop, exclusive perf lock, load 2.3 to 5: loaded, indicative)

The same columns as the bisect's table (far class; "stop" and "hold" against the hold's final
frame at +0, +60, +120, +240). The queue undrained at 2000 on every arm.

| arm | still far pair over 4 / dY | fly far pair over 4 / dY | stop +0 | +60 | +120 | +240 | hold +0 | +60 | +120 | +240 |
|---|---|---|---|---|---|---|---|---|---|---|
| pin,nqueue (the control, from batch 4) | 0.35 / 0.19 | 14.3 / 3.13 | 36.8 / 9.7 | 22.5 / 4.1 | 12.6 / 2.2 | 6.1 / 1.3 | 52.4 / 13.0 | 29.3 / 4.5 | 15.0 / 2.5 | 7.6 / 1.6 |
| tag=hashall (the world-hashed disc point at every distance) | 0.35 / 0.19 | 14.3 / 3.14 | 35.0 / 10.3 | 25.5 / 5.2 | 12.9 / 2.3 | 6.0 / 1.1 | 58.9 / 16.6 | 51.6 / 7.2 | 44.5 / 5.5 | 22.2 / 3.1 |
| tag=hashall,pin,nqueue | 0.36 / 0.20 | 14.3 / 3.15 | 36.8 / 10.3 | 22.8 / 4.5 | 13.2 / 2.5 | 6.6 / 1.3 | 52.5 / 13.1 | 29.3 / 4.5 | 15.0 / 2.5 | 7.6 / 1.6 |
| tag=hashfar,pin,nqueue | 0.36 / 0.20 | 14.3 / 3.15 | 36.8 / 10.3 | 22.8 / 4.5 | 13.2 / 2.5 | 6.6 / 1.3 | 52.5 / 13.1 | 29.3 / 4.5 | 15.0 / 2.5 | 7.6 / 1.6 |
| keyoff,sky0,nqueue,pin (no per-pixel light sample, queue frozen, exposure pinned) | 0.06 / 0.01 | 1.11 / 0.41 | 0.70 / 0.30 | 0.44 / 0.07 | 0.20 / 0.02 | 0.14 / 0.02 | 1.76 / 0.93 | 1.11 / 0.18 | 0.37 / 0.05 | 0.44 / 0.06 |
| sky0 (no sky ray, key on) | 0.37 / 0.14 | 10.7 / 2.56 | 32.6 / 10.0 | 18.7 / 3.5 | 9.9 / 1.5 | 2.9 / 0.5 | 61.3 / 12.3 | 49.8 / 6.6 | 46.1 / 5.6 | 23.7 / 3.2 |
| full (lighting at full resolution) | 0.15 / 0.14 | 14.7 / 3.25 | 31.5 / 9.0 | 24.0 / 5.0 | 12.5 / 2.1 | 3.8 / 0.8 | 55.7 / 15.2 | 50.9 / 7.2 | 45.7 / 5.7 | 12.5 / 1.8 |
| farn0 (far normal off) | 0.31 / 0.17 | 14.8 / 3.77 | 34.3 / 10.2 | 25.9 / 5.0 | 11.7 / 2.1 | 5.0 / 0.9 | 57.5 / 9.8 | 49.2 / 6.0 | 46.4 / 5.6 | 15.3 / 2.1 |
| brick (leaf shadow brick: the canopy casts whole bricks) | 0.17 / 0.09 | 6.6 / 1.56 | 21.6 / 5.1 | 12.5 / 2.3 | 8.6 / 1.5 | 5.2 / 1.0 | 29.5 / 5.1 | 26.1 / 3.5 | 18.9 / 2.6 | 2.3 / 0.5 |
| s0 (sharpen 0) | 0.29 / 0.17 | 13.4 / 2.96 | 35.5 / 9.7 | 23.5 / 4.2 | 12.2 / 2.0 | 5.1 / 0.9 | 59.4 / 11.6 | 49.8 / 6.6 | 46.1 / 5.6 | 23.3 / 3.1 |

**What batch 5 settles:**

1. **Option B, the world-hashed disc point, is rejected by measurement.** `tag=hashall,pin,nqueue`
   and `tag=hashfar,pin,nqueue` are identical to the control `pin,nqueue` to the first decimal
   in every column (hold +120: 15.0 against 15.0 %); `tag=hashall` alone is the default within
   noise (44.5 against 44.7 at +120). A fixed disc point per voxel and face removes the sample's
   variance at a fixed hit, and the mean still restarts from whichever voxel's sample the
   jittered hit lands on first. The mechanism is the restart, not the sample.
2. **The attribution closes.** With no per-pixel light sample, the queue frozen and the
   exposure pinned (`keyoff,sky0,nqueue,pin`) the far class is at the lighting-off floor
   everywhere: fly 1.11 % (albedo 1.34), stop +0 0.70 % (albedo 0.56), the turn's hold +0
   1.76 % (albedo 1.08) and 0.37 % at +120. The whole wobble is the per-pixel means (key and
   sky), the queue's landings and the exposure's adaptation; nothing else is left.
3. **The sky ray's share:** `sky0` takes the fly from 14.2 to 10.7 % and does nothing for the
   drift; `keyoff` took it to 8.4. The key mean is the larger term on the fly, both terms share
   the drift.
4. **A coarser caster steadies the image:** `brick` (the canopy's shadow as whole bricks)
   halves the fly change (6.6 %) and the drift (29 % at the turn's +0 against 59, 19 % at +120
   against 45). A shadow with fewer sub-voxel edges gives the restarted mean less to converge
   over. It points the same way as a visibility baked per brick: the far does not need the
   leaf-level shadow.
5. `full`, `farn0` and `s0` are the default within noise on the fly and the drift (`full`'s
   still is quieter, 0.15 %; `farn0`'s fly mean |dY| is higher, 3.77).

Batch 6 (the RADV RENOIR, `live/wobble/radeon-*.log`: `hold=120,drain=600` on the base
snapshot and `tag=hashfar,hold=120,drain=600`) was running at the hand-off, for the pass times
of the current pipeline on the slow adapter; not read here. Plan 0095 Phase 2 reads the light
pass on both adapters with its own runs.

## The runners

The three scripts under the untracked `live/wobble/`, kept here so the dev can recreate them
(Plan 0095, "What the dev reuses"). `snap.sh <name>` builds the release test binary and copies
it to `live/wobble/bin/<name>`, so a later source edit cannot change a queued arm:

    cargo test --release -q -p flythrough --test gpu_wobble --no-run --message-format=json

under `CARGO_BUILD_JOBS=12` and the 16G scope, the executable path taken from the JSON lines
whose `executable` contains `gpu_wobble`, copied to `live/wobble/bin/$1`. `run-arms.sh
<arms...>` runs the rig once per arm, serially, each under the perf lock and the memory cap:

    VXE_TEST_ADAPTER=${ADAPTER:-rtx} VXE_WOBBLE="$arm" flock /home/igor/Work/voxel-engine/.git/vxe-perf.lock \
      systemd-run --user --scope --quiet -p MemoryMax=16G -p MemorySwapMax=2G -- \
      "$BIN" --ignored --nocapture wobble > "live/wobble/$adapter-$name.log" 2>&1

with `name` the arm's tokens with `/` and `=` replaced by `-`. `table.sh <logs...>` prints one
markdown row per log: the still and fly far figures (`grep "still (classes"`, `"fly (classes"`,
the text after `far`), the stop's and hold's to-final far figures at frames 0, 60, 120 and 240
(the indented `N:` lines after `stop: far pair` and `hold: far pair`, the text after
`to-final far` up to `near`), and the `drained ... after N` word.

### Hand-off (2026-11-20 10:10): Plan 0095 and ADR-0079 take this over

The spike ends here. The design is ADR-0079 (the key visibility baked per brick direction over
the disc, read with no screen history at the far normal's band, the near restart seeded from
it, a stepped key-only re-bake when the sun turns) and Plan 0095 builds it; both cite this log.
The owner answered the two questions below the same day: (i) the coarse far shadow per brick
face direction is accepted for far surfaces; (ii) the far gate follows the far normal's band.
The branch stays as the rig's and the readings' home until Plan 0095 closes; its diagnostic
shader code (`KEY_WORLD_HASH`) is not reused.

### Resume notes (2026-11-20 09:45, superseded by the hand-off above; kept as the record of the spike's state)

**The cause so far, with numbers (RTX, the forest rig, `live/wobble/rtx-*.log`):**
- The wobble is lighting history, not geometry: `albedo` fly 1.3 % against 14.2 % shaded; no
  drift (0.6 % against 35 % at the stop). FSR 2, jitter, far normal, a-trous, half-res,
  exposure: none of them.
- Two terms: (a) the per-pixel running means (the key light's `key_mean`, cap 256; the sky
  pair, cap 64) restart from one binary sample at every history rejection (a turn restarts
  90 % of far pixels; a slow fly restarts far thin features continuously) and converge as
  1/n for seconds: a sub-threshold monotonic drift (per pair under 1 % over 4, yet 45 % of
  far pixels still over 4 levels from their settled value 2 s after a turn). (b) The live
  queue's landings while the camera rests (two thirds of the stop's differing far pixels were
  `no value` faces; the forest's queue never drains, 30 000 frames read).
- `keyoff` halves the fly change and the drift; `keyoff,sky0` leaves the fly at 2.4 %.
  `anyhist` halves the fly change (7.7 %), no effect on the drift. `kh=16` shortens the drift
  at the price of grain. `knj` (no disc jitter) does not help.

**Arms tried:** default, albedo, pin, nqueue, pin+nqueue, drain=30000, off1, own, keyoff,
keyoff+sky0, kh=16, kh=16+sh=16, knj, knj+sky0, nofilter, anyhist, tag=hashfar (table above).
Running at the checkpoint, detached (`live/wobble/batch5.sh`, PID 413986, then `batch6.sh`
on the Radeon for cost): `tag=hashfar,pin,nqueue`, `tag=hashall` (hash at every distance),
`tag=hashall,pin,nqueue`, then on the `base` snapshot `keyoff,sky0,nqueue,pin` (the exposure
question on the canopies), `sky0`, `full`, `farn0`, `brick`, `s0`; then on the Radeon
(`radeon-*.log`) `hold=120,drain=600` and `tag=hashfar,hold=120,drain=600` for the pass times.
Read them with `live/wobble/table.sh live/wobble/rtx-*.log` and `grep diag`.

**The rig:** `apps/flythrough/tests/gpu_wobble.rs` (tokens in `Arm::parse`); snapshots are
built by `live/wobble/snap.sh <name>` and run by `BIN=live/wobble/bin/<name>
live/wobble/run-arms.sh <arms...>` (the runner takes the perf lock per arm). The shader
experiment is in `sky_light.wgsl` (`KEY_WORLD_HASH`, off at the tip) and `shade.wgsl` (`key_t`).
The pass timer's per-frame ms under the readbacks are inflated; the still segment's are the
nearest to the fly-through's.

**What is next:**
1. Read the pending arms. If `tag=hashall,pin,nqueue` beats `pin,nqueue` the hash is a
   candidate for near too; if not, the candidate fix is **a key visibility in the world, not on
   the screen**: the design the owner asked for.
2. Draft the plan (next free 0095) and an ADR: **the key light's visibility is baked per
   brick face direction like the sky's (ADR-0070), over the disc (N rays at bake time, a smooth
   penumbra with no temporal variance), re-baked when the sun steps by more than the disc
   (the day cycle's "changing parts"); far pixels (the far normal's weight over 1/2, a voxel
   under about 4 px) read it with no screen history; near pixels keep the per-pixel ray but
   their restarted mean is seeded from the baked value at `SEED_AGE` (as the sky pair is from
   its record), so a restart is a small correction, not a 1/n climb from one sample.** The
   rejected alternatives for the ADR: a shorter cap (grain, `kh=16`), the world-hashed per-face
   sample (read above), any-history acceptance (half the flicker, none of the drift), a wider
   young pool (POOL_AGE 32 ends long before 256). Cost (a guess to measure): the bake's rays
   +6 directions x 16 samples per brick over the sky's 6 x 256; one more byte per brick
   direction in the store; zero per frame. The sky pair's `no value` wobble before a landing
   is Plan 0094's rule 6 work (its coarse prior), not this plan's; the plan states the seam.
3. The owner's window (W1): a release binary with the fix behind a setting, the forest at
   15:00 and at `10,180`, a slow fly and a turn, arms selectable by flag with a banner word;
   `live/sessionW1.sh` on the live-runner lib, dry-run with `skip`.
4. Owner questions to batch (none sent yet): (i) is a hard-edged far penumbra acceptable if
   the baked value is per brick face (a 0.8 m cell) rather than per voxel; (ii) the far gate:
   by the far normal's band (the default `[2, 6]` px a voxel) or a distance in metres.
