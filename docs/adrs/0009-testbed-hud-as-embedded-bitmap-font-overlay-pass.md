# ADR-0009: The testbed HUD is an embedded 8 × 8 bitmap font drawn in one overlay pass, with no UI library

> **Status:** accepted (2026-10-17, at Plan 0010's close; see Outcome)
> **Date:** 2026-10-16
> **Related plan(s):** [Plan 0010](../plans/done/0010-stats-hud-bitmap-font-overlay.md)

## Context

The owner asked on 2026-10-16 for a toggleable HUD in human runs of the fly-through: frame
rate and frame time, GPU pass times, upload and VRAM figures, RAM, the last edit's result and
cost. Today every reading is printed to the console, and `F1` prints a one-shot summary.

Three forces make the rendering choice a decision:

- **Dependencies are a cost.** `egui` with `egui-wgpu` brings its paint, text and hashing
  crates and tracks `wgpu` major versions on its own cadence; our `wgpu` is pinned at 30.x
  and the integration crate lags such bumps by weeks to months. `glyphon` brings
  `cosmic-text`, `fontdb` and `swash` for text alone.
- **GPU code lives in `vxe-render` only** (`CLAUDE.md`), and the HUD must not disturb the
  readings it displays: the march timer with the HUD on must read what it reads with it off.
- **What is actually needed** is a monospace text grid and one bar strip. Toggles are keys.
  M7's debug views need labels over the image, which the same grid provides; they do not need
  widgets.

The owner chose the zero-dependency option when offered `egui`, `glyphon` and an own bitmap
font.

## Decision

`vxe-render` gains an overlay pass: an embedded public-domain 8 × 8 ASCII bitmap font (95
glyphs, under 1 KiB), a cell buffer of glyph indices laid out as a grid at an integer scale,
and a buffer of recent frame-time samples drawn as a bar strip, all rendered by one fullscreen
fragment pass with alpha blending onto the surface after the presenter's blit. No UI library
is linked. Text layout and stats gathering are the application's; the engine draws rows of
bytes and a strip of numbers.

## Consequences

### Positive

- No new crates, no version coupling to `wgpu`, one pass, under 200 KiB of buffers.
- Integer scale and no filtering make the overlay pixel-exact, so a golden image with the HUD
  on is a bit-exact comparison on lavapipe.
- The same overlay carries M7's debug-view labels.

### Negative

- No widgets: every control is a key or a CLI flag. If M7's debug views need sliders, they
  are keys with printed values, or a UI library is added then in an optional crate without
  touching this pass.
- Monospace 8 × 8 at integer scales only; no proportional text, no Unicode beyond ASCII
  (other bytes render as `?`).
- The font table and the shader's glyph sampling are ours to keep right; a wrong bit is a
  wrong pixel with no library to blame.

## Alternatives considered

### Alternative A: `egui` through `egui-wgpu` and `egui-winit`

Panels, plots and toggles for free. Rejected for the dependency tree and the version coupling
to `wgpu`, for a readout that needs neither. Revisited for M7 only if the debug views turn out
to need real controls.

### Alternative B: `glyphon`

Good proportional text through `cosmic-text`. Rejected because it is still three to four
crates and system font loading for text alone, with the strip drawn by hand anyway.

### Alternative C: CPU-rasterised text uploaded as a texture each frame

Rejected because it needs a rasteriser dependency to produce the bitmap, and re-uploads a
texture per frame for what a 32 KiB cell buffer carries.

## Outcome

Accepted 2026-10-17 at Plan 0010's close. The embedded font, the one blended pass after the
blit, the absence of a UI library and the rejected alternatives stand as written. The
implementation settled four points the text above states loosely or did not foresee:

- **The cell buffer is 128 KiB, not 32 KiB.** It always holds the cap layout, 256 columns by
  128 rows of `u32` glyph indices, and the shader reads only the cells the target reaches.
  With the 1 KiB sample buffer, the font and the parameters, the pass holds about 130 KiB,
  still under the 200 KiB in Consequences. Alternative C's "32 KiB cell buffer" is that
  undercount.
- **"The march timer reads the same with the HUD on" is a 3 % check on one command.** On
  `flythrough --level` the march reads +0.06 % with the HUD on. The streamed world's readings
  are logged, not gated: its cold-start runs vary by about 21 % between runs (Plan 0010,
  second amendment of 2026-10-17).
- **The target height reaches the shader as the draw's first instance** (`instance_index`),
  because `encode` takes no queue to write a uniform. That this holds on D3D12 is unverified
  until a Windows run.
- **While the HUD is shown, the march and HUD timers are read back every frame, blocking.**
  The readback is part of the frame-time cost the 5 % check measured on `--level` (+4.0 %).
