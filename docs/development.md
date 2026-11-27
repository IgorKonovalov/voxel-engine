# Development

How the repository is laid out, how to build and check it, and how the work is run. The
[README](../README.md) is the tour; this page is for working on the engine.

## Layout

| Path | Owns |
|------|------|
| [`crates/svet-world`](../crates/svet-world) | Voxel storage, materials, edit ops and results, spatial queries, prefabs and stamping, collapse, streaming, the op log and its wire encoding. Headless and deterministic: no GPU, no wall clock, no unseeded RNG. |
| [`crates/svet-render`](../crates/svet-render) | `wgpu`: the GPU mirror and its upload, compute ray marching, the baked light store and its live queue, the light list, the sun and sky, denoise, FSR 2, exposure, voxel-model entities, the loading screen, the text overlay, render settings and profiles. All WGSL lives here. |
| [`crates/svet-app`](../crates/svet-app) | Thin, optional `winit` shell: window, input, surface, frame loop, a splash before the world loads. `examples/minimal_window.rs` is the README's window. |
| [`crates/svet-sim`](../crates/svet-sim) | Cylinder sweeps, the character controller and the surface mover. Depends on `svet-world` only. |
| [`crates/svet-assets`](../crates/svet-assets) | `.vox` parsing and writing, palette entries as material rows and the material table file, `.vox` scenes to `svet-world` prefabs and placements; `.rig` files to `svet-entity` models and clips; voxels drawn as text (`.voxtext`). |
| [`crates/svet-net`](../crates/svet-net) | The byte channel, the server tick with its op and input logs, the `Logic` and `Host` traits, shared and owned clients. |
| [`crates/svet-script-wasm`](../crates/svet-script-wasm) | The `wasmtime` host for logic components; `guests/` holds the test components. Not in the default build. |
| [`crates/svet-audio`](../crates/svet-audio) | `cpal` output, the HRTF mixer on the device callback, occlusion and early reflections through the material table; the HRIR pack under `assets/`. Depends on `svet-world` only. |
| [`crates/svet-entity`](../crates/svet-entity) | Voxel-model entities: rigid parts as prefabs, poses in fixed point and Q14 quaternions, clips sampled by tick, the entity ray query. Headless and deterministic; depends on `svet-world` only. |
| [`apps/flythrough`](../apps/flythrough) | The engine's own testbed binary; `logic/` is its dig logic, built natively or as a component; `assets/entities/` is the entities library. |
| [`apps/tester`](../apps/tester) | `svet-tester`, the tester build's launcher: a menu that runs the fly-through per scene and packs a report. |
| [`dist/`](../dist) | What the tester archives ship beside the binaries. |
| [`scripts/`](../scripts) | Repository gates (doc links, images, comment hygiene, index rows, test tiers, test clocks, file and plan size), the changed-crates test runner, the guest build, the clip recorder, the image shoot and the tester packaging. |
| [`docs/`](.) | [Roadmap](roadmap.md), [budgets](nfr.md), [ADRs](adrs/README.md), [plans](plans/README.md), [backlog](backlog.md), [the fly-through's keys and flags](flythrough.md), [the entities](entities.md). |

Further crates (Lua scripting, the network transport) are added one plan at a time; the
roadmap names them.

## Building

The toolchain is pinned in [`rust-toolchain.toml`](../rust-toolchain.toml); `rustup` installs
it on first use. Linux and Windows are the supported platforms.

On Linux, `winit` and `wgpu` need the X11/Wayland headers and a Vulkan driver, `cpal` needs the
ALSA headers, and `gilrs` (the gamepad) needs libudev. On Debian or Ubuntu:

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
cargo run --release -p flythrough -- --source outdoor         # hills and a cave mouth under a sky
cargo run --release -p flythrough -- --level --walk --hud     # start walking, the stats overlay shown
cargo run --release -p flythrough -- --profile portable       # a named render profile
cargo run --release -p flythrough -- --help                   # every flag
```

[`docs/flythrough.md`](flythrough.md) lists every key, the gamepad and every flag;
`flythrough --keys` prints the menu's whole tree. Walking, the coarse grid and collapse run on
the bounded worlds (`--source level` and `--source showcase`) only. Every run prints a
`frames:` line at exit: p50, p99 and max of the frame's render-thread CPU, of `update + poll +
sync`, and of the march's GPU time on the frames it was timed.

The tester build's launcher, `cargo run --release -p svet-tester`, opens a menu that runs the
fly-through once per scene and packs the reports into a zip
([`dist/README-TESTER.txt`](../dist/README-TESTER.txt)).

### Examples and benches

Each crate's examples are under its `examples/` and its benches under `benches/`; run one with
`cargo run --release -p <package> --example <name>` or `cargo bench -p <package> --bench
<name>`. The storage figures come from an example: `cargo run --release -p flythrough --example
world_budget -- --chunks 20,20,8 --census` (the level) or `--stream 16,24 --census` (the
streamed residency); `--veins N --census` prints the census of a synthetic world of `N - 1`
materials, and `--surface` (with `--stream`) projects what a per-voxel attribute would take
(ADR-0011's layout; the mirror no longer holds one). The bounded run also times the coarse grid
and the per-chunk mip build. A `Full` brick is a packed mask, palette and bit planes
(ADR-0012); the readings of the level's and the streamed defaults' RAM are in
[`docs/nfr/03-memory.md`](nfr/03-memory.md). `cargo bench -p svet-world --bench brick_kernel`
times the planar edit kernel against a row write on raw ids over the same brick-edits.

## Checks

These are the gates CI runs on every push, on Linux and Windows (Windows runs the CPU tests
and DX12 pipeline creation on WARP):

```sh
scripts/build-guests.sh
SVET_REQUIRE_SOFTWARE_ADAPTER=1 cargo nextest run --workspace --profile ci   # the gate tier; ci-windows on Windows
cargo nextest run -p flythrough --features wasm --profile ci -E 'binary(component)'
cargo test --workspace --doc
cargo clippy --workspace --all-targets --features flythrough/wasm -- -D warnings
cargo clippy -p flythrough-logic --features component --target wasm32-wasip2 -- -D warnings
cargo fmt --all --check
cargo deny check
node --test "scripts/*.test.mjs"
node scripts/check-doc-links.mjs
node scripts/check-images.mjs
node scripts/check-images.mjs --self-test
node scripts/check-comment-hygiene.mjs
node scripts/check-index-rows.mjs
node scripts/check-index-rows.mjs --self-test
node scripts/check-test-tiers.mjs
node scripts/check-test-clocks.mjs
node scripts/check-test-clocks.mjs --self-test
node scripts/check-file-size.mjs
node scripts/check-plan-size.mjs
```

The guest crates under `crates/svet-script-wasm/guests/` get the same wasm32 clippy. Locally,
`cargo nextest run --profile gate` is the gate tier and `node scripts/test-changed.mjs
--profile quick` tests only the crates a change touched.

GPU tests run headless on a software Vulkan adapter (a pinned build of Mesa's lavapipe) on
CI's Linux arm, where the golden baselines are blessed, so every feature has a test that needs
no window. CI also prints readings, not gates: the gate tests' times against the line in
`docs/nfr.md` section 7, each compute pass's pipeline creation time, and the tester's system
report. [`ci.yml`](../.github/workflows/ci.yml) is the full list; the soak tier runs weekly
([`soak.yml`](../.github/workflows/soak.yml)) and a `v*` tag builds the tester archives
([`release.yml`](../.github/workflows/release.yml)).

## The README's images

Every image the README shows is a row of [`docs/images/shots.json`](images/shots.json): the
scene, the camera, the clock and the flags that make it. `node scripts/shoot-images.mjs`
renders every row headless and encodes it. `node scripts/check-images.mjs` holds each file
under its size cap, every image under `docs/images/` to a row or the manifest's `frozen` list,
and every image reference in the markdown to its alt text. After a change to the
look, refreshing the images is that command and a commit. The options, the check against the
committed files and how a row's tolerance is read are in
[the fly-through's guide](flythrough.md#shooting-the-readmes-images).

## How the project is run

Work is plan-driven. A design question becomes an ADR when there is a rejected alternative worth
remembering ([`docs/adrs/`](adrs/README.md)), then a phased plan in
[`docs/plans/`](plans/README.md), then code. Decisions are recorded once and cited by number; a
closed plan moves to `docs/plans/done/` and gets a changelog entry and a version bump. The
milestones are in the [roadmap](roadmap.md), the budgets every plan states its numbers against
in [`docs/nfr.md`](nfr.md), and parked ideas in the [backlog](backlog.md).

The engine is being built alongside a first consumer game, but **nothing under `crates/` names
that game or any gameplay concept**. A game-specific need arrives as a requirement and is met
with a general mechanism.
