# ADR-0073: Deterministic simulation testing is an in-tree seeded harness at the seams the engine already has: a message-level simulated channel, a delta-debugging shrinker, no framework

> **Status:** proposed
> **Date:** 2026-11-17
> **Related plan(s):** [Plan 0089](../plans/0089-deterministic-simulation-testing-seeded-scenarios-a-simulated-channel-a-mirror-oracle-and-gates.md)
> (draft). Builds on ADR-0007 (the op log, the byte channel, the owned client's desync check),
> ADR-0010 (the logic component and its deterministic WASI), ADR-0003 (the spill) and
> ADR-0008 (the support query). Sources: [Deterministic simulation testing: external
> sources](../references/deterministic-simulation-testing.md).

## Context

The owner asked (2026-11-17) what the engine should take from the article "Deterministic
Simulation Testing" on martinfowler.com. The article's prerequisites are already this
engine's rules, by design rather than retrofit: `vxe-world`, `vxe-sim` and `vxe-entity` are
headless and deterministic, `clippy.toml` denies the wall clock, every generator takes a seed
(`SplitMix64` in `vxe-world::hash`), `vxe-net`'s `Server::tick` is a counter the caller
advances, the logic component sees tick clocks and seeded entropy (`vxe-script-wasm::wasi`),
an op log replays from its seed to the same world hash (`LogFile::replay`,
`Server::resimulate`), an owned client compares every result with the server's and raises
`Desync`, and CI asserts the hash vectors on Linux and Windows. The one deliberately
nondeterministic piece on the server side is the streaming worker pool, whose output is a pure
function of seed and coordinate, and whose residency hash is asserted independent of the worker
count.

What is missing is the harness that *uses* those seams: today every determinism test is a
fixed vector (24 ops, one course, one path). Nothing draws random ops, stamps, collapses,
inputs and faults from a seed, checks invariants after every step, and hands back the seed that
broke one. The article's second half, the simulated network and disk with seeded faults, has no
counterpart at all: `MemChannel` is the only `Channel`, and it delivers everything, whole and in
order.

Three choices could reasonably go another way. **Where the simulated channel sits:** at the
`Channel` trait, which carries whole messages (`Arc<[u8]>`, "arrive whole and in send order"),
or under a byte stream, as FoundationDB's `Sim2Conn` does, which would also exercise the code
that frames bytes into messages. The engine has no such code yet: the framing layer arrives
with M9's QUIC transport. **Whether to build or adopt:** turmoil and madsim simulate a network
for tokio programs by owning the runtime or patching it; Antithesis runs an unmodified binary
under a deterministic hypervisor as a hosted service; loom and shuttle explore thread schedules
of concurrent code. **How a failing run becomes a report:** a seed alone (TigerBeetle,
FoundationDB), a property-testing library's shrinker (proptest, quickcheck), or a shrinker of
our own over the scenario's step list.

The dependency policy weighs here: exact pins, `cargo deny`, and "a new dependency is named
in the plan that adds it". `proptest` brings about ten crates for a strategy and a shrinker;
turmoil brings tokio's runtime into a crate that today depends on `vxe-world` alone.

## Decision

Deterministic simulation testing is an in-tree harness, in a dev-only crate that is not a
default member, over the seams the engine already has. It draws a scenario from one seed with
the engine's `SplitMix64`: ops, stamps, collapse choices, logic inputs, controller moves and
channel faults as steps. It checks invariants after every step: an edit's per-material counts
equal the voxel diff and nothing outside its reported region changed, the support query equals
a brute-force flood over the coarse grid, the replayed log reproduces the hash, owned clients
equal the server, the controller never ends inside a solid, the GPU mirror after an incremental
schedule equals a fresh upload. A failure prints its seed and step, shrinks the scenario by
delta debugging over the step list, and writes the minimal scenario as a file and one JSON line,
so a person or an agent reruns it with `--seed N` or `--scenario FILE`.

The simulated channel is **message-level**: an implementation of the existing `Channel` trait
whose `send` assigns each message a delivery tick from the seeded faults (delay, drop,
duplicate, reorder through unequal delays, a partition per link that heals after a fixed
number of ticks, a clog that holds everything), released by a `SimNet::advance(tick)` the
scenario drives. A byte-level seam under the framing code is **owed to the plan that writes
that code**, M9's transport plan, which adds it as a second `Channel` implementation beside
this one. Where the message-level faults expose a protocol gap today (a dropped `Applied`
leaves an owned client silently stale until an overlapping op), the owned client gains the
smallest fail-safe rule: a seq gap or reversal is a desync, a duplicate is dropped; ordering
and retransmission stay with M9.

The determinism rules become **fitness functions** where they are not yet: `clippy`'s
`iter_over_hash_type` denied in the deterministic crates (an iteration whose order is proven
not to leak carries an `#[expect]` with the reason), `disallowed-methods` gaining thread
spawns and environment reads beside the clock (the worker pool and the scoped generation
carry the grep-able `#[allow]` the app shell's pacing already uses), and a script gate that
scans the edit, collapse, support, stamp, coarse, query and fixed-point sources of `vxe-world`
and all of `vxe-sim` and `vxe-entity` for floating point, taking over the file list of the
fly-through's `generation_sources_contain_no_floating_point` test.

No new dependency: the shrinker is `ddmin` over a `Vec<Step>` on the engine's RNG, and the
harness depends on `vxe-world`, `vxe-net`, `vxe-sim` and `vxe-entity` alone.

## Consequences

### Positive

- Every determinism claim the engine makes gets a generator behind it instead of one vector;
  the batch of seeds in the gate is also a cross-platform vector (its combined hash is pinned
  on both arms).
- A failing seed is a bug report both workflows can use (the roadmap's "Two workflows, both
  first class"; ADR-0072): a person reads the text line and the minimal op list, an agent
  reads the JSON line and reruns the file headless.
- M9's protocol work starts with a fault model and a convergence test already in the tree; the
  QUIC transport is tested against the same `SimNet` scenarios plus a byte-level seam of its own.
- The fitness functions turn three rules of `best-practices.md` ("Results are canonical",
  "Thread order never leaks into results", "no `f32` between op parameters and voxel writes")
  from review items into gates.

### Negative

- A message-level simulator never runs framing code; until M9 adds the byte-level seam, a
  frame split across two reads is untested. Accepted because that code does not exist yet.
- Random scenarios find bugs the fixed vectors did not, in the lane that builds the harness,
  which is the point and also the schedule risk: Plan 0089 says how a finding is handled (a
  local fix in the lane; otherwise a backlog entry with its seed and the batch's seed range
  moved past it, named in the test).
- A dev-only crate is one more workspace member to keep building, and the rename (Plan 0087)
  renames it with the rest.
- `iter_over_hash_type` bites on every `HashMap` iteration in `vxe-world`, including the ones
  that collect and sort (`ChunkStore::coords`, `take_dirty`): each needs an `#[expect]` with
  its reason, a one-time pass of perhaps a dozen sites (a guess until the lint runs).
- Gate time grows by the seeded batch; the plan budgets it under the 10 s line per test and
  measures it at the close.

## Alternatives considered

### Alternative A: A byte-level simulated channel now

Simulate under a byte stream, framing messages ourselves on both ends, so that a frame split
across reads, a partial write and a mid-frame close are exercised today. Rejected because the
engine has no framing code to run: `Channel` carries whole messages by contract, and the layer
that turns a QUIC stream into messages is M9's. A byte-level simulator now would simulate a
transport that does not exist and be rewritten when the real one arrives. It is owed to that
plan, not dropped.

### Alternative B: An external framework (turmoil, madsim, Antithesis, loom or shuttle)

turmoil and madsim give a seeded network and clock to tokio programs by owning or patching the
runtime; the engine's server is synchronous, tick-driven and already clock-free, so adopting
one means wrapping a channel we own in a runtime we do not use, for `vxe-net`'s one dependency
to become tokio's tree. Antithesis runs unmodified binaries under a deterministic hypervisor,
the answer when a codebase has no seams; ours has them, and a hosted service runs nothing in
this repository's CI or on the dev box. loom and shuttle explore thread schedules of
concurrent code, which is the one part of the engine this harness leaves nondeterministic by
design; a schedule-exploration pass over the store lock and the eviction race is a possible
later chore, not this decision.

### Alternative C: proptest for generation and shrinking

Write the scenario as a proptest `Strategy` and get shrinking and a regressions file for free.
Rejected because the scenario is one seed and a step list, which shrinks well by delta
debugging in about a hundred lines on the engine's own RNG, while proptest adds about ten
crates (`rand`, `rand_chacha`, `regex-syntax`, `bit-set` and their trees) to a workspace that
pins and audits every one. Revisit if the generator grows a second value domain with a grammar
(logic inputs as structured messages, say), where a strategy library earns its tree.

## Outcome

_(Added at acceptance if implementation falsifies something above.)_
