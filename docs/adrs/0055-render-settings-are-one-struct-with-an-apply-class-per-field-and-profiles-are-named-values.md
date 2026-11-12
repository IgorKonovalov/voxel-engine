# ADR-0055: Render settings are one struct with an apply class per field, and a profile is a named value of it

> **Status:** proposed
> **Date:** 2026-11-11
> **Related plan(s):** [Plan 0069](../plans/0069-render-settings-one-surface-an-apply-class-per-field-and-named-quality-profiles.md)

## Context

Every mechanism the renderer has gained since M2 arrived with its own knob: `Frame::set_lighting`
(the whole `Lighting` struct), `set_resolution`, `set_look`, `set_lod`, `set_upscaler_kind`,
`set_sharpness`, the bake's slice and digs setters, and many more `pub fn set_*` on `Frame`; `MirrorConfig` (budget, records, light samples, sync budget) at the mirror's creation;
`StreamConfig` (residency, workers) in `vxe-world`. A consumer that wants "the cheap settings"
has to know all of them, and nothing says which ones take effect on the next frame, which ones
throw away the temporal history (and so show a settle, against the owner's "stable image first"),
and which ones need the mirror or the streamer rebuilt. The fly-through spreads them over more
than 100 flags.

The roadmap (decisions table, "Floor, profiles and scene scale", 2026-10-17) promises named
quality profiles: the floor, a portable profile at 720p and 30 to 60 fps, and a performance
profile for fast machines. Plans 0058 (eight lights), 0032 to 0037 (sky, far field, liquids,
clouds, weather, night sky) and 0062 (entities) each add cost that multiplies with the others,
and the static performance review of 2026-11-10 found no plan where the product gets a cap. The
slow adapter (the Cezanne iGPU) reads 30 to 73 ms a frame at the default settings (Plan 0065's
close, at 2530 x 1378 output), so one default cannot serve both adapters.

The question is the shape of the surface: where the settings live, how a consumer learns what a
change costs, and what a profile is.

## Decision

`vxe-render` gains one plain-data struct, `RenderSettings`, that holds by value every setting of
the frame and the mirror (and the streaming values a renderer setting depends on, as
`vxe_world` types), and one table, `RenderSettings::FIELDS`, that names each field's **apply
class**: `Live` (the next frame, no visible change but the setting's own), `Settle` (the next
frame, with the temporal history restarted, so the image settles again) or `Rebuild` (the
mirror, the streamer or the frame must be recreated by the caller). `Frame::apply_settings`
applies every `Live` and `Settle` change, returns the fields that need a rebuild without
touching them, and reports which classes it used. A **profile** is a named constructor,
`RenderSettings::profile(Profile::Floor | Portable | Performance)`, returning plain values the
consumer may edit before applying; the engine never picks a profile by itself, and never by
adapter name or vendor. The default settings are the floor profile's. A setter on `Frame` that
is neither a field of `RenderSettings` nor listed as a per-frame input fails a test in
`vxe-render`, so a new mechanism lands as a documented setting.

## Consequences

### Positive
- One place a consumer reads to see every knob, its class and each profile's value; the
  fly-through, the tester's quality menu and a game's options screen all apply the same struct.
- The live-versus-restart rule is stated per field and checked by a test, not by reading code.
- A profile is data: a consumer can start from `Portable` and raise one field, and a plan that
  adds a mechanism adds a row with each profile's value, where the cost of the product is seen.
- No new dependency: the struct is plain Rust; a consumer that wants a settings file serialises
  it itself.

### Negative
- The existing setters stay for one plan as the struct's implementation, so the API is wider
  until a later pass folds them in; two ways to set a knob invite drift, which the setter test
  only partly bounds (it catches a new setter, not a setter whose effect differs from the field).
- `RenderSettings` reaches into `vxe-world`'s `Residency` and `StreamConfig`'s worker count,
  which the renderer does not own; it carries them as values for the caller to pass on, and a
  `Rebuild` field the caller ignores silently keeps the old value.
- Named profiles are guesses until each is read on both adapters and judged in a window; the
  portable profile is read on the Cezanne iGPU, the nearest machine on hand, not on a device of
  its class.
- A test that scans the source for `pub fn set_` is a text check, blind to a setter spelled
  another way.

## Alternatives considered

### Alternative A: a string-keyed settings registry read from a file (TOML or RON)

Each mechanism registers named keys with a type and a class, and a consumer sets them by name or
loads a file. It gives a game an options file for free. It lost because it adds a parsing
dependency to `vxe-render` (ADR-0001's "dependencies are a cost"), moves type errors from compile
time to run time, and the consumer, which owns its options screen and save format, can serialise
a plain struct itself.

### Alternative B: the profile is chosen by the engine from the adapter, or adapts at run time

The engine reads the adapter (its name, its limits, or a short benchmark at start) and picks a
profile, or scales the internal resolution each frame to hold a frame time. It lost on two
counts: picking by adapter name is a vendor branch, which CLAUDE.md allows only through an ADR of
its own, and a resolution that moves frame to frame moves the image, against the owner's
"stable image first" (2026-11-03). A start-up benchmark that suggests a profile, without applying
it, stays possible later on top of this decision.

### Alternative C: keep the setters, document classes in prose

Each setter's rustdoc says whether it is live, and the roadmap's profiles live in the fly-through
as flag sets. It costs nothing now. It lost because a profile spread over flags cannot be applied
by a consumer or by the tester, the prose rule has no gate (the 2026-10-22 idea went unenforced
for three weeks), and the cost of the M10 product has no row to sit in.

## Outcome

_(Added only at acceptance if implementation falsified something above.)_
