# NFR section 4: edits, the readings

> The budgets are in `../nfr.md` section 4, one bullet per figure; this file holds every
> reading taken against them, under the bullet's name, in the order recorded, each naming
> its machine, bench and commit. New readings go here. A reading that replaces a proposal
> also updates the bullet in `nfr.md`. A bare `§N` in this text is a section of `nfr.md`.

## Cost

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

## Stamp cost

**Readings (dev reference, AMD Ryzen 9 5900HS, `cargo bench -p
vxe-world --bench stamp`, exclusive perf lock, box under outside load of about 2 to 5,
commits 83ab811 and 39d3700):** `vault16` 22.6 to 22.8 µs; `prefab64_mixed` 2.02 to 2.07 ms;
`prefab64_uniform` 1.455 ms. Both under their proposals; the proposals stand as budgets.

## Replay throughput

**Reading (dev reference, AMD Ryzen 9 5900HS, Plan
0007 Phase 6, exclusive perf lock, 1-minute load 1.45):** `cargo bench -p vxe-world --bench
replay`, decode plus replay of a 470 060-byte log file of 10 000 radius-1 m carves on a
solid 4³-chunk world: 279.07 ms (CI 278.48 to 279.70 ms), about 35 800 ops per second.

**Quiet-box reading at the Plan 0031 tip (8dad5ca, same machine and protocol as §4's Plan
0031 reading, three runs alternated with 2796062):** 61.29, 61.29 and 61.20 ms (about
163 000 ops per second) against 103.16, 102.64 and 103.19 ms.

## Tick overhead

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

## Component boundary

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
