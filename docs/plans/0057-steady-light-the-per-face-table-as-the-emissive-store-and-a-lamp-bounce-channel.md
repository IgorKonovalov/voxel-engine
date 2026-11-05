# 0057: Steady light: the per-face table as the only emissive store, faces sampled and guarded in world space, and the probes as a lamp-bounce channel

> **Status:** in-progress (Phases 1 to 6 done on the lane, 2026-11-05; amended 2026-11-05 (owner) with Phase 7, a fix pass for the grain, the young cells and the table, and Phase 8, a second window; Phase 7 not started); Plan 0056's Phase 2 readings are in its log
> **Created:** 2026-11-04
> **Milestone:** M2 in [`docs/roadmap.md`](../roadmap.md) (the "one-sample path tracing ...
> temporal accumulation" bullet and the probe volume's indirect light)
> **Related ADRs:** [ADR-0043](../adrs/0043-emissive-light-from-a-world-space-per-face-table-and-a-lamp-bounce-probe-channel.md)
> (proposed; this plan builds it and its close accepts it), which amends
> [ADR-0018](../adrs/0018-direct-light-per-pixel-one-sample-from-a-hit-buffer-with-svgf-history-killed-by-edits.md),
> [ADR-0023](../adrs/0023-indirect-light-from-a-scrolling-probe-volume.md) and
> [ADR-0036](../adrs/0036-static-emissive-light-on-a-long-full-precision-history-guarded-by-an-emissive-gradient.md)
> and makes [ADR-0037](../adrs/0037-probe-rays-direct-light-keeps-its-energy-apart-from-the-screen-firefly-clamp.md)
> moot. [ADR-0033](../adrs/0033-eye-adaptation-as-an-asymmetric-two-stage-exposure-on-the-gpu.md)
> is untouched (owner, the verdict's item 5).
> **Source:** [Plan 0051](done/0051-steady-light-spike-converge-and-freeze-and-a-world-space-emissive-cache.md)'s
> Phase 4 verdict (owner, 2026-11-04) and its close review's Followups; the readings in
> [`logs/0051.md`](logs/0051.md); [`LIGHT-FEEDBACK.md`](../../LIGHT-FEEDBACK.md), "What the owner
> wants".
> **Starts after:** Plan 0051's close (v0.41.0, on main). Runs beside
> [Plan 0056](done/0056-aliasing-spike-what-moves-on-the-final-image.md)'s lane, which is read-only
> against this plan and hands it the yellow dots' class (0056 Phase 2, done-when 4); this plan
> takes that answer as an input at its Phase 1 and Phase 4. Then Plans 0044 and 0045; Plan 0048
> is moot under ADR-0043 and awaits the owner's "drop".
>
> **Amended 2026-11-04 (architect, for the owner):** Phase 1's stop rule read **stop** (the
> young-cell share of the fast yaws' excess is 0.005 to 0.099 in every positive row,
> `logs/0057.md` Phase 1), and Plan 0056's class answer is **light, not the upscaler, on creases,
> mostly new each frame** (`logs/0056.md` Phase 2). Phase 4 is rewritten: it **drops the warm
> rays and the fill of every young face in view**, and **builds two things only where a reading
> on Phase 3's tree says they are owed**: a fill pass for the cells an edit or the guard
> reopened (the dug hole), against a settle bar of **60 frames at arm's length and 120 at 100
> voxels**, and the composite reading a full-resolution pixel's emissive from its own face's cell
> (the creases), if the half-resolution upsample is read to carry the dots. Phases 1 to 3, 5 and
> 6 stand, with Phase 6's done-when gaining the creases. ADR-0043's fill-pass paragraph is
> amended in its Outcome at the close, as the plan provided.
>
> **Amended 2026-11-04 (architect, for the owner), Phase 2 done-when 7:** Phase 2's fast yaw read
> the opposite of the bar's premise (`logs/0057.md` Phase 2, done-when 7): with the table's read
> off the screen path reads no excess (-911.7 over a floor of 69.3, sd 11.6), and the table reads
> +552.5 over a floor of 27.6 (sd 2.0). The table's read, not the screen path, puts the dots on
> the screen after a yaw. Done-when 7 becomes a reading whose two excesses feed Phase 4's crease
> rule, and the bar it carried (the table puts no excess over its own floor after a yaw) moves to
> Phase 4 as done-when 7, asserted if the own-face read is built. Phase 2 is not held.
>
> **Amended 2026-11-04 (architect, for the owner), from Phase 2 and 3's missed predictions**
> (`logs/0057.md` at bd627c7, Phase 2 done-when 9 and Phase 3 done-when 5). Three decisions:
> (1) the patch's misses at 4 % load are the hash's or the claim's, not the size's: the size
> stays 2^20 and dev reads the slot distribution on the CPU, fixes what it names, and the bar is
> 0 misses; (2) the still hall that moves (native drift 0.257, one frame 0.137, max 175 levels,
> against Plan 0051's 0.004 and 0.001) is a defect against the owner's first rule, attributed by
> class in Phase 4's Part 1 and fixed in a new Part 1b before Parts 2 and 3, with the native
> figures as a bar; (3) the still reopens (4 in 1024 frames at the patch) are a defect, fixed in
> Part 1b by letting the guard reopen only while the world has changed, and the gradient's cost
> (0.092 ms) is a reading against a prediction that compared with a pass Phase 2 did not run: the
> prediction is restated, the cost accepted under `docs/nfr.md` section 2's 6.0 ms line.
>
> **Amended 2026-11-04 (owner), the camera-radius preload.** After Plan 0056's window under
> `--steady b --indirect off` the owner said the light no longer flickers once it settles, but
> "places like this [a dark ceiling corner at the vein] takes some time to settle", "real players
> won't be that slow", "still quite a bit of time before light 'loaded' in dark corners and after
> fast turn", "you can clearly see this light 'loading' when digging holes", and asked to "explore
> idea of preloading light around the camera in certain radius, no matter where this camera
> looks". Phase 4 gains **Part 2b, the camera-radius preload**, built after Part 2 on its pass:
> rays from the eye over the full sphere, out to `R` = 64 voxels, sample the faces they hit
> until each cell holds `C_PRE` samples, at a fixed ray count and sample budget a frame, so the
> faces a turn, a step or a dig at arm's length shows are settled before they are seen. This
> **reinstates the dropped warm rays for the settle, not for the dots**: the dots were read on
> mature cells (Phase 1's young share 0.005 to 0.099; Plan 0056's creases), so warming faces
> could not touch them, and that is why the rays were dropped; the settle is the opposite case,
> cells too young or absent when they come into view (the hole's `S90` 319 and 1891 frames on
> Phase 3's tree against bars of 60 and 120), which warming faces is for. Done-whens 12 to 17,
> Phase 6's run 6 and item (h), the Budgets, Risks, Followups and the Decision follow.
>
> **Amended 2026-11-04 (architect, for the owner), the preload's form.** Part 2b's
> `preload_visibility_readings` read the share of entering pixels within `R` that an eye of the
> previous 60 frames could see at **0.002 and 0.007 on the walks** (1.000 on the yaws,
> `logs/0057.md` Part 2b): walking, a face enters view from behind something, where no eye ray
> reaches. The owner asked for the light around the camera "no matter where this camera looks",
> so the eye rays are not built and **the brick sweep the first amendment rejected is the form**:
> a cursor visits the mirror's bricks within `R` a few a frame, nearest first, finds each lit face
> from the opaque masks without a ray, and lists it for Part 2's fill routine until it holds
> `C_PRE`. The rejection's cost premise fails on the census (about 15 000 to 25 000 faces within
> 64 voxels at the level, from Part 1b's key counts). `R` stays 64 voxels, a brick radius of 8;
> the faces beyond it settle at their footprint. Done-whens 12 to 17, Phase 6's run 6 and item
> (h), the Budgets, Risks, Followups and the Decision follow.
>
> **Amended 2026-11-05 (architect, for the owner), the claim rule, from the level readings**
> (`logs/0057.md` at 4ec11a7, Part 2b's level readings on the restructured tree). The boxes meet
> done-whens 13, 15 and 16, the walk's cost is under its bar, and the level misses done-whens 14
> and 16 with the preload on about as much as off, because **17 563 of the 18 619 faces probed
> at the patch refused a cell**: one sample is a resampled pick over 8 candidates by their
> unshadowed weight plus one shadow ray, so a face lit through a gap, whose best candidates are
> mostly occluded, draws a lit sample with a small probability `p` and 8 of them find it with
> `1 - (1 - p)^8` (about 0.22 at `p` 1/32). The turn's faces in view hold 1024-sample references
> and still miss, so most of the refusals are dim faces, not unlit ones: the owner's dark corner.
> **Decision: every face within `r0` of the eye claims a cell whatever its probes read**
> (`preload_claim_radius`, 24 voxels by default, swept 16, 24 and 32), and is filled to `C_PRE`
> lit or not; beyond `r0` to `R` the 8-probe rule stands; the load and miss guards hold
> everywhere. Rejected: more probe rounds per epoch (option (b)), since a face at `p` 1/32 needs
> about 96 probes to be found nineteen times in twenty, over a third of `C_PRE`, and a face no emitter
> reaches pays them again every epoch, which streaming raises while walking, where a claimed cell
> is filled once for its life; a deterministic centre-to-centre ray per candidate emitter,
> since the candidate table lists chunks of emitter samples, so the test is a loop over every
> emitter within the light radius per face, and a centre ray misses a face lit off its centre
> through a gap; and options (a) and (c), which keep a rule whose miss is attributed. **The
> reference run** claimed through the same rule and kept cells of 1024 samples, so 3 660 of the
> patch walk's 6 349 entering pixels had none: it now claims every face unconditionally, holds
> until every cell reaches `C_CAP`, and a pixel with no reference counts as not settled. Part
> 2b's claim, done-whens 12, 14, 16 and 17, the Budgets, Risks and Phase 6 (h) follow.
>
> **Amended 2026-11-05 (architect, for the owner), the settle bars, from the dig and reference
> readings** (`logs/0057.md` at 693cc90, Part 2b's level on r0 32). Two readings say the level's
> bars measured noise, not settle: the 5-voxel dig's `S90` reads 271 (preload on, 303 off)
> while its cells hold p50 488 samples at frame 16 and 1048 at 60, and two independent
> 4096-sample references agree within 10 % on only 0.08 to 0.66 of the lit cells within `r0`
> by decile. A cell's mean is off the truth by about `sigma / sqrt(n)` of itself, `sigma` one
> sample's relative spread, so a face lit on one sample in 32 (`sigma` about 5.6) needs about
> 8 500 samples to sit within 10 % of the truth nine times in ten (`(1.645 x 5.6 / 0.1)^2`;
> about 2 800 against a 4096-sample reference that holds them), and no frame budget reaches
> 10 % of the truth on the dim deciles. What the owner reported in Plan 0056's window ("once light settles
> it is not flickering anymore") is a value that stops moving, and a running mean's step per
> frame falls as `sigma sqrt(k) / n` (`k` samples a frame). **Decision: the level's settle bars
> become stop-moving bars in display levels.** A pixel is **still** at a frame when its
> emissive part, through the display transform at the run's fixed exposure (`rigs.rs`'s
> `tone_bytes` arithmetic unrounded: ACES, the 2.2 encode, in 8-bit levels), moves by under
> `STILL_LEVEL` = **1 level** (1/255 of full scale) since the frame before; **`Q90`** is the
> last frame at which under 90 % of the counted pixels are still. The derivation is in
> Phase 4's settle bar: the display's encode cancels the dim faces' spread (`D sigma` is within
> 1.2 times from a lit wall to a face lit on 1 in 32), so a cell is still at about
> `n > 135 sqrt(k)` samples whatever its decile, and the hole's fill rate puts `Q90` near 16 to
> 32 frames at arm's length and 47 to 60 at 100 voxels: **the 60 and 120 bars stand, on
> `Q90`** (done-when 15's level half). **Done-whens 14 and 16's level rows** keep their 90 %
> and 99 % on the same criterion (the face's cell through the display transform, frame to
> frame); their distance to the reference becomes a reading per decile beside the two
> references' agreement, its floor. `S90` against the late mean is kept beside every `Q90` as a
> reading, with the **travel after the stop** (levels a pixel moves from `Q90` to `C_CAP`), so
> the slow crawl that a still-moving-under-a-level cell keeps is on the owner's table. **The
> sum of squares stays a Followup** (one or two words a cell, 4 to 8 MB at 2^20): the stop bar
> reads from the rig's readback with no store, and the spread stop is a mechanism the readings
> have not yet asked for. Rejected: option (c) of question 1 (a burst or a larger `C_PRE` for
> the dig), since the derivation says the dig's cells are still by about frame 32 without it;
> a 16 384-sample reference (question 2's (a)), which halves the reference's noise and leaves
> the 10 % bar out of the frame's reach on the dim deciles; and keeping the bars as misses
> (option (a) of both), which would hand the owner a miss the eye cannot see. The box tests'
> asserted halves (done-whens 2, 13, 15 and 16) stand as written: they pass on bright faces.
> Phase 4's settle bar, done-whens 14 to 16, Phase 6 (c) and (h), the Budgets, Risks and
> Followups follow.
>
> **Amended 2026-11-05 (owner), a fix pass: Phase 7, then a second window as Phase 8.** The
> owner's Phase 6 window (`logs/0057.md`, Phase 6) met the stability goal ("there is no flicker
> nether when i turn or go forward/backward") and failed the look on three counts, which the
> owner chose to fix in this plan ("Both: fix now, design in parallel") while a separate
> architect session designs a baked per-chunk light store. **The three faults and the three
> decisions:** (1) **grain at voxel scale**, one value a face with no spatial filter since the
> a-trous left the emissive part in Phase 2: **a world-space blend over each seen cell's
> coplanar neighbour faces, weighted by their sample counts, computed once a cell a frame into a
> word the read takes** (Phase 7 Part B); (2) **young cells show dark**: **the same blend, so a
> young cell is outweighed by its wall's samples, widened from 3 x 3 to 5 x 5 faces where the
> pool is thin** (Part C); (3) **the table too small and its cap off**: in the level the view
> alone loaded the table to 0.257, over `PRE_LOAD_MAX` 0.25, so the preload refused every claim
> (`refused load 364`, the patch's HUD), and the hall read 993 304 of 2^20 cells with 194 079
> missed lighting pixels, about half the view at the window's 843 x 459 lighting grid: **2^22
> cells by default, set by a reading at the window's resolution, an age-out of cells unseen
> beyond `R`, and every load threshold a share of the size** (Part A). The headless occupancy
> readings ran at 192 x 108, a lighting grid about 19 times smaller than the window's, and
> understated the view's load by that much. Phase 7 is built **strip first**: the vein strip
> on the current tree is the before picture, before any mechanism. Phase 6 stands as recorded;
> Phase 8 restates its items. **What the parallel design would also solve:** a store filled at
> chunk load for static emitters lights every face before it can be seen at any range (fault
> 2, beyond `R` too, where the preload cannot reach), and a per-chunk store sized with
> residency has no hash load, misses or cap (fault 3, for static light). **What it would not
> solve:** the grain (fault 1) is the variance of a per-face estimate of `n` samples,
> `sigma / sqrt(n)` of itself, whichever store holds it; a baked face shows the same grain
> unless it is read through a filter, and Part B's blend is that filter, written to read from
> either store. So Part B carries over to the baked design, Part A is the cheapest form that
> lets the window be judged now and may be moot after it, and Part C is the bridge until a
> baked store fills faces beyond `R`. Phase 7, Phase 8, the Decision, the Budgets, Risks,
> Followups, Data shapes and the Status line follow.

## TL;DR

Plan 0051's spike ended with the owner's verdict: emissive light from the world-space per-face
table, read nearest, no indirect gather, exposure unchanged, the probes shrunk to a lamp-bounce
channel whose logic is redone. ADR-0043 records it. This plan makes it the renderer. **The table
becomes the only emissive store** (no screen history, read from the first sample, the a-trous for
the lamp alone), sized and swept so it holds the surfaces around the camera. **A reopened face
is sampled, not only its pixels** (amended): where Phase 3's tree still misses the dug hole's
settle bar, a fill pass draws a fixed number of samples a frame for each cell an edit or the
guard reopened, and where the creases' dots are read to come through the half-resolution
upsample, the composite reads each pixel's own face. **The faces around the camera are
preloaded** (amended 2026-11-04, owner; its form amended the same day): a cursor sweeps the
mirror's bricks within 64 voxels of the eye and fills every face within 24 voxels and every
lit face beyond (amended 2026-11-05), whatever the view
and whether or not any eye could see it, so a fast turn, a walk round a pillar, a dark corner and
a dig at arm's length show light that is already settled. **Invalidation is an edit box
plus a world-space guard:** each cell keeps the emitter that lit it most, and the gradient pass
re-shades that point from the face and reopens the cell where the light changed. **The probe
volume becomes a lamp-bounce channel:** lamp-only rays within the lamp's reach, a running mean
that freezes while the lamp is still, nothing at all with the lamp off, and the scaffolding kept
for a later per-source indirect light. Phase 1 reads the two questions the verdict left open
(`--steady ab --indirect off`, and the dug hole) and whether the young faces are the fast-turn
fault, before any mechanism. The first thing anyone sees is the hall holding still from frame 1
of its first turn, and a dug hole lit within a second. The owner judges the default in the
window at the end, this time with digs. **Amended 2026-11-05 (owner):** the window held still
but showed grain at voxel scale and dark young faces, and the table overflowed at the window's
resolution; Phase 7 fixes the three (a count-weighted blend over each face's coplanar
neighbours, read through by a young face too, and a 2^22 table with an age-out), the vein strip
first, and Phase 8 is the owner's second window.

## Context & problem

`project-context.md` (Rendering, P0): dark scenes, emissive voxels as the light, stable at arm's
length, denoising that survives disocclusion from digging. The owner's rules
(`LIGHT-FEEDBACK.md`): nothing changes while nothing moves; no bright dots in motion; the light
settles while the owner counts to five; the cost matches what is on screen.

**What the spike settled** (Plan 0051, `logs/0051.md`; RTX 3080 Laptop, unasserted): at the
showcase's hall, native, B with indirect off drifts 0.004 at 4096 against 8192 frames against
today's 2.193; at the level the composite's `sigma_view` reads 0.0009 against 0.0413; the frame at
the spawn reads 3.255 ms against 4.944 (RTX) and 21.053 against 35.438 (Cezanne), indicative.
The owner's eye agreed: "there is no radius with better lighting, there is no colored
flickering".

**What it left open, which this plan carries** (the verdict's items and the close's Followups):

1. **The fast turn.** Under B with indirect off the owner saw "problem of fast turn is back",
   and `--steady ab --indirect off` (A's emissive settings over B) was never run. The spike's
   arithmetic: a cell read only after 16 samples, fed by pixels alone, converges at its screen
   footprint, one frame at arm's length (54 lighting pixels a face) and about 48 frames at 88
   voxels (0.34 a face). The young share in the rig's flight read 0.24 at `C_MIN` 16 and 0.14 at
   8. Whether those young faces are the owner's fast-turn fault, or the yellow dots Plan 0056
   hunts, is unread.
2. **The dug hole.** "When i dig it takes some time for light to settle in hole." The box reset
   clears the hole's cells and they refill at the footprint rate; a dig at range is slow by the
   same arithmetic. Beyond the box nothing is invalidated: a wall lit by an emitter the dig
   exposed keeps its capped cells.
3. **The settle bar.** No arm held still within 300 frames at the hall (the least mean at 512
   against its own 8192 was 0.977). The plan states a bar in frames and the mechanism that meets
   it.
4. **The edit-box reset is a spike's:** one dispatch dimension with no cap at 65 535 groups, and
   an overflowed edit frame leaves the table alone.
5. **The lamp channel's reach:** `probes::lamp_reach` at `intensity / d^2 < 1 / 256` is 554 voxels, past
   the volume's far corner (443), so a moving lamp held every probe open.
6. **Readings owed on the arm:** the full-table share, the table's size from occupancy (0.0142
   of 2^22 at most), the arm in flight on both adapters, quiet rounds at rest.
7. **FSR 2 moves the verdict's light:** 0.525 under FSR 2 against 0.004 native. That is Plan
   0056's class to read; this plan asserts at scale 1 and reads at 1.5.

**The `nfr.md` figures touched:** section 2's lighting line (6.0 ms: light, gradient, probes,
gather), denoise line (3.0 ms: temporal, a-trous) and section 3's `Frame::frame_bytes`. The
readings go to `docs/nfr/02-frame-budget.md` and `docs/nfr/03-memory.md`.

## Decision

Build ADR-0043 in six phases, the first a reading. **The table** (Phase 2) carries its key, holds
2^20 cells by default, reads from the first sample, is swept beyond the light radius when the
camera's chunk changes, and replaces the screen's emissive history, reservoirs and guard; the
default flips and the spike's presets go in the same phase. **The guard** (Phase 3) stores the
kept emitter per cell and re-shades it from the face at the stratum cadence. **The hole and the
creases** (Phase 4, amended) are read again on Phase 3's tree, then a fill pass for reopened
cells and an own-face emissive read in the composite are built each only where its reading
says so; the warm rays and the fill of every young face in view are dropped, because Phase 1
read the fast turn's excess outside the young cells. **The camera-radius preload** (Phase 4
Part 2b, amended 2026-11-04, owner; its form amended the same day) sweeps the mirror's bricks
within `R` of the eye and feeds their faces to Part 2's fill routine (every face within `r0`, 24
voxels, and beyond it the faces a probe finds lit; amended 2026-11-05), for the settle the owner
sees in dark corners, after a fast turn, walking and in a dig: faces within `R` are filled to
`C_PRE` before any view reaches them, at a fixed budget a frame. **The lamp channel** (Phase 5) redoes the probe update as lamp-only rays
within a read reach over a running mean that freezes, and keeps the volume's scaffolding. **The
window** (Phase 6) is the owner's, with digs.

We rejected building the fill pass before the reading, because the yellow dots survived every
arm of the spike including no probes and native, so no reading yet says the young faces are the
fault; Plan 0056 reads the class. We rejected keeping a `--steady off` path for the owner's
comparison, because keeping both emissive paths doubles every test; the v0.41.0 binary is the
comparison. We rejected a lower `C_MIN` alone as the fix for the far faces, because it changes
what is shown (the mean of one sample) and not how fast a face gains samples; it is Phase 2's
read and Phase 4's fill that answer the owner's two complaints. The alternatives to the direction
itself are ADR-0043's.

For the preload (amended 2026-11-04, owner; its form amended 2026-11-04, architect, for the
owner) we rejected **rays from the eye over the full sphere**, the form first written, because
the visibility reading puts the walk's entering faces out of every recent eye's sight (0.002 and
0.007 of those within `R`), and within `R` the sweep covers every face they would. We rejected
**rays cast outward from the emitters within `R`**, because they find a face with odds in
proportion to the solid angle it fills from an emitter, so the dim faces lit through a gap (the
owner's dark corner) are found last, and the level's emitters are many exposed voxels sharing a
ray budget. We rejected **a fill at chunk upload**, because the light is GPU work in the frame
whichever thread asks for it, `light_radius` is 4 chunks (256 voxels), 64 times the volume of
`R` and about 1.6 million faces at the vein's density (over the table's 2^20), a face's light
depends on chunks uploaded after it and on edits, and an upload fires once per chunk, not when
the camera comes near. The brick sweep was rejected first for its cost (a scan of 4096 bricks a
frame); the census refutes that: about 2145 bricks within 64 voxels, a sweep of 17 frames at 128
a frame, and 15 000 to 25 000 faces. Prewarming at a dig from the edit's box is Part 2 itself;
the sweep visits a dig's bricks first within `R`, and Part 2 covers the one beyond.

For the fix pass (amended 2026-11-05, owner; Phase 7) we rejected **a blend at read time in the
temporal pass**, per lighting pixel, because its lookups scale with the pixels (about 388 000
at the window's lighting grid) times the kernel, every frame, while a face at arm's length
covers about 54 of them, so the per-cell pass does the same arithmetic once a face. We rejected
**the a-trous back on the emissive part**, because a screen filter's footprint moves with the
view, which is the motion the owner called met ("no flicker ... when i turn"). We rejected
**a filter pass over the whole table**, because most of its 2^22 cells are not in view; the
chosen pass is that filter restricted to the cells seen this frame. We rejected **coarser
storage** (a cell per 2 x 2 faces or per brick face), because a coarse cell straddles shadow
edges and creases, so it needs a second per-face occlusion store to stay honest, and it changes
the key the parallel baked design is choosing. For the young faces we rejected **a per-brick
prior** (a brick mixes every orientation it holds, so a dark wall inherits a lit wall's light
across the crease, and it is a second store) and **seeding the fallback's mean from the screen
neighbourhood** (a screen quantity that moves with the view, and the fallback is for faceless
hits). For the table we rejected **2^21 without an age-out** (the hall's window read 993 304
cells with half the view missing, so the demand is over 2^20 and plausibly over 2^21 while
every cell within the light radius stays) and **no load cap with eviction alone** (the miss
guard then trips first at the hall, which stops the preload as surely as the cap did).

## Architecture diagram

```mermaid
flowchart LR
    subgraph vxe-render
        LP[light pass: lamp NEE + one emissive sample per pixel; adds to the face's cell; capped face draws nothing] --> T[(table: key, count, 3 sums Q17.15, kept emitter, stamp; 2^20 cells)]
        FP[fill pass: F samples a frame per reopened cell in view; brick sweep within R lists lit faces, filled to C_PRE] --> T
        G[gradient pass: re-shades each cell's kept emitter from its face; reopens a changed cell] --> T
        E[edit boxes: clear the dilated box's cells; overflow clears the table] --> T
        S[sweep on a chunk change: clear cells beyond the light radius] --> T
        T --> TP[temporal pass: emissive = cell mean x exposure; lamp part blends as today]
        TP --> AT[a-trous: lamp part only] --> C[composite; if read owed: a cacheable pixel's emissive from its own cell]
        T -.-> C
        PU[lamp channel: lamp-only rays, probes in reach, running mean that freezes] --> PG[gather] --> C
    end
    subgraph flythrough tests
        R[settle rig: wander, settle, dots by young or mature cell, K, young share, occupancy, misses]
    end
```

## Implementation phases

Each phase ships as its own commit. `dev` implements all dev phases in one session with no
review between phases. The architect reviews once at the end, in a fresh session. Lane rule 9c
applies: exploratory runs on the RTX 3080 Laptop through `VXE_TEST_ADAPTER=rtx`, labelled; every
asserted reading on lavapipe.

**The gates.** Per phase: `node scripts/test-changed.mjs --profile quick`, then `--profile gpu`
under the perf lock (every phase here touches rendering), clippy and fmt. At the tip:
`cargo nextest run --workspace --profile gate` under the perf lock, doctests, clippy, fmt, deny,
and the doc and tier gates. `soak` and `full` run only when the owner asks. **Tables, sweeps and
attempts go to `docs/plans/logs/0057.md`**, started from the readings-log template, and the
Implementation log below keeps one row per phase.

**Test times.** Every named test states its expected wall time on lavapipe, from Plan 0051's
tests: a hand-built box (`cache_box`) runs 2 to 3 s, the level loads in about 5 s and renders at
about 20 frames a second in the gate's build at 192 x 108. A test over 30 s quiet is born on the
soak list, named in its phase.

**Plan 0056's lane runs beside this one** on the same base. It owns `apps/flythrough/tests/strip.rs`
and `tests/support/aliasing.rs`; this plan does not touch them. It reads the light through the
`--steady` presets until Phase 2 removes them, so Phase 2 lands after 0056's Phase 2 readings are
in its log, or the orchestrator agrees the order; the log names which.

### Phase 1: The open questions, read
- **Owner skill:** dev
- **What:** the verdict's two open questions and the fast turn's attribution, on the spike's
  code, before any mechanism. This phase changes no shader and no default.
- **The readings,** the rig (`apps/flythrough/tests/settle.rs`) at `Manual(pose.exposure)`,
  both poses, lamp on and off, `up1.5` and scale 1, `b --indirect off` beside
  `ab --indirect off`, with today's rows from `logs/0051.md`:
  - **`ab --indirect off`:** the wander table, `K` and the excess dots at the rig's yaws (`yaw 90
    10 f`, `yaw 90 30 f`, `yaw 180 20 f`) and the flight; the hall (`hero --exposure 8 --adapter
    NVIDIA`) at native and `up`, `N` 256, 512, 4096, 8192: drift and frames to settle, as Phase
    3 of Plan 0051 read them. The question it answers: whether A3 and A4 over B change the fast
    turn at all. Under B they reach only pixels on young or absent cells, so a difference, if
    any, is the young share's.
  - **The dots by cell age (the attribution).** `dot_readings` gains a split of the excess
    into pixels whose cell is young (under 16 samples, or none) and pixels whose cell is mature,
    at the rig's three yaws and at **a 90-degree yaw over 118 frames** (Plan 0056's ten display
    pixels a frame at 1080p, 0.76 degrees a frame), both poses, lamp on and off. **The stop
    rule:** if over half of the excess at the fast yaws lies in young-cell pixels where today
    reads an excess, Phase 4 proceeds as written; if under a quarter, dev stops for an architect
    amendment before Phase 4, with Plan 0056's class answer beside the table if it is in its log;
    between, Phase 4 proceeds and the log says so.
  - **The dug hole by distance.** The rig's dig event at `VEIN_POSE` (arm's length) and a new
    event, a dig of the same radius 100 voxels down the passage: per frame after the dig, the
    emissive part's `S90` over the box's pixels and the young share inside the box, `b --indirect
    off`, `C_MIN` 16 and 1. This is the baseline Phase 4's bar is read against.
  - **The owed readings:** the full-table share (slot-probe failures, a new census counter read
    back beside the no-cell share); occupancy after the rig's flight repeated four times from
    each pose (the table is not cleared between), as the input to the table's size; `b
    --indirect off` in flight on both adapters (`flight_cost_readings`); three quiet rounds of
    the cost at rest at the spawn and the vein on both adapters, or the log says the machine
    did not go quiet and gives the load.
- **Files touched:** `apps/flythrough/tests/settle.rs`, `apps/flythrough/tests/support/settle.rs`,
  `apps/flythrough/src/hud.rs` (a test), `docs/plans/logs/0057.md` (new).
- **Done when:**
  1. `ab_with_indirect_off_applies_both` in `hud.rs`'s tests asserts that `--steady ab --indirect
     off` yields a `Lighting` with `emissive_cache` set, `probe_freeze` set, `skip_converged_draw`
     set and `indirect` false (no GPU, under 1 s). The verdict said its parsing was unchecked.
  2. The tables above are in the readings log, with the hall's rows beside Plan 0051's. No
     reading is asserted.
  3. The stop rule's verdict is one line in the log: proceed, stop, or proceed with the share.
  4. `the_dots_split_by_cell_age_sums_to_the_excess` asserts, on `cache_box`, that the young and
     mature shares of the excess sum to the total (under 5 s).

### Phase 2: The table is the only emissive store, and the default
- **Owner skill:** dev
- **What:** ADR-0043's table in its shipping form, the emissive screen path removed, the default
  flipped, the spike's presets gone.
- **The table:**
  - **Cell.** Ten `u32` words: the key (the voxel's three coordinates and the face, packed by
    dev, with a valid bit; the key replaces the spike's check word), the count, three Q17.15
    sums, the kept emitter's voxel and face (Phase 3 fills them; reserved here), its luminance
    (Phase 3), and a frame stamp (Phase 4). 40 B a cell, `2^20` cells: **41 943 040 B**,
    arithmetic. `CACHE_CELLS` is a `Lighting` setting with that default; `C_CAP` stays 4096 (the
    sums' range, Plan 0051's arithmetic).
  - **Read from the first sample.** `C_MIN` is 1 and is no longer a setting. The temporal pass
    writes the cell's mean times `exposure_now()` as the emissive part for every cacheable hit;
    the pixel's own sample reaches the screen only through the cell. The read is nearest; the
    `smooth` read and `CacheRead` go.
  - **The fallback.** A hit with no face (`rec.field`, a translucent hit, `rec.axis < 0`) blends
    its own sample into the accumulation's emissive channels at the lamp part's `ALPHA` 0.2, as
    the lamp's part does, and resets with it. The accumulation pair returns to `rgba16float`.
  - **The light pass** draws its candidates and casts its shadow ray for a pixel whose cell is
    under the cap, and adds the sample to the cell; a capped cell's pixel does neither. No
    reservoir is written or read: `Sampler`, `reuse_m_cap`, the reservoir textures and
    `converged_draw_skip` go. The firefly clamp stays (ADR-0018).
  - **The a-trous** filters the lamp's part and the fallback's emissive; a cacheable pixel's
    emissive passes through unfiltered (its variance is the cell's, not the pixel's).
  - **The sweep.** When the camera's chunk changes, one dispatch over every cell clears the cells
    whose voxel lies beyond `light_radius` chunks of the camera's chunk (`MirrorConfig::light_radius`,
    the light table's own bound), by the key the cell carries.
  - **The full table.** A key that finds neither its cell nor a free slot in four probes counts a
    miss (an atomic counter in the table's header), and its pixel shows its raw sample through
    the fallback. The HUD's lighting row shows `table <cells> <occupied> <misses>`.
  - **The edit-box reset, fixed.** `clear_cells` dispatches in two dimensions (or in slices of at
    most 65 535 groups) so a dilated box of any size clears; a frame with more than `MAX_EDITS`
    boxes clears the whole table, as it flushes the screen history.
- **The default.** `Lighting::default()` has the table on and `indirect` off (Phase 5 retargets
  `indirect` to the lamp channel); the fly-through and `hero` start there. `--steady`,
  `--cache-read`, `SteadyMode`, `Lighting::{emissive_history, skip_converged_draw, emissive_guard,
  reuse_m_cap, sampler}` and their flags (`--emissive-guard`, `--reuse-m-cap`, `--sampler`) go, with
  `DEFAULT_EMISSIVE_HISTORY`, `DEFAULT_REUSE_M_CAP` and `HISTORY_AGE_MAX` where nothing else reads
  them; `probe_freeze`, `skip_inside_probes` and `probe_lamp` stay until Phase 5. The goldens are
  re-blessed once on lavapipe, with their pixel difference against the old baselines in the log.
  Every test that asserted a deleted mechanism is removed or re-pointed and named in the log
  (`reservoir_reuse_lowers_one_frames_error`, `the_emissive_guard_keeps_history_where_nothing_changed`,
  `static_emissive_light_holds_still`, `a_reveal_behind_a_plug_follows_within_seventy_three_frames`
  and `flythrough::emissive_stillness`'s patch bar among them; the reveal returns in Phase 3).
  `a_turn_reads_converged_cells` is replaced by done-when 7.
- **The readings:** the wander table at both poses, lamp on and off, scale 1 and `up1.5`, with the
  spike's `b --indirect off` rows beside; the hall at native and `up`, `N` 256, 512, 4096, 8192
  (the settle bar's baseline before Phase 4); occupancy and misses after four flights; the pass
  costs at rest and in flight on both adapters, quiet rounds where the machine allows;
  `frame_bytes`.
- **Files touched:** `crates/vxe-render/src/shaders/light.wgsl`, `temporal.wgsl`, `atrous.wgsl`,
  `march.wgsl` (the key helpers), `cache_clear.wgsl`, `shaders/cache_sweep.wgsl` (new),
  `crates/vxe-render/src/marcher.rs`, `lib.rs`, `apps/flythrough/src/main.rs`, `hud.rs`,
  `apps/flythrough/examples/hero.rs`, `apps/flythrough/tests/settle.rs`, `support/settle.rs`,
  `apps/flythrough/tests/lights.rs`, `emissive_stillness.rs`, `upscale.rs` (`frame_bytes`), the
  golden baselines, `.config/nextest.toml`, `docs/plans/logs/0057.md`.
- **Done when:**
  1. `a_face_reads_its_mean_from_the_first_sample` asserts, on `cache_box` at scale 1 with no
     jitter, lamp off, that on the first frame a face is seen every pixel on it carries the same
     emissive value, equal to its cell's sums over its count times the exposure within one
     `f16` step, and that the value is bit-identical across the next 64 frames once the cell is
     capped (`C_CAP` set to 64 for the test). About 70 frames: about 4 s.
  2. `a_hit_with_no_face_takes_the_fallback` asserts, on a box with a translucent slab, that a
     translucent hit's emissive part blends at `ALPHA` (its value after 8 frames equals the
     closed-form blend of its samples within 1 %) and writes no cell. Under 5 s.
  3. `the_sweep_clears_cells_beyond_the_light_radius` asserts, on `cache_box` with
     `light_radius` 1, that after the camera moves one chunk the cells of faces beyond the
     radius read count 0 and the cells within it keep their counts. Under 5 s.
  4. `a_full_table_counts_its_misses` asserts, at `CACHE_CELLS` 64, that a view of more than 64
     faces reads a miss count equal to the faces that found no slot, and that those pixels'
     emissive is non-zero (the fallback). Under 5 s.
  5. `a_large_edit_box_clears_every_cell` asserts, with a test-only dispatch cap of 4 groups,
     that a box of 1000 voxels clears all 6000 cells; and `an_overflowed_edit_frame_clears_the_table`
     asserts that a frame with `MAX_EDITS + 1` boxes leaves every cell at count 0. Under 5 s each.
  6. `frame_bytes_hold_the_table_and_no_emissive_history` in `upscale.rs` asserts `frame_bytes`
     at the default equals main's 130 267 216 B minus the `f32` pair's excess (3 686 400 B,
     ADR-0036's figure) minus the reservoir textures' bytes (dev computes and records) plus
     41 943 040 B, with the arithmetic in the test's comment. Under 5 s.
  7. **Amended 2026-11-04 (architect, for the owner): a reading, not a bar.**
     `a_fast_yaw_reads_an_excess_today_and_none_from_the_table` is renamed
     `the_fast_yaw_excess_by_table_read` and stays on the soak list (two runs of about 200
     frames, 29.0 s read on lavapipe). At `VEIN_POSE`, lamp on, indirect off, FSR 2 at 1.5,
     pinned, `yaw 90 10 f`, frames 1 to 60 after, it runs the default and the test-only
     `table_read: false` arm and prints each arm's excess, floor and floor sd, and the noise
     line (60 x the floor's sd) the old assertion used. It asserts only that each arm's floor
     is measured (a non-zero still-frame dot count with its sd) and that the 60 frames ran; it
     asserts no excess. Phase 2's reading stands as its first row: **read off -911.7 (floor
     69.3, sd 11.6, line 696.6); table +552.5 (floor 27.6, sd 2.0, line 120)**. Beside it the
     log sets Phase 1's rows at the same pose, lamp and yaw (`off` +252, the spike's `b` +158;
     one seed, Phase 1's tree), so the table's excess reads against v0.41.0's. The test's
     first premise (the screen path alone reads an excess) did not hold, and Plan 0051's Phase
     3 read the same; the bar the test carried, that the default puts no excess over its own
     floor after a fast yaw, is the owner's want and is Phase 4's done-when 7, where the
     mechanism that could meet it is built.
  8. The banner and the HUD show the table row (`the_banner_names_the_table`, `hud.rs`, no GPU,
     under 1 s), and `--steady`, `--cache-read`, `--emissive-guard`, `--reuse-m-cap` and
     `--sampler` are refused with a message that names this plan's number.
  9. The readings above are in the log. The predictions: the emissive part's `sigma_view` reads
     0.0000 at scale 1 at both poses and under 0.0015 at `up1.5`; the hall's drift at native 4096
     against 8192 reads under 0.01; occupancy after four flights stays under 0.25 of 2^20 (the
     spike read 59 697 cells after one, so about 240 000 is the guess) and the miss count 0. A
     miss is a reading, and a miss count above 0 sends the size to the architect.
     **Amended 2026-11-04 (architect, for the owner):** read, three predictions missed (the
     patch's emissive `sv` 0.0002 and 0.0003 at scale 1; the patch's full-table share 0.00029
     at 0.0424 occupancy; the hall's native drift 0.257). The first is under the hall's bar
     below and stands as a reading. The misses go to Phase 4's Part 1 (the table's census) and
     the hall to Part 1 (the still hall's attribution); both are fixed in Part 1b. Phase 2 is
     not held.
  10. With every deleted mechanism gone, `cargo build` has no dead `Lighting` field, and the
     gate's test count in the log names each removed and each re-pointed test.

### Phase 3: The world-space guard
- **Owner skill:** dev
- **What:** invalidation beyond the edit box: each cell keeps the emitter that lit it most, and
  the gradient pass re-shades that point from the face and reopens the cell where the light
  changed.
- **The guard:**
  - **The kept emitter.** When the light pass adds a sample whose contribution's luminance is
    above the cell's stored kept luminance, it stores the sample's emitter voxel and face and
    that luminance (`atomicMax` on the luminance word, then a compare-exchange on the point; a
    lost race keeps the other winner, which is as good). The fill pass (Phase 4), if built, does the same.
  - **The re-shade.** The gradient pass, for each 3 x 3 stratum of lighting pixels, picks one
    pixel by its hash of stratum and frame as today, takes its cell, and re-shades the cell's
    kept emitter from the face's centre: one shadow ray toward the emitter face's centre and
    the emitter material's emission now, giving an irradiance `e_new`; `e_old` is the kept
    luminance. Where `|e_new - e_old| / max(e_new, e_old)` exceeds the dead zone (0.05,
    ADR-0036's `LAMBDA_FLOOR`), the cell reopens: count, sums, kept point and luminance cleared.
    A cell with no kept emitter (it was lit by nothing) is re-verified by a fresh sample instead:
    the pixel draws its candidates, and a non-zero result reopens the cell.
  - **Cadence by arithmetic.** One cell a stratum a frame means a face of 54 lighting pixels is
    re-verified about six times a frame and a face of 0.34 pixels about once in 26 frames. The
    guard is for reveals and digs, which the owner sees at arm's length; the far case is a
    reading.
  - **The screen-space emissive lambda** of ADR-0036 is gone with Phase 2; the gradient pass's
    combined lambda for the lamp's part stays.
- **The readings:** the reveal and the dig at the rig's two distances (Phase 1's events): frames
  until the lit wall's cells reopen and until their means are within 10 % of a fresh run's; the
  gradient pass's cost on both adapters; a still scene's reopen count over 1024 frames (the
  prediction: 0).
- **Files touched:** `crates/vxe-render/src/shaders/light.wgsl`, `gradient.wgsl`, `march.wgsl`,
  `crates/vxe-render/src/marcher.rs`, `apps/flythrough/tests/lights.rs`, `settle.rs`,
  `docs/plans/logs/0057.md`.
- **Done when:**
  1. `a_revealed_emitter_reopens_the_cells_it_lights` asserts, on a box with an emitter behind a
     one-voxel plug and a wall of capped cells, scale 1, that after the plug's removal every
     cell of the wall within the emitter's reach reopens within 9 frames and reads within 10 % of
     a fresh run's mean within 16 frames at arm's length. About 100 frames: about 6 s.
  2. `a_removed_emitter_darkens_the_cells_it_lit` asserts, on the same box, that replacing the
     emitter with rock reopens the lit cells within 9 frames and their means fall under 1 % of
     the lit value within 16 frames. About 6 s.
  3. `a_still_scene_reopens_no_cell` asserts that 64 still frames on the box leave every count
     monotone and the reopen counter at 0. About 4 s.
  4. `a_reveal_behind_a_plug_follows_within_seventy_three_frames` returns in `lights.rs` on the
     table, with its bar re-derived from done-when 1 (the plan's claim: within 16 frames at arm's
     length). Its time in the log.
  5. The readings are in the log. The prediction: the gradient pass's cost stays within 0.02 ms
     of Phase 2's on the RTX (it casts the same one ray a stratum).
     **Amended 2026-11-04 (architect, for the owner):** read, both predictions missed. The
     cost: the prediction compared with Phase 2's pass, which Phase 2's default did not encode
     (0.000); against Phase 1's `b --indirect off`, which ran it (0.017 spawn, 0.018 vein), the
     guard reads 0.092 and 0.053 on the RTX and 0.395 and 0.236 on the Cezanne (loaded,
     indicative). That is light plus gradient 0.239 ms of `docs/nfr.md` section 2's 6.0 ms
     lighting line on the RTX and 1.147 ms on the Cezanne, and the frame at the spawn 3.285
     against the Budgets row's guess of 3.555: the cost is accepted, and the prediction is
     restated for Part 1b's tree in Phase 4's done-when 10. The still reopens (patch 0 / 2 / 2 /
     4 at 256 / 512 / 768 / 1024 frames, lamp on and off alike; vein 0) break the owner's first
     rule, are not a reading to accept, and are fixed in Phase 4's Part 1b against Phase 4's
     done-when 9.

### Phase 4: The dug hole and the creases, read on the table, then fixed where read
- **Owner skill:** dev
- **Amended 2026-11-04 (architect, for the owner).** Phase 1's stop rule read stop, so the phase
  as first written (a fill of every young face in view and warm rays from the eye) is not built.
  What it now builds, and what it drops, with the readings that decide each:
  - **Dropped: the warm rays, and the fill of every young face in view.** Both served the fast
    turn's young faces. Phase 1 read the young-cell share of the excess dots at the fast yaws at
    0.005 to 0.099 in every positive row (`b` and `ab`, both poses, lamp on and off, `up1.5` and
    scale 1), and the young dots at most 171 over 60 frames in any row. Plan 0056 read the
    yellow dots as light on creases (90.6 to 96.7 % at 10 px a frame), present native and under
    FSR 2, 19 to 35 % world-anchored and the rest new each frame. Neither describes a face that
    has too few samples, so sampling the faces in view buys the turn nothing, and its burst (the
    first draft's guess: about 150 000 samples at a turn's first frame) is cost the screen does
    not show. `W`, `warm_rays` and the 0-4096-16 384 sweep go with it.
  - **Dropped as a bar: the hall from a fresh start** (the first draft's 256 against 8192 under
    0.5). A fresh start is not a reopened cell, so nothing here fills it; Phase 2's hall rows are
    read again on this phase's tree and the owner judges them in Phase 6's run 1.
  - **Kept, if read owed: a fill pass for reopened cells (the dug hole).** Phase 1 read the hole
    slow at both distances (`S90` 335 frames at arm's length and 2031 at 100 voxels, scale 1;
    `up1.5` reads none), with the young share 0 by frame 16 at arm's length and `b1` (`C_MIN` 1)
    reading the same as `b`, and lamp on and off alike: the hole's cells leave the young class
    fast and their means then converge at the samples the hole's pixels give them. On the spike
    the screen's emissive history and reservoirs still sat in that path, and Phase 2 removes
    them, so the hole is read again before anything is built.
  - **Kept, if read owed: the composite reads a pixel's emissive from its own face (the creases).**
    Plan 0056 puts the dots on creases at both upscalers, and Phases 2 and 3 do not reach the
    crease: Phase 2 removes the screen history, reservoirs and the a-trous on the cached part,
    Phase 3 reopens changed cells, but the composite still builds each full-resolution pixel's
    light from the four half-resolution lighting pixels around it (`composite.wgsl`, `lit_at`),
    weighting by normal and plane, and takes the nearest tap's value when every weight vanishes.
    At a crease, with the camera moving, that nearest tap can lie on the other face, so a pixel
    shows its neighbour face's cell for one frame. This is a candidate the readings below
    attribute or refute, not yet a reading.
  - **Amended 2026-11-04 (architect, for the owner), from Phase 2's done-when 7.** The fast
    yaw's excess is now measured headless on the table: +552.5 over a floor of 27.6 (sd 2.0)
    with the table's read on, and none (-911.7 over 69.3) with it off (`logs/0057.md` Phase 2).
    So the table's read is what puts the dots up after a yaw, and they fall on mature cells
    (Phase 1). The rig's row joins the crease reading as a fifth row, its default taken from
    that reading until Phase 3's tree re-reads it, and the bar Phase 2's done-when 7 carried
    lands here as done-when 7. A second candidate is named, not built: at a true face edge FSR
    2's jitter moves a pixel between two faces' cells each frame, a hard step that Phase 2's
    removal of the screen history no longer smooths (v0.41.0 read +252 at the row, the spike's
    `b` +158). The rig row at scale 1 and at `--lighting full` splits the two.
- **The settle bar for the dug hole, stated.** The owner: "I can count to 5 when light settles"
  (`LIGHT-FEEDBACK.md`) and "when i dig it takes some time for light to settle in hole"
  (`logs/0051.md`, Phase 4, note 6). Five counts is the fault, so the bar is one count, one
  second, at `docs/nfr.md`'s 60 fps target, the slowest rate the budget allows (the fly-through
  presents without vsync, so the RTX reaches it in about a fifth of that): **the rig's dig at
  `VEIN_POSE` at arm's length (`[723, 646, 208]`, 5 voxels from the eye), scale 1, lamp off:
  the hole's emissive `S90` at most 60 frames**; the dig 100 voxels down the passage (Phase 1's
  `dig100`), scale 1, lamp off: **at most 120 frames**, two counts, since the owner sees it from
  range. `up1.5` is read at both and is not asserted (Phase 1 read `S90` none there: the step
  lies within FSR 2's noise).
  - **Amended 2026-11-05 (architect, for the owner): on the level the bar is `Q90`, not `S90`.**
    `S90` asks each pixel to sit within `log2 1.1` of its mean over frames 2048 to 2111, a
    capped cell's 4096-sample estimate, which a dim face reaches only near `C_CAP` (the log's
    settled share 36 % at 16, 51 % at 60, 89 % at 256 while p50 holds 488, 1048 and 3400).
    `Q90` (the header's amendment) asks that the pixel stop moving. **The derivation, a guess
    the reading replaces.** A cell of `n` samples gaining `k` a frame moves its mean by about
    `sigma sqrt(k) / n` of itself (rms). ACES's toe is linear and the encode a power of 1/2.2,
    so a relative change `r` moves a pixel at level `D` by about `D r / 2.2` levels (the
    shoulder only lowers it), and nine frames in ten move under one level when `1.645 D sigma
    sqrt(k) / (2.2 n) < 1`, that is **`n > 0.75 D sigma sqrt(k)`**. A face lit on a share `p`
    of its samples has `sigma` about `sqrt(1 / p)` and `D` about `p^(1/2.2)` times a lit wall's,
    so `D sigma` goes as `p^-0.05`: 1.19 times at `p` 1/32. Take `D sigma` 180 (a lit wall at
    level 180; 255 the bound): **`n > 135 sqrt(k)`**. At arm's length, preload on, the hole's
    cells gain about 7, 13 and 30 samples a frame from frame 16 to 60 at p10, p50 and p90
    (704 - 408, 1048 - 488 and 2052 - 744 over 44) and need 351, 487 and 740; they hold 408,
    488 and 744 at frame 16: **`Q90` about 16 frames, about 32 at the bound** (p50 reaching 690
    at 12.7 a frame). Preload off: p50 296 at 16 gaining 12 a frame, about frame 30. At 100
    voxels: p50 144 at 16 gaining about 3.4 a frame needs 248, **about frame 47, about 60 at the
    bound** (after frame 60 the cells gain about 1 a frame and need 191, under their 292). Frames 1 to 16 cannot be still (about 31 samples a frame into cells of 20). So
    the 60 and 120 frame bars stand with a margin of about 2 at the bound; the
    criterion is tightened in kind (every frame after `Q90` still), not loosened. A face
    still under a level a frame keeps crawling until `C_CAP`, by about `D sigma / (2.2
    sqrt(n))` levels in all, about 4 at `n` 500 (a guess): the **travel after `Q90`** is read
    beside it for the owner.
- **Part 1, the readings first, on Phase 3's tree** (the rig, lavapipe for asserted figures and
  the RTX through `VXE_TEST_ADAPTER=rtx` for the rest, lane rule 9c):
  - **The hole:** `dig_box_readings` at both distances, scale 1 and `up1.5`, lamp on and off:
    `S90`, mean90, settled % and the young share, beside Phase 1's rows. **The rule:** if both
    distances meet the bar at scale 1, the fill pass is not built and the log says so; if either
    misses, Part 2 builds it.
  - **The creases:** Plan 0056's fast yaw (90 degrees at ten display pixels a frame, 240 frames)
    at the vein and the patch, FSR 2 and native, lamp off, at the default and at `--lighting
    full`: the yellow dots a frame by 0056's rule (a dot by the settle rig's rule whose red and
    green are both over twice its blue and green over half of red; dev writes it in
    `support/settle.rs` from `logs/0056.md`, without touching 0056's files), with 0056's `H` rows
    beside (131.95, 253.66, 61.39 and 63.30 a frame: patch FSR 2, patch native, vein FSR 2, vein
    native), and for each yellow dot whether its displayed emissive lies more than 10 % from its
    own face's cell mean times the exposure (the cell read back on the CPU as Phase 1's rig
    does). **The rig's row (amended 2026-11-04):** `the_fast_yaw_excess_by_table_read`'s run
    (vein, lamp on, `yaw 90 10 f`, frames 1 to 60) at the default and at `--lighting full`, each
    at `up1.5` and scale 1, with the excess dots split the same way (displayed emissive more
    than 10 % from its own cell or not); the default at `up1.5` is Phase 2's +552.5 over 27.6
    until this tree re-reads it, and the log sets both beside. **The rule:** if `--lighting full`
    reads at most half the default's yellow dots (the rig's row: half its excess) in at least
    three of the five rows, the rig's row among them, and over half of the default's yellow
    dots (the rig's row: its excess dots) lie more than 10 % from their own cell, Part 3 builds
    the own-face read; otherwise it is not built, and the log carries the split, the rig row's
    scale-1 figure with it, to the plan's Followups. The rig's row is required because it is
    the one row where the table's read was shown to carry the excess.
  - **Amended 2026-11-04 (architect, for the owner): three more readings, on Phase 3's tree,
    before Part 1b, Part 2 or Part 3 is built.** They come first because the owner's one
    priority is a still image, and every later reading (the hole's settle, the creases' dots)
    is read against a frame that must first hold still.
  - **The still hall, by class (the attribution run).** Phase 2 read the hall at native drift
    0.257 (p99 4, max 203, over 8 0.52 %) and one frame 8192/8193 0.137 (max 175, over 2
    0.78 %), against Plan 0051's `b-noind-native` 0.004 and 0.001 (max 1) with the same cell
    arithmetic; at `up` 0.707 and 0.255 against 0.525 and 0.178. A one-frame step of 175 levels
    with nothing moving is not a mean of a thousand samples converging; it is a pixel whose
    value is redrawn. The run repeats the hall (`hero --exposure 8 --adapter NVIDIA`, `N` 4096,
    8192 and 8193, native and `up`, lamp off) with a per-pixel class mask written beside each
    image (a test-only output of the composite or the temporal pass, dev's choice), and splits
    the drift's and the one frame's sums, means, maxima and over-2 and over-8 shares by class.
    The classes, each a candidate the run must keep apart:
    1. **The fallback, no face** (`rec.field`, translucent, `rec.axis < 0`): one sample a
       frame blended at `ALPHA` 0.2, which redraws every frame by construction. The leading
       candidate for the one-frame steps.
    2. **The fallback, a missed key** (a cacheable hit whose key found no slot): the same
       redraw, on a face the table should hold. Its share joins the census below.
    3. **A cell under the cap** (count under `C_CAP` at frame 4096): still accumulating, so
       its mean moves by about its noise over `sqrt(count)` between 4096 and 8192, a broad,
       small drift; it cannot carry a 175-level step in one frame. Read its share of the drift
       against its count.
    4. **A capped cell** (count at `C_CAP`): bit-identical by construction; any change in this
       class is a defect (a duplicate key's second cell, a race in the claim), named as such.
    5. **A reopened cell:** the guard's reopen counter over the run, and the pixels whose cell
       reopened between the two frames (the reopen bit or a count that fell); on Phase 3's tree
       this includes a torn-store tag that reopens.
    6. **The half-resolution upsample at a crease:** at a still native camera the taps are
       fixed, so this class can only pass on a change in classes 1 to 5; the run reads the
       pixels whose four taps span two cells apart, to confirm it carries nothing still.
    7. **FSR 2's jitter across face edges:** `up` only; its share is the `up` row minus the
       native row, class by class, and it is Plan 0056's to judge (this plan asserts native).
    Also counted over the run: whole-table clears (the emission-scale clear of Phase 2's
    deviation, the edit overflow, the sweep), each of which would reset every class at once.
    **The rule:** the class or classes carrying over half of the native one frame's sum, and
    over half of the native drift's, are Part 1b's to fix. If class 3 alone carries the drift
    and the one-frame bar is met without it, the drift is the cells' settle and goes to the
    owner in Phase 6 (a) with its count-by-frames table, not to a mechanism.
  - **The table's census (the misses).** The patch read a full-table share of 0.00029 at an
    occupancy of 0.0424; the vein 0 at 0.0576. Four linear probes at a load of 0.04 under a
    uniform hash leave a key without a slot with a probability near `0.04^4`, about 3 x 10^-6,
    so the 44 448 keys would expect under one missed key in the whole table; a share of pixels
    of 0.00029 is a near face, or many keys. A size change does not answer that; the hash's
    spread or the claim does. Dev reads, on the CPU, the patch's and the vein's key sets (the
    cells dumped after the four flights, by key) through `marcher::cache_slot`: the slot runs'
    lengths against a seeded uniform assignment of the same keys, the keys with no free slot
    in four, and duplicate keys (one key held in two cells, which `cache_claim`'s exchange of
    word 1 before word 0 allows when a second pixel reads word 0 before it is stored). **The
    rule:** whichever of the hash, the claim or the counting (a pixel counted twice, a miss
    counted during a clear) the census names, Part 1b fixes; the size stays 2^20 unless the
    census reads uniform runs and the misses persist, which comes back to the architect.
  - **The still reopens, by cause.** Phase 3 read 4 reopens over 1024 still frames at the
    patch, lamp on and off alike. Dev logs each reopen's branch in `gradient.wgsl`: a kept
    emitter whose re-shade left the dead zone (with `e_old`, `e_new`), a torn-store tag that
    found its key lighting nothing, or a dark capped cell whose fresh sample drew light (a rare
    path a cell of 4096 zero samples has not met, which the scene's stillness does not stop).
    Its verdict line names the cause or causes for Part 1b.
- **Part 1b, the still image, fixed (amended 2026-11-04, architect, for the owner; built before
  Parts 2 and 3).**
  - **The guard reopens only while the world has changed.** With nothing edited, no chunk of
    the light radius uploaded and no emission scale changed (which clears the table already),
    no cell's emissive light can change, so a reopen is noise. The gradient pass's re-shade and
    its fresh sample run, and may reopen a cell, only within `GUARD_WINDOW` frames of such an
    event (a frame counter in the params, reset by the edit boxes, the mirror's uploads within
    `light_radius` and the emission scales; 256 a guess, at least the far face's re-verify
    cadence of about 26 frames ten times over; dev states the events in a comment). Outside the
    window the guard does nothing and the gradient pass encodes the lamp's lambda only. Dev
    also fixes, inside the window, any cause the reopens' attribution names that is not a
    change (a torn tag read as a reopen, a dead zone crossed by the packed luminance's 16-bit
    rounding). Phase 3's done-whens 1 to 4 still hold (the reveal and the removal open the
    window by their edit).
  - **The still hall's attributed class, fixed.** If class 1 or 2 carries it: the fallback
    becomes a running mean that freezes while its pixel's reprojection holds (1 / n up to a
    cap, then held; restarted where the reprojection fails, an edit box touches the pixel or
    the lamp part's reset fires), so a still fallback pixel holds still; the lamp's own `ALPHA`
    is untouched. If class 4 or 5 carries it: the defect the census or the reopens' cause
    names. Any other attribution comes back to the architect with the table before a mechanism
    is built.
  - **The table's misses, fixed** as the census names (a hash that mixes the key's two words
    whole, a claim that stores the key before it publishes it, or a corrected counter); the
    size stays 2^20.
  - **Order:** the guard window, then the misses, then the fallback, each with its test; then
    the hall and the still reopens are re-read on Part 1b's tip before Part 2 starts.
- **Part 2, the reopened fill (if the hole's rule says build):**
  - **The list.** The edit-box clear and the guard's reopen set a `reopened` bit in the cell's
    stamp word; a pixel whose cell carries the bit, is under `C_FILL` samples (swept 64, 256;
    the default is the reading's) and whose stamp is not this frame's sets the stamp
    (compare-exchange) and appends the cell's index to the frame's fill list (at most
    `FILL_LIST_MAX` entries, 16 384 a guess; a full list drops the rest and counts them). A cell
    reaching `C_FILL` clears the bit. A face seen for the first time, with no edit, is never
    listed.
  - **The pass.** One workgroup of `F` lanes per listed cell (`F` swept 4, 8, 16; the default is
    the smallest that meets the bar), each lane drawing one resampled emissive sample from a
    random point on the face, with its shadow ray, added to the cell and offered as the kept
    emitter (Phase 3). It runs after the light pass and before the temporal pass.
  - **Cost by arithmetic.** The list is bounded by the reopened faces in view: the rig's dig of
    radius 10 exposes a sphere's surface, 4 pi 10^2, about 1260 faces, about 1900 with the
    voxels' stair (a guess of 1.5), so at `F` 8 about 15 000 samples on the dig's first frame,
    under a tenth of a 720p lighting pass's emissive work, falling as the cells reach `C_FILL`;
    nothing at rest and nothing on a turn. A guess the readings replace.
- **Part 2b, the camera-radius preload (amended 2026-11-04, owner; its form amended 2026-11-04,
  architect, for the owner, from the visibility reading; built after Part 2, on its fill
  routine).** The owner's settle complaint is the cells a turn, a step or a dig brings into view
  before they hold enough samples, and the ask is to preload the light "around the camera in
  certain radius, no matter where this camera looks". Part 2 fills a cell only after an edit or
  the guard reopens it; a face seen for the first time converges at its footprint (Phase 1: one
  frame at arm's length, about 48 at 88 voxels, slower where most samples are shadowed). Part 2b
  fills **every lit face within `R` of the eye, found from the mirror's bricks, not from rays**,
  so a face is settled before any view reaches it, whether a turn, a walk round a pillar or a dig
  shows it. **Order:** Part 1b, then Part 2, then Part 2b on Part 2's routine, then the end
  readings.
  - **Why not the eye rays (the form first written).** `preload_visibility_readings` (the lane's
    log, Part 2b) read the share of entering pixels within `R` that some eye of the previous 60
    frames could see at **0.002 and 0.007 on the walks** against 1.000 on the yaws: walking, a
    face enters view by coming out from behind something, so rays from the eye cannot reach it,
    and that is the "no matter where this camera looks" the owner asked for. Within `R` the
    brick sweep below covers every face the eye rays would, so the rays add only discovery
    latency (a Followup if done-when 14 reads it) and a second mechanism to test. They are not
    built.
  - **The sweep (discovery).** A cursor visits `B` bricks a frame (`preload_bricks`, swept 64,
    128 and 256; the default the smallest that meets done-whens 13, 14 and 16) from a fixed
    table of the brick offsets within `R / BRICK_EDGE` bricks of the eye's brick (at `R` 64,
    about 2145 of the 17^3 cube), sorted by distance, nearest first; the cursor wraps and does
    not restart when the eye changes brick, so a still camera visits every brick within `R` each
    `ceil(2145 / B)` frames (17 at `B` 128). The bricks the frame's dilated edit boxes touch
    (`light_edits`, as Part 2's addition reads them) are visited first that frame. A visit is one
    workgroup per brick reading the mirror as the march does (`chunk_table`, the record's brick
    bits and entries, a Full brick's opaque mask unit; ADR-0002, ADR-0012, ADR-0041): a brick of
    uniform air, or uniform solid with six uniform solid neighbours, ends at once; otherwise each
    opaque voxel's face toward a cacheable air neighbour (the light pass's rule, as
    `the_table_hash_spreads_a_levels_keys` enumerates it) is looked up by `cache_slot`. A face
    whose cell holds `C_PRE` samples is skipped. A face with no cell draws `k0` samples (8)
    through the fill routine and claims a cell (`cache_claim`) by the claim rule below. A face
    with a cell under `C_PRE` is listed once (a `preloading` bit in the stamp word beside Part 2's
    `CACHE_REOPENED`).
  - **The claim rule (amended 2026-11-05, architect, for the owner, from the level readings).**
    Two bands by the distance from the eye to the face's voxel centre, the test `R` uses:
    - **Within `r0`** (`preload_claim_radius`, 24 voxels by default, swept 16, 24 and 32; the
      default the largest whose walking cost meets done-when 17's 0.15 ms at both poses): a face
      with no cell **claims one whatever its `k0` samples read**, takes them, and is listed for
      the fill to `C_PRE` whether or not a sample was lit (the sweep's `preload_wants` lists a
      cell within `r0` under `C_PRE` without asking for a kept emitter). A face no emitter
      reaches is then filled once to `C_PRE` with zeros and skipped as done after that, for the
      life of its cell, instead of being probed again every epoch. The test is the eye's
      distance, so it is the same on every frame for the same eye.
    - **From `r0` to `R`:** the 8-probe rule, unchanged: the face claims a cell only if one of
      its `k0` samples is lit; a face that draws none is probed once an epoch (the slab stamp).
    - **Both bands:** no claim while the last frame's occupancy is at `PRE_LOAD_MAX` (0.25) or
      over, or its light pass counted a miss. The HUD counts claims by band and refusals by
      cause (load, miss, unlit).
    - **Why `r0` 24.** The faces that carry a turn's pixels are the near ones (a face's pixel
      area falls as `1 / d^2`), `VEIN_CORNER` is defined within 24 voxels of `VEIN_POSE`'s eye
      (done-when 16), and the walk's new faces within `r0` grow as `r0^2` (the flux `pi r0^2
      v`), so at 24 they are about `(24 / 64)^2`, a seventh, of the flux into `R`, and at 32 a
      quarter. Why not all of `R`: the vein walk probed about 211 faces a frame (the cost
      table), and filling every one to `C_PRE` is about 54 000 samples a frame, over `S_PRE`
      and about twice the walk's 0.15 ms by the measured 4.6 ns a sample.
  - **The list and the fill.** A persistent preload list, in the counters buffer past Part 2's
    fill list (the light pipeline's 11 storage buffers are full; a preload pipeline binds the
    world's 9, the table and the counters, as the fill's does), at most `PRELOAD_LIST_MAX`
    entries (32 768 a guess; a full list drops the new entries and counts them; the sweep lists them again on its next pass), ping-ponged each
    frame: Part 2's fill routine (`cache_fill.wgsl`'s `fill` and `light.wgsl`'s `face_point`,
    factored as `fill_face`) draws `k` samples (16, `FILL_LANES`) for each of the list's first
    `S_PRE / k` entries in sweep order, and a cell that reaches `C_PRE` (`preload_until`, swept
    256, 1024 and 4096) clears its bit and leaves; the rest carry to the next frame's list with
    the sweep's new entries. An entry whose cell was swept, cleared or re-keyed since is dropped
    on the key check. The pass runs after the light pass and Part 2's fill, before the temporal
    pass; it adds no store beyond the list and fills the existing table.
  - **The budget.** `S_PRE` (`preload_samples`, 32 768 a guess, swept 16 384 and 32 768) caps the
    frame's samples. When the list is empty, the camera has moved under one voxel and turned
    under one degree since the last frame, and the guard window is closed, the cursor visits
    `B / 16` bricks a frame (the trickle that finds a face a small move opens); otherwise `B`.
  - **The demand, by arithmetic from the census.** `the_table_hash_spreads_a_levels_keys` counts
    28 696 air faces in the 128-voxel cube at the patch and 47 835 at the vein (`logs/0057.md`,
    Part 1b), so about **15 000 and 25 000 within `R` 64** (the sphere is 0.52 of the cube; a
    guess at even density, which done-when 12's census replaces). The first draft's objection
    that a brick list costs a scan of 4096 bricks fails on these figures: the sweep is about
    2145 bricks, at `B` 128 one sweep each 17 frames, and the faces are a tenth of the eye rays'
    occupancy guess. At the vein's density, about 0.023 faces a cubic voxel, a walk at 30 voxels
    a second sweeps `pi R^2 v`, about 386 000 cubic voxels a second, into `R`: **about 150 new
    faces a frame**, half of them lit (a guess), times `C_PRE` 256: **about 19 000 samples a
    frame, under `S_PRE`**. At `Shift`, 120 voxels a second, about 75 000, over `S_PRE` by about
    2.3: the faces then reach the eye with about 256 / 2.3, about 110, samples, which is within
    10 % for a lit face of relative spread near 1 and short of it for a dark one (a reading). A
    face revealed at `d` voxels has been within `R` for `(64 - d) / v`: at 32 voxels, walking, 64
    frames, 1024 samples' time at 16 a frame. A fresh table at the vein: about 12 500 lit faces
    times 256 over 32 768, **about 100 frames**, nearest first. All guesses, which done-when 12's
    fill-time table replaces.
  - **What it cannot fill.** Faces beyond `R`: the vein walk's entering pixels lie 0.825 beyond
    64 voxels (the histogram), each under about one lighting pixel, and they settle at their
    footprint as today (a reading in done-when 14, the owner's in Phase 6 (h)). Faces of chunks
    not yet uploaded, which no view shows either. Faces the table has no room for: in the hall
    (0.185 native, 0.260 up, with 659 and 140 missed lighting pixels; the lane's log, the hall's
    census) the claim rule stops at once at up and as soon as a miss is counted at native, so
    the preload fills there only cells the view made; a larger table is Phase 6 (h), the owner's.
  - **`R`: 64 voxels stands, as a brick radius (8 bricks).** The histogram puts most of the vein
    walk's entering pixels beyond 64, but the sweep's faces and samples grow with `R^3` (96
    voxels: 3.4 times the faces, about 85 000 at the vein and 0.08 of the table, and 2.25 times
    the walk's flux), and a face beyond 64 voxels covers under a lighting pixel. `R` tied to the
    chunk upload is rejected: `light_radius` is 4 chunks (256 voxels), 64 times the volume,
    about 1.6 million faces at the vein's density, over the table. `--preload 96` is read beside
    64 in done-when 14 for the owner's call.
- **Part 3, the own-face read (if the creases' rule says build):** the composite, for a
  full-resolution pixel whose hit has a face key and a cell, takes the emissive part as that
  cell's mean times `exposure_now()`; the lamp part and the fallback's emissive are upsampled as
  today, and a pixel whose face has no cell takes today's upsample. One table lookup a
  full-resolution pixel, at most four probes: **a guess** of under 0.1 ms on the RTX at
  1280 x 720 internal, read.
- **The readings, at the end** (on the phase's tip, whatever was built): the hole at both
  distances against the bar; the creases' yellow dots at 0056's fast yaw, four rows, against
  0056's `H` rows; the rig's dots and `K` at its yaws, both poses, lamp on and off; the hall at
  256, 512, 4096, 8192, native and `up`; the fill and the composite's cost at rest, on the dig's
  first frames and in the flight, both adapters, for each `F` and `C_FILL`; amended 2026-11-04:
  the still hall's class table, the still reopens and the occupancy with misses, against
  done-whens 8, 9 and 11, so a fill or an own-face read that moves a still frame shows there;
  amended 2026-11-04 (owner): each of these with the preload on and off, the preload's cost at
  rest (the trickle), in the flight and on a turn's first frames at each `B` and `S_PRE`, both
  adapters, and the occupancy with misses at the vein, the patch and the hall with it on.
- **Files touched:** `crates/vxe-render/src/shaders/cache_fill.wgsl` (new, Parts 2 and 2b),
  `temporal.wgsl` (the fallback's mean, Part 1b), `crates/vxe-render/src/marcher.rs`'s
  `cache_slot` and its CPU test (Part 1b),
  `cache_clear.wgsl`, `gradient.wgsl` (the reopened bit), `composite.wgsl` (Part 3), `light.wgsl`,
  `march.wgsl`, `crates/vxe-render/src/marcher.rs`, `apps/flythrough/src/main.rs`, `hud.rs`,
  `apps/flythrough/tests/settle.rs`, `support/settle.rs`, `lights.rs`, `upscale.rs`
  (`frame_bytes`), `docs/plans/logs/0057.md`.
- **Done when:**
  1. The log holds Part 1's tables and two verdict lines, the hole's (build or not, with the
     `S90`s) and the creases' (build or not, with the shares); amended 2026-11-04, and three
     more: the still hall's (the classes over half, with their shares), the census's (hash,
     claim or counting) and the reopens' (the cause by branch).
  2. If Part 2 is built: `a_dug_hole_settles_within_the_bar` asserts, on `cache_box` at scale 1,
     lamp off, with the camera 5 voxels and 100 voxels from a dig of radius 10, that the hole's
     emissive `S90` reads at most 60 and at most 120 frames, and that with `F` 0 the 100-voxel
     dig reads over 120 (the test fails first; if it does not, the log says so and the fill's
     need rests on the level's reading). About 2 x 300 frames: about 20 s.
  3. If Part 2 is built: `a_reopened_face_fills_and_a_new_face_does_not` asserts, on `cache_box`
     at scale 1 with a one-voxel face 100 voxels away (under one lighting pixel), that after an
     edit box clears its cell the count reads at least `F` after frame 1 and at least `2F` after
     frame 2, and that the same face first seen with no edit reads at most 2 after frame 2.
     About 4 s.
  4. If Part 3 is built: `a_crease_pixel_reads_its_own_face` asserts, on `cache_box` at half
     lighting with an emitter lighting one wall of an L crease and not the other, the camera
     stepped one full pixel a frame for 16 frames, that every full-resolution pixel whose hit
     has a cell shows its own cell's mean times the exposure within one `f16` step; and that
     with the read disabled by a test-only setting at least one pixel of the dark wall shows the
     lit wall's value in some frame (the test fails first). Under 5 s.
  5. What was built shows in the banner and the HUD (`F`, `C_FILL`, `own-face read`; `hud.rs`'s
     banner test, no GPU, under 1 s), and `frame_bytes` reads Phase 3's plus the fill list's
     bytes if Part 2 is built, asserted in `upscale.rs`.
  6. The end readings are in the log. The predictions: the hole meets the bar at `F` 8; with
     Part 3 the yellow dots a frame read under half of 0056's `H` rows in every row. A miss is a
     reading the owner judges in Phase 6.
  7. **Added 2026-11-04 (architect, for the owner), from Phase 2's done-when 7.** If Part 3 is
     built: `a_fast_yaw_reads_no_excess_from_the_table` asserts, on the rig's row (`VEIN_POSE`,
     lamp on, indirect off, FSR 2 at 1.5, pinned, `yaw 90 10 f`, frames 1 to 60), that the
     default's excess over its floor is at most its noise line (60 x the floor's sd; 120 at
     Phase 2's sd 2.0, under v0.41.0's +252 at the row), and that with the own-face read
     disabled by a test-only setting it reads over that line (the test fails first; Phase 2
     read +552.5). Two runs of about 200 frames: about 30 s, on the soak list from birth. If
     Part 3 is not built, the test is not written and `the_fast_yaw_excess_by_table_read`'s
     row on this phase's tip goes to the log and the Followups.
  8. **Added 2026-11-04 (architect, for the owner), the still hall.** The log holds Part 1's
     class table (native and `up`, drift and one frame, by class, with the clears counted) and
     its verdict line, and the same table re-read on Part 1b's tip and on the phase's tip. The
     bar, at native on the RTX (a reading against a bar, not asserted, since the hall at 8192
     frames does not run on lavapipe): **the one frame 8192/8193 mean at most 0.005 with no
     pixel over 8 levels, and the drift 4096/8192 mean under 0.01** (Plan 0051's arm read 0.001,
     max 1, and 0.004). The asserted half, on lavapipe: if the fallback is fixed,
     `a_still_fallback_pixel_holds_still` asserts, on the translucent-slab box of Phase 2's
     done-when 2, scale 1, lamp off, that after 64 frames the slab's pixels are bit-identical
     over the next 64, and that a one-voxel edit box over the slab restarts them (their value
     changes within 2 frames). About 140 frames: about 6 s. A miss of the native bar goes to
     the owner in Phase 6 (a) with the class table; `up` is a reading for Plan 0056.
  9. **Added 2026-11-04 (architect, for the owner), the still reopens.**
     `a_still_world_never_reopens_a_cell` asserts, on `guard_box` (Phase 3's) with the cause
     the reopens' attribution names built into it (for a rare path: a dark wall lit by an
     emitter seen through a gap small enough that a capped cell drew no light; dev records the
     geometry), scale 1, lamp off, `C_CAP` 256, that over 512 still frames after every cell in
     view is capped the reopen counter reads 0 and every count is monotone, and that with the
     guard window forced open for the whole run (a test-only setting) the counter reads above 0
     (the test fails first; if it does not, the log says so and the bar rests on the level's
     reading). About 600 frames: about 20 s. On the level, `still_reopen_readings` at both
     poses, lamp on and off, 1024 frames, reads **0 at 256, 512, 768 and 1024** on Part 1b's
     tip (RTX, a reading against a bar); Phase 3's done-whens 1 to 4 still pass.
  10. **Added 2026-11-04 (architect, for the owner), the gradient's cost restated.** The
     prediction Phase 3's done-when 5 carried compared with a pass Phase 2 did not encode. On
     Part 1b's tip, RTX, at rest at the spawn and the vein: with the guard window closed the
     gradient pass reads within 0.02 ms of Phase 1's `b --indirect off` (0.017 and 0.018), and
     in the window, read on the dig's first frames, it reads Phase 3's 0.092 and 0.053 within
     0.02 ms (loaded figures, indicative). A miss is a reading; the lighting line's 6.0 ms is
     the bar.
  11. **Added 2026-11-04 (architect, for the owner), the table's census.**
     `the_table_hash_spreads_a_levels_keys` (no GPU, under 5 s) asserts, on the key set of
     every air-facing face within a 128-voxel cube around the patch and around the vein, built
     headless from `vxe-world` at seed 1 (dev's enumeration and the key count, recorded), that
     at 2^20 cells and four probes no key lacks a slot, and that at the power of two nearest a
     load of 0.5 the share of keys without a slot is within twice a seeded uniform
     assignment's of the same keys at the same size. The GPU half: after Part 1b,
     `occupancy_readings` reads a full-table share of **0 at both poses after four flights**
     and no duplicate key in the dump. A miss above 0 at 2^20 comes back to the architect with
     the census before the size changes.
  12. **Added 2026-11-04 (owner), the preload's form; amended 2026-11-04 (architect, for the
     owner).** The log holds `preload_visibility_readings`' table (read: 0.002 and 0.007 on the
     walks), and two new tables. **The census within `R`** (`preload_face_census`, CPU, no GPU,
     `vxe-world` at seed 1, under 5 s, ignored): the cacheable air faces within 64 and 96 voxels
     of `VEIN_POSE`'s, `PATCH_POSE`'s and the showcase hall's eye, and the bricks the sweep's
     early-out leaves to visit, against the arithmetic above. **The fill time** (RTX): from a
     fresh table, the frames until every lit face within `R` holds `C_PRE`, at the vein, the
     patch and the hall, at each `B`, `S_PRE` and `C_PRE`, with the occupancy, the misses, the
     cells the claim rule refused (load, miss, unlit) and the list's drops. Amended 2026-11-05
     (architect, for the owner): the fill time and the claims by band (within `r0`, lit and
     unlit; from `r0` to `R`, claimed and refused by cause) at `r0` 16, 24 and 32, and, from the
     reference run (done-when 14), the faces within `R` by band whose reference mean is 0 (no
     emitter reaches them) against those lit, with the lit ones' share of lit samples by decile,
     so the log says how many of the faces the 8-probe rule refuses beyond `r0` are dim, not
     unlit.
  13. **Added 2026-11-04 (owner), the 180-degree turn.**
     `a_face_behind_the_camera_is_settled_before_the_turn` asserts, on a box with an emitter
     lighting a near wall and a dim corner (lit at under a tenth of the near wall's irradiance)
     6 to 12 voxels behind the camera, scale 1, lamp off, `C_PRE` 256, `C_CAP` 1024, that after
     64 still frames facing away and a 180-degree yaw in one frame, on the first frame after the
     yaw at least 90 % of the corner's pixels read within 10 % of their cell's settled mean (the
     same run continued facing the corner until every corner cell reads `C_CAP`); and that with
     `preload_radius` 0 under 50 % do (the test fails first). 64 frames cover a sweep at `B` 64
     (34 frames) with room. About 2 x 140 frames: about 14 s.
  14. **Added 2026-11-04 (owner), the turn and the walk on the level, readings against bars
     (RTX); amended 2026-11-04 (architect, for the owner); amended 2026-11-05 (architect, for
     the owner), the bands and the reference.** **The reference** (`preload_reference`): the
     run's cameras walked at the largest radius read, every face within it claimed whatever its
     probes read (no claim rule; the load and miss guards are read, and a refused claim is
     printed), each 16th camera and the last held until the lists read empty and every listed
     cell holds `C_CAP` (4096); a reference cell is one at `C_CAP`. **A pixel whose face has no
     reference cell counts as not settled**: it stays in the denominator as a miss, the log
     prints the count by band, and a row whose count is over 1 % of its pixels is not a reading
     of this done-when until the reference is fixed. At 4096 samples a face lit on one sample in
     32 has a relative standard error of about 0.09, so the darkest faces' misses are partly the
     reference's own; each row is also split by the reference mean's decile. **The rows:** at
     `VEIN_POSE` from a fresh table, 120 still frames, then the rig's `yaw 90 10 f`: of the
     pixels whose face was not in view in the 120 frames and lies within `r0`, on the first
     frame each face is in view **at least 90 % read within 10 % of the face's reference mean,
     and 99 % by the tenth frame after the yaw ends**. And the rig's flight at both poses at 0.5
     voxel a frame (30 voxels a second): of the entering pixels within `r0`, on their first
     frame in view **at least 90 % within 10 %**; at 2 voxels a frame (`Shift`) the same, read.
     The bands from `r0` to `R` and beyond `R` (the histogram's, at `R` 64 and 96) are read
     beside each row and not asserted. Each row with the preload off beside it, scale 1, lamp
     off and on, at `C_PRE` 256 and 1024; `up1.5` is read and not asserted. The prediction (a
     guess): the band within `r0` meets the bar at `C_PRE` 256 if its faces lit on under one
     sample in 16 carry about a tenth of its pixels, since such a face holds a relative standard
     error of about 0.24 at 256 samples and 0.12 at 1024 and lands within 10 % about a third and
     three fifths of the time; the decile split reads it. A miss goes to the owner in Phase 6 (h) with the table, split
     by cause: no reference, no cell (the sweep had not reached it, or the claim refused it, by
     cause) or a cell under `C_PRE`. **The asserted half** (lavapipe):
     `a_face_near_the_eye_is_claimed_lit_or_not` asserts, on a box with an emitter, a face 8
     voxels from the eye lit only through a one-voxel gap (dev records the geometry and its lit
     share of samples, under one in 16, from a CPU or GPU count), a face 8 voxels from the eye
     no emitter reaches (checked on the CPU through `World::raycast` against every emitter
     voxel), and a face of the same unlit kind 40 voxels from the eye, `r0` 24, `C_PRE` 256,
     scale 1, lamp off, after one sweep and 32 still frames facing away from all three: the two
     near faces hold cells of `C_PRE` samples, the unlit one's mean 0, the far unlit face holds
     no cell; and with `preload_claim_radius` 0 the gap-lit face holds a cell in under half of
     16 seeds (the test fails first; if it does not, the log says so and the rule's need rests
     on the level's reading). About 2 x 60 frames: about 6 s.
     **Amended 2026-11-05 (architect, for the owner), the rows on the stop-moving criterion.**
     The level rows' "within 10 % of the face's reference mean" is replaced by **still**: the
     face's cell mean, read back as the rig reads cells, through the display transform at the
     run's exposure (the header's amendment), moves by under `STILL_LEVEL` 1 level from the frame
     to the next; a pixel with no cell, or whose cell reopened, counts as moving. The numbers
     stand: **at least 90 % of the turn's pixels within `r0` still on the first frame each face
     is in view, and 99 % by the tenth frame after the yaw ends; on the walk at 0.5 voxel a frame,
     90 % of the entering pixels within `r0` still on their first frame**; `Shift` read. The
     no-reference rule no longer gates a row (the bar needs no reference), so the walk rows the
     log marked "not a reading" are readings. **The prediction, a guess.** By Phase 4's settle
     derivation a face is still when `n > 135 sqrt(k)`, `k` the samples its cell gains a frame
     in view, mostly from its own lighting pixels. On the first frame `n` is `C_PRE`: still at
     256 for `k` up to about 3, at 1024 up to about 57. A voxel face at 192 x 108 covers about
     230 lighting pixels at 8 voxels and 26 at 24 (0.0082 rad a pixel, a 90-degree field, a
     guess; `k` 57 at about 16 voxels, 3 at about 64), so on their first frame faces within about
     16 voxels are not still at 1024 and within about 64 not at 256, and their own pixels quiet
     them after `(135 sqrt(k) - C_PRE) / k` frames: about 8 and 4 at 8 voxels, 17 and 0 at 24
     voxels, at 256 and 1024. So at 1024 the tenth-frame 99 % is predicted met and the
     first-frame 90 % met where faces within about 16 voxels carry under a tenth of the pixels;
     at 256 the first frame is predicted missed and the tenth frame missed where faces between
     about 11 and 55 voxels (`k` 5 to 125, over ten frames to quiet) carry over 1 %. The
     `C_PRE` the bar picks is then a cost question (the Budgets). Each row is split by `k` band (1, 2 to 3, 4 to 57, 58
     and up) as well as by decile, so a miss names the near faces' share. **The reference
     becomes a reading:** each row prints, by decile, the share within 10 % of the reference
     beside the cell check's two-reference share at the same decile (its floor) and their ratio;
     a row at or over its floor is as near the truth as a 4096-sample reference can tell. The
     travel after the stop (levels from the first still frame to the reference) is printed by
     decile beside it. A miss goes to the owner in Phase 6 (h) with the tables.
  15. **Added 2026-11-04 (owner), the dig at arm's length and at range; amended 2026-11-04
     (architect, for the owner).** `a_near_dig_settles_from_the_preload_alone` asserts, on
     done-when 2's box with the camera 5 voxels from the dig, `F` 0 (Part 2 off) and the
     preload on, that the edit box's bricks are visited on the dig's frame and **at least 90 %
     of the hole's lit pixels read within 10 % of their settled mean by frame 16 after the dig**
     (16 samples a frame reach `C_PRE` 256 there; the first frame cannot be held without a burst
     of about 1900 faces times 256 samples, about 1.7 ms on the RTX by the per-sample figure, which
     the plan does not buy), and its `S90` at most 60 frames, Part 2's bar; with both off, `S90`
     over 60 (the test fails first). About 2 x 150 frames: about 15 s. On the level,
     `dig_box_readings` at both distances with the preload on and off, beside Part 2's rows: the
     5-voxel dig's share within 10 % at frames 1, 4 and 16 and its `S90` at most 60 (scale 1,
     lamp off, against the bar); the 100-voxel dig lies beyond `R` and stays Part 2's bar of 120.
     **Amended 2026-11-05 (architect, for the owner), the level half on `Q90`.** On the level the
     bar is **`Q90` at most 60 frames** for the 5-voxel dig and **at most 120** for the
     100-voxel dig, preload on (Phase 4's settle bar, with the derivation: predicted about 16 to
     32 and 47 to 60); `dig_box_readings` prints per row `Q90`, the still share at frames 1, 4,
     16, 60, 128 and 256, `S90` and mean90 as today (readings), the hole pixels' cell counts and
     samples a frame at p10, p50 and p90, and the travel after `Q90` (levels from `Q90` to the
     mean of frames 2048 to 2111, p50 and p90). Preload off is read beside it and not asserted
     (predicted about 30 at arm's length, so the preload's gain on the level dig is a reading,
     not a bar). The box test's asserts above stand.
  16. **Added 2026-11-04 (owner), the dark corner and the walk round a pillar.**
     `a_face_hidden_from_every_eye_is_settled_before_it_is_seen` asserts, on a box with an
     emitter and a pillar, and a lit face behind the pillar that no eye on the path sees until
     the path passes the pillar's edge (dev records the geometry and checks the hiding on the
     CPU through `World::raycast`), the camera moving along the path from 80 voxels out at 0.5
     voxel a frame (30 voxels a second), scale 1, lamp off, `C_PRE` 256: on the first frame any of
     the face's pixels is in view **at least 90 % of them read within 10 % of their settled
     mean**, and with `preload_radius` 0 under 50 % (the test fails first); the same path at 2
     voxels a frame (`Shift`, 120 voxels a second) **at least 90 % by the tenth frame in view**
     (on the box the budget does not bind: the face is within `R` about 24 frames before it shows,
     against a worst case of one sweep, 17 frames at `B` 128, plus 16 frames of fill). About 3 runs of 170 and 50 frames: about 22 s. On the level,
     `VEIN_CORNER` (dev picks it as the lit faces within 24 voxels of `VEIN_POSE`'s eye and above
     it in the lowest tenth of the settled means, records its box; the owner confirms or moves
     it in Phase 6's run 6): the camera walks the 60 voxels of the rig's path into `VEIN_POSE`
     at 0.5 voxel a frame with its heading held 180 degrees from the corner, stops, and turns
     with `yaw 180 20 f`; on every frame the corner is in view **at least 90 % of its pixels
     read within 10 % of their settled mean** (RTX, fresh table, scale 1, lamp off, a reading
     against a bar), the preload off beside it (expected to miss), and at `Shift` read.
     Amended 2026-11-05 (architect, for the owner): the corner is read against done-when 14's
     reference (cells of `C_CAP`, a pixel with none not settled), and the log records the frames
     the corner's faces lay within `r0` of the eye before its first frame in view, against the
     lead the preload needs (one sweep, 17 frames at `B` 128, plus 16 frames of fill); a miss
     with a shorter lead is the lead's, named so, not the claim rule's.
     **Amended 2026-11-05 (architect, for the owner), the corner on the stop-moving criterion.**
     `VEIN_CORNER` is still picked from the reference (its lowest tenth), and its bar becomes
     done-when 14's: **on every frame the corner is in view, at least 90 % of its pixels still**
     (the face's cell through the display transform, under 1 level to the next frame), preload
     off beside it, `Shift` read. By the derivation the corner's dim faces need no more samples
     than a lit wall's to be still (`D sigma` within 1.2 times), and its faces lay within `r0` 74
     to 124 frames before the first in view, so at `C_PRE` 256 or 1024 they enter view with
     their fill done; the prediction is met where the corner's faces gain under about 3 (256) or
     57 (1024) samples a frame in view. The share within 10 % of the reference (read: 0.000 to
     0.381) becomes the decile reading beside the two references' floor, as in done-when 14, with
     the travel after the stop.
  17. **Added 2026-11-04 (owner), the preload's cost, memory and show; amended 2026-11-04
     (architect, for the owner); amended 2026-11-05 (architect, for the owner).** The banner and
     the HUD show `preload R <R> r0 <r0> B <B> C <C_PRE>` or `preload off` (`--preload-claim
     <voxels>` sets `r0`), the HUD the bricks visited, faces listed, cells claimed by band and
     refused by cause, and samples of the frame (`hud.rs`'s banner test, no GPU, under 1 s); `--preload
     <voxels>|off` sets it and a free key (dev's choice, named in the banner) toggles it in the
     window; it is on by default at 64 until the owner's Phase 6 (h). `frame_bytes` reads
     done-when 5's figure plus the preload list and its stats (dev records the bytes), asserted
     in `upscale.rs`. The cost on the RTX, quiet where the machine allows: **at rest after the
     fill at most 0.01 ms (the trickle); in the flight at walking speed at most 0.15 ms**; at
     `Shift` and on a fresh table read; the Cezanne a reading against the lighting line. The
     still hall's native bar (done-when 8) and the still reopens' 0 (done-when 9) hold with the
     preload on; the occupancy after four flights stays under 0.25 at both poses with 0 misses;
     and at the hall the misses with the preload on read no more than with it off.

### Phase 5: The lamp-bounce channel
- **Owner skill:** dev
- **What:** the probe volume redone as ADR-0043's lamp channel, and the emissive part of today's
  probe logic removed.
- **The channel:**
  - **Lamp-only rays.** The probe rays shade their hits with the headlamp term alone: no
    emissive sample or shadow ray, no hit emission, no previous-frame irradiance. `probe_lamp`
    and the `STEADY_NO_LAMP` bit go; the emission-scale read and the emission-change window go
    (a lamp switch-on still opens the window, as the channel's fast fill from zero).
  - **The reach, from a reading.** Dev reads, on `cache_box` and at the level's spawn, the gather's
    irradiance at a lit wall's hit as the lamp-to-probe distance grows, against the wall's direct
    irradiance from the lamp, and sets `probes::lamp_reach` where the bounce falls under 1 % of the
    direct, rounded up to whole probe spacings. The volume's side (`PROBES_PER_AXIS`, a setting
    now) is the smallest power of two whose half-side covers the reach (the plan's guess: 16, so
    25.6 m and an eightfold smaller volume). Only probes within the reach of the lamp are listed;
    the others hold zero and gather as zero.
  - **The blend.** A running mean over visits that freezes at `probe_freeze` visits (Plan 0051's
    A1, now the only blend; `HYSTERESIS`, `YOUNG_UPDATES` and `YOUNG_HYSTERESIS` go), restarted
    by a window, an edit touching the probe, a scroll bringing a new probe into the slot, or the
    lamp moving. While the lamp moves, the probes in reach blend at a fixed `h_move`, swept 0.5,
    0.8 and 0.9 against the flight's `K` with the lamp on and the frames to 90 % after a lamp
    step; the default is the reading's.
  - **Lamp off, nothing runs.** With the lamp off no probe pass is encoded and the gather is
    skipped; the volume is allocated on demand as today (Plan 0052). `Lighting::indirect`
    becomes `lamp_bounce` (on by default); the fly-through gains `--lamp-bounce on|off`, `F8`
    toggles it, the banner names `lamp bounce`, and `--indirect on|off` stays as an alias until
    Plan 0056 closes (its `E` arm and Phase 4 fallback pass `--indirect off`).
  - **Kept** (ADR-0043): the tagged scrolling volume, the maps, the gather's weights and bias,
    the inside flag and skip (`skip_inside_probes` becomes the rule), the edit-touch list, the
    ray set, on-demand allocation.
- **The readings:** the channel's cost at rest and in flight with the lamp on, both adapters,
  against the spike's `off` and `b --indirect off` rows; `K` in the flight at each `h_move`; the
  frames to 90 % after a lamp step at each `h_move`; the wander at both poses with the lamp on;
  `frame_bytes` at the new side.
- **Files touched:** `crates/vxe-render/src/shaders/probe_update.wgsl`, `probe_gather.wgsl`,
  `crates/vxe-render/src/probes.rs`, `marcher.rs`, `apps/flythrough/src/main.rs`, `hud.rs`,
  `apps/flythrough/examples/hero.rs`, `apps/flythrough/tests/indirect.rs`, `golden.rs` (the two
  indirect goldens), `settle.rs`, `.config/nextest.toml`, `docs/plans/logs/0057.md`.
- **Done when:**
  1. `the_channel_holds_the_lamps_bounce_only` asserts, on a closed box with an emitter and the
     lamp, that with the lamp off every probe texel reads exactly 0 over 64 frames, and with the
     lamp on the texels of the probes in reach read within 10 % of a run with the emitter's
     emission set to 0. About 150 frames: about 16 s.
  2. `the_lamp_furnace_reads_its_albedo_times_the_direct_mean` replaces
     `the_furnace_box_reads_pi_of_indirect_irradiance`: a closed box of albedo 0.5 lit by the lamp
     at its centre, the centre probe's tile zeroed to read the rays' direct mean `D`; after 256
     frames the texel mean reads within 4 % of `0.5 D` and every texel within 7 %. This is M2's
     headless proof restated. On the soak list, as its predecessor was.
  3. `a_still_lamp_freezes_the_channel` asserts, at `probe_freeze` 8, that after 8 visits with the
     lamp still the probes' texels and state are bit-identical over 64 frames. About 16 s.
  4. `a_lamp_step_moves_the_probes_in_reach_only` asserts, with the origin pinned so a probe lies
     beyond the reach, that after the lamp moves the probes in reach change within 8 frames and
     the probe beyond stays bit-identical, and that the ones in reach freeze again once the lamp
     is still. About 16 s.
  5. `the_lamp_off_encodes_no_probe_pass` asserts that with the lamp off the probe passes'
     timers read 0 and the frame's probe dispatch count is 0 over 16 frames. Under 5 s.
  6. The emissive probe tests are removed and named in the log: `the_furnace_box_reads_pi_of_indirect_irradiance`,
     `an_emission_step_drops_the_probe_history`, `a_pulse_at_its_mean_leaves_the_probes_still`,
     `indirect_golden_shows_the_vein_s_bounce_light`, `fsr2_carries_the_indirect_golden_s_bounce_light`;
     `a_hole_in_the_wall_lights_the_dark_box_and_filling_it_darkens_it_again` and
     `no_light_leaks_through_a_two_voxel_wall` are re-pointed at the lamp. The soak list in
     `.config/nextest.toml` follows, and `check-test-tiers` exits 0.
  7. The readings are in the log. The predictions, RTX at the spawn at rest, lamp on: the probe
     update under 0.2 ms (the spike's A read 0.016 frozen) and the whole frame under the spike's
     `b --indirect off` 3.255 ms plus 0.3; Cezanne under 21.053 plus 2.0. `frame_bytes` falls by
     seven eighths of the atlases and state (about 44 MB of the 58 837 024 B, arithmetic) at a
     side of 16.
  8. **Added 2026-11-04 (architect, for the owner), the still image holds through the
     channel.** On Phase 5's tip the hall's native row (lamp off, so no probe pass runs) meets
     Phase 4's done-when 8 bar again, and `still_reopen_readings` reads 0 at both poses with the
     lamp on and still (the lamp channel uploads nothing that opens the guard's window). With
     the lamp on and still at the vein, the composite reads bit-identical over 64 frames once
     `probe_freeze` visits have passed (scale 1, the rig, RTX reading; lavapipe asserts it
     through done-when 3's frozen texels). A miss goes to the owner in Phase 6 (a).

### Phase 6: The owner's window, with digs
- **Owner skill:** human
- **What:** the release fly-through on the RTX, one sitting, with the v0.41.0 binary beside it
  for comparison (built from main at 3d4f2ad as `target/release/flythrough-v0.41`, by the
  orchestrator) and Phase 2 to 5's tables and the hall's difference images at hand. Every toggle
  shows in the banner and the HUD.

  ```
  cargo build --release -p flythrough

  # 1. The hall from a fresh start: count to five, then a still minute, a fast turn, a slow
  #    turn, a dig at arm's length and one at range; then the lamp on, move, and off.
  target/release/flythrough --source showcase --shot hall --exposure 8 --hud

  # 2. The same in v0.41.0 for the comparison, lamp off.
  target/release/flythrough-v0.41 --source showcase --shot hall --exposure 8 --hud --steady off

  # 3. The level, auto exposure: still, the lamp toggle both ways, a fast turn, a flight
  #    down the passage and back, a dig at arm's length and one down the passage, at each pose.
  target/release/flythrough --level --seed 1 --pose vein --hud
  target/release/flythrough --level --seed 1 --pose patch --hud

  # 4. The lamp bounce off, to judge what the channel adds and whether it lags.
  target/release/flythrough --level --seed 1 --pose vein --hud --lamp-bounce off

  # 5. At arm's length of a lit wall: the step at a voxel edge, nearest only.

  # 6. (amended 2026-11-04, owner; steps amended the same day, architect) The camera-radius
  #    preload, on and off by its key, each step once with it on and once off: at the vein,
  #    (i) walk in from down the passage and turn fast to the dark ceiling corner (point at it,
  #    to confirm or move VEIN_CORNER); (ii) walk round a pillar or a passage bend at walking
  #    speed, then again with Shift held, watching the faces that come out from behind it;
  #    (iii) stand still, then a fast 180-degree turn; (iv) a dig at arm's length, then one
  #    about 100 voxels down the passage; (v) the same walk with --preload 96, for R. Once at
  #    auto exposure and once with --exposure fixed, to tell the table's settle from the eye
  #    adaptation's (ADR-0033). Then the hall with the preload on and off, the HUD's claimed
  #    and refused cells in view, for the table's size.
  target/release/flythrough --level --seed 1 --pose vein --hud
  target/release/flythrough --level --seed 1 --pose vein --hud --exposure 8
  target/release/flythrough --level --seed 1 --pose vein --hud --exposure 8 --preload 96
  target/release/flythrough --source showcase --shot hall --exposure 8 --hud
  ```
- **Files touched:** this plan's log.
- **Done when:** the log records the owner's words on: (a) whether anything moves at rest, hall
  and level, with Phase 4's still-hall class table (native and `up`, Part 1 and the phase's
  tip) and the still reopens at hand (amended 2026-11-04); if the drift left is the uncapped
  cells' settle (class 3), whether that settle is acceptable is the owner's call here, and the
  `up` drift over native is judged as Plan 0056's; (b) the fast turn, against v0.41.0 and against the spike's session, and the small
  yellow dots on creases while turning (Phase 4's verdict on the own-face read and the rig's
  fast-yaw row, against v0.41.0's, beside it); (c) the
  dug hole at arm's length and at range, in frames or seconds by eye, against Phase 4's bar (60
  and 120 frames); amended 2026-11-05 (architect, for the owner): with done-when 15's `Q90`
  (when the hole stops moving), `S90` (when it reaches its late mean) and the travel after
  `Q90` at hand, so the owner says whether the hole still visibly "loads" after it stops, and
  whether the slow crawl under a level a frame shows; (d) the lamp bounce's look and lag,
  and whether `lamp_bounce` stays on by default; (e) the step at voxel edges; (f) what remains
  visible, by run and pose; (h) (amended 2026-11-04, owner; amended the same day, architect)
  run 6: whether the light still "loads" in the dark corner, round the pillar at walking speed
  and at Shift, after a fast turn and in a dig at arm's length and at range, with the preload on
  against off, with done-whens 12 to 17's tables at hand; whether the preload is on by default
  and at which `R` (64 or 96); (amended 2026-11-05, architect, for the owner) whether the light
  within `r0` (24 by default, or what done-when 17's cost picked) reads "loaded" in the dark
  corner and after a fast turn, and whether the dim faces from `r0` to `R` that the 8-probe rule
  still refuses are visible, with done-when 12's lit-share table and done-when 14's bands and
  deciles at hand, so a wider `r0` (its cost by the Budgets' `r0^2` arithmetic) or a larger
  `C_PRE` near the eye for the darkest faces is the owner's call, not raised unasked;
  (amended 2026-11-05, architect, for the owner) with done-whens 14 and 16's still shares by `k`
  band and decile, the reference shares beside the two references' floor, and the travel after
  the stop: whether a face that has stopped moving but sits off its neighbours (the static grain
  of a cell of a few hundred samples, up to a few tens of percent on the dim deciles) or keeps
  a slow crawl is visible in the dark corner, which is the trigger for the Followups' stop by a
  cell's spread; and which `C_PRE` (256 or 1024) at its cost; whether its off-screen cost (the sweep and fill of faces the view
  does not show, done-when 17's figures) is acceptable against the rule that the cost matches
  what is on screen; whether the faces beyond `R` that still settle at their footprint are
  visible; and, since the hall's table sits near or over `PRE_LOAD_MAX` (0.185 native, 0.260
  up), whether a larger table is wanted there; (g) whether the default is accepted. A "no" on (g) names the run and
  goes to the architect with the tables. The plan closes with that record.
  **Amended 2026-11-05 (owner):** recorded (`logs/0057.md`, Phase 6): no flicker on turning or
  moving; grain and dark faces everywhere else; the direction questioned. The plan no longer
  closes on this record: Phase 7 fixes the three faults and Phase 8 is the second window.

### Phase 7: The grain, the young cells and the table (amended 2026-11-05, owner)
- **Owner skill:** dev
- **What:** the three faults the Phase 6 window showed, inside the table's design: the table
  sized and aged for the window's resolution, a count-weighted blend over each seen face's
  coplanar neighbours, and the same blend widened where a young face's pool is thin. **The vein
  strip is rendered first, on the current tree, as the before picture, before any mechanism.**
  Nothing here touches the stability the owner called met: every input of the blend is a cell
  sum, so a still world reads bit-identical frames, and the still-hall and still-reopen bars
  (Phase 4's done-whens 8 and 9) are re-read on the tip.
- **What the window showed, attributed from the HUD** (`logs/0057.md`, Phase 6; RTX, the
  owner's 2530 x 1378 from 1687 x 919, FSR 2 at 1.5, half-resolution lighting, 843 x 459):
  - **The hall** held 993 304 of 2^20 cells and missed 194 079 lighting pixels a frame, about
    half the view; a missed pixel shows its raw sample through the fallback, so most of the
    hall's grain in `owner-step6d` is the table's, not the estimator's. The headless hall read
    194 194 cells at 192 x 108 (Phase 4, the hall's census): the demand grows with the lighting
    grid, about 19 times larger in the window, and with the path, since no cell inside the
    light radius (256 voxels) leaves before a chunk change sweeps it.
  - **The patch** held 269 051 (0.2566) with 1519 misses: over `PRE_LOAD_MAX`, so the
    preload listed 4 bricks, claimed 0 and refused 364 for load. With the preload claiming
    nothing, every face the view had not filled is young, which is the owner's "dark patches
    everywhere ... it settles slowly" and "preload doesnt really change anything".
  - **The grain in a filled view** is each face's estimate on its own: by Phase 4's settle
    derivation a face of `n` samples sits about `D sigma / (2.2 sqrt(n))` display levels off its
    truth (rms; `D sigma` about 180 from a lit wall to a face lit on one sample in 32), so **5.1
    levels at `C_PRE` 256, 2.6 at 1024 and 1.3 at `C_CAP` 4096**, independently per face:
    salt and pepper at voxel scale even when every cell is capped, and blotches where the
    young faces read 0 (a face lit on one sample in 32 shows 0 until its first lit sample, and
    a mean of few samples of a skewed variable reads under its truth most of the time).
- **Part 0, the before picture (first, on the current tree).**
  - **The vein strip** (`light_strip` in `apps/flythrough/tests/strip.rs`, which Plan 0056 closed
    and this plan now extends; RTX through `VXE_TEST_ADAPTER=rtx`, ignored, a reading): at
    `VEIN_POSE`, from a fresh table, the owner's window size (2530 x 1378 from 1687 x 919, FSR
    2 at 1.5) and at scale 1 (1687 x 919 native, the pixel figures' reference), half-resolution
    lighting, the pose's fixed exposure, lamp off, preload on, nothing moving: the frames **1,
    4, 16, 64, 256, 1024 and 4096** saved as PNG under `live/strip-0057/<commit>/`
    (gitignored), a difference image of each against frame 4096 (amplified, dev's gain, named
    in the file), and per frame **the share of pixels within 1, 2 and 4 display levels of
    frame 4096** (the display transform at the fixed exposure, `rigs.rs`'s `tone_bytes`
    arithmetic). The hall is read the same way after the vein (the owner: "Only the vein" first,
    "the hall later").
  - **The owner's B strip.** The owner's note 6c ("take a screenshot strip after multiple press
    of B button ... compare each screen") names the fly-through's `B` key, which is the edit
    bench (`DigInput::Bench`: carves of radius 1 m within 10 m of the eye, seeded by the press
    count), not a settle. So the strip continues from frame 4096: four bench inputs with the
    window's seeds (`bench_runs` 0 to 3), each followed by frames +1, +4, +16, +64 and +256
    saved beside a difference image against the frame before the press, and per press the
    share of the carved region's pixels within 2 levels of the frame +256.
  - **The grain figures** from the cell dump at each strip frame (the rig reads cells as Phase
    4's do), over the faces with a pixel in view, each face's displayed emissive value `D_f`
    (the value the temporal pass reads, through the display transform at the fixed exposure):
    **`G_plane`**, p50 and p90 over faces with at least two in-plane edge neighbours in the
    dump of `|D_f - mean(D_n)|`, the neighbours as Part B defines them; and **`G_noise`**, p50
    and p90 of `|D_f(seed A) - D_f(seed B)| / sqrt 2` between two runs of the same strip at two
    light seeds, which carries the estimator's noise with the true light cancelled.
    `G_noise` is the bar (it cannot be met by a true gradient); `G_plane` is the figure the
    owner sees, read beside it.
  - **The table at the window's resolution:** the runner's step 1 and step 3 paths scripted
    headless (dev transcribes them from `live/session1.sh`: the hall from a fresh start, 600
    still frames, the rig's `yaw 90 10 f` four times round, a slow `yaw 360 600 f`, the rig's
    flight; the vein and patch the same with the flight down the passage and back), at the
    window's size, preload on: per 60 frames the occupied cells, the misses, the preload's
    claims and refusals by cause, and the peak. This is the input to Part A's size rule. About
    a minute a run on the RTX.
- **Part A, the table (fault 3; built first, because a missed pixel shows its raw sample and
  would confound every grain figure after it).**
  - **The probe count, from a CPU census.** `the_table_hash_spreads_a_levels_keys` gains the
    key sets of Part 0's hall and vein runs (dumped at their peak) inserted at a load of 0.25
    of 2^22 through `marcher::cache_slot`; `CACHE_PROBES` is **the smallest of 4, 8 and 16 that
    leaves no key without a slot** (the hall's census at 2^20 already left 66 and 256 of its
    194 194 and 272 838 keys without a slot at 4 probes and loads 0.185 and 0.260, so 4 is
    expected to fail). None meeting it comes back to the architect with the run lengths.
  - **The size: 2^22 by default**, a `Lighting` setting as today. After the age-out below is
    built, Part 0's runs are repeated: if the peak occupied cells at both poses and the hall
    stay under the eviction mark of 2^21 (3/16 of it, 393 216 cells), the default becomes 2^21;
    if any run's peak is over the mark of 2^22 (786 432) with nothing left to evict (every cell
    seen within `AGE_MIN` or within `R`), it comes back to the architect with the table. The
    cell is at most **12 words, 48 B** (today's ten, plus `seen` and `blend`, Part B; dev may
    pack `seen` into the stamp word if Part 2's listing still reads correctly, and says so):
    **201 326 592 B at 2^22, + 159 383 552 B over today's 41 943 040**; 100 663 296 B at 2^21
    (+ 58 720 256), arithmetic.
  - **The age-out.** The light pass stores the frame into a cell's `seen` word on its first
    touch of the frame (the compare-exchange that also lists it for Part B), and Part B's
    pass stores it into every neighbour cell it reads, so a face only a seen face's kernel
    reads is not aged out from under it. **The eviction pass** runs when the occupancy crosses
    `EVICT_LOAD` (3/16 of the size) upward, and again only after it has grown by 1/32 of the
    size since the last pass while still over, so a still camera over the mark runs it never;
    it clears every cell whose voxel lies beyond `R` (the preload's radius, so the preload's
    cells are never aged out) and whose `seen` is older than `AGE` frames (600, ten seconds at
    60 fps; halved by each pass that leaves the load over the mark, down to `AGE_MIN` 60, and
    reset to 600 once under). It reuses the chunk-change sweep's dispatch, and both walk **at
    most 2^20 cells a frame** (a quarter of the table a frame at 2^22), so no frame pays a whole
    table. A cleared cell leaves a hole in a probe run as the sweep's already do: dev confirms
    in the log that `cache_claim` reads every probe for the key before it takes a free slot, and
    the census reads 0 duplicate keys after an eviction. An eviction is not a world change and
    does not open the guard's window.
  - **The load cap, a share of the size.** `PRE_LOAD_MAX` stays a quarter of the table
    (`PRELOAD_LOAD_DIVISOR` 4), now 1 048 576 cells at 2^22, above the eviction mark, so a
    view whose working set sits under the mark never refuses the preload; the miss guard (no
    claim the frame after a miss) stands.
  - The HUD's table row gains `evicted <cells>` and the pass count; the banner names
    `table 2^22 probes <n>`.
- **Part B, the blend (fault 1).** A world-space interpolation over the table, computed once a
  seen cell a frame, not a screen filter: the inputs are cell sums, which do not move while the
  world and the light do not.
  - **The kernel.** For a cell of face `(v, axis, sign)` with tangent axes `u` and `w`, each
    offset `(du, dw)` of the 3 x 3 around it looks for one neighbour face of the **same axis and
    sign** (a face of another orientation is never a candidate, so no light crosses a crease):
    first the coplanar voxel `v + du u + dw w`, then the one a voxel toward the air
    (`+ sign axis`), then the one a voxel into the rock (`- sign axis`), the first with a cell
    taken; the two stepped candidates follow a voxel staircase, which is how a cave wall at an
    angle is made, and weigh half. The weights are the binomial tent (4 at the centre, 2 at an
    edge, 1 at a corner) times the cell's count (capped at `C_CAP`), so the blended mean is
    `sum(t_i s_i) / sum(t_i n_i)` over the present cells, `s_i` the sums: a pool of the faces'
    samples, not a mean of their means. No material test: the table holds irradiance with the
    albedo divided out (ADR-0018; the composite multiplies it back), so a material boundary on
    one wall is not a light boundary.
  - **What it costs the look.** Where every neighbour is coplanar and equally sampled, the
    tent's variance factor is `sum t^2 / (sum t)^2 = 36 / 256`, so a face's noise falls to
    **0.375 of itself**: 1.9 levels at 256 samples, 0.96 at 1024, 0.48 at 4096 (the
    derivation above; a guess the readings replace). A true edge is softened by one face each
    side: at a hard shadow edge between equal counts the dark face beside it reads a quarter of
    the lit side and the lit face three quarters, and a face two away reads its own side. A
    one-voxel step in a lit floor blends with its neighbours at half weight; a pit one voxel
    wide and deep is locally the same geometry as a step, so its bottom takes part of the rim's
    light (a reading, the Risks). The voxel-edge step the owner accepted with nearest (Plan
    0051) becomes a one-face ramp: the owner's to judge in Phase 8.
  - **The pass.** The blend list: the light pass's first touch of a cell in the frame (the
    `seen` compare-exchange) appends its index, at most one entry a lighting pixel, so the list
    is sized from the lighting grid and never drops. `cache_blend` (new, in `cache_fill.wgsl`
    or its own file, dev's choice) runs after the light pass, Part 2's fill and the preload, and
    before the temporal pass: one lane per listed cell, the kernel's lookups through
    `cache_slot`, the result into the cell's `blend` word (a shared-exponent RGB or dev's
    packing, its error stated through the display transform, under a quarter of a level). The
    temporal pass reads `blend` where it read the mean; a pixel whose own cell is absent (a
    miss, which Part A brings to 0) takes the fallback as today. `--blend on|off` and a free key
    toggle it in the window (dev's choice, named in the banner, `blend 3x3|5x5|off` in the
    HUD), on by default.
  - **At rest.** The pass recomputes a bit-identical value each frame for a still world. If its
    cost at rest reads over 0.02 ms on the RTX at the window's size (done-when 8), dev adds a
    `final` bit (set when the cell and every cell its kernel read are at `C_CAP`; a final
    cell's lane returns at once; cleared by an edit box's clear dilated by the kernel's reach of
    two voxels plus one, by the guard's reopen on the reopened cell's kernel neighbours, and by
    the sweep or the age-out of any cell), and says so in the log.
- **Part C, the young cells (fault 2).** The blend already outweighs a young cell by its wall's
  samples: a cell of 4 samples among eight neighbours of 1024 contributes 16 of about 12 300
  weighted samples, so it reads its wall's light from its first frame. What it cannot do is
  help a wall that is young everywhere, the faces beyond `R` and the faces a refused preload
  left (fault 3's), which pool few samples. **The widening:** where the 3 x 3 pool holds under
  `POOL_MIN` samples (`sum n_i`, 16 384 a guess, swept 4096, 16 384 and 65 536; the default the
  smallest that meets done-when 5), the lane adds the 5 x 5 ring (the binomial 1-4-6-4-1 tent,
  the same candidates and half weight for the stepped ones), a variance factor of `4900 /
  65536`, **0.273** for an equally sampled wall. Nothing else: no prior store, no seed from the
  screen. With Part A the preload claims again, so within `R` the faces hold `C_PRE` before the
  view reaches them, and the blend is what turns those 256 samples a face into a wall's pool.
- **Order:** Part 0 (the strips, the grain figures and the window-resolution table runs on the
  current tree, committed to the log as the before rows), then Part A with its tests and the
  table runs re-read, then Part B with its tests, then Part C, then the end readings: the same
  strips and figures on the tip, beside the before rows.
- **Files touched:** `crates/vxe-render/src/shaders/cache_fill.wgsl` (or a new `cache_blend.wgsl`),
  `light.wgsl` (the `seen` touch and the list), `temporal.wgsl` (the `blend` read),
  `cache_sweep.wgsl` (the age-out and the slices), `march.wgsl` (`CACHE_PROBES`, the cell
  layout), `crates/vxe-render/src/marcher.rs`, `lib.rs`, `apps/flythrough/src/main.rs`, `hud.rs`,
  `apps/flythrough/tests/strip.rs`, `settle.rs`, `support/settle.rs`, `lights.rs`, `upscale.rs`
  (`frame_bytes`), the golden baselines if the blend moves them (re-blessed once on lavapipe,
  the difference in the log), `.config/nextest.toml`, `docs/plans/logs/0057.md`.
- **Done when:**
  1. **The before rows.** The log holds Part 0's tables on the current tree (the commit
     named): the vein strip's shares within 1, 2 and 4 levels of frame 4096 at each of the
     seven frames, at the window's size and at scale 1; the B strip's shares per press; `G_plane`
     and `G_noise` at each frame; the window-resolution table runs (occupancy by 60 frames, the
     peak, the misses, the claims and refusals); and the strips' PNG paths. Nothing is built
     before these rows are committed.
  2. **The table, asserted** (lavapipe): `an_unseen_far_cell_ages_out_and_a_near_one_stays`
     asserts, on a box with three views of about 6000 faces each beyond `R` (test: 16 voxels)
     and a few hundred within it (dev records the geometry and the counts), `CACHE_CELLS`
     16 384 (test; its mark 3072), `AGE` 8 and `AGE_MIN` 4 (test), each view held 32 frames in
     turn: after the third view's 32 frames the occupied cells read at most the third view's
     cells plus those within `R` plus those its kernels read (the first two views' cells beyond
     `R` cleared), every cell within `R` keeps its count, and the census reads 0 duplicate keys;
     and with the age-out disabled (test-only) the third view counts misses, about 18 000 faces
     in 16 384 cells (the test fails first). About 3 x 32 frames: about 6 s.
     `the_table_hash_spreads_a_levels_keys` asserts the probe count's census (no key without a
     slot at 0.25 of 2^22 at the chosen `CACHE_PROBES`; under 5 s, no GPU).
  3. **The table on the window, readings against bars** (RTX, the window's size, the scripted
     paths of Part 0): **0 misses in every frame at the hall with the preload on**, and **at the
     vein and the patch `refused load` 0 in every frame and claims within `r0` above 0 on the
     fresh start**; the peak occupancy, the cells evicted and the eviction passes per run
     printed; the size rule's verdict (2^21 or 2^22) one line in the log. A miss above 0 at the
     hall comes back to the architect with the census.
  4. **The grain** (fault 1). **Asserted** (lavapipe): `the_blend_stops_at_a_crease_and_keeps_a_shadow_edge`,
     on a box with an L crease whose one wall an emitter lights and whose other wall no emitter
     reaches (checked on the CPU through `World::raycast` against every emitter voxel), and a
     flat wall half under a slab that hides every emitter, scale 1, lamp off, `C_CAP` 1024, the
     widening off (test): after every cell in view is capped, every cell of the unlit wall
     reads `blend` exactly 0, the row along the crease included; on the flat wall the face two
     from the shadow edge on the dark side reads exactly 0 and the face beside the edge between
     0.15 and 0.35 of the lit face beside it (the arithmetic: 0.25 at equal counts); about 4 s.
     `a_blend_quiets_a_lit_wall`, on `cache_box` at `C_CAP` 256, two light seeds: the per-face
     spread `G_noise` over a lit wall falls to at most half with the blend on (the arithmetic:
     0.375), and with the blend off it reads the unblended spread (the test fails first); about
     2 x 4 s. `a_still_blend_is_bit_identical`: 64 still frames after every cell is capped read
     every `blend` word and every pixel bit-identical; about 4 s. **On the level, readings
     against bars** (RTX, the window's size, frame 4096 of the vein strip and of the hall's):
     **`G_noise` p50 at most 1 level (`STILL_LEVEL`) and p90 at most 2 levels**, against the
     before rows; `G_plane` p90 at most half its before row. The prediction, a guess: met where
     under a tenth of the faces in view hold under 1024 samples at frame 4096 (a face at 1024
     reads 0.96 levels rms after the blend, so p90 about 1.6); the rows split by the faces'
     count band (under 256, 256 to 1023, 1024 to 4095, capped) so a miss names its band.
  5. **The young cells** (fault 2). **Asserted** (lavapipe): `a_young_face_reads_its_walls_light`,
     on `cache_box` with a lit wall of capped cells (`C_CAP` 1024) and one face's cell cleared
     by a test-only hook (an edit box would clear its neighbours too): on the next frame the
     face's displayed emissive lies within 2 levels of its eight neighbours' blended mean, and
     with the blend off it does not (the test fails first); about 4 s. **On the level, readings
     against bars** (RTX, the window's size, preload on, fixed exposure, lamp off): the vein
     strip's **share of pixels within 4 levels of frame 4096 is at least 0.9 by frame 120** at
     scale 1 (two counts at 60 fps, the dig's bar at range); and at `VEIN_POSE` from a fresh
     table, 120 still frames then the rig's `yaw 90 10 f` (done-when 14's row), of the pixels
     whose face was not in view in the 120 frames and lies within `R`, **at least 90 % within 4
     levels of the same pixel at a settled frame (the view held 4096 frames after the yaw) on
     their first frame in view, and 90 % within 2 levels by the 16th frame after the yaw ends**;
     beyond `R` read, not asserted (the baked design's band). The prediction, a guess: within
     `R` a face enters view with `C_PRE` 256 samples, 1.9 levels rms after the blend against
     about 0.5 settled, so about 0.96 of pixels within 4 levels; by the 16th frame its pixels
     add about 50 a frame at arm's length, about 1000 in all, so about 0.93 within 2. Each row
     beside its before row and the blend off; `POOL_MIN`'s sweep in the same table.
  6. **The B strip after.** Per bench press, the carved region's share within 2 levels of frame
     +256 at frames +1, +4, +16 and +64, beside the before rows, and the 5-voxel dig's `Q90`
     (done-when 15's level row) re-read: at most 60 frames, as before.
  7. **The still image holds.** On Phase 7's tip, Phase 4's done-when 8 (the hall native one
     frame mean at most 0.005, no pixel over 8 levels; the drift under 0.01) and done-when 9
     (`still_reopen_readings` 0 at 256, 512, 768 and 1024 at both poses) read again with the
     blend and the age-out on, and the rig's fast yaw row (`the_fast_yaw_excess_by_table_read`)
     is read beside Phase 4's. A miss of either bar is a defect of this phase, not a reading.
  8. **The cost and the memory.** On the RTX at the window's size, quiet where the machine
     allows, at rest and walking at both poses: `cache_blend`'s slot, the light pass's excess
     over Phase 6's tree (the `seen` touch and the list), the age-out's slices on the frames
     they run and the preload's cost. **The bar: Phase 7's passes add at most 0.25 ms to the
     frame at rest and walking** (the guess: 0.05 to 0.15 ms for the blend, a few hundredths
     for the touch), and **at rest the blend pass at most 0.02 ms or the `final` bit built**;
     a frame with a sweep or eviction slice read, not barred (the guess: about 0.1 to 0.15 ms
     a slice of 2^20 cells). The Cezanne read against `docs/nfr.md` section 2's 6.0 ms lighting
     line. `frame_bytes` in `upscale.rs` asserts Phase 6's tree's figure plus the table's delta
     at the chosen size (+ 159 383 552 B at 2^22, + 58 720 256 at 2^21) plus the blend list, the
     arithmetic in the test's comment; under 5 s.
  9. The banner and the HUD show `table <cells> probes <n>`, `evicted`, and `blend
     3x3|5x5|off` (`hud.rs`'s banner test, no GPU, under 1 s); the toggle key works in the
     window and the HUD names the mode.

### Phase 8: The owner's second window (amended 2026-11-05, owner)
- **Owner skill:** human
- **What:** Phase 6's runner steps (`live/session1.sh`), the orchestrator rebuilding it on Phase
  7's tip, with Phase 6's binary beside it (built from the lane's tree before Phase 7 as
  `target/release/flythrough-p6`) for the comparison, the blend's toggle key in both runs, and
  Phase 7's strips at hand: the vein strip and the B strip before and after, their difference
  images, and the grain, young-cell and table tables. The owner's step 6c is run as the owner
  asked it: in the window, press `B` several times at the vein and compare the screens.
- **Files touched:** this plan's log.
- **Done when:** the log records the owner's words, with the strips beside, on: (a) whether
  anything moves at rest, hall and level; (b) whether turning and moving still show no flicker
  (the stability Phase 6 met must hold); (c) the dark patches on faces not looked at when the
  scene loaded (step 1) and in the caves while moving (steps 3 and 3b): gone, or how long they
  take; (d) the grain at rest at the hall and the vein (steps 1 and 6d), blend on against off;
  (e) the distant caves (step 5), which beyond `R` are Part C's widening only and the parallel
  baked design's to fill; (f) the preload: whether it now changes anything at the vein and the
  hall (steps 6 to 6d), with the HUD's claims and refusals in view, and the `B` presses (6c);
  (g) the blend's look: the voxel step and the shadow edges softened by one face, against the
  nearest read the owner kept in Plan 0051, and whether a pit's lit rim shows; (h) the table's
  memory at the size Phase 7 chose (+ 159 383 552 B at 2^22) against `docs/nfr.md` section 3's
  1.5 GB line; (i) whether the default is accepted, or the direction moves to the baked
  per-chunk design. A "no" on (i) names the step and goes to the architect with the tables,
  beside the parallel design. The plan closes with that record.

## Data shapes

```rust
// illustrative: vxe-render
pub struct Lighting {
    // ... the lamp, candidates, shadow walk, filter, exposure, tone map as today ...
    pub table_cells: u32,          // 2^20 by default; a power of two
    pub fill_samples: u32,         // F: samples a frame per reopened cell in view; 0 off (if built)
    pub fill_until: u32,           // C_FILL: a reopened cell leaves the list at this count (if built)
    pub own_face_read: bool,       // the composite reads a pixel's own cell (if built)
    pub preload_radius: u32,       // R voxels of the brick sweep around the eye; 64 default, 0 off
    pub preload_bricks: u32,       // B bricks the sweep visits a frame (B / 16 at rest once filled)
    pub preload_until: u32,        // C_PRE: a listed face's cell is filled to this count
    pub preload_samples: u32,      // S_PRE: the frame's cap on the preload's samples
    pub lamp_bounce: bool,         // the probe channel; on by default, nothing runs lamp-off
    pub probe_freeze: u32,         // visits to converge; the only blend
    pub probe_side: u32,           // PROBES_PER_AXIS, from the reach
    // amended 2026-11-05 (owner), Phase 7:
    pub blend: Blend,              // Off | Tent3 | Tent3Widened { pool_min: u32 }; widened by default
    pub evict_age: u32,            // AGE: frames unseen before a cell beyond R may go; 600
}
```

```wgsl
// illustrative: a cell is 10 u32s, table[slot * 10 + i]
// 0..1 key (voxel xyz and face, valid bit)   2 count   3..5 sums Q17.15
// 6..7 kept emitter (voxel and face)          8 kept luminance (f32 bits)   9 frame stamp
// Phase 7 (amended 2026-11-05): 10 seen (the last frame a pixel or a kernel read it)   11 blend (packed RGB)
```

## Budgets

Base figures are Plan 0051's cost tables (`logs/0051.md`, Phases 2 and 3), the level's spawn,
1080p from 720p, half tier, lamp on, **loaded (indicative), not quiet**. `off` is today
(v0.41.0's default); `b --indirect off` is the verdict's arm as spiked.

| `nfr.md` figure | Base (read, indicative) | Expected | How the close measures it |
|---|---|---|---|
| §2 GPU frame | RTX `off` 4.944 ms, `b --indirect off` 3.255; Cezanne 35.438, 21.053 | The default (table, guard, lamp channel, and whatever Phase 4 builds): **a guess** of the `b --indirect off` figure plus 0.3 ms on the RTX and 2.0 ms on the Cezanne, lamp on; on a dig's first frames the reopened fill adds its burst, nothing on a turn (Phase 4's reading) | `flythrough --level --seed 1 --headless --render 1920x1080 --frames 2700 --adapter NVIDIA\|AMD [--pose vein]`, three quiet rounds alternated with the v0.41.0 binary, the perf lock, the memory cap; and the rig's flight on both adapters |
| §2 lighting line (6.0 ms): light + gradient + probes + gather | RTX `off` 0.802 + 0.086 + 1.058 + 0.092; `b --indirect off` 0.122 + 0.017 + 0 + 0 | light near the arm's (a capped view draws nothing); gradient within 0.02 ms of the arm's (one ray a stratum); probes **a prediction** under 0.2 ms RTX with the lamp still, the moving case Phase 5's reading; the reopened fill and the own-face read **guesses** in Phase 4, each only if built. **Amended 2026-11-04:** the guard read (Phase 3, loaded) light + gradient 0.147 + 0.092 ms RTX and 0.752 + 0.395 Cezanne at the spawn, 4 % and 19 % of the line, accepted; with Part 1b's window closed at rest the gradient returns to about Phase 1's 0.017 (a prediction, Phase 4 done-when 10). **Amended 2026-11-04 (owner; the form amended the same day, architect), the preload, guesses by arithmetic:** a sample with its shadow ray costs about 3.5 ns on the RTX (v0.41.0's light pass, 0.802 ms over 640 x 360 lighting pixels), so `S_PRE` 32 768 is about 0.11 ms; the walk's demand, about 19 000 samples a frame at 30 voxels a second, about 0.07 ms, and `Shift` binds at `S_PRE`. The sweep at `B` 128: most bricks end on their entry word, a surface brick's 512 voxels test six neighbours each through the mirror's lookup, a guess of under 0.01 ms; at rest the trickle (`B / 16`) and an empty list, under 0.01 ms. About 0.12 ms in flight. The Cezanne at Phase 3's light ratio (5.1) about 0.6 ms. The line then reads about 0.36 ms RTX and 1.8 ms Cezanne of 6.0 (6 % and 30 %), before Part 2's fill. **Amended 2026-11-05 (architect, for the owner), read and guessed:** the walk read 0.062 (patch) and 0.094 ms (vein), about 4.6 ns a sample in the light pass's excess (0.059 ms over 12 798 samples at the vein), at rest 0.015 and 0.012 (loaded; a quiet re-read owed against 0.01). The claim within `r0` 24 adds, **a guess**, the faces probed a frame (444 and 211, read) times `(24 / 64)^2` times their refused share (0.94 and 0.75, read) times the 248 samples to `C_PRE`: about 14 500 and 5 600 samples, **+0.07 and +0.03 ms, about 0.13 and 0.12 ms walking**; at `r0` 32 the patch is about 0.18, over the bar, so the sweep picks; at rest nothing once the faces within `r0` hold `C_PRE` (their slabs are done); at `Shift` `S_PRE` binds (about 0.19 ms, read, not a bar). **Amended 2026-11-05 (architect, for the owner), `C_PRE` under the stop bar, a guess:** if done-when 14 picks 1024, the claim within `r0` draws about 1016 samples a face instead of 248, about 4.1 times the 14 500 and 5 600 above, about 59 000 and 23 000 a frame walking, over `S_PRE` 32 768 at the patch, so the cost stays near `S_PRE`'s 0.15 ms (4.6 ns a sample) and the fill lags instead: the lead done-when 16 reads (the fill-time table read 226 and 323 frames at 1024 against 57 and 83 at 256). `C_PRE` 1024 is the default only if done-when 17's walking bar holds; otherwise 256 stays and the choice is the owner's in Phase 6 (h). **Amended 2026-11-05 (owner), Phase 7, read and guessed:** the readings the fix pass starts from are light plus gradient about 0.24 ms on the RTX (Phase 3, 0.147 + 0.092, loaded) and 1.147 on the Cezanne, and the preload 0.080 and 0.106 ms walking at `r0` 32 (patch, vein; RTX, loaded) and 0.414 and 0.668 on the Cezanne; the window's HUD at the hall read light 3.18 ms with half the view missed (each missed pixel draws its own sample every frame), which Part A's 0 misses should return toward the level's figure (a guess). Phase 7 adds, **guesses**: the blend pass 0.05 to 0.15 ms RTX at the window's size (one lane per seen cell, eight to 24 neighbour lookups of one to `CACHE_PROBES` slots each, against the temporal pass's 0.065 to 0.12 ms for one lookup a lighting pixel), a few hundredths for the `seen` touch, and about five times that on the Cezanne (Phase 3's light ratio), 0.25 to 0.75 ms; the age-out and the sweep 0.1 to 0.15 ms on a frame that runs a slice of 2^20 cells, none at rest. The bar (done-when 8): at most 0.25 ms added on the RTX at rest and walking. The line then reads about 0.6 ms RTX and about 3 ms Cezanne walking of 6.0 (guesses) | the same runs, per pass; the preload's cost at rest, in flight and on a turn (done-when 17), at each `r0`; Phase 7's done-when 8 at the window's size |
| §2 denoise line (3.0 ms): temporal + a-trous | RTX 0.086 + 0.497 | unchanged or lower (the a-trous over one channel). **Amended 2026-11-05 (owner), Phase 7:** the window read temporal 0.12 and 0.18 ms, a-trous 0.77 and 0.76 (patch, hall; HUD, RTX, loaded); Phase 7 adds nothing here (the blend is its own pass, under the lighting line) and the a-trous over the fallback should fall with the misses (a guess) | the same runs |
| §3 `Frame::frame_bytes` | main 130 267 216 B; the spike's table on, 89 617 600 B at the rig's setting | **arithmetic:** + 41 943 040 (table) + the fill list if Phase 4 builds it, + the preload list (`PRELOAD_LIST_MAX` 32 768 entries ping-ponged, about 256 KiB, a guess) and its stats words (amended 2026-11-04: no other store; it fills the table, about 15 000 to 25 000 faces within `R` 64 at the level, half lit, 0.007 to 0.012 of 2^20 by the census, mostly cells the view also makes, and claims none past `PRE_LOAD_MAX` 0.25 or after a frame's miss; amended 2026-11-05: the claim within `r0` adds no store, only a uniform word, and its cells, the faces within 24 voxels of a walked path inside the 256-voxel light radius, are about 23 000 on a straight 512-voxel walk at the vein's density, about 0.022 of 2^20, **a guess**, beside today's read 0.010 and 0.024; amended 2026-11-05: the stop-moving bars add no store, and a sum of squares a cell for a spread stop stays a Followup: one word is + 4 194 304 B at 2^20 (cells of 11 words), two if the integer squares need 64 bits to stay order-independent at 4096 samples, + 8 388 608 B), - 3 686 400 (the `f32` pair) - the reservoir textures - about 44 MB of the probe atlases and state at a side of 16, lamp on; dev records the exact sum in the test's comment. **Amended 2026-11-05 (owner), Phase 7, arithmetic:** the cell grows to at most 12 words (48 B: `seen` and `blend`) and the default to 2^22 cells, 201 326 592 B, **+ 159 383 552 B** over the 41 943 040 table (2^21 if Part A's rule picks it: 100 663 296 B, + 58 720 256), plus the blend list at 4 B a lighting pixel (921 600 B at 640 x 360, 1 552 960 at the window's 844 x 460). Against Phase 5's tip (Phase 4's 163 488 480 B less the probes' 49 674 240, about 113.8 MB, derived; dev records the exact figure) the default reads about 274 MB, about 18 % of `docs/nfr.md` section 3's 1.5 GB line for everything else in VRAM | `frame_bytes` asserted in `upscale.rs` at the default, printed at the close; Phase 7's done-when 8 | `frame_bytes` asserted in `upscale.rs` at the default, printed at the close |

Determinism is not touched: everything is in `vxe-render`. The table's integer sums make a
face's mean independent of the order its pixels add, which the close reads as a bit-identical
cell dump across two runs of the same still frame on lavapipe.

## Risks & open questions

- **The young faces are not the fast-turn fault** (read, Phase 1; Plan 0056: light on creases).
  The crease candidate (the half-resolution upsample's nearest tap on the other face) is
  unread until Phase 4's Part 1; if it is refuted, the dots have no mechanism in this plan and
  go to the Followups with the split.
- **The table's read raises the fast yaw's dots over v0.41.0's** (read, Phase 2: +552.5 over
  27.6 at the rig's row, where Phase 1 read v0.41.0 at +252 and the spike's `b` at +158). Phase
  2 lands with it; Phase 4's done-when 7 is the bar, and if Part 3 is not built or misses it,
  the default ships to Phase 6 with that excess, and the owner judges it there (item b).
- **The guard sees one point a cell.** A second emitter behind the first changes a face's light
  without moving the kept point's re-shade; the cell is caught only when a new sample lands. The
  reveal test is the bar; a two-emitter test is a reading.
- **The reopened fill's burst on a large dig.** Its cost is the reopened faces in view times
  `F`, bounded by `FILL_LIST_MAX`; a collapse or a wide carve lists more than the rig's dig. The
  dig's first-frame reading and `F`'s sweep bound it. A turn lists nothing.
- **The hall from a fresh start is not filled** (amended): it converges at its pixels' rate, as
  Phase 2 reads it. If the owner calls it slow in Phase 6's run 1, it is a Followup.
- **One value a face at a lit wall's edge.** The step is the accepted look (owner, the verdict);
  the owner sees it again in Phase 6 with digs.
- **The fallback flickers.** The field look, translucent hits and rays from inside rock blend one
  sample a frame at `ALPHA` with no table. The field look is opt-in; a smooth material's key is a
  later plan. **Amended 2026-11-04:** the leading candidate for the still hall's one-frame steps
  (Phase 2: 0.137, max 175, against 0.001); Phase 4's Part 1 attributes it and Part 1b's
  freezing mean fixes it if it carries them. The freezing mean in motion is today's blend.
- **The guard window misses a change** (amended 2026-11-04). With the guard gated to frames after
  an edit, an upload within the light radius or an emission change, a change that is none of
  these goes unseen until the next window. Nothing in the engine changes emissive light otherwise
  today; a later moving emitter (an entity's light) must open the window, and its plan says so.
- **The still hall's drift is the cells' settle** (amended 2026-11-04). If Phase 4's attribution
  puts the drift on uncapped cells, no mechanism here removes it at 4096 frames (about 68 s at
  60 fps); the owner judges it in Phase 6 (a), and a fill for those cells is the Followup below.
- **The census refutes the hash and the misses persist at a uniform spread** (amended
  2026-11-04): then the size or `CACHE_PROBES` comes back to the architect with the census, and
  ADR-0043's Outcome records it.
- **The table fills in a dense scene.** The sweep bounds it to the light radius; the castle's
  92 % air stamps and the showcase's pillars are the dense cases to read occupancy on. A miss
  count above 0 at the default size goes to the architect.
- **The preload is outrun** (amended 2026-11-04, owner; amended the same day, architect). At
  `Shift` (120 voxels a second) a face first seen at 8 voxels has had about 28 frames within
  `R`, the sweep may reach it up to one sweep late (17 frames at `B` 128), and the walk's demand,
  about 75 000 samples a frame, is over `S_PRE` 32 768 by about 2.3. There the faces settle at
  the preload's lowered rate plus their footprint, never slower than without it; the level's
  `Shift` rows are readings and the box's is a bar at the tenth frame (done-when 16).
- **The sweep visits what no one will see** (amended 2026-11-04, architect, for the owner). It
  lists every lit face within `R`, including the far side of a wall no path reaches, so its
  samples outrun the eye rays' in a dense, walled-off level; the census and the fill-time table
  (done-when 12) read how many, and the claim rule and `PRE_LOAD_MAX` bound the table. A face
  lit only rarely may draw 8 unlit samples and take no cell; it then fills at its footprint when
  seen, as today (the refused count reads it). Amended 2026-11-05 (architect, for the owner):
  read at the patch as 17 563 of 18 619 faces probed, most of them dim rather than unlit; within
  `r0` they now claim a cell regardless, and from `r0` to `R` they still refuse, which done-when
  12's lit-share table and done-when 14's band from `r0` to `R` read for the owner.
- **Dark faces need more samples than bright ones** (amended 2026-11-04, owner). A face lit
  through a gap sees most of its samples shadowed, so its relative spread is several times a
  lit wall's and 10 % takes far more than 256 samples; that is the owner's dark corner. The
  `C_PRE` sweep up to 4096 (`C_CAP`) and done-when 16 read it; if 4096 at the default `B` and
  `S_PRE` does not meet done-when 16 within the walk's 112 frames, the shortfall goes to the owner in Phase 6
  (h) with the table, not to a larger budget unasked. Amended 2026-11-05: done-when 14 reads
  the band within `r0` at `C_PRE` 256 and 1024, split by the reference mean's decile, and the
  reference at `C_CAP` 4096 is itself about 0.09 noisy for a face lit on one sample in 32.
  Amended 2026-11-05 (architect, for the owner): read so (two references agree within 10 % on
  0.08 to 0.66 of the lit cells by decile); the level bars are now stop-moving bars in display
  levels, under which a dim face needs about as many samples as a lit wall, and the distance to
  the reference is a reading against that floor.
- **A still face can sit off the truth** (amended 2026-11-05, architect, for the owner). The
  stop bar passes a cell whose mean has stopped moving under a level a frame while it is still
  `sigma / sqrt(n)` off its true value, and it then crawls by its residual (about 4 levels in
  all at `n` 500, a guess) until `C_CAP`. Neighbouring faces of one wall may then differ by a
  few levels at rest. The travel after the stop is read beside every stop bar; whether it
  shows is the owner's in Phase 6 (c) and (h), and a stop by a cell's spread that caps a cell
  once it is still (the Followups) is the answer if it does.
- **The claim within `r0` fills unlit faces** (amended 2026-11-05, architect, for the owner).
  A face within `r0` that no emitter reaches takes a cell and `C_PRE` samples of zero, once for
  the cell's life: about 248 samples wasted a face, a guess of up to 0.07 ms walking at the
  patch, inside the walk's bar by the Budgets' arithmetic, and the `r0` sweep reads it. If the
  walk's cost at 24 reads over 0.15 ms, `r0` falls to 16 and the band from 16 to 24 goes to the
  owner in Phase 6 (h); a cheaper unlit test inside `r0` (a sum of squares to stop on, the
  Followups' stop rule) is the next form, not more probes.
- **The preload works off the screen** (amended 2026-11-04, owner). It spends samples on faces
  the view does not show, against "the cost matches what is on screen". The owner asked for it;
  the trickle at rest bounds the waste, and whether it is on by default is the owner's (Phase 6
  (h)).
- **The table at the hall's load** (amended 2026-11-04, owner; the figures amended the same day
  from the hall's census). The hall's view holds 0.185 of 2^20 native and 0.260 up, with 659 and
  140 missed lighting pixels in the last frame, so the claim rule (no claim after a frame's miss,
  none past `PRE_LOAD_MAX`) lets the preload claim no cell there and its unseen faces settle as
  today; done-when 17 asserts it adds no miss. Raising the size is the owner's call in Phase 6 (h), against `docs/nfr.md`
  section 3's 1.5 GB line (2^21 cells add 41 943 040 B).
- **Exposure's settle reads as the light's** (amended 2026-11-04, owner). Turning into a dark
  corner at auto exposure, ADR-0033's eye adaptation also brightens over seconds, which no fill
  shortens; Phase 6's run 6 at a fixed exposure tells the two apart.
- **The lamp channel's lag.** `h_move` trades flicker for lag; the owner judges it in Phase 6
  and may turn `lamp_bounce` off.
- **Plan 0056 reads the light through `--steady` until Phase 2 removes it.** The lane order is
  the orchestrator's; the log names it.
- **FSR 2 moves the final image** (0.525 against 0.004 native in the spike). This plan asserts at
  scale 1; the display-level bar is Plan 0056's.
- **The blend softens true light** (amended 2026-11-05, owner). A shadow edge and the voxel-edge
  step become a one-face ramp, and a one-voxel pit's bottom takes part of its rim's light,
  since a pit and a step are the same local geometry to the kernel. The crease test bars the
  crease (no light across orientations); the pit and the ramp are the owner's in Phase 8 (g),
  and the half weight of the stepped candidates is the knob if they show.
- **The grain is not all the estimator's** (amended 2026-11-05, owner). Half the hall's view
  missed the table in the window, so its grain was mostly the fallback's raw samples; if the
  hall still reads grainy at 0 misses, the estimator's share is what is left, and `G_noise`
  splits it by count band. A band under 256 samples at frame 4096 (faces beyond `R` at their
  footprint) that misses the bar after the widening is the parallel baked design's to fill, not
  a larger kernel unasked.
- **An aged-out cell comes back young** (amended 2026-11-05, owner). A face beyond `R` unseen
  for `AGE` frames (down to `AGE_MIN` 60 at a hall over the mark) restarts from no samples when
  the view returns to it, and reads through the blend's pool until its own fill; the hall's
  turn-back is read in Part 0's scripted path.
- **The demand outgrows 2^22** (amended 2026-11-05, owner). If the hall's working set at the
  window's resolution, after the age-out at `AGE_MIN`, stays over 3/16 of 2^22, the table at
  this design cannot hold the view; it goes to the architect beside the parallel design, whose
  per-chunk store has no such load.
- **Probe holes and duplicates after eviction** (amended 2026-11-05, owner). Clearing cells
  mid-run is what the chunk-change sweep already does; Part 1b's census found the claim's
  duplicate keys once (fixed). Done-when 2 reads 0 duplicates after an eviction.

## What this plan does NOT do

- Fix the yellow dots beyond the crease's own-face read (Phase 4, Part 3, if read owed): any
  other light mechanism the split leaves is a Followup for the architect, and an upscaler or
  geometry share is 0056's followup plan.
- A fill of every young face in view, or warm rays aimed at the fast turn's dots: dropped by the
  amendment of 2026-11-04 on Phase 1's stop rule. Rays from the eye do not return for the
  preload either: Part 2b's brick sweep fills the faces within `R` whatever the view (amended
  2026-11-04, architect, for the owner).
- Fill faces beyond `R`, rays from the emitters, or a fill at chunk upload (Decision); or a
  burst that holds a dig's faces on its first frame (done-when 15).
- Set a display-level drift or flicker bar under FSR 2, or change the upscaler, sharpening or
  the far normal: Plan 0056.
- Indirect light from emissive sources: a per-source feed into the lamp channel, a later plan
  when the owner asks ("maybe we will come back with indirect light for some sources later").
- Touch ADR-0033's exposure constants (owner, the verdict's item 5).
- Give the field look or translucent hits a table key: a later plan, if the owner takes the
  field for a material class.
- Plan 0048's schemes: moot under ADR-0043 (no emissive sample at the probe rays); the owner's
  "drop" is owed on it.
- Plan 0044's waste items in the frame (FSR 2's `prepare`, the far normal on default pixels),
  or re-basing the frame ledger in `docs/nfr/02-frame-budget.md`: readings go there, the
  re-base is Plan 0044's.
- The Radeon's walk-speed spike, or any change to the surface.
- (amended 2026-11-05, owner) A store filled at chunk load for static emitters, or any change to
  the key: the parallel architect session designs it with its own ADR, and Phase 7 stays inside
  ADR-0043's table. A screen-space filter on the emissive part, or coarser cells (Decision).

## Owner answers

The verdict (owner, 2026-11-04, Plan 0051 Phase 4, recorded in `logs/0051.md`):

1. **Direction:** B with indirect off. `--steady ab --indirect off` is this plan's first
   question (Phase 1).
2. **Read mode:** nearest. The `smooth` read goes (Phase 2).
3. **The probes:** shrink to a lamp-bounce channel, built here (Phase 5), the scaffolding kept
   for a later return of indirect light for some sources; today's probe logic redone, not
   patched.
4. **The deletes:** ADR-0043's roster, reconciling "B's list plus the probes" with the shrink.
5. **Exposure's settle:** no change.
6. **Next:** Plan 0056 runs now beside this plan; the slow settle in a dug hole is this plan's
   (Phases 1 and 4).

Open at draft: the "go".

## Implementation log

> Written by `dev`: one row per phase as its commit lands, then the close block. **The phases
> above are the contract. This section records what happened.** Observations, never conclusions:
> no pass list, no self-assessment. Deviations and unmet done-whens are **always** disclosed.
> Keep it shorter than `## Implementation phases`. Readings, gate counts, attempts, sweeps and
> resume notes go to `docs/plans/logs/0057.md`.

| phase | owner | state | commit |
|---|---|---|---|
| 1: The open questions, read | dev | not started | |
| 2: The table is the only emissive store, and the default | dev | not started | |
| 3: The world-space guard | dev | not started | |
| 4: Faces are sampled: the fill pass and the warm rays | dev | not started | |
| 5: The lamp-bounce channel | dev | not started | |
| 6: The owner's window, with digs | human | not started | |

### Notes

_(Deviations from the plan, with the commit, stated without justification. Done-whens not
satisfiable as stated, and what was done instead. Followups noticed and not acted on. One line
each. Empty is fine.)_

### Close triggers

_(Facts for the architect. No recommendations, and no suggested version bump.)_

- **What shipped:** feature / fix-only / docs-chore-only
- **Public API changed:** types, traits, functions, shader interfaces, config (list them, or none)
- **Budget readings:** each `nfr.md` figure touched, with its measured value, machine and commit
- **Gate at the tip:** the commands run (build, nextest, doctests, clippy, fmt, deny), exit codes, test counts
- **Outstanding `human` phases:** which, or none
- **Phase 7 (amended 2026-11-05, owner):** the table's size and `CACHE_PROBES` chosen, with the
  peak occupancy at the window's resolution; the before and after rows of the strips, `G_noise`
  and `G_plane`; Phase 4's done-whens 8 and 9 re-read; `frame_bytes` at the chosen size

## Followups

- **The yellow dots, if Phase 4's Part 1 refutes the crease candidate** (amended 2026-11-04,
  architect): light on creases, mostly new each frame, not young cells (Phase 1) and not the
  upscaler (Plan 0056 Phase 2), with no mechanism yet attributed. It belongs to no plan yet: the
  architect takes Phase 4's split and drafts the spike for the plan after this one. The same
  holds if Part 3 is built and Phase 4's done-when 7 misses (amended 2026-11-04, from Phase 2's
  done-when 7): the spike starts from the rig's fast-yaw row (table read on and off, `up1.5` and
  scale 1, `--lighting full`) and the second candidate, a pixel at a true face edge moving
  between two cells under FSR 2's jitter with no screen history to smooth the step.
- **The hall's settle from a fresh start**, if the owner calls it slow in Phase 6: a fill for
  cells opened after a table clear, the same list as Phase 4's Part 2; no plan yet. Amended
  2026-11-04: the same fill answers a still hall's drift if Phase 4 puts it on uncapped cells
  and the owner calls it visible. Amended 2026-11-04 (owner): Part 2b's preload fills the
  fresh start's faces within `R` of the eye, where the table is under `PRE_LOAD_MAX`; what
  remains is the hall beyond `R` and the hall's table over that load.
- **The preload's other forms** (amended 2026-11-04, owner; amended the same day, architect,
  from the visibility reading): eye rays beside the sweep as a faster discoverer of the faces in
  view, if done-when 14's misses split mostly to faces with no cell yet (the sweep's latency);
  a larger table for the hall, if the owner asks in Phase 6 (h); a stop rule by a cell's spread
  instead of its count (the cell holds no sum of squares today), if the dark faces' `C_PRE`
  reading says a count is the wrong stop (amended 2026-11-05, architect, for the owner: not
  built in this plan; its triggers are the stop bars' readings showing a `k` band or decile
  still moving at the default `C_PRE` while the others are still at a quarter of it, or the
  owner calling the still faces' grain or crawl visible in Phase 6 (c) or (h); its form then
  caps a cell, bit-identical as at `C_CAP`, once its standard error through the display
  transform is under `STILL_LEVEL`, and ends the preload's fill there, at one or two words a
  cell, the Budgets' figure); and, for static emitters, a store kept per chunk and
  filled once per upload, so a face is lit before it can be seen at any range, the strongest
  form of the owner's "loaded", which needs a store the table is not (a later ADR, if the faces
  beyond `R` are called visible in Phase 6 (h)).
- **ADR-0043's Outcome at the close** (amended 2026-11-04, owner; amended the same day,
  architect): the warm rays it named stay dropped, for the turn's dots and for the settle; the
  settle is Part 2b's brick sweep, and the Outcome records it with Part 2b's readings.
- **ADR-0043's table-sizing input, for its Outcome at the close** (amended 2026-11-04,
  architect): the occupancy after four flights (Phase 2: 0.0576 vein, 0.0424 patch of 2^20),
  the dense cases the Risks name (the castle, the showcase hall) read once on Phase 4's tip, the
  census's verdict (hash, claim or counting) and its fix, and the misses at 0 after it. The size
  stays 2^20 at this plan's close unless a dense case reads over 0.25; shrinking it to save
  memory is not this plan's, since the image comes first. Amended 2026-11-05 (owner): the
  window read a dense case over 0.25 at the patch (0.2566) and 0.947 at the hall, so the size is
  Phase 7 Part A's (2^22 by default, 2^21 by its rule), with `CACHE_PROBES`, the age-out and the
  load cap as a share of the size; the Outcome records them and the window-resolution occupancy.
- **The parallel baked per-chunk store** (amended 2026-11-05, owner): a separate architect
  session's design and ADR. If it lands, Part A's size and age-out may be moot for static light
  (it carries the faces' light without a hash), Part C's widening is the bridge it replaces
  beyond `R`, and Part B's blend is read through by either store; the plan that builds it
  states which of the three it keeps.
- **A stop rule by a cell's spread** (amended 2026-11-05, owner): the existing Followup's
  trigger includes Phase 8 (d) calling the grain visible after the blend.
