# ADR-0089: The streamed mirror's sync takes a 6 ms time budget a frame, on top of the workers' slot image

> **Status:** proposed (Plan 0111 Phase 6c, 2026-11-28; accepted at the plan's close if 6d's bars hold)
> **Date:** 2026-11-28
> **Related plan(s):** [Plan 0111](../plans/0111-the-stagger-in-flight-on-streamed-scenes-read-quiet-after-plan-0107-then-cut.md)
> (Phases 6b to 6d); [Plan 0107](../plans/done/0107-the-mirrors-slot-image-packed-on-the-stream-workers.md)
> built the image; [Plan 0064](../plans/done/0064-the-windows-frame-pacing-workers-and-readbacks-that-wait.md)
> built the budget. Supersedes the rejection of
> [ADR-0056](0056-the-mirrors-slot-image-is-packed-on-the-stream-workers-and-the-render-thread-only-allocates-and-copies.md)'s
> Alternative A; ADR-0056's Decision stands and its default flips with this record.

## Context

The quiet headless cave at the tester's line (Plan 0111, Phases 2 to 6b, RTX 3080 Laptop,
release) has one kind of slow frame left after Plan 0111's 6a: the **arrival frame**. A focus
move queues a slab of 200 to 500 chunks nearest first; eight stream workers finish it inside one
frame, one poll absorbs it and one sync uploads it, 13 to 33 ms, with no budget on the sync
(`sync_budget_time` `None`, the fly-through's `--sync-ms` default). The per-chunk cost in that
sync is about 65 us on every path (cave 13 ms over 200 chunks, castle 19.6 over 305, outdoor
32.8 over 504). The slab's size, not a per-brick cost, is the frame.

ADR-0056 rejected a time budget on the render-thread sync (its Alternative A) on Plan 0064's
reading: residency p95 3.0 to 13.7 s against 0.30 s under a 2 to 8 ms budget. That reading was
taken at about 300 us a chunk, where an 8 ms budget carried about 27 chunks a frame and the
queue fell behind the camera. Plan 0074 and Plan 0107 cut the per-chunk cost since; at 65 us a
chunk, 6 ms carries about 90 chunks a frame and a slab lands over 3 to 6 frames, 30 to 60 ms at
the window's rate, by arithmetic. The slot image (ADR-0056, Plan 0107, `--mirror-pack workers`)
cuts the per-chunk cost further by copying the chunk's slot words instead of packing them, and
was left off by default at Plan 0107's close because its own bar (20 % off the sync's p99) was
missed at 14 to 15 %, with three conditions named for a flip.

Plan 0111 Phase 6b read both settings as they exist, two runs an arm alternated, quiet (the rows
are in `docs/plans/logs/0111.md`, Phase 6b, and in the plan's amendment of 2026-11-28):

| quiet cave, two runs | default | `--sync-ms 6` | `--mirror-pack workers` | both |
|---|---|---|---|---|
| `over_33ms` | 141, 150 | 15, 16 | 33, 37 | 14, 8 |
| sync-led slow frames | 132, 143 | 6, 8 | 23, 29 | 8, 3 |
| sync p99, ms | 17.2, 17.3 | 7.6, 7.6 | 14.9, 15.3 | 7.9, 8.0 |
| `full residency after` against the default | | +11.9 %, +12.3 % | -1.6 %, -2.1 % | +5.5 %, +4.7 % |
| fill at 5 s and 15 s | | equal | equal | equal |

The quiet castle, default against both: `over_33ms` 43 / 32 to 3 / 3, residency -2.5 %. The loaded
cave (one run an arm): 306 to 52. The eight-CPU stand-in's fresh forest, two pairs: `march GPU`
p50 does not follow the packing arm (+1.3 %, -2.2 %); its fill at 5 s and 15 s within 1.5 %.

## Decision

The fly-through's and so the tester's streamed mirror runs with **a 6 ms time budget on the
sync and the workers' slot image, as a pair**: `--sync-ms 6` and `--mirror-pack workers` are the
defaults, each still a setting with its other arm (`--sync-ms 0`, `--mirror-pack inline`, the
`U` key). `MirrorConfig::new` packs on the workers by default and keeps no time budget (a
consumer names its frame's share; the budget needs the consumer's sync clock). The pair is the
decision because the budget's residency cost is the budget over the per-chunk cost: the budget
alone misses the 10 % residency clause (+12 %) and the pair meets it (+5 %), with the fill at 5
s and 15 s unchanged in every arm.

## Consequences

### Positive
- The arrival slab lands over frames, not in one: the quiet cave's frames over 33 ms fall by a
  factor of ten to twenty, the castle's by fourteen, the loaded cave's by six, with no new code.
- The sync's p99 halves (17 to 8 ms), which puts `update + poll + sync` p99 at 12.7 to 12.9 ms
  at the tester's 2560 x 1440, under `docs/nfr.md` section 2's 16.6 ms line, where the default
  reads 18.2 to 18.5 ms over it (the line's 14.82 ms reading on record is at 1080p).
- The 2 ms the slot image gives at the sync's p99 ships, and Plan 0107's three conditions are
  read (two met, one on the fill in place of a residency no run reached).

### Negative
- Full residency (the last of the window's 57 777 chunks) lands 0.6 to 0.7 s later on 12.7 s,
  quiet. What the camera sees at 5 s and 15 s is unchanged; the tail is later.
- The budget stops after the chunk that crosses it, uploaded whole, and does not reach the
  frame's table clears and column tiles (a window move's own `copy`, 5 to 9 ms with no brick),
  so the slowest sync reads 10 to 11 ms, not 6. Plan 0111 Phase 6h owns the flush.
- A chunk the camera moves into may be drawn as its LOD record for a few frames more than
  before, since the slab lands nearest first over 3 to 6 frames. Not read on screen yet (Plan
  0111 Phase 7).
- Under a loaded box the residency read +19.9 % later on one unalternated pair whose two runs
  were not at one load; a reading, re-read by 6d with the loads matched.
- Two defaults change at once; the fly-through's and the tester's rows and goldens that name
  them move, and the identity test ADR-0056 owed (a promotion and a demotion with an image held)
  lands with the flip.

## Alternatives considered

### Alternative A: a cap on the chunks a poll absorbs
Bounding `poll` at N chunks a frame would spread the slab one stage earlier and bound `events`
too. It leaves the sync's `copy` of the window move (the table runs and column tiles) where it
is, holds finished chunks in the streamer's done queue for the same frames the budget holds them
pending on the mirror, and is new code where the time budget exists, is tested
(`gpu_mirror_budget`) and is already the sliced start's mechanism. Rejected.

### Alternative B: fewer stream workers
Plan 0111 Phase 2's Hold C read `--workers 4` at 15 frames over 33 ms against 178, because the
slab takes two frames to produce and lands in two polls. It slows every load (full residency
14.97 s against 12.72) and reads 4 of 16 logical CPUs, so the eight-CPU floor would run two.
Rejected.

### Alternative C: the budget alone, without the slot image
Read in Phase 6b: it removes the same frames (15, 16 over 33 ms) but puts full residency 12 %
later and the run's wall 2 s longer; the pair reads +5 % and the same wall as the default. The
image's lower per-chunk cost is what lets 6 ms carry the slab. Rejected by reading.

### Alternative D: ADR-0056's Alternative A as written (no budget)
Plan 0064's reading stands for its day: at about 300 us a chunk a budget starved the window. It
does not hold at 65 us. The rejection is superseded here; the record's Decision (the image on
the workers) is unchanged and its default flips.
