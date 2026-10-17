# ADR-0007: Server-owned op log replayed from seed, a byte channel, and a client that shares the world in solo play

> **Status:** proposed
> **Date:** 2026-10-16
> **Related plan(s):** [Plan 0007](../plans/0007-op-log-byte-channel-shared-and-owned-clients.md)

## Context

The first consumer requires a server/client split of responsibilities even in solo play
(`project-context.md`, Networking P0): the server owns generation and edit application, the
client owns rendering, audio and input, and the two talk over an in-memory channel so that the
same code path later carries real networking. Late join is "seed plus edit log" (P1), and edits
are already "compact operations, not voxel diffs, for replays, saves and possible networking"
(P1 under Determinism).

What exists: `EditOp` is a `Copy` value of fixed-point integers; `World::apply` is exact, takes
`&self` under per-chunk locks (ADR-0002) and returns an `EditResult`; `World::voxel_hash` is
canonical; the CI vector asserts a hash on both arms. Plan 0002 (ADR-0003) persists edited
chunks on eviction as a RAM-and-disk cache and leaves op-log replay to M5.

Four forces make this a decision rather than a default:

- **RAM.** A level-size bounded world is about 400 MiB on the CPU (`nfr.md` §3 reading). A
  client with its own copy doubles that, and doubles generation, in the common case: solo.
- **Determinism.** The bytes that carry an op must decode to the identical op on both arms.
  A format whose layout belongs to a compiler (struct dumps) or to a library's defaults is a
  risk we can only test for, not reason about.
- **Dependencies.** `serde` plus a binary codec is three to five crates for a handful of
  fixed-width fields.
- **Networking later.** M9's QUIC transport moves bytes. If solo play already moves bytes, M9
  adds a transport and nothing else.

## Decision

The server owns the authoritative `World` and an `OpLog` of `(tick, seq, op)` entries. Every
op enters through `Server::submit`, which assigns the next `seq`, and is applied at the next
`Server::tick` in `seq` order. An applied op is appended to the log and announced to every
attached client as an `Applied` message carrying the op and its `EditResult`; a refused op
(`NotResident`) is announced as `Refused` and not logged. A tick that drained directly
submitted ops also logs a boundary entry after them, carrying the first seq left to the
logic; replay skips it, and re-simulation uses it to apply the submitted ops and resume the
seq count (added 2026-10-17 while proposed; Plan 0007's amendment names the alternatives).

The channel is a trait of two byte queues (`send`, `try_recv`) with one in-memory
implementation now; M9's QUIC transport is another implementation of the same trait. The
client has two modes behind one type. **Shared** holds the same `Arc<World>` as the server
and reads results from the messages; this is solo play, and it costs no second world.
**Owned** holds its own `World`, applies every `Applied` op itself, compares its result with
the server's, and raises `Desync` on the first mismatch; this is the networked client, and it
is tested from day one against the shared mode on the same hash.

Log entries, messages and the save file use a fixed little-endian encoding with a format
version, written by hand in `vxe-world`. A save is a header (seed, source version, extent)
plus the log; a load regenerates from the seed and replays. Nothing in the engine serializes
voxels for persistence; the eviction cache of ADR-0003 is a cache, not a format.

## Consequences

### Positive

- Solo play costs one world, and still exercises the bytes that networking will carry.
- A world is reproducible from a few bytes per op: saves, replays, desync diagnosis and
  late join (M9) are the same mechanism.
- The result in `Applied` lets a shared client skip recomputation, and lets an owned client
  detect divergence on the op that caused it, not frames later.
- The op log is the single order of truth. Whatever thread interleaving fed `submit`, the
  log's `seq` order is what replay follows.

### Negative

- Two client modes are two code paths. Both must be held to the same hash by tests, or the
  owned mode rots until M9 finds out.
- The log grows without bound until M9 compacts it. A sphere carve entry is 47 bytes; a
  session of 100 000 ops is under 5 MB in RAM and on disk. Acceptable for now, not forever.
- Replay costs regeneration plus every op. At Plan 0001's 4.9 ms per 100 carves, 100 000 ops
  replay in about 5 s before Plan 0003 lands.
- An op whose chunk is not resident is refused and not logged, so replaying a streaming
  world must reproduce residency. This ADR does not solve that; bounded worlds come first and
  the server's ownership of streaming focus points is a follow-up after Plan 0002.
- A hand-written encoding is more code to keep right than a derive, and every new op kind
  (Plan 0008's stamp, Plan 0009's collapse ops) adds a tag and a test.

## Alternatives considered

### Alternative A: Function calls on a shared world, no bytes in solo

The cheapest split: a `Server` type that applies ops and returns results, and a client that
reads the same world. Rejected because the networked path would then be written later as a
second code path, which is exactly the retrofit the consumer's requirements warn against, and
because nothing would exercise an encoding until M9.

### Alternative B: Two worlds always

The client owns its world from day one, so solo play simulates networking faithfully.
Rejected because it doubles RAM and generation in the common case with no gain in
correctness: the owned mode is built and tested anyway, and running it in solo buys nothing
the test does not.

### Alternative C: `serde` with `bincode` or `postcard`

Derive the encoding. Rejected because the layout would be the codec's rather than ours, and
the dependency count is out of proportion to a few fixed-width fields. A format we write is a
couple of hundred lines whose bytes we can assert on both arms.

### Alternative D: Snapshot saves of serialized chunks

Save the world as chunk data, as the eviction cache already does. Rejected because a snapshot
has no history: it cannot replay for late join or locate a desync, and it grows with the
world instead of with the edits. M9 may add a compressed snapshot as a compaction of the log,
which is a different thing from replacing it.

## Outcome

_(Added at acceptance if Plan 0007 falsifies something above.)_
