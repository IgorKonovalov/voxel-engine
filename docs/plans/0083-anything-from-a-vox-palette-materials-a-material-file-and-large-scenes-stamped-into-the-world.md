# 0083: Anything from a `.vox`: palette materials, a material file, and large scenes stamped into the world

> **Status:** in-progress (approved 2026-11-16, the owner's go with every recommended answer; .vox scale 1 world voxel); amended 2026-11-18 (the Phase 6 re-read after Plans 0077 and 0080 merged): a structure is a belt item of Plan 0077's placement mode, the cleared box a stamp of written air, the settings a `structures` menu group; questions 5 to 8 answered "all recommended"; **Phase 7's window done 2026-11-18 (83a cave, 83b outdoor): the verdict adds Phase 7b (the ghost is a coarse copy of the structure in its frame, the spider's rows on every source, the docs) and a five-minute owner look (7c) before Phase 8; see "Phase 7 verdict". 7c done 2026-11-19 (83c): the ghost passes; its verdict adds Phase 7d (the ground snap, `snap ground | free` on `F`, a chooser digit swaps, `E` places, the walker first, a click cancels the quit question) and window 83d (7e) before Phase 8; see "Phase 7c verdict"**
> **Created:** 2026-11-16
> **Milestone:** M7 in [`docs/roadmap.md`](../roadmap.md) (stamping at authored-building
> scale), with M6's `.vox` import; the engine side in `vxe-assets` and `vxe-world`, the testbed
> side in `apps/flythrough`.
> **Related ADRs:**
> [ADR-0065](../adrs/0065-materials-are-data-a-table-file-in-the-rig-grammar-and-every-vox-palette-entry-becomes-a-row.md)
> (proposed; the material file and palette rows) and
> [ADR-0066](../adrs/0066-a-large-prefab-is-stored-as-bricks-and-its-stamp-is-a-placement-the-world-finishes-as-chunks-arrive.md)
> (proposed; brick prefabs, placements past residency), both accepted at this plan's close.
> Rests on [ADR-0014](../adrs/0014-prefab-identity-in-the-op-log.md)
> (the stamp op and the save's prefab table),
> [ADR-0003](../adrs/0003-sphere-residency-with-lod-ring-and-edit-persistence.md) (residency
> and the spill), [ADR-0046](../adrs/0046-static-emissive-light-is-baked-per-chunk-when-a-chunk-loads-or-changes.md)
> (the bake follows a changed chunk) and
> [ADR-0051](../adrs/0051-rigs-and-clips-are-text-files-sampled-in-fixed-point-by-tick.md)
> (the `.rig` grammar and the viewer's reload).
> **Source:** the owner, 2026-11-16: "import nearly anything that can be created as a .vox
> file. Large architecture is something I'm particularly interested in"; materials from the
> `.vox` palette and the material table as a hot-reloaded data file, both approved the same day.
> **Starts after:** Plan 0063 and the clip-recorder chore merge (the viewer, `rig.rs`, the
> entities library), beside Plans 0078 and 0080; Phases 6 to 8 after Plans 0080 and 0077
> merge (the menu's dynamic page, the picker's placement at the crosshair). See "Order".
> **Amended 2026-11-18 (architect, the Phase 6 re-read at 99801643, which holds main 63e3695a
> with Plans 0077 and 0080 closed):** the section "Amended 2026-11-18" below replaces Phase 6's
> block and is Phase 7's errata; the dev reads it before the phases. Phases 1 to 5 and 8 stand.

## Amended 2026-11-18 (the Phase 6 re-read): a structure is a belt item

Phase 6 as approved names `apps/flythrough/src/menu/`, a `Page::dynamic`, `src/hud.rs` and a
`place` action on an open page. None of them is in the tree, and the flow it describes is the
one Plan 0077's first window struck (the panel over the view while placing). This section says
what the tree has, picks the flow, and rewrites Phase 6 against it.

### What the tree has (the plan's name, then the tree's)

| The plan says | The tree has |
|---|---|
| `apps/flythrough/src/menu/`, `Page::dynamic` | `apps/flythrough/src/hud/menu/{mod,entries,page,route,write,filter}.rs`; the one dynamic page is the hard-coded `PageId::Entities`, its rows `Row::Item(usize)` (four lines each beside an icon tile, `ITEM_LINES`) built each frame from `Modes::walkers.library`; `Enter` gives `Activated::Choose { entry }`, a digit `Activated::Assign { entry, slot }` |
| `src/hud.rs` (the banner) | `src/hud/mod.rs` with `hud/modes.rs` (`Modes`, `HudRow`, `BANNER_MS`); the notice row is `Run::say`, the lights' banner row `Modes::show_lights` |
| "the aim point from the crosshair's hit as Plan 0077's picker takes it" | `Run::aim_feet(extent)` over `belt::stand` (`entities/belt.rs`): the camera ray's first world hit within `PLACE_REACH` (300 voxels), never nearer than `max(NEAR_MIN 30, NEAR_SCALE 1.5 x the longest extent)`, dropped to the floor under a wall or ceiling hit |
| `place` from the open page | 0077's three states: the chooser only chooses and fills the belt (`Belt { slots: [Option<usize>; 9], selected, on, turn }`); placement mode with the menu closed stands the ghost (`Walkers::preview`, a marked instance, ADR-0074) at the aim; the left click places and the mode stays; `R` turns 22.5 degrees; the right click and `Esc` leave |
| `clear box on\|off`, `scale 1..4` as page actions | a setting is an entry of a menu `Group` (`toggles::TOGGLES`; a `Toggle` with `number: Some(Number)` for a stepped value and a `HudRow` that shows its mode), as `dig radius` is under `moving` (Plan 0080) |
| `src/structures.rs` "new" | there since Phase 4: `VoxArg`, `read`, `add_rows`, `scene`, `offset`, `ClearedSource`, `clear_box`, `ops` (a `VoxOps { prefabs, ops, chunks }`: a box carve, then a stamp per placement, submitted once the box's chunks are resident), `source`, `chapel_vox` |
| the castle "through `VoxScene`" | `castle::castle_vox()` gives the `VoxFile` (27 tiles of at most 256 a side, 21 kept) and `above_map()` its explicit `VoxMap`; `--castle` builds it on a thread (`Pending::StreamBuilding`, Plan 0075) and places it at generation through `HallSource` |

### Decision: (a) a structure joins Plan 0077's chooser, belt and placement mode

A structure is one more thing a belt slot holds; its chooser is a second page of items; the
click that places an entity places a structure. Reasons: the owner asked for the belt
(2026-11-18, "a belt of things that is possible to place") and for the chooser to close before
placing, and judges in the window, where a second grammar would show; one stand rule, one
click, one `R`, one `Esc` for everything placed, so `docs/flythrough.md`'s key table gains a
group and no new keys; and the chooser's mechanism (`Row::Item`, `Choose`, `Assign`)
generalises by giving the item a kind, which is the whole of option (c) that is needed.
Rejected: **(b)** a place-from-the-page action, 0077's Phase 3 flow the owner struck, and a
second placement path to keep in step with the first; **(c)** a general dynamic `Page` type
with an ADR: nothing needs it, two pages over one `Row::Item` kind is the generalisation, and
ADR-0061 already fixes the one-component rule. No ADR: the rejected options are plan-level and
recorded here.

### Mechanism

- **The item.** `belt::Item { Entity(usize), Structure(usize) }` (or the dev's name).
  `Belt.slots: [Option<Item>; SLOTS]`; `Row::Item(Item)`; `Activated::Choose { item }` and
  `Assign { item, slot }`. `Belt::fill` takes the entities' `Library::order` and then the
  structures' order into the slots left (the committed libraries: walker, spider, chapel,
  castle; owner question 8). The entity paths keep their `usize`.
- **The structures library.** `structures::Library { entries, notes }` on `Modes`
  (`hud/modes.rs`, beside `walkers`), scanned at start from `structures::LIBRARY`
  (`<name>/<name>.vox`, name order; `--structures-dir DIR` as `--library-dir`), plus the
  entry `castle` (generated) last. An entry holds its `VoxFile`, its `VoxScene` at scale 1,
  its bounds and the rows it added; a file that does not read or load is an entry in error
  (grey name, red reason, as 0077's). Every file's palette rows go into the world's table in
  `setup::world_table` before `World::generate` (ADR-0065; today `--vox`'s only), so the
  committed library adds the chapel's rows to every run; the start-up line says `structures: N
  entries, +R rows`. The castle adds no row (its indices map through `castle::above_map()` to
  the file's castle rows); its `VoxFile` is generated on a worker thread (the `--castle`
  build's `castle_vox_on`) the first time its slot is selected or its icon is asked, and until
  then its slot's name row and its chooser row read `castle: building` in grey, as an entry
  not built.
- **The chooser.** `PageId::Structures`, rows `Row::Item(Item::Structure(i))` at `ITEM_LINES`
  each: a blank line, `chapel   4 placements  4.8 x 3.2 x 4.0 m` (grey with its red reason for
  an entry in error), `rows +255   slot 3` (or `no slot`), a blank line. Reached by `Tab`, the
  `structures` group row and its first entry `choose a structure` (`Act::StructurePicker`, no
  key; owner question 6), and by the pad the same way. `Enter`, the left click, `South` and
  `` ` `` on an entry hand over as on the Entities page (`Menu::input`'s `Entities` arm serves
  both pages); a digit assigns and the page stays. `Menu::open_structures(selected)` beside
  `open_entities`. The Entities page lists entities only.
- **The settings.** `Group::Structures` after `Entities` in `Group::ALL` (13 groups), page
  title `structures`, three entries: `choose a structure` (the action), `clear box` (a toggle,
  `on|off`, default on) and `structure scale` (a number 1..4, step 1, default 1), the two
  toggles' modes on a row of their own, `HudRow::Structures` (or `Controls`, the dev's call),
  their state on `Modes` (`clear_box: bool`, `structure_scale: u32`). Both are read at the
  ghost and at the click; a scale change re-stands the ghost.
- **The ghost of a structure is a frame entity** (owner question 5): a `vxe_entity::Model` of
  the structure's box at the scale and turn, a real instance in the set as the entity ghost
  is, marked with the ghost colour (ADR-0074), touching no world voxel. For a box every side of
  which is at most `MAX_EDGE` (64) voxels, the frame is its twelve edges, one part each, one
  voxel thick, in the structure's most-used row (any opaque row: the mark's outline and tint
  carry the meaning). For a larger box (the castle's 600 x 600 x 500) the frame is its eight
  corners, each one part of three 32-voxel arms, one voxel thick, meeting at the corner. Both
  stay under `MAX_PARTS` (16) and `MAX_EDGE`; the model is made per entry, scale and turn and
  kept (`structures::frame_model`). The structure itself cannot be the ghost (ADR-0050's caps,
  the backlog's traced preview); the belt's name row names it beside the frame.
- **Where a structure stands** (owner question 7): `belt::stand_box(eye, fwd, size, cast) ->
  corner` beside `stand`, and `Run::aim_corner(size)` beside `aim_feet`, the one placement
  function of its kind. `p` is the camera ray's first world hit within `PLACE_REACH` on any
  face, else the view's point at `NEAR_MIN`; `p` is dropped to the floor as `stand` drops (the
  same code); `h` is `fwd` with z zeroed and normalised (`+x` when `fwd` is vertical); `s =
  (|h.x| size.x + |h.y| size.y) / 2` is the box's support along `h`; the footprint's centre is
  `c = p + h s`, so the box's near edge or corner is at `p` and the box grows away from the
  camera, never around it; the base is at `p.z`; the corner, to a voxel, is `[c.x - size.x /
  2, c.y - size.y / 2, p.z]`. `size` is the scene's at the scale, x and y swapped at an odd
  quarter turn. No `NEAR_SCALE` for a box: 1.5 x the castle's 600 voxels is 90 m, past
  `PLACE_REACH`, so `stand`'s rule would put every large structure 90 m along the view, in rock.
- **The turn.** A stamp has the 24 cube rotations, so `R` on a structure slot turns the box a
  **quarter turn** about z (`Rot::Z90` composed; 22.5 degrees is not a `Rot`): `Belt.turn`
  counts 22.5 degree steps, so `R` on a structure adds four and the stamp's `Rot` is `Z90` to
  the power of `turn / 4` (or the dev's cut); the turn is kept until the mode ends, as today.
  `VoxScene::placed_at` takes the quarter turns: each placement's origin and `Rot` composed
  about the box's minimum corner, in integers; the bounds swap x and y at an odd turn.
- **The click** submits through the server, as every edit does (`net.server.submit`, the
  tick's events print and say the lines): with `clear box` on, first a **stamp of written air**
  the size of `structures::clear_box` (a new `Prefab::uniform(size, material)`: every brick
  uniform and written, 4 B each after Phase 3, its id streamed as any prefab's, registered by
  id on the server and the client as the scene's prefabs are), then one `Op::Stamp` per
  placement of the scene at the scale and turn, in order. A stamp and not the box carve
  because a carve is an edit, which a non-resident chunk refuses, and a castle's box at
  `r_full` 6 is never fully resident (Phase 5's reading: 21 placements pending), so
  `--vox-op`'s wait would never end at runtime; a stamp past residency is a placement the world
  finishes as chunks arrive (ADR-0066), and air written by a stamp is air. `structures::ops`
  emits the air stamp for `--vox-op` too, `VoxOps.chunks` and the residency wait go (the ops
  go in on the first tick), so the two paths stay one and
  `the_chapel_as_ops_equals_the_chapel_from_the_source` keeps holding on the voxels. Forced
  residency was rejected in ADR-0066. After the click the mode stays, the frame ghost
  re-stands, and the notice reads `placed chapel 4.8 x 3.2 x 4.0 m: 5 ops, 4 chunks now, 0
  pending` (the chunks from the tick's results, the pending count from
  `ChunkStore::pending_placements`).
- **The words.** The structures row (`HudRow::Structures`) holds the two toggles' modes, as
  every toggle shows its mode in its row (`clear box on`, `structure scale 1`); the belt's
  name row names the selected structure and its size at the scale (`chapel  4.8 x 3.2 x 4.0 m`,
  at scale 2 `chapel  9.6 x 6.4 x 8.0 m  x2`); the frame shows the turn.
- **The icons.** A structure's tile is made through `icon::icon` over a model of the scene
  when the scene fits one (every prefab at most `MAX_EDGE` a side, at most `MAX_PARTS`
  placements: the chapel's four), on the icons' worker as an entity's is, its tile index after
  the entities' (`MAX_TILES` 32 holds both libraries); a structure that does not fit (the
  castle) keeps its words in its slot and its row, as an entry whose tile has not arrived.
- **`--castle`** keeps its path (the hall, the generation-time placement). The plan's "the
  castle through `VoxScene`" is met by the castle entry: its runtime placement goes through
  `VoxScene::load(castle_vox(), above_map(), ..)`, and the test asserts that the scene's
  prefabs are the tiles `HallSource` stamps, not that `--castle`'s start changes.
- **Under `--tester`** there is no belt and no chooser, as 0077 left it.

### Phase 6 (amended): a structure in the belt
- **Owner skill:** dev
- **What:** the mechanism above: the item kind, the structures library and its rows at start,
  the Structures chooser, the `structures` group, the frame ghost, the box rule, the quarter
  turn, the air stamp and the click, the icons, the words.
- **Files touched:** `apps/flythrough/src/entities/{belt,picker,placed,icon}.rs` (the item,
  `stand_box`, the frame ghost in the set and the marks, the tile index),
  `apps/flythrough/src/structures.rs` (the library scan and entry, `frame_model`, `ops` with
  the air stamp; split under `structures/` if it nears its cap),
  `apps/flythrough/src/hud/menu/{mod,page,route,write,entries}.rs` (`Group::Structures`,
  `PageId::Structures`, `Row::Item(Item)`, `Choose` and `Assign` by item, the title and
  footer), `apps/flythrough/src/hud/modes.rs` (`Modes::structures`, `clear_box`,
  `structure_scale`, `HudRow::Structures`), `apps/flythrough/src/toggles.rs` (the group's
  three entries, `Act::StructurePicker`), `apps/flythrough/src/run/{keys,frame,edit,
  hud_feed}.rs` (`aim_corner`, the structure's click, the prefabs registered, the notice),
  `apps/flythrough/src/setup.rs` (the library's rows in `world_table`, the start-up line,
  `vox_ops` without the wait), `apps/flythrough/src/cli/model.rs` (`--structures-dir`;
  `flags.rs` is at its cap), `apps/flythrough/src/app.rs`, `apps/flythrough/src/castle.rs`
  (the castle entry's scene), `crates/vxe-world/src/prefab.rs` (`Prefab::uniform`),
  `crates/vxe-assets/src/scene.rs` (`placed_at` with the quarter turns), new
  `apps/flythrough/src/run/structure_tests.rs` (`Run` is crate-private, as `picker_tests.rs`),
  `apps/flythrough/tests/{structures,castle,toggles,menu,picker}.rs`,
  `crates/vxe-world/tests/stamp.rs`, `crates/vxe-assets/tests/scene.rs`,
  `apps/flythrough/assets/structures/README.md`, `docs/flythrough.md` (the `structures`
  group, the belt's paragraph, `--structures-dir`, `--vox-op`'s row), `docs/plans/logs/0083.md`.
- **Done when** (CPU, no device; `run/structure_tests.rs` runs on a rock box large enough for
  the chapel's box, with the committed libraries and the castle entry left unbuilt; each
  under 1 s unless named):
  1. `the_library_lists_every_structure` (`tests/structures.rs`): the scan of
     `assets/structures/` gives one entry per directory holding `<name>/<name>.vox`, in name
     order, then `castle`; each file's palette resolves against the fly-through's table with
     the row count the README states (the chapel 255; a second file of the same palette 0); a
     directory without its `.vox` is a note; a file that does not parse is an entry in error
     with its reason.
  2. `a_uniform_prefab_is_written_air_one_word_a_brick` (`crates/vxe-world/tests/stamp.rs`,
     under 0.5 s): `Prefab::uniform([620, 620, 510], AIR)` holds 78 x 78 x 64 bricks, every one
     uniform and written, `bytes()` under 2 MB and `written_count` 196 044 000; stamped at the
     corner of a solid 64³ chunk it leaves every voxel of the overlap air and the result counts
     the rock row at the overlap's size; two constructions give one id.
  3. `a_quarter_turn_turns_the_scene` (`crates/vxe-assets/tests/scene.rs`): the chapel's
     scene `placed_at` with one quarter turn has every placement's origin and `Rot` equal to
     the hand-rotated values about the box's minimum corner, its bounds' x and y swapped, and
     its stamped voxels equal the unturned stamp's rotated voxel by voxel; four quarter turns
     are the identity.
  4. `the_belt_holds_a_structure` (`run/structure_tests.rs`): the belt fills walker, spider,
     chapel, castle, slot 5 empty; the wheel to slot 3 enters the mode with the chapel's frame
     ghost in the set (twelve parts, marked with the ghost colour) and nothing placed; `Tab`,
     `structures`, `choose a structure`, `Down` to the castle, `7` puts it in slot 7 and the
     page stays; `Enter` on the chapel selects slot 3, closes the menu and enters the mode;
     `` ` `` on the Structures page does the same; the Entities page lists no structure.
  5. `a_structure_stands_by_the_box_rule` (`run/structure_tests.rs`): with the camera 1.7 m
     over the floor looking 40 degrees down along +x (the floor hit about 2.6 m out) the
     chapel's corner has its minimum x at the hit's voxel, its y centre within one voxel of the
     hit's y and its base on the floor's top, so the eye is outside the box; after `R` once
     the box's x and y extents swap and the corner follows; at `structure scale` 2 the box
     doubles and the corner stays at the hit; aimed into the dark with no hit within 30 m the
     corner's x is the view's 3 m point's; a 600-wide box starts at the hit too and never holds
     the eye.
  6. `a_placed_structure_is_logged_stamps` (`run/structure_tests.rs`, under 5 s): the left
     click on the chapel's slot submits in one tick one stamp of a `Prefab::uniform` the size
     of `clear_box` and four stamps of the scene's prefabs, all kind 2 in the run's op log,
     the air stamp first; after the tick the world holds Phase 4's per-row counts of the chapel
     inside air with the 1 m margin; the mode stays and a new frame ghost stands; with `clear
     box` off there are four stamps and no air; a replay of the log on a fresh world hashes
     equal; `the_chapel_as_ops_equals_the_chapel_from_the_source` (`tests/structures.rs`) holds
     with `ops` emitting the air stamp.
  7. `the_castle_loads_through_the_scene` (`tests/castle.rs`): `VoxScene::load` of
     `castle_vox()` under `above_map()` gives 21 placements whose prefab ids are the ids
     `HallSource` stamps, in order; `--castle`'s world hash is unchanged; a castle entry before
     its build is grey with `castle: building`.
  8. The words: `every_toggle_shows_its_mode_in_the_hud_grid` (`tests/toggles.rs`) passes with
     `clear box on` and `structure scale 1` in their row; 0080's
     `every_toggle_and_action_is_an_entry`, `the_source_reads_no_key_the_tables_miss`,
     `every_live_entry_answers`, `the_fixed_set_is_the_table` and `the_route_is_one_function`
     pass with the group and the page; `--keys` lists `structures` with its three entries;
     0077's `the_belt_draws_its_slots` holds with `chapel` in slot 3 (its icon once made) and
     `castle` in slot 4; the belt's name row on slot 3 reads `chapel  4.8 x 3.2 x 4.0 m` and at
     scale 2 `chapel  9.6 x 6.4 x 8.0 m  x2`; after a click the notice reads `placed chapel 4.8
     x 3.2 x 4.0 m: 5 ops, N chunks now, 0 pending`; `docs/flythrough.md` has the group and the
     belt's structures; `node scripts/check-doc-links.mjs` exits 0.
  9. Readings (`docs/plans/logs/0083.md`): the frame models' pool bytes (the chapel's edges,
     the castle's corners; guesses 150 KiB and 1 MiB of the 8 MiB pool); the chapel's icon
     time; `Prefab::uniform` of the castle's box, its construction with the id (guess under
     100 ms, the hash over 196 M bytes) and its stamp call; the structures' scan and rows at
     start against Phase 4's start-up line.

### Phase 7 (errata): the owner's window gains the belt
The owner reaches a structure through the belt, not a page: (a) and (b) read "finds it in the
belt (the wheel, or the Structures chooser) and places it with the click". Added checks, with
the owner's words into the readings log: the chapel's frame ghost in the cave (question 5),
`R`'s quarter turn, `structure scale` 2 from the menu, `clear box` off then on, where the box
stands against the crosshair (question 7); the castle's corners frame, its click and the fly
to its far side as (b) says; whether structures belong in the belt beside the entities
(question 8). The picks the phase names (the emission factor `k`, the default scale, `clear
box` on) stand.

### Phase 8 (addition)
`gpu_hud_golden` gains `the_structures_chooser` (the page with the chapel's icon and the
castle's words, pixel exact on lavapipe, under 10 s). The rest stands.

### Budgets added 2026-11-18 (guesses until Phase 6 reads them)

- **The frame ghost:** 12 parts of at most 64 x 1 x 1 voxels (the chapel) or 8 parts of
  32 x 32 x 32 (the castle's corners) in the 8 MiB entity pool while a structure slot is
  selected in the mode; freed when the slot or the mode changes; the mark's cost is 0077's.
- **The air stamp:** the castle's box as a uniform prefab is 389 376 bricks x 4 B, about 1.6 MB
  in RAM and in the log per castle placement; the chapel's about 3 KB. Section 3's prefab RAM
  line reads it with the castle's prefabs.
- **Rows at start:** the committed library adds the chapel's 255 rows to every run (64 B a row
  on the GPU, 16 KB), under the "1000 rows 64 KB" line of the Budgets table.

### What this amendment reverses in the text above

Phase 6's "places the selected `.vox` at the crosshair" from the open page, and the Decision's
"from a Structures page of Plan 0080's menu, at the crosshair at runtime", read: from the
belt, in placement mode, with the click. "The preview is the box's footprint on the HUD" (the
Decision, "What this plan does NOT do") is superseded by the frame ghost under question 5; a
traced preview of the structure itself stays out (backlog). The Decision's `clear box` "carved
clear first" is a stamp of written air, not a carve op. Phase 6's aim rule ("the footprint
centred on the hit") is the box rule under question 7.

### Owner questions added 2026-11-18 (each with a recommendation)

**Answered 2026-11-18 (owner, "all recommended, go ahead"):** 5 the marked frame, 6 no new key,
7 the box grows away from the aim, 8 the belt starts with the entities then the structures.

5. **The ghost of a structure.** Recommended: a marked frame entity (its twelve edges, or
   its eight corners for a box over 64 voxels a side) standing where the structure will stand,
   blue as the entity ghost is, with the name and size in the belt's name row. Alternative:
   the words alone, no frame (the plan's original "the HUD shows its box"): cheaper, but
   nothing in the world shows where a 5 m chapel lands before the click.
6. **Reaching the Structures chooser.** Recommended: through the menu (`Tab`, `structures`,
   `choose a structure`) and through the belt (a structure's slot by the wheel or a digit), no
   new key; `` ` `` stays the entities'. Alternative: a key for the structures chooser, one
   more bind of the kind Plan 0080 removed.
7. **Where a structure stands.** Recommended: the box grows away from the aim: its near edge
   at the crosshair's hit (dropped to the floor under a wall or a ceiling), its base on the
   floor, so the camera is never inside it and a castle starts where you look. Alternative:
   the footprint centred on the hit, as the plan wrote: a castle then surrounds the camera and
   its stamp may bury it.
8. **The belt's fill.** Recommended: the belt starts with the entities (walker, spider), then
   the structures (chapel, castle) in the free slots, so the wheel reaches a structure at once.
   Alternative: a structure enters the belt only from its chooser.

## Phase 7 verdict (2026-11-18): a coarse ghost, the spider's rows, the castle on the surface

The owner's window ran twice: session 83a in the cave (`--source cave`, the binary at
036b5c25) and 83b on the outdoor hills (`--source outdoor --time 12:00`, at 036b5c25 and then
at 0e85862a). Two fixes landed between them: fd650c02 (the outdoor world's table holds the
structures library's rows; before it every structure refused to load under `--source outdoor`
with `model 0 uses palette indices with no mapping`) and 0e85862a (a one-part coarse icon for
a scene over the model's limits, made on the castle's build worker). 83b ended with "all works
but castle flickers badly and there is no icon for spider", then "no, done". The owner's words
are quoted below; Phase 7b copies them, with the HUD readings of the three screenshots, into
the readings log (the lane's `live/window83a/` is not tracked).

### The verdicts

1. **The ghost.** Owner: "there was no ghost besides box for placing, we should have something
   simplified at least to understand what it is". This revises question 5: the marked frame
   alone does not say what will stand there. Decision: **the ghost is a coarse copy of the
   structure inside its frame**, one marked instance at world scale; the mechanism, its
   rejected options and its cost are under Phase 7b. The Phase 6 amendment's "a traced preview
   of the structure itself stays out (backlog)" and the Decision's "the preview is the box's
   footprint on the HUD" are superseded by it; a full-detail preview stays out.
2. **The castle in the cave.** Owner: "castle is build without surrounding rocks, and its too
   big for cave, we should test it on surface" (23.png: the clear box cut a room-sized void and
   the castle stands in it with the cave's cut faces around; HUD `clear box on, structure scale
   1`, fps 49.5, queue depth 18 129, missing 4278). Decision: **documented, neither refused nor
   warned about.** The fly-through has no notion of a room, the clear box is question 4's pick
   and did what it says, and the engine never judges whether a structure fits its surroundings
   (a game does, with the edit result's counts). `docs/flythrough.md` and the structures README
   say that a structure larger than the cave's rooms stands in its cleared box with the cut rock
   around it, and that a large structure is judged on the outdoor hills. **The window's scene
   for structures is the outdoor hills** (`--source outdoor --time 12:00`), as 83b ran; the
   chapel's cave check stays for Phase 8's golden, since a chapel fits a cave.
3. **The spider's icon.** Owner: "there is no icon for spider" (25.png: the walker, chapel,
   terrace and castle slots draw icons, slot 2 reads `spider`). The lane's binary read with
   `--library`: `spider error: rig line 13: unknown material crystal`, on the cave's table and
   on the outdoor table alike. Not a regression of this plan: the spider's `material` lines
   name the showcase's `crystal`, `amethyst` and `ember`, and Plan 0077 left it in error on
   every source but the showcase (`run/picker_tests.rs`'s header, the entities README). It is
   the class of fd650c02 (an item names rows the world's table lacks) and gets the castle's
   cure from Phase 6: a rig's `material` line that names a row the scene's selection lacks
   takes that row from the material file at the start. A bug done-when with a test across the
   sources, under Phase 7b.
4. **The castle's icon arrives with its first build** (0e85862a: the castle's `VoxFile` is
   generated on a worker the first time its slot is selected, its icon with it, so the slot
   reads `castle` until then). Options: build the castle at every start on a worker (about
   45 MB of prefabs kept, a transient `VoxFile` several times that and seconds of CPU beside
   the stream's start, in a run that may never place it); ship a precomputed icon as an asset
   (a 16 KB image that must track a generator, with a 5 s test to prove it does); accept it.
   **Recommended and taken: accept it.** The slot's words say `castle`, then `castle:
   building` in grey, then the icon; the docs say so.
5. **The castle flickers inside, under the sky.** Owner: "castle flickers badly" (25.png, at
   0e85862a: inside the placed castle the walls clip white and blue, the vault is dark and
   grainy with sparkles; HUD queue depth 10 751, missing 0, scene -4.1 EV, eye +1.34 EV, fps
   71). Reading: a freshly stamped large structure under a sky, that is the sky ray's region at
   rest with a young history and the exposure moving on it. **Routed, not fixed here:** it is
   what ADR-0076 (proposed, on main at 53379f0c) rules 1 (a still world gives a still image),
   2 (view independence) and 3 (one monotonic fade) forbid, Plan 0090's counter measures it,
   and Plan 0086's lane owns the sky ray's history. The backlog gains the placed castle's
   interior as a counter scene.
6. **The questions the owner left blank count as accepted** (the phase's "the picks the phase
   names stand"): question 8 (the belt starts with the entities, then the structures), the
   default `structure scale` 1, `clear box` on by default, and the emission factor `k` at
   `DEFAULT_EMISSION_FACTOR` 0.3. The colours, the glass and the glow were the owner's "yes"
   in session 83 (readings log) and are not asked again.

### Phase 7b: the ghost is a coarse copy, and the spider builds on every source
- **Owner skill:** dev
- **What:** `vxe-entity` gains a coarse grain per part; the structure's ghost becomes one
  model, its coarse body plus its frame; the entities library's named rows join every world
  table; the docs and the readings log follow the verdict.
- **The ghost's mechanism.** A `Part` gains `coarse: u8`, **world voxels per model voxel**,
  1 to 16 (16 x `MAX_EDGE` is 1024, `edit_axis_max`, the largest box a stamp takes, so every
  structure that can be placed has a body). `detail` keeps its meaning (model voxels per world
  voxel, 1, 2 or 4) and a part is fine or coarse, never both (`coarse` over 1 needs `detail`
  1; `ModelError::BadCoarse`). `MAX_PARTS` and `MAX_EDGE` stay, in model voxels. The factor
  enters exactly where `detail` enters, as its inverse, in integers: `ComposedPose::part_point`
  (`m * rel * coarse / (16384 * detail)`), `part_reach` (times `coarse`), `part_raycast` (a
  local coordinate is carried in units of `1 / (256 * 16384 * coarse)` model voxel, so `l0 =
  pivot * 16384 * coarse + detail * M^T (world - pivot)` stays an integer; `hi` and the
  crossing edges scale with it; the header's overflow bound gains four bits and is re-read)
  and `Model::id` (the coarse byte after the detail byte; no model id is pinned or persisted,
  the icon cache and the pool are in-process). In `vxe-render` the grain is one float, model
  voxels per world voxel (`detail / coarse`): `PartXform::from_parts` and `blend_parts` take
  it where they take `detail`, `instances.rs` writes its inverse in word 27 (the shadow
  offset's scale, which a coarse part wants larger), and `entity.wgsl` changes only that
  comment: the part transforms already carry the scale. At `coarse` 1 every pose, hit and
  transform is bit-identical to today's.
  The ghost: `structures::ghost_model(scene, quarters, row) -> Model`, part 0 the **body**,
  `coarse_model`'s volume of the scene at the `structure scale` (k the least with every side at
  most 64: the chapel k 1 at scales 1 to 3 and 3 at scale 4, the castle 10) turned by the
  quarter turns (`Prefab::rotated`, exact) at `coarse` k; then the frame's parts as today
  (twelve edges for a box at most 64 a side, else eight corners): 13 or 9 parts. One instance,
  marked with the ghost colour (ADR-0074: the outline at the instance's edge, the tint over
  the body), touching no world voxel, standing by the box rule with the body's corner at the
  box's corner (the body overhangs the box by under k voxels on a side k does not divide).
  The body's volume per entry and scale is kept beside the icon's model: the castle's at scale
  1 is made on its build worker (0e85862a's icon volume is the same one), the others on the
  frame thread at the first stand; a body that reads over 50 ms there moves to the icons'
  worker, and the frame stands alone until it arrives. The icon is unchanged.
- **Rejected:** (a) the structure itself as an entity: ADR-0050's caps (the castle is 800
  parts at `MAX_EDGE`), and raising them costs every entity's pool and walk; (b) the coarse
  model drawn at `detail` 1 inside the frame, a 6 m maquette of a 60 m castle: no engine
  change, but it lies about scale where the owner wants to see what will stand there; (c) a
  temporary stamp of the structure in a ghost row: it writes voxels the bake, the collision and
  the log see (ADR-0074's ghost touches nothing); (d) Plan 0082's pose scale: a pose property
  that animates, capped at 4 and not merged; (e) the overlay drawing each placement's box: the
  overlay is 2-D text and tiles (ADR-0075), and a wire box is the frame already. ADR-0050
  fixes `detail` at 1, 2 or 4; the coarse grain extends it, and the close review records it
  (a dated note on ADR-0050's successor or a short ADR of its own, the architect's call then);
  this section carries the rejected options until it does.
- **The spider's rows.** `setup::world_table` appends, **after** the structures library's
  rows, the rows the entities library's rigs name in their `material` lines that the table
  lacks, taken from the material file by name (the castle rows' path of Phase 6,
  `add_library_rows`, generalised); a name the file lacks either stays an error on the entry.
  Appended last, so no pinned hash moves (the chapel's and the castle's rows keep their ids).
- **Files touched:** `crates/vxe-entity/src/{model,pose,raycast}.rs`,
  `crates/vxe-entity/tests/raycast.rs`, `crates/vxe-render/src/entities/{display,instances}.rs`,
  `crates/vxe-render/src/shaders/entity.wgsl` (the comment),
  `apps/flythrough/src/structures/library.rs` (`ghost_model`, the body kept per scale, the
  castle's on its worker), `apps/flythrough/src/entities/{picker,placed}.rs` (the ghost through
  `ghost_model`; `FrameGhost`'s name is the dev's call), `apps/flythrough/src/setup.rs`
  (`world_table`), `apps/flythrough/src/entities/library.rs` (the names a rig's `material`
  lines carry, read without building), `apps/flythrough/src/materials.rs` (a row by name from
  the file), `apps/flythrough/src/run/structure_tests.rs`, `apps/flythrough/src/run/picker_tests.rs`
  (its header), `apps/flythrough/tests/entities.rs` (a doc comment),
  `apps/flythrough/tests/gpu_hud_golden.rs` and its goldens if re-blessed, `docs/flythrough.md`,
  `apps/flythrough/assets/structures/README.md`, `apps/flythrough/assets/entities/README.md`,
  `docs/plans/logs/0083.md`.
- **Done when** (CPU unless named; each under 1 s unless named):
  1. `a_coarse_part_spans_k_world_voxels_a_voxel` (`crates/vxe-entity/tests/raycast.rs`): a
     one-part model of a 2 x 2 x 2 volume of material 3 at `coarse` 10, its pivot at its
     corner and its root at the origin: a ray along +x from (-5, 5, 5) voxels hits the -x face
     at t 5 voxels in voxel (0, 0, 0); from (-5, 15, 15) in voxel (0, 1, 1); from (-5, 25, 5)
     it misses (the body ends at 20); `ComposedPose::part_point` of the local corner (2, 2, 2)
     is the world point (20, 20, 20); the part's reach is the same part's reach at `coarse` 1
     with its radius ten times over; `Model::new` refuses `coarse` 0 and 17, and `coarse` 2
     with `detail` 2, as `BadCoarse`; the model's id differs from the same volume's at
     `coarse` 1. `entity_hits_match_the_fixed_vectors` and `clip_poses_match_the_fixed_vectors`
     hold unchanged.
  2. `a_coarse_part_renders_at_world_scale` (`crates/vxe-render/src/entities/display.rs`, a
     unit test beside the detail-1 cube's): the same cube's `world_box` spans 20 world voxels
     on each axis and its instance's word 27 reads 10.0; at `coarse` 1 the instance words are
     byte-identical to today's. The entity goldens hold byte for byte:
     `crates/vxe-render/tests/gpu_entities.rs`, `apps/flythrough/tests/gpu_entity_library.rs`
     and `gpu_entity_pool.rs` (the phase's gpu binaries, lavapipe, each under its tier's line
     as today).
  3. `the_ghost_is_a_coarse_copy_in_its_frame` (`run/structure_tests.rs`; the castle's half
     waits for its build as `the_built_castle_has_an_icon` does, under 10 s): the chapel's slot
     stands one marked ghost of 13 parts: part 0 a 48 x 32 x 40 volume at `coarse` 1 whose
     written voxels equal the chapel's scene placed at the origin voxel by voxel, parts 1 to 12
     the edges; at `structure scale` 2 the ghost has 9 parts, part 0 48 x 32 x 40 at `coarse`
     2 and eight corners; after `R` once part 0 is 32 x 48 x 40 and the frame's extents swap;
     the castle's ghost has 9 parts, part 0 60 x 60 x 50 at `coarse` 10 equal to its icon's
     volume; the ghost's corner is the box rule's corner, the instance carries the ghost
     colour's mark, and the world's voxel hash is unchanged while it stands. `the_belt_holds_a
     _structure`'s "twelve parts" becomes 13.
  4. `the_spider_builds_on_every_source` (`run/structure_tests.rs`, one test per heavy source
     as 0e85862a split them, each under 10 s): for `--source cave`, `--source outdoor`,
     `--castle` and `--source showcase` the entities library built against the run's world
     table loads `spider` with no error and its nine `material` lines map to rows named
     `shell`, `rubble`, `rock`, `ore`, `crystal`, `amethyst`, `ember` and the walker's two; on
     the first three sources `crystal`, `amethyst` and `ember` are the table's last three rows
     (after the structures library's), on the showcase its own; `the_chapel_stands_at_the_spawn`
     and `the_castle_loads_through_the_scene` hold with their pinned hashes; after the icons'
     wait the spider's tile is ready and the belt's slot 2 draws it; the start-up line names
     the rows added (`entities: 2 entries, +3 rows`, or the dev's words, asserted).
     `picker_tests.rs`'s header and `the_spider_has_its_rows_parts_and_clips`'s comment say
     the spider is in error against the bare `cave_materials()` and builds against a run's
     table. `the_belt_and_the_chooser_over_the_carved_view` (`gpu_hud_golden`) is re-blessed
     on lavapipe if its run now draws the spider's tile, and the log says so.
  5. The words and the docs: `docs/flythrough.md` (the belt's paragraph: the ghost is a coarse
     copy of the structure in its frame; the castle's icon arrives with its first build; a
     structure larger than the cave's rooms stands in its cleared box, judge it on the hills),
     the structures README (the same three), the entities README (the spider's rows join every
     source's table), `docs/plans/logs/0083.md` (the window's entry: the owner's words from
     this section, the HUD readings of 23 to 25.png, the `--library` reading);
     `node scripts/check-doc-links.mjs` exits 0.
  6. Readings (`docs/plans/logs/0083.md`): the castle's body in pool bytes (guess under
     300 KB, from the corners frame's 303 872 B for 262 144 voxels) and its make time on the
     worker; the chapel's body at scales 1 and 4, make time on the frame thread (guess under
     30 ms; over 50 ms moves it to the worker); the entity term of the frame with the castle's
     ghost filling the view, from 7c's HUD, no bar (a ghost is transient).

### Phase 7c: the owner looks at the ghost
- **Owner skill:** human
- **What:** five minutes on the outdoor hills at noon, batched with another plan's window: the
  owner selects the castle's slot, sees the coarse castle in its frame where it will stand,
  turns it with `R`, selects the chapel at scale 2, and finds the spider's tile in slot 2.
- **Files touched:** `docs/plans/logs/0083.md` (the owner's words, the HUD's entity term).
- **Done when:** the owner says whether the ghost reads as the structure it stands for. A
  "yes" sends Phase 8 on; a "no" with words comes back to the architect for the mechanism.

### Phase 8 after the verdict

Phase 8 runs after 7c. Its `chapel_golden` keeps the dark cave; `the_structures_chooser`
(the addition above) stands; the picks it writes as defaults are the accepted ones under
verdict 6, so it changes no default.

**Owner picks 2026-11-19 (orchestrator's batch), carried by Phase 7d (under "Phase 7c
verdict"), not Phase 8:** the belt order puts the walker first (walker, spider, then the
structures); and a click or a pad button cancels the "Esc again to quit" question (Plan 0079
Phase 6's confirmation, closed in v0.65.0; this plan carries the change since it is the open
lane on the fly-through). Their done-whens are 7d's 6 and 7.

### Budgets added 2026-11-18 (guesses until Phase 7b reads them)

- **The ghost's body:** one part of at most 64 model voxels a side at `coarse` k in the 8 MiB
  entity pool while a structure slot is selected, beside the frame's parts: the castle's
  60 x 60 x 50 (guess under 300 KB), the chapel's 48 x 32 x 40 (guess under 80 KB); freed as
  the frame is.
- **Rows at start:** the spider's three rows on the cave, outdoor and castle tables (64 B a
  row on the GPU), under the Budgets table's "1000 rows 64 KB" line.
- **The frame with a ghost in view:** the entity term of a 64-voxel-a-side part filling the
  view, read in 7c, no bar.

## Phase 7c verdict (2026-11-19): the ghost passes; the snap, the free mode, the digit and the key

The owner's window 83c ran on the outdoor hills at noon with no mouse at hand (the lane's
`live/window83c/verdicts.md` and `castle-ghost-floating.png`, untracked; Phase 7d copies the
words into the readings log). **The coarse ghost passes:** it reads as the castle, `R`'s
quarter turn, the chapel at scale 2 and the spider's icon are all fine. Four requests came out
of it; they and the two 2026-11-19 picks are **Phase 7d**, a five-minute look at the snap and
the free mode is **Phase 7e** (window 83d), and Phase 8 runs after it. The lane merges main
first: the `Esc again to quit` question (`run/mod.rs`'s `QUIT_ASK_WORDS`, Plan 0079 Phase 6,
v0.65.0) is on main and not in this branch yet.

### The verdicts

1. **The ghost floats.** The picture: the castle's ghost over a hill, the camera under its near
   wall, the far half over the valley with air under it, the HUD's `clear box on, structure
   scale 1`. The box rule puts the base at `p.z`, the aim's hit, and a 60 m footprint on hills
   meets terrain 10 to 20 m away from that height. Decision: **by default the box snaps to the
   terrain under its footprint**, defined below (the lowest column; which column wins on a
   slope is owner question 9, with a recommendation).
2. **A free mode.** Owner: a toggle "e.g. Space" into a mode where the structure goes anywhere,
   in the air too. Decision: a `snap` setting in the `structures` group, `ground | free`
   [ground], with a key. **Not `Space`:** `Space` is a fixed key (`hud::menu::FIXED_KEYS`),
   held to rise in fly mode and to jump in walk mode, and placement mode keeps the camera
   free, so a press toggle on it would flip the snap on every rise; the owner's words named
   the idea, not the key. **The key is `F`**, the pad's **D-pad up** in the mode: both are
   unbound (`F` is only a typed letter of the menu's filter, `route.rs`; D-pad up and down act
   in the menu only, on the selection and a number's edit), and `hud::menu::entries::
   bound_keys()` is the check. The mode shows in the window (the words below).
3. **A digit on an occupied slot.** In the entities chooser the owner pressed `5` meaning "go
   to the castle's slot"; the digit put the cursor's entry into slot 5 over the castle, with no
   evident way back. Decision: **a digit on an occupied slot swaps**: the cursor's entry takes
   the slot and the slot's holder takes the entry's old slot; nothing leaves the belt, the same
   press undoes it, and the notice says both moves. Rejected: a confirmation (a second press
   within 3 s): a modal step a keyboard-only owner must learn, and the first press then looks
   like nothing happened; words alone after the replacement (`castle replaced; structures,
   choose a structure, 5`): the loss stays and the way back is three steps.
4. **Placing has no key.** Decision: **`E` places** in placement mode as the left click does
   (a fixed key beside the click; the pad keeps `R2`). Not `Enter`: it is the readings' key
   with the menu closed, and the owner reads the frame line right after a castle's placement;
   not `Space`: held to rise. `E` is unbound (a typed letter of the filter only).

### Phase 7d: the snap, the free mode, the swap, the placing key, the walker first, the Esc cancel
- **Owner skill:** dev
- **What:** a structure's box snaps to the terrain under its footprint; `snap ground | free`
  in the `structures` group with `F` and D-pad up; a chooser digit on an occupied slot swaps;
  `E` places; the belt starts with the walker; a click or a pad button cancels the quit
  question. Entities keep `stand` (the feet at the hit): the snap and the free mode are the
  box rule's, structures only.
- **The snap (ground).** `belt::stand_box` finds `p` and the footprint as today (the near
  side at the aim, growing away), then sets the base from the footprint's columns: a lattice
  of downward casts over the footprint, `n = min(8, ceil(size / 8) + 1)` an axis, the
  footprint's corners among them (the chapel 48 x 32: 7 x 5 = 35 casts; the castle 600 x 600:
  64; a box under 8 a side: its 4 corners), each from `p.z + FLOOR_DROP` straight down over
  `2 x FLOOR_DROP` (20 m). A column's top is its hit's height; a cast that starts inside a
  solid voxel counts as `p.z + FLOOR_DROP`; a miss counts as `p.z - FLOOR_DROP` (the cap: a
  valley deeper than 10 m under a castle still shows air, and the box never sinks out of
  sight). **The base is the lowest column's top** (question 9); the corner is `[c.x - size.x
  / 2, c.y - size.y / 2, base]`. The casts run on the client's world as today's `cast` does,
  once per re-stand; `f32` as `stand_box` is (the testbed, not the engine).
- **The free mode.** `snap free`: `p` is the aim's first hit within `PLACE_REACH` on any face
  as it is (no drop to the floor), else the view's point at `PLACE_REACH` (30 m, the farthest
  the aim reaches; a nearer stand is a hit); no column casts; the base at `p.z`, the footprint
  by the box rule. So a castle in the air stands where you look at the sky, 30 m out, and a
  castle on a hilltop stands at the hit with nothing sunk.
- **The words.** The structures HUD row reads `clear box on, structure scale 1, snap ground`
  (or `snap free`); a toggle prints the notice `snap free` / `snap ground` and re-stands the
  ghost at once; while the snap is free the belt's row carries the word `free` in yellow after
  its ninth slot, so the mode shows whenever the belt does. The Keys page and `--keys` list `F`
  on `snap` and `E` with the click.
- **The swap.** `Belt::assign(s, item)`: the slot `s` empty or holding `item`: as today; `s`
  holding `old` and `item` in slot `t`: `s` gets `item`, `t` gets `old`; `item` in no slot:
  `old` moves to the first empty slot after `s` (wrapping), or with no empty slot leaves the
  belt. The call returns what moved; the chooser's notice reads `slot 5 walker, slot 1 castle`
  or `slot 5 walker, slot 6 castle` or `slot 5 walker; castle left the belt`, and the entries'
  `slot <n>` rows refresh. The belt's frame moves to `s` as today.
- **The placing key.** `E` in placement mode, the menu closed, is the left click's placement
  (the same path, ops and notice, the mode stays); out of the mode `E` does nothing; in the
  menu it types.
- **The walker first.** `Belt::fill` puts the built-in walker's entry (`library::BUILTIN_NAME`)
  first, then the rest of the entities library's order, then the structures: walker, spider,
  chapel, castle, slot 5 empty.
- **The Esc cancel.** After the first `Esc` shows `Esc again to quit`, a mouse button or any pad
  button cancels the question as "any other key" does today.
- **Files touched:** `apps/flythrough/src/entities/belt.rs` (`stand_box`, `assign`, `fill`,
  the belt's `free` word), `apps/flythrough/src/toggles.rs` (`snap`, `E` in the fixed set or
  beside the click, the dev's call), `apps/flythrough/src/hud/menu/{mod,page,entries}.rs`
  (`FIXED_KEYS`, `Assign`'s notice, the Keys page), `apps/flythrough/src/run/keys.rs` (`E`,
  `F`, D-pad up, the quit question's cancel), `apps/flythrough/src/hud/modes.rs` (`snap`),
  `apps/flythrough/src/structures/words.rs`, `apps/flythrough/src/run/{structure_tests,
  picker_tests,menu_tests}.rs`, `docs/flythrough.md` (the Fixed table: `E`; the shortcuts
  table: `structures | F | snap`; the pad table: D-pad up; the Placement mode paragraphs: the
  snap, the free mode, the swap, the belt's order), `apps/flythrough/assets/structures/README.md`,
  `apps/flythrough/assets/entities/README.md` (the belt's order), `docs/plans/logs/0083.md`.
- **Done when** (CPU; each under 1 s unless named; the run tests on the chapel's run under 10 s
  as `the_belt_holds_a_structure` is):
  1. `a_snapped_box_sinks_to_its_lowest_column` (a unit test in `belt.rs` beside
     `the_belt_formats`, a synthetic `cast` that counts its calls): a step terrain whose top is
     z 100 at x < 300 and z 70 from x 300 on, an eye at (0, 0, 110) looking at (50, 0, 100),
     the castle's 600 x 600 x 500 box: `p` is (50, 0, 100) (the floor hit), the footprint
     x 50..650, the corner (50, -300, 70) and the downward casts number 64; the step moved to
     x 700 (past the footprint) gives (50, -300, 100); the step's far side at z -200 gives the
     base 0 (the cap, `p.z - FLOOR_DROP`); the chapel's 48 x 32 x 40 box over the step at x 80
     gives the base 70 with 35 casts; a step at x 80 **up** to z 250 (a wall inside the
     footprint, above the casts' start at z 200) gives the base 100, since a cast that starts
     inside solid counts as high and the lowest wins. `a_structure_stands_by_the_box_rule` holds: its floor is level, so the corner is
     unchanged.
  2. `a_free_box_stands_where_the_aim_points` (the same module): with the snap free, the eye at
     (0, 0, 110) along +x with no hit: the corner's x is 300 (`PLACE_REACH`) and z 110, one
     cast in all; looking 30 degrees up at a ceiling hit at t 100 on a downward face: `p` is
     the hit as it is (z 160), the base 160, no drop; the step terrain of test 1: the base is
     100 (`p.z`), not 70.
  3. `the_snap_key_frees_the_ghost` (`run/structure_tests.rs`): in the chapel's slot on the
     rock box, `F` once: the structures HUD row reads `snap free`, the notice `snap free`, the
     belt's cells carry `free`, and the ghost's corner is `stand_box`'s free answer for the
     frame's eye and aim (asserted against the function); `F` again: `snap ground`, no `free`
     in the belt, the ground corner; with the menu closed the pad's D-pad up does the same;
     with the menu open `F` types `f` into the filter and the snap stays; `--keys` prints the
     `snap` entry with `F`; `bound_keys()`'s uniqueness test holds with `E` and `F` in it.
  4. `a_digit_on_an_occupied_slot_swaps` (`Belt::assign`, a unit test): slots [A, B, C, D,
     empty..]: `assign(3, A)` gives [D, B, C, A, ..]; `assign(0, A)` again gives [A, B, C, D,
     ..] (the same press undoes it); `assign(0, E)` with E in no slot gives [E, B, C, D, A, ..]
     (A to the first empty slot after 0); with nine slots full, `assign(0, J)` gives J in slot
     1 and A gone, the return naming A as left; `assign(1, B)` changes nothing. And in
     `run/picker_tests.rs`, `the_chooser_swaps_an_occupied_slot`: the entities chooser open,
     the cursor on the walker (slot 1), `4`: slot 4 walker, slot 1 castle, the notice `slot 4
     walker, slot 1 castle`, the belt's frame on slot 4, the castle's row in the structures
     chooser reads `slot 1`; `1` with the walker under the cursor: back to walker 1, castle 4.
  5. `a_key_places_as_the_click_does` (`run/structure_tests.rs`): in the chapel's slot `E`
     submits the ops `a_placed_structure_is_logged_stamps` asserts for the click (the air box
     then the placements), the notice reads `placed chapel 4.8 x 3.2 x 4.0 m: ...`, the mode
     stays; in the walker's slot `E` places `walker #1` as `a_click_places_and_the_mode_stays`
     has the click do; out of the mode `E` submits no op and prints no notice.
  6. `the_belt_holds_a_structure` (or `the_belt_draws_its_slots`) asserts slot 1 walker, 2
     spider, 3 chapel, 4 castle, 5 empty; Phase 7b's "the spider takes belt slot 1" deviation
     is reversed and its Notes line says so.
  7. `a_click_cancels_the_quit_question` (`run/menu_tests.rs`, beside main's `Esc again to
     quit` test): `Esc`, a left click, `Esc` within 2 s: the run is not quitting and the banner
     no longer reads `Esc again to quit` after the click; the same with the pad's `South` in
     place of the click; `Esc`, `Esc` still quits.
  8. The words and the docs: `docs/flythrough.md` (the three tables and the paragraphs named
     above), the structures README (the snap and the free mode), the entities README (the
     belt's order), the Keys page; `docs/plans/logs/0083.md` holds the 83c window's words from
     `live/window83c/verdicts.md` and the picture in words (the castle's ghost over the hill,
     the HUD rows legible in it); `node scripts/check-doc-links.mjs` exits 0.
  9. Readings (`docs/plans/logs/0083.md`): the snap's casts per re-stand on the castle's box
     over the outdoor hills, wall time on the dev box (guess under 0.2 ms for 64 casts of at
     most 200 voxels; over 1 ms moves the lattice to 6 x 6 and the plan says so).

### Phase 7e: the owner looks at the snap and the free mode (window 83d)
- **Owner skill:** human
- **What:** five minutes on the outdoor hills at noon (`--source outdoor --time 12:00`),
  batched with another plan's window: the castle's slot aimed across a slope (the base at the
  lowest column: does the uphill side standing in the cleared cut read right, or should the
  highest or the centre column win, question 9); `F` to free and the castle over the valley
  from the sky; `E` places; in the entities chooser a digit on the castle's slot, then the same
  digit back.
- **Files touched:** `docs/plans/logs/0083.md` (the owner's words).
- **Done when:** the owner says whether the snapped castle and the free mode read right and
  which column wins on a slope. A "yes" sends Phase 8 on with the picks as defaults; a "no"
  with words comes back to the architect.

### Phase 8 after 7e

Phase 8 runs after 7e, unchanged: `chapel_golden` keeps the dark cave, `the_structures_chooser`
stands (the belt's golden `the_belt_and_the_chooser_over_the_carved_view` runs with the snap on
the ground, so the `free` word changes no golden), and the picks it writes as defaults are 83d's
(the slope rule) with the accepted ones under verdict 6.

### Owner question added 2026-11-19 (with a recommendation; 83d answers it by eye if blank)

9. **Which column wins on a slope.** Recommended: **the lowest** column under the footprint:
   nothing floats, and on a slope the uphill side stands in the cut `clear box` makes anyway
   (a castle cut into the hill; with `clear box` off the uphill walls are in rock, documented).
   Alternatives: the highest column (the owner's literal "dropped until its base meets the
   terrain": the box clears every column, but on a slope everything downhill of the first
   contact floats, which is the complaint); the centre column (half of each defect). A
   foundation filled under the footprint down to the terrain, which would remove the choice,
   is a stamp that writes into air only: Plan 0084's write policy, parked in the backlog until
   it lands.

### Budgets added 2026-11-19 (guesses until Phase 7d reads them)

- **The snap's casts:** at most 64 downward casts of at most 200 voxels per re-stand of a
  structure's ghost (each frame the camera moves while a structure slot is selected), on the
  client's world, guess under 0.2 ms on the dev box; nothing on the GPU, nothing kept.

## TL;DR

Today a `.vox` reaches the world only through Rust: its colours are thrown away, every
palette index needs a `material` line naming a row some Rust wrote, a 256³ model costs 36 MB
of dense prefab, and a runtime stamp fails as soon as its box touches a chunk the camera's
residency sphere does not hold. This plan makes a `.vox` **load as painted**: every palette
entry becomes a material row carrying its colour and MagicaVoxel material (glass, glow,
metal), the fly-through's material table moves to a **text file** in the `.rig` grammar,
reloaded while the window is open, prefabs become **brick-sparse**, a stamp larger than the
residency window becomes a **placement the world finishes as chunks arrive**, the scene graph's
hidden layers and reflected frames load, and the fly-through gets a **Structures page** in Plan
0080's menu that places any `.vox` of `apps/flythrough/assets/structures/` at the crosshair,
diggable, lit by the bake, collided with, as a logged op. The first thing anyone sees:
`flythrough --vox apps/flythrough/assets/structures/chapel/chapel.vox` and a chapel with blue
glass and glowing lamps stands ahead of the spawn in its own colours, with no rig and no Rust
touched; later, the Plan 0042 castle placed from the menu in the running world, its far towers
filling in as you fly toward them.

## Context & problem

- **Colours.** `VoxMap::to_prefab` refuses any used palette index the map leaves `Unset`
  (`crates/vxe-assets/src/map.rs`); the `.rig`'s `material <idx> <name>` resolves against a
  table written in Rust (`apps/flythrough/src/scene.rs`, about fifteen rows). The `.vox`'s
  `RGBA` is parsed and ignored; `MATL` (glass, emit, metal) is skipped with every other chunk.
- **The table.** `MaterialTable` rows are Rust (`cave_materials`, `castle_materials`,
  `showcase_materials`); the consumer's requirement is a data-driven table
  (`project-context.md`, Materials P0), and the owner wants artists to define materials in a
  file hot-reloaded like the `.rig`. `GpuWorld::rewrite_materials` already rewrites a row's
  render fields at runtime for a table of the same rows.
- **Size.** `Prefab` is dense: Plan 0042's castle held 343 566 464 B for 21 tiles, 92 % air;
  "nearly anything" at a hundred tiles is 3.6 GB against the 4 GB world-in-RAM budget
  (`docs/nfr.md` section 3).
- **Reach.** `World::stamp` fails with the first non-resident chunk its box meets
  (`crates/vxe-world/src/stamp.rs`), so a castle at runtime is limited to the camera's
  `r_full` sphere (about 100 m) and placed only near the camera. Plan 0042 placed its castle at
  generation time through `StampedSource`, which is not an op and is not saved.
- **The scene graph.** `VoxFile::placements` composes `nTRN`/`nGRP`/`nSHP` with the 24
  rotations (the `voxtext.rs` note on rotation is about the text export only); a reflected
  `_r` (determinant -1) is a load error; `_hidden` nodes and `LAYR` are not read, so a hidden
  layer is placed. Scale is fixed at one `.vox` voxel per 10 cm world voxel.
- **The testbed.** `--castle` is the only `.vox` scene path; it is generated, mapped by hand
  (`castle.rs`'s `above_map`, `dungeon_map`) and placed at generation. There is no way to drop
  a MagicaVoxel file in and see it. Plan 0080's menu and Plan 0077's picker (approved) give a
  dynamic page and a place-at-the-crosshair pattern to reuse.
- **Figures touched:** `docs/nfr.md` section 3 (prefab RAM, world in RAM), section 4 (stamp
  cost), section 7 (first frame with a scene, the gate's wall time), section 6 (the stamp
  vector `0x7ee7_eeb7_0956_d069` must hold through the storage change).

## Decision

We load a `.vox` as data end to end. `vxe-assets` reads `MATL`, `LAYR`, `_hidden` and
`_layer_id`, resolves every palette entry to a material row (a `material` line wins, else a
row named by colour and material, shared across files; ADR-0065), parses a material table
file in the `.rig` grammar and writes it back canonically, materialises a reflected frame as a
mirrored model with a proper rotation, and scales a model by an integer factor. `vxe-world`
stores a `Prefab` as 8³ bricks with uniform bricks collapsed, keeps the content id as the dense
content's hash, and turns a stamp that reaches non-resident chunks into a placement applied as
chunks are generated or reloaded (ADR-0066); the op, the log and the server are unchanged. The
fly-through reads its table from `apps/flythrough/assets/materials.txt` (`--materials` for
another), reloads it once a second like the viewer reloads the rig, places `--vox` scenes at
start and, from a Structures page of Plan 0080's menu, at the crosshair at runtime, with the
structure's box carved clear first by default so it stands in rock. We rejected TOML for the
file, used-only palette rows and nearest-row mapping (ADR-0065), and dense prefabs, forced
residency, voxel lists and octrees (ADR-0066). We rejected a traced preview of a structure
before it is placed: the entity pool's parts are capped at 64 voxels, so the preview is the
box's footprint on the HUD (backlog).

## Architecture diagram

```mermaid
flowchart LR
    subgraph assets["vxe-assets"]
        V[".vox: models, RGBA, MATL,<br/>LAYR, nTRN/nGRP/nSHP"] --> S["VoxScene::load<br/>(placements, stages, mirrors, scale)"]
        M["materials.txt"] --> T["MaterialFile::parse"]
        S --> P["PaletteRows: a row per entry<br/>(material line wins)"]
    end
    subgraph world["vxe-world"]
        T --> MT["MaterialTable<br/>(fixed at generate)"]
        P --> MT
        S --> PF["Prefab (bricks)"]
        PF --> ST["World::stamp<br/>resident now +<br/>Placement for later"]
        ST --> OP["Op::Stamp kind 2<br/>(log, server, replay)"]
        ST --> CH["chunks, spill seq"]
    end
    subgraph app["apps/flythrough"]
        F["--vox, --materials,<br/>Structures page"] --> S
        F --> M
        R["reload once a second"] --> T
        R --> RW["GpuWorld::rewrite_materials<br/>+ re-bake"]
    end
```

## Implementation phases

Each phase ships as its own commit. `dev` implements the dev phases in one session, Phase 7 is
the owner's window, and Phase 8 follows it. Phases 1 to 5 touch `crates/` and the `--vox` flag
only and may run before Plans 0080 and 0077 merge; Phase 6 needs both.

### Phase 1: palette rows, and the `MATL` chunk (walking skeleton)
- **Owner skill:** dev
- **What:** `vxe-assets` parses `MATL` into `VoxFile.materials` and resolves every palette entry
  of a `.vox` to a row of a `MaterialTable` under construction, so a `.rig` with no `material`
  lines builds, and its model shows its painted colours in the viewer.
- **Files touched:** `crates/vxe-assets/src/vox.rs` (`MATL`: `_type`, `_rough`, `_emit`,
  `_flux`, `_alpha`/`_trans`, `_metal` into `VoxMaterial`), new `crates/vxe-assets/src/rows.rs`
  (`PaletteRows`: a table being built, a `BTreeMap<RowKey, MaterialId>` of rows already added by
  colour and material; `resolve(file, explicit: &VoxMap) -> VoxMap`), `src/rig.rs` (`material
  <idx> auto|air|skip` arms; `Rig::build` takes a `PaletteRows`), `src/lib.rs`,
  `crates/vxe-assets/tests/vox.rs`, `tests/rig.rs`, `apps/flythrough/src/viewer.rs` (resolve
  the viewer's `.vox` at start), `apps/flythrough/src/setup.rs`.
- **Done when:**
  1. `a_matl_chunk_reads_glass_emit_and_metal` (`tests/vox.rs`, under 0.1 s): a hand-built file
     with `MATL` entries for indices 1 (`_glass`, `_trans 0.5`), 2 (`_emit 1`, `_flux 2`) and
     3 (`_metal`, `_rough 0.2`) parses to those values; an index with no `MATL` is `Diffuse`.
  2. `every_palette_entry_becomes_a_row_once` (`tests/vox.rs`, under 0.1 s): a file whose
     `RGBA` holds 255 entries resolved into an empty table adds exactly 255 rows (a palette
     where two entries share a colour and material adds 254); index 1 `#ff0000` gives albedo
     `[1, 0, 0]`, index 2 `#808080` gives `0.2158605` on each channel (the exact sRGB decode of
     128/255, to 1e-6); glass gives `TRANSLUCENT` with opacity 0.5; emit gives emission
     `albedo * 1 * 100 * k` at the default `k`; a second file with the same palette adds no
     row, and a row named `vox #ff0000` is found by name.
  3. `a_material_line_wins_over_the_palette` (`tests/rig.rs`, under 0.1 s): a rig with
     `material 7 glow_amber` over a `.vox` whose index 7 is `#ff0000` maps 7 to the table's
     `glow_amber` row and adds no `vox #ff0000` row; `material 8 air` writes air; a rig with no
     `material` lines builds with every used index mapped.
  4. In the fly-through, `--model` of `walker.rig` with its `material` lines removed shows the
     walker in its `.voxtext` colours; the HUD's entity row names the rows added
     (`rows +N`). A test over the viewer's reload step (`a_rig_without_material_lines_builds`,
     `apps/flythrough/tests/..`, no device, under 1 s) asserts the model's part voxels carry
     the palette rows' ids.

### Phase 2: the material table as a file, hot-reloaded
- **Owner skill:** dev
- **What:** `vxe-assets` parses and writes a material table file in the `.rig` grammar; the
  fly-through's table comes from `apps/flythrough/assets/materials.txt`, and a change to the
  file while the window is open rewrites the rows' render fields and re-bakes what the change
  moves.
- **Files touched:** new `crates/vxe-assets/src/materials.rs` (`MaterialFile::parse`,
  `canonical`, `MaterialFileError` with its line), `src/lib.rs`, new
  `crates/vxe-assets/tests/materials.rs`, new `apps/flythrough/assets/materials.txt` (the cave,
  castle, showcase and rig-wood rows in today's order; `scene.rs`'s constants assert the
  file's names), `apps/flythrough/src/scene.rs` (the Rust rows become a test that the file
  reproduces them), `src/setup.rs` (`--materials <path>`), `src/run/` (the reload step beside
  the viewer's: modified time once a second, `rewrite_materials`, the queue re-baking the
  resident chunks whose census holds a row whose emission changed, else nothing), `src/hud.rs`
  (the banner: `materials reloaded`, or the error with its line, or `materials: shape changed,
  restart`), `docs/flythrough.md`.
- **Done when:**
  1. `a_table_file_round_trips` (`tests/materials.rs`, under 0.1 s): the fly-through's
     `materials.txt` parses to a table equal to `showcase_materials()` row for row (id, name,
     every field), and `canonical` of that table parses back equal.
  2. `a_bad_line_names_itself` (same file): a `color #12` line fails with its line number and
     "6 hex digits"; a `flags glow` line names the unknown flag; two rows of one name fail.
  3. `a_field_change_reloads_and_a_shape_change_waits` (`apps/flythrough/tests/..`, no device,
     under 1 s): the reload step given a file with rock's albedo changed yields a rewrite of the
     same row count and marks the rock row changed; given a file with a row added, or with the
     glass row's `translucent` flag removed, yields no rewrite and the banner's `restart` words.
  4. In the window: edit `materials.txt`'s `castle glass` colour, save, see the glass change
     within two seconds; edit `ore`'s emission, see the ore's light re-bake; the banner shows
     `materials reloaded`. Reading: the re-bake's chunks and wall time to settle after an
     emission change in the showcase (`docs/plans/logs/0083.md`).

### Phase 3: prefabs as bricks
- **Owner skill:** dev
- **What:** `Prefab` stores 8³ bricks with uniform bricks collapsed; ids, rotations, stamps
  and the wire are unchanged in meaning; the castle's prefab bytes fall.
- **Files touched:** `crates/vxe-world/src/prefab.rs` (the brick grid; `get`, `rotated`,
  `encode`/`decode` of the brick form as `LOG_FORMAT` 3 with format 2 decoding; `id` streamed
  from bricks in dense canonical order; `written_count`; `encoded_len`), `src/stamp.rs` (`blit`
  per brick: a uniform written brick into a chunk brick-aligned is one `set_brick`; otherwise
  voxels), `src/wire.rs`, `src/log.rs` (`LOG_FORMAT`), `crates/vxe-world/tests/stamp.rs`,
  `tests/log.rs`, `benches/stamp.rs` (a `prefab256_castle_tile` case from a generated tile),
  `apps/flythrough/src/castle.rs` (`prefab_bytes` reads `Prefab::bytes`), `docs/nfr/03-memory.md`,
  `docs/nfr/04-edits.md`.
- **Done when:**
  1. `ids_do_not_change_with_the_storage` (`tests/stamp.rs`, under 1 s): the 16³ vault's id
     and a 64³ mixed prefab's id equal their values pinned from the dense form (the test pins
     them before the change, in the same commit's first half); the generation-time stamp
     vector still reads `0x7ee7_eeb7_0956_d069` on Linux and Windows.
  2. `a_uniform_brick_is_one_word` (`tests/stamp.rs`): a 64³ all-rock prefab holds 512 uniform
     bricks and `Prefab::bytes()` under 4 KB; a 64³ prefab with one odd voxel per brick holds
     512 full bricks and about 557 KB (512 x 1088 B); `get` agrees with the dense form on 10 000
     seeded coordinates for a seeded 24³ prefab under every `Rot`.
  3. `a_format_2_save_still_replays` (`tests/log.rs`): a log file encoded under format 2
     (bytes committed as a fixture) decodes and replays to the same hash as its format 3
     re-encoding.
  4. Readings: the castle's kept prefabs (`--castle`, printed as today) in bytes against
     343 566 464 (guess: under a third); `prefab64_mixed` within 10 % of 2.07 ms,
     `prefab64_uniform` under 1.455 ms, `prefab256_castle_tile` read, no proposal.
     **Amended 2026-11-17 (owner):** the `prefab64_mixed` bar is within 10 % of the pre-change
     base on the same box under the same lock (today 3.80 ms on the Ryzen 9 5900HS at
     26a76ef7; the tip read +1.4 % quiet at dbb59413), not of the 2.07 ms the bench read on the
     earlier tree.

### Phase 4: the scene: layers, hidden nodes, mirrors, scale, and `--vox` at start
- **Owner skill:** dev
- **What:** `VoxScene::load` turns a `.vox` into prefabs and placements honouring `LAYR`,
  `_hidden`, reflections and an integer scale; the fly-through places a scene at start through
  `StampedSource`, its box carved clear first.
- **Files touched:** `crates/vxe-assets/src/vox.rs` (`LAYR`; `_hidden` and `_layer_id` on
  transforms; `names` for layers), new `crates/vxe-assets/src/scene.rs` (`VoxScene { prefabs,
  placements, layers, bounds }`; a reflected frame mirrors the model's voxels along the
  reflected axis and composes a proper `Rot`; `scale k` repeats each voxel k³ times, k in 1..=4
  so a 256 model stays within `edit_axis_max` 1024), `src/lib.rs`, `tests/vox.rs`, new
  `tests/scene.rs`, `apps/flythrough/src/cli/flags.rs` (`--vox <path>[@x,y,z][*k]`,
  `--vox-clear on|off`, default on), `src/setup.rs` (a `StampedSource` over the cave with the
  box plus a 1 m margin carved to air when clear is on, as `HallSource` does for the castle),
  new `apps/flythrough/assets/structures/README.md` and `chapel/chapel.voxtext` + `chapel.vox`
  (three models: nave, a mirrored transept pair through one reflected frame, a tower; blue
  glass windows, two `_emit` lamps, a hidden `scaffold` layer; about 48 x 32 x 40 voxels),
  `apps/flythrough/tests/structures.rs`, `docs/flythrough.md`.
- **Done when:**
  1. `hidden_layers_and_nodes_are_not_placed` (`tests/scene.rs`, under 0.1 s): a file with
     a model on a hidden layer and one under a `_hidden 1` transform places neither; the same
     file with the flags off places both.
  2. `a_reflected_frame_loads_as_a_mirrored_model` (`tests/scene.rs`): an `_r` byte with
     determinant -1 over an L-shaped model yields a placement whose `Rot` has determinant +1
     and whose voxels, stamped, land where the reflection puts them (asserted voxel by voxel
     against a hand-mirrored copy); `VoxFile::placements` still refuses it, `VoxScene` does not.
  3. `scale_repeats_voxels` (`tests/scene.rs`): a 2 x 1 x 1 model at `*3` is a 6 x 3 x 3
     prefab whose `written_count` is 54; `*5` is refused naming the range.
  4. `the_chapel_stands_at_the_spawn` (`apps/flythrough/tests/structures.rs`, headless, no
     device, under 5 s): `--vox` of the chapel generates a world whose voxel hash equals a
     pinned value on both CI arms, with the glass row translucent and the lamp rows emissive
     in the table; the model voxels in the world are counted per row and match the `.vox`'s
     per-index counts.
  5. In the window: `flythrough --vox apps/flythrough/assets/structures/chapel/chapel.vox`
     shows the chapel ahead of the spawn in its colours, lamps lighting the nave, glass seen
     through; a carve into its wall removes voxels of `vox #...` rows and the HUD's edit result
     names them.

### Phase 5: placements that outlive residency
- **Owner skill:** dev
- **What:** `World::stamp` writes the resident chunks now and keeps a placement for the rest;
  generation and spill reload apply pending placements; `StampedSource` reads the same list.
- **Files touched:** `crates/vxe-world/src/stamp.rs` (`Placement`, the pending list on the
  store under the map's lock, `seq`; `stamp` no longer fails on a non-resident chunk;
  `StampedSource` wraps the list), `src/store.rs` (`pending_placements()`), `src/stream/`
  (the generated chunk's blit of placements meeting it), `src/persist.rs` (the spill carries
  `seq`; a spill without it reads 0), `src/source.rs`, `crates/vxe-world/tests/stamp.rs`,
  `tests/stream.rs`, `tests/persist.rs`, `crates/vxe-net/tests/` (a stamp through the server
  over a non-resident chunk is logged once and replays equal), `apps/flythrough/src/setup.rs`
  (`--vox` at start becomes a stamp op on the first tick when `--vox-op` is given, so the two
  paths can be compared), `docs/nfr/04-edits.md`.
- **Done when:**
  1. `a_stamp_past_residency_finishes_when_the_chunk_arrives` (`tests/stream.rs`, under 2 s):
     a 3-chunk-wide prefab stamped with the middle chunk not resident returns `Ok` with the
     two resident chunks' counts and one pending placement; after the streamer makes the
     middle chunk resident, the world's voxel hash equals a world where every chunk was
     resident at the stamp, and the pending count is 0.
  2. `a_spilled_chunk_reloads_with_the_placements_it_missed` (`tests/persist.rs`): a chunk
     edited, evicted (spilled), then covered by a stamp, reloads holding both the edit and the
     stamp's voxels; a chunk spilled after the stamp reloads without a second write (the
     stamp's voxels are counted once in a tally over the reload).
  3. `the_source_path_and_the_op_path_agree` (`tests/stamp.rs`): the chapel placed through
     `StampedSource` and the chapel stamped as an op on a resident world hash equal once both
     are fully resident (Plan 0008's vault test, at scene scale).
  4. `a_replayed_placement_is_one_op` (`crates/vxe-net/tests/`): a stamp over a half-resident
     world appears once in the log at kind 2, and a client replaying it with a different
     residency ends with the same hash once resident.
  5. Readings (`docs/plans/logs/0083.md`, dev reference): the castle (`--castle`'s prefabs,
     through the op path at the spawn) stamped at runtime: wall time of the call, chunks
     written now, pending placements, and the live queue's time to settle the castle's light
     after it (guess: the call under 500 ms with 92 % air skipped as uniform bricks, from 7.8
     ns a voxel in `prefab64_mixed`; the settle is the queue's budget, read, no proposal).

### Phase 6: the Structures page in the fly-through (as approved; replaced by "Phase 6 (amended)" under "Amended 2026-11-18" above)
- **Owner skill:** dev
- **What:** a dynamic Structures page of Plan 0080's menu lists the structures library and
  places the selected `.vox` at the crosshair as a logged stamp, the box cleared first by
  default; the generated castle is an entry.
- **Files touched:** `apps/flythrough/src/menu/` (the page, `Page::dynamic` as Plan 0077's
  Entities page; entries: the library's files, `castle (generated)`; actions `place`,
  `clear box on|off`, `scale 1..4`), `src/structures.rs` (the library scan at start, its
  palette rows resolved before `World::generate`; the aim point from the crosshair's hit as
  Plan 0077's picker takes it, the origin the model's minimum corner at the hit with the
  footprint centred on it and the base on the hit voxel's top), `src/hud.rs` (the banner:
  name, size in metres, rows, chunks written now, pending), `src/setup.rs`, `src/castle.rs`
  (the castle's prefabs through `VoxScene` and the palette rows instead of `above_map`, so
  `--castle` is the same path; the hall stays), `apps/flythrough/tests/structures.rs`,
  `tests/toggles.rs`, `apps/flythrough/assets/structures/README.md`, `docs/flythrough.md`.
- **Done when:**
  1. `the_library_lists_every_structure` (`tests/structures.rs`, no device, under 1 s): the
     page's entries name every directory of `assets/structures/` plus the castle; each file's
     palette resolves against the fly-through's table with the row count the README states.
  2. `a_placed_structure_is_a_logged_stamp` (`tests/structures.rs`, headless, under 5 s): the
     chapel placed at a chosen aim point appears in the run's op log as one kind 2 op whose
     origin puts the model's base on the aimed voxel's top; a replay of the log reproduces the
     world hash; with `clear box` on, a box carve precedes it in the log.
  3. `the_castle_loads_through_the_scene` (`tests/castle.rs`): `--castle`'s world hash equals
     its value before this phase (the hand maps and `VoxScene` give the same prefabs; if the
     palette rows reorder ids, the test pins the new hash and the Implementation log says so).
  4. The page shows its mode: the title `menu > structures`, the selected entry, `clear box`
     and `scale` with their values, and after a place the banner's line; `tests/toggles.rs`
     asserts the words.

### Phase 7: the owner's window (done 2026-11-18; the verdict and Phases 7b and 7c are under "Phase 7 verdict" above)
- **Owner skill:** human
- **What:** the owner makes a scene in MagicaVoxel and places it, then the castle, and judges
  colour, glow, glass, scale and the fill-in.
- **Files touched:** `docs/plans/logs/0083.md` (the owner's words, the picks under "Risks &
  open questions" answered).
- **Done when:** on Linux, the owner (a) makes anything in MagicaVoxel with a glass material, an
  emit material, a metal material, a mirrored copy of a group, a hidden layer and a rotated
  group, saves it under `assets/structures/<name>/`, restarts, finds it on the Structures page
  and places it, digs into it, looks through its glass and at its glow; (b) places the castle
  from the page in the running world and flies to its far side; (c) reloads `materials.txt`
  with a colour change while looking at it. The owner says whether the colours match
  MagicaVoxel's, picks the emission factor `k` (Phase 1's default is a guess), the default
  scale (1), whether `clear box` stays on by default, and whether the fill-in of a far
  placement reads as acceptable; the frame time and the settle after the castle's placement
  are read from the HUD.

### Phase 8: the owner's picks, the goldens, the readings, and the tip
- **Owner skill:** dev
- **What:** the window's picks become defaults, the chapel gets a golden, the readings go to
  `nfr`, the docs follow, and the gate runs at the tip.
- **Files touched:** the defaults the picks name; new `apps/flythrough/tests/gpu_structures.rs`
  (the chapel placed in the dark cave, a fixed pose, 64 frames, baked as `gpu_golden.rs`'s
  `showcase_hall` bakes; its golden under `tests/golden/`), `docs/nfr.md` sections 3 and 4
  (the castle's prefab bytes replace 343 566 464; the stamp readings), `docs/nfr/03-memory.md`,
  `docs/nfr/04-edits.md`, `docs/flythrough.md`, `CLAUDE.md` (the `vxe-assets` line: the
  material file and the scene), `apps/flythrough/assets/structures/README.md`.
- **Done when:**
  1. `chapel_golden` (`gpu_structures.rs`, lavapipe, guess 8 s; born `#[ignore = "soak"]` if
     it reads over 10 s): the pose's image within SSIM 0.98 of its blessed golden, the glass
     pixels' mean blue over red, and the lamps' pixels over 0.5 luma.
  2. The castle's prefab bytes, the stamp bench, the runtime castle's wall time and the
     settle time are in `nfr` with machine and commit; `node scripts/check-doc-links.mjs`,
     `check-comment-hygiene.mjs`, `check-index-rows.mjs` and `check-plan-size.mjs` exit 0;
     `cargo nextest run --profile gate` is green at the tip, with its wall time read.

## Data shapes

```rust
// illustrative, vxe-assets
pub enum VoxMaterialKind { Diffuse, Metal { rough: f32 }, Glass { trans: f32, rough: f32 },
                           Emit { emit: f32, flux: u8 }, Media }
pub struct VoxMaterial { pub kind: VoxMaterialKind }      // one per palette index, from MATL

pub struct PaletteRows<'t> { table: &'t mut MaterialTable, by_key: BTreeMap<RowKey, MaterialId>,
                             pub emission_factor: f32 }
impl PaletteRows<'_> {
    /// Every entry of `file`'s palette as a row; `explicit` entries kept as they are.
    pub fn resolve(&mut self, file: &VoxFile, explicit: &VoxMap) -> VoxMap;
}

pub struct MaterialFile { pub rows: Vec<Material> }         // ids 1.. in file order
impl MaterialFile { pub fn parse(text: &str) -> Result<Self, MaterialFileError>;
                    pub fn canonical(&self) -> String; }

pub struct VoxScene { pub prefabs: Vec<Arc<Prefab>>,
                      pub placements: Vec<(usize, VoxelCoord, Rot)>,  // prefab, min corner, rot
                      pub layers: Vec<(String, Vec<usize>)>,          // name, placement indices
                      pub bounds: ([i32; 3], [i32; 3]) }
```

```rust
// illustrative, vxe-world
pub struct Prefab { size: [u32; 3], bricks: Vec<PrefabBrick> }       // ceil(size/8)^3, x fastest
enum PrefabBrick { Uniform { material: MaterialId, written: bool }, Full(Box<FullBrick>) }
struct FullBrick { voxels: [MaterialId; 512], mask: [u64; 8] }      // 1088 B

pub struct Placement { prefab: Arc<Prefab>, origin: VoxelCoord, rot: Rot, seq: u64,
                       remaining: u32 }                               // chunks not yet written
```

A material file (the grammar of `rig.rs`; `#` to the end of a line is a comment):

```text
material rock
  color #6b615a
  roughness 0.9
  acoustic 0.1 0.6 0
material castle glass
  color #4059d9
  flags translucent
  opacity 0.4
material ore
  color #b87333
  emission 2.0 1.2 0.5
```

## Budgets

| Figure | `nfr.md` | Expected | How the close reads it |
|---|---|---|---|
| Prefab RAM, the castle's kept tiles | section 3 | 343 566 464 B dense; brick form a guess under a third | Printed by `--castle`, asserted in Phase 3's log, Phase 8 writes it |
| World in RAM with the castle | section 3, 4 GB | down by the prefab saving; otherwise as Plan 0042 read (+0.3 GB RSS) | `VmHWM` at `--frames 3000`, with and without `--castle`, as Plan 0042 |
| Stamp cost | section 4 | `prefab64_mixed` within 10 % of the pre-change base on the same box (amended 2026-11-17, owner; 3.80 ms today); `prefab64_uniform` under 1.455 ms; a 256³ tile read, no proposal | `cargo bench -p vxe-world --bench stamp`, dev reference, perf lock, base and tip alternated |
| Runtime castle placement | section 4 (new line) | the call under 500 ms on the dev box, a guess; pending placements, settle time read | Phase 5's rig, `docs/plans/logs/0083.md` |
| First frame with `--vox chapel` | section 7 | within 100 ms of the plain cave's | the start-up lines, three runs |
| Material rows on the GPU | section 3, everything else | 64 B a row; 1000 rows 64 KB | `GpuWorld::mirror_bytes`'s material term |
| `Wide` bricks from many-colour structures | section 3 | a reading on the castle and the owner's scene, no proposal | `--census` with and without the structure |
| Gate wall time | section 7 | plus the new CPU tests (under 15 s together) and one golden | the tip's gate log through `scripts/test-times.mjs` |

## Risks & open questions

- **The table is fixed after generate.** Every structure the fly-through may place must have
  its palette resolved before `World::generate`; the library is scanned at start, and a file
  dropped in later is listed as `new at restart`. A consumer that loads `.vox` files during
  play reserves rows ahead or restarts the world; the plan does not lift the rule (backlog: a
  row's class changed live by re-packing its bricks).
- **Emission scale.** MagicaVoxel's `_emit` and `_flux` are renderer-relative; the default
  factor is a guess the window sets. Until then the chapel's lamps may blow out or vanish under
  the auto exposure.
- **Determinism.** The storage change must keep every content id and the stamp vector; the
  brick-order hash streams the dense canonical bytes, and the test pins the ids before the
  change in the same commit. The mirror, scale and placement paths are integer; the colour
  decode is float but feeds the table, not the voxels.
- **Spill ordering.** A spill without `seq` is read as 0 and receives every placement; a chunk
  that was spilled after a placement wrote it carries `seq` past it and is not written twice.
  The test in Phase 5 covers both. A placement list that grows without bound (a game placing
  thousands of far structures) is a memory risk the pending count exposes; no cap here.
- **The far fill-in.** A placement's far chunks appear as they are generated, so a castle's
  towers pop in with the streaming edge, and their light bakes on arrival, as any chunk does.
  The owner judges it in Phase 7; a preload of a placement's chunks ahead of the view is a
  followup if it reads badly.
- **Many colours, `Wide` bricks.** A structure painted in dozens of colours per brick makes
  `Wide` bricks (16 units); Phase 8 reads the census. If the mirror grows notably, the backlog's
  per-row dedup by colour quantisation is the lever.
- **Merge seams.** `rig.rs` is edited by Plan 0079's Phase 3 (`body`, `stride`) and here
  (`material ... auto|air|skip`); `viewer.rs` by Plan 0078's Phase 2 (free on reload) and here
  (palette rows at start). Small, additive, and the "Order" below keeps them sequential.

### Owner questions (each with a recommendation)

1. **Which palette entries become rows.** Recommended: every entry of a loaded `.vox`, deduped
   by colour and material across files (ADR-0065), so a repaint shows on reload. Alternative:
   only the used entries (fewer rows, a fresh colour waits for a restart).
2. **The material file's grammar.** Recommended: the `.rig` line grammar, one parser style, no
   dependency (ADR-0065). Alternative: TOML, as the apps' settings file (a `toml` dependency in
   `vxe-assets`, a second grammar beside the rig).
3. **A large stamp past residency.** Recommended: write the resident chunks now and keep a
   placement the world finishes as chunks arrive (ADR-0066). Alternative: force the chunks
   resident, write and spill them (whole counts, a stall of seconds for a castle).
4. **Scale and the cleared box.** Recommended: one `.vox` voxel is one world voxel (10 cm) by
   default, integer up-scale 2 to 4 per placement; the fly-through carves the structure's box
   clear before placing it (`clear box` on), since the cave is rock. Alternative: a default scale
   of 2 (a MagicaVoxel voxel as 20 cm; coarser detail, half the models for a given size).

## Order

Phases 1 to 5 start once Plan 0063 and the clip-recorder chore have merged (the viewer and the
entities library this plan builds on) and run beside Plans 0078 and 0080; Phase 6 waited for
Plans 0080 and 0077 to merge (both closed 2026-11-18, merged into this lane at bab3cfca) and
was re-read against them in "Amended 2026-11-18" above; it starts on the owner's answers to
questions 5 to 8. Phase 7 can share the owner's window with Plan 0079's Phase 5. Plan 0084 (staged structures)
starts after this plan closes. The plan holds nothing of M10: Plan 0069 and then 0032 run as
the owner set them (after Plans 0062 and 0063), beside this one. Plan 0082 (entity part scale,
in a parallel draft) scales an entity's parts finer than a world voxel; this plan's scale is an
integer up-scale of a world structure; neither reads the other's code.

## What this plan does NOT do

- **Staged structures**, a stamp with a write policy and a region, a reveal over ticks and a
  demolish: [Plan 0084](0084-staged-structures-stages-from-a-vox-a-stamp-with-a-write-policy-and-a-region-a-reveal-and-a-demolish.md)
  and ADR-0067. This plan reads layers and keeps them on `VoxScene` for it.
- **Down-scaling** a model (a majority vote per 2³) and a non-integer scale (backlog).
- **A traced preview** of a structure before placing it (backlog); the HUD shows its box.
- **A row's class changed live** (stone to glass without a restart) (backlog).
- **MagicaVoxel animation frames** (`nTRN` frames past 0), cameras (`rCAM`), render objects
  (`rOBJ`) and palette notes (`NOTE`): skipped as today.
- **A general asset pipeline** or an editor: the library is a directory, the file is text.
- **The consumer's material file**: the fly-through's `materials.txt` is the testbed's; a
  game writes its own and loads it through `MaterialFile::parse` (the render example of Plan
  0071 may show it later).

## Implementation log

> Written by `dev`: one row per phase as its commit lands, then the close block. **The phases
> above are the contract. This section records what happened.** Observations, never conclusions:
> no pass list, no self-assessment. Deviations and unmet done-whens are **always** disclosed.
> Readings, gate counts, attempts, sweeps and resume notes go to the readings log, linked below.

Readings, gate counts and attempts: [`logs/0083.md`](logs/0083.md).

| phase | owner | state | commit |
|---|---|---|---|
| 1: palette rows and `MATL` | dev | done | d853afc3 |
| 2: the material file, hot-reloaded | dev | done | 26a76ef7 |
| 3: prefabs as bricks | dev | done; `prefab64_mixed` +1.4 % over the base, met under the 2026-11-17 amendment (Notes) | a8a53e7a |
| 4: the scene: layers, mirrors, scale, `--vox` | dev | done | a221e764 |
| 5: placements past residency | dev | done | 7a7a6f19 |
| 6 (amended): a structure in the belt | dev | done; deviations in Notes | c83ebcf8 |
| 7: the owner's window | human | done 2026-11-18 (83a cave, 83b outdoor); verdict in "Phase 7 verdict" | fd650c02, 0e85862a (fixes between 83a and 83b) |
| 7b: the coarse ghost, the spider's rows, the docs | dev | done; deviations in Notes (the spider takes belt slot 1) | 4c5b5616, 6a457a07, a4dec4c2, d8c8b284 |
| 7c: the owner looks at the ghost | human | done 2026-11-19 (83c, no mouse): the ghost passes; verdict in "Phase 7c verdict" | |
| 7d: the snap, the free mode, the swap, `E`, the walker first, the Esc cancel | dev | done after merge 49c5cbd3; deviations (columns start 0.5 over `p`, not 10 m; the 600 box's cap) in the log | 11f3dbe8 |
| 7e: the owner looks at the snap and the free mode (83d) | human | done 2026-11-19: "its better"; the lowest column stands; the scale limit owed to Phase 8 | |
| 8: picks, goldens, readings, tip | dev | done; the settle after the runtime castle not read; the golden on the goldens' floor, not SSIM (Notes) | 9f06b84e |

### Notes

- Phase 1: `Rig::build` takes `&mut PaletteRows`; `Rig::build_mapped` (a palette closure) and
  `Rig::explicit_map` are added beside it, and the viewer's reload resolves against the fixed
  table through `resolve_existing` (by row name).
- Phase 1: a glass row's name carries its roughness too (`vox #rrggbb glass 0.5 0.1`), a metal
  row's its roughness (`metal 0.2`); `_media`/`_cloud` rows are `media <trans>`.
- Phase 1: `VoxFile` drops `Eq` (its `materials` hold `f32`) and gains `Default`; `encode` writes
  `MATL` (`write.rs`, outside the phase's list) so the round trip holds.
- Phase 1: `DEFAULT_EMISSION_FACTOR` is 0.3 (a guess for Phase 7). The palette rows of `--model`'s
  `.vox` are added in `setup.rs` (`world_table`) and `app.rs` sets the viewer's `rows +N`.
- Phase 1 done-when 4's window half (the walker in its colours on screen) waits for Phase 7.
- Phase 2: each scene takes the file's rows by name in its own order (`materials::Selection`):
  the castle's, the showcase's and the rigs' rows all follow the cave's at id 5, so one table
  of the whole file would move ids. `materials.txt` holds the showcase's table as its first seven
  rows (done-when 1 compares those), then the castle's and `rig wood`.
- Phase 2: the composition, the reload step and the watch are a new
  `apps/flythrough/src/materials.rs`; the run's poll is in `run/frame.rs`; `--materials` is in
  `cli/model.rs` (`flags.rs` is at its 1200-line cap); the banner is the lights' banner row, not
  `hud.rs`. The Rust table functions in `scene.rs` stay (tests use them) and a unit test asserts
  the file reproduces each. A missing default file falls back to a built-in copy, not watched.
- Phase 2: a row's class for the reload is its flags and whether it emits; a `crisp` change
  also waits for a restart.
- Phase 2 done-when 4 (the window, and the reading of the re-bake after an emission change)
  waits for Phase 7.
- Phase 3: a brick is a `u32` word (a material and a written bit, or the index of a stored
  1088-byte brick) rather than the plan's `PrefabBrick` enum, so a uniform brick is 4 B; the id
  is computed once and kept. `Prefab::from_fn`, `grid`, `brick_counts`, `bytes` and
  `decode_dense` (format 2) are new. `wire.rs` is unchanged (the wire names prefabs by id).
- Phase 3: `castle::prefab_bytes` reads `Prefab::bytes`; `dense_prefab_bytes` keeps the old sum
  for the soak test and the start-up line. `prefab256_castle_tile` is a castle-shaped 256³
  tile built in the bench (vxe-world cannot reach the fly-through's generator).
- Phase 4: `VoxFile` gains `reflected` (a reflected frame's `_r` byte; the node's `rot` holds the
  reflection times a mirror of x), `hidden`, `node_layers` and `layers`; `parse` accepts a
  reflection, `try_placements` refuses it (`placements` stops before it), `decode_frame` is new
  and `decode_rotation` still refuses. `encode` writes the new fields; files without them encode
  byte for byte as before.
- Phase 4: `VoxScene::load(file, map, SceneOptions { scale, honour_hidden })`; the plan's
  "flags off" is `honour_hidden: false`. A reflected placement mirrors its model along x and
  composes the proper `Rot`. `VoxScene::placed_at` and `size` are added.
- Phase 4: `--vox` and `--vox-clear` are in `cli/model.rs`, not `cli/flags.rs` (at its cap); the
  structure code is a new `apps/flythrough/src/structures.rs` (the plan names it in Phase 6).
  `--vox` places in the streamed cave only. The carve covers the box and 1 m on its sides and
  above, not below, so the base stands on what is under it; the default corner is 3 m ahead of
  the spawn (+x), centred in y, the base 1.6 m below the spawn. Index 0 (empty cells) is `skip`.
- Phase 4: `chapel.vox` is `chapel.voxtext` (models only) plus `structures::chapel_vox` (`MATL`,
  layers, the reflected transept); `the_chapel_assets_are_in_step` keeps the two equal.
- Phase 4 done-when 5 (the chapel in the window) waits for Phase 7.
- Phase 5: a placement keeps the set of chunks it still has to write, not a `seq`; an inserted
  chunk takes every pending placement that lists it, in stamp order, under the store's write
  lock (the stamp keeps its placement under the map's read lock). The spill format is unchanged
  (no `seq` word): a placement lists a spilled chunk until it is reloaded and written once.
  `persist.rs`, `source.rs` and `StampedSource` are unchanged.
- Phase 5: a chunk a placement writes into is marked edited, so it is persisted, not
  regenerated, when it leaves; a LOD-only load builds its mips without the placement (the far
  ring shows the structure once the chunk is full). `EditResult::aabb` stays the bounds of the
  voxels changed now, not the whole box. The worker's `Prepared` result of a chunk a placement
  changed is dropped from the poll's events.
- Phase 5: `ChunkStore::pending_placements` and `placement_writes` are new. A bounded world's
  stamp past its extent now writes the resident part and keeps the rest pending; the vxe-world
  test `stamp_outside_the_extent_...` and the fly-through's `p_stamps_the_vault_at_the_aim_point`
  (`edits.rs`, outside the phase's list) assert that now.
- Phase 5: `--vox-op` submits the box carve and the stamps on the first tick every chunk of the
  carved box is resident (the carve is an edit, which a missing chunk refuses);
  `the_chapel_as_ops_equals_the_chapel_from_the_source` (`tests/structures.rs`) checks the
  chapel's two paths, and the vxe-world `the_source_path_and_the_op_path_agree` a synthetic
  scene (vxe-world's tests cannot read the fly-through's assets).

- Fix dbb59413: `prefab64_mixed` read 15 % over the dense-form base (quiet A/B at 764f0d18); the
  stamp now reads a prefab brick once per sub-box instead of `Prefab::cell` per voxel (+1.4 %
  in the quiet A/B at dbb59413). Under the 2026-11-17 amendment (0ef2b8fe, owner: within 10 %
  of the pre-change base on the same box and lock, 3.80 ms) that reading meets Phase 3's bar.

- Phase 6: `VoxScene::placed_at(by, quarters)` takes the quarter turns (callers pass 0);
  `VoxScene::turned_bounds` is new; `structures::clear_box` and `structures::ops` take the
  quarter turns, and `VoxOps` loses `chunks` (no residency wait).
- Phase 6: `Prefab::id` streams runs of air as one multiplication by the prime's power and a
  uniform line along x at once (the same value; readings log). `VoxScene`'s prefab at scale 1
  is built dense through `Prefab::new` (the castle's scene 20 s to 4.4 s in a test build).
- Phase 6: the castle entry maps through `structures::castle_map`, the castle rows by name
  (under `--castle` that is `castle::dungeon_map`); `world_table` appends the material file's
  seven castle rows when the table has none, so the castle places in its colours in the cave.
  Done-when 7 loads the castle under `dungeon_map`: `above_map` alone refuses the dungeon
  tiles' index 8. The castle places at scale 1 only, and a box past the edit limit is refused.
- Phase 6: the `structures` group lists its toggles before its action (the menu's rule), so
  `choose a structure` is its third entry, not its first.
- Phase 6: the frame's row is the first opaque written voxel's row of the scene's first
  prefab, not the most-used row.
- Phase 6: `R` on a structure's slot adds four 22.5-degree steps; the quarter turns are
  `turn / 4`.
- Phase 6: files outside the list: `structures/{library,words}.rs` (the split),
  `hud/modes_tests.rs` (`modes.rs`' tests moved out, the file was at 1199 lines),
  `entities/pool.rs` (the frame's upload and free), `run/mod.rs` and `test_support.rs` (the
  `structure_placed` field, the test module), `cli/{mod,parse}.rs` (the `structures_dir`
  field), and for the changed API `run/picker_tests.rs`, `run/tests.rs` and
  `tests/gpu_hud_golden.rs` (`Item`, `append_belt(&Modes)`, `Belt::fill(entities,
  structures)`).
- Phase 6: 0077's `the_belt_draws_its_slots` is unchanged (its run has no structures
  library); the chapel and castle slots and the chapel's icon are asserted in
  `the_belt_names_a_structure` (1.2 s: it waits for the icon).
- Phase 6: the HUD's mode block gains a row and the menu's root a thirteenth group, so
  `gpu_hud_golden`'s `hud.png` (24 953 bytes) and `menu.png` (12 358 bytes) differed at the tip
  gate; both re-blessed on lavapipe after reading them: the `clear box on, structure scale 1`
  row inserted under the lights row (the rows below it one line down), and `structures` after
  `entities` on the root page.
- Phase 7b: done-when 4's "slot 2" read slot 1 (the spider led by name); Phase 7d reverses it:
  the walker first, `assert_spider_builds` asserts walker 1, spider 2.
- Phase 7b: a body over `FRAME_BODY_VOXELS` (2^20 box voxels) is made on a worker of its own,
  not the icons'; the ghost's key gains whether the body is in.
- Phase 7b: k follows the formula and done-when 3 (k is the scale at 1 to 3), not the
  mechanism's "k 1 at scales 1 to 3".
- Phase 7b: `Model::reach` public, `MAX_COARSE` exported, every model id moved (coarse byte).
- Phase 7b and 8: files outside the list and the rest: readings log, per phase.

- Phase 8: golden on the goldens' floor, not SSIM; settle not read; `CLAUDE.md` not edited (log).

### Close triggers

_(Facts for the architect. No recommendations, and no suggested version bump.)_

- **What shipped:** feature. **Public API changed:** `vxe-assets` rows, material file, scene, writer; `vxe-world` brick
  `Prefab`, pending placements; `vxe-entity` `Model::reach`, `MAX_COARSE`. Readings: `nfr` 3, 4
- **Gate at the tip (9f06b84e):** 1228 passed, 134 skipped, 1176 s, load 24 at the start; `cargo
  deny` advisories fail (wasmtime 49.0.5). Readings log, "Tip gate after Phase 8"
- **Outstanding `human` phases:** none

## Followups

- A preload of a placement's chunks ahead of the view, if the far fill-in reads badly in Phase 7.
