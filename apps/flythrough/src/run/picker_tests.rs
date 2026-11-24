//! The run's tests of the chooser and placement mode (Plan 0077 Phases 3 and 5): the wheel,
//! the digits and the pad on the belt, the chooser's hand-over, where the ghost stands, the
//! clicks, the remove key, the one panel, the caps and the tester's filter. A rock box with
//! the walker's rows around the camera, the committed library scanned against the cave's table
//! (the walker builds; the spider is in error, since the bare `cave_materials()` lacks the
//! showcase's rows it names), unless a test names another. A run's own table holds those rows
//! (`setup::world_table`), so there the spider builds on every source.

use super::*;
use crate::entities::belt::{self, BELT_ROWS, Item};
use crate::entities::library;
use crate::hud::menu::{Activated, PageId, Row};
use crate::run::hud_feed::Overlay;
use crate::test_support::bare_run;
use svet_app::{Axis, Flow, Input, KeyCode, MouseButton, PadButton};
use svet_world::{ChunkExtent, EditOp, FIXED_ONE, Fixed3, MaterialTable, Mode, OpResult, World};

/// A rock box of `table` hollow over `[-8, x_end) x [-30, 30) x [-8, 30)`: the floor's top at
/// z = -8, the far wall's face at x = `x_end`.
fn rock_box(table: &MaterialTable, x_end: i64) -> World {
    let world = World::generate(
        ChunkExtent::new([-1, -1, -1], [2, 1, 1]),
        &svet_world::UniformSource(crate::cave::ROCK),
        table,
    );
    world
        .apply(&EditOp::box_(
            Fixed3([-8, -30, -8].map(|v: i64| v * FIXED_ONE)),
            Fixed3([x_end, 30, 30].map(|v: i64| v * FIXED_ONE)),
            Mode::Carve,
        ))
        .expect("resident");
    world
}

/// The cave's table with the walker's rows.
fn cave_table() -> MaterialTable {
    crate::walker::with_walker(&crate::cave_materials())
}

/// A run in `world` with the committed library built against its table.
fn run_in(world: World) -> Run {
    let mut run = bare_run(world);
    let lib = library::scan(std::path::Path::new(library::DEFAULT_DIR));
    run.modes.walkers = crate::entities::Walkers::default().with_library(lib);
    run.poll_viewer(0.0);
    run
}

/// A run in a 60-voxel rock box of the cave's table: the camera at 0.5 looks along +x at the
/// far wall, 59.5 voxels away.
fn picker_run() -> Run {
    run_in(rock_box(&cave_table(), 60))
}

/// One frame's input step: the key step, then interactive flight with what passed; the
/// key step's exit.
fn step(run: &mut Run, input: &Input) -> Option<Flow> {
    let (pass, exit) = run.key_step(input);
    if exit.is_none() {
        run.flight_step(pass.input(), 1.0 / 60.0);
    }
    exit
}

fn frame_keys(run: &mut Run, input: &Input) {
    assert!(step(run, input).is_none(), "the run goes on");
}

fn pressed(k: KeyCode) -> Input {
    let mut i = Input::default();
    i.key_down(k);
    i
}

fn pad(b: PadButton) -> Input {
    let mut i = Input::default();
    i.pad_down(b);
    i
}

/// One wheel line toward the user: the belt's next slot.
fn wheel_down() -> Input {
    let mut i = Input::default();
    i.add_scroll(-1.0);
    i
}

fn click(b: MouseButton) -> Input {
    let mut i = Input::default();
    i.button_down(b);
    i
}

/// The notice row's words, when one is shown.
fn notice(run: &Run) -> Option<&str> {
    run.notice.as_ref().map(|(l, _)| l.as_str())
}

/// The library index of the entry named `name`.
fn entry(run: &Run, name: &str) -> usize {
    let lib = &run.modes.walkers.library;
    lib.entries
        .iter()
        .position(|e| e.name == name)
        .unwrap_or_else(|| panic!("{name} in the library"))
}

/// The edit ops one server tick of `run` applied.
fn tick_ops(run: &mut Run) -> usize {
    let net = run.net.as_mut().unwrap();
    assert_eq!(net.server.tick().logic_error, None);
    crate::test_support::applied(&net.client.poll())
        .into_iter()
        .filter(|r| matches!(r, OpResult::Edit(_)))
        .count()
}

/// The camera at `at`, `pitch_deg` above the horizon, looking along +x.
fn look(run: &mut Run, at: [f32; 3], pitch_deg: f32) {
    run.fly.camera.position = svet_render::glam::Vec3::from_array(at);
    run.fly.camera.yaw = 0.0;
    run.fly.camera.pitch = pitch_deg.to_radians();
}

/// Phase 5 done-when, amended by Phase 9 (1): closed, one wheel step enters the mode at slot
/// 1 with the walker's ghost standing and the dig radius unchanged; a second selects slot 2
/// (the spider, in error under the cave's table: no ghost, its reason in the notice); eight
/// more wrap to slot 1; `3` selects the empty slot 3 (no ghost, the name row names it). In the
/// mode `4` selects slot 4 and removes nothing, `1` and `6` select slots 1 and 6 and drop and
/// fire nothing, and `Delete` removes the aimed placed entity. `Esc` ends the mode and the run
/// goes on. Out of the mode `2 3 5 7 8 9` leave the run as it was, `4` removes the newest (its
/// bind's old function), `1` drops a light and `6` fires one; the wheel still enters the mode.
/// A second `Esc` is the quit. No action is `placement mode`.
#[test]
fn the_wheel_and_the_digits_enter_the_mode() {
    let mut run = picker_run();
    let (walker, spider) = (entry(&run, "walker"), entry(&run, "spider"));
    let radius = run.modes.dig_radius;
    frame_keys(&mut run, &wheel_down());
    let w = &run.modes.walkers;
    assert!(w.belt.on);
    assert_eq!((w.belt.selected, w.belt.entry()), (0, Some(walker)));
    assert_eq!(w.preview.as_ref().and_then(|p| p.entry), Some(walker));
    assert_eq!(run.modes.dig_radius, radius);

    frame_keys(&mut run, &wheel_down());
    let w = &run.modes.walkers;
    assert_eq!((w.belt.selected, w.belt.entry()), (1, Some(spider)));
    assert!(w.preview.is_none() && w.set.is_empty());
    let words = notice(&run).expect("the refusal");
    assert!(
        words.starts_with("spider: ") && words.contains("crystal"),
        "{words}"
    );
    for _ in 0..8 {
        frame_keys(&mut run, &wheel_down());
    }
    assert_eq!(run.modes.walkers.belt.selected, 0);
    assert!(run.modes.walkers.preview.is_some());

    frame_keys(&mut run, &pressed(KeyCode::Digit3));
    let w = &run.modes.walkers;
    assert_eq!((w.belt.selected, w.belt.entry()), (2, None));
    assert!(w.preview.is_none() && w.set.is_empty());
    let (mut text, mut colours) = (String::new(), Vec::new());
    let now = w.belt.changed_at.expect("a change");
    belt::append_belt(
        &mut text,
        &mut colours,
        (80, 45),
        &run.modes,
        now,
        crate::hud::BANNER_MS,
        &mut belt::Paint::default(),
    );
    let name_row = text.lines().nth(45 - BELT_ROWS).expect("the name row");
    assert_eq!(name_row.trim(), "slot 3 empty  (choose with `)");

    // In the mode every digit is the belt's: `4`, `1` and `6` too.
    let ids = three_in_a_row(&mut run);
    let newest = |run: &Run| run.modes.walkers.newest.len();
    let placed = |run: &Run| {
        let w = &run.modes.walkers;
        w.placed.iter().map(|p| p.id).collect::<Vec<_>>()
    };
    for (key, slot) in [
        (KeyCode::Digit4, 3),
        (KeyCode::Digit1, 0),
        (KeyCode::Digit6, 5),
    ] {
        frame_keys(&mut run, &pressed(key));
        let b = &run.modes.walkers.belt;
        assert!(b.on, "{key:?}");
        assert_eq!(b.selected, slot, "{key:?}");
        assert_eq!(placed(&run), ids, "{key:?} removes nothing");
        assert_eq!(newest(&run), 3, "{key:?} drops and fires nothing");
    }
    frame_keys(&mut run, &pressed(KeyCode::Delete));
    assert_eq!(placed(&run), [ids[0], ids[2]]);

    assert_eq!(step(&mut run, &pressed(KeyCode::Escape)), None);
    let w = &run.modes.walkers;
    assert!(!w.belt.on && w.preview.is_none());
    assert_eq!(w.set.len(), 2);

    // Out of the mode no digit enters it; `1`, `4` and `6` keep their binds.
    for key in [2, 3, 5, 7, 8, 9].map(|d| crate::toggles::BELT_KEYS[d - 1]) {
        frame_keys(&mut run, &pressed(key));
        let w = &run.modes.walkers;
        assert!(!w.belt.on && w.preview.is_none(), "{key:?}");
        assert_eq!((w.set.len(), newest(&run)), (2, 2), "{key:?}");
        assert_eq!(placed(&run), [ids[0], ids[2]], "{key:?}");
    }
    frame_keys(&mut run, &pressed(KeyCode::Digit4));
    assert_eq!(placed(&run), [ids[0]], "`4` removes the newest");
    frame_keys(&mut run, &pressed(KeyCode::Digit1));
    assert_eq!(run.modes.walkers.newest.last(), Some(&false), "`1` drops");
    assert_eq!(newest(&run), 2);
    frame_keys(&mut run, &pressed(KeyCode::Digit6));
    assert_eq!(newest(&run), 3, "`6` fires");
    assert!(!run.modes.walkers.belt.on);
    frame_keys(&mut run, &wheel_down());
    assert!(run.modes.walkers.belt.on, "the wheel enters");
    assert!(
        crate::toggles::ACTIONS
            .iter()
            .all(|a| a.label != "placement mode")
    );

    assert_eq!(step(&mut run, &pressed(KeyCode::Escape)), None);
    assert_eq!(
        step(&mut run, &pressed(KeyCode::Escape)),
        None,
        "asks first"
    );
    assert_eq!(step(&mut run, &pressed(KeyCode::Escape)), Some(Flow::Exit));
}

/// The chooser's overlay at 1280 x 720 (80 x 45 cells at the menu's scale 2): the rows
/// `Run::place_menu` writes with the belt `Run::chooser_belt` appends under them, the menu's
/// backdrop box and the belt's.
fn chooser_overlay(run: &mut Run) -> (Vec<String>, svet_render::HudBox, svet_render::HudBox) {
    run.write_menu();
    let menu = run.place_menu([1280, 720]);
    let belt = run
        .chooser_belt([1280, 720])
        .expect("the belt under the chooser");
    let rows = run.hud.overlay.split('\n').map(str::to_owned).collect();
    (rows, menu, belt)
}

/// Asserts the chooser's overlay ends in the belt's seven rows with the frame on slot `slot`
/// (0-based), the menu's backdrop ending above the belt's name row.
fn assert_belt_under(run: &mut Run, slot: usize) {
    let (rows, menu, belt) = chooser_overlay(run);
    assert_eq!(rows.len(), 45);
    let first = 45 - BELT_ROWS;
    let top = format!("{}+------+", " ".repeat(4 + belt::SLOT_COLS * slot));
    assert_eq!(rows[first + 1], top, "slot {}", slot + 1);
    assert_eq!(rows[first + 6], top);
    let digits: String = (1..=9).map(|d| format!(" {d}      ")).collect();
    assert_eq!(
        rows[first + 2].replace('|', " ").trim_end(),
        format!("    {}", digits.trim_end())
    );
    assert!(
        menu.row + menu.rows <= first as u32,
        "{menu:?} over the belt"
    );
    assert_eq!(
        belt,
        svet_render::HudBox {
            col: 3,
            row: first as u32 + 1,
            cols: 74,
            rows: 6
        }
    );
}

/// Phase 5 done-when, amended by Phase 9 (2): `` ` `` opens the chooser on the walker, the
/// belt drawn under the panel with its frame on slot 1; `Down` to the spider, `2` with the
/// page open puts the spider in slot 2, moves the frame there, and the page stays; `5` puts
/// it in slot 5 (out of slot 2) and the frame follows; `Enter` on the walker selects the slot
/// that holds it, closes the page and enters the mode with the walker's ghost; `` ` `` in the
/// mode opens the chooser again with the belt under it, the mode and the ghost gone, and
/// `Enter` hands back with the ghost.
#[test]
fn the_chooser_fills_the_belt_and_hands_over() {
    let mut run = picker_run();
    let (walker, spider) = (entry(&run, "walker"), entry(&run, "spider"));
    frame_keys(&mut run, &pressed(KeyCode::Backquote));
    assert_eq!(run.menu.page(), Some(PageId::Entities));
    assert_belt_under(&mut run, 0);
    let v = run.run_view();
    let row = |run: &Run| run.menu.selected_row(&run.modes, &v);
    assert!(matches!(row(&run), Some(Row::Item(i)) if i == Item::Entity(walker)));
    frame_keys(&mut run, &pressed(KeyCode::ArrowDown));
    assert!(matches!(row(&run), Some(Row::Item(i)) if i == Item::Entity(spider)));
    frame_keys(&mut run, &pressed(KeyCode::Digit2));
    assert_eq!(run.modes.walkers.belt.slots[1], Some(Item::Entity(spider)));
    assert_eq!(run.modes.walkers.belt.selected, 1);
    assert_eq!(run.menu.page(), Some(PageId::Entities));
    assert_eq!(notice(&run), Some("spider in slot 2"));
    assert_belt_under(&mut run, 1);
    frame_keys(&mut run, &pressed(KeyCode::Digit5));
    let b = &run.modes.walkers.belt;
    assert_eq!(
        (b.slots[1], b.slots[4], b.selected),
        (None, Some(Item::Entity(spider)), 4)
    );
    assert_eq!(run.menu.filter(), "");
    assert!(!b.on && run.modes.walkers.preview.is_none());
    assert_belt_under(&mut run, 4);

    // keys:typed-begin
    frame_keys(&mut run, &pressed(KeyCode::ArrowUp));
    // keys:typed-end
    frame_keys(&mut run, &pressed(KeyCode::Enter));
    assert!(!run.menu.is_open());
    let w = &run.modes.walkers;
    assert!(w.belt.on);
    assert_eq!((w.belt.selected, w.belt.entry()), (0, Some(walker)));
    assert_eq!(w.preview.as_ref().and_then(|p| p.entry), Some(walker));
    assert_eq!(run.chooser_belt([1280, 720]), None, "closed");

    frame_keys(&mut run, &pressed(KeyCode::Backquote));
    assert_eq!(run.menu.page(), Some(PageId::Entities));
    let w = &run.modes.walkers;
    assert!(!w.belt.on && w.preview.is_none() && w.set.is_empty());
    assert_belt_under(&mut run, 0);
    frame_keys(&mut run, &pressed(KeyCode::Enter));
    let w = &run.modes.walkers;
    assert!(w.belt.on && !run.menu.is_open());
    assert_eq!(w.preview.as_ref().and_then(|p| p.entry), Some(walker));
}

/// Done-when 4 of Plan 0083 Phase 7d: with the structures library in the belt (walker,
/// spider, chapel, castle), the entities chooser's cursor on the walker and `4` swaps it with
/// the castle (slot 4 walker, slot 1 castle), the notice says both moves, the frame is on
/// slot 4 and the castle's chooser row reads `slot 1`; `1` with the walker under the cursor
/// swaps them back.
#[test]
fn the_chooser_swaps_an_occupied_slot() {
    use crate::structures as st;
    use std::path::Path;
    let mut table = cave_table();
    st::add_library_rows(Path::new(st::LIBRARY), None, &mut table);
    let mut run = run_in(rock_box(&table, 60));
    run.modes.structures = st::Library::load(Path::new(st::LIBRARY), &table, &cave_table());
    let walker = entry(&run, "walker");
    let castle = run
        .modes
        .structures
        .entries
        .iter()
        .position(|e| e.name == "castle")
        .expect("the castle");
    run.fill_belt();
    assert_eq!(
        run.modes.walkers.belt.slots[3],
        Some(Item::Structure(castle))
    );
    frame_keys(&mut run, &pressed(KeyCode::Backquote));
    let v = run.run_view();
    assert!(matches!(
        run.menu.selected_row(&run.modes, &v),
        Some(Row::Item(i)) if i == Item::Entity(walker)
    ));
    frame_keys(&mut run, &pressed(KeyCode::Digit4));
    let b = &run.modes.walkers.belt;
    assert_eq!(
        (b.slots[0], b.slots[3], b.selected),
        (Some(Item::Structure(castle)), Some(Item::Entity(walker)), 3)
    );
    assert_eq!(notice(&run), Some("slot 4 walker, slot 1 castle"));
    assert_eq!(run.menu.page(), Some(PageId::Entities));
    let mut rows = String::new();
    st::words::write_item_rows(&run.modes, castle, &mut rows);
    assert!(rows.ends_with("   slot 1"), "{rows}");
    frame_keys(&mut run, &pressed(KeyCode::Digit1));
    let b = &run.modes.walkers.belt;
    assert_eq!(
        (b.slots[0], b.slots[3], b.selected),
        (Some(Item::Entity(walker)), Some(Item::Structure(castle)), 0)
    );
    assert_eq!(notice(&run), Some("slot 1 walker, slot 4 castle"));
}

/// Phase 9 (4): with `--model` naming the library's own walker rig the library holds one
/// walker entry (`--model`'s, flagged): the belt is the walker in slot 1, the spider in slot
/// 2, slot 3 empty, and the chooser lists two entries.
#[test]
fn a_model_rig_from_the_library_fills_one_slot() {
    let mut run = bare_run(rock_box(&cave_table(), 60));
    let dir = std::path::Path::new(library::DEFAULT_DIR);
    let rig = dir.join("walker/walker.rig");
    let viewer = crate::viewer::Viewer::open(&rig, None).expect("opens");
    let lib = library::scan(dir).with_model(viewer);
    run.modes.walkers = crate::entities::Walkers::default().with_library(lib);
    run.poll_viewer(0.0);
    let (walker, spider) = (entry(&run, "walker"), entry(&run, "spider"));
    assert!(run.modes.walkers.library.entries[walker].flagged);
    frame_keys(&mut run, &pressed(KeyCode::Backquote));
    let b = &run.modes.walkers.belt;
    assert_eq!(
        b.slots[..3],
        [Some(Item::Entity(walker)), Some(Item::Entity(spider)), None]
    );
    let v = run.run_view();
    let items = run.menu.rows(&run.modes, &v);
    let items: Vec<Item> = items
        .iter()
        .filter_map(|r| match r {
            Row::Item(i) => Some(*i),
            _ => None,
        })
        .collect();
    assert_eq!(items, [Item::Entity(walker), Item::Entity(spider)]);
}

/// The ghost's feet with the camera at `at`, `pitch_deg` above the horizon along +x, in
/// placement mode at the walker's slot (or the slot holding `name`).
fn ghost_feet(run: &mut Run, name: &str, at: [f32; 3], pitch_deg: f32) -> [f32; 3] {
    look(run, at, pitch_deg);
    let i = entry(run, name);
    run.fill_belt();
    let s = run
        .modes
        .walkers
        .belt
        .slot_of(Item::Entity(i))
        .expect("a slot");
    enter_slot(run, s);
    run.modes.walkers.preview.as_ref().expect("a ghost").feet
}

/// Placement mode at slot `s` (0-based): a wheel step in when out of it, then the slot's
/// digit.
fn enter_slot(run: &mut Run, s: usize) {
    if !run.modes.walkers.belt.on {
        frame_keys(run, &wheel_down());
    }
    frame_keys(run, &pressed(crate::toggles::BELT_KEYS[s]));
}

/// Phase 5 done-when: the camera 1.7 m over the floor looking 40 degrees down (the floor hit
/// about 2.6 m out): the walker's feet on the floor at the view's 3.0 m point, dropped down,
/// within one voxel; the spider's (showcase table) at 1.5 times its longest extent (its
/// rest pose's 6.9 m, so 10.35 m); aimed level at a wall 6 m out, the walker's feet on the floor under the hit moved back by half its
/// footprint; aimed into an empty world with no hit within 30 m, at 3.0 m along the view.
#[test]
fn the_ghost_stands_by_the_rule() {
    let a = 40f32.to_radians();
    let mut run = picker_run();
    let feet = ghost_feet(&mut run, "walker", [0.0, 0.0, 9.0], -40.0);
    let want = [30.0 * a.cos(), 0.0, -8.0];
    assert!(
        (0..3).all(|k| (feet[k] - want[k]).abs() <= 1.0),
        "{feet:?} {want:?}"
    );
    let ex = run
        .modes
        .walkers
        .extent_of(Some(entry(&run, "walker")), &cave_table());
    assert!(ex[2] > ex[0].max(ex[1]) && ex[2] * 1.5 < 30.0, "{ex:?}");
    run.modes.walkers.belt.end();
    let feet = ghost_feet(&mut run, "walker", [0.0, 0.0, 9.0], 0.0);
    let want = [60.0 - 0.5 * ex[0].max(ex[1]), 0.0, -8.0];
    assert!(
        (0..3).all(|k| (feet[k] - want[k]).abs() <= 1.0),
        "{feet:?} {want:?}"
    );

    let mut run = run_in(World::new(&cave_table()));
    let feet = ghost_feet(&mut run, "walker", [0.0, 0.0, 9.0], 20.0);
    let b = 20f32.to_radians();
    let want = [30.0 * b.cos(), 0.0, 9.0 + 30.0 * b.sin()];
    assert!(
        (0..3).all(|k| (feet[k] - want[k]).abs() < 1e-3),
        "{feet:?} {want:?}"
    );

    let table = crate::walker::with_walker(&crate::showcase_materials());
    let mut run = run_in(rock_box(&table, 120));
    let spider = entry(&run, "spider");
    assert!(run.modes.walkers.library.entries[spider].error().is_none());
    let ex = run.modes.walkers.extent_of(Some(spider), &table);
    let near = 1.5 * ex[0].max(ex[1]).max(ex[2]);
    assert!((near - 103.5).abs() < 1.0, "{ex:?}");
    let feet = ghost_feet(&mut run, "spider", [0.0, 0.0, 9.0], -40.0);
    let want = [near * a.cos(), 0.0, -8.0];
    assert!(
        (0..3).all(|k| (feet[k] - want[k]).abs() <= 1.0),
        "{feet:?} {want:?}"
    );
}

/// Plan 0079 Phase 4 done-when 4: the camera 12 m from a flat `-x` wall face (beyond the
/// spider's `near` of about 10.4 m), aiming square at it, in placement mode at the spider's
/// slot (showcase table): the ghost's mover is a surface body attached with normal `(-256, 0,
/// 0)`, its centre at the hit moved 6 voxels (1536) along `-x`, its turn taking `+z` to `-x`
/// within 2 per component; the left click leaves that `Placed` with the same state, and the
/// next tick with the wander off keeps it attached with nothing moved. At the same aim the
/// walker's ghost stands on the floor below the hit moved back by half its footprint, a
/// ground body turned about `+z` only.
#[test]
fn a_spider_placed_on_a_wall_is_on_the_wall() {
    let table = crate::walker::with_walker(&crate::showcase_materials());
    let mut run = run_in(rock_box(&table, 120));
    assert!(!run.modes.walkers.walking);
    let hit = [120 * FIXED_ONE, 0, 9 * FIXED_ONE];
    let feet = ghost_feet(&mut run, "spider", [0.0, 0.0, 9.0], 0.0);
    assert_eq!(feet, [120.0, 0.0, 9.0]);
    let g = run.modes.walkers.preview.clone().expect("a ghost");
    let m = g.mover.as_ref().expect("a mover");
    let st = *m.surface().expect("a surface body");
    assert!(st.attached);
    assert_eq!(st.normal, [-256, 0, 0]);
    assert_eq!(st.centre.0, [hit[0] - 1536, hit[1], hit[2]]);
    let up = m.now.root_q.rotate_vector([0, 0, 256]);
    assert!(
        (0..3).all(|k| (up[k] - [-256, 0, 0][k]).abs() <= 2),
        "{up:?}"
    );
    frame_keys(&mut run, &click(MouseButton::Left));
    let w = &run.modes.walkers;
    let p = w.placed.last().expect("placed");
    assert_eq!((p.id, &p.mover), (g.id, &g.mover));
    let steps = run.modes.walkers.tick(&run.world);
    let (_, s) = steps.iter().find(|(id, _)| *id == g.id).expect("ticked");
    assert!(s.on_surface && s.moved == Fixed3::default(), "{s:?}");

    let ex = run
        .modes
        .walkers
        .extent_of(Some(entry(&run, "walker")), &table);
    let feet = ghost_feet(&mut run, "walker", [0.0, 0.0, 9.0], 0.0);
    let want = [120.0 - 0.5 * ex[0].max(ex[1]), 0.0, -8.0];
    assert!(
        (0..3).all(|k| (feet[k] - want[k]).abs() <= 1.0),
        "{feet:?} {want:?}"
    );
    let g = run.modes.walkers.preview.as_ref().expect("a ghost");
    let m = g.mover.as_ref().expect("a mover");
    assert!(m.ground().is_some());
    let q = m.now.root_q.0;
    assert_eq!((q[1], q[2]), (0, 0), "{q:?}");
    assert_eq!(m.now.root_q.rotate_vector([0, 0, 256]), [0, 0, 256]);
}

/// Phase 5 done-when: the left click leaves one placed walker where the ghost stood (same id,
/// clip and yaw) and a new ghost, and digs nothing; `R` four times (22.5 degree steps, the
/// owner's answer to question 16) then a click places a walker turned 90 degrees from facing
/// and the next ghost keeps the turn; `Right` in the mode changes the ghost's clip and the
/// placed entity keeps it; the right click ends the mode with no fill and no placement.
#[test]
fn a_click_places_and_the_mode_stays() {
    let mut run = picker_run();
    look(&mut run, [0.0, 0.0, 9.0], -40.0);
    frame_keys(&mut run, &wheel_down());
    let g = run.modes.walkers.preview.clone().expect("a ghost");
    frame_keys(&mut run, &click(MouseButton::Left));
    let w = &run.modes.walkers;
    assert_eq!(w.placed.len(), 1);
    let p = &w.placed[0];
    assert_eq!(
        (p.id, p.feet, p.yaw, p.play.clip),
        (g.id, g.feet, g.yaw, g.play.clip)
    );
    let next = w.preview.as_ref().expect("a new ghost");
    assert_ne!(next.id, g.id);
    assert_eq!(notice(&run), Some("placed walker #1 (idle)"));
    assert_eq!(tick_ops(&mut run), 0, "no dig in the mode");

    for _ in 0..4 {
        frame_keys(&mut run, &pressed(KeyCode::KeyR));
    }
    frame_keys(&mut run, &click(MouseButton::Left));
    let eye = run.fly.camera.position.to_array();
    let w = &run.modes.walkers;
    let p = &w.placed[1];
    let turned = p.yaw - crate::walker::facing(p.feet, eye);
    assert!(
        (turned - std::f32::consts::FRAC_PI_2).abs() < 1e-4,
        "{turned}"
    );
    let n = w.preview.as_ref().expect("a ghost");
    let turned = n.yaw - crate::walker::facing(n.feet, eye);
    assert!(
        (turned - std::f32::consts::FRAC_PI_2).abs() < 1e-4,
        "{turned}"
    );

    let before = run.modes.walkers.preview.as_ref().unwrap().play.clip;
    frame_keys(&mut run, &pressed(KeyCode::ArrowRight));
    let after = run.modes.walkers.preview.as_ref().unwrap().play.clip;
    assert_ne!(after, before);
    frame_keys(&mut run, &click(MouseButton::Left));
    assert_eq!(run.modes.walkers.placed[2].play.clip, after);
    assert_eq!(notice(&run), Some("placed walker #3 (walk)"));

    frame_keys(&mut run, &click(MouseButton::Right));
    let w = &run.modes.walkers;
    assert!(!w.belt.on && w.preview.is_none());
    assert_eq!((w.placed.len(), w.set.len()), (3, 3));
    assert_eq!(tick_ops(&mut run), 0, "no fill leaving the mode");
}

/// Phase 6 done-when: in the mode the displays' marks hold the ghost's id alone, outlined
/// and tinted 0.35 in the ghost colour (the frame's first mark colour as `feed` hands it);
/// after a click the placed entity has no mark and the new ghost has it; out of the mode the
/// list is empty.
#[test]
fn the_ghost_is_marked_and_the_placed_are_not() {
    let mut run = picker_run();
    look(&mut run, [0.0, 0.0, 9.0], -40.0);
    frame_keys(&mut run, &wheel_down());
    let ghost = |run: &Run| run.modes.walkers.preview.as_ref().expect("a ghost").id;
    let mark = |colour| svet_render::Mark {
        colour,
        outline: true,
        tint: 0.35,
    };
    let g = ghost(&run);
    run.modes.walkers.refresh_displays();
    assert_eq!(
        run.modes.walkers.marks(),
        [(g, mark(svet_render::MARK_COLOURS[0]))]
    );
    frame_keys(&mut run, &click(MouseButton::Left));
    let red = [1.0, 0.0, 0.0];
    run.modes.walkers.ghost_colour = Some(red);
    run.modes.walkers.refresh_displays();
    let next = ghost(&run);
    assert_ne!(next, g);
    assert_eq!(run.modes.walkers.placed[0].id, g);
    assert_eq!(run.modes.walkers.marks(), [(next, mark(red))]);
    frame_keys(&mut run, &click(MouseButton::Right));
    run.modes.walkers.refresh_displays();
    assert_eq!(run.modes.walkers.marks(), []);
}

/// The cell words of `text` (one row a line, white) with `colours` per row and `paint`
/// laid over them, as the overlay writes them: rows of `svet_render::HUD_GRID_COLS` words.
fn cell_words(
    text: &str,
    colours: &[svet_render::HudColour],
    paint: svet_render::HudPaint<'_>,
) -> Vec<u32> {
    let cols = svet_render::HUD_GRID_COLS as usize;
    let mut cells = vec![0u32; text.split('\n').count().max(45) * cols];
    for (r, line) in text.split('\n').enumerate() {
        for (c, b) in line.bytes().enumerate().take(cols) {
            cells[r * cols + c] = u32::from(b) | (colours[r] as u32) << 8;
        }
    }
    paint.apply(&mut cells);
    cells
}

/// Phase 5 done-when, re-asserted whole at Phase 7: in the mode at 1280 x 720 (80 x 45 cells
/// at the HUD's scale 2) the belt's rows are the overlay's last seven, centred: the name row,
/// slot 1's frame top, the nine slots' digits, their names, two blank rows, the frame's
/// bottom; slot 1 framed by `+ - |`, and in the cell words the overlay writes every `+`, `-`
/// and `|` cell yellow and every other belt cell white; the backdrop's box under the slot
/// rows; the name row reads `walker` (yellow) for `BANNER_MS` after a wheel step and is blank
/// after; out of the mode the overlay has no belt rows and the stats overlay is as `F2` left
/// it.
#[test]
fn the_belt_draws_its_slots() {
    let mut run = picker_run();
    let stats = run.hud.visible;
    frame_keys(&mut run, &wheel_down());
    let w = &run.modes.walkers;
    let at = w.belt.changed_at.expect("a change");
    let draw = |now: f64| {
        let mut text = String::from("row a\nrow b");
        let mut colours = vec![svet_render::HudColour::White; 2];
        let mut paint = belt::Paint::default();
        let b = belt::append_belt(
            &mut text,
            &mut colours,
            (80, 45),
            &run.modes,
            now,
            crate::hud::BANNER_MS,
            &mut paint,
        );
        (text, colours, b, paint)
    };
    let (text, colours, b, paint) = draw(at + crate::hud::BANNER_MS - 1.0);
    let rows: Vec<&str> = text.split('\n').collect();
    assert_eq!((rows.len(), colours.len()), (45, 45));
    assert_eq!(&rows[..3], ["row a", "row b", ""]);
    let belt = &rows[45 - BELT_ROWS..];
    let pad = |n: usize| " ".repeat(n);
    assert_eq!(belt[0], format!("{}walker", pad(4 + (72 - 6) / 2)));
    assert_eq!(belt[1], format!("{}+------+", pad(4)));
    let digits: String = (1..=9).map(|d| format!(" {d}      ")).collect();
    assert_eq!(
        belt[2],
        format!("{}|1     |{}", pad(4), &digits[8..]).trim_end()
    );
    assert!(
        belt[3].starts_with(&format!("{}|walker|", pad(4))),
        "{}",
        belt[3]
    );
    assert_eq!(&belt[3][12..], " spider");
    assert_eq!(belt[4], format!("{}|      |", pad(4)));
    assert_eq!(belt[5], belt[4]);
    assert_eq!(belt[6], belt[1]);
    use svet_render::HudColour::{White, Yellow};
    assert_eq!(
        &colours[45 - BELT_ROWS..],
        [Yellow, White, White, White, White, White, White]
    );
    assert!(paint.tiles.is_empty(), "no icon has arrived");
    let cells = cell_words(&text, &colours, paint.with(&[b]));
    let cols = svet_render::HUD_GRID_COLS as usize;
    let mut framed = 0;
    for (r, line) in rows.iter().enumerate().skip(45 - BELT_ROWS + 1) {
        for c in 0..80 {
            let byte = line.as_bytes().get(c).copied().unwrap_or(b' ');
            let colour = (cells[r * cols + c] >> 8) & 0xff;
            let frame = matches!(byte, b'+' | b'-' | b'|');
            framed += usize::from(frame);
            let want = if frame { Yellow } else { White } as u32;
            assert_eq!(colour, want, "cell ({c}, {r}) {:?}", byte as char);
        }
    }
    assert_eq!(framed, 2 * 8 + 2 * 4, "the selected slot's frame");
    assert_eq!(
        b,
        svet_render::HudBox {
            col: 3,
            row: 39,
            cols: 74,
            rows: 6
        }
    );
    let (text, _, _, _) = draw(at + crate::hud::BANNER_MS + 1.0);
    assert_eq!(text.split('\n').nth(45 - BELT_ROWS), Some(""));

    let (text, colours) = (run.hud.text.clone(), run.hud.colours.clone());
    assert!(run.belt_rows([1280, 720], false).is_some());
    frame_keys(&mut run, &pressed(KeyCode::Escape));
    run.hud.text = text.clone();
    run.hud.colours = colours.clone();
    assert_eq!(run.belt_rows([1280, 720], false), None);
    assert_eq!((&run.hud.text, &run.hud.colours), (&text, &colours));
    assert_eq!(run.hud.visible, stats);
}

/// Phase 7 done-when 3: on the first frame in the mode the walker's icon has not arrived
/// (asked for at the library's build, taken at a later poll), so slot 1 lays no tile and
/// shows `walker`; once the worker's icon is taken the belt lays tile `walker`'s entry index
/// over the slot's 4 x 4 block (the tile bit in those cell words) and drops the slot's name,
/// and the chooser lays the same tile at the entry's first line; the overlay is handed the
/// icon once. A reload of the walker's rig that changes its model (a pivot moved) forgets the
/// icon until the new one arrives, under the same tile index.
#[test]
fn the_icon_arrives_after_the_words() {
    let from = std::path::Path::new(library::DEFAULT_DIR);
    let dir = std::env::temp_dir().join(format!("svet-icon-reload-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for name in ["walker", "spider"] {
        std::fs::create_dir_all(dir.join(name)).expect("dir");
        for ext in ["rig", "vox"] {
            let f = format!("{name}/{name}.{ext}");
            std::fs::copy(from.join(&f), dir.join(&f)).expect("copied");
        }
    }
    let mut run = bare_run(rock_box(&cave_table(), 60));
    run.modes.walkers = crate::entities::Walkers::default().with_library(library::scan(&dir));
    run.poll_viewer(0.0);
    frame_keys(&mut run, &wheel_down());
    let wi = run
        .modes
        .walkers
        .belt
        .entry()
        .expect("slot 1 holds the walker");
    let belt_of = |run: &Run| {
        let (mut text, mut colours) = (String::new(), Vec::new());
        let mut paint = belt::Paint::default();
        let b = belt::append_belt(
            &mut text,
            &mut colours,
            (80, 45),
            &run.modes,
            0.0,
            crate::hud::BANNER_MS,
            &mut paint,
        );
        (text, colours, b, paint)
    };
    let (text, _, _, paint) = belt_of(&run);
    assert!(!run.modes.walkers.icons.ready(wi));
    assert!(paint.tiles.is_empty());
    assert!(text.split('\n').nth(45 - 4).unwrap().contains("|walker|"));

    run.modes.walkers.icons.wait();
    assert!(run.modes.walkers.icons.ready(wi));
    let (text, colours, b, paint) = belt_of(&run);
    let tile = svet_render::HudTile {
        col: 4 + 2,
        row: (45 - BELT_ROWS + 2) as u32,
        index: wi as u32,
    };
    assert_eq!(paint.tiles, [tile]);
    assert!(!text.split('\n').nth(45 - 4).unwrap().contains("walker"));
    let cells = cell_words(&text, &colours, paint.with(&[b]));
    let cols = svet_render::HUD_GRID_COLS as usize;
    for r in 0..45 {
        for c in 0..80 {
            let inside = (6..10).contains(&c) && (40..44).contains(&r);
            let bit = cells[r * cols + c] & svet_render::TILE_BIT != 0;
            assert_eq!(bit, inside, "cell ({c}, {r})");
        }
    }
    let ups: Vec<u32> = run.modes.walkers.icons.uploads().map(|(i, _)| i).collect();
    assert_eq!(ups, [wi as u32]);
    assert_eq!(
        run.modes.walkers.icons.uploads().count(),
        0,
        "handed over once"
    );
    frame_keys(&mut run, &pressed(KeyCode::Backquote));
    let mut paint = belt::Paint::default();
    run.menu.write_with(
        &run.modes,
        &run.run_view(),
        None,
        &mut String::new(),
        &mut Vec::new(),
        &mut paint,
    );
    assert_eq!(
        paint
            .tiles
            .iter()
            .map(|t| (t.col, t.index))
            .collect::<Vec<_>>(),
        [(2, wi as u32)]
    );
    frame_keys(&mut run, &pressed(KeyCode::Escape));

    let rig = dir.join("walker/walker.rig");
    let text = std::fs::read_to_string(&rig).expect("reads");
    let moved = text.replace("pivot 3 3 0 detail 2", "pivot 3 3 1 detail 2");
    assert_ne!(moved, text, "the head's pivot line");
    std::fs::write(&rig, moved).expect("saved");
    std::fs::File::options()
        .write(true)
        .open(&rig)
        .expect("opens")
        .set_modified(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(10))
        .expect("stamped");
    run.poll_viewer(1000.0);
    assert!(
        !run.modes.walkers.icons.ready(wi),
        "the old icon is forgotten"
    );
    run.modes.walkers.icons.wait();
    assert!(run.modes.walkers.icons.ready(wi));
    let ups: Vec<u32> = run.modes.walkers.icons.uploads().map(|(i, _)| i).collect();
    assert_eq!(ups, [wi as u32], "the same tile index");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Phase 5 done-when: on the pad, `DpadRight` enters the mode at slot 2, `DpadLeft` back to
/// 1, `West` turns the ghost (the cutaway as it was), `R2` places, `East` ends the mode;
/// `Start`, the `entities` row, `choose an entity` and `South` on the spider reach the mode
/// at the spider's slot.
#[test]
fn the_pad_places_from_the_view() {
    let mut run = picker_run();
    look(&mut run, [0.0, 0.0, 9.0], -40.0);
    frame_keys(&mut run, &pad(PadButton::DpadRight));
    assert!(run.modes.walkers.belt.on);
    assert_eq!(run.modes.walkers.belt.selected, 1);
    frame_keys(&mut run, &pad(PadButton::DpadLeft));
    assert_eq!(run.modes.walkers.belt.selected, 0);
    assert!(run.modes.walkers.preview.is_some());
    let cutaway = run.modes.row(crate::hud::HudRow::Controls);
    frame_keys(&mut run, &pad(PadButton::West));
    assert_eq!(run.modes.walkers.belt.turn, 1);
    assert_eq!(run.modes.row(crate::hud::HudRow::Controls), cutaway);
    let mut r2 = Input::default();
    r2.set_axis(Axis::TriggerR, 1.0);
    frame_keys(&mut run, &r2);
    assert_eq!(run.modes.walkers.placed.len(), 1);
    frame_keys(&mut run, &Input::default());
    frame_keys(&mut run, &pad(PadButton::East));
    assert!(!run.modes.walkers.belt.on && run.modes.walkers.preview.is_none());
    assert_eq!(run.modes.walkers.placed.len(), 1);

    frame_keys(&mut run, &pad(PadButton::Start));
    let at = crate::hud::menu::Group::ALL
        .iter()
        .position(|&g| g == crate::hud::menu::Group::Entities)
        .expect("a group");
    for _ in 0..at {
        frame_keys(&mut run, &pad(PadButton::DpadDown));
    }
    frame_keys(&mut run, &pad(PadButton::South));
    assert_eq!(
        run.menu.page(),
        Some(PageId::Group(crate::hud::menu::Group::Entities))
    );
    run.write_menu();
    assert!(
        run.hud
            .text
            .lines()
            .nth(1)
            .unwrap()
            .starts_with("> choose an entity"),
        "{}",
        run.hud.text
    );
    frame_keys(&mut run, &pad(PadButton::South));
    assert_eq!(run.menu.page(), Some(PageId::Entities));
    frame_keys(&mut run, &pad(PadButton::DpadDown));
    frame_keys(&mut run, &pad(PadButton::South));
    let spider = entry(&run, "spider");
    let w = &run.modes.walkers;
    assert!(!run.menu.is_open() && w.belt.on);
    assert_eq!(w.belt.entry(), Some(spider));
}

/// Phase 3 done-when, in the mode: with three placed and the crosshair on the second,
/// `Delete` removes the second only, with the menu closed, in placement mode and with the
/// chooser open; `4` afterwards removes the third out of the mode, and in the mode selects
/// slot 4 and removes nothing (Phase 9 (1)).
#[test]
fn the_remove_key_takes_the_aimed_entity() {
    for state in ["closed", "placing", "open"] {
        let mut run = picker_run();
        let ids = three_in_a_row(&mut run);
        match state {
            "placing" => enter_slot(&mut run, 2),
            "open" => run.act(crate::toggles::Act::Picker),
            _ => {}
        }
        frame_keys(&mut run, &pressed(KeyCode::Delete));
        let placed = |run: &Run| {
            run.modes
                .walkers
                .placed
                .iter()
                .map(|p| p.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(placed(&run), [ids[0], ids[2]], "{state}");
        assert!(run.modes.walkers.set.get(ids[1]).is_none());
        assert_eq!(run.menu.is_open(), state == "open");
        assert_eq!(run.modes.walkers.belt.on, state == "placing");
        if state == "open" {
            // `4` types into the open menu's search: close it first.
            frame_keys(&mut run, &pressed(KeyCode::Escape));
        }
        frame_keys(&mut run, &pressed(KeyCode::Digit4));
        if state == "placing" {
            assert_eq!(placed(&run), [ids[0], ids[2]], "{state}");
            assert_eq!(run.modes.walkers.belt.selected, 3);
        } else {
            assert_eq!(placed(&run), [ids[0]], "{state}");
        }
    }
}

/// Three walkers standing on the floor across the view, the second dead ahead; `4`'s marks
/// pushed as a placement pushes them.
fn three_in_a_row(run: &mut Run) -> Vec<svet_entity::EntityId> {
    let table = run.world.materials().clone();
    let walker = entry(run, "walker");
    let ids = [-12.0, 0.5, 12.0].map(|y| {
        run.modes
            .walkers
            .place_entry(&table, Some(walker), [20.0, y, -8.0], 0.0)
            .expect("placed")
    });
    run.modes.walkers.newest.extend([true; 3]);
    run.aim_entities();
    assert_eq!(run.modes.walkers.aimed, Some(ids[1]));
    ids.to_vec()
}

/// Phase 3 done-when: from the Keys page `` ` `` shows the chooser (no ghost: the chooser
/// only chooses); `F1` from it shows the Keys page; `Esc` closes, and the stats overlay is as
/// `F2` left it, in both states of it.
#[test]
fn the_overlay_shows_one_panel() {
    for stats in [false, true] {
        let mut run = picker_run();
        run.hud.visible = stats;
        frame_keys(&mut run, &pressed(KeyCode::F1));
        assert_eq!(run.menu.page(), Some(PageId::Keys));
        frame_keys(&mut run, &pressed(KeyCode::Backquote));
        assert_eq!(run.menu.page(), Some(PageId::Entities));
        assert!(run.modes.walkers.preview.is_none());
        assert_eq!(Overlay::of(false, run.menu.is_open(), stats), Overlay::Menu);
        frame_keys(&mut run, &pressed(KeyCode::F1));
        assert_eq!(run.menu.page(), Some(PageId::Keys));
        assert_eq!(run.modes.walkers.set.len(), 0);
        frame_keys(&mut run, &pressed(KeyCode::Escape));
        assert!(!run.menu.is_open());
        assert_eq!(run.hud.visible, stats);
    }
}

/// Phase 3 done-when, in the mode: with 64 entities placed the mode shows no ghost and the
/// notice row reads `entities 64/64 full`; a click places nothing.
#[test]
fn the_caps_hold_with_a_ghost() {
    let mut run = picker_run();
    let table = run.world.materials().clone();
    let walker = entry(&run, "walker");
    for i in 0..64 {
        let feet = [(i % 8) as f32 * 6.0, (i / 8) as f32 * 6.0 - 24.0, -8.0];
        run.modes
            .walkers
            .place_entry(&table, Some(walker), feet, 0.0)
            .expect("placed");
    }
    frame_keys(&mut run, &wheel_down());
    assert!(run.modes.walkers.belt.on);
    assert!(run.modes.walkers.preview.is_none());
    assert_eq!(notice(&run), Some("entities 64/64 full"));
    frame_keys(&mut run, &click(MouseButton::Left));
    assert_eq!(run.modes.walkers.placed.len(), 64);
    assert_eq!(run.modes.walkers.set.len(), 64);
    assert_eq!(notice(&run), Some("entities 64/64 full"));
}

/// Phase 3 and 5 done-when: under `--tester` `` ` ``, the wheel and `2` map to no tester
/// action, so the tester has no chooser and no placement mode; and an entry in error handed
/// over from the chooser stands no ghost and says its error in the notice row.
#[test]
fn the_tester_has_no_picker_and_an_entry_in_error_refuses() {
    use crate::path::{TesterInput, tester_action};
    assert_eq!(tester_action(TesterInput::Key(KeyCode::Backquote)), None);
    assert_eq!(tester_action(TesterInput::Key(KeyCode::Digit2)), None);
    let mut run = picker_run();
    run.args.tester = true;
    frame_keys(&mut run, &wheel_down());
    frame_keys(&mut run, &pressed(KeyCode::Digit2));
    assert!(!run.modes.walkers.belt.on && run.modes.walkers.preview.is_none());
    run.args.tester = false;

    let spider = entry(&run, "spider");
    run.act(crate::toggles::Act::Picker);
    run.menu_act(Activated::Choose {
        item: Item::Entity(spider),
    });
    let words = notice(&run).expect("the refusal");
    assert!(
        words.starts_with("spider: ") && words.contains("crystal"),
        "{words}"
    );
    assert!(run.modes.walkers.belt.on);
    assert!(run.modes.walkers.placed.is_empty() && run.modes.walkers.preview.is_none());
}

mod reading {
    use super::*;

    /// The belt's format time (`Run::belt_rows` over the stats rows at 1280 x 720) over 60
    /// frames in placement mode with the walker's ghost standing; printed, not asserted. Run
    /// by hand: `--run-ignored only -E 'test(=run::picker_tests::reading::the_belt_formats)'`.
    #[test]
    #[ignore = "a reading"]
    fn the_belt_formats() {
        let mut run = picker_run();
        frame_keys(&mut run, &wheel_down());
        let mut us: Vec<u32> = (0..60)
            .map(|_| {
                frame_keys(&mut run, &Input::default());
                run.hud.text.clear();
                run.hud.colours.clear();
                let t = crate::now();
                run.belt_rows([1280, 720], false).expect("in the mode");
                ((crate::now() - t).as_secs_f64() * 1e6) as u32
            })
            .collect();
        us.sort_unstable();
        println!(
            "belt format_us over 60 frames: median {} min {} max {}",
            us[30], us[0], us[59]
        );
    }
}
