# Changelog

One entry per closed plan, written at the architect's close ceremony together with the version
bump in `Cargo.toml` (`workspace.package.version`). Format: [Keep a Changelog](https://keepachangelog.com/),
SemVer 0.x (any minor may break the API until 1.0).

## [Unreleased]

## [0.38.0] - 2026-10-31

Plan 0050 (a minor bump: public API added and removed, a flag bit moved; ADR-0038 and ADR-0039 accepted with Outcomes). A watertight contour surface. A crack spike classifies how each primary ray reached its hit (a pass then an interior voxel, a pass then an entry already inside, a pass then a crossing) and the shadow ray's self-hit, shown by a `cracks` view; it named class 2 as the owner of the dark wedges at ledges and the bright triangles at facet vertices. The plane code's octahedral normal moves to codes `0..=62` (ADR-0038), so an axis normal decodes exactly and the 45-degree slab's class 2 steps go to 0. Three closing rules were measured and snap kept (ADR-0039): after a pass the walk hits at the passed voxel's exit with the passed plane's normal (before a voxel without a plane) or the mean of the two (before an entry inside), so classes 1 and 2 read 0 on the analytic scenes and at both poses (2247 and 5266 pixels before). The hit pass at the level's spawn reads +1.3 % on the RTX 3080 Laptop (0.616 against 0.608 ms) and +0.4 % on RADV RENOIR (4.298 against 4.279), loaded; the attribute pass's initial fill is unchanged (+0.1 %). The extend and blend rules and the old hit rule are gone. The cracks view writes its palette in display values, so the window's post chain keeps its colours. The owner's check found the wedges and triangles gone; voxel-sized flat-faced boxes remain and go to Plan 0053. Goldens re-blessed: `look_contour.png`, `look_wall_contour.png`, `look_wall_contour_grain.png`, `look_wall_contour_sharp.png`, `showcase_hall.png`; every face golden unchanged. Two mirror hash pins moved with the plane code.

- **Added: `ViewMode::Cracks`, `CrackShares`, `SurfaceTuning::shadow_push` (off by default; `Params::flags` bit 14), a second hit pipeline for the cracks view (`override CRACKS_VIEW`).**
- **Changed: the plane attribute's normal code (encode `round((p + 1) * 31)` clamped to 62, decode `q / 31 - 1`); a contour hit after a pass closes the crack (snap) in every contour look.**
- **Removed: `CrackClosing` and `SurfaceTuning::closing`, added and removed within the plan.**
- **Added (fly-through): the `cracks` view (`F9`, `--view cracks`) with the class shares on the HUD, `--pose vein|patch`, `--shadow-push on|off`; `flythrough::{Pose, VEIN_POSE, PATCH_POSE, POSES}`. The banner names the shadow push when set.**
- **Changed (tests): `detail_after_lighting_is_exact_for_the_lamp`'s floor is 98.0 % under snap (owner).**

## [0.37.0] - 2026-10-30

Plan 0052 (a minor bump: public API broken and removed, no feature; no ADR). The second whole-tree review's defects, waste and hidden decisions. `drop_region` lands a part-cell drop whose lower layers are air instead of panicking when the chunk below is absent, and checks every landing cell's residency before composing. A full chunk that arrives outside its band is swept by the next poll. `trace_materials` and `stamp` take the world's limits. The wasm host pre-charges a bound on `raycast`, `count-in-box` and `unsupported` before any host work and refunds after, so fuel bounds what the server spends; a `1 024^3` count now traps before the host counts, also over an empty world (owner, 2026-10-30). The fly-through exits 1 after its normal teardown on a replay log that does not read, a mirror sync that fails or an autopilot settle that times out; no thread but main exits the process; the dig logic refuses a result of the wrong kind instead of panicking, and the native logic is documented fail-stop. A default `Frame` no longer holds the 54 689 824 B probe volume with indirect light off. An op's row spans live in an arena of the brick rows it reaches (`tool` 1.4 % over the base, `tool_long` level, within the owner's 2 %). Voice smoothers settle on exact zeros: 64 voices at gain 0 cost what 64 fresh ones do (1.17 against 1.79 ms). Goldens unchanged; every pinned hash unchanged.

- **Changed: `World::trace_materials` returns `Result<Vec<Run>, EditError>`; `World::stamp` refuses `OutOfRange`; `vxe_render::limits_for` returns `Result<wgpu::Limits, LimitsError>` capped at the offer; `Channel::send` takes and `try_recv` returns `Arc<[u8]>`, and a broadcast shares one encode; `Frame::probes` is built on first use; `Params::light_span` is `_pad4` and `_pad5` carries `lamp_pose`.**
- **Added: `ShellConfig::{present_mode, power_preference, prefer_srgb, pacing}`, `ShellConfig::default()` (today's values), `Pacing::{Unthrottled, Cap { hz }}`; `LimitsError`, `limits_within`, `STORAGE_BUFFERS_NEEDED`; `Frame::lamp_changed`; `WorkRates::{count_bound, ray_bound, support_bound}` and `WasmConfig::support_max_region`.**
- **Removed: `CandidateDraw`, `Lighting::draw`, `LIGHTING_UNIFORM_DRAW`; `Upscaler::lanczos()` and the `Reconstruction`, `HistoryFetch`, `Blend` and `StillClip` selectors (one path left: Gaussian, Catmull-Rom, mean nearness, light-change clip); `ProbeVolume::set_light_restarts`; `SyncStats::buffers_created` and `copies`; `ShardLayout::shards`.**
- **Changed (fly-through): exit status 1 on a session failure; the engine crates come from the workspace table; the indirect golden's control pass renders 64 frames and the showcase's dark pass 8; the showcase golden left the slow list. `check-test-tiers` also scans `test = true` examples and a binary's test modules.**

## [0.36.0] - 2026-10-29

Plan 0047 (a minor bump: public API added and defaults changed; ADR-0036 accepted with an Outcome). Static emissive light holds still with the lamp off. The temporal pass's accumulation pair is `rgba32float`, so a long emissive history keeps its mean: it agrees with an `f64` replay within 0.0001 % over 2048 frames at a 512-frame history, against 13.92 % in `rgba16float`. The emissive history defaults to 256 frames (was 32). An emissive-only gradient guard (`Params::lighting` bit 10) re-shades last frame's kept emitter points whenever emissive candidates are on, and shortens that history where the light changed. A whole reveal behind a one-voxel plug reaches 90 % in 1 frame (430 without the guard); a 10 x 10 opening takes 222. The probe volume's slices are spread by a bijective hash of the slot, not by `slot mod slices`, which ends the probe gather's whole-screen steps at the vein pose. A headless patch bar on the level (`flythrough::emissive_stillness`, lavapipe, 192 x 108, `Manual` exposure) reads 1.50 patches a step at the patch pose (from 74 to 78) and 0.05 at the vein pose. The probe furnace test now judges against the probe rays' own direct mean `D` (7.7 to 8.4 % under pi because of the firefly clamp, which Plan 0048 takes). The hole test reads against the box 2048 frames after the carve, and `indirect.png` renders at 1600 frames. Quiet and loaded readings: the gradient pass costs 0.083 to 0.085 ms on the RTX 3080 Laptop and 0.454 to 0.462 ms on RADV (Cezanne); the hashed slices add +4.20 % to the probe update on RADV with the lamp off (quiet, bar 5 %); `frame_bytes` +3 686 400 B at the default. The owner's check found still walls much better. Indirect light that settles over seconds after a move or a lamp toggle, and light dots on a fast turn or a dig, go to Plan 0049. Goldens re-blessed: `fsr2.png`, `mode_banner.png`, `indirect.png` (at 1600 frames, mean luma -0.59 % against the base there), `showcase_hall.png`.

- **Added: `Lighting::reuse_m_cap` and `Lighting::emissive_guard` (on by default), `DEFAULT_REUSE_M_CAP` (20), `LIGHTING_EMISSIVE_GUARD`, `probes::slot_slice`, `readback::read_float`.**
- **Changed: `DEFAULT_EMISSIVE_HISTORY` is 256 (was 32); `Frame::accumulated` is `rgba32float`; `Params` gains `reuse_m_cap`; the gradient pass's strata carry the guard's lambda in `w`; `ProbeVolume::update_list` takes each slice's slots by `slot_slice`.**
- **Added (fly-through): `--emissive-guard on|off` (also in `hero`) and `--reuse-m-cap N`; the banner and the start line name the guard and the cap (`hud::BANNER_ROWS` is 7).**
- **Added (tests): the stillness rig (`tests/support/stillness.rs`) with its patch bar, sweep and a test-only filmstrip behind `STILL_FILMSTRIP`.**

## [0.35.0] - 2026-10-28

Plan 0038 (a minor bump: testbed features added, no engine crate changed, no ADR). A showcase cavern for the hero and demos. `flythrough::showcase` is a bounded, seeded, integer-only world of 14 x 12 x 7 chunks: a half-ellipsoid hall about 72 x 58 x 24 m over a noisy floor, five pillars, stalactites and stalagmites, two ledges, a 10 m chasm and a natural bridge, every brick no feature touches emitted through a corner early-out. Four emissive ores light it with the lamp off: amber (the existing ore row) on the west walls, cyan crystal clusters on the east floor, violet amethyst on the east walls, red ember at the chasm's bottom. After the owner's walk no ore lies on the roof or a downward face, about 60 % of the floor is dark and the crystals give the most light samples (29 806 in all, seed 1). A determinism vector pins the source; CPU tests pin the shape, the features, the open volume, the ore zones, the dark floor and every shot's clear view. `hero` renders the shots and `docs/images/hero.png` is re-shot from `hall` at 1920x1080. On the RTX 3080 Laptop the shots read 5.6 to 6.6 ms a frame at the defaults, on RADV RENOIR 42.7 to 49.5 ms (loaded); 71 046 `Full` bricks, a 134.6 MB bounded budget against the level's 259.2 MB; windowed first frame 2.05 to 2.08 s cold, 0.66 to 0.70 s warm. New golden `showcase_hall.png`; every other golden is unchanged.

- **Added (fly-through): `flythrough::showcase` (`ShowcaseSource`, `SHOWCASE_CHUNKS`, `CRYSTAL`, `AMETHYST`, `EMBER`, `Shot`, `SHOTS`, `Shot::named`), `showcase_materials`, `with_showcase_rows` and the three emission constants; `hud::Modes::with_source` and `with_probe_spacing`.**
- **Added (fly-through): `--source cave|level|showcase` (`--level` is `--source level`) and `--shot hall|chasm|crystals`; on the showcase the lamp is off, the light radius 19 and the exposure the shot's manual 1.5 unless given, and `--save`, `--replay`, `--vault`, `--autopilot`, `--room` and `--castle` are refused. The banner's line starts `source showcase` there and names `probes N` whenever indirect light is on.**
- **Changed (fly-through): `level_mirror_budget` takes the world, not a `Full` brick count.**
- **Changed (examples): `hero` defaults to the showcase's `hall` shot at 1920x1080, lamp off, indirect on at probe spacing 32 and our upscaler at 1.5, and takes `--source`, `--shot NAME|all`, `--out-dir`, `--indirect`, `--probe-spacing`, `--internal-scale`, `--jitter`, `--sharpen`, `--light-radius`, `--adapter`, `--lamp`, `--size` and `--exposure auto|E`; `world_budget` takes `--source showcase|level`.**

## [0.34.0] - 2026-10-28

Plan 0040 (a minor bump: public API broken and added; ADR-0033 accepted with an Outcome). Eye adaptation: the exposure is automatic by default and adapts like an eye, fast toward bright (0.15 s) and slow toward dark in two stages (2 s, then 10 s below a cone floor of 2^-6), at most 16, so a dark-adapted cave stays dim. The composite meters its radiance before exposure as log2 luminance per 8 x 8 tile, a one-workgroup exposure pass at the end of the frame weights the tiles toward the view's centre and steps the state on the GPU with exact exponentials split at the floor, and the next frame reads it as a uniform; a ring of read-back buffers gives the CPU the state a few frames late. Every HDR store (light, temporal, a-trous) is pre-exposed, history and the gradient pass's comparison are rescaled by `current / previous`, the moments are clamped at 255, and our upscaler, whose history is display-encoded, clips still pixels while the exposure moves by more than 1/64 EV a frame. Headless bars match the GPU's curve to the stepped model frame by frame; on the level the one-step closed form is a reading (0.1 to 0.2 EV off, because the meter falls over about 30 frames). Quiet readings at 1080p internal against main 05a3a28: on the RTX 3080 Laptop the meter adds 0.05 to 0.07 ms to the composite and the exposure pass reads 0.073 to 0.075 ms; on RADV RENOIR +0.106 ms and 0.060 ms; `frame_bytes` +260 128 B. A manual exposure other than 1.0 moves images by f16 rounding; every golden is unchanged except the fly-through's `mode_banner.png`.

- **Added: `vxe_render::exposure` (`Exposure`, `AutoExposure` with `eye`, `symmetric`, `instant`, `step`, `exposure`, `floor_log2`; `Metering`, `ExposureState`, `ExposureReadout`, `EXPOSURE_WGSL`, `METER_TILE`, `METER_LOG2_LIMIT`, `STATE_HELD`), `Lighting::manual_exposure`, `Frame::{set_dt, reset_exposure, set_metering, exposure_readout, exposure_state}`; `FRAME_PASSES` gains `exposure` (last).**
- **Changed: `Lighting::exposure` is `Exposure` (was `f32`; `Exposure::Manual(1.0)` is the old default); every frame shader's group 0 gains the exposure uniform at binding 13; FSR 2's tone pass receives pre-exposed colour and no longer multiplies by the exposure.**
- **Added (fly-through): `--exposure auto|E` (default auto), `--adaptation eye|symmetric|instant`, `--adapt LIGHT,CONE,ROD,FLOOR`, `--metering centre|average`, `Q` cycling the exposure mode on the banner, the stats HUD's exposure row, and the row on stdout while the exposure moves.**
- **Changed (fly-through): the default exposure is automatic (eye); `--exposure 1` is the previous look.**

## [0.33.0] - 2026-10-27

Plan 0041 (a minor bump: public API broken and added; ADR-0034 accepted with an Outcome, and an Outcome note on ADR-0024). AMD FSR 2.2.1's upscaling path ported to WGSL in `vxe-render` beside our upscaler, and the fly-through's default moved to our upscaler at scale 1.5. The port, under `shaders/fsr2/` with AMD's MIT notice in each file, runs an input pass (device depth, unjittered motion and a reactive mask of edits, light changes and translucent layers from the hit buffer), SPD's luminance pyramid in two dispatches, reconstruct previous depth (`atomicMax` on a storage buffer), depth clip (the edit boxes force a clip), lock, accumulate, RCAS and a tone pass after the upscaler; FP32, no wave operations, no added feature, widened storage formats. It holds its still bars at scale 1, the carve, history, lock, lamp, flicker and scale-1.5 wobble bars, but misses the slow-pan bars against ours (88 voxels at 0.37 px a frame and scale 1.5: GE 0.00688 against 0.04773; no port defect found) and the still bars at scale 1.5; those are readings. On the RTX 3080 Laptop at 1080p and scale 1.5 its nine passes read 1.89 to 1.91 ms against a 1.2 ms bar, the frame 5.71 to 5.81 ms against ours 4.39 to 4.51; on RADV RENOIR 10.0 ms of passes; `frame_bytes` 305 574 892 B against ours 181 154 464 B (loaded readings). The owner's windowed comparison found "not much difference" and chose `own` at 1.5 as the default. New golden `fsr2.png`; every other golden is unchanged.

- **Added: `vxe_render::fsr2` (`Fsr2`, `Fsr2Settings`, `UpscalerKind`), `Frame::{set_upscaler_kind, upscaler_kind, fsr2, fsr2_mut, reset_upscaler_history, current_copy, layers}`, `marcher::composite_hdr_source`; `FRAME_PASSES` gains FSR 2's nine passes.**
- **Changed: `jitter_at` takes a phase count.**
- **Added (fly-through): `--upscaler off|own|fsr2`, `F10` cycles it, and an upscaler banner shown for 3 s at the start and after each change.**
- **Changed (fly-through): the default is scale 1.5 with our upscaler, jittered (`--internal-scale` default 1.5; our upscaler jitters unless `--jitter off`); `--upscaler off` or `--internal-scale 1` is Plan 0024's frame. The library's `Frame` still defaults to Plan 0024's frame.**

## [0.32.0] - 2026-10-27

Plan 0039 (a minor bump: public API added; an Outcome on ADR-0011). The contour look at arm's length. A headless sharpness harness (a rough rock wall and a convex edge at 1 m under the headlamp, a fine-band and a mid-band contrast, an edge spread and an axis-fraction guardrail beside Plan 0005's) ranks the softness's sources: the half lighting tier recovers 74.6 % of the face-to-contour fine-band gap, the filter 3.5 %. Five candidates behind switches, each byte for byte today's when off: a weighted and a volume-matched plane fit in the attribute pass's existing `u16`, a face-normal mix (fails the guardrail, a diagnostic), a third detail octave, and the detail's lamp term applied after lighting, where the composite scales the direct irradiance by the detailed-to-undetailed `n.l` ratio and adds the probes' indirect light unscaled. The fly-through's `--look` takes presets and `N` cycles them with the name on the banner; a fit change refits the resident bricks under the pass's cap. The owner picked `contour-sharp` (the weighted fit plus the ratio) as the default and re-confirmed it with probe GI; `--look contour` keeps the previous look and `contour-grain` stays opt-in. Against main on the RTX 3080 Laptop and RADV RENOIR: hit -0.7 % and +0.1 %, light plus composite +0.8 % and +0.2 %, the attribute pass's quiet p99 median +0.1 % and +7.6 %; mirror bytes unchanged. Open: sharp voxel corners the owner saw on some smooth walls.

- **Added: `vxe_render::{PlaneFit, PlaneOffset, SurfaceFit, SurfaceTuning}`, `LookParams::tuning`, `GpuWorld::{set_surface_fit, surface_fit, refitting, rewrite_materials}`; `vxe_world::Material::detail_fine`.**
- **Added (fly-through): `LookPreset`, `--look contour-sharp|contour-grain`, `--plane-fit`, `--plane-offset`, `--face-mix`, `--detail-fine`, `--detail-after`; the banner's look row and `refitting`; the hero example's `--look`.**
- **Changed (fly-through): the default look is `contour-sharp`; the startup line names the preset and its parts.**

## [0.31.1] - 2026-10-22

Plan 0046 (a patch bump: a platform fix on Windows; ADR-0035 accepted with an Outcome). The workspace builds `wgpu` with `static-dxc`, so on x86_64 Windows MSVC the DX12 backend compiles with a statically linked DXC (`mach-dxcompiler-rs`, whose build script downloads the prebuilt library) and never falls back to FXC; the feature is inert on Linux. `deny.toml` clarifies that crate as `MIT AND NCSA` and allows `NCSA`: a Windows build owes DXC's notice. A headless example, `pipeline_times`, times each of the renderer's eleven compute-pipeline creations; on the `windows-latest` CI arm (WARP) the cold total is 76.6 s under FXC and 2.5 s under DXC (attrib 42.4 s against 0.6 s). The attribute pass's execution on WARP stays slow, so the shutdown test keeps `--look plain` there; that cost is a followup. The debug `flythrough.exe` reads 37 744 128 B with the 113 589 902 B static library linked.

- **Added: `vxe_app::backend_label`; `vxe_render::{UPSCALE_WGSL, SHARPEN_WGSL}` at the crate root; the `pipeline_times` example; a Windows-only `dx12_compiler` test.**
- **Changed: the shell's `adapter:` line and the fly-through's `headless on` line name the backend and, on DX12, the requested shader compiler; the headless instance reads wgpu's environment variables.**
- **Changed (CI): pipeline-time steps on both arms.**

## [0.31.0] - 2026-10-22

Plan 0029 (a minor bump: public API added; ADR-0023 accepted with an Outcome). Indirect light from a probe volume that scrolls with the camera. A 32^3 grid of irradiance probes at 16 voxels (1.6 m) follows the camera in toroidal tagged slots; each frame one probe in 8 and every probe an edit touched trace 64 full-walk rays whose hits return the direct estimate plus the previous irradiance there, blended into 6 x 6 octahedral irradiance and 14 x 14 visibility maps held in storage buffers (hysteresis 0, then 0.5 for 8 updates, then 0.97). A gather pass reads the eight probes around each hit with Chebyshev visibility weights, renormalised over live probes, and the composite adds `albedo E / pi`. A light change the frame knows (the lamp and its parameters, the shadow walk, the candidate count, an emission scale) opens a 16-frame window refreshing one probe in 4 at hysteresis 0, and a lamp change resets the temporal pass's lamp history; probe rays read their own per-material emission scale, so the fly-through's ore pulse feeds them its mean. On the RTX 3080 Laptop at 1080p half lighting, update plus gather reads 1.57 ms (2.70 ms inside a window) and the lighting line 4.4 ms; the frame's targets grow by 58 837 024 B. The furnace box reads pi within 5.3 %, two boxes behind a 2-voxel wall leak 0.36 %, the lamp-off box falls under a tenth by frame 14. New golden `indirect.png`; `mode_banner.png` reblessed.

- **Added: `ProbeVolume`, `Lighting::{indirect, probe_rays, probe_slices}`, `Frame::{set_probe_spacing, traced_probe_rays, probe_light_window, set_emission_scales}`, `ProbeVolume::set_light_restarts`; `FRAME_PASSES` gains "probes" and "gather".**
- **Changed: `limits_for` requests two more storage buffers per stage; the emission-scale buffer holds a pixel half and a probe half; `light.wgsl`'s estimator is callable functions.**
- **Added (fly-through): `--indirect on|off` (on), `--probe-spacing N`, `--probe-rays N`, `--probe-slices N`, `--lamp-toggle N`, `--fade-reading`, `F8`; the HUD's probe row; `M` pulses the ore at its mean for the probes.**

## [0.30.0] - 2026-10-22

Plan 0042 (a minor bump: public API added). A generated gothic castle stamped at generation. `vxe_assets::encode` (with `encode_rotation`) writes a `VoxFile` back as `.vox` bytes, `PACK` and the `nTRN`/`nGRP`/`nSHP` scene graph included, and parsing its output gives back the same file. The fly-through's `--castle` generates a 60 x 60 x 40 m castle with a dungeon in integer-only code (curtain walls, four towers, a gatehouse, a chapel keep with a rib vault, lancets, a rose window and a spire; a spiral stair to a corridor, eight barred cells and a pillared crypt) as a 27-tile scene, encodes, parses and maps it to seven new material rows, and stamps it through `StampedSource` over `HallSource`, which carves the hall and a rock plinth around the unbounded cave. `--castle-out PATH` writes the `.vox`, `--castle-shot NAME` starts at one of four shots and key `I` cycles them, the banner names the shot. A determinism vector pins the `.vox` bytes and the castle chunks' voxel hash; golden `castle_gate.png`. On the dev box (loaded, indicative): 21 tiles kept, 343 566 464 B of prefabs (about 92 % air), a 52 623 529 B `.vox`, about 0.3 GB more peak RSS, castle chunks generated in a mean of 424 us against 187 us for the hall alone.

## [0.29.1] - 2026-10-22

Plan 0043 (a patch bump: tooling only, no crate code changes; the Plan 0012 precedent). Test tiers. `.config/nextest.toml` splits the suite into three profiles over one classification: a gpu filter (every `vxe-render` integration test plus eight named fly-through binaries, in a `lavapipe` test group of one thread) and a slow list of the five tests measured at 20 s or more. `--profile quick` is everything outside both, takes no perf lock and terminates a test at 30 s; `--profile gpu` is the gpu filter minus the slow list; `--profile full` (and the default profile) is everything. Quick, gpu and slow partition full exactly (470 + 124 + 5 = 599 at the close). `scripts/check-test-tiers.mjs` fails when a test file that reaches a device (itself or through its `support` module) is outside the gpu filter, when a filter entry matches no file, or when the quick and gpu copies of the filters drift; CI runs it, runs the workspace suite as `--profile full`, and runs only `binary(component)` under `--features wasm`. The dev, orchestrate and architect skills, the lane rules and `CLAUDE.md` name the tiers. On the dev box (loaded): quick 45 s with no lock, against a 4 to 7 minute locked full run per phase before.

- **Added: `.config/nextest.toml` (profiles `quick`, `gpu`, `full`), `scripts/check-test-tiers.mjs` and its fixture `scripts/fixtures/test-tiers/`.**
- **Changed (CI): the workspace nextest step runs `--profile full`; the wasm step runs only `tests/component.rs`; a `Test tiers` step.**

## [0.29.0] - 2026-10-22

Plan 0028 (a minor bump: public API broken and added; ADR-0012's last step, with Outcomes on ADR-0012, ADR-0013 and ADR-0011). Size-classed mirror slots. A `Full` brick's mirror slot is its packed CPU bytes in 64-byte units: the opaque mask, a header (the brick's palette, the translucent `2^3` words, surface prefix popcounts, the attribute unit count), and its plane units copied from the CPU words, or 16 units of raw ids above 16 materials; `upload_brick` narrows nothing. Slots sit in ten bins (2 to 36 units) over 4096-unit pages that take a bin when first issued and return to the free pages when empty; the reservation, the partial sync, deferral and the pressure rule stand, the slot term read on units. Records lose the chunk palette (2112 B), and the masks buffer and its binding go. With contour attributes a full chunk's brick also carries a chunk-local surface mask and one `u16` attribute per surface voxel, addressed by popcount; a dirty brick re-uploads its within-chunk face neighbours. The attribute pass runs four bricks a workgroup over surface voxels only, emits no clear items, and reads a LOD-only neighbour through its level-1 mip. Every golden renders byte for byte. On the dev box the level's mirror reads 62 846 720 B plain (232 436 992 B before) and 157 872 640 B contour (680 491 136 B), the default at 16/24 holds `r_full` 16 at 871 679 852 B live with pressure 0.65, the attribute pass reads p99 0.71 to 0.75 ms with a 25 ms backlog age, and plain `sync` p50 0.17 ms (about 4.9 ms before); the default look's `sync` p50 reads 3.1 ms, a followup. The fly-through gains `--adapter SUBSTRING` and a `bins:` line at full residency; the first slow-adapter (RADV iGPU) readings are in `docs/nfr.md`.

- **BREAKING: the mirror's slot format.** `MirrorConfig::max_shard_slots` is `max_shard_units`; the records lose the chunk palette; the masks buffer binding is gone; `AttribItem` carries the entry less `FULL_BIT`; `AttrWorkStats::clears` is gone; `march.wgsl` and `attrib.wgsl` read the header unit, the surface mask and popcount addressing.
- **Added: `UNIT_BYTES`, `UNIT_WORDS`, `PAGE_UNITS`, `BINS`, `bin_of`, `slot_units`, `payload_units`, `attr_offset`, `surface_attr_units`, `chunk_surface`, `BinStats`, `GpuWorld::bin_stats`, `live_units`, `spare_units`, `spare_slots`, `bin_of_unit`, `attrib::BRICKS_PER_GROUP`; in `vxe-world`, `solid_mask` and `surface_mask`.**

## [0.28.0] - 2026-10-22

Plan 0030 (a minor bump: public API broken and added; ADR-0024 accepted with an Outcome). A temporal upscaler of our own, shipped opt-in. `Frame` renders at an internal size and presents at an output size (`RenderScale` 1, 1.5 or 2); each `View` carries a per-frame Halton (2, 3) sub-pixel jitter of 16 phases (`jitter_at`); the temporal pass and the reservoir reuse reproject as if every ray went through its pixel's centre (`reproject_centred`) and accept history from the 3 x 3 previous pixels. When the frame jitters or the sizes differ, an upscale pass at the output size reprojects last frame's output by the nearest hit's world position, rejects it in edit boxes, on a mismatch or a miss, clips it to the YCoCg box of the 3 x 3 samples (still pixels only where the lamp's term at the hit changed), reconstructs the current colour with a 3 x 3 Gaussian of width 0.6 output pixels, fetches history through Catmull-Rom and blends by the nearest sample's nearness; an RCAS sharpening pass follows. On the RTX 3080 Laptop at 1080p output and scale 1.5 upscale, sharpen and composite read 0.76 ms (the 1.5 ms line) and the frame about half of scale 1's, on the RADV iGPU 18.8 against 38.1 ms (loaded readings). The owner's windowed check kept Plan 0024's frame as the default: the library and the fly-through default to scale 1 with the jitter off (no upscaler); `--internal-scale 1.5 --jitter on` is the performance option (the fly-through's default became scale 1.5 with this upscaler, jittered, in 0.33.0, Plan 0041; the library's default is unchanged). An FSR 2 port is the next upscaler plan (Plan 0041, ADR-0034). New golden `upscaled.png`; every other golden is unchanged.

- **BREAKING: `View` gains `jitter` and `_pad2` (112 bytes); `Params` gains `output_size`; `Frame::target` is the output-size image while upscaling; `project_prev` in `march.wgsl` no longer takes a jitter.**
- **Added: `Frame::with_sizes`, `set_sizes`, `internal_size`, `output_size`, `set_jitter`, `jitter`, `upscaling`, `set_sharpness`, `sharpness`, `upscale_history`, `set_upscaler`, `upscaler`; `RenderScale`, `jitter_at`, `JITTER_PHASES`, `UPSCALE_WGSL`, `SHARPEN_WGSL`; `marcher::{Upscaler, Reconstruction, HistoryFetch, Blend, StillClip, LAMP_FRAMES}`.** `FRAME_PASSES` gains "upscale" and "sharpen".
- **Added (fly-through): `--internal-scale 1|1.5|2` (default 1; 1.5 since 0.33.0), `--jitter on|off` (default off; our upscaler jitters unless `--jitter off` since 0.33.0), `--sharpen S` (default 0.5) and the HUD's upscale row while the upscaler runs.**

## [0.27.0] - 2026-10-22

Plan 0024 (a minor bump: public API broken and added; ADR-0018 and ADR-0025 accepted with Outcomes). Direct light that holds in the dark. The marcher stops shading: `Frame` (the old `Marcher` is now its hit pass) runs a hit pass that writes a per-pixel hit record, an optional A-SVGF gradient pass, a light pass, a temporal pass, a variance pass with four a-trous iterations, and a composite with exposure and an ACES tone map into the `rgba8` target, over a list of `View`s. The mirror extracts exposed emissive voxels into a reserved light-sample buffer (16 B a sample); only chunks within `MirrorConfig::light_radius` of the focus hold a range, written nearest first under a per-sync budget and evicted farthest first, and each frame a power-over-distance table of those chunks feeds the light pass's candidates. The light pass takes the headlamp and one emissive sample resampled from 8 candidates, with spatiotemporal reservoir reuse or the flat list, and one shadow ray through the full or the coarse (2^3 block) walk. The temporal pass reprojects by hit position, resets history inside the frame's edit boxes (`Frame::invalidate`), keeps the emissive part on a long history (`Lighting::emissive_history`, 32) and the lamp's on a short one, and raises the emissive blend for 12 frames after an emission-scale change (`Frame::set_emission_scale`). Lighting runs at full or half resolution with a depth-and-normal upsample. Defaults (the owner's picks): half resolution, coarse walk, reuse, edit-box history. On the dev box at 1080p the defaults read light 2.0 to 2.8 ms, light plus temporal 2.1 to 2.9 ms against the 6.0 ms line and a-trous 1.1 ms against 3.0; full resolution misses both lines, and light extraction (p99 2.2 ms, max 6.1 ms against 1.0 and 3.0) is a recorded miss for Plan 0028. The fly-through gains `F3` to `F7`, `M`, a 3 s mode banner after every change, the HUD's pass rows and the flags `--lighting`, `--shadow-walk`, `--sampler`, `--history`, `--emissive-history`, `--filter`, `--lamp`, `--emissive-ore` and `--light-radius`; the ore glows. Every shaded golden is re-blessed.

- **BREAKING: `Marcher` is the hit pass of the new `Frame`;** shading moved to `Frame`'s passes, and `Params` takes views and the lighting fields.
- **Added: `Frame`, `Lighting`, `LightingResolution`, `Sampler`, `ShadowWalk`, `HistoryKill`, `ToneMap`, `CandidateDraw`, `View`, `MAX_VIEWS`, `MAX_EDITS`, `FRAME_PASSES`, `GRADIENT_STRATUM`, `DEFAULT_EMISSIVE_HISTORY`, `LightParams::turn`, and `GpuWorld`'s light samples (`LightSample`, `light_buffer`, `light_range`, `light_chunks`, `light_table_base`, `light_radius`, `light_span`, `light_samples`).**
- **BREAKING: `MirrorConfig` gains `light_samples`, `light_chunk_cap`, `light_radius` and `light_sync_samples`; `SyncStats` gains `light_*` counters.** A struct literal adds them. Storage buffers per stage rise to 10.

## [0.26.0] - 2026-10-21

Plan 0027 (a minor bump: public API broken and added; ADR-0022 accepted with an Outcome). Mirror uploads through a staging belt. `GpuWorld::sync` takes the frame's `CommandEncoder` and writes every staged run (pool slots, masks, records, table runs, material rows) into a `wgpu::util::StagingBelt` slice of 4 MiB chunks with a `copy_buffer_to_buffer` recorded into that encoder, recalled on submit; `Queue::write_buffer` is gone from `sync`. The mirror's bytes are unchanged (readback equality under a 16 MiB budget, every golden byte for byte). A sync that leaves the belt holding more than `BELT_KEEP_CHUNKS` (3) chunks replaces it, so an unbudgeted first sync's burst is released. On a quiet dev box plain `sync` p50 reads 1.40 to 1.66 ms (3.28 to 3.69 ms before); `nfr.md` section 2's 1.0 ms line and the plan's under-5 % bar for the belt's own share (25.4 %) stay recorded misses, accepted by the owner. The fly-through prints a `frames:` line at every exit (p50, p99 and max of frame CPU, `update + poll + sync` and march GPU), a `staging belt` line on the autopilot and `--frames` exits, and its autopilot's unready-near ring follows `r_full`.

- **Changed: `GpuWorld::sync(world, queue, encoder)`**; the caller submits the encoder before anything reads the mirror. `SyncStats::writes` counts belt copy commands.
- **Added: `GpuWorld::belt_chunks`, `GpuWorld::belt_trims`, `BELT_CHUNK_BYTES` (re-exported at the crate root) and `gpu_world::BELT_KEEP_CHUNKS`.**

## [0.25.0] - 2026-10-21

Plan 0031 (a minor bump: a new cargo feature; no default-build API change). Edit cost, third pass. `World::apply` builds each brick's eight shape words straight from the op's row spans, solved once per op (`OpRows`: a sphere's rows walked by `sphere_row_spans` over the op's whole bounds, another shape's solved at first use), instead of a per-brick 64-entry span array and `shape_words`; an `Inside` brick takes `box_words`. Voxels, results and every pinned hash are unchanged, and a new test pins the `worst` ops' 2721 captured brick-edits and their spans hash. The feature `edit-stages` on `vxe-world` adds a stage timer (`World::apply_staged`, `EditStages`, both doc-hidden) and the edit bench's `stages` arm; the default build reads no clock. On a quiet dev box `worst_warm` reads 0.77 ms (1.32 ms for the previous tree in the same window) and the fly-through `B` line 0.99 to 1.00 ms, and `nfr.md` section 4's edit-cost line returns to 2.0 ms on the dev box.

- **Added: feature `edit-stages` on `vxe-world`**, with `#[doc(hidden)] World::apply_staged` and `#[doc(hidden)] EditStages`.

## [0.24.0] - 2026-10-21

Plan 0020 (a minor bump: public API added and broken; ADR-0021 accepted, superseding ADR-0015's level and room-change clauses; ADR-0020 accepted). Cavern reverb in `vxe-audio`. `Acoustics` traces a 128-ray fan from the listener in slices of 16 (`RoomEstimate`, `RoomTargets`, `Command::SetRoom`): volume, surface, absorption, Eyring's `T60` (capped at 8 s) and the tail's energy, with no tail for a fan that hits nothing and a fan from inside a solid discarded. The mixer owns two sixteen-line Householder feedback delay networks; a room change hands the tail to the idle one with an equal-power input crossfade (`TailLines::Handover`, the default; `Glide` kept), so no line holding signal moves. Early taps are keyed by ray direction, glide at the walk's own rate and fade instead of swapping, and sum into sixteen direction buses each heard through its own HRIR; the echo lines hold 600 ms and reflections reach 100 m. `MixSettings` (`Mixer::set_mix`, `Command::SetMix`) carries an echo trim, a tail trim, a tail pre-delay, an optional direct-path propagation delay (off), and `RoomCurve`s of the room's mean free path for the echo and tail levels, whose default knots are the owner's listening picks (`MixSettings::flat()` restores the physical levels). A streamed voice (`Stream`, `Command::PlayStream`, `PlayParams::direct`) and `Input` play a microphone at the listener. The fly-through gains `--room`, `--mic`, `--echo-db`, `--tail-db`, `--predelay-ms`, `--flat-mix` and the keys `Y`, `[`, `]`, `-`, `=`, `9`, `0`, `O`, `U`. The mixer's 0.6 and 1.0 ms dev-box proposals are recorded misses (0.6897 and 1.1441 ms on a loaded box), accepted by the owner.

- **BREAKING: `AcousticTargets` loses `tap_count` and gains `send`; taps are slot-indexed, and `reflections()` returns `[Option<Reflection>; 16]`.**
- **BREAKING: `Command` gains `SetRoom`, `PlayStream` and `SetMix` and no longer derives `Clone`; `PlayParams` gains `direct`; `AudioError` gains `NoInputDevice` and `Curve`.** An exhaustive match or literal adds them.
- **Changed: a reflection keeps `(1 - A) ^ 0.2` of its amplitude** (one 10 cm skin in and out), and `MixSettings::default()` carries the room curves, so echoes and tail are louder or quieter than the physical levels by room size.

## [0.23.0] - 2026-10-21

Plan 0023 (a minor bump: public API added and changed; ADR-0017 accepted with an Outcome). The render thread's mirror CPU. The attribute work list is a pure builder, `vxe_render::attr_work::{build, build_split}` over a `MirrorView` that `GpuWorld` implements, counted in `AttrWorkStats`; a shell chunk looks up its 26 neighbour chunks and visits only full neighbours' border bricks, so a chunk crossing's evictions cost 26 lookups each instead of 488 brick visits. `MirrorConfig::attr_cap` caps the compute items per `update_attrs` (`usize::MAX` from `new`; the fly-through's streaming mirror sets 8192) and keeps the rest in a carry-over queue resolved to slots when taken; an edit's re-uploaded bricks and their neighbours always run first, and a brick written this sync whose compute waits gets a clear item (`attrib::CLEAR_BIT`, bit 31 of the slot word) that writes "no plane", so it shows face-mode normals, never stale planes. `GpuWorld` gains `prepare_attrs`, `encode_attrs`, `attr_stats`, `attr_list`, `attr_queue_age` and `attr_buffer_builds`; `AttribPass::reserve` sizes the work buffer once. `Streamer::update` and the residency sweep are incremental for one-chunk focus steps, equal to the full rebuild under a test oracle (`set_incremental_oracle`, `oracle_checks`, doc-hidden), and `GpuWorld`'s window shift reads the leaving slab. The autopilot prints per-stage streaming CPU, the builder's counters, the carry-over age and a GPU-credited `frame bar` line. On the quiet dev box the contour look's `update_attrs` p99 fell from 138 to 180 ms to 5.5 to 6.0 ms (a recorded miss against 4 ms); the streaming line stays a recorded miss at about 5 ms p50 in plain flight. No world hash moved.

- **Changed: `GpuWorld::update_attrs` returns compute plus clear items**, and `attrib.wgsl`'s work item reads bit 31 of its slot word as a clear flag.
- **BREAKING: `MirrorConfig` gains `attr_cap`.** A struct literal adds `attr_cap: usize::MAX` for the old behaviour.

## [0.22.0] - 2026-10-20

Plan 0022 (a minor bump: public API added and broken; ADR-0016 accepted, amending ADR-0008's drop clause). A falling region crushes what the support query calls load-free. `World::drop_region_through(cells, max_drop, yield_below)` passes through, and destroys, every solid voxel outside the region whose cell held fewer than `yield_below` solid voxels when the op started and whose material is not `INDESTRUCTIBLE`; the destroyed voxels are `DropResult::crushed`, and `touched`, `aabb` and `edit_result` include them. `drop_region` is `drop_region_through` with a threshold of 0 and is unchanged. `World::pile_rubble(cells, rubble, count, max_depth) -> PileResult` spreads `count` rubble voxels over the cells' footprint columns with `remove_region`'s rubble stage, now shared, without its carve. On the wire a drop with a threshold is op kind 5 (`OP_DROP_THROUGH`, `23 + 12n` bytes) and a pile kind 6 (`OP_PILE`, `27 + 12n`); a drop with threshold 0 stays kind 3 byte for byte, so earlier saves replay unchanged, and a kind-5 entry with threshold 0 is the new `WireError::NonCanonical`. `LOG_FORMAT` stays 2. The fly-through's logic drops with `YIELD_BELOW` (`vxe_net::DEFAULT_SUPPORT.solid_min`, which the fly-through now sets on its server), piles a fall's whole crushed volume as rubble when it lands (`op piled`, `landed ..., crushed K voxels, P rubble`), prints `wedged ...` for a region that cannot move and `drop refused: ...` for a refused drop, and remembers up to `STUCK_MAX` (64) stuck regions so as not to report them again until a dig changes them. The collapse course hash moves to `0xd223_d848_c8fc_ca0f` (blessed on Linux); every session hash stays.

- **BREAKING: `Op::Drop` gains `yield_below: u16`, and `Op` gains `Pile`.** An exhaustive literal adds `yield_below: 0` for the old behaviour; an exhaustive match on `Op` adds `Op::Pile`.
- **BREAKING: `DropResult` gains `crushed`, and `OpResult` gains `Pile(PileResult)`.** An exhaustive literal adds `crushed: vec![]`; an exhaustive match adds the variant.
- **BREAKING: a drop's op result encodes its `crushed` counts after `moved`.** A logic component built against 0.21 misreads a drop result; rebuild it.

## [0.21.0] - 2026-10-20

Plan 0026 (a minor bump: public API added, nothing broken; ADR-0019 accepted). Gamepad flight for demos. `vxe_app::Input` carries a gamepad's analog axes (`Axis`: `MoveX`, `MoveY`, `LookX`, `LookY`, `TriggerL`, `TriggerR`) and pad buttons (`PadButton`, by position) that name no gamepad library, with `axis`, `pad_held`, `pad_pressed`, `set_axis`, `pad_down`, `pad_up` and `release_pad`; axes persist across frames and pad presses clear at `end_frame`. `App::poll(&mut self, &mut Input)` runs before every `frame`, with a no-op default. The fly-through owns `gilrs` (`=0.11.2`, fly-through only) and feeds it in `poll` under `--pad on|off` (on by default in a window, never headless or under the autopilot; a machine where gilrs cannot start runs without a pad). The left stick moves at a speed set by its deflection (radial dead zone 0.15, squared response), the right stick looks at 2.5 rad/s, `R3` latches boost, `L1`/`R1` sink and rise, `R2` digs and `L2` fills on a press edge, the D-pad steps the dig radius, and the face buttons, `Start` and walking `South` mirror `G`, `X`, `H`, `F2` and `Space`. The newest connected pad is active; each connect and disconnect prints a line, the startup line names the pad and its power or `pad: none`, and the HUD gains a `pad` row (`HudStats::pad`, `PadRow`, `PadPower`). CI's Ubuntu arm installs `libudev-dev`. No `nfr.md` line moves.

## [0.20.0] - 2026-10-20

Plan 0025 (a minor bump: public API added and one struct literal broken; no ADR). The cutaway view: with `LookParams::cutaway` (`Params::flags` bit 10, `vxe_render::marcher::FLAG_CUTAWAY`) the primary ray passes through solid until it has entered an empty cell of a resident chunk, then hits the next surface as usual, so a camera in rock or outside the world sees each cavern's far wall through the hull and rock with nothing behind it is a miss. Hits are shaded by a fixed hemisphere light (world z up) with no lamp or shadow ray, and a miss is a flat grey-blue sky. The fly-through turns it on while the camera's voxel is neither air nor translucent (`fly::CutawayMode`); `X` cycles auto, on and off, `--cutaway auto|on|off` sets the start state, and the HUD shows a `cutaway` row while it is on. New golden `cutaway_level.png`. A debug view, off in air, so no `nfr.md` line moves; the steps view from 40 m outside the level reads a mean of 49.53 steps per ray, p99 170, on the dev box (diagnostic under §2).

- **BREAKING: `LookParams` gains `cutaway: bool`.** An exhaustive struct literal adds `cutaway: false`, or uses `..LookParams::default()`.

## [0.19.0] - 2026-10-20

Plan 0013 (a minor bump: public API changed and added; ADR-0011 Outcome addendum). The fly-through starts in contour with procedural detail on and attribute pages in the mirror, with rock at `[4.0, 0.1]` (`flythrough::ROCK_DETAIL`). `--look plain` starts in face mode without attribute pages or rock detail, the look of the earlier face readings. A contour-plane hit's shadow ray starts half a voxel off the hit along the plane normal, not the face normal, so contour's only-with-shadows specks on the golden scene fall from 372 to 52, under face mode's 169 (`look_contour.png` reblessed). `AttribPass` keeps its uniform, a work buffer grown to powers of two and its bind group between calls (`AttribPass::buffer_builds`, `work_capacity`). The GPU chunk table is created filled with `ABSENT`. The autopilot reports the attribute pass's work list, render-thread CPU and GPU time over its flight, and `world_budget --surface` counts plane voxels. No footprint or residency change: under the 3 GiB reservation the pressure rule holds the default at `r_full` 12 to 13, peak live 2.86 GB. On the dev box, under load, the attribute pass's render-thread cost under motion reads p99 139 to 167 ms, a visible hitch, and is an open followup.

- **BREAKING: `AttribPass::encode` takes `&mut self` and a `&wgpu::Queue`.** It writes its uniform and work list through the queue, so at most one encoded pass may be submitted per `queue.submit`.
- **Changed default: the fly-through's look.** Without `--look` it is contour with detail. `--detail` matters only under `--look plain`, and `--rock-detail` is refused there.

## [0.18.0] - 2026-10-20

Plan 0021 (a minor bump: public API broken, added and removed; ADR-0012 Outcome addendum). A `Full` brick is a packed box: a 512-bit occupancy mask, an ascending palette of its non-air materials and bit planes indexing it. It comes in five fixed classes by material count (72, 136, 200 and 352 B, and 1024 B of raw ids above 16), behind the same 16-byte slot, and its encoding is canonical, so `Brick` equality is voxel equality. Edits are word operations with popcount tallies. Raycast and `trace_materials` skip a clear 2^3 block as one cell, and the coarse cell, region counts and `vxe-sim` bodies read masks and popcounts natively. Collapse moves whole z-layers and merges palettes. The mirror upload maps palette entries from their match words and leaves every mirror byte as before. Every pinned hash and golden is unchanged. On the dev box, resident chunks at the streamed defaults fall from 3.24 to 0.44 GB, and the warm worst-case edit from 3.03 to 1.34 to 1.52 ms (a loaded reading). Added: `Brick::decode`, `modify`, `class`, `payload_bytes` and `for_each_solid_in`; `Packed` and `PackedClass` (in the hidden `packed` module); `PackedStats` (`World::packed_stats`); `BrickCensus` and `WorldCensus` (`World::brick_census`); the fly-through's `--census`, and `world_budget --census` and `--veins`.

- **BREAKING: `Brick::Full` holds an opaque `Packed`, not `Box<[u16; 512]>`.** A match on `Brick::Full(v)` that indexed `v` reads through `Brick::get` or `Brick::decode` instead. A write goes through `Brick::modify`, which re-encodes the brick in canonical form. Build a brick with `Brick::from_voxels` or `Brick::from_fn`, as before.
- **BREAKING: the brick-array pool is removed:** `World::reserve_brick_arrays`, `World::pool_len` and `World::BRICK_ARRAY_POOL_CAP`. A packed box is allocated on the edit path, which costs 1.8 % of the warm worst case.
- **BREAKING: `SPILL_VERSION` is 2.** A brick is a tag byte and its class words. A version-1 spill file reports `SpillError::Version`, and a non-canonical box reports `Corrupt`.

## [0.17.0] - 2026-10-20

Plan 0017 (a minor bump: public API added and changed; ADR-0013 accepted, superseding in part ADR-0004's growth Outcome). The mirror reserves its pool shards, masks and records once at creation from `MirrorConfig::budget_bytes` (default `DEFAULT_BUDGET_BYTES`, 3 GiB) and `MirrorConfig::record_chunks`, split by `ShardLayout::with_budget`, and never creates or copies a buffer again (`SyncStats::buffers_created` and `copies` read 0). An empty pool page serves either kind, plain or attribute. `GpuWorld::pressure` is the larger of pool-slot and record occupancy; the fly-through lowers `r_full` a chunk after 30 frames above `PRESSURE_HIGH` (0.9) and restores it after 300 under `PRESSURE_LOW` (0.7), through the new `Streamer::set_radii`. Added: `budget_for`, `PAGE_BYTES`, `PLACEHOLDER_BYTES`, `ShardLayout::reserved_slots`, `reserved_records` and `reserved_bytes`, `GpuWorld::reserved_slots`, `reserved_records`, `pages_issued`, `free_pages`, `spare_slots`, `MirrorError::RecordsTooLarge`, `SyncStats::deferred_chunks`, `vxe_world::chunks_in_ball`; the fly-through's `--mirror-budget` and `--residency` and a HUD pressure row.

- **Changed meaning: `GpuWorld::allocated_bytes()`** is the reservation (the budget rounded down to whole pages and records), fixed from creation, not bytes grown on demand. The `nfr.md` §3 line is now read on live bytes (`mirror_bytes`) against it.
- **Changed contract: `GpuWorld::sync`** writes the nearest pending chunks that fit the reservation, each whole, and defers the rest, which stay pending with their dirty bits (`SyncStats::deferred_chunks`). A chunk is never written in part. A demotion to LOD-only is never blocked by an earlier deferral. A sync that would write nothing returns `PoolTooLarge` or `RecordsTooLarge` and writes nothing, the material rows and queued table changes included; before, a sync past the pool grew it.

## [0.16.0] - 2026-10-20

Plan 0019 (a minor bump: public API added and changed). `vxe-world` holds `WorldLimits` per world (`World::limits`, `World::with_limits`) and refuses an out-of-range shape, ray, box query, drop or removal with `EditError::OutOfRange { what: Field }` (wire error tag 2) before any arithmetic, changing nothing: coordinates and radii within 2^30 fixed units, a capsule's axis within 2^30, an edit's bounds within 1 024 voxels per axis, a ray's reach within 2^22 fixed units per axis, a box query within 1 024 voxels per axis and 2^30 voxels, a region list within 4 096 cells. `World::raycast`, `count_in_box` and `count_in_box_with_stats` return `Result`; `World::raycast_with_stats` and `find_unsupported_with_stats` report their steps. The `wasmtime` host charges each import's work to the tick's fuel at `WorkRates` on `WasmConfig` (per call, per voxel, per step, per argument byte, the bytes before any host work), so a greedy query ends the tick on fuel; `WasmLogic::fuel_left`; `Host::raycast_with_steps` and `Host::unsupported_with_steps` (provided); `vxe_net::max_changed_cells` caps the server's support query. ADR-0010 Outcome addendum.

## [0.15.0] - 2026-10-20

Plan 0018 (a minor bump: public API added and changed). Stamp, drop and remove are op kinds 2 to 4 (`Op::{Stamp, Drop, Remove}`, no longer `Copy`) with wire encodings, applied through `Server::submit`, `Host::apply(Op)` and the WIT `apply`, logged, and announced as `OpResult::{Edit, Drop, Remove}`, which the owned client checks. A stamp names its prefab by content id in a `PrefabRegistry` (ADR-0014). `LOG_FORMAT` 2 carries the stamped prefabs in the save's header, verified on decode (`WireError::PrefabMismatch`), and format 1 still reads. `EditError::UnknownPrefab` takes wire error tag 3. The server keeps a `CoarseGrid` current after every applied op and answers `Host::unsupported` (a WIT import too), configured by `Server::set_support`. The fly-through's `P` stamp and collapse run in `DigLogic` (`DigInput::{Stamp, Crumble}`), and `--save`/`--replay` no longer refuse them or `--vault`.

## [0.14.0] - 2026-10-19

Plan 0015 (a minor bump: public API added and changed). The marcher walks a hierarchy of occupancy bits: 512 brick bits head each chunk record and `EMPTY_BIT` in the chunk table skips an empty chunk on its table read alone; a 512-bit opaque mask per brick (72 B a slot, in groups of eight) gives the 4^3, 2^3 and voxel skips from one 64-byte line; the walk holds its chunk, bits words, brick entry and mask slab across steps. The streamed cave's spawn march falls from 6.8 to 1.95 ms on the dev box. A cell exit follows plane order, so the image is independent of every skip (`LightParams::skips` is the comparison switch; the goldens re-blessed once). The translucent class: `MaterialFlags::TRANSLUCENT`, `Material::opacity`, `TRANSLUCENT_BIT` in brick entries and a bounded second walk (`LookParams::translucent_layers`, default 4, `--translucent-layers`), with `ViewMode::Albedo`. A `steps` debug view (`ViewMode::Steps`, `readback::StepHistogram`, a HUD row). `Brick::solid_bits`. `gpu_world` layout constants change (`RECORD_WORDS` 656, `MASK_BYTES` 72, `SLOT_MASK` and the record index 29 bits).

## [0.13.0] - 2026-10-18

Plan 0016 (a minor bump: public API added and changed). The character controller lifts a body an edit filled into to the nearest free pose (`MoveResult::lifted`) or reports it `embedded` (ADR-0005 Outcome addendum). A hosted logic that traps restarts from its seed up to `WasmConfig::max_restarts` (default 8) and reports `LogicError::Restarted` through `Logic::take_restart` and `TickReport::restart`, replacing fail-stop (ADR-0010 Outcome addendum). `World::remove_region` carries the rubble rounding remainder across columns, so a thin region leaves about a quarter of its volume as rubble. The fly-through banks fractional scroll so smooth-scrolling devices step the dig radius in 0.1 m steps. The HUD refreshes without allocating (`HudOverlay::set_lines`, `allocations`); `PassTimer::read_ms` drops its queue parameter.

## [0.12.0] - 2026-10-17

Plan 0006 (a minor bump, a feature plan): the `vxe-audio` crate. A headless mixer renders positional voices to binaural stereo through a packed SADIE II head (subject D2, 2950 directions, 128 taps, Apache 2.0) and plays them through `cpal` on the device callback, which allocates nothing and drains a bounded command channel per block. `World::trace_materials` walks the integer DDA through solids as material runs; `Acoustics` probes occlusion and transmission through the material table's acoustic triple and sixteen early-reflection taps per voice on the caller's thread. The fly-through plays a knock, eight hum notes, a looping pulse, toggles the echoes and meters the callback under `--audio-debug`. ADR-0006 accepted.

## [0.11.1] - 2026-10-17

Plan 0012 (a patch bump, the owner's call on 2026-10-17 over the close's "no version"): CI runs `scripts/check-comment-hygiene.mjs` (no relative link or plan-relative narration in `.rs`/`.wgsl` comments) and `scripts/check-index-rows.mjs` with `--self-test` (rows inside `roster:begin` markers at most 320 bytes); seeded fixture trees under `scripts/fixtures/`.

## [0.11.0] - 2026-10-17

Plan 0011: the scripting half of M5 (ADR-0010, accepted). A logic crate builds natively or as
a WebAssembly component, and a `wasmtime` host runs the component so that it can observe
nothing but its seed and its inputs. The same logic, native and as a component, gives
byte-equal op logs and equal world hashes from the same inputs: the echo logic pins
`0x8f7d_4016_15d9_5c4d`, and the fly-through session gives `0x86ca427a47e8908c` either way
(asserted on Linux; the Windows arm confirms it after a push). Readings (`docs/nfr.md`
sections 4 and 7; AMD Ryzen 9 5900HS, `wasmtime` 49.0.3, 07a3be1): a host call costs 1.37 to
1.42 us over native, which misses the 1 us proposal; the budget is now 2 us per `apply`
call. The `Bench` tick through the component is 1.031 to 1.034x native, under the 2x budget.
An empty component tick reads 1.1 to 1.25 us. The incremental default build reads 1.46 to
1.48 s and compiles no `wasmtime`.

- New crate `vxe-script-wasm` (not a default member): the WIT world `wit/logic.wit` (`init`,
  `tick`, and host imports `apply`, `material-at`, `raycast`, `count-in-box`, whose `hit`
  record mirrors `vxe_world::Hit`). `WasmLogic` implements `Logic` for a component, with
  `load`, `try_init` and `try_tick`. `WasmConfig` sets the fuel per call, a memory cap over
  all of the component's memories, and the clocks' nanoseconds per tick. The WASI context is
  deterministic: both clocks read `tick x frame_ns`, timers are always ready, entropy is a
  stream seeded from the seed and the tick, stdout is discarded and stderr is kept per call
  for a trap's message. There are no files, environment, arguments or network. A logic that
  fails a call (fuel, memory cap, trap, or a panic in a host import) is stopped for the run.
  Features: `host` (default, `wasmtime`) and `guest` (the bindings and `export_logic!`). The
  test components are under `guests/` (echo, probe, spin, hog, timer, chatty), with the
  `boundary` bench.
- `vxe-net`: `LogicError` (`Load`, `Trap`, `Fuel`, `Memory`, `Stopped`); a provided
  `Logic::take_error`; `TickReport::logic_error`, which makes `TickReport` no longer `Copy`
  (breaking); `Server::init_error`. A `test-logic` feature exports the test world, the seeded
  carves and `EchoLogic` for other crates' tests.
- `apps/flythrough`: the dig logic moves to the `flythrough-logic` crate
  (`apps/flythrough/logic/`), re-exported as `flythrough::logic`; it is a native rlib, or a
  component with its `component` feature. `--logic native|wasm[=PATH]` picks the logic. Its
  `wasm` form needs a build with `--features wasm`, off by default. A failed hosted logic is
  printed once, and the session continues without it.
- `scripts/build-guests.sh` builds every component into `target/guests/`. CI installs
  `wasm32-wasip2` and builds the guests. It runs `nextest -p flythrough --features wasm`, the
  `--features wasm` clippy, and `wasm32-wasip2` clippy on the guests and on
  `flythrough-logic --features component`. `rust-toolchain.toml` lists the target.
- New workspace pins: `wasmtime` and `wasmtime-wasi` `=49.0.3`, `wit-bindgen` `=0.61.1`,
  `rand_core` `=0.10.1`, `getrandom` `=0.4.3`, `wasip2` `=1.0.4`, `bytes` `=1.12.1`, `tokio`
  `=1.53.3`.

## [0.10.0] - 2026-10-17

Plan 0007: the simulation half of M5 (ADR-0007, accepted; ADR-0010's trait half, still
proposed until Plan 0011). The server owns the world and its op and input logs, clients talk
to it over a byte channel, a `Logic` decides edits from inputs through a `Host`, and a session
is saved as seed plus logs and rebuilt by replay or re-simulation. Readings (`docs/nfr.md`
section 4; AMD Ryzen 9 5900HS, CPU only, c844a8f): 100 inputs through one tick cost 84.7 us
more than 100 direct applies, under the 100 us figure that is now a budget; decoding and
replaying 10 000 radius-1 m carves takes 279 ms (about 35 800 ops/s, bound by the edit cost;
no budget). A sphere carve entry is 47 bytes. The op-log hash `0x5ac8_737d_db0f_ccec` is
blessed on Linux; the Windows arm confirms it after a push.

- New crate `vxe-net`: `Channel`, `MemChannel` and `Endpoint`; `Message` (`Applied`,
  `Refused`, `Event`, `Input`); `Logic`, `Host` and `NoLogic`; `Server` with `submit`,
  `attach`, `tick` (logging a boundary entry after directly submitted ops) and `resimulate`
  over the op and input logs; `Client::shared` (the server's `Arc<World>`) and
  `Client::owned` (its own world, `Event::Desync` on the first mismatched result).
- `vxe-world`: `OpLog` (op entries and the `0x80` boundary entry), `InputLog`, `LogFile` (a
  60-byte header of seed, source version and extent, then both logs), the `wire` module with
  `WireError` including `OutOfOrder`, and `World::replay`.
- `apps/flythrough`: `DigLogic` decides digs, fills and the bench, and the binary sends them
  as inputs through a server and a shared client. `--save PATH` and `--replay PATH` under
  `--level` write and rebuild a session (`ReplayError` covers a cave-version or extent
  mismatch). `--bench N` sends N bench inputs on the first frame. `P`, `--vault` and collapse
  are refused under `--save` (`--vault` and collapse also under `--replay`), since they edit
  the world outside the op log.
- Tests: `crates/vxe-world/tests/log.rs`, `crates/vxe-net/tests/` (two-thread submission,
  interleaving and re-simulation, owned-client desync), `apps/flythrough/tests/replay.rs`, and
  the op-log hash in `apps/flythrough/tests/determinism.rs`. Benches: `vxe-world`'s `replay`
  and `vxe-net`'s `tick`.

## [0.9.0] - 2026-10-17

Plan 0010: M7's stats HUD (ADR-0009, accepted). The fly-through gets a toggleable overlay of
text rows and a frame-time strip, drawn by one blended pass with no UI library. Readings
(`docs/nfr.md` sections 2 and 3; NVIDIA GeForce RTX 3080 Laptop GPU, Ryzen 9 5900HS, f91726f):
the HUD pass takes 0.038 ms at 1080p and scale 2, against a 0.1 ms proposal. With the HUD shown
on `--level`, the march reads +0.06 % and the frame mean +4.0 %. `World::stats` takes 14.85 ms
on the streamed cave, so it is sampled once per 240 frames. The overlay holds about 130 KiB of
VRAM by arithmetic. The on-screen check awaits the owner.

- `vxe-render`: `HudOverlay` (`set_text`, `set_rows`, `set_samples`, `encode`,
  `encode_timed`), `HudColour`, the embedded `font8x8` table and `hud.wgsl`. The pass draws an
  8 x 8 glyph grid at an integer scale and a bar strip with a 16.67 ms line onto the target
  after the blit.
- `vxe-render`: `PassTimer`, a two-timestamp GPU pass timer. `MarchTimer` is now an alias of
  it. Breaking: `MarchTimer::new` takes the queue and returns `Option` (`None` without
  `TIMESTAMP_QUERY`).
- `apps/flythrough`: `F2` toggles the HUD and `--hud` starts with it shown. The rows show the
  frame window (mean, median, p99, max), the CPU time, the march and HUD GPU times, the upload,
  the mirror and pool, resident chunks and CPU bytes, the last edit, the camera and chunk, and
  RSS on Linux. `--frames N` prints the same rows at exit. The world's stats and RSS are
  sampled at a cadence while the HUD is shown.
- Tests: `crates/vxe-render/tests/hud.rs` (glyph pixel counts, the strip, an ignored 1080p
  reading) and `apps/flythrough/tests/hud_golden.rs`, with its baseline `golden/hud.png` blessed
  on lavapipe. With the HUD off, the output equals `golden/primary.png` byte for byte.

## [0.8.0] - 2026-10-17

Plan 0009: the collapse half of M8 (ADR-0008, accepted). A support query on the coarse grid
reports which solid regions lost their anchor after an edit, and two integer ops drop a region
straight down or remove it with rubble. Every reading is under its proposal (`docs/nfr.md`
section 4, "Collapse cost"; AMD Ryzen 9 5900HS): the query 327-396 us against 1 ms, the slab
drop 49-72 us against 100 us, the removal 119-127 us against 200 us, and the 4096-cell cube
drop 6.8-8.6 ms against 20 ms. Three windowed checks await the owner.

- `vxe-world`: `CellCoord` (a global coarse-grid cell), `CellChange::cell` and
  `CoarseGrid::cell_at`.
- `vxe-world`: `SupportParams`, `Anchors`, `Region` and `find_unsupported`, a capped flood over
  load-bearing cells from an edit's changed cells. A chunk outside a non-empty extent reads as
  air, so `boundary_anchors` decides whether the extent's faces hold anything up.
- `vxe-world`: `World::drop_region`, a rigid in-place vertical translation by the air gap
  under the region, at most `max_drop`. `World::remove_region` carves the region and piles a
  fraction as rubble; it spares `INDESTRUCTIBLE` voxels, which `drop_region` moves.
  `DropResult` and `RemoveResult` carry `edit_result()` for `CoarseGrid::update`.
- `vxe-world` tests and benches: the `collapse` bench (query, slab and cube drops aligned and
  unaligned, removal) and the collapse course test in `tests/collapse.rs`, pinned at
  `0x3178_0a15_61c1_3041`.
- `apps/flythrough`: on `--level`, the support query runs after every dig and an unsupported
  region falls 8 voxels a frame. `R` toggles removal with rubble instead of the fall, and `F1`
  prints the last query's time. A `RUBBLE` material (row 4) joins `cave_materials()`.

## [0.7.0] - 2026-10-17

Plan 0014: the second pass on the `docs/nfr.md` §4 edit cost. Edits are cheaper, but the
2 ms figure is still missed; the choice between a brick-representation ADR and a measured §4
cap awaits the owner. Readings (AMD Ryzen 9 5900HS, `cargo bench -p vxe-world --bench edits`,
paired with the Plan 0003 tip): `worst` 3.55 -> 3.19-3.26 ms, `mixed` 2.55 -> 2.33-2.36 ms,
`tool` 8.05 -> 6.78-6.80 ms, and the new `worst_warm` 3.03-3.05 ms.

- `vxe-world`: a brick-array pool. Expanding a uniform brick takes a pooled array, collapsing
  one and evicting a chunk give arrays back. New API: `World::BRICK_ARRAY_POOL_CAP` (8192
  arrays, at most 8 MiB), `World::reserve_brick_arrays` and `World::pool_len`.
- `vxe-world`: the sphere's row spans come from a brick-row span walk, and a capsule row is
  solved analytically from its nearest centre plus a bisection instead of a linear scan.
- `vxe-world`: `World::stamp` takes and gives back pooled arrays.
- `vxe-world` tests and benches: a uniform-world reference oracle
  (`apply_matches_the_reference_on_uniform_worlds`); the `worst_warm` and `tool_warm` edit
  bench cases (after a reserve) and the `expand` bench group.
- `apps/flythrough`: tops the pool up to a 4096-array reserve after each frame's edits and
  prints the pool size on its `bench:` line.

## [0.6.0] - 2026-10-17

Plan 0008: the headless halves of M6 (`.vox` import) and M7 (prefab stamping). The stamp
bench reads 22.7 us for a 16³ shelled box and about 2 ms for a 64³ prefab against the 100 us
and 5 ms proposals (`docs/nfr.md` section 4). Two windowed checks await the owner.

- `vxe-assets` (new crate, depends on `vxe-world` only): a `.vox` parser for `SIZE`, `XYZI`,
  `RGBA`, `PACK` and the `nTRN`/`nGRP`/`nSHP` scene graph, other chunks skipped by size;
  the generated MagicaVoxel default palette; `IMAP` presence reported; `VoxMap` with
  `Slot::{Air, Material, Skip, Unset}` and `to_prefab`; scene-graph `placements()` composing
  integer translations and rotations, with `Placement::min_corner`. Hostile input is refused
  without a panic: `MAX_MODEL_EDGE` 256, `MAX_PLACEMENTS` 65_536, `MAX_TRANSLATION` 1 << 20,
  and composed translations checked against `i32`.
- `vxe-world`: `Prefab` (dense materials plus a write mask) and `Rot` (the 24 axis-aligned
  rotations); `World::stamp`, which writes every masked voxel and returns an `EditResult`
  like an edit; `StampedSource`, a `WorldSource` that blits placements after its base, in
  order and independent of the thread count (pinned hash); the `stamp` bench.
- `apps/flythrough`: `--vault` places a shelled box ahead of the spawn, and `P` stamps one at
  the aim point through the same post-edit path as a dig.

## [0.5.0] - 2026-10-17

Plan 0004: M3, collision, a character controller and coarse grids (ADR-0005). The F2 wall
sweep reads 26.3 us against the 5 us proposal (`docs/nfr.md` section 8) and awaits the
owner's call; a body embedded by an edit is not pushed out (no depenetration).

- `vxe-sim` (new crate, depends on `vxe-world` only): a vertical-cylinder `Body`, integer
  `distance2` and `overlaps`, a conservative-advancement `sweep` that leaves out boxes the
  move does not close on and never takes an advance into a box, and a `CharacterController`
  with walk, step-up, wall slide, gravity, jump with the landing speed reported, and a
  multi-tick ledge climb; a pinned course hash in `tests/course.rs`; `isqrt`,
  `MAX_SWEEP_ITERATIONS`, `sweep_with_stats`, `distance2_with_stats`; the `controller` bench.
- `vxe-world`: `CoarseGrid` (one `CoarseCell { solid, dominant }` per brick, updated from an
  `EditResult`, returning sorted `CellChange`s, with `chunk_loaded` and `chunk_evicted` hooks
  not yet wired to the streamer) and `WalkableMap` with `WalkableParams` and
  `WalkableStats`, updated incrementally from the grid's changes; the `coarse` bench.
- `apps/flythrough`: walk mode on the bounded level (`--walk`, `G`, `W A S D` at 3 m/s,
  `Space` to jump and climb), the coarse grid and walkable map kept from every edit (`K`
  prints them), and the coarse-grid lines in `world_budget --chunks`.
- Dev profile: `opt-level = 2` for `vxe-sim`.

## [0.4.1] - 2026-10-17

Plan 0003: the edit-cost pass. `World::apply` returns the same voxels and results as before
(a brute-force oracle and both pinned hashes say so) and the `worst` edit bench drops from
about 4.6 to 3.56 ms on the dev box, still over the 2 ms figure in `docs/nfr.md` §4, which
stands; Plan 0014 is the second pass.

- `vxe-world`: edits resolve the mode once per op into a target table, write each brick row
  as a span solved from the shape, and tally counts by material id and bounds once per brick.
  Public, newly: `Shape::voxel_bounds` and `Shape::contains` (were crate-private), and
  `Shape::row_span` (new), the inclusive run of voxel `x` indices in a row whose centres lie
  inside the shape.
- Tests and benches: `reference_apply` (a brute-force edit model in test support), the
  oracle test over 200 seeded ops, a row-span property test, and the `worst`, `mixed` and
  `tool` cases of the `edits` bench.

## [0.4.0] - 2026-10-16

Plan 0005: the surface-look spike, the first M2 plan. The owner chose per-voxel contour planes
with procedural detail (ADR-0011); the fly-through's default stays face normals until the
follow-up fixes contour's shadow-ray origin and its streamed footprint, which runs out of
memory on an 8 GiB card at the default radius (`docs/nfr.md` §3).

- `vxe-world`: `Material::detail` (feature scale in voxels, strength in `[0, 1]`; default no
  detail).
- `vxe-render`: `LookParams`, `NormalMode` (`Face`, `Trilinear8`, `Gradient27`, `Contour`) and
  `ViewMode` (`Shaded`, `Normals`, `HitMask`) in `Params.flags`, set with `Marcher::set_look`;
  `GpuWorld::with_surface_attrs`, a mirror whose full chunks' bricks live in attribute pages
  of the existing pool shards, filled by the `attrib.wgsl` pass (`update_attrs`,
  `update_attrs_timed`); contour-plane hits in the marcher; procedural per-material detail in
  the shader. Breaking: `MATERIAL_FLOATS` 8 -> 16 (64-byte material rows), and `free_slots`
  returns only the plain pages' free list.
- `apps/flythrough`: keys `N` and `V`; flags `--look`, `--view`, `--detail`, `--rock-detail`;
  `look_materials`, `ROCK_DETAIL`; the look harness (`tests/look.rs`: cavity normal error,
  pillar silhouette error, the comparison over every mode) and goldens `look_trilinear8.png`,
  `look_gradient27.png`, `look_contour.png`, `look_detail.png`.

## [0.3.0] - 2026-10-16

Plan 0002: the streaming half of M1, which closes M1 with recorded misses on the streaming
and streamed primary-ray lines (`docs/nfr.md` §2).

- `vxe-world`: `Streamer`, two-tier sphere residency (full detail inside `r_full`, an
  LOD-only ring of mips out to `r_lod`, with hysteresis) on a `std::thread` worker pool
  (ADR-0003); edited-chunk persistence in RAM with checked disk spill files; `ChunkMips` under
  a conservative majority rule; a dirty-chunk set on the store; `World::apply` excludes
  eviction for the whole op; unbounded worlds through `World::new`.
- `vxe-render`: a mirror window that follows the camera with a nearest-first, byte-budgeted,
  batched `sync`; 8-bit per-chunk palettes with a wide fallback; the brick pool sharded over
  four storage bindings sized from adapter limits (ADR-0004); LOD records and a mip-level
  walk in the marcher; `MirrorError::RecordsTooLarge` beside `PoolTooLarge`;
  `GpuWorld::allocated_bytes` beside the live `mirror_bytes`. Breaking: `SLOT_BYTES` replaces
  `BRICK_BYTES`, `pool_shard` replaces `pool`, `Params` changes layout.
- `apps/flythrough`: the unbounded streaming cave by default (`--level` for the bounded one),
  a headless autopilot that reads streaming CPU, residency latency, mirror bytes (live and
  allocated) and peak RSS, `world_budget --stream`, and the `lod.png` golden.
- Dev profile: `opt-level = 2` for `vxe-world` and `flythrough`.

## [0.2.0] - 2026-10-16

Plan 0001: the bounded half of M1.

- `vxe-world`: a bounded brick-map world (`u16` voxels, uniform-or-full 8³ bricks, 64³
  chunks under per-chunk locks, ADR-0002), a material table with an indestructible flag,
  integer edit ops (sphere, box, capsule, tool-along-ray; carve, fill, replace) that return
  per-material counts, touched chunks and bounds, an integer raycast, point and box queries,
  brick occupancy masks, and FNV-1a and value-noise helpers.
- `vxe-render`: a GPU mirror of the world (brick pool, masks, chunk records, chunk table)
  uploaded per dirty brick in batched runs, and a WGSL compute ray marcher with chunk, brick,
  block and voxel steps, an offset headlamp and one shadow ray.
- `vxe-app`: a `winit` shell with a surface, input and a frame loop.
- `apps/flythrough`: a seeded integer-noise cave, the fly-through with dig, fill, a carve
  benchmark and frame readings, a cross-platform determinism vector, and golden images on
  lavapipe.

## [0.1.0] - 2026-10-16

- Repository bootstrapped: workspace skeleton, harness, ADR-0001, roadmap and NFR budgets.
  No engine code yet.
