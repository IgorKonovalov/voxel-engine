# Deterministic simulation testing: external sources

An annotated reading list on testing a system by running it under a seeded simulation of its
clock, network and disk, so that every failure is a seed, collected 2026-11-17 while drafting
ADR-0073 and Plan 0089. Every entry names what it is and what we take from it, so a plan can
cite it by title. Each fact was read on the page cited on that day unless marked *unverified*
(a search digest or a secondary report); re-read before leaning on an unverified line.

## The article the owner asked about

- **Unmesh Joshi, "Deterministic Simulation Testing", martinfowler.com/articles/deterministic-simulation-testing.html.**
  Defines the technique as replacing the clock, the network and storage with simulated
  components driven by one single-threaded, tick-based loop, so a test can inject hard
  failures and slowdowns and every execution is reproducible from its seed. Names four
  sources of nondeterminism (the clock, thread scheduling, network timing, disk timing) and
  six coding rules on the deterministic side of the seam: no system clock, no unseeded
  randomness, no raw threads or standard futures, no direct I/O, no iteration over unordered
  collections, no cross-thread callbacks across the seam. Enforces them as *fitness functions*:
  a static rule test on every build (ArchUnit) and runtime owner-thread checks where static
  analysis cannot see which thread runs a lambda. The simulated network queues messages with
  delivery ticks and injects drops, delays, partitions and clogging from one seeded PRNG; the
  simulated file keeps `visible` and `durable` bytes, `sync` copies one to the other and
  `crash` discards the unsynced part. On the boundary: "A message-level simulator is simpler,
  and it exercises everything a protocol depends on", "But it never runs the code that turns
  bytes into messages"; a byte-level simulator (FoundationDB's) catches framing bugs such as a
  frame split across two reads. On cost: "Deterministic simulation testing cannot be added as
  an afterthought; it requires upfront architectural investment." Says nothing about shrinking
  a failing run; the seed reproduces it. What we take: the seam list, the six rules as gates,
  the message-level boundary now with the byte-level one owed to the real transport, and the
  tick-stamped fault log as the bug report.

## Systems that run it

- **FoundationDB, simulation testing (Will Wilson, "Testing Distributed Systems with
  Deterministic Simulation", Strange Loop 2014; apple.github.io/foundationdb/testing.html).**
  The database runs under its own actor language (Flow) with a deterministic scheduler; the
  simulator implements the real network interface at the byte level (`Sim2Conn`), with partial
  writes, in-flight limits and random closes, and the whole cluster runs in one process from one
  seed. What we take: the byte-level seam belongs under the real transport code; our transport
  (M9, QUIC) is where that seam is added. The talk's date is from memory (*unverified*).
- **TigerBeetle, the VOPR (Viewstamped Operation Replicator; docs.tigerbeetle.com, "VOPR").**
  A single-threaded, tick-driven replica design and a packet simulator over whole messages:
  per message, delay, loss, partition, clogging and replay (a message delivered twice). Runs
  seeds continuously; a failing seed is rerun by number. What we take: message-level faults
  decided per message from one PRNG, including duplicates; a batch of seeds in CI with a seed
  as the bug report; no shrinking, small scenarios per seed instead.
- **Antithesis (antithesis.com).** A commercial deterministic hypervisor that intercepts OS
  calls and virtualises time, so an unmodified program in any language runs reproducibly from a
  seed. Founded by FoundationDB engineers (*unverified*). What we take: the option for a
  codebase without seams; ours has them, so a hosted service is not the fit, and it would run
  nothing in this repository's CI.

## Rust tools

- **turmoil (tokio-rs/turmoil; docs.rs, version 0.6.x per its README's dependency line).**
  Runs several hosts on one thread under a simulated tokio runtime and a network with
  partitions, latency and loss steered by hand or by a seeded RNG. Needs the code under test
  to be tokio-async and to use turmoil's socket types. What we take: the fault vocabulary. Not
  taken: our server is synchronous and reads no clock; wrapping it in an async runtime to
  simulate a channel we already own adds a runtime and a dependency tree for no reach.
- **madsim (madsys-dev/madsim).** Patches tokio and related crates through `[patch]` and
  intercepts libc clock and random calls; used by RisingWave (*unverified*). What we take:
  nothing directly; the `[patch]` replacement of the runtime is far more intrusive than our
  seams need.
- **loom (tokio-rs/loom) and shuttle (awslabs/shuttle).** Schedule exploration for concurrent
  code: loom permutes interleavings and memory orderings exhaustively under a `cfg(loom)`
  build with its own sync types, shuttle samples schedules randomly. What we take: a possible
  later tool for the few concurrent seams (the store write lock, the eviction race); out of
  scope for a harness of the deterministic side.
- **proptest (proptest-rs) and quickcheck.** Property testing: a strategy generates values
  and shrinks a failing one along its value tree (quickcheck through `Arbitrary::shrink`);
  proptest persists failing seeds in a `proptest-regressions` file. What we take: shrinking
  toward a minimal failing case is what makes a failing run a bug report; not taken as a
  dependency, since our scenario is an op list under one seed and a delta-debugging shrink
  over it is a hundred lines on the engine's own `SplitMix64`.

## Fitness functions and shrinking

- **Neal Ford, Rebecca Parsons, Patrick Kua, "Building Evolutionary Architectures" (O'Reilly,
  2017).** An architectural fitness function is "any mechanism that provides an objective
  integrity assessment of some architectural characteristic", run continuously. What we take:
  the name for what `clippy.toml`'s clock ban, the `check-*.mjs` gates and the hooks already
  are, and the frame for the lints Plan 0089 adds.
- **Andreas Zeller, Ralf Hildebrandt, "Simplifying and Isolating Failure-Inducing Input",
  IEEE TSE 2002 (delta debugging, `ddmin`).** Minimises a failing input by removing subsets and
  keeping the removal when the failure stays, in O(n^2) tests worst case and close to O(log n)
  when the failure is one cause. What we take: the shrinker over a scenario's step list.
