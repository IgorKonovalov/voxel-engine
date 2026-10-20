# Changelog

One entry per closed plan, written at the architect's close ceremony together with the version
bump in `Cargo.toml` (`workspace.package.version`). Format: [Keep a Changelog](https://keepachangelog.com/),
SemVer 0.x (any minor may break the API until 1.0).

## [Unreleased]

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
