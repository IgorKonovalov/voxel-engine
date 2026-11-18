//! The run's tests of a structure in the belt (Plan 0083 Phase 6): the belt's fill and the
//! Structures chooser, the frame ghost, the box rule, the quarter turn and the scale, the
//! click's logged stamps and the words. A rock box of the cave's table with the walker's rows
//! and the structures library's rows, the committed entities and structures libraries, the
//! castle entry left unbuilt.

use std::path::Path;

use super::*;
use crate::entities::belt::{self, BELT_ROWS, Item};
use crate::entities::library;
use crate::hud::menu::{Activated, Group, PageId, Row};
use crate::structures::{self as st, CLEAR_MARGIN};
use crate::test_support::bare_run;
use vxe_app::{Input, KeyCode, MouseButton};
use vxe_world::{AIR, ChunkExtent, EditOp, FIXED_ONE, Fixed3, MaterialTable, Mode, Op, World};

/// The cave's table with the walker's rows: the table before the library's rows.
fn base_table() -> MaterialTable {
    crate::walker::with_walker(&crate::cave_materials())
}

/// [`base_table`] with the structures library's rows (`structures::add_library_rows`).
fn world_table() -> MaterialTable {
    let mut t = base_table();
    st::add_library_rows(Path::new(st::LIBRARY), None, &mut t);
    t
}

/// A rock box of `table` hollow over `[-8, 120) x [-40, 40) x [-8, 60)`: the floor's top at
/// z = -8, room for the chapel's box and its margin 2.6 m ahead.
fn rock_box(table: &MaterialTable) -> World {
    let world = World::generate(
        ChunkExtent::new([-1, -1, -1], [2, 1, 1]),
        &vxe_world::UniformSource(crate::cave::ROCK),
        table,
    );
    world
        .apply(&EditOp::box_(
            Fixed3([-8, -40, -8].map(|v: i64| v * FIXED_ONE)),
            Fixed3([120, 40, 60].map(|v: i64| v * FIXED_ONE)),
            Mode::Carve,
        ))
        .expect("resident");
    world
}

/// A run in `world` with the committed libraries, the entities built against its table.
fn run_in(world: World) -> Run {
    let mut run = bare_run(world);
    let lib = library::scan(Path::new(library::DEFAULT_DIR));
    run.modes.walkers = crate::entities::Walkers::default().with_library(lib);
    let table = run.world.materials().clone();
    run.modes.structures = st::Library::load(Path::new(st::LIBRARY), &table, &base_table());
    run.poll_viewer(0.0);
    run
}

fn structure_run() -> Run {
    run_in(rock_box(&world_table()))
}

fn frame_keys(run: &mut Run, input: &Input) {
    let (pass, exit) = run.key_step(input);
    assert!(exit.is_none(), "the run goes on");
    run.flight_step(pass.input(), 1.0 / 60.0);
}

fn pressed(k: KeyCode) -> Input {
    let mut i = Input::default();
    i.key_down(k);
    i
}

fn wheel_down() -> Input {
    let mut i = Input::default();
    i.add_scroll(-1.0);
    i
}

fn click() -> Input {
    let mut i = Input::default();
    i.button_down(MouseButton::Left);
    i
}

/// The camera at `at`, `pitch_deg` above the horizon, looking along +x.
fn look(run: &mut Run, at: [f32; 3], pitch_deg: f32) {
    run.fly.camera.position = vxe_render::glam::Vec3::from_array(at);
    run.fly.camera.yaw = 0.0;
    run.fly.camera.pitch = pitch_deg.to_radians();
}

/// The eye 1.7 m over the floor's top (z = -8), looking 40 degrees down along +x.
const EYE: [f32; 3] = [0.5, 0.5, 9.0];
/// Where that view meets the floor's top: x = 0.5 + 17 / tan 40 degrees.
fn floor_hit_x() -> f32 {
    0.5 + 17.0 / 40f32.to_radians().tan()
}

/// The library index of the structure named `name`.
fn structure(run: &Run, name: &str) -> usize {
    run.modes
        .structures
        .entries
        .iter()
        .position(|e| e.name == name)
        .unwrap_or_else(|| panic!("{name} in the structures"))
}

/// Into placement mode at slot `s` (0-based): the wheel, then the slot's digit.
fn enter_slot(run: &mut Run, s: usize) {
    if !run.modes.walkers.belt.on {
        frame_keys(run, &wheel_down());
    }
    frame_keys(run, &pressed(crate::toggles::BELT_KEYS[s]));
}

/// The frame ghost's box: the extent of its parts' voxels in its rest pose.
fn frame_size(run: &Run) -> [u32; 3] {
    let f = run.modes.walkers.frame.as_ref().expect("a frame ghost");
    let mut hi = [0u32; 3];
    for p in f.model.parts() {
        let s = p.volume.size();
        for k in 0..3 {
            hi[k] = hi[k].max((-p.pivot[k] / 256) as u32 + s[k]);
        }
    }
    hi
}

fn notice(run: &Run) -> Option<&str> {
    run.notice.as_ref().map(|(l, _)| l.as_str())
}

/// Done-when 4: the belt fills walker, spider, chapel, castle, slot 5 empty; the wheel to slot
/// 3 enters the mode with the chapel's frame ghost in the set (twelve parts, marked in the
/// ghost colour) and nothing placed; `Tab`, `structures`, `choose a structure`, `Down` to the
/// castle and `7` put it in slot 7 with the page staying; `Enter` on the chapel selects slot
/// 3, closes the menu and enters the mode; `` ` `` on the Structures page does the same; the
/// Entities page lists no structure.
#[test]
fn the_belt_holds_a_structure() {
    let mut run = structure_run();
    let walker = run
        .modes
        .walkers
        .library
        .entries
        .iter()
        .position(|e| e.name == "walker");
    let spider = run
        .modes
        .walkers
        .library
        .entries
        .iter()
        .position(|e| e.name == "spider");
    let (chapel, castle) = (structure(&run, "chapel"), structure(&run, "castle"));
    run.fill_belt();
    let b = &run.modes.walkers.belt;
    assert_eq!(
        b.slots[..5],
        [
            walker.map(Item::Entity),
            spider.map(Item::Entity),
            Some(Item::Structure(chapel)),
            Some(Item::Structure(castle)),
            None
        ]
    );

    look(&mut run, EYE, -40.0);
    for _ in 0..3 {
        frame_keys(&mut run, &wheel_down());
    }
    let w = &mut run.modes.walkers;
    assert!(w.belt.on);
    assert_eq!((w.belt.selected, w.belt.structure()), (2, Some(chapel)));
    let f = w.frame.as_ref().expect("the chapel's frame");
    assert_eq!(f.model.parts().len(), 12);
    let id = f.id;
    assert!(w.set.get(id).is_some());
    assert!(w.placed.is_empty() && w.preview.is_none());
    w.refresh_displays();
    let colour = vxe_render::MARK_COLOURS[0];
    assert!(
        w.marks()
            .iter()
            .any(|(i, m)| *i == id && m.outline && m.colour == colour),
        "{:?}",
        w.marks()
    );

    // `Tab`, the `structures` group, `choose a structure`.
    frame_keys(&mut run, &pressed(KeyCode::Tab));
    let at = Group::ALL
        .iter()
        .position(|&g| g == Group::Structures)
        .unwrap();
    for _ in 0..at {
        frame_keys(&mut run, &pressed(KeyCode::ArrowDown));
    }
    frame_keys(&mut run, &pressed(KeyCode::Enter));
    assert_eq!(run.menu.page(), Some(PageId::Group(Group::Structures)));
    let v = run.run_view();
    let labels: Vec<&str> = run
        .menu
        .rows(&run.modes, &v)
        .iter()
        .filter_map(|r| match r {
            Row::Entry(e) => Some(e.label),
            _ => None,
        })
        .collect();
    let choose = labels.iter().position(|&l| l == "choose a structure");
    for _ in 0..choose.expect("the action") {
        frame_keys(&mut run, &pressed(KeyCode::ArrowDown));
    }
    frame_keys(&mut run, &pressed(KeyCode::Enter));
    assert_eq!(run.menu.page(), Some(PageId::Structures));
    assert!(!run.modes.walkers.belt.on, "the menu ends the mode");
    let row = |run: &Run| run.menu.selected_row(&run.modes, &run.run_view());
    assert!(matches!(row(&run), Some(Row::Item(Item::Structure(j))) if j == chapel));
    frame_keys(&mut run, &pressed(KeyCode::ArrowDown));
    assert!(matches!(row(&run), Some(Row::Item(Item::Structure(j))) if j == castle));
    frame_keys(&mut run, &pressed(KeyCode::Digit7));
    let b = &run.modes.walkers.belt;
    assert_eq!(
        (b.slots[3], b.slots[6]),
        (None, Some(Item::Structure(castle)))
    );
    assert_eq!(run.menu.page(), Some(PageId::Structures));
    assert_eq!(notice(&run), Some("castle in slot 7"));

    // keys:typed-begin
    frame_keys(&mut run, &pressed(KeyCode::ArrowUp));
    // keys:typed-end
    frame_keys(&mut run, &pressed(KeyCode::Enter));
    assert!(!run.menu.is_open());
    let b = &run.modes.walkers.belt;
    assert!(b.on);
    assert_eq!((b.selected, b.structure()), (2, Some(chapel)));
    assert!(run.modes.walkers.frame.is_some());

    run.act(crate::toggles::Act::StructurePicker);
    assert_eq!(run.menu.page(), Some(PageId::Structures));
    frame_keys(&mut run, &pressed(KeyCode::Backquote));
    assert!(!run.menu.is_open());
    assert_eq!(run.modes.walkers.belt.structure(), Some(chapel));
    assert!(run.modes.walkers.belt.on);

    run.act(crate::toggles::Act::Picker);
    assert_eq!(run.menu.page(), Some(PageId::Entities));
    let rows = run.menu.rows(&run.modes, &run.run_view());
    assert!(
        rows.iter().all(|r| matches!(r, Row::Item(Item::Entity(_)))),
        "{rows:?}"
    );
}

/// Done-when 5: the camera 1.7 m over the floor looking 40 degrees down along +x: the chapel's
/// box has its minimum x at the floor hit's voxel, its y centre within a voxel of the hit's y
/// and its base on the floor's top, the eye outside it; `R` swaps its x and y extents and
/// the corner follows; at `structure scale` 2 the box doubles with its corner at the hit;
/// aimed level into an empty world, the corner's x is the view's 3 m point's; a 600-wide box
/// starts at the hit too and does not hold the eye.
#[test]
fn a_structure_stands_by_the_box_rule() {
    let mut run = structure_run();
    let chapel = structure(&run, "chapel");
    look(&mut run, EYE, -40.0);
    enter_slot(&mut run, 2);
    assert_eq!(run.modes.walkers.belt.structure(), Some(chapel));
    let hit = floor_hit_x().floor() as i32;
    let corner = |run: &Run| run.modes.walkers.frame.as_ref().expect("a frame").corner;
    let c = corner(&run);
    assert_eq!(frame_size(&run), [48, 32, 40]);
    assert_eq!(c[0], hit, "{c:?}");
    assert!((c[1] as f32 + 16.0 - EYE[1]).abs() <= 1.0, "{c:?}");
    assert_eq!(c[2], -8);
    assert!(EYE[0] < c[0] as f32, "the eye is outside the box");

    frame_keys(&mut run, &pressed(KeyCode::KeyR));
    assert_eq!(notice(&run), Some("structure turned 90 degrees"));
    assert_eq!(run.modes.walkers.belt.quarters(), 1);
    let c = corner(&run);
    assert_eq!(frame_size(&run), [32, 48, 40]);
    assert_eq!(c[0], hit, "{c:?}");
    assert!((c[1] as f32 + 24.0 - EYE[1]).abs() <= 1.0, "{c:?}");
    frame_keys(&mut run, &pressed(KeyCode::KeyR));
    frame_keys(&mut run, &pressed(KeyCode::KeyR));
    frame_keys(&mut run, &pressed(KeyCode::KeyR));
    assert_eq!(run.modes.walkers.belt.quarters(), 0);

    let scale = crate::toggles::TOGGLES
        .iter()
        .find(|t| t.name == "structure scale")
        .expect("the toggle");
    run.menu_act(Activated::Step(scale, 1));
    run.sync_ghost();
    assert_eq!(run.modes.structure_scale, 2);
    let f = run.modes.walkers.frame.as_ref().expect("a frame");
    assert_eq!(
        f.model.parts().len(),
        8,
        "a box over 64 a side: its corners"
    );
    assert_eq!(frame_size(&run), [96, 64, 80]);
    let c = corner(&run);
    assert_eq!((c[0], c[2]), (hit, -8));
    assert!((c[1] as f32 + 32.0 - EYE[1]).abs() <= 1.0, "{c:?}");

    let big = run.aim_corner([600, 600, 500]);
    assert_eq!((big[0], big[2]), (hit, -8));
    let inside = (0..3).all(|k| {
        let s = [600, 600, 500][k] as f32;
        (big[k] as f32) <= EYE[k] && EYE[k] < big[k] as f32 + s
    });
    assert!(!inside, "{big:?}");

    let air = World::generate(
        ChunkExtent::new([-1, -1, -1], [2, 1, 1]),
        &vxe_world::UniformSource(AIR),
        &world_table(),
    );
    let mut dark = run_in(air);
    look(&mut dark, [0.5, 0.5, 0.5], 0.0);
    let c = dark.aim_corner([48, 32, 40]);
    assert_eq!(c[0], (0.5 + belt::NEAR_MIN).floor() as i32, "{c:?}");
}

/// The run's op log's stamps, in order: each one's prefab id.
fn stamps(run: &Run) -> Vec<u64> {
    let net = run.net.as_ref().unwrap();
    net.server
        .ops()
        .entries()
        .iter()
        .filter_map(|e| match e.op() {
            Some(Op::Stamp { prefab, .. }) => Some(*prefab),
            _ => None,
        })
        .collect()
}

/// Done-when 6: the left click on the chapel's slot submits one stamp of a uniform air prefab
/// the size of `clear_box` and four stamps of the scene's prefabs, logged in that order and
/// applied in one tick; the world then holds the chapel's per-row counts (Phase 4's) in its
/// box, air in the margin; the mode stays with a new frame; the notice names the ops, the
/// chunks and the pending placements. With `clear box` off there are four stamps and no air.
/// A replay of the log on a fresh world hashes equal.
#[test]
fn a_placed_structure_is_logged_stamps() {
    let mut run = structure_run();
    let chapel = structure(&run, "chapel");
    look(&mut run, EYE, -40.0);
    enter_slot(&mut run, 2);
    let c = run.modes.walkers.frame.as_ref().expect("a frame").corner;
    frame_keys(&mut run, &click());
    run.tick_server();
    let scene = run.modes.structures.entries[chapel]
        .scene()
        .unwrap()
        .clone();
    let offset = [0, 1, 2].map(|k| c[k] - scene.bounds.0[k]);
    let (lo, hi) = st::clear_box(&scene, offset, 0).unwrap();
    let air = vxe_world::Prefab::uniform([0, 1, 2].map(|k| (hi[k] - lo[k] + 1) as u32), AIR);
    let mut want = vec![air.id()];
    want.extend(
        scene
            .placements
            .iter()
            .map(|&(i, _, _)| scene.prefabs[i].id()),
    );
    assert_eq!(stamps(&run), want);

    // Phase 4's per-row counts of the chapel, in its box.
    let base = base_table().len() as u16;
    let mut counts = std::collections::BTreeMap::<u16, u64>::new();
    for z in lo[2]..=hi[2] {
        for y in lo[1]..=hi[1] {
            for x in lo[0]..=hi[0] {
                let m = run
                    .world
                    .material_at(vxe_world::VoxelCoord([x, y, z]))
                    .unwrap();
                if m >= base {
                    *counts.entry(m).or_default() += 1;
                }
            }
        }
    }
    let mut got: Vec<u64> = counts.into_values().collect();
    got.sort_unstable();
    assert_eq!(got, [16, 710, 804, 1728, 6724]);
    let mid = [c[0] - 5, c[1] + 16, c[2] + 20];
    assert_eq!(run.world.material_at(vxe_world::VoxelCoord(mid)), Some(AIR));
    assert!(lo[0] == c[0] - CLEAR_MARGIN && hi[2] == c[2] + 39 + CLEAR_MARGIN);

    assert!(run.modes.walkers.belt.on);
    assert!(run.modes.walkers.frame.is_some());
    let n = notice(&run).expect("a notice");
    assert!(
        n.starts_with("placed chapel 4.8 x 3.2 x 4.0 m: 5 ops, ")
            && n.ends_with(" chunks now, 0 pending"),
        "{n}"
    );

    let fresh = rock_box(&world_table());
    let net = run.net.as_ref().unwrap();
    for e in net.server.ops().entries() {
        if let Some(op) = e.op() {
            op.apply(&fresh, net.server.prefabs()).expect("applies");
        }
    }
    assert_eq!(fresh.voxel_hash(), run.world.voxel_hash());

    let mut off = structure_run();
    off.modes.clear_box = false;
    look(&mut off, EYE, -40.0);
    enter_slot(&mut off, 2);
    frame_keys(&mut off, &click());
    off.tick_server();
    assert_eq!(stamps(&off), want[1..]);
    assert!(notice(&off).is_some_and(|n| n.contains(": 4 ops, ")));
}

/// Done-when 8's belt words: slot 3 shows `chapel` and slot 4 `castle`, the chapel's icon in
/// slot 3 once made; the name row on slot 3 reads `chapel  4.8 x 3.2 x 4.0 m`, at scale 2
/// `chapel  9.6 x 6.4 x 8.0 m  x2`, and on the castle before its build `castle: building`.
#[test]
fn the_belt_names_a_structure() {
    let mut run = structure_run();
    look(&mut run, EYE, -40.0);
    let draw = |run: &Run| {
        let (mut text, mut colours) = (String::new(), Vec::new());
        let at = run.modes.walkers.belt.changed_at.expect("a change");
        belt::append_belt(
            &mut text,
            &mut colours,
            (80, 45),
            &run.modes,
            at,
            crate::hud::BANNER_MS,
            &mut belt::Paint::default(),
        );
        let rows: Vec<String> = text.lines().map(str::to_owned).collect();
        (
            rows[45 - BELT_ROWS].trim().to_owned(),
            rows[45 - BELT_ROWS + 3].clone(),
            colours,
        )
    };
    enter_slot(&mut run, 2);
    let (name, slots, _) = draw(&run);
    assert_eq!(name, "chapel  4.8 x 3.2 x 4.0 m");
    let x0 = (80 - belt::BELT_COLS) / 2;
    let cell = |s: usize| {
        slots[x0 + s * belt::SLOT_COLS + 1..x0 + (s + 1) * belt::SLOT_COLS - 1]
            .trim()
            .to_owned()
    };
    assert_eq!(
        (cell(2), cell(3)),
        ("chapel".to_owned(), "castle".to_owned())
    );
    // The chapel's icon, once made, in slot 3's tile after the entities'.
    run.modes.walkers.icons.wait();
    let tile = belt::tile_of(&run.modes, Item::Structure(structure(&run, "chapel")));
    assert!(run.modes.walkers.icons.ready(tile));
    let mut paint = belt::Paint::default();
    belt::append_belt(
        &mut String::new(),
        &mut Vec::new(),
        (80, 45),
        &run.modes,
        0.0,
        crate::hud::BANNER_MS,
        &mut paint,
    );
    let col = (x0 + 2 * belt::SLOT_COLS + 2) as u32;
    assert!(
        paint
            .tiles
            .iter()
            .any(|t| t.index == tile as u32 && t.col == col),
        "{:?}",
        paint.tiles
    );
    run.modes.structure_scale = 2;
    assert_eq!(draw(&run).0, "chapel  9.6 x 6.4 x 8.0 m  x2");
    frame_keys(&mut run, &pressed(KeyCode::Digit4));
    let (name, _, colours) = draw(&run);
    assert_eq!(name, "castle: building");
    assert_eq!(colours[45 - BELT_ROWS], vxe_render::HudColour::Grey);
}

/// `Esc` on a structure's slot ends the mode and takes its frame out of the set; the run goes
/// on.
#[test]
fn escape_leaves_a_structure_slot() {
    let mut run = structure_run();
    enter_slot(&mut run, 2);
    let id = run.modes.walkers.frame.as_ref().expect("a frame").id;
    let (_, exit) = run.key_step(&pressed(KeyCode::Escape));
    assert_eq!(exit, None);
    assert!(!run.modes.walkers.belt.on);
    assert!(run.modes.walkers.frame.is_none());
    assert!(run.modes.walkers.set.get(id).is_none());
}
