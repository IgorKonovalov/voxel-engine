# ADR-0077: Frame profiling goes through the `profiling` facade, collected by puffin behind an app feature

> **Status:** proposed
> **Date:** 2026-11-19
> **Related plan(s):** [Plan 0091](../plans/0091-the-frame-profiler-scopes-across-threads-a-gpu-lane-a-dump-for-agents-and-a-hud-ticker.md)

## Context

The engine times what it was told to time: `PassTimer` reads 23 GPU passes a frame through
timestamp queries, the sync has stage clocks, the bake queue's step has laps, the HUD holds a
240-frame ring with p50, p99 and max, and `run.json` and the `sync:` and `frames:` lines carry
the figures. What it lacks is a tree of CPU scopes per frame across the threads (the main
loop, the loading worker, the generation threads, the audio callback), a frozen frame to drill
into, and one sink the tools can read. At least seven backlog entries wait on "a profile of X"
(the eviction scan in a windowed glide, the outdoor settle's queue step, the sync's `alloc`,
the dispatch lap, the first-block fades, the queue step's `filter`, `Walkers::tick`), and
each so far has been answered by a bespoke lap table written for the occasion.

Three constraints shape the choice. The engine is a library: a consumer that does not profile
must pay nothing, in code or in dependencies. `vxe-world` reads no clock (`clippy.toml` bans
`Instant::now` across the workspace; the allowed reads carry a reason) and takes no new
dependency. And a dependency is a cost: exact pins, `cargo deny`, no C++ toolchain in the
default build (ADR-0035 already carries one static C library, DXC, for the Windows arm).

Two facts settle most of it. The `profiling` crate (1.0.18) is already in the lockfile:
`wgpu-core` and `wgpu-hal` depend on it with no backend feature, under which its macros
(`scope!`, `function_scope!`, `finish_frame!`) expand to nothing. It is a facade: the final
binary picks the collector by enabling one feature (`profile-with-puffin`,
`profile-with-tracy`, `profile-with-tracing`, ...), and that choice lights every scope in the
graph, including the 144 in `wgpu-core` and 60 in `wgpu-hal` (counted in the registry sources
at 30.0.1). puffin 0.20.0 (MIT OR Apache-2.0, MSRV 1.92, under our 1.97.1) is pure Rust and
pulls `anyhow`, `cfg-if`, `itertools` and `parking_lot`, all in the lockfile, plus `byteorder`;
its optional `packing` and `serialization` features (bincode, lz4) stay off. `profiling`
1.0.18 pins `puffin ^0.20`, so the facade and the latest puffin agree.

## Decision

The engine's threads are instrumented with the `profiling` facade's macros, pinned at the
lockfile's `=1.0.18` with no features, in `vxe-render` (the frame's encode and submit, the
sync's stages, the queue step's stages, the bake loop's submits) and in the apps (the frame
loop, the tick, the entity movers, the workers they spawn), and nowhere in `vxe-world`, whose
work the callers scope from outside. The collector is puffin, pinned, enabled only by the
fly-through's `profiler` feature (`profiling/profile-with-puffin`), so a build without the
feature carries no collector and no scope code. The GPU passes join as a lane written by our
exporter from `PassTimer`'s timestamps, placed at their frame's submit, and the exporter
writes what the tools read: a Chrome trace (Perfetto) and a text tree for the human and the
agent, `scopes_top` in `run.json` for the tester's report. The in-app view is the HUD's own
ticker and frozen-frame rows, drawn by the overlay's text grid (ADR-0009), not a UI library.
Tracy stays available through the same facade as a later `profiler-tracy` feature if a
sampling view is wanted; it is not taken now because its client builds a C++ library and
`profiling` 1.0.18 pins `tracy-client ^0.18` while 0.19 is current.

## Consequences

### Positive
- Zero cost off: no scope code, no collector, no new crate in a consumer's or the tester's
  build; the facade is in the lockfile already.
- One sink: wgpu's own scopes, the engine's and the app's land in the same frames, so a stall
  inside `wgpu-core`'s validation or a buffer map is seen beside ours.
- The agent's path is text (`tree.txt`, `scopes_top`, a ranking script); the human's is
  Perfetto on the same dump or the HUD, so a reading quoted in a plan can be re-ranked.
- Budget scopes and allocation counts attach to the same frames.

### Negative
- puffin is an instrumentation profiler: it sees only the scopes we wrote, never a hot loop
  with no scope in it; the sampling view (`perf`, Tracy's sampling) stays a separate tool.
- The GPU lane's placement is approximate: a pass's timestamps arrive one to three frames
  after its submit, and the lane is drawn against its own frame's submit, not the GPU's clock.
- Two more crates under the feature (puffin, byteorder), and a feature matrix: CI builds the
  fly-through with `--features profiler` once or the instrumented path rots.
- `wgpu-core`'s 144 scopes are noise in a frame of ours; the exporter's tree folds them under
  the engine scope that called them, and the HUD rows show engine scopes only.
- The `Instant` ban stays: puffin reads the clock inside its own crate, which the workspace's
  clippy configuration does not lint, but a `budget_scope!` of ours that reads `Instant`
  carries the allow attribute and its reason like the shell's pacing reads do.

## Alternatives considered

### Alternative A: `tracing` spans with `tracing-tracy` or `tracing-chrome`

`tracing` gives structured spans with fields and a subscriber model, and `tracing-tracy`
(0.12) feeds Tracy's viewer, the best live profiler on the desk. It lost on cost: `tracing`'s
spans go through the global dispatcher on every enter and exit once a subscriber is installed
(a cost with no feature to compile out of a library), the engine already uses `log` and would
carry two logging facades, and `tracy-client-sys` compiles Tracy's C++ client, which our
Windows arm and `cargo deny` would then own at a pinned viewer version. `profiling`'s `profile-with-tracing`
feature keeps this open as a backend if structured fields are ever wanted.

### Alternative B: our own `vxe-profile` crate, Bonsai's way

Bonsai's `bonsai_debug` is about 3.8 k lines of C++: per-thread scope trees per frame from an
arena, a 128-frame ring, a frame ticker, flame bars, a function list, a memory view, its own
immediate-mode UI. We could write the same in Rust with our overlay as the UI. It lost because
the collector is the part a library already does well (puffin's per-thread streams and frame
view are a few hundred lines we would re-write and then debug), while the parts we do need to
own (the GPU lane, the exporter, the ranking script, the HUD rows, the budget scopes) are the
same work under either choice. The HUD view of Plan 0091 is this alternative's UI on puffin's
data.

### Alternative C: more bespoke lap tables, no profiler

Each stall so far got a lap table (`StageClock`, `StepStages`, the per-second `sync:` line) and
a reading in a plan's log. It lost because every table is written after the question is known
and sees one subsystem; the backlog's seven open "a profile of X" entries each need a new one,
and the owner's "performance lacks" of 2026-11-17 was placed only after the window, by hand.

## Outcome

_(none yet)_
