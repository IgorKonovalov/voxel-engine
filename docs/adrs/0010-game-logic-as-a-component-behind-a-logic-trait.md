# ADR-0010: The game's server logic is a component behind a `Logic` trait, developed natively and shipped as WebAssembly; Lua is deferred

> **Status:** accepted (2026-10-17, at the Plan 0011 close; fail-stop overturned 2026-10-18, see Outcome addendum)
> **Date:** 2026-10-16
> **Related plan(s):** [Plan 0007](../plans/done/0007-op-log-byte-channel-shared-and-owned-clients.md) (the trait and the server loop), [Plan 0011](../plans/done/0011-wasm-host-for-the-logic-component.md) (the host)
> **Supersedes:** the founding interview's "WASM for deterministic server logic, Lua for client scripts" (`docs/roadmap.md`, API surface), in the part that concerns what the hosts are for

## Context

The founding interview chose scripting hosts, WebAssembly for server logic and Lua for client
scripts, without saying what the first script would be. On 2026-10-16 the owner looked at the
first consumer's draft (`~/Work/voxel-roguelite`) and the systems it will build on
(`~/Work/recompilcation-project`, a reimplementation-grade specification of Diablo II's item
creation, monster spawning, level generation and, from ADR-0014 there, combat simulation) and
decided that the whole of the game's server logic runs as a component.

The facts that make this a decision:

- **The logic is integer and seed-driven.** Every system the recompilation project documents
  is integer arithmetic on per-thing seeds: the RNG core, the seed tree, treasure classes,
  quality rolls, affixes, monster pools, packs, combat formulas. Such code is bit-identical in
  WebAssembly and native alike, so determinism, replays and late join follow from the
  arithmetic rather than from discipline.
- **The engine knows no gameplay** (`CLAUDE.md`). Tables, treasure classes, monster families,
  the consumer's attention and noise models have to live somewhere that is not `crates/`.
- **Co-op is planned** (consumer note 0004): solo runs an in-process server; co-op replaces
  the channel. A listen server, a dedicated server, a replay and, later, client-side
  prediction of digging all want the identical logic.
- **Debugging inside WebAssembly is poor**, the component model and its toolchain still move,
  and `wasmtime` is a heavy build. A developer iterating on game rules cannot pay that every
  run.
- **Big data must not cross the boundary.** A chunk is 512 KiB at `u16`; the generator runs
  on a thread pool at over 100 M voxels per second per thread (`nfr.md` §5). A generator
  inside a single-threaded component would lose both.

## Decision

The game's server logic implements one trait, `Logic`, with `init(host, seed)` and
`tick(host, tick, inputs) -> output`. The engine's server (`vxe-net`, Plan 0007) drains client
inputs once per tick, calls `tick`, applies the edits the logic requests through a `Host`
handle (queries and edits only), logs the inputs and the ops, and announces results and the
logic's output to clients. Inputs and outputs are opaque bytes the logic defines; the engine
never decodes them.

The logic crate compiles two ways from one source: **linked natively** behind the trait for
development, tests and the fly-through, and **as a WebAssembly component** hosted by
`vxe-script-wasm` (Plan 0011) for shipping, mods and the determinism guarantee. The host is
configured deterministically: canonical NaNs, no wall clock, no entropy, fuel metering per
tick. The walking skeleton of the host is the equality of the two: the same logic, native and
as a component, driven by the same input log, produces the same op-log bytes and the same
world hash on both CI arms.

Level generation splits: the logic produces a compact level layout (rooms, corridors, vault
placements, material regions, the indestructible shell) and the engine's parallel generator
turns the layout into voxels with integer noise. Voxels never cross the boundary; layouts do.
The layout format is its own plan; Plan 0008's generation-time stamping is its first piece.

Lua is deferred. No consumer needs client-side scripts, and the engine has no UI for them to
hook. The Lua host gets a plan when a consumer asks.

## Consequences

### Positive

- One artifact runs solo, listen server, dedicated server, replay and, later, prediction.
- The engine stays gameplay-free by construction: the boundary is queries, edits and bytes.
- Determinism across platforms, replays from seed plus inputs, and desync diagnosis come from
  the component's configuration and the logic's integer arithmetic, not from a policy.
- The native path keeps the everyday loop fast and debuggable; the component path is a CI
  gate and a shipping format.
- Mods are components against the same WIT world.

### Negative

- **Two build targets of one crate.** A dependency the logic pulls in must build for both; a
  crate that reads a clock or spawns threads fails on the component target late.
- **The host API grows with every engine plan.** Each new query (grid, support, stamp, the
  controller) is a trait method and a WIT import, amended into the boundary when its plan
  closes. The boundary is the engine's most visible API and it changes often at first.
- **Entities cross the boundary every tick.** The logic owns monsters, players, items on the
  floor and projectiles; the host must receive a compact snapshot of their transforms and
  models for rendering and replication. That API is defined with M6.
- **Per-call overhead.** A host import costs tens of nanoseconds in `wasmtime`; a tick with
  thousands of queries (pathfinding at ARPG counts, which the consumer names as its hardest
  problem) needs a measurement early, and may need batched query imports.
- **Toolchain churn and build cost.** `wasmtime` and the component tooling are heavy and
  moving; the host crate stays out of `default-members`, and the guest target must be
  installed in CI.
- **Re-simulation requires a pure logic.** A native logic that reads a clock or an unseeded
  RNG replays wrongly and nothing stops it but review; the component configuration removes
  those sources only for shipped logic.

## Alternatives considered

### Alternative A: An engine-owned loop with script hooks (the edit rule)

The engine runs the server loop and calls scripts at hooks: on edit result, on tick, on
generation. Rejected because a game of this depth needs a hook for every system, the loop
would become the engine's framework by accretion (against ADR-0001), and the game's own state
would be split between the engine's loop and the scripts.

### Alternative B: Native logic only

The logic crate behind the trait, linked natively, with determinism by integer discipline, as
`vxe-world` already does. Rejected not for determinism but for everything else the component
buys: mods, sandboxing, the identical artifact for prediction, and a guarantee that does not
depend on review. The native path is kept as the development loop, so this alternative is
half of the decision rather than a rejection of it.

### Alternative C: Lua for server logic

One runtime for both sides. Rejected because Lua's number model is floating point, its
performance on table-heavy integer systems is a fraction of compiled code, and it offers no
memory sandbox or fuel accounting without a custom build.

### Alternative D: Generation inside the component

The component implements `WorldSource` and fills chunks over the boundary. Rejected for the
boundary cost (half a megabyte per chunk) and the loss of the thread pool; the layout split
keeps the deterministic decisions in the logic and the voxel work in the host.

## Outcome

**2026-10-17, at the Plan 0011 close (v0.11.0).** The decision stands: the echo logic and
the fly-through's dig logic, native and as a component, give byte-equal op logs and equal
world hashes on Linux (the Windows arm confirms it after a push). The build refines or
corrects the text above in these points:

- **A failed logic is stopped (fail-stop).** wasmtime does not enter a component instance
  again after a trap, and the component model leaves a trapped instance's state undefined.
  When a hosted logic fails a call (fuel, memory cap, any trap), the failure is reported once
  and every later tick reports `LogicError::Stopped`; the server carries on without the
  logic. Ops the logic applied before failing stay applied and logged. Rejected: (a) a fresh
  instance and `init(seed)` again (the logic restarts while the world keeps its edits, and
  `init`'s ops apply twice), (b) a fresh instance without `init` (the same, with the seed
  lost), (d) restoring a snapshot taken before the tick (the component API exposes no state
  copy, a guest `save`/`load` pair changes the WIT world, and the world has no undo).
- **The WIT `hit` record mirrors `vxe_world::Hit`**: `{ voxel, face: option<face>, t,
  material }`, not a normal and a distance.
- **`LogicError` lives in `vxe-net`**, beside the trait, so the server can carry it:
  `Logic::take_error` (provided, default `None`) is called after `init` and every `tick`,
  `TickReport::logic_error` surfaces it, and `Server::init_error` returns `init`'s failure.
- **The build split is by cargo features.** `vxe-script-wasm` has a default `host` feature
  (`WasmLogic` on `wasmtime`) and a `guest` feature (the bindings and `export_logic!`); the
  fly-through's `--logic wasm` sits behind its `wasm` feature, off by default, so the
  everyday build compiles no `wasmtime`.
- **The component has clocks and entropy, fixed by the tick, not none.** Both clocks read
  `tick x frame_ns`; timers (`subscribe-duration`, `subscribe-instant`) are always ready, so
  a sleep returns at once; `wasi:random` is a xorshift stream seeded from the logic's seed and
  the tick, and `insecure-seed` (the standard library's `HashMap` keys) is fixed at
  instantiation. "No wall clock, no entropy" above means none that varies between runs.
- **A host call costs about 1.4 µs, not tens of nanoseconds** (an `apply` of a radius-1 m
  carve over native, the tick's entry and the inputs' lowering included; `wasmtime` 49.0.3,
  `nfr.md` §4). The budget is 2 µs per `apply` call; a 100-carve tick through the component
  is 1.03× native.
- **Host-call work is not metered by fuel.** Fuel counts the guest's instructions; a query
  the host runs for it (a large `count_in_box`, a long raycast) costs the guest one call. A
  known limit; bounding or metering it is a follow-up. A panic in a host import (the world
  asserting on a shape the guest chose) becomes the logic's trap, not a server panic.
- **Re-simulating a failure is exact only with the same component bytes, `WasmConfig` and
  `wasmtime` version.** Fuel accounting can change between releases and the fuel per tick
  moves the failing tick; the log file's header records none of them yet.

### Outcome addendum, 2026-10-18 (Plan 0016 close, v0.13.0)

**Fail-stop is overturned** (the owner's call, 2026-10-17): alternative (a) above, a fresh
instance and `init(seed)` again, is taken, capped. A failed call is followed on the next tick
by a fresh instance that runs `init` at tick 0's clock and random stream, then that tick;
`WasmConfig::max_restarts` (default 8) caps the restarts, after which every tick reports
`LogicError::Stopped` as before. Each restart surfaces as `LogicError::Restarted` through
`Logic::take_restart` and `TickReport::restart`, beside the failure. Ops applied before the
failure stay applied and logged; a restart, like the failure, re-simulates exactly from the
log under the same component bytes, `WasmConfig` and `wasmtime` version. The hazard recorded
under (a) stands: an `init` that applies ops applies them again on each restart; no test has
such a guest yet (a Plan 0016 followup). One restart of `flythrough-logic` reads 26.9 to
29.3 µs median (`docs/nfr.md` section 4).
