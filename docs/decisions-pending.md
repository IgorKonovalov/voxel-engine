# Decisions pending: the owner's review list

Written 2026-10-17 after the overnight run that closed plans 0003, 0004, 0007 to 0012 and 0014
(main at v0.11.0, not pushed). Each section gives the context, the options with their costs, a
recommendation and what an answer sets in motion. Delete this file once every item is answered;
the answers themselves land in the plans, ADRs and `nfr.md` as usual.

Answer in any form ("D1: B, D2: accept, ..."); anything not answered keeps the recommendation
that is already in effect, where one is.

Contents:

- [D1. Plan 0013: the contour attribute footprint](#d1-plan-0013-the-contour-attribute-footprint)
- [D2. Plan 0006: the listening checks and two audio budgets](#d2-plan-0006-the-listening-checks-and-two-audio-budgets)
- [D3. Edit cost: a new brick representation, or a measured target](#d3-edit-cost-a-new-brick-representation-or-a-measured-target)
- [D3a. Brick format options](#d3a-brick-format-options)
- [D4. The cylinder sweep at 26.3 us](#d4-the-cylinder-sweep-at-263-us)
- [D5. Push main](#d5-push-main)
- [D6. Calls taken overnight that you can overturn](#d6-calls-taken-overnight-that-you-can-overturn)
- [D7. Follow-up plans to draft](#d7-follow-up-plans-to-draft)
- [Windowed checks](#windowed-checks)

---

## D1. Plan 0013: the contour attribute footprint

**Context.** You picked contour as the best-looking surface mode. Contour stores a plane per
surface voxel in "attribute pages" on the GPU. At the current full-detail radius (16 chunks,
about 102 m) those pages add 1.71 GB on top of the world mirror, so the streamed world needs
about 3.31 GB live, against `nfr.md` §3's 3 GB line. Plan 0013 measured four ways to shrink it
(Phases 1 to 3 are done on branch `plan-0013`; Phase 4 is your pick).

The candidates, as the plan defines them:

- **A, surface voxels only:** a 64 B mask per brick plus 2 B per voxel that actually has a
  plane. Lossless. Needs a variable-size allocator inside the GPU pool, because the plane count
  per brick changes with every edit; that allocator, its fragmentation and compaction are new
  work in Phase 5.
- **B, an 8-bit code per voxel:** the plane quantised to 3+3 bits of direction and a 2-bit
  offset. Lossy (normal error doubles). Fixed size, simple.
- **C, one plane per 2x2x2 block:** 128 B per brick. Smallest. Lossy at 2-voxel features
  (silhouettes widen).
- **D, a smaller full-detail radius** with today's layout: 12 chunks (77 m) or 14 (90 m).
  Lossless, no new code, gives up view distance against §3's "about 100 m".

Measured (Phase 3; allocation multipliers 1.44x and 1.93x are the grow-never-shrink buffer
overhead: 1.44x is the plan's guess, 1.93x is what a streamed flight actually reached):

| Row | Mirror live | Alloc x1.44 | Alloc x1.93 | Normal error / silhouette |
|---|---|---|---|---|
| current, 16 chunks | 3.31 GB | 4.77 GB | 6.38 GB | 7.41 deg / 32 px |
| A, 16 | 1.82 GB | 2.63 GB | 3.52 GB | 7.41 deg / 32 px |
| B, 16 | 2.46 GB | 3.54 GB | 4.74 GB | 14.71 deg / 40 px |
| C, 16 | 1.81 GB | 2.61 GB | 3.50 GB | 5.50 deg / 44 px |
| D, 12 | 1.96 GB | 2.83 GB | 3.79 GB | 7.41 deg / 32 px |
| D, 14 | 2.56 GB | 3.69 GB | 4.94 GB | 7.41 deg / 32 px |
| A or C at 14 | 1.56 GB | 2.24 GB | about 3.0 GB | as A or C |
| A or C at 12 | 1.35 GB | 1.94 GB | 2.59 GB | as A or C |

**The finding that matters most.** D at 12 chunks was flown for real: live memory peaked at
2.13 GB but *allocated* memory reached 4.10 GB, because GPU buffers grow and never shrink
(ADR-0004). So under motion no single 16-chunk row fits 3 GB once allocation is counted. There
are two levers, and they compose:

1. **Bytes per brick** (A, B, C).
2. **Radius** (D), or **the allocation growth rule** (let buffers shrink or reuse slots
   better), which is the ADR-0004 question the plan's Risks named.

**Options.**

- **A at 16, plus the growth-rule question.** Pros: lossless, full radius, same look you
  approved. Cons: the biggest Phase 5 (a sub-slot allocator); still 3.52 GB at the measured
  1.93x, so it only fits if allocation overhead is also tamed.
- **C at 16.** Pros: smallest and simplest (fixed size), lowest normal error on the metric.
  Cons: lossy at 2-voxel features (silhouettes 44 px vs 32); same 3.50 GB at 1.93x.
- **A or C at 12 to 14 chunks.** Pros: the only rows under 3 GB at 1.93x (2.59 to about 3.0
  GB). Cons: view distance 77 to 90 m, plus whichever layout cost you pick.
- **D alone (12 chunks).** Pros: no new code at all. Cons: measured 4.10 GB allocated, so it
  does not fit unless the growth rule changes too.
- **Change the growth rule first, then pick.** Pros: attacks the 1.93x that sinks every row;
  could make A or C at 16 fit. Cons: an ADR-0004 revision and its own plan before 0013 can
  finish.

**How to judge the lossy ones by eye** (in the plan's worktree, one at a time):
`cd .claude/worktrees/plan-0013 && cargo run --release -p flythrough -- --level --seed 1 --look contour --attrib-quant code8`
(B), then `block2` (C), then `none` (reference). `N` cycles normal modes, `V` detail, `L`
shadows. Look at silhouettes and walls close up and at about 5 m.

**Recommendation.** Look at `block2` (C) first. If it holds up by eye, take **C at 14 chunks**
(about 3.0 GB at the measured overhead, 90 m radius, smallest code) and open a separate plan
for the growth rule; if that later brings overhead down, the radius can go back to 16. If C
does not hold up, take **A at 14**.

**What an answer starts.** The plan-0013 branch merges main (it is 10 plans behind, with
known conflicts in `march.wgsl`, `gpu_world.rs` and `world_budget.rs`), then dev builds
Phase 5. Phase 6 (you set rock detail strength by eye) and Phase 7 (contour becomes the
default) follow.

**Update, 2026-10-17 (after the brick decision).** ADR-0012 (proposed) now settles the
footprint's final form as **A, surface-only**, stored at the popcount index of a surface mask
inside the brick's variable-size mirror slot; that arrives with ADR-0012's mirror-slot plan,
after Plan 0015. So Phase 4's pick is the **interim** row that lets contour become the default
now: the recommendation is **D at 14** (no lossy change to undo later; about 3.69 GB allocated
at the measured 1.44x overhead, so the growth-rule follow-up is still needed) or **C at 14** if
`block2` holds up by eye and you accept a lossy interim. Plan 0013's Phase 4 wording ("its own
ADR") is satisfied by ADR-0012; the architect amends the plan when the branch merges main.

---

## D2. Plan 0006: the listening checks and two audio budgets

**Context.** Spatial audio is implemented and reviewed on branch `plan-0006` and fixed after
review (the mixer was 4.7x too slow because its FIR loop could not vectorise; now 0.36 ms).
The HRIR pack is byte-identical to the one you approved without clicks. It cannot close
without your ears, and two readings miss their proposals.

**D2a. The listening checks** (headset worn swapped, as before;
`cd .claude/worktrees/plan-0006 && cargo run --release -p flythrough -- --seed 1 --audio`,
or `--audio-debug` to also record a wav):

1. Dig a hole, press `H`, turn until the hole is at the left screen edge, press `H` again: the
   knock should come from the left.
2. Dig, press `J`, walk behind rock: the hum should turn duller and quieter, clear again in
   line of sight, and no echo should be louder than the hum behind rock.
3. Press `J`, turn slowly and fast: no clicks.
4. With about 8 sounds playing, press `F1` and note the callback max, active taps and the last
   acoustics update time.

**D2b. `Acoustics::update` at 1.03 ms against a 0.5 ms proposal.** This is the per-frame
occlusion and reflection probe for 32 sources. The 0.5 ms figure had no arithmetic behind it.
The cost is about 240 rays x about 100 voxel steps x about 40 ns, and each step is the exact
integer DDA shared with `World::raycast`, with a chunk-map lookup and lock per step.

- **Option (a), record the reading as the budget (about 1.0 ms) with its arithmetic, and make
  speed-ups follow-ups** (cache the chunk handle across steps; run probes on a worker thread).
  Pros: honest, closes now; probes are capped by count and cadence so 1 ms is a ceiling, not
  every frame. Cons: about 6% of a 16.7 ms frame while all sources are due at once (walking can
  make that every few frames).
- **Option (b), optimise `trace_materials` in this plan.** Pros: the budget stays tight. Cons:
  the DDA is deliberately shared with the engine's raycast; changing it here touches a pinned
  hash's code path and delays the close.
- **Recommendation: (a).**

**D2c. HRIR pack load at 13.6 ms against 10 ms.** Measured in the release fly-through on a
quiet box; a release test build read 7 to 9 ms. It runs once at startup.

- **Accept 13.6 ms as the reading** (startup only), or **ask for a faster loader** (for
  example a memory-mapped pack). Recommendation: accept; a startup cost this small does not
  earn new code.

**Merge note.** `default-members` in `Cargo.toml` is now an explicit list (plan 0011 needed
that so the default build skips `wasmtime`). When 0006 merges, `crates/vxe-audio` must be
added or it silently drops out of `cargo build`. The branch is also behind main and will need a
merge.

---

## D3. Edit cost: a new brick representation, or a measured target

**Context.** `nfr.md` §4 says 100 radius-1 m carves cost under 2 ms. Plans 0003 and 0014 took
the worst case from 4.92 ms to 3.19-3.26 ms (3.03 ms with a warm array pool) on the dev box,
and stopped there. The profile of the warm worst case (3.04 ms):

| Part | Share | Time |
|---|---|---|
| Writing the voxels row by row (`write_rows`) | 54.6% | about 1.66 ms |
| Solving each row's extent inside the sphere | 12.2% | 0.37 ms |
| Row-span cache lookup | 7.1% | 0.22 ms |
| Expanding and collapsing bricks | 5.2% | 0.16 ms |
| Everything else (per-brick work, tallies) | about 21% | about 0.64 ms |

100 carves write about 417 000 voxels, about 4 ns (16 cycles) per voxel to read the old
value, decide the new one, write it and count it. Even if the solve, the lookup and the
expansion all went to zero, the edit would still take about 2.4 ms. **Nothing inside today's
representation (ADR-0002: a brick is either uniform or a full `[u16; 512]`) reaches 2 ms.**

- **Option A: a brick-representation ADR** (for example a bitmask or palette brick, or arena
  handles), superseding part of ADR-0002.
  - Pros: a carve becomes per-row word operations and the tallies come from popcounts; a guess
    of 0.2 to 0.4 ms for the write puts the warm worst case near 1.7 ms on the dev box; it
    probably lowers RAM too.
  - Cons: every reader of bricks changes (raycast, mips, coarse grid, walkable map, stamp,
    collapse, persistence format, the GPU mirror's narrowing); a multi-phase plan with a
    byte-identical determinism gate; and on the baseline machine (guessed 1.5 to 2x slower)
    it may still miss 2 ms.
  - The format options, each with its edit cost, memory, readers, determinism risk and size,
    are laid out in [D3a](#d3a-brick-format-options) below.
- **Option B: revise §4 to a measured cap** (for example "100 radius-1 m carves in at most 3.5
  ms on the dev reference"), or restate it around the consumer's real per-frame edit volume.
  - Pros: free now; the consumer asks for "many edits per frame", and the fly-through's real
    dig is one 0.5 m op, so 100 x 1 m in one frame is a stress figure rather than a stated
    need. The representation can be revisited when a profile of real play shows edit cost.
  - Cons: the target moves to meet the code; if the consumer later does need 100 big edits a
    frame, the work returns with more of the engine built on today's representation.
- **Recommendation: B now, with A recorded as a deferred follow-up**, to be reopened when the
  baseline CPU is measured (M2's close) or a real frame shows edit cost.

**Answered, 2026-10-17.** The owner reframed the question around frame-time stability and
chose **A**, in the form of D3a's option 3 on both sides of the upload: ADR-0012 (proposed),
with Plan 0015 (the traversal hierarchy, drafted) first, then the CPU-format plan whose Phase 1
is D3a's census-and-kernel spike, then the mirror-slot plan. The 2 ms figure stays as written
until the format plan's close reads the new kernel; D3a's six questions are answered in
ADR-0012's Context and Decision except the first (dev box or baseline for the 2 ms figure),
which the format plan's close puts to the owner with the reading in hand.

---

## D3a. Brick format options

**The short answer.** Two candidates are worth measuring: **a faster row kernel on today's
`[u16; 512]`** (option 5, no format change) and **an occupancy mask plus a bit-planar palette**
(option 3). By the arithmetic below, both bring the row write from about 1.66 ms down to a
few tenths of a millisecond on the dev box. **Neither reaches 2 ms on the baseline machine
alone**, because 1.38 ms of the warm worst case is not the row write. What the format buys
beyond the edit is RAM: it would shrink the world's resident bricks from about 3.2 GB to
under 1 GB, which is a guess from the census below. **Recommendation: a two-phase spike that
measures both kernels on the bench's exact spans before any ADR.** Every millisecond figure
in this section is a guess from arithmetic unless it names a reading.

### What the cost is made of

The warm worst case (`worst_warm`, 3.04 ms, Plan 0014's close) writes about 417 000 voxels
(100 x 4169) through `write_rows`: 1.66 ms, about 4 ns or 16 cycles a voxel. The loop in
`crates/vxe-world/src/edit.rs` handles one voxel at a time. For each it reads the old `u16`,
does a bit lookup in the per-op carve table (Plan 0003 built that LUT, so it already exists),
branches, stores, compares against the run being counted, and updates the first and last
changed voxel. Three facts shape every option:

- **A brick row is 8 voxels of `u16`, exactly 16 bytes.** A row is one SIMD register, or two
  `u64` words. The scalar loop spends 16 cycles where a whole-row operation could spend about
  2.
- **Rows are short.** The average chord of a radius-10 sphere along x is 4r/3, about 13
  voxels, cut at brick edges into segments of about 5 to 6. That gives about 70 000 to 80 000
  row segments per 100 carves, a guess consistent with the 54 600 brick-row rows Plan 0014
  counted. Per-row overhead therefore matters as much as per-voxel cost.
- **The remaining 1.38 ms does not depend on the format.** It is the row solve (0.37 ms),
  `RowCache::span` (0.22 ms), expand and collapse (0.16 ms), and the per-brick work: coverage
  tests, chunk locks and the tally (about 0.64 ms). A format can trim expand, collapse and part
  of the lookup, perhaps 0.1 to 0.2 ms. The rest stays.

**The baseline factor.** The baseline CPU is not on hand. Plans 0003 and 0014 guessed it at 1.5
to 2 times slower than the dev box (Ryzen 9 5900HS), and the projections below use that guess.
On it, today's warm worst case would be about 4.6 to 6.1 ms.

**The material census.** No per-brick material census exists. The testbed cave
(`apps/flythrough/src/cave.rs`) has four materials: air, rock, ore and an indestructible shell.
Ore is rare: the fly-through's `B` bench carved rock 340 502 and ore 126. So almost every
testbed `Full` brick holds `{air, rock}`, a few hold three materials, and none holds more than
four. The consumer's real levels (ores, crystal veins, runtime variants such as "crystal lit")
will hold more; how many per brick is unknown. The memory figures below assume 2 to 4
materials per brick, and the spike's first job is to count them.

**The RAM baseline.** At the default residency (`r_full` 16, `r_lod` 24), resident chunks read
about 3.24 GB (`nfr.md` §3), against a 4 GB line with 3.87 GB peak RSS, the tightest margin in
the engine. By arithmetic, about 2.8 M `Full` bricks and mip-bricks at 1 KiB each account for
2.86 GB of that (17 077 full chunks x 512 x 18.7 %, plus 40 700 LOD chunks x 73 x 39 %). About
11.7 M brick slots at 16 B add 0.19 GB.

### Option 1: palette brick, packed indices

**The format.** `Full` becomes a per-brick palette of the distinct materials (`u16` each) plus
512 indices packed at 1, 2, 4 or 8 bits, chosen by palette size (the Minecraft paletted
container). More than 256 materials falls back to raw `u16`. `Uniform` stays as it is.

- **Edits.** Carve, fill and replace each do one LUT step per palette entry: a per-op table
  maps each palette entry to its new entry, or to "unchanged". The row write then becomes
  shift-and-mask work on packed words. A 1-bit row is one byte; a 2-bit row is a `u16`. The
  tally can come from a per-entry popcount only if the code first builds a match mask per
  entry, which packed indices make awkward above 1 bit. A fill that brings in a material the
  palette lacks can push the width past its limit, and then the whole brick is repacked
  (512 reads and writes, about 1 to 2 µs, a guess). Per-voxel equivalent: about 1 to 2 ns
  (guess). Write share: **about 0.4 to 0.9 ms** in place of 1.66 ms.
- **Warm worst case (projection).** About 1.7 to 2.2 ms on the dev box, about 2.5 to 4.4 ms on
  the baseline.
- **Memory per `Full` brick.** At 1 bit, about 64 B of indices plus a 4 B palette and an 8 B
  header; at 2 bits, about 128 B plus the palette. With the slot and allocator overhead, about
  100 to 160 B, against about 1056 B today.

### Option 2: occupancy bitmask plus per-voxel materials

**The format.** `Full` keeps `[u16; 512]` and gains a 64-byte solid mask, one bit per voxel. A
voxel whose bit is clear is air, whatever its material slot holds.

- **Edits.** A carve builds the shape's mask (from the row spans, one byte per row) and ANDs
  it with the solid mask and with the carvable materials. To find which materials were
  carvable and to tally them, the loop iterates the set bits and reads each voxel's `u16`.
  That is about 1 to 2 cycles a bit, with no write to the material array. Clearing the mask
  bits removes the voxels. A fill writes the material slot of every filled voxel and sets its
  bit, which costs about what option 5's row write costs. A replace works as it does today,
  restricted to solid bits. Write share for the carve: **about 0.2 to 0.5 ms** (guess). Fills
  gain less.
- **Warm worst case (projection).** About 1.55 to 1.85 ms on the dev box, about 2.3 to 3.7 ms
  on the baseline.
- **Memory.** 1088 B of payload per `Full` brick, 6 % more than today. RAM goes the wrong way.
- **Determinism risk.** Stale materials stay under clear bits. Every reader has to go through
  the mask, or a hash or upload sees ghosts. Each reader is a place this can go wrong.

### Option 3: occupancy mask plus a bit-planar palette (options 1 and 2 combined)

**The format.** `Full` becomes a 64-byte solid mask, a palette of the brick's non-air
materials, and `ceil(log2(n))` bit planes of 64 B each that index into that palette (none
when n is 1). This is option 1 stored as planes, with air pinned to the mask, which makes
every whole-brick operation a word operation. For `{air, rock}` it is the mask alone plus a
one-entry palette. For `{air, rock, ore}` it is the mask plus one plane.

- **Edits.**
  - **Carve.** The shape mask S is built from the spans, one byte per row; a later step could
    fuse this into the span walk. For each carvable palette entry p, the kernel computes
    `hit_p = solid & S & match(p)` and adds `popcount(hit_p)` to the tally for p. Then it sets
    `solid &= !(S & carvable)`. Each step is 8 `u64` words per plane, about 50 to 150 word
    operations per brick for 1 or 2 planes. Bounds come from OR-folding the hit words.
  - **Fill.** The fill mask is `!solid & S`. The kernel sets those bits and writes the fill
    material's index into the planes. If the material is new to the palette, it appends an
    entry, adding a zero plane when the width grows; no repack is needed.
  - **Replace.** If `to` is absent and every `from` voxel lies inside S, the kernel renames the
    palette entry, which is O(1). Otherwise it rewrites the planes for `match(from) & S`.
  - **Cost.** About 300 to 400 cycles per brick-edit, about 75 to 100 ns, against about 260 ns
    today (1.66 ms over about 6400 brick-edits). The brick-edit count assumes 40 to 64 bricks
    a carve (Plan 0014 counted 24.9 expansions a carve; ADR-0002 bounds a carve at 64
    bricks). Write share: **about 0.2 to 0.5 ms** (guess). Expanding a `Uniform` brick
    becomes a 64 B mask set to all ones or all zeros, and the collapse test becomes a mask
    compare, so about 0.1 of the 0.16 ms expand and collapse share also goes.
- **Warm worst case (projection).** About 1.4 to 1.8 ms on the dev box, about 2.1 to 3.6 ms on
  the baseline.
- **Memory per `Full` brick.** `{air, rock}`: about 70 B of payload. Three or four materials:
  about 130 to 200 B. Five to eight: about 260 B. Up to 256 non-air materials: about 580 B. A
  wide brick: about 1.1 KiB. At the testbed census, the world's resident bricks would drop from
  about 3.2 GB to about 0.7 to 0.85 GB (guess).

### Option 4: run-length rows

**The format.** Each of a brick's 64 x-rows is a list of runs (start or length, plus
material). A row offset table makes the size variable.

- **Edits.** A carve or fill splices the span into the row's runs. The tally comes from run
  overlaps, which is cheap and exact. Rows with 1 or 2 runs are fast; noisy rows have up to 8
  runs, and the splice branches heavily. About 20 to 40 cycles per row segment (guess). Write
  share: **about 0.4 to 0.9 ms**.
- **Warm worst case (projection).** About 1.8 to 2.3 ms on the dev box, about 2.7 to 4.6 ms on
  the baseline.
- **Memory.** About 200 to 600 B per `Full` brick (guess), variable, which needs a
  variable-size allocator.
- **Reading a voxel.** A random `get` scans up to 8 runs, slower than every other option on
  the raycast and the queries.

### Option 5: keep `[u16; 512]`, change the kernel only

**The format.** Unchanged. `write_rows` processes a whole row segment at once: one 16-byte
load, then a lane mask from `(x0, x1)` via a 64-entry table or an iota compare.

- **The kernel.**
  - **Carve.** Compare all lanes against the segment's first value and against air. If those
    two values cover the span (most cave rows: rock, or rock and air), do one LUT lookup per
    distinct value, one blend-store, `popcount(mask)` into the tally, and take the bounds from
    `trailing_zeros` and `leading_zeros` of the 8-bit change mask. Rows with three or more
    distinct values fall back to today's scalar loop.
  - **Fill and replace.** One compare (`== AIR` or `== from`) and a blend.
  - **Portability.** Written as fixed `[u16; 8]` or `u128` lane code that LLVM vectorizes on
    the x86-64 SSE2 baseline both CI arms share: no `core::arch`, no `unsafe`, no
    target-specific path.
- **Cost.** About 15 to 25 cycles per segment, over about 76 000 segments (guess). Write share:
  **about 0.3 to 0.7 ms**.
- **Warm worst case (projection).** About 1.7 to 2.1 ms on the dev box, about 2.5 to 4.2 ms on
  the baseline.
- **Memory.** Unchanged (about 1056 B per `Full` brick).
- **Readers.** None change. The determinism risk is limited to the fallback path, which
  `apply_matches_the_reference_over_seeded_ops` and `..._on_uniform_worlds` already cover.

### Option 6: arena handles instead of `Box` per brick

**The idea.** Orthogonal to the format. `Full(Box<..>)` becomes `Full(u32)`, a handle into a
per-chunk arena, so a brick slot shrinks from 16 B to 4 to 8 B. A per-chunk arena keeps the
chunk lock as the only lock, and eviction drops one `Vec`.

- **Edit gain.** Small. The Plan 0014 pool already removed the allocation (`Brick::expand` is
  2.9 %), and locality gains are unmeasured: about 0 to 0.1 ms (guess). The warm worst case
  stays about 2.9 to 3.0 ms.
- **Memory.** About 8 to 12 B per slot and about 16 B of allocator header per `Full` brick:
  about 0.15 to 0.19 GB of RAM at the default residency (guess).
- **Cost to readers.** The real cost is the API. `Chunk::brick` can no longer return
  `&Brick` holding the data, so it returns a view type, and every reader that matches
  `Brick::Full(v)` changes. `Brick` is public, and `vxe-sim`'s `body.rs` matches on it.
- **Pairing.** Option 1 or 3 needs a size-classed store anyway (64, 128, 256 and 512 B), so the
  arena is best taken together with a format, not alone.

### Option 7: 4^3 sub-bricks inside 8^3

**The idea.** Each brick holds 8 sub-bricks, each `Uniform` or 64 raw voxels, so a carve
writes fully covered sub-bricks as one value.

- **Edit gain.** In a radius-10 sphere, fully covered 4^3 cells hold roughly a quarter of the
  voxels (an inner sphere of radius about 6.5; a guess with lattice effects). Carves into an
  already dug cave gain less. Write share: about 1.2 to 1.5 ms. The warm worst case would be
  about 2.6 to 2.9 ms.
- **Memory.** About 550 to 1050 B per surface brick (guess).
- **Readers.** A third level appears in every reader.
- **Verdict.** Not worth it. Option 3's word operations already make a fully covered region
  cost almost nothing, and a z-layer of a plane is already one `u64` (index `x + 8y + 64z`).

### What every brick reader must change (options 1 to 4, and 6)

With option 5, no reader changes. For the format options, a **decode-to-scratch adapter**
(`brick.decode(&mut [u16; 512])`, about 0.5 to 2 µs per brick, a guess) lets a slow reader
keep its current code. The edit kernel and the hot readers get native paths.

| Reader | Where | Change under a palette or mask format (options 1 and 3) |
|---|---|---|
| Raycast and DDA, `trace_materials` | `raycast.rs` (`trace_materials` on branch `plan-0006`) | `get(i)` becomes a palette lookup (option 3: a mask bit, then 1 or 2 plane bits). Air tests become a mask-bit read, so a later change can skip empty 2^3 blocks or z-layers by word, as the GPU does |
| Mips / LOD | `mips.rs`, mip-bricks stored as `Brick` | Majority of 8 children: adapter first. Mip-bricks shrink like world bricks (the 39 % `Full` LOD ring is about 1.2 GB of today's RAM) |
| Coarse grid | `coarse.rs` `CoarseCell::of` | Solid count = `popcount(mask)`. Dominant = per-entry popcounts. Gets faster |
| Walkable map | `walkable.rs` | None: it reads the coarse grid |
| Region queries, sweeps, bodies | `query.rs`, `vxe-sim` `body.rs` | `get` plus native mask paths; `body.rs` matches on `Brick` variants, a public-API break |
| Stamp / prefabs | `stamp.rs`, `source.rs` | Per-voxel writes with palette growth: decode, write, encode per brick (adapter), later native |
| Generation | `WorldSource::brick`, `cave.rs` | `Brick::from_fn` / `from_voxels` encode at the end; sources unchanged |
| Collapse | `collapse.rs` (the heaviest reader, 22 matches) | Rubble's `v.map(kept)` becomes a palette remap (O(entries)). A vertical drop by whole voxels is a z shift; option 3 makes a z-layer one `u64` per plane |
| Persistence / spill | `persist.rs`, `SPILL_VERSION` 1 | Either keep the `u16` spill format (decode on write) or bump the version and write the compact form (smaller spill, about 10x) |
| Op log, wire, replay | `log.rs`, `wire.rs` | **Unchanged**: ops, not voxels |
| World hashes | `hash.rs` (per-voxel `get(i)`) | **Unchanged** if hashes keep reading decoded voxels. Pinned constants (`determinism_vector`, `raycast_hit_list_is_pinned`, the collapse, op-log and stamped-world hashes) must not move |
| GPU mirror upload and narrowing | `vxe-render` `gpu_world.rs` | The GPU keeps 8-bit per-chunk palette indices (ADR-0004); **the format does not reach the shader** in this decision. The upload remaps at most n brick entries to chunk-palette indices and expands, so the per-voxel narrowing scan (about 150 ms on the first sync) shrinks. The occupancy mask comes from the planes |
| Equality in tests | `#[derive(PartialEq)]` on `Brick` | Has to compare decoded voxels, or the encoding has to be canonical |

### Determinism

The tallies are exact under every option: integer counts per material, sorted by id at the
end, so the order in which a kernel counts does not matter. Popcounts are exact. Hashes read
decoded voxels, so a format change moves no pinned hash, provided the encoding is invisible
to them. The specific risks are:

- **Option 1:** a repack on a width change.
- **Option 2:** stale materials under clear bits.
- **Options 1 and 3:** the same voxels can have two encodings (palette order, stale entries
  after a carve). Any encoding is deterministic for the same op sequence, which is what §4
  and §6 require. Equality and checksums still have to go through decoded voxels, or the
  encoding has to be canonicalized: entries in first-appearance order, compacted at collapse.
  That is an ADR clause.
- **The gate.** Every format plan reruns both oracle tests, the five pinned hashes on both CI
  arms, and Windows (still unproven on the newer hashes, see D5).

### Complexity

- **Option 5.** One plan of 2 or 3 phases. No ADR is strictly needed; the kernel's choice is a
  code comment.
- **Option 3 (and 1).** One ADR superseding ADR-0002's CPU brick clause, plus a plan of about
  7 phases:
  1. the type with `get`, `set` and `decode`, plus an equivalence oracle against `[u16; 512]`;
  2. the edit kernel;
  3. hot readers natively (raycast, coarse, query, body);
  4. cold readers through the adapter (mips, collapse, stamp);
  5. generation and spill;
  6. the GPU upload remap;
  7. readings (edit, RAM, upload).

  It breaks the public API (`Brick`), so it is a minor bump with a consumer note. **Option 6**
  folds into it as the store of the variable sizes.
- **Options 2 and 4.** Plan sizes like option 3, with less to show for it.

### What each enables later

- **Option 3**, the most:
  - about 2.4 GB of RAM back against the 4 GB line;
  - spill files and any future chunk snapshot (late join without full replay, network or
    disk streaming) about 10x smaller before any general compression;
  - collapse drops and rubble as word operations and palette remaps;
  - raycast and `trace_materials` skipping air by mask word (this also feeds D2b's 1.03 ms);
  - coarse cells by popcount;
  - a path to a variable-width GPU slot, which is ADR-0004's rejected Alternative A, later.
    The sub-slot allocator that D1's option A needs would serve it too.
- **Option 1:** about the same RAM and network gains, without the word-operation readers.
- **Option 5:** the edit gain only, but now and cheaply. Its kernel is thrown away if option 3
  follows.

### Comparison

All millisecond and byte figures are guesses from the arithmetic above, except "today", which
is the Plan 0014 reading (dev box) and its baseline projection.

| Option | Row write (now 1.66 ms) | Warm worst, dev box | Warm worst, baseline (x1.5 to x2) | CPU bytes per `Full` brick (2 to 4 materials) | Readers changed | Determinism risk | Size |
|---|---|---|---|---|---|---|---|
| Today | 1.66 ms (reading) | 3.04 ms (reading) | 4.6 to 6.1 ms | about 1056 | none | none | none |
| 1 Palette, packed | 0.4 to 0.9 ms | 1.7 to 2.2 ms | 2.5 to 4.4 ms | about 100 to 160 | all (adapter for cold ones) | low: repack, encoding | ADR + about 7 phases |
| 2 Mask + `u16` | 0.2 to 0.5 ms (carve) | 1.55 to 1.85 ms | 2.3 to 3.7 ms | about 1100 | all | medium: stale materials | ADR + about 6 phases |
| 3 Mask + planar palette | 0.2 to 0.5 ms | 1.4 to 1.8 ms | 2.1 to 3.6 ms | about 70 to 200 | all (adapter for cold ones) | low: encoding | ADR + about 7 phases |
| 4 Run-length rows | 0.4 to 0.9 ms | 1.8 to 2.3 ms | 2.7 to 4.6 ms | about 200 to 600, variable | all, slower `get` | low | ADR + about 7 phases |
| 5 Row kernel only | 0.3 to 0.7 ms | 1.7 to 2.1 ms | 2.5 to 4.2 ms | about 1056 | none | low: fallback path | 2 or 3 phases |
| 6 Arena handles | 1.55 to 1.66 ms | 2.9 to 3.0 ms | 4.4 to 6.1 ms | about 1030 (and 4 to 8 B a slot) | all (view type) | none | ADR + 3 or 4 phases |
| 7 4^3 sub-bricks | 1.2 to 1.5 ms | 2.6 to 2.9 ms | 3.9 to 5.8 ms | about 550 to 1050 | all, a third level | low | ADR + about 7 phases |

### Recommendation

**Spike first, then decide.** The two best candidates, 3 and 5, land within each other's
guess ranges on the edit, so only a measurement separates them. They differ sharply in cost
and in what else they buy.

The spike is a small plan, dev-owned, with no ADR and no change to `Brick`:

1. **Census.** A `world_budget` mode that counts distinct materials per `Full` brick and
   mip-brick at the default streamed residency and on the level after the `B` bench, as a
   histogram of 1, 2, 3 to 4, 5 to 16 and more than 16. That settles the memory column for
   options 1 and 3 on real numbers.
2. **Kernels.** A criterion bench (`benches/brick_kernel.rs`) that replays the `worst_warm`
   case's exact per-brick spans through three kernels: today's `write_rows`, option 5's row
   kernel, and option 3's planar carve, fill and replace, on a stand-alone type. Each kernel's
   tallies and bounds are asserted equal to the reference before timing. Read ns per
   brick-edit and the projected `worst_warm`, quiet box, paired.

Decide by the numbers:

- **If option 3's kernel is not clearly faster than option 5's** (about 1.5x or more on the
  write), ship option 5 as a 2-phase plan and leave the format for the RAM question.
- **If it is clearly faster, or the owner wants the RAM**, write the ADR for option 3 with
  option 6's per-chunk arena as its store.

Either way, **D3's option B still applies to the baseline machine.** No row in the table
reaches 2 ms there, because the 1.38 ms outside the row write stays. The spike's readings set
the cap's number.

### Questions for the owner

1. **Is the 2 ms figure meant for the dev box or the baseline machine?** On the baseline, no
   option reaches it without also attacking the solve, the lookup and the per-brick work.
2. **Does RAM count in this decision?** Peak RSS is 3.87 GB against 4 GB today. If yes,
   option 3 earns its cost even with a modest edit gain. If no, option 5 is the cheap answer.
3. **How many materials will a real brick hold?** Ore and crystal veins, runtime variants and
   prefab trims all add entries. Up to 4 per brick keeps option 3 at 1 or 2 planes; beyond 16
   its advantage fades.
4. **May `Brick` break as a public type?** It is `pub` and `vxe-sim` matches on it. Options 1
   to 4 and 6 replace it with a view type.
5. **Spike first, or straight to an ADR for option 3?** The spike costs a short plan and
   removes the guesswork above.
6. **Should the format ever reach the GPU?** If yes, it ties to D1's sub-slot allocator and
   ADR-0004's Alternative A; if no, the mirror stays 8-bit per voxel and only the upload
   changes.

---

## D4. The cylinder sweep at 26.3 us

**Context.** The collision sweep (a vertical cylinder moving through voxels) had a 5 us
proposal; you accepted the measured 17.5 us earlier. The walk-check fixes (climbing that
works, no freezing in corners, an overlap guard) cost about 50% on a quiet box: the F2 wall
case now reads 26.3 us per sweep. `nfr.md` §8 says "awaits the owner's call". A full
controller tick on terrain is 2.6 us against a 20 us budget, so the sweep is not hurting a
frame today.

- **Accept 26.3 us as the budget.** Pros: the walk felt natural in your check; the tick
  budget has 7x headroom. Cons: a body-heavy scene (many NPCs sweeping) costs more than
  planned.
- **Profile against 20 us** as a small follow-up plan. Pros: holds the line before more
  bodies arrive. Cons: work now for a cost no consumer feels yet.
- **Recommendation: accept, and record "profile if a scene sweeps more than about 100 bodies
  a tick"** as the trigger to revisit.

---

## D5. Push main

**Context.** Main is many commits ahead of `origin/main` (v0.4.0 to v0.11.0). Nothing was
pushed overnight because you had not asked. Pushing also triggers CI's Windows arm, which has
never run the new cross-platform determinism checks: the collapse course hash, the op-log
hash, the stamped-world hash, and native-versus-component equality. Until it runs, `nfr.md`
§6's "both arms" claims are pending.

- **Push now.** Pros: Windows proves (or disproves) determinism while the context is fresh.
  Cons: if Windows fails, main's CI goes red until a fix lands.
- **Hold.** Pros: none beyond reviewing first. Cons: the Windows unknowns grow with every
  plan built on top.
- **Recommendation: push after reading this file.** Say "push" and it is one command.

---

## D6. Calls taken overnight that you can overturn

These were applied under your standing "go with your recommendations". Each closed plan's
`## Close` section lists them as overturnable. If you overturn one, the plan or ADR gets an
amendment and dev a small pass.

| Plan | Call | Why | Main alternative and its cost |
|---|---|---|---|
| 0004 | A body an edit fills into is not pushed out (no depenetration); recorded as a known gap | Only an edit onto the body causes it; `G` refuses an overlapping placement | Lift the body out by up to a step height: a small fix pass |
| 0007 | A 13-byte boundary entry in the op log marks where directly submitted ops end and logic ops begin | Only option that replays exactly, leaves ADR-0010 alone and moves no budget or hash | Log submitted ops in the input log (breaks "engine never decodes inputs"), or a third log (bigger header) |
| 0007 | `--vault`, `P` and collapse are refused under `--save`/`--replay` instead of being logged | Logging them needs op kinds 2 to 4 and a way for the log to name a prefab, likely an ADR | Wire them now: a plan of its own (D7) |
| 0008 | Hostile `.vox` caps: model edge 256, 65 536 placements, translation 2^20 | A 70-byte file could otherwise allocate 3 GB; 1 KB could ask for 32 GB of placements | Larger caps: only if a real asset needs them |
| 0009 | Chunks outside a bounded world count as air, so `boundary_anchors` means something | Before, the flag did nothing | Treat the edge as rock: the flag stays inert |
| 0009 | Rubble spares indestructible voxels; a rigid drop moves them | Matches how a carve treats them | Destroy them: breaks the material table's rule |
| 0010 | The overlay's 5% cost check gates on the bounded level only; the streamed figure is logged | The streamed runs vary 21% on their own while chunks load | Gate after full residency: a new run mode |
| 0011 | A script that traps (fuel, memory, panic) stays stopped; later ticks report `Stopped` | A restart would think it is tick 0 and re-apply its setup edits | Restart from seed (silent state reset), or snapshots (needs an ADR) |
| 0011 | Script timers are always ready; host-side panics become a script trap | Real-time timers broke determinism; a panic killed the server | None sensible |
| 0011 | Per host call budget 2 us (the 1 us proposal missed at 1.4 us); host-call work is not metered by fuel | A whole tick through the component is only 1.03x native | Batch the imports now: work before any consumer needs it |
| 0012 | No version bump for the repository gates | Nothing a consumer links changed | A patch bump |
| 0014 | Phase 3's target restated to what was measured (span share 19% to 13%, not under 5%) | Under 5% needs about 11 cycles a row | Keep the plan blocked: see D3 instead |

---

## D7. Follow-up plans to draft

Say which to draft; the architect writes each as a normal plan for your approval.

1. **Route stamp, drop and remove through the server** (op kinds 2 to 4 in the op log, the
   wire and the logic host), so `P` and collapse work under `--save`. Needs a decision on how
   the log names a prefab (an id plus content hash in the header, or the prefab bytes), likely
   an ADR.
2. **Meter host-call work and range-check shapes in `vxe-world`**: today a script can ask for
   a raycast or a box count big enough to take seconds, and extreme shapes panic in debug
   builds (now caught and turned into a trap, but the engine should refuse them).
3. **A small dev pass of leftover nits**: the overlay allocates a little every frame while
   shown, its memory estimate leaves out the brick-array pool, a stale "4 Mi" comment, an
   unused parameter, two missing doc comments.
4. **The GPU buffer growth rule** (ADR-0004), if D1 goes the way recommended.
5. **The sweep profile**, only if D4 is "profile".

---

## Windowed checks

Each was proven headless; these are the looks only you can judge. Run from the repo root on
main unless noted.

- **Prefab vault** (plan 0008): `cargo run --release -p flythrough -- --level --vault`. A dark
  1.6 m box sits about 10 m ahead along +x; digging around it leaves its shell.
- **Stamp key** (plan 0008): `cargo run --release -p flythrough -- --level`, aim at rock within
  10 m and press `P`: a 1.6 m box appears with a corner at the aimed voxel, and a `stamped:`
  line prints. Aim at air: `stamped: no surface within 10 m`.
- **Collapse** (plan 0009): on `--level`, dig out the rock under a ledge. The terminal prints
  `unsupported: N cells`, the piece falls 8 voxels a frame and prints `landed after ...`. Press
  `R` and repeat under another ledge: it turns into a rubble pile. `F1` shows the last query's
  time.
- **Stats overlay** (plan 0010): `cargo run --release -p flythrough -- --level --hud`. Rows at
  top left (over-budget rows yellow), a frame-time strip with a red 16.67 ms line at bottom
  left, `F2` hides and shows it.
- **Audio** (plan 0006): see D2a.
- **Footprint** (plan 0013): see D1.
