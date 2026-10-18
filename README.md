# voxel-engine

[![CI](https://github.com/IgorKonovalov/voxel-engine/actions/workflows/ci.yml/badge.svg)](https://github.com/IgorKonovalov/voxel-engine/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

![A cave passage in the fly-through's bounded level, lit by the headlamp](docs/images/hero.png)

<sub>The bounded level, seed 1, ray-marched under the headlamp on an RTX 3080 Laptop GPU at
1600x900. Regenerate with `cargo run --release -p flythrough --example hero -- --at 40,661,261
--yaw 300 --pitch -8`.</sub>

A game-agnostic voxel engine in Rust. A streaming world of small voxels (10 cm at the first
consumer's scale) is ray-marched in compute shaders through [`wgpu`](https://wgpu.rs), edited at
runtime with deterministic operations that report what they changed, and queried for collision,
acoustics and navigation from the same data.

It is a **library workspace, not a framework**. There is no ECS, no scene editor and no asset
pipeline beyond voxel-model import. A game composes the crates it needs and owns its own loop
and entity model.

> **Status: pre-alpha.** Milestones M1 (fly-through with digging), M3 (collision and a
> character controller) and M5 (the simulation split and a WebAssembly logic host) have
> shipped, with parts of M2, M6, M7 and M8. Spatial audio (M4) is implemented and awaits a
> listening check. [`CHANGELOG.md`](CHANGELOG.md) says what has shipped,
> [`docs/roadmap.md`](docs/roadmap.md) what comes next, and
> [`docs/plans/README.md`](docs/plans/README.md) what is in flight.

## What works today

- **A brick-map world** of `u16` voxels in 8³ bricks and 64³ chunks, with a material table
  whose rows carry an opaque payload for the game. Bounded worlds, and unbounded ones
  streamed around a focus point on a worker pool, with an LOD-only ring of mips out past full
  detail and edited chunks kept in RAM or spilled to disk.
- **Deterministic edits.** Sphere, box, capsule and tool-along-ray shapes; carve, fill and
  replace modes; each op returns per-material counts, the touched chunks and bounds. Integer
  math throughout, pinned by cross-platform hashes.
- **Queries:** an integer raycast, point and box queries, brick occupancy masks, and a coarse
  per-brick grid with a walkable map that update incrementally from each edit's result.
- **Prefabs:** MagicaVoxel `.vox` import with a palette-to-material map, and prefab stamping
  in any of the 24 rotations, at generation time or at runtime, with the same edit result.
- **Collapse:** a support query over the coarse grid finds regions an edit left unsupported;
  deterministic ops drop them as a rigid piece or turn them into rubble.
- **Collision:** vertical-cylinder sweeps and a character controller with step-up, wall
  slide, gravity, jumping and ledge climbing, in integer math.
- **The simulation split:** a server owns the world and an op log; clients are shared or own
  a replica with desync detection; a session replays from seed plus log to the same hash.
  Game logic runs behind a trait, natively or as a sandboxed WebAssembly component with
  deterministic WASI, fuel and a memory cap, proven equal to the native build.
- **A compute ray marcher** in WGSL over a sharded GPU mirror of the world, uploaded
  incrementally from the dirty set, with a headlamp, a shadow ray, smoothed and per-voxel
  contour normals, and a stats overlay with a frame-time strip.
- **The fly-through** testbed: fly or walk, dig, fill and stamp, watch unsupported rock fall,
  save and replay sessions, and print what each edit changed, plus headless benchmarks and
  golden images.

The figures behind these, met and missed, are in [`docs/nfr.md`](docs/nfr.md).

## Where it is going

- **Streaming world.** Voxels live in a chunked brick map with LOD mips, paged around focus
  points and generated on background threads by a source the game provides.
- **Compute ray marching.** A DDA ray marcher in WGSL lights the world from few sources,
  including emissive voxels, then denoises and upscales to the baseline GPU's budget.
- **Deterministic edits.** Shape, parameters and tool in; per-material counts and the touched
  region out. The same op on the same world gives bit-identical results on Linux and Windows,
  so a game can hang gameplay on destruction and replicate edits as compact ops.
- **Spatial queries.** Raycasts, sweeps, region counts and coarse grids serve collision,
  acoustics and navigation without a second representation of the world.
- **Scripting hosts.** WASM for deterministic server-side logic (shipped), Lua for client-side
  scripts, so a game's rules can be data rather than engine code.
- **Spatial audio.** HRTF playback on the device callback, with occlusion and early reflections
  traced through the same voxels.

The budgets these claims are held to (frame time, VRAM, edit cost, baseline hardware) are in
[`docs/nfr.md`](docs/nfr.md). A figure there without a measurement is a guess and says so.

## Layout

| Path | Owns |
|------|------|
| [`crates/vxe-world`](crates/vxe-world) | Voxel storage, materials, edit ops and results, spatial queries, streaming. Headless and deterministic: no GPU, no wall clock, no unseeded RNG. |
| [`crates/vxe-render`](crates/vxe-render) | `wgpu` upload, compute ray marching, lighting, denoise, upscale. All WGSL lives here. |
| [`crates/vxe-app`](crates/vxe-app) | Thin, optional `winit` shell: window, input, surface, frame loop. |
| [`crates/vxe-sim`](crates/vxe-sim) | Cylinder sweeps and the character controller. Depends on `vxe-world` only. |
| [`crates/vxe-assets`](crates/vxe-assets) | `.vox` parsing and the palette-to-material map, producing `vxe-world` prefabs. |
| [`crates/vxe-net`](crates/vxe-net) | The byte channel, the server tick with its op and input logs, the logic trait, shared and owned clients. |
| [`crates/vxe-script-wasm`](crates/vxe-script-wasm) | The `wasmtime` host for logic components; `guests/` holds the test components. Not in the default build. |
| [`apps/flythrough`](apps/flythrough) | The engine's own testbed binary; `logic/` is its dig logic, built natively or as a component. |
| [`scripts/`](scripts) | Repository gates (doc links, comment hygiene, index rows) and the guest build script. |
| [`docs/`](docs) | [Roadmap](docs/roadmap.md), [budgets](docs/nfr.md), [ADRs](docs/adrs/README.md), [plans](docs/plans/README.md). |

Further crates (audio, Lua scripting, the network transport) are added one plan at a time; the
roadmap names them.

## Building

The toolchain is pinned in [`rust-toolchain.toml`](rust-toolchain.toml); `rustup` installs it on
first use. Linux and Windows are the supported platforms.

On Linux, `winit` and `wgpu` need the X11/Wayland headers and a Vulkan driver. On Debian or
Ubuntu:

```sh
sudo apt-get install pkg-config libxkbcommon-dev libwayland-dev mesa-vulkan-drivers
```

Then:

```sh
cargo build
cargo run --release -p flythrough
```

To run the fly-through's logic as a WebAssembly component, add the target once and build the
components first:

```sh
rustup target add wasm32-wasip2
scripts/build-guests.sh
cargo run --release -p flythrough --features wasm -- --level --logic wasm
```

### The fly-through

With no flags it streams an unbounded seeded cave around the camera. Digs, fills and the bench
go through an in-process server, so every edit you see came back as a client event.

| Input | Does |
|-------|------|
| Mouse, `W` `A` `S` `D`, `Space` / `C` | Look and fly; hold `Shift` to go faster |
| Left / right click | Dig / fill at the crosshair; the edit result prints to the terminal |
| Scroll | Change the dig radius |
| `L` | Toggle the shadow ray |
| `N` | Cycle the normal mode: face, trilinear8, gradient27, contour (contour only under `--look`) |
| `V` | Toggle procedural surface detail |
| `B` | Run the carve benchmark in place |
| `P` | Stamp a 16-voxel room with an indestructible shell at the crosshair |
| `G` | Toggle fly and walk mode (bounded level only); walking, `Space` jumps or climbs a ledge |
| `K` | Print the coarse grid and walkable map |
| `R` | Toggle what unsupported rock does after a dig: fall (default) or crumble into rubble |
| `F1` | Sample the next frames and print frame-time readings |
| `F2` | Toggle the stats overlay |
| `Esc` | Quit |

Walking, the coarse grid and collapse run on the bounded level (`--level`) only.

| Flag | Does |
|------|------|
| `--level` | The bounded level instead of the streamed cave |
| `--walk` | With `--level`: start in walk mode |
| `--seed N` | World seed (default 1) |
| `--hud` | Start with the stats overlay shown |
| `--logic native\|wasm[=PATH]` | Run the dig logic natively (default) or as a component; `wasm` needs `--features wasm` |
| `--vault` | Generate a 16-voxel room with an indestructible shell 10 m along +x from the spawn |
| `--headless` | Run the same frames on an offscreen target, with no window |
| `--frames N` | Exit after `N` frames, printing the `F1` readings over the last ones |
| `--bench N` | Send `N` `B` benchmark inputs on the first frame |
| `--autopilot` | Dig, fly 300 m out and back through the streamed cave, print streaming readings |
| `--render WxH` | Fix the marcher's resolution; the window scales it |
| `--sync-budget MiB` | Bytes uploaded to the GPU mirror per frame |
| `--look MODE` | Start in normal mode `face`, `trilinear8`, `gradient27` or `contour`, with attribute pages for contour and rock detail |
| `--view VIEW` | Write `shaded` (default), `normals`, `hitmask` or `steps` (red: walk steps, green: pool reads per ray) to the target; with `--frames`, `steps` prints the last frame's mean and p99 steps per ray |
| `--detail` | Start with procedural surface detail on |
| `--rock-detail SCALE,STRENGTH` | Rock's detail feature scale in voxels and strength in [0, 1], under `--look` (default 4,0.3) |
| `--save PATH` | With `--level`: write the session's log file to `PATH` on exit and print the world hash; refuses `P`, turns collapse off and does not combine with `--vault` |
| `--replay PATH` | With `--level`: re-simulate a saved session before the first frame and print its hash; turns collapse off and does not combine with `--vault` |

### Checks

These are the gates CI runs on every push, on both Linux and Windows:

```sh
scripts/build-guests.sh
cargo nextest run --workspace
cargo nextest run -p flythrough --features wasm
cargo test --workspace --doc
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo deny check
node scripts/check-doc-links.mjs
node scripts/check-comment-hygiene.mjs
node scripts/check-index-rows.mjs --self-test
```

GPU tests run headless on a software Vulkan adapter (Mesa's lavapipe) in CI, so every feature
has a test that needs no window. [`ci.yml`](.github/workflows/ci.yml) is the full list.

## How the project is run

Work is plan-driven. A design question becomes an ADR when there is a rejected alternative worth
remembering, then a phased plan in [`docs/plans/`](docs/plans/README.md), then code. Decisions
are recorded once and cited by number; a closed plan moves to `docs/plans/done/` and gets a
changelog entry and a version bump. [`CLAUDE.md`](CLAUDE.md) is the orientation map for anyone,
human or tool, working in the tree.

The engine is being built alongside a first consumer game, but **nothing under `crates/` names
that game or any gameplay concept**. A game-specific need arrives as a requirement and is met
with a general mechanism.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without
any additional terms or conditions.
