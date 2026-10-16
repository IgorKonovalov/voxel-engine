# voxel-engine

[![CI](https://github.com/IgorKonovalov/voxel-engine/actions/workflows/ci.yml/badge.svg)](https://github.com/IgorKonovalov/voxel-engine/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

A game-agnostic voxel engine in Rust. A streaming world of small voxels (10 cm at the first
consumer's scale) is ray-marched in compute shaders through [`wgpu`](https://wgpu.rs), edited at
runtime with deterministic operations that report what they changed, and queried for collision,
acoustics and navigation from the same data.

It is a **library workspace, not a framework**. There is no ECS, no scene editor and no asset
pipeline beyond voxel-model import. A game composes the crates it needs and owns its own loop
and entity model.

> **Status: pre-alpha.** The repository holds the workspace skeleton, the architecture decisions,
> the roadmap and the budgets. No engine code has landed yet. See
> [`docs/roadmap.md`](docs/roadmap.md) for what comes first and
> [`CHANGELOG.md`](CHANGELOG.md) for what has shipped.

## What it will do

- **Streaming world.** Voxels live in a chunked brick map with LOD mips, paged around focus
  points and generated on background threads by a source the game provides.
- **Compute ray marching.** A DDA ray marcher in WGSL lights the world from few sources,
  including emissive voxels, then denoises and upscales to the baseline GPU's budget.
- **Deterministic edits.** Shape, parameters and tool in; per-material counts and the touched
  region out. The same op on the same world gives bit-identical results on Linux and Windows,
  so a game can hang gameplay on destruction and replicate edits as compact ops.
- **Spatial queries.** Raycasts, sweeps, region counts and coarse grids serve collision,
  acoustics and navigation without a second representation of the world.
- **Scripting hosts.** WASM for deterministic server-side logic, Lua for client-side scripts,
  so a game's rules can be data rather than engine code.

The budgets these claims are held to (frame time, VRAM, edit cost, baseline hardware) are in
[`docs/nfr.md`](docs/nfr.md). A figure there without a measurement is a guess and says so.

## Layout

| Path | Owns |
|------|------|
| [`crates/vxe-world`](crates/vxe-world) | Voxel storage, materials, edit ops and results, spatial queries, streaming. Headless and deterministic: no GPU, no wall clock, no unseeded RNG. |
| [`crates/vxe-render`](crates/vxe-render) | `wgpu` upload, compute ray marching, lighting, denoise, upscale. All WGSL lives here. |
| [`crates/vxe-app`](crates/vxe-app) | Thin, optional `winit` shell: window, input, surface, frame loop. |
| [`apps/flythrough`](apps/flythrough) | The engine's own testbed binary: fly, dig, read edit results. |
| [`docs/`](docs) | [Roadmap](docs/roadmap.md), [budgets](docs/nfr.md), [ADRs](docs/adrs/README.md), [plans](docs/plans/README.md). |

Further crates (simulation, audio, scripting, networking, assets) are added one plan at a time;
the roadmap names them.

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
cargo run -p flythrough
```

### Checks

These are the gates CI runs on every push, on both Linux and Windows:

```sh
cargo nextest run --workspace
cargo test --workspace --doc
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo deny check
```

GPU tests run headless on a software Vulkan adapter (Mesa's lavapipe) in CI, so every feature
has a test that needs no window.

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
