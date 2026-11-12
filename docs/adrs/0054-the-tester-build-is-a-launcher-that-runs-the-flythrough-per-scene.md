# ADR-0054: The tester build is a launcher that runs the fly-through once per scene

> **Status:** proposed
> **Date:** 2026-11-11
> **Related plan(s):** [Plan 0068](../plans/0068-a-tester-build-for-linux-and-windows-scenes-paths-and-a-report.md)

## Context

The owner wants a build another person can run on their own Linux or Windows machine: pick one
of several scenes, watch an automatic flight along a fixed path or fly freely, and send back a
report with the machine's details, the run's frame times and the log. The tester is not a
developer. They should not see the fly-through's flag list or its debug keys.

The fly-through (`apps/flythrough`) is where the owner tunes the look. Its frame loop, world
setup and readings sit in one `main.rs` of about 9,300 lines, which Plan 0045 is scheduled to
split last in the current sequence. The streamed cave, the showcase hall and the castle are
built there from flags (`--source`, `--castle`, `--shot`, `--castle-shot`), and `--frames`
already prints readings at exit. `winit` creates one event loop per process, so switching scenes
inside one window means tearing the world, mirror and bake down and rebuilding them in place,
which the fly-through has never done.

What the tester runs must look like what the owner tuned. A second render setup drifts from the
first the day a default changes in `main.rs` and not in the copy.

## Decision

The tester build is two binaries in one archive: a new small app, `apps/tester` (binary
`vxe-tester`), and the release `flythrough`. The tester opens a window with a menu (scene, auto
flight or free flight, quality, run all benchmarks, save report) and starts the fly-through as a
child process per run, with curated flags plus `--tester`, `--path NAME` and `--report DIR`. It
captures the child's stdout and stderr into the run's log, reads the run's `run.json` when the
child exits, and zips the report folder. The fly-through stays the single place the look is
configured; `--tester` hides its debug keys and shows a key-help panel. Switching scenes is a
relaunch of the child, not an in-place rebuild.

## Consequences

### Positive
- The tester sees exactly the fly-through's default look, from the same code path and flags.
- The fly-through gains three flags and a path player, not a menu, a scene switcher or report
  packaging; `main.rs` grows little ahead of Plan 0045.
- A crash, panic or hang in a run is contained: the launcher survives it, records the exit
  code and the tail of the log, and the report still saves.
- Headless proof splits cleanly: the path, the report's JSON and the zip are CPU-testable.

### Negative
- Two windows: the launcher's stays open behind the run's. A scene switch costs a process
  start, adapter and device creation and the scene's load (seconds; the showcase bakes).
- The contract between the two binaries is flags, a JSON file and an exit code. It is a
  private, versioned contract; both ship in one archive, so it never crosses versions, but it is
  stringly typed and a renamed flag fails at run time, which the plan's launcher test catches.
- The launcher needs its own window and text drawing (`vxe-app` and `vxe-render`'s HUD
  overlay), and its own adapter for the system report.

## Alternatives considered

### Alternative A: a tester mode inside the fly-through

One binary with a menu overlay, path playback, report writing and in-process scene switching.
The smoothest experience, with one window and no relaunch. It lost because scene switching
needs a teardown and rebuild of the world, mirror, bake and audio that the fly-through has
never had, and all of it lands in the 9,300-line `main.rs` that Plan 0045 has yet to split.

### Alternative B: a new app with its own frame loop

`apps/tester` on `vxe-app` and `vxe-render` directly, scenes from the fly-through's library, no
digging, audio or server. The cleanest code. It lost because the render setup the fly-through
builds from its defaults would be copied, and the copy drifts from the look the owner tunes;
the tester would report on a renderer nobody looks at.
