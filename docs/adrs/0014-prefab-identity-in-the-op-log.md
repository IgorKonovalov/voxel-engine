# ADR-0014: A stamp op names its prefab by content id, and a save carries the prefabs it names

> **Status:** accepted (2026-10-17)
> **Date:** 2026-10-17
> **Related plan(s):** [Plan 0018](../plans/done/0018-server-routed-stamp-drop-and-remove.md)

## Context

The op log (ADR-0007) holds op kind 1, the shape edits, and reserves kinds 2 to 4 for
stamp, drop and remove (`crates/vxe-world/src/wire.rs:23-28`). Stamping writes a `Prefab`
(size, `u16` voxels, a mask; canonical, with a content-defined `Prefab::id()` of FNV-1a 64
over size, voxels and mask, `prefab.rs:249-263`) at an origin with one of 24 rotations. A
16³ vault is 8 716 bytes of content; a model at the `MAX_MODEL_EDGE` of 256 is about
35 MB. Drop and remove carry a region of up to `max_region` cells (4 096 in the fly-through,
12 bytes each, about 49 KB).

Three constraints decide how a stamp op refers to its prefab. ADR-0010 says voxels never
cross the component boundary: a logic sends ops as wire bytes, so an op that carried the
prefab's voxels would move them through `apply` and into every `Applied` message. ADR-0007
says a save is a header plus the log and that late join is seed plus log, so a save that
depends on an asset the reader may not have, or may have in another version, replays to a
different world without saying so. The wire's counts are `u16` (`put_count`), so a 35 MB
payload does not fit an entry as entries are encoded today.

## Decision

**A stamp op carries the prefab's 64-bit content id, the origin and a rotation index; the
server resolves the id in a registry the application fills before use, and a save's header
carries a table of every prefab the log names, with its content.** `LOG_FORMAT` becomes 2:
after the fixed fields, a prefab count and, per prefab, its id and its canonical bytes
(size, voxels, mask). Decoding a save re-hashes each table entry and refuses the file when
an id does not match its content; replaying a stamp whose id is in neither the registry nor
the table is a refusal naming the id, never a silent skip. A live session's clients hold
the same registry as the server (the application registers the same assets on both); a
client that lacks an id refuses the op and reports a desync, as it does for any result
mismatch. Prefab content over the channel for late join is M9's, with the compressed
snapshot ADR-0007 anticipates.

## Consequences

### Positive

- A stamp entry is 34 bytes (12-byte entry header, kind, id, origin, rotation) whatever the
  prefab's size, and nothing but ids crosses the component boundary.
- A save is self-contained and self-checking: the content is in the file, and the id is
  verified against it on load.
- The registry gives the game a place to attach prefab assets to the server once, and the
  same place serves a future late-join message.

### Negative

- **Saves grow by the prefabs they name**, once per prefab: 8.7 KB for the vault, up to
  about 35 MB for a 256³ model. A log that stamps one large model a thousand times still
  carries its content once.
- **FNV-1a 64 is not collision resistant.** Two prefabs with the same id are refused at
  registration, so a collision is detected where the assets are loaded rather than in a
  replay; the hash can be replaced in a later format version if a consumer's asset count
  makes 64 bits uncomfortable.
- **The header is no longer 60 bytes**, so `LOG_HEADER_BYTES` becomes a function of the
  table, and every test that adds 60 to an expected length changes.
- **A client must hold the prefab before the op arrives**; until M9 that is the
  application's duty, and a missing id is a desync, not a download.

## Outcome (2026-10-20, Plan 0018 closed, v0.15.0)

Shipped as decided. The stamp entry is 34 bytes. The 16³ vault adds `8 + 8 716` bytes to a
save's header once, however often it is stamped. A flipped content byte is refused as
`WireError::PrefabMismatch`. `PrefabRegistry` lives in `vxe-world`, so the server, the owned
client and `LogFile.prefabs` share one type. `EditError::UnknownPrefab` takes wire error
tag 3, which leaves tag 2 for Plan 0019's `OutOfRange`. An owned client that lacks a stamp's
prefab reports a desync with `ours: None`. Format-1 files still decode, with an empty table.

## Alternatives considered

### Alternative A: the prefab's voxels inline in each stamp op

Self-contained per op and the simplest decoder. Rejected: it moves voxels through `apply`
and every `Applied` message, which ADR-0010 forbids; a 256³ model does not fit the wire's
`u16` counts; and a vault stamped a hundred times would cost 870 KB of log for 3.4 KB of
ids.

### Alternative B: ids only, with the content in a side file or asset directory

The smallest save. Rejected: the save stops being self-contained, a replay depends on the
asset version on disk, and late join would ship the side file anyway. The content hash
check would turn a mismatch into a refusal, but only after the file is found.

### Alternative C: a registry index (`u32`) in the op instead of the content id

Four bytes shorter per op. Rejected because the index is only meaningful against a
registration order the save would have to pin, while the content id is meaningful in any
session that holds the prefab.
