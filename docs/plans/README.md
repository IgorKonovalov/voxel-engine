# Plans index

The one-minute "what's in flight" view. Read this first each session instead of re-deriving
state from `git log`. Completed plans move to `done/`.

**Next free number: 0002** (ADRs are a separate sequence; next free there is **0003**).

## Active roster

| Plan | Title | Status | Live constraint |
|------|-------|--------|-----------------|
| [0001](0001-bounded-brick-map-edits-and-headlamp-marcher.md) | Bounded brick map, deterministic edits, and a headlamp ray marcher | approved | First half of M1 in [`docs/roadmap.md`](../roadmap.md); the bounded world, all edit ops, queries, cave source, primary marcher plus one shadow ray, window shell, fly-through. Streaming and LOD are Plan 0002, drafted after this one closes. Paired with ADR-0002. |

## Recently closed

_none_

## Conventions

- One plan per milestone slice, numbered when drafted, never reserved ahead.
- A row carries at most two sentences of live constraint. What landed and why belongs in the
  plan file.
- A closed plan leaves this table; `done/` records it.
