# ADR-0057: The render API is three objects and a settings struct, with a compiled example as the contract

> **Status:** proposed
> **Date:** 2026-11-12
> **Related plan(s):** [Plan 0071](../plans/0071-the-consumers-path-a-render-example-a-renderer-tested-alone-names-and-errors.md)

## Context

A consumer renders a frame through three objects today: `GpuWorld` (the mirror of the world's
bricks on the GPU), `Frame` (the passes and their history) and `Presenter` (the blit to a
surface), plus `vxe-app`'s shell or the consumer's own window and device. The way to call them is
written nowhere but the testbed: `apps/flythrough/src/main.rs:4033-4120` and `:5760-5830`,
twelve steps across the three objects. `Frame` has 98 public methods, 52 of them used by nothing
under `crates/` and 10 by nothing at all; `vxe-render` exposes 13 modules and 680 public items,
191 of them GPU bit-layout constants; the crate doc is six lines with no example (the review of
2026-11-12, section 3.1).

Plan 0045 (approved) narrows `vxe-render` to a curated re-export list. Plan 0069 (approved)
replaces most of `Frame`'s setters with one `RenderSettings` struct and three named profiles.
Both change the surface; neither says what the surface is for, or writes down the call sequence
a game follows. The first consumer (`~/Work/voxel-roguelite`) needs M1 to M5, all of which are
in the tree.

ADR-0001 chose a library workspace over a framework: the game owns its loop and its window. That
rules out an engine-owned render loop, and leaves the question of how thick the render API is.

## Decision

The render API **stays three objects**, `GpuWorld`, `Frame` and `Presenter`, configured by
`RenderSettings` (Plan 0069) and fed by the consumer's own `wgpu::Device` and `wgpu::Queue`, with
**a compiled example as the contract**: `crates/vxe-render/examples/one_frame.rs` renders a
world built from a closure to a PNG headless, in under 60 lines of consumer code, is built and
run in CI, and is quoted by the crate doc and the README. What the example does not need is not
public: every `Frame` method the example, the tests and the testbed do not call leaves or goes
`pub(crate)`; readbacks and raw textures stay behind one `debug` module the example never
imports. A consumer who reads the example knows how to render a frame; a method the example
does not show is a tool, and says so in its doc.

## Consequences

### Positive
- One file answers "how do I render a frame", and CI breaks when the answer stops compiling.
- The three objects keep their jobs (ADR-0002's mirror, ADR-0018's frame, the blit), so no
  mechanism moves; only visibility and documentation change.
- Plan 0069's settings struct and Plan 0045's re-exports have a consumer-shaped test of what
  they must keep.

### Negative
- The example is one more thing a render plan must keep compiling; that is the point, but it
  is a cost on every lane.
- Methods that leave the public surface may be ones a future consumer wanted; the CHANGELOG
  names each, and a 0.x minor may break (the CHANGELOG's own rule).
- Three objects are still three objects; a consumer wires `sync`, `encode` and `present`
  itself. A facade would hide that, at the price below.

## Alternatives considered

### Alternative A: a `Renderer` facade over the three objects
One type with `new(device, world, settings)`, `sync`, `render(camera) -> &Texture`. Shorter to
call, but it would own the encoder and the submit order, which the testbed and the consumer's
own passes (a HUD, the game's UI, a debug overlay) need to interleave with; the facade would grow
the hooks back. It also hides the mirror's `SyncStats` and the frame's history controls that a
game's streaming and edit code need. Rejected: the facade re-creates the framework ADR-0001
declined.

### Alternative B: a C ABI or a trait-object plugin surface
Stable for a non-Rust consumer, but no consumer asked for one (the roadmap's "API surface" row:
Rust crates by git dependency plus WASM for logic and Lua for client scripts). Rejected until a
consumer that is not Rust exists.

### Alternative C: leave the surface as it is and write a guide
Cheapest; the guide would drift the day a setter moved (the review found 42 testbed-only
methods and 10 dead ones in `Frame` after 68 plans). Rejected: a document is not a gate, and
`CLAUDE.md` says a rule that keeps getting broken needs a mechanical one.

## Outcome

_(At acceptance.)_
