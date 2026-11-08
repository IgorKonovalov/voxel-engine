# Backlog

Ideas, parked questions and owed follow-ups that no plan, ADR or roadmap milestone carries yet.
This is the owner's parking lot: anything said in a session that is "later" lands here, dated,
so it is visible in the tree instead of in an agent's memory.

Rules:

- **One entry per idea**, dated when it was parked, with the context it came from, what it would
  take, and the condition for picking it up. Keep an entry under about ten lines; the long form
  goes in the plan or ADR that eventually takes it.
- **An entry leaves when a plan or ADR takes it.** Replace its body with one line naming the plan
  or ADR, or delete it when the owner drops the idea. The roadmap still owns the milestones; an
  idea that becomes a milestone item moves there.
- **Anyone edits this file**, by hand, in the session the idea comes up. The architect reads it at
  every plan interview and close review and asks which entries are ripe.

## Engine mechanisms

### Chunk-scale world dynamics: a cellular material step and an event scheduler

Parked 2026-11-03 and 2026-11-04. The owner wants Noita or Teardown style dynamics (sand, flowing
liquids, fire, reactions) and "large world events" that run over many chunks. Today the storage
layer is ready and the simulation layer is absent: 64³ chunks behind per-chunk locks, two-tier
streaming residency, every op returning its touched chunks, the coarse grid updated from that
result, and a server tick that only changes the world when an op is applied. Nothing revisits
chunks over time, there is no active set or dirty-chunk scheduler, and collapse (ADR-0008) is
pull-based and per edit.

Feasibility read (2026-11-03): powders, liquids, fire and material-pair reactions fit as one
deterministic op kind, "step N with seed S", through the op log, over resident chunks only, with
a brick checkerboard for order and a per-chunk dirty bitmap as the active set; cost is set by
active voxels, not resident ones, and the GPU mirror's re-upload of every active brick may
starve before the CPU does. Rigid bodies and rotating pieces conflict with ADR-0008 and stay on
the cosmetic client-side layer. Gas as voxels conflicts with M8's separate coarse volume.

What it takes: an ADR that generalises M10's "flowing liquids" item into the cellular step plus a
per-tick chunk scheduler (active set, per-tick budget, resident chunks only; rejected: GPU-side
authoritative simulation, a rigid-body engine), then a plan with a headless proof (a sand column
released over a pit lands as the same pile with the same hash on both CI arms) and a per-tick
reading against a new `nfr.md` line.

Pick up when the lighting base is settled: Plans 0056, 0057 and 0058 closed.

### Per-source indirect light

Parked 2026-10-22 (Plan 0029 live session 11), reaffirmed 2026-11-04 (Plan 0051 verdict). The
owner wants bounce light only from some sources: "fire on a stick or fire in a dungeon" feed the
probes, the headlamp and simple lights are direct only. ADR-0043 shrinks the probe volume to a
lamp-bounce channel and keeps its logic for exactly this later redesign; ADR-0044 gives lights a
fixed list. The missing piece is a per-light and per-emissive-material "feeds the bounce channel"
flag and the channel's redesign. Pick up after Plan 0058 closes and the owner has seen the
steady image.

### Render settings surface and named quality profiles

Parked 2026-10-22. Every mechanism choice (shadow walk, lighting tier, sampler, look, upscaler,
later sky, clouds, far field) should be a documented engine setting a consumer picks, with named
profiles (floor, portable, performance) the roadmap promises. Today `Frame` exposes lighting,
resolution, look and LOD at runtime; construction-time settings (mirror budget, residency) are
separate and live-versus-restart is undocumented. What it takes: a best-practices rule that a new
mechanism lands as a documented setting, and a plan for the settings surface and the profiles.
The 2026-11-03 answer on M10's RADV budget leaned on this plan coming before Plan 0032.

### Far normal range: the owner picked `[1, 2]`

Plan 0055 shipped `[2, 6]` as a provisional pick. Plan 0056's window (2026-11-04) picked `[1, 2]`
by eye, against a headless reading of about 7 times `[2, 6]`'s still jumps under FSR 2, and
named the edge dots left at `[1, 2]` the next aliasing item. Owed: a plan whose first phase
makes `[1, 2]` the default with ADR-0042's Outcome, then the edge dots, and removes the emitter
cap knob (Plan 0056's Followups). Draft it with the antialiasing plan at Plan 0059's Phase 3 verdict;
Plan 0059's reference runs at `[1, 2]` by flag meanwhile.

### The table's edges, if the owner's verdict keeps it

Parked 2026-11-05 at Plan 0057's close. All of this goes if Plan 0059's Phase 13 picks the bake, and
is owed if it picks the table: a frame over `MAX_EDITS` clears the whole table (the vein's `B`
presses, 78 to 87 carves); a cell out of view past the guard's 256-frame window stays stale; a
capped cell can show a blend word from before a neighbour's last change when it enters view; the
parity skip can age out a far cell only an off-parity cell's kernel reads; the walk's slice frames
were never read on their own; Radeon lighting reads over the 6.0 ms line walking, the blend adding
0.64 to 1.38 ms. Pick up at that verdict.

### The lamp channel's faults

Parked 2026-11-05 at Plan 0057's close. `the_lamp_furnace_reads_its_albedo_times_the_direct_mean` takes
its `D` from the shader's output, so it does not defend the albedo: `D` wants an independent reading
of the lamp's irradiance at the rays' hits. Inside probes are traced while the lamp moves
(`probe_update.wgsl`'s `probe_idle` returns on `MOVED_BIT` before its inside check): cost only, unread on
the RTX. `a_lamp_step_moves_the_probes_in_reach_only` checks one probe. Pick up with Plan 0058's Phase 4.

### Slow-pan verdict for FSR 2 (ADR-0034)

The slow pan ADR-0034 named as FSR 2's verdict ran in Plan 0056's window (2026-11-04, steps 2
and 5b), and the owner's word was "FSR 2 at 1.5 stays". Owed: ADR-0034's dated Outcome, written
with the far-normal plan above.

## Tooling and process

### Shader toolchain spike

Parked 2026-10-31. Compare naga's SPIR-V against `spirv-opt -O` on the hit shader (VGPRs,
milliseconds, both adapters), then try wgpu subgroups and `shader-f16` on the walk. Rust versus
C for the CPU side was asked and answered no (GPU about 9 of 10.6 ms, CPU about 1 ms). Run when
no lane holds the GPU.

### A held fallback still moves on screen

Parked 2026-11-05 from the ruling on `the_vein_wall_patches_are_counted`. With the table's read off at the patch pose on 299b5c8, the accumulation is bit-identical over the window (0 pixels changed), yet the filter's output reads 90.95 patches a step and the composite 51.10. The lamp is off and the gather is not added, so the filter is the only part left that moves. By reading the code (not isolated): `temporal.wgsl` keeps blending each frame's noisy sample into the moments after the emissive part holds, and `atrous.wgsl` weights its taps by that variance. `a_still_fallback_pixel_holds_still` reads the accumulation, so it does not see this. Plan 0059 keeps the fallback for no-face hits, so glass and the field look may shimmer at rest. That patch-pose run makes every hit a fallback, so it is an extreme case; a translucent surface has not been read. What it takes: a spike that reads a translucent wall's composite at rest, then a fix that holds the moments with the mean, or passes a held part around the filter as the table's part does. Plan 0059's Phase 2 reads a glass pane at rest and its Phase 3 window looks at it (owner, 2026-11-05); pick up on that answer.

### The gate tier is over its wall-time budget

Read 2026-11-05 on 7cd77ad (`docs/nfr.md` section 7): 1313.4 s, about 22 minutes, loaded, against the 12 min proposal, with the two tests then over the 30 s line already on the soak list. `flythrough::settle` is the largest binary at 338 s. Ruled 2026-11-05 on the four tests over the line in that run: `the_tone_pass_reproduces_the_composites_bytes_on_every_golden_pose` split (the every-pose test on the soak list; the primary pose alone stays in the gate at 9.2 s), `castle_gate_golden` went back under with its warm-up removed (25.7 s cold, 7.4 s warm, alone), and `smoothed_normals_cut_the_normal_error_and_move_no_hit` and `a_face_hidden_from_every_eye_is_settled_before_it_is_seen` stay (23.9 s and 24.1 s alone). Those two move only if a gate run started on a quiet machine reads them over the line. Moving all four would save at most about 95 s of serial time, so this is not how the budget gets met. Still owed: either cut the slowest `settle` and `golden` tests or re-set the proposal. Much of `settle`'s time goes with the table at Plan 0059's deletion, so re-read then. `the_lamp_toggle_opens_the_light_window` read 98.0 s on CI's last red run, on an older tree, and reads 18.0 s here, so it stays in the gate; CI's next run shows whether it is still over the 60 s line. Pick up as a dev chore when the owner next wants the gate faster or CI green. Read 2026-11-08 on CI's Linux arm, Mesa 26.2.6, the GPU tests in parallel: the same tests move 25 to 40 % between runs (`a_face_hidden_from_every_eye_is_settled_before_it_is_seen` 55.6 s on run 42470428595 and 77.6 s on 42477325818; `the_lamp_toggle_opens_the_light_window` 53.2 then 70.5; `fsr2_adds_little_flicker_on_a_still_vein` 49.8 then 62.8), with several more between 45 and 58 s, so moving tests one push at a time does not converge. Ruled 2026-11-08 (owner): CI's budget step warns per test over 60 s instead of failing the push, until the light work settles; a test that stays over moves to the soak list by hand. Still owed: the architect re-reads `docs/nfr.md` section 7's 60 s line (it says a test over the line moves to the soak list; CI no longer enforces it), at Plan 0059's deletion or when the owner next wants the gate faster.

### `vxe-render::exposure` hangs on WARP since Plan 0057 Phase 7

Read 2026-11-06 from CI run 42062512872 on 7914635. Windows' gate step ran over 2 h until the owner cancelled it. The previous run (41886647253, 9dbb933, before Phase 7) took 28 to 81 s for each of these tests on WARP. On 7914635: `auto_exposure_finds_the_key` 45 s (2 frames), `limits_compensation_and_dt` 94 s, `light_adaptation_is_fast` 120 s, `dark_adaptation_runs_in_two_stages` 412 s (602 frames), `a_flash_leaves_the_eye_glared` 4001 s. Still running at the cancel, with their elapsed times: `a_wall_split_in_half_meters_the_mean_of_its_halves` about 11 800 s, `history_survives_an_exposure_step` about 11 760 s, `no_target_overflows_at_the_glare` about 11 160 s, and `the_readout_arrives_within_four_frames_without_waiting` about 7 800 s. On local lavapipe at 7914635 all 11 take 3.8 to 7.2 s, as on CI's Ubuntu before. A guess, not isolated: a four-frame test stuck for hours is a GPU wait that never returns on WARP (`device.poll(wait_indefinitely)` or a blocking readback), and the rest pay a WARP-only cost that grows with the frame count, about 0.6 s a frame extra. The suspect is the full 201 MB table at `DEFAULT_TABLE_CELLS`, which WARP's binding limit allows while lavapipe's 128 MiB halves it. Ruled 2026-11-06 (owner): the ci profile kills a test at 600 s, so Windows fails red by name instead of hanging, and stays red until a dev chore profiles Phase 7's passes on WARP through CI diagnostic runs, after Plan 0059 Phase 3. The same run (42222147229) read it again: `flythrough::shutdown the_headless_flythrough_exits_through_every_teardown_stage_within_bound`, a 3-frame headless run at 64 x 36, was killed by its own 120 s bound with an empty stderr. Plan 0059's Phase 3 is past, so the chore is unblocked; the owner started it on 2026-11-08 in the same lane as the Mesa fix. Read 2026-11-08 in that lane: the empty stderr does not place the hang. The test discarded the run's stdout, where the adapter, `device ready`, the mirror, `near residency` and `first frame` lines go, and the fly-through logs nothing at info before its teardown, so a run stuck anywhere from the adapter request to frame 2 leaves the same empty stderr. `a_mirror_that_cannot_sync_exits_1_after_the_teardown` passed in 17.8 s in that run, so on WARP the adapter, the device and the frame's creation return. The lane's diagnostics, for one push: the test now prints the run's stdout with its stderr on a kill; the headless fly-through logs timestamped `diag:` lines through start-up (adapter with its binding limit, device, frame with its table cells and bytes, each streaming sync, init) and each frame (start, submitted, the GPU wait and its return); and a Windows CI step runs the debug fly-through on WARP six times, killed past 180 s each: as the test runs it, 35 frames (the median GPU time per pass), 35 frames with a 2^16-cell table (`VXE_DIAG_TABLE_CELLS`), with `--preload off`, with `--cutaway on` (no fill or preload), and on the fixed `--level`. Locally on lavapipe the 3-frame run reaches `init done` at 5.4 s and exits at 7.9 s (loaded, load 5.6), and a 35-frame run with the small table reads the per-pass line (light 81.4 ms of a 112.7 ms GPU frame at 64 x 36), so each variant prints what the push needs.

Read 2026-11-08 from CI run 42470428595 on 22970c8, the diagnostic step: WARP (`Microsoft Basic Render Driver`, Dx12, static DXC, driver 10.0.26100.33648), the debug fly-through at 64 x 36, each run killed past 180 s. In every run the device is ready at 0.2 to 0.5 s (6.6 s for `--level`) and the frame, its pipelines created, at 13.0 to 15.5 s.

| Run | Outcome | Frame 0 on the GPU |
|---|---|---|
| 3 frames, as the shutdown test runs it | killed at 180 s | submitted at 17.7 s, the wait never returned |
| 35 frames | killed at 180 s | submitted at 17.6 s, never returned |
| 35 frames, a 2^16-cell table | killed at 180 s | submitted at 16.8 s, never returned |
| 35 frames, `--preload off` | exited 0 at 62 s | 17.3 s (17.4 to 34.7 s); each later frame about 0.4 s |
| 35 frames, `--cutaway on` (no fill or preload) | exited 0 at 58 s | 14.1 s (17.3 to 31.4 s) |
| 35 frames, `--level` | killed at 180 s | submitted at 23.1 s, never returned |

The preload-off run's per-pass GPU medians over 30 frames: frame 57.7 ms, hit 13.5, gradient 0.28, light 5.2, fill 0.02, blend 2.1, temporal 0.37, atrous 2.5, probes 22.6, gather 0.58, composite 0.83, the FSR 2 passes 9.6 in all (accumulate 5.7, depth clip 1.5), exposure 0.08; march GPU p50 57.1 ms, p99 81.6 ms. Cutaway's frame is 27.4 ms (hit 13.3, light 0.1, probes 0). The gate step ran 429 of 730 tests before fail-fast cancelled it: the shutdown test failed at 122.5 s, and `vxe-render::exposure a_wall_split_in_half_meters_the_mean_of_its_halves` was killed at 600.2 s, while `auto_exposure_finds_the_key` passed in 65.7 s and `a_flash_leaves_the_eye_glared` in 71.5 s. The exposure tests split the same way as the runs. `BoxRig` sets `candidates: 0`, which turns the preload off (`preload_want` in marcher.rs), and those tests finish. The split wall and `ceiling_frame` (the tests stuck for hours on 7914635) keep the default candidates, so the preload is on, and they never finish. So on WARP, frame 0 never finishes whenever the preload runs, at any table size. Most fly-through GPU tests skip on Windows, since their device is Vulkan-only.

Read 2026-11-08 in the chore-ci-preload lane, with no root cause found. Frame 0's preload work is small: the args pass (one workgroup), the sweep (8 x B = 1024 workgroups of 64 lanes) and `shade_preload` with its light rows only, because the fill and probe lists stay empty until frame 1. So one huge dispatch is not what WARP stalls on. A read of `cache_fill.wgsl`, light.wgsl's `cache_claim`, `cache_find` and `cache_seen`, march.wgsl's walk helpers, naga's HLSL of the preload's entry points, and wgpu 30's DX12 indirect path (the `num_workgroups` root constants after its indirect validation) found:
- no unbounded loop: the one retry loop, `cache_claim`'s compare-exchange, exits in naga's HLSL, which sets `exchanged` to `old == 0`;
- no barrier or `workgroupUniformLoad` behind a branch that is not workgroup-uniform;
- no index out of range;
- no dispatch that is unbounded.

Mesa 26.0.8 does not place the fault either. With `shade_px` removed from `shade_preload`, frame 0 completes and frame 1 dies in `preload_sweep`. With `--preload off` it dies in `shade`. The 26.0.x compiler bug hits any large entry point of this module, not a preload construct, so it is a separate fault from WARP's, which finishes with the preload off. On lavapipe 26.2.5 (LLVM 23, loaded), pipeline creation reads `shade` 148 ms, `shade_preload` 290 ms, `preload_sweep` 58 ms and `preload_args` 16 ms, so no compile blows up there. Still unknown: which of frame 0's three preload dispatches never finishes on WARP, and why. The next step is the owner's call:
- (a) a diagnostic push that names the dispatch. The headless fly-through's frame 0 would submit the args pass, the sweep and the light pass separately, with a GPU wait and a `diag:` line after each. Or variants could skip the sweep, or run `shade` in place of `shade_preload` with the args pass and the sweep kept.
- (b) cap or split the preload's dispatches. The readings above argue against it.
- (c) a WARP-only path. It branches on the backend, so it needs an ADR.
- (d) leave Windows red on these tests in the meantime.

### llvmpipe SIGSEGV on CI's Ubuntu since Plan 0057 Phase 7

Read 2026-11-06 from the same run. Three gate tests died with SIGSEGV on lavapipe: `flythrough::castle_golden castle_gate_golden` (57.5 s, 20.8 s on the run before), `flythrough::fsr2 a_sideways_step_keeps_fsr2s_history` (20.8 s) and `a_still_camera_has_no_disocclusion` (27.7 s). The driver was the same on both runs: `mesa-vulkan-drivers 26.0.8-1ubuntu0.3`, LLVM 21.1.8, image ubuntu-26.04. Every test in that run that rendered a shaded frame crashed. The GPU tests that passed render none (`frame_bytes_under_fsr2_swap_the_histories_for_fsr2s_inventory`) or render cutaway, where the fill and preload do not run. Memory is not it: each test peaks at 0.73 to 0.92 GB and four at once at 3.7 GB locally, against the runner's 16 GB. It does not reproduce on local lavapipe (Mesa 26.2.5, LLVM 23): the ci profile at four threads passed 729 of 729. A guess: a Mesa 26.0.8 llvmpipe crash on a Phase 7 shader, not concurrency. Read 2026-11-08 from CI run 42222147229 on 7c73b37, the GPU tests one at a time: `castle_gate_golden` still died with SIGSEGV at 60 s on the same driver, so concurrency is ruled out and the driver crash stands; the ci profile's `@global` line is back. The owner chose (2026-11-08) a chore lane: a newer Mesa on the runner first, a local Mesa 26.0.x bisect of the pass if that is not available. Read 2026-11-08 in that lane (Launchpad, the day's indexes): the kisak-mesa PPA publishes `26.2.6~kisak1~r` for resolute (26.04) amd64, with its libdrm `2.4.134-3~kisak~r`, and the runner's archive has 26.0.8-1ubuntu0.3 at most; oibaf's PPA publishes no mesa at all. kisak's resolute build links Ubuntu's `libllvm21`, so it would have moved the runner to Mesa 26.2.6 on the same LLVM 21.1.8, not to the local pair (Mesa 26.2.5, LLVM 23). Ruled 2026-11-08 (owner): no kisak-mesa PPA and no other package source whose version cannot be pinned; the owner does not trust packages that can't be pinned, and the PPA indexes its latest build only. The runner keeps its stock Mesa 26.0.8. The LLVM 21 suspicion is accepted, and the pass is to be found on a local build of Mesa 26.0.8 from the release tarball against LLVM 21. Read 2026-11-08 in that lane, on local builds under `~/build/mesa-26.0.8`: the tarball `mesa-26.0.8.tar.xz` from mesa.freedesktop.org, sha256 caf1c0061a68e88dfa74967a7e780c0e85d65b6c4e334cd69095a5dc54ad78bc as published in its release notes, signature good from Eric Engestrom's key 57551DE15B968F6341C248F68D8E31AFC32428A6; LLVM 21.1.8 from Arch's extra `llvm21` and `llvm21-libs` 21.1.8-1, signatures and sync-db sha256 checked, extracted rather than installed. Lavapipe only, meson 1.9.1 with `-Dbuildtype=debugoptimized -Db_ndebug=true -Dvulkan-drivers=swrast -Dgallium-drivers=llvmpipe -Dplatforms= -Dopengl=false -Dglx=disabled -Degl=disabled -Dgbm=disabled -Dgles1=disabled -Dgles2=disabled -Dllvm=enabled -Dshared-llvm=enabled -Dvideo-codecs= -Dvalgrind=disabled -Dlibunwind=disabled -Dbuild-tests=false`, 804 steps in 53 s at `ninja -j12` on the dev box. It reproduces: `castle_gate_golden` dies with SIGSEGV at 24.6 s (adapter `Mesa 26.0.8 (LLVM 21.1.8)`), and so do `a_sideways_step_keeps_fsr2s_history` and `a_still_camera_has_no_disocclusion`. The core's crashing thread is a llvmpipe compute worker in JIT code with no symbols (`mov (%rbx),%rax` with rbx 0, a null pointer loaded from the shader's stack frame); its registers hold the table's mask 0x1fffff, the last cell's byte offset 0x5ffffd0 and the blend header's dirty word at 0x6000004, so the shader was addressing the per-face table. A local dispatch trace in lavapipe (a patch kept beside the build, not in the repo) names the dispatch: with the preload on, the last dispatch before the crash is the indirect 8 x 8 one, `shade_preload` (cache_fill.wgsl, the light pass with the preload's fill and probes); with `--preload off` it is the plain light pass `shade` (light.wgsl). Both run `shade_px`. Not Phase 7 alone: a bisect of the fly-through's 3-frame headless run on this build passes on 9dbb933 (main, CI's last green) and on 2bf4568 (plan 0057 Phase 4 part 2), and crashes on their merge a337f76 and every later point read (dc2a470, ef8cf7d, ca2b0e9, 4de5c4e, 41cda0c). The merge's only change to march.wgsl is Params' 16-byte tail (`emitter_cap`, `dither`), so the crash follows the shader's build, not a single feature, which is what a compiler bug looks like. Not LLVM 21: the same Mesa 26.0.8 tarball built against Arch's LLVM 23.1.3 crashes the same test, and Mesa 26.2.6 (sha256 b9436b3e1a8be76fbd4823dc884f90596ab7c6ac5e1867a305e66ffccfb31a92, same signer, same options) built against the same LLVM 21.1.8 passes all three tests. The accepted LLVM 21 suspicion is refuted: the crash is Mesa 26.0.x's own, fixed by 26.2. The owner's options, no shader changed: CI builds the pinned 26.2.6 tarball against the runner's LLVM 21 (55 s here at 12 jobs; the runner has 4 cores, and the build could be cached by its hash), or waits for the runner's image to ship Mesa 26.2. Ruled 2026-11-08 (owner): (a). CI's Linux arm builds lavapipe from the 26.2.6 release tarball, checked by its sha256 (no signature check, which would need an unpinned keyserver fetch), against the runner's own `llvm-21-dev`, with the options above and meson 1.9.1, ninja 1.13.0, mako 1.3.10, packaging 25.0 and pyyaml 6.0.3 from PyPI. It caches the install by the hash, the options and the runner image, points `VK_DRIVER_FILES` and `VK_ICD_FILENAMES` at it, and fails the job unless vulkaninfo reports Mesa 26.2.6. The workflow's exact steps, run in an `ubuntu:26.04` container capped at 4 CPUs, built it in 102 s and the loader reported `Mesa 26.2.6 (LLVM 21.1.8)`. The build comes out once the runner image ships Mesa 26.2 or later.

## Later apps

### Web demo as a separate app

Decided 2026-10-16: a WebGPU and wasm demo for blog interactivity is a later, for-fun item and
its own app under `apps/`. The fly-through and the native paths never change or lose performance
for it; any crate support is target-gated. Not before the engine has steady lighting, coloured
materials, a sky and a way to build interesting architecture.

### Antialiasing after the look-first light

Decided 2026-11-05 (owner, Plan 0057's second window): "antializing next", after the emissive-only
reference of the look-first Plan 0059. The owner's fast turn "still has artifacts" (step 1); the
reference runs with no upscaler, so it tells the light's part from the image's. Picks up when
the owner has judged the reference.
