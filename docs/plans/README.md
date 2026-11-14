# Plans index

The one-minute "what's in flight" view. Read this first each session instead of re-deriving
state from `git log`. Completed plans move to `done/`.

**Next free number: 0075** (ADRs are a separate sequence; next free there is **0059**).

## Active roster

<!-- roster:begin cap=320 -->
| Plan | Title | Status | Live constraint |
|------|-------|--------|-----------------|
| [0032](0032-sun-and-sky-hillaire-atmosphere-time-of-day-and-an-outdoor-testbed.md) | Sun and sky: a Hillaire atmosphere, a time of day, sun and sky ray into the direct target, an outdoor testbed | approved | M10, ADR-0026, ADR-0027. Redesigned 2026-11-10. After 0058 and the render-profile plan. |
| [0033](0033-far-field-clipmap-to-a-kilometre.md) | The far field: a clipmap of three coarse chunk levels to about 1 km | approved | M10, ADR-0028 (proposed). Amended 2026-11-10. After Plan 0032. |
| [0034](0034-water-surface-waves-reflection-refraction-underwater-fog.md) | Liquid surfaces: analytic waves, traced reflection and refraction, absorption under the surface | approved | M10, ADR-0029 (proposed). Amended 2026-11-10. After Plan 0033. |
| [0035](0035-aerial-perspective-and-volumetric-clouds-with-cloud-shadows.md) | Aerial perspective over the near and far field, and volumetric clouds that shadow the ground | approved | M10, ADR-0030 (proposed). Amended 2026-11-10. After Plan 0034. |
| [0036](0036-weather-rain-snow-wind-wet-surfaces-snow-cover-lightning.md) | Weather: rain, snow, wind, wet surfaces and snow cover, lightning | approved | M10, ADR-0031 (proposed). Amended 2026-11-10. After Plan 0035. |
| [0037](0037-night-sky-stars-moon-phases-sky-bodies-and-aurora.md) | The night sky: stars, the moon's phases, sky bodies, an aurora that lights the ground, a night tint | approved | M10, ADR-0032 (proposed). Amended 2026-11-10. After Plan 0036. |
| [0044](0044-hot-path-and-defect-pass.md) | The hot-path and defect pass: frame, sync, walk, edits and audio callback waste, verified defects | in-progress | Code health. Side lane (Phases 1, 6, 7 and their Phase 0 rows) merged d6d2dfa; Phases 2, 3, 5, 8 after 0058; Phase 4 went to Plan 0070. |
| [0058](0058-dynamic-lights-a-list-of-analytic-lights-dropped-carried-and-bounced.md) | Dynamic lights: eight analytic lights, exact, dropped and carried; the direct part in rgb; a bounce flag | approved | M2, ADR-0044 (proposed). Refreshed 2026-11-13 at a312c19; owner answers 7-10 taken. |
| [0062](0062-entities-voxel-models-on-rigid-parts-traced-lit-shadowed-and-queried.md) | Entities: voxel models on rigid parts, lit and shadowed by the list lights, a headless ray query | in-progress | M6, ADR-0050 (proposed). Phase 1 merged 74ae796; Phases 2-6 after 0058. |
| [0063](0063-rigs-and-clips-keyframe-animation-a-model-viewer-and-text-authoring.md) | Rigs and clips: keyframe animation by tick, a rig text file, a model viewer, voxels from text | in-progress | M6, ADR-0051 (proposed). Phases 1-2 merged 7ca423c; Phases 3-4 after 0062. |
| [0069](0069-render-settings-one-surface-an-apply-class-per-field-and-named-quality-profiles.md) | Render settings: one surface, an apply class per field, and named quality profiles | approved | M10's first, ADR-0055 (proposed). Approved 2026-11-11, recommendations taken. After 0045, 0058, 0044. |
| [0068](0068-a-tester-build-for-linux-and-windows-scenes-paths-and-a-report.md) | A tester build: Linux and Windows archives, cave, hall and castle, a camera path and free flight, a report zip | in-progress | M7, ADR-0054 (proposed). Phases 1-5 merged 56008fa; Phase 6 Linux run done, Windows run owed (owner: later). |
| [0071](0071-the-consumers-path-a-render-example-a-renderer-tested-alone-names-and-errors.md) | The consumer's path: a render example as the contract, a renderer tested alone, one name per thing, errors | approved | Cross-cutting, ADR-0057. From the 2026-11-12 review. After 0045, 0069 and 0044's Phase 8. |
| [0073](0073-a-settings-menu-and-file-for-the-fly-through-and-the-tester.md) | A settings menu and a settings file for the fly-through and the tester | approved | ADR-0058. Apps only, over 0069's RenderSettings. After 0069 and 0071. |
| [0074](0074-the-syncs-light-stages-placement-spikes-attributed-and-exposed-emitters-by-masks.md) | The sync's light stages: the windowed spikes attributed, placement searches bounded, exposed emitters by masks | approved | M2, no new ADR. Approved 2026-11-13; a lane beside 0058. |
<!-- roster:end -->

## Sequence

The order the active plans run in (owner, 2026-11-10, set at the review of every active plan;
amended 2026-11-11: Plan 0045 moves ahead of Plan 0058 and the rest of Plan 0044).
Each starts on main after the one before it closes, unless its line says otherwise.

1. **Plan 0060**, the live queue: closed 2026-11-11 (v0.45.0). Plan 0061's Phase 1 (readings only) and Plan
   0064's Phases 1 and 2 (the pacing spike and the owner's pick) ran beside it.
   - **Plan 0064**, the window's frame: closed 2026-11-11 (v0.46.0).
   - **Plan 0065**, the baked light's waste: closed 2026-11-11 (v0.47.0).
   - On 2026-11-10 the look-driven plans (0058, 0061, 0032 to 0037) marked their phases
     before the window as exploration: image tests are written after the pick, in each plan's
     last dev phase.
2. **Plan 0061**, antialiasing: closed 2026-11-11 (v0.48.0).
   - **Plan 0067, the frame's CPU**: closed 2026-11-12 (v0.49.0). Approved 2026-11-11 (owner go
     with the defaults on its open points), after 0061 and ahead of 0058 (owner, 2026-11-11, after Plan 0060's and 0064's
     windows: "we are not there - either lagging too much, or the picture is unacceptable";
     "address performance next after this set of plans"). Its Phase 1 is readings only and may
     run beside 0061's Phase 2 once 0064 and 0065 have merged. It takes Plan 0065's Phase 6 (the
     queue's step) and the backlog's per-second bake budget and missing-light fallback (a
     neighbour's or parent's light, owner 2026-11-11; ADR-0052 and ADR-0053 accepted the same
     day); the Radeon's hit and FSR 2 passes (26 and 18.5 ms at the window) stay with
     the profiles plan (step 6); 0065 closed with them over their lines.
3. **Plan 0045**, the structure pass: closed 2026-11-13 (v0.50.0); later plans read old paths
   through its "Where things moved". Moved ahead of 0058 and 0044's render phases (owner
   2026-11-11). It starts once Plans 0061, 0067 and 0068 have merged, while no lane edits the
   files it splits. Its Phase 0 re-measures and the architect confirms the seams before a file
   moves. Plan 0044's render device helper (its Phase 8) now follows it.
4. **Plan 0058**, dynamic lights, on the split tree. The architect re-reads its Phases 1 and 2
   against 0045's tip before its lane starts, since 0061 rewrote `temporal.wgsl`,
   `composite.wgsl` and `Lighting` and 0045 moves them. Plan 0069's per-profile light cap is a
   field at or under 0058's `MAX_LIGHTS`; 0058 changes nothing for it.
5. **Plan 0044**, the hot-path pass, its Phases 2, 3, 5 and 8 (Phase 4 went to Plan 0070, closed 2026-11-13; the side lane, Phases 0, 1, 6
   and 7, merged at `d6d2dfa`). Phase 0's render rows re-inventory on the split tree.
6. **Plan 0069, render settings and profiles** (approved 2026-11-11, every recommendation
   taken), then the owed plan removing the `own` upscaler (not drafted; owner, 2026-11-11:
   Portable takes FSR 2, so the removal goes ahead).
7. **M6, entities: Plan 0062, then Plan 0063** (owner, 2026-11-10: ahead of M10,
   because the first consumer is a cave game).
   0062's Phase 1 and 0063's Phases 1 and 2 ran as a side lane and merged (74ae796, 7ca423c).
   - **Plan 0068**, the tester build (approved 2026-11-11): a side lane whose Phases 1-5 merged at 56008fa;
     it adds new files and three flags to the fly-through, and its Windows run is owed.
8. **M10 in plan order: 0032, 0033, 0034, 0035, 0036, 0037.** 0032 builds on Plan 0058's direct
   target, and each later plan relies on 0032's "Interface for Plans 0033 to 0037" section.

**Plans from the architecture review of 2026-11-12** (`../reviews/2026-11-12-architecture-review.md`),
approved by the owner on 2026-11-12 ("45 after 67, 70 after 45. all draft plans can be
approved"): **Plan 0070** (the sync off the frame), closed 2026-11-13 (v0.52.0), taking
Plan 0044's Phase 4; **Plan 0072** (development speed), closed 2026-11-13 (v0.51.0);
**Plan 0071** (the consumer's path) after Plans 0045, 0069 and 0044's Phase 8, before M10.

**Plan 0073** (a settings menu and file for the fly-through and the tester, approved 2026-11-12 with every recommendation): after Plans 0069 and 0071.

**Plan 0074** (the sync's light stages, approved 2026-11-13): a lane beside Plan 0058 (owner, 2026-11-13, 0058's answer 9); it touches `gpu_world/` and the fly-through's `report.rs` only, and shares docs and the perf lock (its "Beside Plan 0058").

Every active plan is approved (owner, 2026-11-10; Plans 0070 to 0072 on 2026-11-12; Plan 0069 on 2026-11-11; Plans 0064 and 0065 the same day, after the
static performance review). Plan 0044's Phase 2 ran as Plan 0064's Phase 3 (closed 2026-11-11), and its Phase 5
stays after Plan 0058. Approval accepts the design; a plan's lane
starts only when the plans its "Starts after" line names have closed and merged, and where that
line asks for it, after the architect has re-read the plan against the new tip. The approved
plans were amended on 2026-11-10;
their figures for wall time and frame cost are estimates until a phase measures them.

## Recently closed

<!-- roster:begin cap=320 -->
| Plan | Title | Closed |
|------|-------|--------|
| [0070](done/0070-the-sync-off-the-frame-slot-words-packed-on-the-workers.md) | The sync in place: rescan set dense, light walks on change; sync p50 180 to 17 us, `update + poll + sync` p99 45 to 24 ms, over 16.6; worker packing deferred | 2026-11-13, v0.52.0 |
| [0072](done/0072-development-speed-the-build-graph-a-narrowed-gate-and-one-compile-per-ci-profile.md) | Development speed: deps at line tables (cold build 455 to 403 s), `-p` gate and `--clippy`, reflink-seeded lanes (137 s); lavapipe stays at 1; CI readings owed | 2026-11-13, v0.51.0 |
| [0045](done/0045-structure-pass-split-the-mega-files-share-the-fixtures.md) | The structure pass: GpuWorld 11 fields, Frame 15, main.rs 36 lines; vxe-render's root 100 names; file-size and fn-length gates; benches at the start's | 2026-11-13, v0.50.0 |
| [0067](done/0067-the-frames-cpu-a-cheap-queue-step-a-bake-budget-by-time-and-no-black-for-missing-light.md) | The frame's CPU: a still queue drains at 0.075 ms a step; missing light reads neighbour, then parent (`chain`); bake stays frame:1.5 | 2026-11-12, v0.49.0 |
| [0061](done/0061-antialiasing-far-bands-under-the-baked-light-and-the-edges-left.md) | Antialiasing: the far store read by the shading normal and filters weighed by face; the owner: "no aliazing", a bit soft | 2026-11-11, v0.48.0 |
| [0065](done/0065-the-baked-lights-waste-atomics-a-filter-of-zeros-and-a-scan-per-frame.md) | The baked light's waste: temporal 0.32 to 0.07 ms, lamp-off a-trous a third, FSR 2 0.8 ms less; images byte-identical | 2026-11-11, v0.47.0 |
| [0064](done/0064-the-windows-frame-pacing-workers-and-readbacks-that-wait.md) | The window's frame: free pacing the default, below the compositor's; no frame-path GPU waits; the sync's time budget kept off | 2026-11-11, v0.46.0 |
| [0060](done/0060-the-live-queue-by-the-view-and-withdrawing-passed-work.md) | The live queue: a view score with the class as a tier, passed jobs withdrawn; the owner kept the withdrawal, saw no change from the order | 2026-11-11, v0.45.0 |
| [0066](done/0066-test-cost-a-10-s-soak-line-and-named-gpu-binaries-per-phase.md) | Test cost: the soak line at 10 s on lavapipe, 26 tests to soak, gate 505 s; `--binaries` per phase; CPU vectors stay in the gate | 2026-11-11, no bump |
| [0059](done/0059-loaded-light-a-per-chunk-store-baked-at-load-and-at-edit.md) | Loaded light: the owner picked the bake; a brick store at 8/4096 filled by a live queue and re-baked by digs; the table deleted | 2026-11-09, v0.44.0 |
| [0057](done/0057-steady-light-the-per-face-table-as-the-emissive-store-and-a-lamp-bounce-channel.md) | Steady light: the table holds still, the lamp channel stands; the owner moved to a look-first bake, vein bars left to it | 2026-11-05, v0.43.0 |
| [0056](done/0056-aliasing-spike-what-moves-on-the-final-image.md) | The aliasing spike: FSR 2's jumps are jitter on silhouettes, the dots light on creases; FSR 2 at 1.5 stays, far normal `[1, 2]` picked by eye (next plan) | 2026-11-04, v0.42.0 |
| [0051](done/0051-steady-light-spike-converge-and-freeze-and-a-world-space-emissive-cache.md) | The steady-light spike: A, B and C read behind settings; the owner picked B with indirect off, nearest read, the probes to a lamp-bounce channel | 2026-11-04, v0.41.0 |
| [0048](done/0048-probe-rays-direct-light-without-the-clamp-bias.md) | Dropped by the owner: moot under ADR-0043, the probe rays gather no emissive light; nothing was built | 2026-11-04, abandoned |
| [0055](done/0055-clarity-pass-no-detail-fsr2-default-and-a-far-normal-spike.md) | The clarity pass: procedural detail gone, FSR 2 at 1.5 the default, the far normal on at `[2, 6]` (provisional) | 2026-11-03, v0.40.0 |
| [0049](done/0049-light-settles-fast-and-young-pixels-hold-no-dots.md) | Abandoned by the owner: its Phase 1 readings kept in the plan, its rig under tag `archive/plan-0049`; Plan 0051 takes the work | 2026-11-03, abandoned |
| [0054](done/0054-field-surface-trilinear-zero-set-of-the-27-fraction.md) | Honest cubes the default, the planes gone (hit plus light 1.397 ms, mirror 60 % smaller at the level); the field surface built and kept opt-in | 2026-11-03, v0.39.0 |
| [0053](done/0053-one-voxel-features-on-the-contour-surface-majority-fit-and-a-normal-field.md) | One-voxel features: a box metric kept; the majority fit and the normal field failed the eye and are gone; ADR-0040 rejected | 2026-11-03, v0.39.0 |
| [0050](done/0050-watertight-contour-surface-crack-spike-and-closing-rule.md) | A watertight contour surface: a crack spike, then snap closes the wedges and triangles at +1.3 % (RTX) and +0.4 % (Radeon); the one-voxel boxes go to Plan 0053 | 2026-10-31, v0.38.0 |
| [0052](done/0052-second-review-defects-and-the-waste-the-first-pass-missed.md) | The second review: five defects fixed, fuel charged before the walk, exit 1 on failure, the idle 54.7 MB probe volume and the losing lab modes gone | 2026-10-30, v0.37.0 |
| [0047](done/0047-emissive-light-holds-still-long-guarded-history.md) | Emissive light holds still: an f32 history of 256 frames, an emissive-only guard and hashed probe slices; 1.5 patches a step from 77; motion faults to Plan 0049 | 2026-10-29, v0.36.0 |
| [0038](done/0038-showcase-cavern-and-hero-shots.md) | A showcase cavern lit by four ores, lamp off: a 72 m hall, three named shots, half dark with crystals leading (owner, Phase 6a); the hero re-shot from `hall` | 2026-10-28, v0.35.0 |
| [0040](done/0040-eye-adaptation-asymmetric-auto-exposure-and-pre-exposure.md) | Eye adaptation: asymmetric two-stage auto exposure metered on the GPU, pre-exposed HDR targets; owner chose 0.15 / 2 / 10 s and max 16 | 2026-10-28, v0.34.0 |
| [0041](done/0041-fsr2-port-to-wgsl.md) | FSR 2.2.1 in WGSL beside our upscaler: still, ghost and lamp bars held, slow pans and 1.2 ms (1.9 ms) missed; owner kept it opt-in, `own` at 1.5 the default | 2026-10-27, v0.33.0 |
| [0039](done/0039-contour-look-sharpness-measured-and-tuned.md) | The contour look at arm's length: a sharpness harness, five candidates; contour-sharp (weighted fit, lamp detail after lighting) the default; mirror bytes unchanged | 2026-10-27, v0.32.0 |
| [0046](done/0046-dx12-shaders-through-bundled-static-dxc.md) | DX12 shaders through bundled static DXC: WARP's eleven pipelines 76.6 s cold under FXC, 2.5 s under DXC; the attribute pass's WARP execution stays slow | 2026-10-22, v0.31.1 |
| [0029](done/0029-indirect-light-probe-volume.md) | Indirect light from a probe volume that scrolls with the camera: bounce kept at 1.6 m, update plus gather 1.57 ms, a light change refreshes it in 16 frames | 2026-10-22, v0.31.0 |
| [0042](done/0042-generated-gothic-castle-stamped-at-generation.md) | A generated gothic castle: a `.vox` writer, 21 tiles stamped into a carved hall; 344 MB of prefabs, about 92 % air, +0.3 GB RSS | 2026-10-22, v0.30.0 |
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
