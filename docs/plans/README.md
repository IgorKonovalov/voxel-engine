# Plans index

The one-minute "what's in flight" view. Read this first each session instead of re-deriving
state from `git log`. Completed plans move to `done/`.

**Next free number: 0046** (ADRs are a separate sequence; next free there is **0035**).

## Active roster

<!-- roster:begin cap=320 -->
| Plan | Title | Status | Live constraint |
|------|-------|--------|-----------------|
| [0029](0029-indirect-light-probe-volume.md) | Indirect light from a probe volume that scrolls with the camera | approved | M2, ADR-0023 (proposed). After Plan 0024. |
| [0032](0032-sun-and-sky-hillaire-atmosphere-time-of-day-and-an-outdoor-testbed.md) | Sun and sky: a Hillaire atmosphere, a time of day the game sets, the sun and moon as light samples, sky light into the probes, an outdoor testbed source | approved | M10, ADR-0026 and ADR-0027 (proposed). After Plans 0024 and 0029. |
| [0033](0033-far-field-clipmap-to-a-kilometre.md) | The far field: a clipmap of three coarse chunk levels to about 1 km | approved | M10, ADR-0028 (proposed). After Plans 0028 and 0032. |
| [0034](0034-water-surface-waves-reflection-refraction-underwater-fog.md) | Liquid surfaces: analytic waves, traced reflection and refraction rays, and absorption under the surface | approved | M10, ADR-0029 (proposed). After Plan 0032; its upscaler half after Plan 0030. |
| [0035](0035-aerial-perspective-and-volumetric-clouds-with-cloud-shadows.md) | Aerial perspective over the near and far field, and volumetric clouds that shadow the ground | approved | M10, ADR-0030 (proposed). After Plans 0032 and 0033. |
| [0036](0036-weather-rain-snow-wind-wet-surfaces-snow-cover-lightning.md) | Weather: rain, snow, wind, wet surfaces and snow cover, lightning | approved | M10, ADR-0031 (proposed). After Plan 0035. |
| [0037](0037-night-sky-stars-moon-phases-sky-bodies-and-aurora.md) | The night sky: a star field, the moon's phases, sky bodies the game lists, an aurora that lights the ground, and a night tint | approved | M10, ADR-0032 (proposed). After Plans 0032 and 0035. |
| [0038](0038-showcase-cavern-and-hero-shots.md) | A showcase cavern lit by four ores, and a hero still with named shots and no headlamp | approved | M2, testbed only, no ADR. After Plans 0029 and 0030. |
| [0039](0039-contour-look-sharpness-measured-and-tuned.md) | The contour look at arm's length: a sharpness metric, the sources of its softness, named presets the owner compares | approved | M2, no ADR. After Plan 0028. |
| [0040](0040-eye-adaptation-asymmetric-auto-exposure-and-pre-exposure.md) | Eye adaptation: an automatic exposure that is fast toward bright and slow toward dark, metered on the GPU, with pre-exposed HDR targets | approved | M2, ADR-0033 (proposed). After Plan 0029; before Plan 0032. |
| [0041](0041-fsr2-port-to-wgsl.md) | AMD FSR 2.2.1 ported to WGSL: its six passes fed from the hit buffer, compared against our upscaler and the native frame | approved | M2, ADR-0034 (proposed). After Plan 0030. |
| [0042](0042-generated-gothic-castle-stamped-at-generation.md) | A generated gothic castle with a dungeon: a `.vox` writer, a 27-tile scene imported and stamped at generation into a carved hall, its costs read | approved | M7, no ADR. Testbed plus `vxe-assets`; independent of the other lanes. |
| [0044](0044-hot-path-and-defect-pass.md) | The hot-path and defect pass: frame, sync, walk, edits and audio callback waste, six verified defects | approved | Code health, no ADR. After every plan active on 2026-10-22. |
| [0045](0045-structure-pass-split-the-mega-files-share-the-fixtures.md) | The structure pass: split the mega files, share the fixtures, gate their regrowth | approved | Code health, no ADR. After Plan 0044. |
<!-- roster:end -->

## Recently closed

<!-- roster:begin cap=320 -->
| Plan | Title | Closed |
|------|-------|--------|
| [0043](done/0043-test-tiers-quick-gpu-full.md) | Test tiers: quick (CPU, no lock) per phase, gpu under the lock for render phases, full at close; 470 + 124 + 5 = 599, a gate on the gpu list | 2026-10-22, v0.29.1 |
| [0028](done/0028-size-classed-mirror-slots-and-surface-only-attributes.md) | Size-classed mirror slots: the level 232 to 63 MB, the default at `r_full` 16 under 0.9 GB, attribute pass p99 0.75 ms; the default's `sync` 3.1 ms open | 2026-10-22, v0.29.0 |
| [0030](done/0030-temporal-upscaler.md) | A temporal upscaler of our own: jittered rays, hit-position reprojection, RCAS; every bar met, shipped opt-in as a performance setting, the default stays Plan 0024's frame | 2026-10-22, v0.28.0 |
| [0024](done/0024-direct-light-hit-buffer-emissive-samples-and-svgf-denoiser.md) | Direct light: a hit buffer, emissive light samples, one shadow ray, an SVGF denoiser; half tier, coarse walk, reuse, a 32-frame emissive history; extraction p99 missed | 2026-10-22, v0.27.0 |
| [0027](done/0027-mirror-uploads-through-a-staging-belt.md) | Mirror uploads through a staging belt: `write_buffer` gone, plain `sync` p50 halved to 1.4 to 1.7 ms; the belt's share and the 1.0 ms line still missed | 2026-10-21, v0.26.0 |
| [0031](done/0031-edit-cost-third-pass.md) | Edit cost, third pass: a stage profile, shape words from the spans and one span solve per op; the `B` line at 1.0 ms, the line back to 2.0 ms | 2026-10-21, v0.25.0 |
| [0020](done/0020-cavern-reverb-directional-echoes-and-gliding-taps.md) | Cavern reverb: a late tail sized by the room the listener sees, echoes from their walls, taps that glide, and the player's own voice in the cave | 2026-10-21, v0.24.0 |
| [0023](done/0023-render-thread-cpu-attribute-hitch-and-streaming-line.md) | The render thread's mirror CPU: the attribute hitch cut with a narrowed shell and a capped pass, an incremental stream queue; the streaming line still missed | 2026-10-21, v0.23.0 |
| [0022](done/0022-a-falling-region-crushes-sub-threshold-voxels.md) | A falling region crushes the sub-threshold voxels under it and piles them as rubble, and the logic tells a wedge from a refusal and a landing | 2026-10-20, v0.22.0 |
| [0026](done/0026-gamepad-flight-generic-axes-in-the-shell-gilrs-in-the-testbed.md) | Gamepad flight for demos: generic axes and pad buttons in the shell's input, gilrs in the testbed only | 2026-10-20, v0.21.0 |
| [0025](done/0025-cutaway-view-see-the-world-from-outside-its-rock.md) | The cutaway view: a camera in rock or outside the world sees through the hull to the first cavern's walls | 2026-10-20, v0.20.0 |
| [0013](done/0013-contour-default-shadow-origin-and-streamed-footprint.md) | Contour as the fly-through's default: the shadow-ray origin, a streamed footprint under 3 GB, and a weaker rock detail | 2026-10-20, v0.19.0 |
| [0021](done/0021-packed-cpu-brick-census-kernel-and-readers.md) | The packed CPU brick: a census and kernel bench, then masks, planes and a palette under every reader | 2026-10-20, v0.18.0 |
| [0017](done/0017-mirror-budget-reservation-page-kinds-and-pressure.md) | The mirror reserves its budget: fixed buffers, empty pages that change kind, and a pressure signal the streamer follows | 2026-10-20, v0.17.0 |
| [0019](done/0019-metered-host-calls-and-shape-range-checks.md) | Host calls charged by their work, and a world that refuses out-of-range shapes and queries | 2026-10-20, v0.16.0 |
| [0018](done/0018-server-routed-stamp-drop-and-remove.md) | Stamp, drop and remove as op kinds 2 to 4 through the server, the log and the logic host | 2026-10-20, v0.15.0 |
| [0015](done/0015-traversal-hierarchy-chunk-bits-brick-masks-and-translucent-class.md) | The traversal hierarchy: chunk bits, full-resolution brick masks, a one-read-per-level walk, and the translucent class | 2026-10-19, v0.14.0 |
| [0016](done/0016-post-overnight-fix-pass-depenetration-restart-and-nits.md) | The post-overnight fix pass: depenetration, a logic that restarts after a trap, and the review nits | 2026-10-18, v0.13.0 |
| [0006](done/0006-spatial-audio-hrtf-mixer-and-occlusion.md) | Spatial audio: a headless HRTF mixer on the device callback, occlusion and early reflections through the material table | 2026-10-17, v0.12.0 |
| [0011](done/0011-wasm-host-for-the-logic-component.md) | A WebAssembly host for the logic component, proven by native-versus-component equality | 2026-10-17, v0.11.0 |
| [0007](done/0007-op-log-byte-channel-shared-and-owned-clients.md) | Server-owned op log, a byte channel, the logic boundary, shared and owned clients, and replay from seed plus log | 2026-10-17, v0.10.0 |
| [0010](done/0010-stats-hud-bitmap-font-overlay.md) | A toggleable stats HUD: a bitmap-font text grid and a frame-time strip in one overlay pass | 2026-10-17, v0.9.0 |
| [0009](done/0009-support-query-and-collapse-ops.md) | A support query on the coarse grid, and deterministic drop and rubble ops for collapse | 2026-10-17, v0.8.0 |
| [0014](done/0014-edit-cost-brick-array-pool-and-cheaper-spans.md) | Edit cost, second pass: a brick-array pool, cheaper sphere spans and an analytic capsule row; 2 ms missed | 2026-10-17, v0.7.0 |
| [0008](done/0008-vox-import-and-prefab-stamping.md) | MagicaVoxel import with a material map, and prefab stamping with 24 rotations at generation time and at runtime | 2026-10-17, v0.6.0 |
| [0012](done/0012-comment-hygiene-and-index-row-gates.md) | Two repository gates copied from Ritmolux: comment hygiene in sources, and a byte cap on index rows | 2026-10-17, v0.11.1 |
| [0004](done/0004-cylinder-sweeps-character-controller-and-coarse-grid.md) | Cylinder sweeps, a character controller, and a per-brick coarse grid with change notifications | 2026-10-17, v0.5.0 |
| [0003](done/0003-edit-cost-span-fills-and-per-op-tables.md) | Edit cost under 2 ms: row spans, a per-op target table and per-brick tallies | 2026-10-17, v0.4.1 |
| [0005](done/0005-surface-look-spike.md) | The surface-look spike: smoothed normals, per-voxel contour planes and procedural detail, measured on screen | 2026-10-16, v0.4.0 |
| [0002](done/0002-streaming-residency-lod-ring-and-mirror-narrowing.md) | Streaming residency, an LOD-only ring, and a narrowed, sharded mirror | 2026-10-16, v0.3.0 |
| [0001](done/0001-bounded-brick-map-edits-and-headlamp-marcher.md) | Bounded brick map, deterministic edits, and a headlamp ray marcher | 2026-10-16, v0.2.0 |
<!-- roster:end -->

## Conventions

- One plan per milestone slice, numbered when drafted, never reserved ahead.
- A row carries at most two sentences of live constraint. What landed and why belongs in the
  plan file.
- A closed plan leaves this table; `done/` records it.
