# 0001: Bounded brick map, deterministic edits, and a headlamp ray marcher

> **Status:** in-progress
> **Created:** 2026-10-16
> **Milestone:** M1 in `docs/roadmap.md` (first of two plans; Plan 0002 adds streaming and LOD)
> **Related ADRs:** [ADR-0002](../adrs/0002-brick-map-layout-and-per-chunk-locked-access.md) (storage layout, GPU mirror, access model), ADR-0001 (stack)

## TL;DR

We build the walking skeleton of the engine: a bounded brick-map world in `vxe-world` with a
material table, the full set of edit operations in integer math returning per-material counts,
CPU raycast and region queries, a seeded integer-noise cave, and a `wgpu` compute ray marcher
in `vxe-render` that draws that cave lit by a headlamp with one shadow ray. `vxe-app` gets a
`winit` shell and `apps/flythrough` lets you fly through the cave, dig, and read the counts on
the console. The first thing a reader sees is a headless test: a radius-10-voxel sphere carved
out of solid material 3 returns `{3: 4169}`. The first thing the user sees is a golden image of
the cave on lavapipe, then the fly-through on the dev box. No streaming, no eviction, no LOD:
the world is a finite box of chunks generated up front, and ADR-0002 fixes the layout so that
Plan 0002 adds paging without changing it.

## Context & problem

Nothing in `crates/` exists beyond doc comments. The consumer's P0 list
(`project-context.md`, "The first consumer") needs, in this order of dependency: sparse storage
of a 128 × 128 × 48 m level at 10 cm with a 16-bit material id; carve ops (sphere, box,
capsule, tool-along-ray) whose per-material counts are exact and identical on every machine;
an indestructible flag; raycast, point and region queries; generation as a pure function of
the seed; and a first-person image stable at arm's length. The roadmap fixes the storage
proposal and leaves the access model to this plan's ADR.

`nfr.md` figures touched: §3 world RAM and VRAM (bricks only; mips are Plan 0002), §4 edit cost
and upload volume, §2 primary-ray time (a dev-box reading, labelled as such), §6 determinism
across the two CI arms, §7 time to first frame.

## Decision

We implement ADR-0002 as written: `u16` voxels, 8³ bricks that are `Uniform` or `Full`, 64³
chunks in a map of `Arc<RwLock<Chunk>>`, edits applied in place under per-chunk write locks in
coordinate order after a residency check, a GPU mirror of brick pool, brick masks, chunk
records and a wrapped chunk table, and a marcher that walks chunk, brick, 2 × 2 × 2 block and
voxel levels. The renderer ships primary visibility plus one spot light with one shadow ray;
real lighting is M2. The world is bounded in this plan: a `WorldSource` is evaluated for every
chunk of a stated extent before the first frame, in parallel over chunks with output
independent of thread count.

We rejected a world-first plan without a renderer because the owner wants pixels before the
mechanism debates (smoothing, contours) start, and a render-first plan over dense arrays
because the uniform-brick representation is exactly what the renderer must mirror. We rejected
splitting this into a world plan and a render plan because one close review should see the
store and its mirror together.

## Architecture diagram

```mermaid
flowchart LR
    subgraph consumer["apps/flythrough"]
        SRC[CaveSource<br/>seed -> chunk]
        IN[input: fly, dig]
        OUT[stdout: edit results, stats]
    end
    subgraph world["vxe-world (headless)"]
        ST[(ChunkStore<br/>ChunkCoord -> Arc&lt;RwLock&lt;Chunk&gt;&gt;)]
        MT[MaterialTable]
        ED[EditOp -> EditResult]
        Q[raycast / material_at / count_in_box]
        DT[dirty bricks]
    end
    subgraph render["vxe-render (wgpu)"]
        MIR[GpuWorld mirror:<br/>brick pool, masks,<br/>chunk records, chunk table]
        SH[march.wgsl<br/>chunk > brick > block > voxel<br/>+ shadow ray]
        TGT[rgba8 target]
    end
    subgraph app["vxe-app (winit)"]
        WIN[window, surface,<br/>frame loop]
    end
    SRC -->|generate| ST
    IN -->|ops| ED --> ST
    ED -->|counts| OUT
    ST --> Q
    ST --> DT -->|per-brick upload| MIR
    MT -->|material buffer| MIR
    MIR --> SH --> TGT --> WIN
```

## Implementation phases

Each phase ships as its own commit. `dev` implements all phases in one session with no review
between phases. The architect reviews once at the end, in a fresh session. Phase 1 is the
walking skeleton.

Shared conventions for every phase:

- Coordinates: `VoxelCoord` and `ChunkCoord` are `[i32; 3]` newtypes; voxel `v` lies in chunk
  `v.div_euclid(64)` and brick `v.rem_euclid(64).div_euclid(8)`. Fixed-point positions are
  `[i64; 3]` in units of 1/256 voxel; the centre of voxel `i` is `256 * i + 128`. Shape tests
  compare squared distances in `i64` (sphere, box) or `i128` (capsule, which multiplies two
  squared lengths). No `f32` or `f64` appears in any edit predicate, result or hash.
- A test world is built from a `WorldSource`; tests use `UniformSource(material)` and a
  closure source evaluated per voxel. A "solid material-3 world" means `UniformSource(3)` over
  chunks `[-2, 2)³` unless stated otherwise. The material table in tests has rows 0 (air),
  1..=5 (plain) and 4 flagged indestructible.
- Every new dependency is pinned exactly in `[workspace.dependencies]` at the then-latest
  compatible release; the ones expected are listed under "Data shapes".
- Hashes in tests are FNV-1a 64 implemented in `vxe-world`'s test support (no crate), fed in
  ascending chunk coordinate order, bricks in slot order, voxels in x-fastest order; a
  `Uniform` brick hashes as its 512 expanded values so representation changes never move the
  hash.

### Phase 1: Brick map and the first carve
- **Owner skill:** dev
- **What:** `vxe-world` stores a bounded world of `u16` voxels in uniform-or-full bricks inside
  locked chunks, owns a material table, and carves a sphere returning per-material counts and
  the touched chunk set.
- **Files touched:** `Cargo.toml` (workspace deps), `crates/vxe-world/Cargo.toml`,
  `crates/vxe-world/src/{lib.rs, coords.rs, brick.rs, chunk.rs, store.rs, material.rs,
  source.rs, edit.rs, fixed.rs}`, `crates/vxe-world/tests/edits.rs`,
  `crates/vxe-world/tests/support/mod.rs`.
- **Done when:**
  - `World::generate(extent, &source, &table)` over chunks `[-2, 2)³` from `UniformSource(3)`
    holds 64 chunks and 32 768 brick slots, every one `Uniform(3)`, and `World::stats()` reports
    `full_bricks == 0`.
  - `world.apply(&EditOp::sphere(centre_of_voxel([0,0,0]), 10 * 256, Mode::Carve))` returns
    `Ok(EditResult)` with `changed_from == [(3, 4169)]`, `touched` equal to the eight chunks
    with coordinates in `{-1, 0}³` in ascending order, and `aabb == ([-10,-10,-10], [10,10,10])`.
  - After that carve, `material_at([0,0,10]) == Some(0)`, `material_at([5,5,7]) == Some(0)`,
    `material_at([0,0,11]) == Some(3)`, `material_at([6,6,6]) == Some(3)`, and `stats()`
    reports exactly 32 `Full` bricks (the bricks the sphere enters; the 32 other bricks of its
    bounding box contain no carved voxel and stay `Uniform`).
  - Carving the same sphere again returns `changed_from == []` and `touched == []`.
  - A brick whose voxels all become air collapses: carving a box covering exactly brick
    `[0,8)³` leaves `stats().full_bricks` unchanged from before the box was applied.
  - `vxe-world` has no dependency that links a GPU API; `cargo clippy -p vxe-world -- -D
    warnings` passes with the wall-clock ban in `clippy.toml` in force.

### Phase 2: All shapes, all modes, indestructible, atomic failure
- **Owner skill:** dev
- **What:** box and capsule shapes; `Carve`, `Fill(material)` and `Replace { from, to }` modes;
  the indestructible flag honoured by `Carve` and ignored by `Replace`; an op touching a
  non-resident chunk fails before changing anything.
- **Files touched:** `crates/vxe-world/src/{edit.rs, shape.rs, material.rs}`,
  `crates/vxe-world/tests/edits.rs`.
- **Done when:**
  - In the solid material-3 world, `EditOp::box_(min = [0,0,0], max = [1024,1280,1536],
    Mode::Carve)` (fixed-point, half-open) returns `changed_from == [(3, 120)]` (4 × 5 × 6
    voxel centres fall inside).
  - `EditOp::capsule(a = centre_of_voxel([0,0,0]), b = centre_of_voxel([10,0,0]), r = 3 * 256,
    Mode::Carve)` returns `changed_from == [(3, 413)]`.
  - In a world where `x >= 0` is material 4 (indestructible) and `x < 0` is material 3, the
    radius-10 sphere carve at the origin returns `changed_from == [(3, 1926)]` and
    `material_at([0,0,0]) == Some(4)` afterwards.
  - In the same world, `Mode::Replace { from: 4, to: 5 }` with that sphere returns
    `changed_from == [(4, 2243)]` (4169 - 1926) and `material_at([0,0,0]) == Some(5)`: replace
    ignores the flag.
  - After the radius-10 carve in the solid material-3 world, `Mode::Fill(2)` with a box of
    voxels `[-1,1]³` (fixed `[-256,-256,-256]..[512,512,512]`) returns `changed_from == [(0, 27)]`;
    filling it again returns `[]`; `Fill` never overwrites a non-air voxel.
  - A radius-10 sphere centred on voxel `[127,0,0]` of the `[-2,2)³` world spans voxels
    `x ∈ [117,137]`, `y, z ∈ [-10,10]` and so reaches four non-resident chunks: `[2,-1,-1]`,
    `[2,-1,0]`, `[2,0,-1]` and `[2,0,0]`. It returns `Err(EditError::NotResident([2,-1,-1]))`,
    the first of them in ascending coordinate order (ADR-0002), and the world hash is
    identical before and after.
  - A fixed list of ops mixing every shape and mode produces the same `changed_from` vector
    whether applied to a world generated with one thread or sixteen (generation parallelism is
    invisible).

### Phase 3: Raycast, queries, tool-along-ray
- **Owner skill:** dev
- **What:** an integer DDA raycast over bricks (uniform bricks skipped whole), point and region
  material queries, and the tool-along-ray op built on the raycast.
- **Files touched:** `crates/vxe-world/src/{raycast.rs, query.rs, edit.rs}`,
  `crates/vxe-world/tests/{queries.rs, edits.rs}`.
- **Done when:**
  - After the radius-10 carve in the solid material-3 world, `count_in_box([-10,-10,-10],
    [10,10,10])` (inclusive voxel coordinates) returns `[(0, 4169), (3, 5092)]` in ascending
    material order.
  - `count_in_box` over a box that exactly covers chunk `[1,1,1]` of the uncarved world returns
    `[(3, 262144)]` and, by an instrumented counter in the test support, reads zero voxel
    arrays (uniform bricks contribute 512 without a read).
  - `raycast(origin = centre_of_voxel([0,0,0]), dir = [0,0,256], max_t = 100 * 256)` in the
    carved world returns `Some(Hit { voxel: [0,0,11], face: NegZ, t: 2688, material: 3 })`
    (from z = 0.5 to the face at z = 11.0 is 10.5 voxels); `t` is exact fixed-point, computed
    by cross-multiplication, never by a float division.
  - The same ray with `max_t = 2687` returns `None`; a ray starting inside voxel `[0,0,11]`
    returns `Some(Hit { voxel: [0,0,11], t: 0, face: None, .. })` (`face` is an `Option`).
  - A ray along `[256, 256, 256]` from the origin centre of the carved world hits the voxel the
    brute-force reference walk in the test (stepping the ray in 1/1024-voxel increments and
    reporting the first solid voxel) also reports, for ten seeded directions.
  - `EditOp::tool_along_ray(origin = centre_of_voxel([0,0,0]), dir = [0,0,256], max_t = 100 *
    256, r = 2 * 256, Mode::Carve)` in the carved world returns `changed_from == [(3, 26)]`: the
    capsule from the origin to the hit face point `(128, 128, 2816)` with radius 2 voxels
    reaches 26 solid voxels at z = 10, 11 and 12 outside the earlier sphere.
  - With no hit within `max_t`, the capsule ends at `origin + dir * max_t / 256`.
  - The raycast is a pure function: a test asserts byte-identical hit lists over 1000 seeded
    rays against a blessed FNV-1a hash (the first cross-arm assertion; the full vector is
    Phase 4).

### Phase 4: Cave source, determinism vector, world budget readings
- **Owner skill:** dev
- **What:** a seeded integer-noise cave `WorldSource` in `apps/flythrough` (the engine ships
  no terrain), the CI determinism vector asserted on both arms, a `criterion` bench for the edit
  budget, and a release example that prints the RAM and mirror sizes at two scene sizes.
  `flythrough` gains a library target (`src/lib.rs`, exporting `pub mod cave`) next to its
  binary, so its own tests and examples can use `CaveSource`. `vxe-world` does not depend on
  `flythrough`, not even as a dev-dependency: that would be a cycle, and it would pull `winit`
  and `wgpu` into `vxe-world`'s test build. Everything that needs the cave therefore lives
  under `apps/flythrough/`.
- **Files touched:** `apps/flythrough/src/{lib.rs, cave.rs}`, `apps/flythrough/Cargo.toml`,
  `apps/flythrough/tests/determinism.rs`, `apps/flythrough/examples/world_budget.rs`,
  `crates/vxe-world/benches/edits.rs`, `crates/vxe-world/Cargo.toml` (the `criterion`
  dev-dependency and the bench target), `crates/vxe-world/src/hash.rs` (FNV-1a and the
  integer hash and value noise helpers the cave builds on, because they are reusable and
  deterministic), `docs/nfr.md`.
- **Done when:**
  - `CaveSource { seed, extent }` produces chunks from integer value noise with a 64-bit mix
    hash, trilinear interpolation in fixed point, and a threshold; materials are 1 rock, 2 ore
    (veins from a second noise), 3 shell on the outermost 2 voxels of the extent flagged
    indestructible, 0 air; a spherical air pocket of radius 3 m exists at the stated spawn
    point. No `f32`, `f64`, `sin`, `exp` or `powf` appears in `apps/flythrough/src/cave.rs`
    or `crates/vxe-world/src/hash.rs` (a test greps both).
  - Same seed, same chunk: identical bytes; seeds 1 and 2 differ in at least one chunk.
  - A brick whose eight corner densities all sit beyond a margin from the threshold is emitted
    `Uniform` without per-voxel evaluation; the spawn pocket and the shell are applied after.
  - `apps/flythrough/tests/determinism.rs` generates the cave with seed `0x5EED_0001` over chunks `[0,4)³`,
    applies a fixed list of 24 ops (every shape and mode, one of them failing on a
    non-resident chunk), hashes the voxels and every result (and the error) with FNV-1a 64,
    and asserts the hash equals a literal constant blessed on Linux. CI runs it on both arms
    through `cargo nextest run --workspace`; the constant is not loosened to make Windows pass.
  - `cargo bench -p vxe-world --bench edits` measures 100 radius-10 carves at distinct seeded
    positions in a fresh solid world per iteration (every carve expands uniform bricks, the
    worst case) and the log records the reading on the dev box against `nfr.md` §4 (2 ms).
  - `cargo run --release -p flythrough --example world_budget -- --chunks 20x20x8 --seed 1`
    and `-- --chunks 32x32x32 --seed 1` print: chunk count, `Full` and `Uniform` brick counts
    and the `Full` fraction, CPU bytes by accounting (`Full` bricks × 1024 + chunks × slot
    array), mirror bytes by the same formula the uploader uses (`Full` bricks × (1024 + 8) +
    chunks × (512 × 4) + table), generation wall time and voxels per second per thread. The
    example carries the `#[allow(clippy::disallowed_methods)]` for its one clock read. The
    log records both readings; `nfr.md` §3 and §5 get the fraction and the throughput as
    dev-box readings.

### Phase 5: GPU mirror and primary-ray marcher with a golden image
- **Owner skill:** dev
- **What:** `vxe-render` mirrors the store into the ADR-0002 buffers, uploads dirty bricks,
  and marches primary rays in a WGSL compute shader into an `rgba8` target, shaded as albedo
  times a spot-light term without shadows. A headless golden test runs on lavapipe. It lives
  in `apps/flythrough/tests/`, because the scene is the cave and `vxe-render` cannot depend on
  `flythrough` (the Phase 4 reasoning). Mirror and upload tests that need no cave stay in
  `crates/vxe-render/tests/`.
- **Files touched:** `crates/vxe-render/Cargo.toml`, `crates/vxe-render/src/{lib.rs,
  gpu_world.rs, camera.rs, marcher.rs, readback.rs}`,
  `crates/vxe-render/src/shaders/march.wgsl`, `crates/vxe-render/tests/mirror.rs`,
  `apps/flythrough/Cargo.toml` (`pollster` and `png` dev-dependencies),
  `apps/flythrough/tests/golden.rs`, `apps/flythrough/tests/golden/primary.png`,
  `apps/flythrough/tests/support/mod.rs`.
- **Done when:**
  - `GpuWorld::new(device, extent)` sizes the chunk table to the extent, grows the brick pool
    by whole pages (initially from `stats().full_bricks`), and `sync(&world, &queue)` uploads
    every dirty brick and clears the dirty bits; after a second `sync` with no edits, zero
    bytes are written (a counter that `crates/vxe-render/tests/mirror.rs` reads, on a
    closure-source world).
  - A brick that collapses to `Uniform` frees its pool slot and the chunk record entry becomes
    `uniform | material`; the free list reuses the slot on the next expansion.
  - The shader walks chunk table, chunk record, then voxels of a `Full` brick with a
    three-level DDA (the block level is Phase 8); a uniform-air entry advances the ray by a
    whole brick; a uniform-solid entry hits at the brick entry face. Workgroup size 8 × 8; the
    dispatch is `ceil(width / 8) × ceil(height / 8)` and the shader bounds-checks the pixel.
  - The device is requested with `max_storage_buffer_binding_size` and `max_buffer_size` set to
    the adapter's values (clamped to what the pool needs); the golden test prints the
    adapter's name and both limits, and the log records them for lavapipe and for the dev box.
  - `apps/flythrough/tests/golden.rs` requests a software adapter (`force_fallback_adapter`),
    builds the cave
    with seed 7 over chunks `[0,6)³` with the spawn at its centre, applies one radius-10 carve
    centred where the camera's +x view ray from the spawn voxel's centre first enters rock
    (5.7 m ahead at seed 7: the spawn pocket opens into a natural cavern, so a carve at a fixed
    1.5 m or 3 m lands in air), renders 320 × 180 from the fixed spawn pose, reads the target
    back, and compares to `apps/flythrough/tests/golden/primary.png`: at most 0.1 % of pixels
    differ by more
    than 2/255 in any channel (exact match is expected on the same Mesa; the tolerance absorbs a
    runner upgrade). If no adapter is found the test prints `skipped: no software adapter` and
    passes, so the Windows arm stays green; `VXE_BLESS=1` rewrites the golden.
  - The golden shows the carve: the test also renders the same scene without the carve and
    asserts that at least 2 % of pixels differ from the golden by more than 8/255 (the carve
    is visible), and that the golden holds at least two colour bins each covering 5 % of the
    pixels or more (a uniform image fails; the indestructible shell bounds the extent, so no
    ray misses). A colour bin
    is the top 3 bits of each of r, g and b (512 bins), because continuous spot shading
    rarely repeats an exact colour.

### Phase 6: Headlamp with a shadow ray
- **Owner skill:** dev
- **What:** a spot light carried with the camera but offset from the eye (right 0.25 m, down
  0.15 m, so shadows are visible), cone falloff, inverse-square attenuation, and one shadow ray
  per pixel from the hit point offset half a voxel along the face normal toward the lamp.
- **Files touched:** `crates/vxe-render/src/shaders/march.wgsl`, `crates/vxe-render/src/{lib.rs,
  marcher.rs, light.rs}`, `apps/flythrough/tests/golden.rs`,
  `apps/flythrough/tests/golden/headlamp.png`, `apps/flythrough/tests/support/mod.rs` (if the
  occluder scene's world or camera lands there).
- **Done when:**
  - `apps/flythrough/tests/golden/headlamp.png` is blessed on lavapipe for the Phase 5 scene
    with the lamp on (`LightParams::headlamp()`) and compared under the same floor.
  - On the Phase 5 scene, the same render with the shadow ray disabled (a `LightParams` flag
    that exists for this test and the fly-through's debug toggle) has no pixel darker than the
    shadowed image in any channel: shadows only ever remove light. The test prints the
    fraction of pixels brighter by more than 8/255 and the log records it (0.31 % measured
    before this amendment). There is no floor on that fraction. The walls are 4 to 6 m away,
    so the 0.29 m lamp offset gives too little parallax for a floor to mean anything, and
    `headlamp.png` already pins the cave's shadows.
  - **Shadows exist, on a scene where the count can be computed.** An occluder scene in
    `golden.rs`: `UniformSource` air over chunks `[0,2)³` under `cave_materials()`, with rock
    (material 1) filled into a wall
    at voxels x `[44,46)`, y and z `[0,128)`, and into an occluder slab at x `[28,30)`, y and
    z `[56,68)`. The camera sits at `(8.5, 64.5, 64.5)` looking along +x (yaw 0, pitch 0,
    vfov 70°), at 320 × 180, with `LightParams::headlamp()`, so the lamp sits at
    `(8.5, 62.0, 63.0)`. Rendered with shadows on and off, the scene has no pixel darker
    unshadowed. **Between 820 and 840 pixels** (830 computed) are brighter by more than
    8/255. Each of those pixels lies in columns `[130,207]` and rows `[63,140]` (row 0 at
    the top): an L-shaped band along the occluder's left and top edges, the side away from
    the lamp. Every other pixel is identical between the two renders.
    - *Arithmetic (done in this amendment with an f64 model of `march.wgsl`: pixel-centre
      rays, the half-voxel shadow-ray offset, the cone smoothstep, inverse square, and
      gamma 1/2.2).* Eye and lamp both lie inside the slab's y and z ranges, so the slab's
      near face at x = 28 (19.5 voxels from the eye) casts the shadow on the wall face at
      x = 44 (35.5 voxels away; the shadow ray starts at 35.0). The visible band is the lamp
      offset times (35.0 / 19.5 − 1) = 0.795: 1.99 voxels wide on the left and 1.19 on the
      top. At 0.276 voxels per pixel on the wall, that is 7 columns (130 to 136) and 4 rows
      (63 to 66) of pixel centres. Every shadow edge and image edge lies 0.42 px or more from
      a pixel centre, so moving the eye or the lamp by 0.06 voxels flips no pixel. The dimmest
      shadowed pixel reads 129/255 (rock's blue channel) unshadowed, far above the 8/255
      threshold. The ±10 px
      tolerance absorbs f32 DDA error at the slab's corners. If the count lands outside it,
      the shadow ray is wrong. Stop and report; don't widen the range.
  - `golden/primary.png` is unchanged by this phase. It renders with `LightParams::at_eye()`
    (no offset, no shadow ray) and compares bit-identical.

### Phase 7: Window shell and the fly-through
- **Owner skill:** dev
- **What:** `vxe-app` wraps `winit` 0.30's `ApplicationHandler` with a surface and a frame
  loop that hands input and a frame to a user trait; `apps/flythrough` generates the
  level-size cave, flies, digs, and prints edit results and stats.
- **Files touched:** `crates/vxe-app/Cargo.toml`, `crates/vxe-app/src/{lib.rs, input.rs,
  shell.rs}`, `apps/flythrough/src/{main.rs, cave.rs, fly.rs}`, `apps/flythrough/Cargo.toml`,
  `crates/vxe-render/src/present.rs` (blit of the `rgba8` target to the surface).
- **Done when:**
  - `cargo run --release -p flythrough -- --seed 1` opens a window on the dev box, prints
    `first frame after <n> ms` with n under 3000 (`nfr.md` §7), and shows the cave from the
    spawn pose. Generation of the 20 × 20 × 8 chunk extent runs in parallel over chunks with
    `std::thread::scope`.
  - WASD and mouse look (cursor locked) fly the camera; left click applies
    `tool_along_ray` with radius 0.5 m along the view ray; right click fills a sphere of
    radius 0.3 m with material 1 at the hit point; scroll changes the radius; `L` toggles the
    shadow ray; `B` applies 100 radius-10 carves at seeded positions within 10 m and prints
    the CPU time of the edits and the bytes `sync` uploaded; `F1` prints frame time (CPU, and
    GPU through timestamp queries when `Features::TIMESTAMP_QUERY` is available), `Full`
    brick count and mirror bytes; `Esc` quits.
  - Every edit prints one line in the form `carved: rock 1234, ore 12 (3 chunks, 0.21 ms)`
    using the material table's names.
  - The only wall-clock reads outside `vxe-app` are in `apps/flythrough`, each behind an
    `#[allow(clippy::disallowed_methods, reason = ...)]`; `vxe-world` and `vxe-render` have
    none.
  - CI builds `flythrough` on both arms (no windowed test runs).

### Phase 8: Brick occupancy masks
- **Owner skill:** dev
- **What:** the 64-bit occupancy mask per `Full` brick in the mirror and a block-level step
  in the marcher, with the primary-ray timing read before and after.
- **Files touched:** `crates/vxe-world/src/brick.rs` (mask builder), `crates/vxe-render/src/{gpu_world.rs,
  marcher.rs, light.rs}`, `crates/vxe-render/src/shaders/march.wgsl`,
  `crates/vxe-world/tests/bricks.rs`, `apps/flythrough/tests/golden.rs`,
  `apps/flythrough/tests/golden/{primary.png, headlamp.png}`, `docs/nfr.md`.
- **Done when:**
  - `Brick::occupancy_mask()` of a brick with the single solid voxel `[3,3,3]` is `1 << 21`
    (block `(1,1,1)`, bit `bx + 4 by + 16 bz`); of an all-air brick, 0; of an all-solid brick,
    `u64::MAX`; and the mask of any brick equals the brute-force mask computed in the test over
    64 seeded random bricks.
  - The marcher walks the 4 × 4 × 4 mask before touching voxels: an unset block advances the
    ray by a 2 × 2 × 2 block in one step.
  - **A masks-off switch reproduces the Phase 7 walk exactly.** A `masks` flag in the march
    params (alongside `LightParams::shadows`, existing for this test and the fly-through's
    debug use) makes the shader step voxels without reading `brick_masks`. Before re-blessing,
    the masks-off renders of the Phase 5 scene under `LightParams::at_eye()` and
    `LightParams::headlamp()` match the committed Phase 7 `primary.png` and `headlamp.png`
    byte for byte, and the log records that check.
  - **The mask moves only isolated corner pixels.** A test in `golden.rs` renders the carved
    cave with masks on and off, under `at_eye()` and under `headlamp()`, and for each pair:
    - prints the count of pixels differing by more than 2/255 in any channel, and the log
      records both counts (19 and 23 measured before this amendment, on llvmpipe, Mesa 26.2.2);
    - asserts that count is at most 57 (the existing 0.1 % floor of 57 600 pixels);
    - asserts **no two differing pixels are 8-neighbours**. A mask bug (a wrong bit layout,
      a skip past an occupied block) opens holes the size of a block, and a 2 × 2 × 2 block
      at the cave's 4 to 6 m wall distance covers several pixels at 320 × 180, so it shows as
      a cluster. A tie broken differently at a grazing corner moves one pixel. If the
      differing pixels cluster, the mask is wrong: stop and report, don't loosen the check.
  - `golden/primary.png` and `golden/headlamp.png` are re-blessed on lavapipe with masks on
    and compared under the existing floor from then on. The occluder scene still reads
    between 820 and 840 shadowed pixels, inside the computed band.
  - *Why not bit-exact (amended after the Phase 8 block).* Skipping a block reaches the next
    boundary by a different `f32` path than stepping its two voxels, so at a near-tie corner
    the two walks can pick adjacent voxels. That is the disagreement Risks already accepts
    between the GPU and CPU walks, now between two GPU walks. Making block and voxel steps
    traversal-invariant was considered and not taken: it is open-ended and need not reproduce
    the Phase 7 goldens either. The per-pixel cause is a hypothesis, not checked against the
    CPU raycast; the isolation assertion is what separates it from a mask defect.
  - The log records the primary-pass GPU time at 1920 × 1080 from the fly-through's spawn
    pose on the dev box before and after this phase (timestamp queries), and `nfr.md` §2 gets
    the reading labelled dev reference.

## Data shapes

```rust
// illustrative: vxe-world
pub struct VoxelCoord(pub [i32; 3]);
pub struct ChunkCoord(pub [i32; 3]);
pub struct Fixed3(pub [i64; 3]);           // 1/256 voxel
pub type MaterialId = u16;                 // 0 is air

pub enum Brick { Uniform(MaterialId), Full(Box<[MaterialId; 512]>) }
pub struct Chunk { bricks: [Brick; 512], dirty: [u64; 8] }
pub struct ChunkStore { chunks: RwLock<HashMap<ChunkCoord, Arc<RwLock<Chunk>>>> }

pub struct Material {
    pub name: String, pub albedo: [f32; 3], pub roughness: f32, pub emission: [f32; 3],
    pub hardness: f32, pub flags: MaterialFlags /* INDESTRUCTIBLE */,
    pub acoustic: [f32; 3] /* absorption, transmission, resonance */, pub payload: u64,
}

pub enum Shape { Sphere { c: Fixed3, r: i64 }, Box_ { min: Fixed3, max: Fixed3 },
                 Capsule { a: Fixed3, b: Fixed3, r: i64 } }
pub enum Mode { Carve, Fill(MaterialId), Replace { from: MaterialId, to: MaterialId } }
pub struct EditOp { pub shape: Shape, pub mode: Mode }   // tool_along_ray resolves to Capsule
pub struct EditResult {
    pub changed_from: Vec<(MaterialId, u32)>,  // ascending material; voxels that changed, by old id
    pub touched: Vec<ChunkCoord>,              // ascending
    pub aabb: (VoxelCoord, VoxelCoord),        // inclusive bounds of changed voxels
}
pub enum EditError { NotResident(ChunkCoord) }
pub trait WorldSource { fn chunk(&self, coord: ChunkCoord, out: &mut Chunk); }
```

```text
GPU mirror (ADR-0002), all storage buffers, little-endian u32 words:
  brick_pool:    [slot][256 x u32]   two u16 voxels per word, x fastest
  brick_masks:   [slot][2 x u32]     bit b = block (b & 3, (b >> 2) & 3, b >> 4) has a non-air voxel
  chunk_records: [chunk][512 x u32]  bit 31 set: pool slot in bits 0..31; clear: material in bits 0..16
  chunk_table:   [wrapped coord]u32  record index, or 0xFFFF_FFFF when absent
  materials:     [id][8 x f32]       albedo rgb, roughness, emission rgb, flags as f32 bits
  params:        uniform             camera, extent, window, light, flags
```

Expected new dependencies (pinned by `dev` at the then-latest compatible release): `wgpu`,
`winit`, `glam`, `bytemuck`, `pollster` (blocking adapter and buffer-map awaits), `log` and
`env_logger` (the fly-through only), and as dev-dependencies `png` (golden images) and
`criterion` (the edit bench). `vxe-world` takes none of them.

## Budgets

| `nfr.md` figure | Expected effect | How the close measures it |
|---|---|---|
| §3 world RAM and VRAM | First readings of the `Full` brick fraction at level size and at the 100 m cube; the mips line stays a proposal until Plan 0002 | `world_budget` example, both sizes, release, dev box; plus the fly-through's `F1` mirror bytes at level size |
| §4 edit cost (100 carves of radius 1 m in 2 ms) | Read, not changed | `cargo bench -p vxe-world --bench edits`, dev box; the fly-through's `B` key for the in-frame figure with upload bytes |
| §2 primary rays (3.0 ms at 1080p, baseline) | First dev-box reading, before and after masks | Timestamp queries from the fly-through's spawn pose at 1920 × 1080, dev box, labelled dev reference |
| §5 generation throughput | First reading (bounded, synchronous) | voxels per second per thread from `world_budget` |
| §6 determinism | Asserted | `tests/determinism.rs` on both CI arms, every push |
| §7 time to first frame (3 s) | Read | the fly-through's startup line, dev box |

The baseline GPU is not on hand; every GPU figure in this plan is a dev-reference reading and
says so.

## Risks & open questions

- **Storage buffer binding limits.** wgpu's default `max_storage_buffer_binding_size` is
  128 MiB and `max_buffer_size` 256 MiB; the level-size scene can need up to 1.6 GB if every
  brick were `Full`. Phase 5 requests the adapter's limits and records them for lavapipe and
  the dev box. If the measured `Full` fraction times 1 KiB exceeds what an adapter binds,
  Plan 0002 shards the pool across bindings; this plan does not.
- **lavapipe speed.** A software adapter marching 57 600 pixels through a 6³-chunk cave should
  take well under a second; if a golden test exceeds 30 s on CI, shrink the scene, not the
  resolution floor.
- **GPU and CPU DDA disagreement.** The shader walks in `f32`; the CPU raycast is integer. At a
  grazing voxel corner they may pick adjacent voxels. The dig lands where the CPU says; a
  one-voxel visual mismatch at corners is accepted and noted, not hidden. The Phase 8 block
  skip makes the same kind of choice differently from the voxel walk; Phase 8 bounds it to
  isolated pixels.
- **Blessing across arms.** The determinism constant is blessed on Linux. If the Windows arm
  disagrees, the cause is a non-integer path and is a defect to fix; the test is not loosened.
- **Generation cost against the first-frame budget.** 840 M voxels of noise per start is too
  slow if every voxel is evaluated; the uniform-brick early-out on corner densities is the
  mechanism, and the throughput reading tells whether it suffices.
- **Pointer lock on Wayland.** `winit` 0.30 supports `CursorGrabMode::Locked` on Wayland; if
  Hyprland refuses it, fall back to `Confined` plus relative motion and note it.
- **Threading.** No lock is held across generation; the uploader takes one chunk read lock at a
  time; ops lock chunks in ascending order. A test asserts generation output is independent of
  thread count; nothing else in this plan runs concurrently with edits.
- **Readings from Phases 4 and 5 that miss their figures** (edit bench against §4, the `Full`
  fraction against §3's assumption, the 32³ mirror against the §3 world line, the per-binding
  limits on lavapipe and the dev box; numbers in the Implementation log's Close triggers). None
  blocks Phases 6 to 8. The golden scenes bind well under lavapipe's 128 MiB, and the
  fly-through's level-size pool fits the dev box's 2 GiB binding. The close review weighs
  each one: fix it in Plan 0002 (pool sharding, palette narrowing), take an optimisation
  followup, or revise the figure in `nfr.md` with the reading.
- **Open:** whether `Fill` should report the material it wrote as well as the air it replaced.
  The result keys by old material only; the consumer's P0 needs removed-per-material, which
  that gives. Revisit when a consumer asks.

## What this plan does NOT do

- **Streaming, eviction, edit persistence for paged-out chunks, LOD mips, LOD rays:** Plan 0002.
- **The far-field clipmap** beyond the resident set: its own plan after M1.
- **Per-chunk palette narrowing of the mirror and sharding the brick pool:** taken in Plan 0002
  or a follow-up if this plan's VRAM readings demand it (ADR-0002).
- **Real lighting, emissive extraction, denoising, upscaling, the smooth-surface spike:** M2.
- **Sweeps, character controller, coarse grids, change notifications:** M3.
- **A save format or an op log on disk:** M5 (seed plus op log).
- **A baseline-GPU reading:** the card is not on hand (`nfr.md` §1); M2 close.
- **Debug views and a profiler overlay:** M7. The fly-through prints to stdout.

## Implementation log

> Written by `dev`: one row per phase as its commit lands, then the close block. **The phases
> above are the contract. This section records what happened.** Observations, never conclusions:
> no pass list, no self-assessment. Deviations and unmet done-whens are **always** disclosed.
> Keep it shorter than `## Implementation phases`.

| phase | owner | state | commit |
|---|---|---|---|
| 1: Brick map and the first carve | dev | done | b78443f |
| 2: All shapes, all modes, indestructible, atomic failure | dev | done | 4f2cd02 |
| 3: Raycast, queries, tool-along-ray | dev | done | 64d93d8 |
| 4: Cave source, determinism vector, world budget readings | dev | done | 62bb87a |
| 5: GPU mirror and primary-ray marcher with a golden image | dev | done | 6a3ab55 |
| 6: Headlamp with a shadow ray | dev | done | 4da3c9a |
| 7: Window shell and the fly-through | dev | done | 84b4f55 |
| 8: Brick occupancy masks | dev | done | 3561541 |

### Notes

_(Deviations from the plan, with the commit, stated without justification. Done-whens not
satisfiable as stated, and what was done instead. Followups noticed and not acted on. One line
each. Empty is fine.)_

- Phase 1: the `Box_` shape landed in `edit.rs` in Phase 1, because the collapse done-when carves a box (commit of Phase 1).
- Phase 1: `EditResult::aabb` is `Option<(VoxelCoord, VoxelCoord)>`, `None` when nothing changed (commit of Phase 1).
- Phase 1: the collapse done-when ("`full_bricks` unchanged") cannot hold after the sphere, where brick `[0,8)³` is `Full`. Per the owner, the test runs it on a fresh solid world (0 -> 0, brick is `Uniform(0)`) and also asserts 32 -> 31 after the sphere, with the box carving 32 voxels (commit of Phase 1).
- Phase 2: the done-when expects `NotResident([2,0,0])` for the sphere at `[127,0,0]`, but the shape also reaches non-resident `[2,-1,-1]`, `[2,-1,0]` and `[2,0,-1]`, and `[2,-1,-1]` comes first in ascending order. Routed to `/architect` by the owner before Phase 2 starts; resolved by the plan amendment in 6a9a240.
- Phase 4 (owner-approved): `CaveSource` is unreachable from `vxe-world` tests, so `flythrough` gets a lib target and `determinism.rs` and `world_budget.rs` move under `apps/flythrough/`; applied to the plan in 6a9a240.
- Phase 2: `crates/vxe-world/src/lib.rs` (the `shape` module and re-exports) and `crates/vxe-world/tests/support/mod.rs` (the FNV-1a world hash) changed outside the phase's `Files touched`, with the owner's approval (commit of Phase 2).
- Phase 2: `Shape` moved from `edit.rs` to `shape.rs` and is re-exported from the crate root as before (commit of Phase 2).
- Phase 2: `NotResident` is raised only for a missing chunk that holds a voxel centre inside the shape, decided brick by brick and then per voxel; a missing chunk inside the shape's bounding box but holding no inside centre does not fail the op (commit of Phase 2).
- Phase 2: the capsule's `Outside` test for a box is conservative (bounding box, then the box's bounding sphere against the segment), so surface bricks fall back to the per-voxel test; counts are unaffected (commit of Phase 2).
- Phase 2: `Mode::target` returns `None` when the new material equals the old, so `Fill(0)` and `Replace { from: m, to: m }` change and report nothing (commit of Phase 2).
- Phase 3: `crates/vxe-world/src/lib.rs` (modules, re-exports) and `crates/vxe-world/tests/support/mod.rs` (`SplitMix64`, `carved_world`) changed outside the phase's `Files touched` (commit of Phase 3).
- Phase 3: the voxel-array read counter is crate API, `World::count_in_box_with_stats` returning `QueryStats { voxel_arrays_read }`, not a test-support counter; test support cannot see inside the crate (commit of Phase 3).
- Phase 3: `count_in_box` counts are `u64`; voxels in non-resident chunks are not counted (commit of Phase 3).
- Phase 3: `EditOp::tool_along_ray` takes `&World` as its first argument and returns the resolved capsule op; the caller applies it (commit of Phase 3).
- Phase 3: the raycast skips a non-resident chunk as one empty 64³ cell; `Hit::t` is the exact entry parameter rounded down (commit of Phase 3).
- Phase 3: the ten brute-force directions are `[256,256,256]` plus nine from SplitMix64 seed `0x1ca57` (commit of Phase 3).
- Phase 4: `crates/vxe-world/src/lib.rs` (`pub mod hash`) and the root `Cargo.toml` (`criterion` in `[workspace.dependencies]`) changed outside the phase's `Files touched` (commit of Phase 4).
- Phase 4: the cave's material table is `flythrough::cave_materials()` in `apps/flythrough/src/lib.rs`, not in `cave.rs`, because its albedo rows are floating point (commit of Phase 4).
- Phase 4: `hash.rs` also carries `World::voxel_hash` and `ValueNoise::{cell_of, corners, interpolate}`; the cave evaluates a non-uniform brick from one set of lattice corners per noise, bit-identical to `sample` (commit of Phase 4).
- Phase 4: cave parameters: caverns on a 64-voxel lattice, detail on a 16-voxel lattice at 1/8 weight, air above density 6000, ore on a 16-voxel lattice above 28000, early-out margin 4; spawn is the extent's centre voxel (commit of Phase 4).
- Phase 4: the float grep also rejects digit-dot-digit literals (commit of Phase 4).
- Phase 4: `criterion` 0.8.2 is taken with default features off (`cargo_bench_support` only) (commit of Phase 4).
- Phase 4: the edit bench reads 4.92 ms against the §4 budget of 2 ms (see Close triggers).
- Phase 4: the measured `Full` fraction (23.41 % at level size, 20.79 % at 32³ chunks) is above the 10 % `nfr.md` §3 assumption; the 32³ mirror (3496.2 MiB) exceeds the 3 GB §3 world-VRAM line.
- Phase 4: the determinism constant `0x2b6b_6a0d_1f9b_8d75` is blessed on Linux; the Windows arm has not run it yet (nothing pushed).
- Phase 5: the golden carve is centred where the camera's +x view ray first enters rock (5.7 m ahead at seed 7), not 1.5 m ahead: at seed 7 the spawn pocket opens into a natural cavern, so carves at 1.5 m and at 3 m both lie in air. Owner's decision (commit of Phase 5).
- Phase 5: the root `Cargo.toml` changed outside `Files touched`: `bytemuck`, `glam`, `png`, `pollster`, `wgpu` pins, and `vxe-world` as a workspace dependency with `version = "=0.1.0"`, because `cargo deny` treats a versionless path dependency of a publishable crate as a wildcard (commit of Phase 5).
- Phase 5: `wgpu` is taken with default features off: `std`, `parking_lot`, `vulkan`, `dx12`, `wgsl` (commit of Phase 5).
- Phase 5: the golden test's adapter is the Vulkan backend's fallback adapter and must report `DeviceType::Cpu`; otherwise it prints `skipped: no software adapter`. `crates/vxe-render/tests/mirror.rs` takes any adapter, fallback first (commit of Phase 5).
- Phase 5: `vxe-render` also exports `limits_for(adapter, pool_bytes)`, `MARCH_WGSL` (validated through `naga` in `mirror.rs`), and re-exports `wgpu` so `flythrough` tests take no direct `wgpu` dependency (commit of Phase 5).
- Phase 5: camera units are voxels, z up; the lamp sits at the eye with intensity 1200, cone 0.35 to 0.70 rad, and the output is clamped then raised to 1/2.2 (commit of Phase 5).
- Phase 5: `GpuWorld::sync` takes `&mut self` and returns `Result<SyncStats, MirrorError>`; pool pages are 4096 slots (4 MiB) (commit of Phase 5).
- Phase 5: lavapipe binds at most 134 217 728 B per storage buffer, below the level-size pool (383 608 Full bricks, about 375 MiB); the RTX 3080 binds 2 147 483 644 B, below the 32x32x32 pool (3 487 326 Full bricks, about 3.3 GiB).
- Phase 6: `LightParams` carries intensity and cone angles as well as the offsets and the `shadows` flag; `LightParams::at_eye()` keeps the Phase 5 values (intensity 1200, cone 0.35 to 0.70 rad). `Marcher::render` and `Marcher::params` take `&LightParams` (commit of Phase 6).
- Phase 6: the occluder scene reads 823 pixels brighter by more than 8/255 (range 820 to 840, 830 computed), all inside columns [130,207] and rows [63,140]; the cave scene reads 0.31 %. Both on lavapipe (llvmpipe, Mesa 26.2.2) (commit of Phase 6).
- Phase 6: `primary.png` and `headlamp.png` re-blessed under `VXE_BLESS=1` came out byte-identical to the committed files (commit of Phase 6).
- Phase 7 (owner-approved): `crates/vxe-render/src/marcher.rs` changed outside `Files touched`: `Marcher::encode` takes optional `ComputePassTimestampWrites`, `Marcher::resize`, and `MarchTimer` (two timestamp queries around the march pass) (commit of Phase 7).
- Phase 7: also outside `Files touched`: the root `Cargo.toml` (`winit` =0.30.13 with `rwh_06`, `x11`, `wayland`, `wayland-dlopen` and no client-side decorations; `log` =0.4.34; `env_logger` =0.11.11 with default features off), `crates/vxe-render/src/lib.rs` (`present` module, re-exports of `Presenter`, `MarchTimer` and `glam`), and `crates/vxe-render/src/shaders/present.wgsl` with a `naga` validation unit test in `present.rs` (commit of Phase 7).
- Phase 7: `log` is a dependency of `vxe-app` (cursor-grab fallback warning, adapter line), not of the fly-through only (commit of Phase 7).
- Phase 7: the world generates on a spawned thread while the shell creates the window and device, and is joined in `App::device_request`; Vulkan instance creation took 1.64 s on the dev box with the dGPU asleep (commit of Phase 7).
- Phase 7: `first frame after <n> ms` does not hold under 3000 on every cold start: 2973, 3151 and 3156 ms when the dGPU had been idle (one run started after `runtime_status` read `suspended`), 1916 to 1920 ms warm (see Close triggers). The initial mirror upload (383 608 one-brick `write_buffer` calls, 853 to 888 ms) is the largest item after device creation; batching it is not done here.
- Phase 7: scroll changes the dig radius (0.1 m steps, 0.1 to 3 m); the fill radius stays 0.3 m. `B` prints one aggregated `carved:` line for its 100 carves plus one `bench:` line with edit CPU time and sync bytes, not 100 lines. Movement is `W`/`A`/`S`/`D`, `Space` up, `C` down, `Shift` x4 (commit of Phase 7).
- Phase 7: the fly-through takes `--render WxH` (fixed marcher resolution, scaled to the window) and `--frames N` (exit after N frames, printing the `F1` reading over the last 30); `F1` prints medians over 30 frames (commit of Phase 7).
- Phase 7: the windowed run was launched on the dev box (Hyprland, Wayland) only with `--frames`; WASD, mouse look, clicks, `L`, `B`, `F1` and `Esc` were not driven by input in this session. `Fly` and `Input` have headless unit tests; the edit-line format has one (commit of Phase 7).
- Phase 8: blocked on the bit-exact golden done-when (owner's call); resolved by the plan amendment in 8074a34.
- Phase 8: the `masks` switch is a `LightParams` field (default `true` in `at_eye()` and `headlamp()`), sent as `FLAG_MASKS` (bit 1 of `Params::flags`). The fly-through has no key for it: `apps/flythrough/src` is outside Phase 8's `Files touched` (commit of Phase 8).
- Phase 8: before re-blessing, masks-off renders of the carved cave under `at_eye()` and `headlamp()` matched the committed Phase 7 `primary.png` and `headlamp.png` in every byte (0 pixels differing), on llvmpipe (LLVM 22.1.8, Mesa 26.2.2). That was a one-time test, removed before the re-bless (commit of Phase 8).
- Phase 8: `masks_move_only_isolated_pixels` reads 17 pixels (`at_eye()`) and 15 (`headlamp()`) differing by more than 2/255 between masks on and off, none touching another, on the same adapter. The block notes' 19 and 23 do not match their own 0.0295 % and 0.0260 %, which are 17 and 15 of 57 600 (commit of Phase 8).
- Phase 8: with the mask bit index transposed in `march.wgsl` (`k.z + 4 k.y + 16 k.x`), the same test read 6604 differing pixels under `at_eye()` and failed on the count. The shader was restored before the commit (commit of Phase 8).
- Phase 8: `primary.png` and `headlamp.png` were re-blessed with masks on; the occluder scene still reads 823 and the cave shadow fraction 0.31 % (commit of Phase 8).
- Phase 8: `sync` writes each `Full` brick's mask as its own 8-byte `write_buffer`, doubling the per-brick write calls. The initial level-size upload went from 853 to 888 ms (Phase 7) to 1889 to 1976 ms, and `first frame after` read 4231, 2972, 2869 and 2983 ms, over the §7 budget of 3000 on one run (see Close triggers).
- Phase 8: `crates/vxe-world/tests/bricks.rs` uses `is_multiple_of` to satisfy `clippy::manual_is_multiple_of` (commit of Phase 8).
- Review fix, finding 1 (time to first frame): `GpuWorld::sync` stages `Full` bricks and their masks in CPU buffers kept in `GpuWorld` and writes each run of consecutive pool slots, capped at one 4096-slot page, with one `write_buffer` for the voxels and one for the masks; `SyncStats.bytes` is unchanged (402 449 984 B). `mirror.rs` reads back one slot's mask after the initial sync. The goldens read 0.0000 % and a `VXE_BLESS=1` run left both PNGs byte-identical (commit of finding 1).

### Close triggers

_(Facts for the architect. No recommendations, and no suggested version bump.)_

- **What shipped:** the bounded brick-map world with deterministic integer edits (sphere, box, capsule; carve, fill, replace) that report per-material counts, touched chunks and bounds; raycast, box queries and tool-along-ray; the seeded cave source and a pinned determinism hash; the GPU mirror with incremental upload and occupancy masks; the compute marcher with an offset headlamp and one shadow ray; the `winit` shell and the fly-through with dig, fill, readings and a timed `--frames` mode. Phases 1 to 8, commits b78443f to 3561541.
- **Public API changed:** new crates' first public surface. `vxe-world`: `World`, `ChunkStore`, `WorldStats`, `Brick` (with `occupancy_mask`), `Chunk`, coordinate types, `Fixed3`/`FIXED_ONE`/`FIXED_HALF`/`centre_of_voxel`, `EditOp`/`EditResult`/`EditError`/`Mode`/`Shape`, `Material`/`MaterialFlags`/`MaterialId`/`MaterialTable`/`AIR`, `QueryStats`, `Hit`/`Face`, `WorldSource`/`UniformSource`, `hash`. `vxe-render`: `Camera`, `GpuWorld`/`MirrorError`/`SyncStats`, `LightParams` (with `shadows` and `masks`), `Marcher`/`MarchTimer`, `Presenter`, `readback`, `limits_for`, `MARCH_WGSL`, re-exported `wgpu` and `glam`. `vxe-app`: `App`, `Flow`, `Gpu`, `ShellConfig`, `ShellError`, `run`, `Input`, `KeyCode`, `MouseButton`, re-exported `wgpu` and `winit`. `flythrough` lib: `cave`, `cave_materials`.
- **Budget readings:**
  - §4 edit cost: `cargo bench -p vxe-world --bench edits`, 100 radius-10 carves on a fresh solid world: 4.9199 ms (CI 4.8698 to 4.9768 ms). AMD Ryzen 9 5900HS, release bench profile, Phase 4 commit.
  - §3 world memory: `world_budget --chunks 20x20x8 --seed 1`: 3200 chunks, 383 608 Full / 1 254 792 Uniform (23.41 %), CPU 419 028 992 B, mirror 402 437 184 B. `--chunks 32x32x32 --seed 1`: 32 768 chunks, 3 487 326 Full / 13 289 890 Uniform (20.79 %), CPU 3 839 457 280 B, mirror 3 666 029 424 B. AMD Ryzen 9 5900HS, release, Phase 4 commit.
  - §5 generation: 0.509 s at 20x20x8 (1.029e8 voxels/s/thread), 4.785 s at 32x32x32 (1.122e8 voxels/s/thread), 16 threads, same machine and commit.
  - §7 time to first frame: `cargo run --release -p flythrough -- --seed 1` (20x20x8 chunks, 1280x720 requested, NVIDIA GeForce RTX 3080 Laptop GPU, Vulkan, Hyprland): 2973, 3151 and 3156 ms after dGPU idle; 1916 and 1920 ms warm. Breakdown of one idle run: generation 574 ms (in parallel with device setup), device ready at 1887 ms, mirror upload 888 ms (399 381 120 B). Phase 7 commit.
  - §3 mirror at level size from the fly-through: 399 381 120 B after the initial sync, 383 608 Full bricks, same machine and commit. With masks (Phase 8 commit 3561541): 402 449 984 B.
  - §2 primary pass (march with headlamp and shadow ray): `flythrough --seed 1 --render 1920x1080 --frames 300`, median of the last 30 frames, spawn pose, NVIDIA GeForce RTX 3080 Laptop GPU, Vulkan, driver 610.57.04. Without masks (84b4f55): 1.992, 2.005, 1.992 ms. With masks (3561541): 1.575, 1.586, 1.587 ms, and 1.586 ms on a fourth run. Recorded in `nfr.md` §2 as dev reference.
  - §7 time to first frame with masks (3561541, same command and machine as the §2 runs): 4231, 2972, 2869 and 2983 ms; mirror upload 1952, 1976, 1889 and 1946 ms for 402 449 984 B.
  - §7 time to first frame after the finding 1 fix (staged runs; commit of finding 1, built with `cargo build --release -p flythrough`, run as `target/release/flythrough --seed 1 --frames 60`; NVIDIA GeForce RTX 3080 Laptop GPU, Vulkan, Hyprland, AMD Ryzen 9 5900HS). Cold (dGPU PCI `runtime_status` read `suspended` just before each launch): 2226, 2301 and 2230 ms, with device ready at 1838, 1903 and 1843 ms and generation 478, 558 and 464 ms. Warm (launched back to back, dGPU `active`): 1063, 1150 and 1187 ms, device ready at 671, 759 and 784 ms. Mirror upload on all six runs: 349 to 357 ms for 402 449 984 B (383 608 bricks, 3200 chunks).
- **Adapter limits (Phase 5, from `wgpu` 30.0.1 `Adapter::limits`):** llvmpipe (LLVM 22.1.8, Mesa 26.2.2, Vulkan): `max_storage_buffer_binding_size` 134 217 728, `max_buffer_size` 2 147 483 647. NVIDIA GeForce RTX 3080 Laptop GPU (Vulkan): 2 147 483 644 and 4 294 967 295. AMD Radeon Graphics (RADV RENOIR): 2 147 483 644 and 2 147 483 647.
- **Gate at the tip (3561541, Linux, dev box):** `cargo build`, `cargo nextest run --workspace` (51 passed, 0 skipped), `cargo test --doc`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check` and `cargo deny check` all exit 0; `node scripts/check-doc-links.mjs` (28 links resolve); `node --test .claude/hooks/*.test.mjs` (34 passed). The Windows CI arm has not run (nothing pushed).
- **Outstanding `human` phases:** none.

## Followups

- Per-chunk 8-bit palette in the mirror (ADR-0002, "named follow-up"), after the VRAM reading.
- A `Fill` result that also reports the written material, if a consumer needs it.
