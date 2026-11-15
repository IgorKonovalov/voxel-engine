# ADR-0059: Loading runs as time-sliced steps on the app's frame loop, behind an engine loading frame the app can replace

> **Status:** proposed
> **Date:** 2026-11-15
> **Related plan(s):** [Plan 0075](../plans/0075-a-loading-screen-steppable-start-up-a-progress-signal-and-a-default-loading-frame.md)

## Context

The fly-through does all its start-up work inside `App::init` (`apps/flythrough/src/run/mod.rs`,
`Run::init`): the mirror's first sync of a level, or the streamed world's loop to near
residency, then the light pre-roll (`Run::queue_pre_roll`, `apps/flythrough/src/run/queue.rs`),
a loop of `mirror.sync` and `Frame::bake_queue_step` that ends when the camera's near chunks
hold their light and the coarse pass is done, boxed at 5 s past the near chunks and capped at
120 s. `vxe_app`'s shell calls `init` from winit's `resumed` and requests the first redraw only
after it returns, so the window shows nothing, and winit's event loop is not pumped, for the
whole of it: on a streamed cave run the pre-roll alone read 3.6 s to the near chunks and 8.6 s
to the end of the coarse pass, ended by its 5 s box (`docs/plans/logs/0060.md`, Phase 1,
run 1). The tester (ADR-0054) starts the fly-through as a child per scene, so every tester run
opens on a blank, unresponsive window.

The owner asked (2026-11-15) for a loading screen with an animation and a progress bar, in the
tester build and for the engine. ADR-0001 makes the engine a library: the game owns its loop and
its window, so the engine cannot own a loading state machine that drives the frame loop. ADR-0009
gives the engine a text overlay (`HudOverlay`) and no UI library.

The pre-roll is already a loop of short steps (each one sync and one bake step of
`PRE_ROLL_SAMPLES` samples); the level's first sync is one call that read 349 to 517 ms on the
bounded cave (`docs/nfr.md` section 7), and the streamed near residency is a loop of syncs. All
of them use the one `wgpu::Queue` the frame loop presents through, and `Frame` and `GpuWorld`
are owned by the run, not shared.

## Decision

Loading is **work the app steps from its own frame loop, a wall-time slice per frame, with a
loading frame drawn and presented between slices**. `App::init` creates the device-side objects
and returns; while loading, the app's `frame` runs loading steps until its slice (a proposal of
25 ms, read in Plan 0075) is spent, then encodes a loading frame into the surface and returns,
and the shell presents it and pumps the event loop as for any frame. The engine supplies the
pieces, not the sequence: the pre-roll as a steppable `vxe_render::PreRoll` whose time box is
measured on the work time the caller passes in (so a sliced and an unsliced load box at the
same work), a `LoadProgress` of app-named, app-weighted stages whose overall fraction never moves
back, and a default `vxe_render::LoadingScreen` (a background, an animation, a progress bar and
text rows on the `HudOverlay`) that takes the time and the progress as arguments. A game that
wants its own look draws its own frame from the same `LoadProgress` and never constructs the
`LoadingScreen`. `vxe-app`'s shell does not change.

## Consequences

### Positive
- The window presents from its first frame after the device, the event loop runs (the window
  can be moved, closed and is never reported as hung), and `Esc` can end a run that is loading.
- No new thread and no shared ownership of `Frame` or `GpuWorld`: the steps run where they run
  today, in the same order, so a sliced pre-roll bakes what an unsliced one bakes on a level.
- A consumer gets the same mechanism the testbed uses, with no engine-owned loop (ADR-0001).

### Negative
- Loading takes longer by the loading frames' cost, one present and one overlay pass per slice;
  Plan 0075 budgets it (`docs/nfr.md` section 7) and reads it on and off.
- A single step longer than the slice is a visible hitch in the animation. Each step must be
  bounded: the level's first sync gets a time budget across steps, and the bake step's GPU work
  in flight is capped, or the present waits behind it.
- The run gains a loading state that every per-frame path (the frame counter, `--frames`, the
  path player, the benchmark's warm-up, the report, the HUD's wall times) must skip.
- Work before the device (the level's generation join in `device_request`, which sizes the
  device's limits from the level's content) still shows nothing; Plan 0075 reads how long it is.

## Alternatives considered

### Alternative A: present loading frames from inside the blocking `init`
Hand `init` a closure that draws and presents a loading frame between pre-roll steps. The least
code, but winit's event loop is not pumped while `init` runs: no resize, no close, no input, and
a compositor or Windows marks the window as not responding after a few seconds; the pre-roll
can run 120 s. Rejected because a loading screen on a window that cannot be closed is worse than
the blank one.

### Alternative B: load on a worker thread while the main thread draws
Move the sync, the streaming loop and the pre-roll to a thread, and hand `Frame` and `GpuWorld`
back when done. The GPU work shares the one queue with the loading frame's present, so the
present still waits behind the bake's dispatches and the thread buys no smoothness the slice
does not; it adds `Send` requirements on `Frame` and `GpuWorld`, a hand-back protocol, and a
second order in which the mirror and the bake can interleave. Rejected for the cost with no gain.

### Alternative C: the tester's launcher shows the loading screen in its own window
The launcher's menu window draws "loading" while the child starts hidden. It covers the tester
only, not the fly-through or a consumer; the child's window still opens blank when it appears,
and two windows trade focus on every run. Rejected because the owner asked for the engine too.
