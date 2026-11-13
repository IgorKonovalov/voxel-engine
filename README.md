# voxel-engine

[![CI](https://github.com/IgorKonovalov/voxel-engine/actions/workflows/ci.yml/badge.svg)](https://github.com/IgorKonovalov/voxel-engine/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

![A cavern seen between pillars of stepped stone cubes, lit only by glowing ore: a cluster of cyan crystals in the middle distance over a red line of ember in a chasm, flecks of pale amber ore on the pillars and violet ore on a wall at the left, stalactites hanging from a red dome](docs/images/hero.png)

<sub>The showcase cavern (`--source showcase`), seed 1, its `hall` shot: from voxel
(190, 384, 218), 9 m over the west floor, at yaw 0 and pitch -12 degrees, lamp off, so all of
the light is the four ores' (amber, cyan crystal, violet amethyst and red ember). 8192 frames at
a manual exposure of 8, the emissive light read from the baked light store, FSR 2 on at
internal scale 1.5 (1280x720 for 1920x1080) with sharpening 0.5, the default `face` look
(honest cubes with the far normal at `[2, 6]`) and the default lighting, on an RTX 3080 Laptop GPU at 1920x1080,
rendered 2026-11-13 at v0.52.0.
Regenerate with
`cargo run --release -p flythrough --example hero -- --exposure 8 --frames 8192 --out docs/images/hero.png`
(every other setting above is the example's default).</sub>

A game-agnostic voxel engine in Rust. A streaming world of small voxels (10 cm at the first
consumer's scale) is ray-marched in compute shaders through [`wgpu`](https://wgpu.rs), edited at
runtime with deterministic operations that report what they changed, and queried for collision,
acoustics and navigation from the same data.

It is a **library workspace, not a framework**. There is no ECS, no scene editor and no asset
pipeline beyond voxel-model import. A game composes the crates it needs and owns its own loop
and entity model.

> **Status: pre-alpha.** Milestones M1 (fly-through with digging), M3 (collision and a
> character controller) and M5 (the simulation split and a WebAssembly logic host) have
> shipped, with parts of M2, M6, M7 and M8. Spatial audio (M4) has shipped: HRTF playback,
> occlusion, early reflections and a late reverb sized by the room. [`CHANGELOG.md`](CHANGELOG.md) says what has shipped,
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
- **The simulation split:** a server owns the world and an op log of every edit, stamp, drop
  and removal (a save carries the prefabs its stamps name); clients are shared or own a
  replica with desync detection; a session replays from seed plus log to the same hash.
  Game logic runs behind a trait, natively or as a sandboxed WebAssembly component with
  deterministic WASI, fuel and a memory cap, proven equal to the native build.
- **A compute ray marcher** in WGSL over a sharded GPU mirror of the world, reserved once
  and uploaded incrementally from the dirty set, walking a hierarchy of occupancy bits, with
  a headlamp, a shadow ray, honest cubes (a smooth field surface is an
  opt-in look, ADR-0041), a translucent material class, and a stats overlay with a frame-time strip.
- **Spatial audio:** a headless HRTF mixer on the device callback, with occlusion,
  transmission and early reflections probed through the same voxels and the material
  table's acoustic triple, and a late reverb sized by the room the listener sees.
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
- **Spatial audio.** HRTF playback with occlusion, early reflections and a late reverb sized
  by the room the listener sees (shipped).

The budgets these claims are held to (frame time, VRAM, edit cost, baseline hardware) are in
[`docs/nfr.md`](docs/nfr.md). A figure there without a measurement is a guess and says so.

## Layout

| Path | Owns |
|------|------|
| [`crates/vxe-world`](crates/vxe-world) | Voxel storage, materials, edit ops and results, spatial queries, streaming. Headless and deterministic: no GPU, no wall clock, no unseeded RNG. |
| [`crates/vxe-render`](crates/vxe-render) | `wgpu` upload, compute ray marching, lighting, denoise, upscale. All WGSL lives here. |
| [`crates/vxe-app`](crates/vxe-app) | Thin, optional `winit` shell: window, input, surface, frame loop. |
| [`crates/vxe-sim`](crates/vxe-sim) | Cylinder sweeps and the character controller. Depends on `vxe-world` only. |
| [`crates/vxe-assets`](crates/vxe-assets) | `.vox` parsing and writing and the palette-to-material map, producing `vxe-world` prefabs; `.rig` files to `vxe-entity` models and clips; voxels drawn as text (`.voxtext`). |
| [`crates/vxe-net`](crates/vxe-net) | The byte channel, the server tick with its op and input logs, the logic trait, shared and owned clients. |
| [`crates/vxe-script-wasm`](crates/vxe-script-wasm) | The `wasmtime` host for logic components; `guests/` holds the test components. Not in the default build. |
| [`crates/vxe-audio`](crates/vxe-audio) | `cpal` output, the HRTF mixer on the device callback, occlusion and early reflections through the material table; the HRIR pack under `assets/`. Depends on `vxe-world` only. |
| [`crates/vxe-entity`](crates/vxe-entity) | Voxel-model entities: rigid parts as prefabs, poses in fixed point, the entity ray query. Headless and deterministic; depends on `vxe-world` only. |
| [`apps/flythrough`](apps/flythrough) | The engine's own testbed binary; `logic/` is its dig logic, built natively or as a component. |
| [`apps/tester`](apps/tester) | `vxe-tester`, the tester build's launcher: a menu that runs the fly-through per scene and packs a report. |
| [`dist/`](dist) | What the tester archives ship beside the binaries. |
| [`scripts/`](scripts) | Repository gates (doc links, comment hygiene, index rows, test tiers, file and plan size), the changed-crates test runner, the guest build and the tester packaging. |
| [`docs/`](docs) | [Roadmap](docs/roadmap.md), [budgets](docs/nfr.md), [ADRs](docs/adrs/README.md), [plans](docs/plans/README.md), [the fly-through's keys and flags](docs/flythrough.md). |

Further crates (Lua scripting, the network transport) are added one plan at a time; the
roadmap names them.

## Building

The toolchain is pinned in [`rust-toolchain.toml`](rust-toolchain.toml); `rustup` installs it on
first use. Linux and Windows are the supported platforms.

On Linux, `winit` and `wgpu` need the X11/Wayland headers and a Vulkan driver, and `cpal`
needs the ALSA headers, and `gilrs` (the gamepad) needs libudev. On Debian or Ubuntu:

```sh
sudo apt-get install pkg-config libxkbcommon-dev libwayland-dev mesa-vulkan-drivers libasound2-dev libudev-dev
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

```sh
cargo run --release -p flythrough -- --level                  # the bounded level: walk, collapse, coarse grid
cargo run --release -p flythrough -- --source showcase        # a 72 m hall lit by four ores
cargo run --release -p flythrough -- --level --walk --hud     # start walking, the stats overlay shown
cargo run --release -p flythrough -- --help                   # every flag
```

| Input | Does |
|-------|------|
| Mouse, `W` `A` `S` `D`, `Space` / `C` | Look and fly; hold `Shift` to go faster |
| Left / right click | Dig / fill at the crosshair; the edit result prints to the terminal |
| Scroll | Change the dig radius |
| `G` | Toggle fly and walk mode (bounded worlds only); walking, `Space` jumps or climbs a ledge |
| `F2` | Toggle the stats overlay |
| `F7` | Toggle the headlamp |
| `F10` | Cycle the upscaler: off, own, fsr2 |
| `N` | Cycle the look presets |
| `Esc` | Quit |

[`docs/flythrough.md`](docs/flythrough.md) lists every key (lighting, light queue, views,
audio, gamepad) and every flag. Every toggle shows its mode in a banner and prints it to
stdout. Walking, the coarse grid and collapse run on the bounded worlds (`--source level` and
`--source showcase`) only. Every run prints a `frames:` line at exit: p50, p99 and max of the
frame's render-thread CPU, of `update + poll + sync`, and of the march's GPU time on the
frames it was timed.

The tester build's launcher, `cargo run --release -p vxe-tester`, opens a menu that runs the
fly-through once per scene and packs the reports into a zip
([`dist/README-TESTER.txt`](dist/README-TESTER.txt)).

The storage figures come from an example: `cargo run --release -p flythrough --example
world_budget -- --chunks 20,20,8 --census` (the level) or `--stream 16,24 --census` (the streamed
residency); `--veins N --census` prints the census of a synthetic world of `N - 1` materials,
and `--surface` (with `--stream`) projects what a per-voxel attribute would take (ADR-0011's layout; the mirror no longer holds one).
The bounded run also times the coarse grid and the per-chunk mip build. A `Full` brick is a
packed mask, palette and bit planes (ADR-0012), so the level's bricks take 58 MiB of RAM and
the streamed defaults' about 400 MiB ([`docs/nfr.md`](docs/nfr.md) section 3).
`cargo bench -p vxe-world --bench brick_kernel` times the planar edit kernel against a row
write on raw ids over the same brick-edits.

### Checks

These are the gates CI runs on every push, on Linux and Windows (Windows runs the CPU tests
only):

```sh
scripts/build-guests.sh
cargo nextest run --workspace --profile ci        # the gate tier; ci-windows on Windows
cargo nextest run -p flythrough --features wasm --profile ci -E 'binary(component)'
cargo test --workspace --doc
cargo clippy --workspace --all-targets --features flythrough/wasm -- -D warnings
cargo clippy -p flythrough-logic --features component --target wasm32-wasip2 -- -D warnings
cargo fmt --all --check
cargo deny check
node --test ".claude/hooks/*.test.mjs"
node --test "scripts/*.test.mjs"
node scripts/check-doc-links.mjs
node scripts/check-comment-hygiene.mjs
node scripts/check-index-rows.mjs
node scripts/check-index-rows.mjs --self-test
node scripts/check-test-tiers.mjs
node scripts/check-file-size.mjs
node scripts/check-plan-size.mjs
```

The guest crates under `crates/vxe-script-wasm/guests/` get the same wasm32 clippy. Locally,
`cargo nextest run --profile gate` is the gate tier and `node scripts/test-changed.mjs
--profile quick` tests only the crates a change touched.

GPU tests run headless on a software Vulkan adapter (Mesa's lavapipe) on CI's Linux arm, so every feature
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
