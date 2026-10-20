# ADR-0019: Gamepad input is generic axes and pad buttons in the shell's `Input`, fed by the testbed's own gilrs

> **Status:** proposed
> **Date:** 2026-10-20
> **Related plan(s):** [Plan 0026](../plans/0026-gamepad-flight-generic-axes-in-the-shell-gilrs-in-the-testbed.md)

## Context

The fly-through needs a gamepad for demos. `winit` has no gamepad support; the standard
crate is `gilrs`, which on Linux links the system libudev and adds seven crates, and on
Windows builds the `windows` crate. `vxe-app` is the thin, optional window shell (ADR-0001);
a game composes the crates it needs and owns its loop, and `CLAUDE.md` counts every
dependency as a cost. `vxe_app::Input` is today a keyboard-and-mouse snapshot the shell
hands to `App::frame` immutably.

## Decision

`vxe_app::Input` carries analog axes and pad buttons as plain enums with no gamepad-library
types, and `App::poll(&mut self, input: &mut Input)` runs before each frame so an app can
feed them from any source. The fly-through depends on `gilrs` and feeds `Input` in `poll`.
No engine crate depends on a gamepad library.

## Consequences

### Positive
- `Fly`, `Walk` and any consumer read one merged input; a consumer with its own gamepad
  stack feeds the same axes.
- The engine's dependency tree and CI packages are unchanged for everyone but the testbed.

### Negative
- Every consumer that wants a pad writes the same twenty-line gilrs bridge the testbed has.
  If two consumers do, a `vxe-app` feature can host it then.

## Alternatives considered

### Alternative A: gilrs inside `vxe-app` behind a `gamepad` feature

Simplest for consumers, but the optional shell would carry a native dependency and a CI
package for the sake of the testbed, before any consumer has asked.

### Alternative B: A private pad state in the fly-through beside `Input`

No shell change, but `Fly` and `Walk` would read two sources and every test of movement
would set up both.

## Outcome

_(Added at acceptance if implementation falsified something above.)_
