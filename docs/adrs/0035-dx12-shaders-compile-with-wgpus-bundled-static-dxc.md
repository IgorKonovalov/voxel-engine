# ADR-0035: DX12 shaders compile with wgpu's bundled static DXC, never FXC

> **Status:** accepted (2026-10-22, with an Outcome)
> **Date:** 2026-10-22
> **Related plan(s):** [Plan 0046](../plans/done/0046-dx12-shaders-through-bundled-static-dxc.md)

## Context

On DX12, wgpu turns each WGSL module into HLSL through naga and hands it to a Microsoft
compiler when a pipeline is created. Which one it uses is `Dx12Compiler::Auto` by default
(wgpu-types 30.0.1, `backend.rs`): a statically linked DXC if the build carries the
`static-dxc` feature, else `dxcompiler.dll` from the `PATH`, else the system's FXC
(`d3dcompiler_47.dll`). The workspace's `wgpu` line (`Cargo.toml`, `features = ["std",
"parking_lot", "vulkan", "dx12", "wgsl"]`) carries no `static-dxc`, and no machine we run sets
`WGPU_DX12_COMPILER` or ships the DLL, so every DX12 run today compiles with FXC. FXC is the
legacy Shader Model 5 compiler; it unrolls and inlines aggressively and its compile time grows
steeply with nested loops and switches.

The cost showed up on the only DX12 machine we read: the `windows-latest` CI arm (no Vulkan
driver, so DX12 on WARP). After Plan 0028 rewrote `attrib.wgsl` (1a163a9, f0b8650, merged at
81810d3), the attribute tests of `vxe-render`'s `mirror` binary went from 15.3 s (run
39737094500 at 578c8d3) to 152.8 to 158.2 s (runs 39808788735 and 39823877459), while the Linux
arm stayed at 0.33 to 0.50 s. The headless fly-through's shutdown test, which creates every
pipeline before its three frames, now runs past its 120 s bound. The suspects are the attribute
pass's 1000-cell occupancy loop, whose `switch` inlines `solid()` with three 4-way pool-shard
switches and the `NB_MIP` default branch, and `voxel_attr`'s 27-tap nest unrolled twice; and,
on every start, Plan 0030's upscale and sharpen pipelines, which each prepend the whole of
`MARCH_WGSL` and are built even at render scale 1. **That FXC is the cause is an inference
from CI logs; FXC was not run locally.** Plan 0046 Phase 1 measures it.

This is a decision with a real other side. FXC ships with every Windows install and costs
nothing to build or link; DXC through `static-dxc` is a new transitive crate whose build script
downloads a prebuilt static library from a third party's GitHub release. Whether to pay that
cost, or to fit our shaders to FXC, or to make each consumer supply DXC, could reasonably go any
of those ways.

## Decision

The engine builds `wgpu` with its `static-dxc` feature, in the workspace dependency table, so
every consumer of `vxe-render` gets a DX12 backend that compiles with a statically linked DXC
and never falls back to FXC. The instance keeps `Dx12Compiler::Auto`, which prefers static DXC
whenever it is compiled in, so `WGPU_DX12_COMPILER=fxc` stays available to diagnose a shader
against FXC where the code reads the environment. The feature is inert off Windows: `wgpu-hal`
30.0.1 declares `mach-dxcompiler-rs` only under `cfg(all(windows, not(target_arch =
"aarch64"), target_env = "msvc"))`, so a Linux build compiles and downloads nothing new, and
`windows-aarch64` and `windows-gnu` targets (which we do not ship) keep FXC. Our shaders are not
rewritten to suit FXC: a shader's shape is chosen for the GPU and the reader, and a slow FXC
compile is not a defect to design around.

## Consequences

### Positive
- DX12 pipeline creation goes through the compiler Microsoft maintains (Shader Model 6), which
  is expected to compile the attribute and march-prefixed passes in a fraction of FXC's time.
  Expected, not measured: Plan 0046 Phase 1 reads both.
- No consumer has to ship `dxcompiler.dll` or set an environment variable, and a player's
  machine compiles the same way CI does.
- The Windows CI arm stops spending minutes per run on shader compiles, and the shutdown test's
  temporary `--look plain` workaround comes out.
- Shader authors keep writing loops and switches for clarity; FXC's unroll heuristics stop
  being a design constraint.

### Negative
- **A build-time download from a third party.** `mach-dxcompiler-rs` (crate license MIT) has a
  build script that runs `curl` against `github.com/DouglasDwyer/mach-dxcompiler/releases` and
  `tar` to unpack a prebuilt `machdxcompiler` static library, a Zig-built fork of Microsoft's
  DirectXShaderCompiler (hexops' `mach-dxcompiler`). The download is not checksummed by the
  crate, so its integrity rests on GitHub and the release staying as it is; an offline or
  vendored Windows build (`cargo --offline`, `cargo vendor`) fails at that build script; a
  deleted release breaks every fresh Windows build until we move.
- **`cargo deny` sees only the crate's MIT.** The linked binary is DXC, whose own license is the
  University of Illinois/NCSA Open Source License (LLVM's pre-2019 license), with notice
  obligations on binary redistribution. The plan records that in `deny.toml` (a clarified
  expression and an allowed `NCSA`) so the gate names it, and a consumer that ships a Windows
  build owes DXC's notice.
- **A larger Windows binary and a longer clean Windows build.** The static DXC library is
  LLVM-sized; the guess is tens of MB added to each Windows executable that links `vxe-render`,
  plus the download on every clean build. Both are guesses until Plan 0046 reads them on the
  CI arm.
- **Compiler behaviour changes under our shaders on Windows.** DXC and FXC differ in
  precision, uniformity handling and the HLSL naga emits for each Shader Model, so a Windows
  test that passed on FXC can change result. Such a change is a finding to fix in the shader or
  the test, not a reason to fall back.
- **The DXC is the release that `mach-dxcompiler-rs` names**, not one we choose, and it moves
  only when the crate (and wgpu's `^0.1.4` requirement) does. The lockfile pins it.

## Alternatives considered

### Alternative A: stay on FXC and rewrite the shaders it compiles slowly

Restructure `attrib.wgsl` so FXC does not inline `solid()` with its three 4-way pool-shard
switches into each of the 1000 occupancy cells (a non-unrolled loop, a flattened shard
lookup, the `NB_MIP` branch hoisted), stop `voxel_attr`'s 27-tap nest unrolling twice, and
create Plan 0030's upscale and sharpen pipelines lazily so a scale-1 start does not compile
`MARCH_WGSL` twice more. It adds no dependency. It lost because it makes FXC's heuristics a
standing constraint on every shader we write, the next rewrite regresses the same way with no
gate to catch it until the Windows arm slows, and it fights a compiler Microsoft no longer
develops. The lazy upscale and sharpen pipelines are a separate, compiler-independent saving,
left to a followup of Plan 0046.

### Alternative B: require consumers to supply DXC

Leave the build as it is and document that a Windows consumer ships `dxcompiler.dll` (and
`dxil.dll`) beside the executable or sets `WGPU_DX12_COMPILER=dxc`. It adds no build-time
download to the engine. It lost because `Auto` falls back to FXC silently when the DLL is
missing, so the slow path is what a player gets whenever packaging slips; every consumer
repeats the same packaging work; and CI would need the DLL staged too, so the engine pays the
cost anyway, just less visibly.

### Alternative C: a test-only workaround, and park the cost

Keep the shutdown test on `--look plain` (landing separately now), raise the Windows arm's
timeouts, and revisit later. It costs nothing today. It lost because the cost is not a test
artefact: a consumer's first start on DX12 pays the same compiles, and a parked cost with a
workaround hiding it is how a regression stays unnoticed.

## Outcome


**2026-10-22, at acceptance (Plan 0046 close, CI run 39843578678 on f3ed4a3, `windows-latest`,
DX12 on WARP).** The premise held for compile time: static DXC creates the eleven compute
pipelines in 2507.9 ms cold against FXC's 76 623.6 ms, attrib 596.0 ms against 42 412.2 ms.
Two expectations in Positive were falsified. The shutdown test's `--look plain` workaround
does not come out: with DXC the headless fly-through on the default look still ran past its
120 s bound (121.055 s), and the `mirror` attribute tests still each pass 90 s, so on WARP the
cost is the attribute pass's execution, not its compile, and the Windows arm still spends
minutes on it. The workaround is back (7c4c086), and the WARP execution cost is a followup of
Plan 0046. The binary-size guess read as 37 744 128 B for the debug `flythrough.exe`, with a
113 589 902 B static library linked into it.
