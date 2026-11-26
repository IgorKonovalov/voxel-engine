# ADR-0083: The live bake queue runs on a bake thread; the render thread applies its write lists and issues its submits

> **Status:** accepted (2026-11-26, at Plan 0099's close, v0.76.0, with an Outcome; proposed 2026-11-22 with Plan 0099; amended while proposed, 2026-11-25: the
> edit's frame waits, the owner's answer to that plan's question 6; lists apply in the order
> made, an edit's with those before it; 2026-11-26: the wait ends on the step's lists, the
> books' snapshot follows)
> **Date:** 2026-11-22
> **Related plan(s):** [Plan 0099](../plans/done/0099-the-bake-step-off-the-main-thread-a-bake-thread-write-lists-and-the-in-view-drain.md).
> Extends [ADR-0017](0017-mirror-cpu-work-bounded-in-the-frame-loop-narrowing-on-stream-workers.md)
> (the mirror's rule stands; the bake queue, which that ADR predates, gets its own owner
> thread). Rests on [ADR-0046](0046-static-emissive-light-is-baked-per-chunk-when-a-chunk-loads-or-changes.md)
> (the store), [ADR-0048](0048-the-live-queue-orders-by-the-view-and-withdraws-the-work-a-flight-has-passed.md)
> (the order), [ADR-0080](0080-the-live-queue-bakes-what-the-image-reads-a-record-by-rule-a-tier-by-size-on-screen-and-a-drain-slice.md)
> (the tiers and the drain slice) and
> [ADR-0076](0076-light-stability-rules-world-history-per-source-a-fade-per-transition-and-a-ratcheted-pop-counter.md)
> rules 3 and 6. [ADR-0056](0056-the-mirrors-slot-image-is-packed-on-the-stream-workers-and-the-render-thread-only-allocates-and-copies.md)
> (slot words on the workers) is a separate follow-up and is not touched.

## Context

The live bake queue's step runs on the render thread, once a frame. Plan 0096 took the queue's
depth out of it (the drained step reads 0.05 to 0.07 ms) and gave the queue an end, and its
window on 2026-11-22 (RTX 3080 Laptop, release `1844a99b`, 2530 x 1378, the forest from a
fresh start) read what is left: the frame's CPU 28.0 ms p50 against 11.6 ms of GPU march, the
step in regime B 28.0 / 51.1 ms p50/p99 over 465 steps, the owner's "works, but fps around
30", and on the hills "fully settled in around a minute or so if I am looking for stable fps
(70+)". The cost is per job opened, not per chunk waiting: headless at 1920 x 1080 (Plan
0096's readings log, Phase 5, `e3b03e75`, loaded) the stage means of a regime B step read
`list` 10.0 to 18.1 ms, `collect` 5.0 to 7.0, `submit` 5.0 to 8.1, `fades` 4.3 to 7.5 and
`summaries` 1.6 to 2.6, together 25.9 to 43.3 ms against a proposed line of 1.0 ms p50.

Two of the owner's rules meet here. A stable image is the one priority, so no fix may let a
frame read a half-landed record. And there is no waiting for light (ADR-0076 rule 6): the
drain must not get longer to buy the frame rate back.

ADR-0017 kept the mirror's CPU work in the frame loop and rejected a mirror-prep thread
because it would split the one owner of `sync`'s reservation and deferral rules across two
threads. It names the lighting work as the next cost on the same line and says the rejected
alternatives come back if the bounded version is not enough. For the bake queue the bounded
version has been built (Plans 0067 and 0096) and is not enough.

Three facts about the code decide the shape. The queue's books (the candidate pool, the open
jobs and their sample states, the store's block map, its allocator, its CPU copy of every
block, the fades, the summaries) have one owner today and no other system writes them. What
the step reads of the frame is small: the view, the mirror's change list (already produced
once a step), a handful of membership questions to the mirror, and world chunks behind their
own locks (the listing already reads them from scoped threads). And what the image reads of
the store is bytes in three GPU buffers, written by `Queue::write_buffer`; a staged write
goes out with the next submit from any thread.

## Decision

**The live queue's books and all of its per-job CPU work run on one dedicated bake thread.
The render thread keeps every `wgpu` submit and every write to a buffer the image reads, and
does them at one fixed point in the frame, from lists the bake thread prepared.**

- **The bake thread owns** the queue and the store's books: the order and the opening, the
  listing, the planning of a submit's rounds and its entry table, the read-back's accumulation
  and the blocks it closes, the placement, the fades' mixes and the summaries and parents.
  Nothing else writes that state.
- **The render thread sends one message a frame and never waits for an answer.** The message
  holds the view, the frame's number, the mirror's changes since the last message with each
  changed chunk's new state (held full, LOD-only, gone, its lights ready), the store's window,
  the read-backs that mapped, and the GPU time of the submits read. The bake thread keeps its
  own shadow of the mirror's membership from those changes; it never reads the mirror. The
  inbox coalesces: a newer view replaces an older one and change lists append, so a stalled
  bake thread costs the render thread nothing and the inbox is bounded by the residency.
- **One exception: the edit's frame waits for the dig** (owner, 2026-11-25). A frame whose
  message carries an edit's box does not return from the bake call until the bake thread has
  served that message, and applies that list in the same frame, as lockstep does: the dug
  bricks' held record is in the store before that frame's light pass reads it, so no frame
  shows a hole without it. The wait is the rest of the step in progress, which admits no
  further candidate once a box is sent, plus the dig's own step. It has no timeout; it ends on
  the list, on the thread's panic or on the stop. That frame also issues the dig's submit
  behind every submit planned before it and not yet issued (the one the step before planned,
  and one carried from an earlier frame), so it issues two as a rule and at most the thread's
  depth. No other frame waits, and a dig's read-back crosses as every read-back does. The
  wait ends on that step's lists and submit: the snapshot of the books the public reads
  consult is made after them and crosses as its own entry, and the render thread's mailbox
  answers by the older of the lists applied and the books held (amended 2026-11-26: the
  snapshot read 1.2 to 1.4 ms of the wait on the RTX, and nothing in the frame's picture
  reads it).
- **Results cross as write lists.** One bake step emits one ordered list of buffer writes
  (a block's words, its table word, the summary and parent words, a fade's shown words, the
  store's window when it moves) and at most one prepared submit (the planned rounds and the
  entry table's bytes). The render thread applies whole lists, in order, each inside one frame,
  up to a byte cap the bake thread sizes its lists to; what does not fit waits a frame. **An
  edit's list is never carried, and never passes a list made before it: the frame it reaches
  applies every list up to it, whatever the cap** (amended 2026-11-25: applied first, it was
  written over by an older step's writes to the same chunk). It then
  encodes and issues the prepared submit after the frame's sync, where the step issues it
  today. **A block's words and its table word are always in the same list, so no frame reads
  one without the other.** The outbox is bounded and the bake thread waits when it is full.
- **A result is checked against the changes in the order the render thread saw them.** A
  submit is stamped with the frame it went out in; its read-back is placed only after the
  bake thread has taken every change list up to the frame the read-back mapped in, so a chunk
  changed in reach while the job flew goes stale exactly as it does today.
- **A fade is paced by the render thread's frame number, not by bake steps.** The bake thread
  mixes each fading block for the newest frame it has heard of. A late bake step makes one
  larger step of the same fade; the shown value never moves back and ends on the target
  (ADR-0076 rule 3 holds).
- **The same bake-side code runs in lockstep** (one bake step to completion inside the frame's
  call, on the calling thread) as a setting. The pre-roll, a one-shot bake, a dig's ring where
  the consumer asks for it at once, and every test or headless run that asserts bytes before
  the queue has drained use lockstep. A window defaults to the thread.
- **`svet-world` is untouched in kind.** The bake thread reads chunks through the per-chunk
  locks the stream workers already use and holds one for no longer than one chunk's listing.
  The world gains no GPU type, no clock and no bake concept.

## Consequences

### Positive
- The render thread's share of the bake becomes copies and one submit, bounded per frame by a
  byte cap. The frame while the queue drains is paced by the GPU, which is where the owner's
  70+ fps is (the march read 11.6 ms at his window).
- The drain slice is granted per frame, so a frame rate that doubles grants the bake twice the
  GPU time a wall second. The drain gets shorter if the bake thread keeps the submits fed
  (Plan 0099 Phase 4), not longer.
- The store's bytes do not depend on the mode. A record is a function of the chunk's content
  in reach, its tier and its sample sequence; the thread changes when it lands, not what
  lands. ADR-0046's bit-identity is asserted across lockstep, the thread and a one-shot bake.
- The render side holds no rule of the queue's, only a list to apply. ADR-0017's decisive
  objection, a rule owner split across threads, does not arise.
- The lockstep mode keeps every existing test, golden and reading rig deterministic frame by
  frame, and is the path for a target with no threads.

### Negative
- **With the thread on, which frame a record lands in is not reproducible run to run.** Any
  assertion on the undrained image (the pop counter of Plan 0090, a golden taken before the
  drain, a path report's frame-N hash) must run in lockstep or assert only drained states.
  A reading taken in lockstep does not measure the mode the owner sees; every perf reading
  names its mode.
- A frame of latency, or more under load, between a mirror change and the queue's reaction to
  it, and between a block being ready and the image reading it. Rule 6's coarse answer covers
  the wait; a dig's ring is the case where it shows, and it gets a priority path and a bar.
- A dig's frame is longer by the edit's wait, on the render thread, and a bake thread that
  stalls stalls that frame with it. The wait is read and bounded by stops in the plan, not by
  a timeout.
- The store splits into books on the bake thread and buffers on the render thread. Every
  public read of the books (statistics, a block's words, the bake file, Plan 0089's mirror
  oracle) becomes a snapshot request or a lockstep call. That is an API change.
- One more thread on a floor of 4 cores and 8 threads, beside the stream workers and the
  listing's fan-out. More runnable threads than cores has cost the frame's tail before (the
  tester's `--workers` runs). No floor machine is on hand; a pinned run on the dev box stands
  in and says so.
- The bake thread holds chunk read locks while the main thread may want a write lock for an
  edit. The wait is bounded by one chunk's listing (about 1 ms, a reading from Plan 0096);
  it is a new way for an edit to stall and is measured.
- Memory: block words are copied into write lists (bounded by the outbox's cap), and the
  shadow membership is one small entry per resident chunk. Both are stated as guesses in the
  plan until read.
- The render thread still pays the `write_buffer` copies. ADR-0017's Outcome found the copies
  alone over the streaming line for the mirror. If they are over the line here too, the byte
  cap holds the frame and the landings wait, which lengthens the drain; the next lever
  (the bake thread fills a mapped staging belt, ADR-0022's mechanism, and the render thread
  only encodes copies) is named in the plan and needs no new decision.

## Alternatives considered

### Alternative A: spread the step over the existing stream workers

ADR-0017's narrowing and ADR-0056's pattern: pure functions of a chunk's content run on the
workers at load. The workers already prepare a chunk's face masks and the no-face verdict
(Plan 0096 Phase 2). Moving the whole listing there takes 10 to 18 of the 26 to 43 ms. It
lost because the rest is not a pure function of one chunk: the read-back's accumulation, the
round planning, the placement, the fades and the summaries are all state of the queue's
books, 16 to 25 ms that would stay on the render thread, sixteen times the line. A face list
made at load is also stale when a neighbour arrives and is made for chunks the queue never
bakes. The workers keep what they do today; the bake thread may fan its listing out
(Plan 0099 Phase 4).

### Alternative B: cap what a step opens, on the render thread

Bound the step by time instead of moving it: open candidates until 1 ms is spent. A job costs
about 1 to 2 ms of step to open and carry, so a 1 ms cap admits under one job a frame, and the
fades and the collect are paid per frame whatever was opened. Plan 0096's Phase 4 amendment
already found the opening's fixed cap starving the submits (0.60 to 0.69 of the granted slice
drawn at the per-second rig). It lost because it moves the cost from one of the owner's
complaints to the other: the frame rate comes back and the drain, 16 to 22 s in view at the
window against a proposed 10, gets longer. The idea survives where it belongs, as the byte
cap on what the render thread applies a frame.

### Alternative C: amortise the listing across frames

List a chunk incrementally, or keep its face list from one opening to the next, so no step
pays a whole listing. It attacks the largest single stage and nothing else: with the listing
free the step still reads 16 to 25 ms. It lost on that arithmetic. A kept face list also
costs memory per waiting chunk and must be dropped whenever a neighbour changes.

### Alternative D: a free-running bake thread that also submits and writes

`wgpu`'s device and queue are `Send + Sync`, so the bake thread could issue its own
dispatches and `write_buffer` calls and the render thread would do nothing at all. It lost on
image stability. A staged write goes out with the next submit from either thread, so a
block's words and its table word could land in different frames, and one frame would read a
table word whose block has not arrived: a pop, by construction, at a rate nobody could
reproduce. The bake's dispatch could also go out between two halves of a mirror sync. Both
can be fenced with a lock around the queue, which is a lock the render thread waits on.

### Alternative E: fork and join inside the frame

Run the step on a second thread while the render thread syncs and encodes, and join before
the frame's submit. No protocol, no latency, the frame stays deterministic. It lost because
the frame still waits for the step: a 28 ms step joined in a frame is a 28 ms frame. It
overlaps a few milliseconds of sync and no more.

### Alternative F: an edit's box takes the path every message takes

The render thread never waits, with no exception: the box goes out with the frame's message
and the held record comes back in a later frame's list. No hitch on a dig. It lost on image
stability: until that list is applied the hole's new faces read the old block, which holds no
record for them, the picture the owner rejected before the held record existed (Plan 0098's
windows 98c and 98d), for a few frames a dig. Its sibling, **the whole step back in lockstep
while a dig waits or bakes**, keeps the picture and lost on cost: about 28 ms a frame in a deep
queue for as long as the dig's rings take, where the wait is one frame's few milliseconds
(a guess of 3 to 6 on the RTX 3080 Laptop until Plan 0099 Phase 3b reads it).

## Outcome

**Accepted 2026-11-26, at Plan 0099's close (v0.76.0).** The decision stands as amended while
proposed: one bake thread owns the queue's books, the render thread applies write lists in the
order made and issues the submits, the same code runs in lockstep, and an edit's frame waits
for its step's lists. The owner at the plan's window (2026-11-26, RTX 3080 Laptop at Floor and
RADV RENOIR at Portable, release `cde2e966`): "with thread it got so much better, this is the
way!"; the image "all good", the dig "good as well", the RENOIR "nope, fps better". Every
figure below is a quiet reading at `cde2e966` unless marked, pinned in `docs/nfr.md` sections 2
and 4 with its rows in `docs/plans/logs/0099.md`.

What the implementation confirmed:

- **The render thread's share is copies and one submit.** With the thread on it reads p50 0.76
  to 0.80 ms and p99 1.2 to 1.3 on the RTX forest and hills, 0.30 to 0.32 and 0.53 to 0.59 on
  the RENOIR, against the step's 28.0 / 51.1 ms at Plan 0096's window. The frame while the
  view waits is 8.3 ms on the RTX forest (21.9 in lockstep) and 24 ms on the RENOIR (33.7).
  The copies fit the line; the staging belt named under "Negative" was not needed.
- **The store's bytes do not depend on the mode**: the drained store is bit-identical across
  lockstep, the thread and a one-shot bake (`gpu_queue`), and no golden moved.

What it falsified or changed:

- **"The drain gets shorter if the bake thread keeps the submits fed" holds only in part.**
  One thread does not feed the RTX's slice: the submits carry 0.58 to 0.61 of the granted
  drain slice against the plan's bar of 0.85 (the RENOIR 1.00). Two factors, both read: a bake
  step takes 10.5 to 11.1 ms from one pick-up to the next against a frame of 8.3, with no wait
  in it, and with every table row held a submit carries 0.66 to 0.87 of its cap. The forest's
  view is final at 17.9 and 19.0 s against a line of 10 (31 s in lockstep; every chunk in view
  has some light at about 8 s), the hills' at 6.5 s. Three fixes missed their derivations
  inside the plan (the opening's level, the depth in flight, more table rows) and the plan
  stopped there, disclosed; the levers left are in `docs/backlog.md` ("The bake thread's feed
  on the RTX").
- **"One bake step emits one ordered list and at most one prepared submit" became several
  lists a step, each under the frame's cap** (1 MiB), an edit's list between the landings made
  before and after it, and a unit larger than the cap sent whole as a list that is never
  carried. A frame issues one submit and carries the rest to the next; the thread plans to a
  depth of 3 submits unread.
- **The edit's wait is not "a guess of 3 to 6 ms".** A dig's frame while the queue is deep
  reads about 44 ms with the thread on against 45 in lockstep on the RTX (the frames around it
  8.4 against 21.9), about 20 against 32 once drained; the RENOIR 48 against 43, and 28 against
  34. The light at the dig lands sooner with the thread (about 170 ms against 230 on the RTX).
  The owner chose to keep the wait as built (the plan's question 8, (a)). Not resolved: whether
  the RENOIR's wait while the view waits is equal to lockstep's call or over it (10.6 and 12.6
  ms against 10.9 and 9.3, two runs a side).
- **A dig can find every table row with rounds in flight**, and ring 1 then lands one round
  trip later (3 of 12 digs on one RTX timeline). Disclosed, not fixed; the lever is the
  owner's (a point to leave a step at).
- **A window defaults to the thread; a profile does not set it.** `bake_thread` is off in
  every profile's row and under `--headless`, so every test, golden and headless reading stays
  in lockstep unless it names the thread.
- **The public reads became a snapshot** as "Negative" said: `Frame::bake_queue_mut` is `None`
  while the thread runs, a caller's write goes through `Frame::set_bake_queue`, and reads
  answer from the books' snapshot the thread makes after each step (1.2 to 1.3 ms a step on
  the RTX, part of why the step is longer than the frame).
- **Memory was not read.** The outbox is bounded by construction at 8 MiB; its peak and the
  bytes of the books' snapshot have no line (`docs/backlog.md`, "Plan 0099's readings not
  taken"). The floor's CPU has a pinned stand-in only: on 8 logical CPUs of the dev box the
  feed reads 0.52 against 0.62 unpinned, the share and the frame unchanged.
- **The inbox's bound is the bake thread's pace.** The change list is bounded by the
  residency as stated; the boxes, prepared masks and read-backs a coalesced message appends
  are not, and grow for as long as a step stalls (not read; the backlog's "The bake thread's
  leftovers").

The flash the owner sees when digging deeper than 2 to 3 m is the same with the thread off, by
his word; it is not this decision's and goes to Plan 0105.
