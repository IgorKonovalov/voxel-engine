# Architecture review, 2026-11-26: work off the render thread, after the bake thread

> **Written:** 2026-11-26, on main at `9b1cd85d` (v0.77.0), in the `arch-threads-review`
> worktree. **Read-only:** no cargo, no GPU, no timing run, no perf lock (three dev lanes held
> it). Every figure is a reading already in the tree, named by file, plan and machine, or it
> says "unread", "derived" or "a guess". "RTX host" is the dev box: Ryzen 9 5900HS, 16
> threads, RTX 3080 Laptop. "RENOIR" is its AMD iGPU, the slow adapter.
> **Asked for by the owner:** "I want to invoke architect for a question with bake thread. I
> think it is clearly improved performance and application became more responsive. I want to
> research if similar idea can be used for something else, and whether it make sense at all".
> **Read:** the frame loop (`crates/svet-app/src/shell.rs`, `apps/flythrough/src/run/`,
> `crates/svet-render/src/frame/`), the mirror (`crates/svet-render/src/gpu_world/`), the
> streamer (`crates/svet-world/src/stream/`), the bake thread (`crates/svet-render/src/bake/`),
> the edit, entity, audio and logic paths, `docs/nfr.md` and `docs/nfr/`, Plan 0099 and its
> log, ADR-0017, ADR-0056 and ADR-0083, the backlog.
> **What this is not:** a plan or an ADR. The owner picks first.

## The answer

**The idea makes sense in one more place for certain, in one more place probably, and nowhere
else today.** The bake step was the one large piece of CPU work on the render thread. With
the bake thread on, the render thread's whole CPU frame at a still camera reads 1.49 ms p50
and 2.84 ms p99 on the RTX host (forest, headless 1080p, while the view bakes;
[Plan 0099's log](../plans/logs/0099.md), the thread-on row of its grouped table). There is
no second 20 ms step hiding in a still frame.

What is left over the frame line is three things, and only the first is a threading question:

1. **The mirror's sync while the camera moves** (`update + poll + sync` p99 15.7 to 23.7 ms
   against 16.6). Its largest stage is per-brick packing on the render thread. The fix is
   already decided ([ADR-0056](../adrs/0056-the-mirrors-slot-image-is-packed-on-the-stream-workers-and-the-render-thread-only-allocates-and-copies.md):
   pack on the stream workers), is ripe by the backlog's own rule since 2026-11-23, and is not
   drafted. This is the same idea in a cheaper form: no new thread, the workers that exist.
2. **Start-up** (6 to 7 s of light pre-roll before the first world frame on every start;
   36 s of pipeline creation on a first-ever start on the RTX). The bake thread changes what
   the pre-roll is worth, and nobody has read it since. Two short readings decide it.
3. **A dig's frame** (about 44 ms on the RTX with a deep queue, thread on or off). This is a
   wait the owner chose for the picture. It must not become asynchronous; it gets shorter
   only by a shorter dig step.

Everything else is either already off the render thread, too small to pay for a thread, or
must stay where it is for determinism or for the picture. **No shared worker pattern, no job
system and no `rayon`**: the floor machine has no idle core to give one.

| Rank | Candidate | Expected gain | Risk | The first reading |
|---|---|---|---|---|
| 1 | Slot image packed on the stream workers (ADR-0056) | **measured cost:** `alloc` p99 6.4 to 9.4 ms of a sync p99 of 19.4 (RTX host, cave bench, Plan 0096); **derived gain:** most of `alloc`, `size` and `light_note` leave the frame's tail | low for the picture (bytes identical by test), medium in code (the slot format crosses a thread) | the sync's stages re-read at v0.77.0 with the bake thread on, one glide |
| 2 | The pre-roll shortened, the bake thread finishing behind the first frame | **measured cost:** 5.7 to 7 s of every start (Plan 0103's log); **gain a guess:** 3 to 5 s, if the first picture holds | the picture: an unlit or popping first frame breaks the owner's first rule | two starts: the first world frame after a coarse-only pre-roll, by the owner's eye |
| 3 | Pipelines built on several threads at a cold start, and default-off arms built after the first frame | **measured cost:** 35.5 to 36.7 s cold on the RTX, 11.7 to 12.4 s on the RENOIR; **gain a guess** (whether the driver compiles in parallel is unread) | low | one cold start with the build split over four threads |
| 4 | The acoustics probes on a worker | **measured cost:** 0.74 to 1.03 ms a call at 32 sources (bench, RTX host); unread in the app | low, no picture | the readings line's `acoustics update` ms in a walk with the consumer's source count |
| 5 | The server tick, the logic and the movers on a tick thread | **measured cost:** +0.40 ms (64 walkers), +3.07 ms (64 spiders) of frame CPU p50 (Plan 0079) | high: the consumer owns the loop; ordering against edits | none now; a profile of the spider's tick first (the backlog's entry) |
| - | A second bake thread, or more listers | the feed is 0.6 of its slice | on the floor it starves the frame | the backlog's "The bake thread's feed on the RTX" |
| no | Edits, the support query and collapse | 2 ms a hundred carves; 0.3 to 0.6 ms a query | determinism, results are synchronous by contract | - |
| no | Encode, uniforms, submits, present | inside the 1.49 ms | every submit is ordered | - |
| no | HUD text, hot reload, the profiler, logs | under a millisecond, or a dev tool | - | - |

## 1. What is on the render thread today

One thread runs the winit loop, the fly-through's frame and every `wgpu` call. In call order
(`apps/flythrough/src/run/frame.rs`, `Run::frame` and `prepare_frame`; the shell's redraw in
`crates/svet-app/src/shell.rs`):

| Work | Where | Reads, writes | When | Reading |
|---|---|---|---|---|
| Swapchain acquire, present | `shell.rs` redraw | surface | every frame | **unread** on the CPU: outside `cpu_ms` (which runs from `key_step` to the submit) |
| Input, menu, camera, the walk controller (60 Hz accumulator, at most 8 ticks a frame) | `key_step`, `flight_step`, `fly.rs` | world ray and sweep queries | every frame | unread alone; inside the 1.49 ms |
| Acoustics probes | `audio_step`, `Acoustics::update` | 1 occlusion ray a due voice, 16 reflection rays, 16 of 128 room rays a call; sends commands to the mixer | every frame, a voice due on a 0.1 m move or every 500 ms | 0.74 to 1.03 ms a call at 32 sources, all due (`cargo bench -p flythrough --bench acoustics`, RTX host, `docs/nfr.md` section 9); in the app unread since 2026-10-17 |
| The server tick: inputs, the logic (native or the WASM component, called synchronously), ops applied, the coarse grid, the support query and collapse inside the logic | `tick`, `tick_server`, `svet-net` `server.rs` | writes the world, the logs in memory | once a rendered frame; heavy only on an edit | 100 carves 2.0 ms; tick overhead 85 µs per 100 inputs; the query after a dig 0.49 to 0.62 ms (RTX host, `docs/nfr.md` section 4) |
| Streaming: `update` (sorts the wanted set), `poll` (drains every finished chunk, no cap a frame, inserts under the store's write lock, evicts) | `stream_step`, `svet-world` `stream/mod.rs` | the residency plan, the store's map | every frame while streaming | `update + poll + sync` at rest p50 0.03 to 0.09 ms; in motion p99 15.7 to 23.7 ms (RTX host and RENOIR, Plans 0074 and 0088, `docs/nfr.md` sections 2 and 5). `poll` alone: timed (`stages_ms`), no pinned figure |
| The mirror's `sync`: select by distance to a 2 MiB byte budget, size each chunk, **pack every brick's slot words**, allocate slots, note emitters, place lights, copy through the staging belt | `gpu_world/mod.rs`, `window.rs`, `slot.rs` | chunk write locks, neighbour read locks for bands; the slot tables, free lists, GPU buffers | every frame; work only when chunks arrive or change | cave bench, RTX host, Plan 0096 Phase 7: `alloc` p99 9.4 ms of a sync p99 of 19.4, about 9.5 µs a brick, 13 383 bricks at the slowest sync; at Plan 0096's close `alloc` p99 6.42 ms against a line of 3.0 |
| The bake's render side: notes of the mirror's changes, the message, the write lists applied (at most 1 MiB a frame), one prepared submit issued | `step_bake_queue`, `bake/link.rs` `render_frame` | the store's three GPU buffers | every frame | p50 0.76 to 0.80 ms, p99 1.2 to 1.3 (RTX host), 0.30 to 0.32 and 0.53 to 0.59 (RENOIR) (Plan 0099, quiet, `cde2e966`) |
| The edit's wait: the frame that carries a dig's box blocks on a condition variable until the bake thread has served it | `link.rs` `take_waiting` | - | a dig's frame only | the wait 14.5 and 21.8 ms p50 while the view waits; the dig's longest frame about 44 ms, of which 8.4 to 12.4 ms is not the dig's call (RTX host, Plan 0099) |
| Entities: clips and poses, movers (60 Hz, at most 8 ticks a frame, `svet-sim` against the world), the ring placement's rays, instances packed and written | `configure_frame`, `entities/` in the app | world queries; one instance buffer | every frame | 64 walkers +0.40 to 0.42 ms, 64 spiders +3.07 ms of frame CPU p50 (Plan 0079); 64 sixteen-part entities `sample` 0.14 ms and the display blend 0.50 ms (Plan 0063), both RTX host, loaded |
| Lights list, sky, uniforms, bind groups, the pass recording, the HUD's cells, the submit | `configure_frame`, `frame/encode.rs`, `hud_feed.rs` | uniform and storage buffers | every frame | no stage timer on `configure_frame`, `encode` or the submit; together inside the 1.49 ms. The HUD's formatting has one (`format_us`), no pinned figure |
| A pipeline built on its first use | `frame/pipelines.rs` `pipeline` (a `OnceLock` per pass) | - | mid-run: a light arm picked live, the counting variants, the first sky, an upscaler change | `absorb`'s cold start 70 s (Plan 0094's window); the cold light step 130 to 210 ms (backlog) |
| Blocking read-backs | the steps and cracks views, the probes' states, frame 0 | - | only with that view or reading on | the steps view about 14 fps (backlog) |
| File polls (materials, rig and vox models), stdout lines | `frame.rs`, `viewer.rs` | a stat, then a read on change | once a second | unread; trivial by construction |

**Start-up and a scene switch.** World generation runs on spawned threads and the renderer's
build (`build_gfx`: every non-lazy pipeline, the entities, the bake) on a "start-up build"
thread while loading frames draw. The loading frames themselves step the upload, the
residency and **the light pre-roll in lockstep on the render thread**, in slices, and wait
for the GPU every frame. Readings (Plan 0103's log, loaded, indicative): with warm pipelines
the first world frame comes at 6.5 to 9.5 s on the RTX and 6.8 to 7.1 s on the RENOIR, 5.7 to
7 s of it the pre-roll, against a 3 s proposal; cold, the pipeline span is 35.5 to 36.7 s and
11.7 to 12.4 s. A pre-roll bake step runs 70 to 140 ms against a 25 ms slice, so loading
frames run near 24 fps (Plan 0075). A scene switch tears down and joins the world's build on
the render thread after one loading frame: unread.

**Already off the render thread:** chunk generation, prefab stamping at generation time, LOD
mips and the bake's face masks (stream workers, half the logical cores); the bake step and
its listing pool; the audio mix and HRTF (the device callback); entity icons; structure
builds in the menu. There is no meshing: the march reads the mirror.

Two things the walk turned up that are not threading questions:

- **The frame's CPU figure does not cover the acquire or the present**, and no timer brackets
  `configure_frame`, `encode` or the submit. Plan 0091's scopes close that; until then "the
  frame CPU" is `key_step` to the submit.
- **The fly-through ticks the server once a rendered frame**, not on a fixed step (the walk
  controller and the movers do run on 60 Hz accumulators). That is the testbed's choice and
  the engine reads no clock, but a consumer who copies the testbed's loop gets a tick rate
  that follows the frame rate. A sentence for the consumer's guide.

## 2. Why the bake thread worked, and the test for a second one

The bake step paid on every count at once:

- **Long against the frame:** 28 ms of step beside 11.6 ms of GPU, every frame for half a
  minute.
- **While the user moved:** the fresh start and every flight.
- **A narrow read set:** the view, the mirror's change list and a few membership questions,
  shadowed in one small struct; voxels read through the per-chunk locks the workers already
  used.
- **A write set that could be listed:** bytes in three GPU buffers, applied in the order
  made.
- **Late is not wrong:** rule 6's coarse answer covers a record that lands a frame late, and
  a block's words and its table word share a list, so no frame reads half a landing.
- **One owner:** nothing else wrote the queue's books, so ADR-0017's objection (a rule's
  owner split across threads) did not arise.

And it cost what [ADR-0083](../adrs/0083-the-live-bake-queue-runs-on-a-bake-thread-the-render-thread-applies-its-write-lists-and-issues-its-submits.md)
said it would: a lockstep mode kept for every test, a snapshot for every public read, one
blocking wait for the edit, and a frame's landing that is no longer reproducible run to run.

A candidate that misses "long against the frame" does not pay for those costs. Most of the
table in section 1 misses it.

## 3. The candidates

### 3.1 The mirror's packing, on the stream workers (rank 1: do it)

`sync` packs each arriving chunk's bricks into slot words on the render thread, sizes them,
and walks their emitters. A stream worker has just generated that chunk and already computes
its face masks for the bake (`FacePrep`, the `ChunkPrep` hook). ADR-0056 (accepted
2026-11-12) puts the slot image, the sizes and the emissive samples in that same hook; the
render thread keeps allocation and the copy, so the reservation and deferral rules keep one
owner. The image is a pure function of the chunk's content and the material table, so the
mirror's bytes are the same whichever thread packed them, and a test says so.

Against the test: long against the frame in the tail (p99, not p50: at rest the sync is
0.04 ms); runs exactly while the user moves; the read set is one chunk the worker owns; the
write set is bytes; late is already normal (a chunk is mirrored hundreds of milliseconds
after it is wanted). It fits better than the bake step did, because it needs **no new thread
and no shadow**.

- **Gain (derived from readings):** the stages that leave are `alloc`'s brick loop, `size`
  and `light_note`. On the RTX host's cave bench `alloc` was 48 % of the sync's p99 (9.4 of
  19.4 ms, Plan 0096 Phase 7). What stays: selection, allocation, the light placement, the
  copy. Whether the tail lands under 16.6 ms is not derivable; the plan's first phase reads
  it.
- **Risk:** the slot format crosses a thread; a chunk edited after its image was made takes
  today's path for its dirty bricks only (ADR-0056 says so); the field surface's band repair
  reads neighbours and stays on the render thread. An edited chunk's cost does not change.
  The picture does not change at all.
- **New since ADR-0056 was deferred:** the bake thread and its listers now read chunks under
  the same locks the sync takes for writing. `SyncStats::lock_us` counts that wait and has no
  reading with the thread on. Packing on the workers shortens the time the sync holds a
  chunk's write lock, so the two meet less.
- **The first reading (not taken):** the tester's cave bench and one forest glide, headless,
  release, the bake thread on, at v0.77.0: the `sync:` stages' p99 with `lock`, and `poll`'s
  p99 from `stages_ms`. Two runs of about a minute under the lock. If `alloc + size +
  light_note` is still about half the tail, the plan goes; if `lock` or `poll` has grown past
  them, the plan's order changes.

### 3.2 Start-up: the pre-roll and the cold pipelines (rank 2 and 3: read first)

**The pre-roll.** It exists so the first world frame is lit. It runs in lockstep on the
render thread because a pre-roll must be reproducible and was written before the thread. With
the thread on, the forest's every in-view chunk has some light about 8 s after the first
frame and the frame runs at 8.3 ms meanwhile (Plan 0099). So the question the backlog parked
"until Plan 0099 closes" is now open: **how short can the pre-roll be if the bake thread
finishes behind the first frame?** The gain is a guess of 3 to 5 s of the 6 to 7. The risk is
the owner's first rule: a first picture that is dark or that pops as records land is worse
than a longer bar. This is decided by the owner's eye, not by a derivation (the pop counter
that would count it is Plan 0090's and is not built).

- **The first reading (not taken):** two starts on the forest and the hall, the pre-roll cut
  to its coarse tier, the thread on: the time to the first world frame and the owner's look
  at the 10 s after it. Under five minutes. No flag was found that shortens the pre-roll's
  stages (`--pre-roll-drain` lengthens the bake after the first frame, the opposite), so the
  reading needs a small switch; that is the chore's one piece of code, a setting.

**The cold pipelines.** The build is already off the render thread; it is one thread
creating pipelines one after another. `wgpu`'s device is shared across threads, so the build
can be split. Whether the NVIDIA and RADV compilers then run in parallel is **unread**; the
gain is a guess between nothing and the core count. The same split serves the backlog's
"default-off arms built in the background after the first world frame": a light arm picked
live compiles on the render thread today, and the old pipeline can keep drawing until the new
one is ready, shown as a mode line on screen. Late is safe there: the picture changes when
the pipeline is ready, as it would have anyway.

- **The first reading (not taken):** one cold start (the driver's cache cleared) with the
  non-lazy passes split over four threads, against the serial 36 s; the per-pass times
  already print. A few minutes a run. Parked Plan 0103 (the cache) is beside this, not
  instead of it: a cache does nothing for a first start.

### 3.3 The acoustics probes (rank 4: backlog)

Clean on every count but size. The probes read the world through integer rays, write
commands into a channel that already exists, carry no determinism contract, and a result
half a second late is already the design. A probe worker was named as a follow-up when the
update was measured. But the cost is about 1 ms at 32 sources all due, and a voice is due
only when it moves or twice a second; the fly-through plays a handful. **A thread for 1 ms is
not worth one of the floor's eight logical CPUs.** If the consumer's scenes carry dozens of
moving sources, the cheaper first step is the other follow-up named there (a chunk handle
cached across a ray's steps), then a worker.

### 3.4 The tick, the logic and the movers (rank 5: not an engine thread)

The measured cost is real: 64 spiders add 3.07 ms of frame CPU, over the whole headroom line.
The classic answer is a simulation thread with the frame blending between ticks, and the
blend exists (`DisplayPose::between`). Two things stop it being an engine change now:

- **Ordering.** Movers read the world that ops write. `svet-sim` is deterministic only if a
  tick's queries see the world between ops, never during one. So the movers cannot leave
  alone; the tick, the logic and the movers leave together or not at all. That is the
  consumer's loop, which the engine does not own (a library, not a framework). When
  `svet-net` gets its real transport the server is another process anyway.
- **The cost has not been attributed.** The backlog's entry suspects allocation in the pose
  work, not the sweeps. A profile comes before a thread.

What the engine owes here is small: `World`'s queries are already safe from another thread
(the bake thread is the proof), and the guide should say so.

### 3.5 More of the bake on more threads (no)

The bake thread feeds 0.6 of its slice on the RTX because a step takes 10.5 to 11.1 ms
against a frame of 8.3. A second bake thread splits the books' one owner, which is what made
the first one simple. The levers are inside the step (the backlog's "The bake thread's feed
on the RTX": plan a submit per frame spanned, keep the snapshot's sets, walk the open jobs
once). On the floor's stand-in the feed already drops to 0.52.

### 3.6 What must not move

- **Edits.** An op returns what it changed, synchronously, and ops are ordered; that is the
  API and the determinism contract (`docs/nfr.md` sections 4 and 6). The cost is 2 ms a
  hundred carves. Nothing to gain, the contract to lose.
- **The support query and collapse.** They run inside the logic's tick so the collapse is
  part of the same tick's ops. Read at 0.3 to 0.6 ms. One caveat for a later plan: a single
  query has no budget but the changed-cell cap (129 cubed) and, under WASM, fuel; a very
  large edit can make one long tick. That is a slicing question for the logic, not a thread.
- **The edit's wait.** The dig's frame waits so that no frame shows a hole without its
  record. Making the dig asynchronous is exactly the pop the owner's first rule forbids, and
  he chose the wait (Plan 0099, question 8 (a)). About 32 to 35 ms of the 44 is the dig's own
  bake call; it shortens by a shorter dig step, which is the territory of Plan 0105 and the
  feed follow-on.
- **Every submit, the encode, the uniforms, the present.** `wgpu` orders work by submit, the
  mirror's copies must precede the march that reads them, and ADR-0083 keeps every submit on
  the render thread for that reason. Together they are inside 1.49 ms.
- **The mirror's allocation and deferral.** One owner, on the render thread (ADR-0017). The
  slot image moves; the rules do not.
- **Streaming's insert.** A pointer swap under the store's lock. It has no cap a frame, which
  is worth a reading (3.1) but not a thread.

### 3.7 Not worth it

- **The HUD's text:** formatted into kept buffers, one cell a character, no allocation by
  test. Under a millisecond.
- **Shader hot reload (Plan 0092):** a developer's tool; a 400 ms hitch on a save is the
  feature working. An error keeps the old pipelines.
- **The frame profiler (Plan 0091):** it moves nothing. It is the instrument that would have
  made this review a reading instead of a walk: scopes on every thread. Any plan from this
  review is easier after it, and none needs to wait for it (the sync's stages have clocks
  already).
- **Logs and reports:** the op and input logs are in memory; `run.json` and stills are
  written at exit.
- **The blocking read-backs** (the steps view at 14 fps): the fix is the non-blocking
  read-back ring the timers use, not a thread.

## 4. The general shape

**Do not lift a worker pattern out of the bake thread yet.** Its parts are a coalescing
inbox, a bounded outbox of ordered lists, a wait for one message, a shadow, a panic carried
to the frame and a join on drop. None of the candidates above would use them:

- the slot image rides the stream workers' existing results (`Done::Full` already carries
  the prepared masks);
- the pipelines are a one-shot build and a `OnceLock`;
- the acoustics would use the mixer's command channel.

A mechanism with one user is that user's code. A second user would need a typed message that
coalesces, a bounded result queue that makes the worker wait, the panic's text, and a
lockstep mode; when one appears, lift exactly those four.

**Two rules are worth lifting now, as rules, not code:**

1. **Every thread has a lockstep mode, and tests run in it.** The same code on the calling
   thread, as a setting. This is what kept every golden and every headless reading
   reproducible through Plan 0099, and it is the path for a target with no threads.
2. **Results cross as ordered data and the render thread applies them at one fixed point.**
   No thread but the render thread writes what the image reads.

**No job system and no `rayon`.** `rayon` is not a dependency (the only async runtime in the
workspace table is `tokio`, pinned for the WASM host's stream traits, behind a feature). The
fan-outs that exist use `std::thread::scope` and two kept pools. A shared pool with
priorities is the eventual answer if three or more pools compete, but it means rewriting the
streamer's queue and the lister together, for no reading that asks for it.

**The floor has no idle core.** The thread counts are written for the floor to come out
exactly full: workers are half the logical CPUs (`default_workers`), listers are the rest
less two (`lister_threads`), plus the render and bake threads. On 8 logical CPUs that is
4 + 2 + 1 + 1 = 8; on the RTX host's 16 it is 8 + 6 + 1 + 1, and Plan 0099 counted 24 threads
in the process. Its pinned run (4 cores, 8 logical CPUs, a stand-in, not the floor): the
frame 8.6 ms against 8.3, the feed 0.52 against 0.62, and the pre-roll's 5 s ended with 4 328
chunks resident against 17 077. **So the frame survives the floor and the background work
does not keep up.** Any design that adds a dedicated thread assumes an idle core the floor
does not have; say so in its plan, and prefer work that rides a pool that exists. That is
why rank 1 is rank 1.

One loose end the walk found: the two formulas live in two crates (`apps/flythrough` and
`svet-render`) and agree by comment only. A consumer who picks a different worker count
breaks the sum silently.

## 5. Recommended next steps

1. **Draft the slot-image plan under ADR-0056**, its first phase the reading in 3.1. It
   touches `gpu_world/`, the streamer's prep hook and `bake/faces.rs` (where `FacePrep`
   lives), so its lane starts **after Plans 0101 and 0105 merge**; the draft and the Phase 1
   reading can be made beside them. It also takes the glide's `light place` and closes the
   backlog's "The sync's `alloc` stage", as the backlog already routes.
2. **One short start-up reading**, as a chore whose only code is a setting that shortens the
   pre-roll: the coarse-only pre-roll with the thread on (3.2), with the owner's look. It may run beside Plan 0093 (which shoots
   stills and does not touch the bake). The cold-pipeline split is the second half, before
   the next tester archive. A start-up plan follows only if the readings say so.
3. **Nothing now** for the acoustics worker, the tick thread, a second bake thread, a worker
   pattern or a job system. Each has a backlog entry with its pick-up condition.

The roadmap's order does not move: 0101 and 0105, 0093, then 0091. Step 1's lane fits after
0101 and 0105 and before or beside 0091.

## 6. Owner questions

1. **The slot image on the workers: draft the plan now?** (a) **Yes, recommended:** drafted
   now with Phase 1 the re-read, the lane after 0101 and 0105 merge. (b) Take the re-read
   alone first and draft only if it confirms. (c) Leave it parked.
2. **The start-up reading: take it?** (a) **Yes, recommended:** the pre-roll's reading now
   as a chore beside 0093, the cold-pipeline split before the next tester archive. (b) Only
   the pre-roll. (c) Neither until a tester complains about the start.
3. **A shared worker pattern, a job system or `rayon`?** (a) **None, recommended:** write
   the two rules of section 4 into the architect's best practices and the thread count's
   sum into one place, at the next plan that touches a thread. (b) Lift the mailbox now.
4. **The acoustics worker and the tick thread:** (a) **backlog, recommended**, picked up
   when the consumer's scene asks (dozens of moving sources; creatures on the server tick).
   (b) Spike the acoustics worker now as the cheap second user.
5. **The dig's frame (about 44 ms):** (a) **leave the wait, recommended:** it is the
   picture's guarantee; the cut is the dig step's length, with Plan 0105 and the feed's
   follow-on. (b) Reopen question 8 of Plan 0099.

**The owner's answers (2026-11-26): "all recommended".** 1 (a): drafted the same day as
[Plan 0107](../plans/0107-the-mirrors-slot-image-packed-on-the-stream-workers.md), Phase 1 the
re-read, the lane after Plans 0101 and 0105 merge. 2 (a): drafted as
[Plan 0108](../plans/0108-the-start-up-readings-a-shorter-pre-roll-and-the-cold-pipelines-split.md),
the pre-roll reading beside Plan 0093, the cold-pipeline split before the next tester archive.
3 (a): the two rules, and a third (no job system or thread pool dependency without an ADR),
are in the architect's best practices; the thread count's sum in one place is Plan 0107's
Phase 7. 4 (a): the backlog, as written. 5 (a): the wait stays. Both plans are drafts and
await his go.
