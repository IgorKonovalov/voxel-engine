# Plans index

The one-minute "what's in flight" view. Read this first each session instead of re-deriving
state from `git log`. Completed plans move to `done/`.

**Next free number: 0003** (ADRs are a separate sequence; next free there is **0005**).

## Active roster

| Plan | Title | Status | Live constraint |
|------|-------|--------|-----------------|
| [0002](0002-streaming-residency-lod-ring-and-mirror-narrowing.md) | Streaming residency, an LOD-only ring, and a narrowed, sharded mirror | draft | Second half of M1; approve only after Plan 0001 closes (it builds on 0001's Phase 8 masks and its review's call on the readings). Paired with ADR-0003 and ADR-0004. |

## Recently closed

| Plan | Title | Closed |
|------|-------|--------|
| [0001](done/0001-bounded-brick-map-edits-and-headlamp-marcher.md) | Bounded brick map, deterministic edits, and a headlamp ray marcher | 2026-10-16, v0.2.0 |

## Conventions

- One plan per milestone slice, numbered when drafted, never reserved ahead.
- A row carries at most two sentences of live constraint. What landed and why belongs in the
  plan file.
- A closed plan leaves this table; `done/` records it.
