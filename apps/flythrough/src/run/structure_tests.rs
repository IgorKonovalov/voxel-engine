//! The run's tests of a structure in the belt (Plan 0083 Phases 6 and 7b): the belt's fill and
//! the Structures chooser, the ghost (a coarse copy in its frame), the box rule, the quarter
//! turn and the scale, the click's logged stamps and the words. A rock box of the cave's table
//! with the walker's rows and the structures library's rows, the committed entities and
//! structures libraries, the castle entry left unbuilt.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use super::*;
use crate::entities::belt::{self, BELT_ROWS, Item};
use crate::entities::library;
use crate::hud::menu::{Activated, Group, PageId, Row};
use crate::structures::{self as st, CLEAR_MARGIN};
use crate::test_support::bare_run;
use vxe_app::{Input, KeyCode, MouseButton};
use vxe_assets::VoxScene;
use vxe_world::{
    AIR, ChunkExtent, EditOp, FIXED_ONE, Fixed3, MaterialTable, Mode, Op, Prefab, Rot, VoxelCoord,
    World,
};

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

/// The ghost's frame: the extent of its frame's parts' voxels in its rest pose (part 0, the
/// coarse body, left out when there is one: 13 or 9 parts, against the frame's 12 or 8).
fn frame_size(run: &Run) -> [u32; 3] {
    let f = run.modes.walkers.frame.as_ref().expect("a frame ghost");
    let parts = f.model.parts();
    let body = usize::from(parts.len() % 2 == 1);
    let mut hi = [0u32; 3];
    for p in &parts[body..] {
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
/// 3 enters the mode with the chapel's ghost in the set (13 parts: its coarse body and twelve
/// edges, marked in the ghost colour) and nothing placed; `Tab`, `structures`, `choose a structure`, `Down` to the
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
    assert_eq!(f.model.parts().len(), 13);
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
        9,
        "a box over 64 a side: its body and its corners"
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

/// The table of the world `flags` start (`setup::build_world`, one worker): the run's own
/// table, the generation or the castle's build joined.
fn table_of_run(flags: &[&str]) -> MaterialTable {
    let mut v = flags.to_vec();
    v.extend(["--workers", "1"]);
    let mut args = crate::cli::parse_args_from(crate::test_support::argv(&v)).expect("parses");
    let (_, pending) = crate::setup::build_world(&mut args, &crate::edits::Failure::default());
    match pending {
        crate::app::Pending::Stream(b) => b.0.materials().clone(),
        crate::app::Pending::StreamBuilding(h) => h.join().expect("built").0.materials().clone(),
        crate::app::Pending::Level(h) => h
            .join()
            .expect("generated")
            .expect("a world")
            .materials()
            .clone(),
        crate::app::Pending::Failed => panic!("{flags:?}: no world"),
    }
}

/// Asserts that each library file's scene loads against the table of the world `flags` start
/// with no refusal (an unmapped palette index refuses it), that the entries are the cave's,
/// and that the castle's palette maps.
fn assert_maps_the_structures(flags: &[&str]) {
    let cave = st::Library::load(Path::new(st::LIBRARY), &world_table(), &base_table());
    let table = table_of_run(flags);
    let args = crate::cli::parse_args_from(crate::test_support::argv(flags)).unwrap();
    let base = crate::setup::base_table(&args, &crate::setup::materials(&args));
    let lib = st::Library::load(Path::new(st::LIBRARY), &table, &base);
    assert_eq!(lib.entries.len(), cave.entries.len(), "{flags:?}");
    for (e, c) in lib.entries.iter().zip(&cave.entries) {
        assert_eq!(e.error(), None, "{flags:?}: {}", e.name);
        assert_eq!(e.name, c.name, "{flags:?}");
    }
    let chapel = &lib.entries[structure_index(&lib, "chapel")];
    assert!(chapel.scene().is_some(), "{flags:?}: the chapel's scene");
    assert!(
        st::castle_map(&table).is_ok(),
        "{flags:?}: the castle's palette maps"
    );
}

/// The streamed cave's and the outdoor hills' worlds hold the structures library's rows and
/// the castle's.
#[test]
fn the_cave_and_outdoor_worlds_map_the_structures() {
    assert_maps_the_structures(&[]);
    assert_maps_the_structures(&["--source", "outdoor"]);
}

/// `--castle`'s world holds the structures library's rows and the castle's.
#[test]
fn the_castle_world_maps_the_structures() {
    assert_maps_the_structures(&["--castle"]);
}

/// The showcase's world holds the structures library's rows and the castle's.
#[test]
fn the_showcase_world_maps_the_structures() {
    assert_maps_the_structures(&["--source", "showcase"]);
}

fn structure_index(lib: &st::Library, name: &str) -> usize {
    lib.entries.iter().position(|e| e.name == name).expect(name)
}

/// A scene of `placements` (prefab index, minimum corner, turn) of `prefabs`, its bounds
/// their turned boxes'.
fn scene_of(prefabs: Vec<Prefab>, placements: Vec<(usize, [i32; 3], Rot)>) -> VoxScene {
    let mut hi = [i32::MIN; 3];
    for &(i, o, r) in &placements {
        let s = r.apply_size(prefabs[i].size());
        for k in 0..3 {
            hi[k] = hi[k].max(o[k] + s[k] as i32 - 1);
        }
    }
    VoxScene {
        prefabs: prefabs.into_iter().map(Arc::new).collect(),
        placements: placements
            .into_iter()
            .map(|(i, o, r)| (i, VoxelCoord(o), r))
            .collect(),
        layers: Vec::new(),
        bounds: ([0; 3], hi),
    }
}

/// The coarse icon model at one voxel a voxel: a 3 x 2 x 2 prefab of twelve materials under
/// each of the 24 turns, in a 4 x 6 grid 4 voxels apart (24 parts, over the model's 16, so
/// `scene_model` makes none), reads back every placement's voxels as `Prefab::rotated` turns
/// them, and nothing else.
#[test]
fn the_coarse_icon_turns_as_the_stamp() {
    let p = Prefab::from_fn([3, 2, 2], |l| Some((1 + l[0] + 3 * l[1] + 6 * l[2]) as u16));
    let turns: Vec<Rot> = Rot::all().collect();
    assert_eq!(turns.len(), 24);
    let placements: Vec<_> = (0..24)
        .map(|i| (0, [4 * (i % 4), 4 * (i / 4), 0], turns[i as usize]))
        .collect();
    let scene = scene_of(vec![p.clone()], placements.clone());
    assert!(st::scene_model(&scene).is_none(), "24 parts fit no model");
    let model = st::icon_model(&scene).expect("a coarse model");
    assert_eq!(model.parts().len(), 1);
    let v = &model.parts()[0].volume;
    assert_eq!(v.size(), scene.size());
    let mut written = 0;
    for (_, o, r) in placements {
        let t = p.rotated(r);
        let s = t.size().map(|n| n as i32);
        for z in 0..s[2] {
            for y in 0..s[1] {
                for x in 0..s[0] {
                    let at = [o[0] + x, o[1] + y, o[2] + z];
                    assert_eq!(v.get(at), t.get([x, y, z]), "{r:?} at {at:?}");
                    written += 1;
                }
            }
        }
    }
    assert_eq!(v.written_count(), written, "nothing outside the placements");
}

/// A scene over [`MAX_EDGE`] a side: a 200 x 10 x 10 bar of material 3 under a 1 x 1 x 1 cube
/// of material 5 at its far end. Its icon model is one part, 4 voxels a voxel (50 x 3 x 3),
/// every voxel written: 3 where the bar's samples land, 5 in the cell the cube's sample
/// (its own corner) leads.
#[test]
fn the_coarse_icon_shrinks_a_large_scene() {
    let bar = Prefab::uniform([200, 10, 10], 3);
    let cube = Prefab::uniform([1, 1, 1], 5);
    let scene = scene_of(
        vec![bar, cube],
        vec![
            (0, [0, 0, 0], Rot::IDENTITY),
            (1, [196, 0, 0], Rot::IDENTITY),
        ],
    );
    assert!(
        st::scene_model(&scene).is_none(),
        "200 voxels is over the edge"
    );
    let model = st::icon_model(&scene).expect("a coarse model");
    let v = &model.parts()[0].volume;
    assert_eq!(v.size(), [50, 3, 3]);
    assert_eq!(v.written_count(), 50 * 3 * 3);
    assert_eq!(v.get([49, 0, 0]), Some(5), "the later placement first");
    assert_eq!(v.get([48, 0, 0]), Some(3));
    assert_eq!(v.get([49, 1, 0]), Some(3));
}

/// The castle's build carries its icon: on the run's first selection of the castle the build
/// starts, and once it arrives the castle's belt slot has an icon, a coarse model of its
/// 600 x 600 x 500 voxel box (over the model's limits) at 10 voxels a voxel.
#[test]
fn the_built_castle_has_an_icon() {
    let mut run = structure_run();
    look(&mut run, EYE, -40.0);
    let castle = structure(&run, st::CASTLE_NAME);
    run.fill_belt();
    let slot = run
        .modes
        .walkers
        .belt
        .slots
        .iter()
        .position(|s| *s == Some(Item::Structure(castle)))
        .expect("the castle in the belt");
    enter_slot(&mut run, slot);
    run.modes.structures.wait();
    let scene = run.modes.structures.entries[castle]
        .scene()
        .expect("built")
        .clone();
    assert!(st::scene_model(&scene).is_none());
    let model = st::icon_model(&scene).expect("a coarse model");
    assert_eq!(
        model.parts()[0].volume.size(),
        scene.size().map(|v| v.div_ceil(10))
    );
    run.poll_viewer(0.0);
    run.modes.walkers.icons.wait();
    let tile = belt::tile_of(&run.modes, Item::Structure(castle));
    assert!(run.modes.walkers.icons.ready(tile), "the castle's icon");
}

/// The written, non-air voxels of `scene` turned `quarters` quarter turns, with its box's
/// minimum corner at the origin: each placement's prefab turned as placed
/// (`Prefab::rotated`) at its place, a later placement's voxel over an earlier one's.
fn placed_voxels(scene: &VoxScene, quarters: u8) -> BTreeMap<[i32; 3], vxe_world::MaterialId> {
    let (lo, _) = scene.turned_bounds(quarters);
    let mut out = BTreeMap::new();
    for (p, o, r) in scene.placed_at([0; 3], quarters) {
        let at = [0, 1, 2].map(|k| o.0[k] - lo[k]);
        out.extend(voxels(&p.rotated(r), at));
    }
    out
}

/// The written, non-air voxels of `p`, moved by `at`.
fn voxels(p: &Prefab, at: [i32; 3]) -> BTreeMap<[i32; 3], vxe_world::MaterialId> {
    let s = p.size().map(|v| v as i32);
    let mut out = BTreeMap::new();
    for z in 0..s[2] {
        for y in 0..s[1] {
            for x in 0..s[0] {
                if let Some(m) = p.get([x, y, z]).filter(|&m| m != AIR) {
                    out.insert([at[0] + x, at[1] + y, at[2] + z], m);
                }
            }
        }
    }
    out
}

/// Phase 7b done-when 3: the chapel's slot stands one ghost of 13 parts, part 0 a 48 x 32 x 40
/// body at `coarse` 1 whose voxels are the chapel's scene placed at the origin, parts 1 to 12
/// its frame's edges, with its corner at the box rule's and the ghost colour's mark; after `R`
/// the body is 32 x 48 x 40 and is the scene turned once, the frame's extents swapped; at
/// `structure scale` 2 the ghost is 9 parts, its body 48 x 32 x 40 at `coarse` 2 inside the
/// 96 x 64 x 80 frame's eight corners; at scale 4 the frame stands alone until the body
/// (64 x 43 x 54 at `coarse` 3) arrives from its worker; the castle's ghost, once built, is 9 parts, its body
/// 60 x 60 x 50 at `coarse` 10 and its icon's volume. The world's voxels never change.
#[test]
fn the_ghost_is_a_coarse_copy_in_its_frame() {
    let mut run = structure_run();
    let hash = run.world.voxel_hash();
    let (chapel, castle) = (structure(&run, "chapel"), structure(&run, st::CASTLE_NAME));
    look(&mut run, EYE, -40.0);
    enter_slot(&mut run, 2);
    assert_eq!(run.modes.walkers.belt.structure(), Some(chapel));
    let ghost = |run: &Run| run.modes.walkers.frame.clone().expect("a ghost");
    let scene = run.modes.structures.entries[chapel]
        .scene()
        .expect("the chapel's scene")
        .clone();

    let f = ghost(&run);
    let parts = f.model.parts();
    assert_eq!(parts.len(), 13);
    let body = &parts[0];
    assert_eq!(
        (body.volume.size(), body.coarse, body.detail, body.pivot),
        ([48, 32, 40], 1, 1, [0; 3])
    );
    assert_eq!(voxels(&body.volume, [0; 3]), placed_voxels(&scene, 0));
    assert!(parts[1..].iter().all(|p| p.coarse == 1 && p.detail == 1));
    assert_eq!(frame_size(&run), [48, 32, 40]);
    assert_eq!(f.corner, run.aim_corner([48, 32, 40]));
    let w = &mut run.modes.walkers;
    w.refresh_displays();
    let colour = vxe_render::MARK_COLOURS[0];
    assert!(
        w.marks().iter().any(|(i, m)| *i == f.id
            && m.outline
            && m.colour == colour
            && m.tint == crate::entities::placed::GHOST_TINT),
        "{:?}",
        w.marks()
    );

    frame_keys(&mut run, &pressed(KeyCode::KeyR));
    let f = ghost(&run);
    let body = &f.model.parts()[0];
    assert_eq!((body.volume.size(), body.coarse), ([32, 48, 40], 1));
    assert_eq!(voxels(&body.volume, [0; 3]), placed_voxels(&scene, 1));
    assert_eq!(frame_size(&run), [32, 48, 40]);
    assert_eq!(f.corner, run.aim_corner([32, 48, 40]));
    for _ in 0..3 {
        frame_keys(&mut run, &pressed(KeyCode::KeyR));
    }

    let scale = crate::toggles::TOGGLES
        .iter()
        .find(|t| t.name == "structure scale")
        .expect("the toggle");
    run.menu_act(Activated::Step(scale, 1));
    run.sync_ghost();
    assert_eq!(run.modes.structure_scale, 2);
    let f = ghost(&run);
    assert_eq!(f.model.parts().len(), 9);
    let body = &f.model.parts()[0];
    assert_eq!((body.volume.size(), body.coarse), ([48, 32, 40], 2));
    assert_eq!(frame_size(&run), [96, 64, 80]);
    assert_eq!(f.corner, run.aim_corner([96, 64, 80]));
    // At scale 4 the box (192 x 128 x 160) is over `FRAME_BODY_VOXELS`: the frame stands alone
    // until the body arrives from its worker, then the ghost is remade with it.
    run.menu_act(Activated::Step(scale, 1));
    run.menu_act(Activated::Step(scale, 1));
    run.sync_ghost();
    assert_eq!(run.modes.structure_scale, 4);
    assert_eq!(ghost(&run).model.parts().len(), 8, "the frame alone");
    let table = run.world.materials().clone();
    let made = run.modes.structures.wait_body(chapel, 4, &table);
    assert!(matches!(made, Ok(Some(_))), "the body at scale 4");
    run.sync_ghost();
    let f = ghost(&run);
    assert_eq!(f.model.parts().len(), 9);
    let body = &f.model.parts()[0];
    assert_eq!((body.volume.size(), body.coarse), ([64, 43, 54], 3));
    assert_eq!(frame_size(&run), [192, 128, 160]);
    for _ in 0..3 {
        run.menu_act(Activated::Step(scale, -1));
    }
    assert_eq!(run.modes.structure_scale, 1);

    run.fill_belt();
    let slot = run
        .modes
        .walkers
        .belt
        .slots
        .iter()
        .position(|s| *s == Some(Item::Structure(castle)))
        .expect("the castle in the belt");
    enter_slot(&mut run, slot);
    run.modes.structures.wait();
    run.sync_ghost();
    let f = ghost(&run);
    assert_eq!(f.model.parts().len(), 9);
    let body = &f.model.parts()[0];
    assert_eq!((body.volume.size(), body.coarse), ([60, 60, 50], 10));
    let built = run.modes.structures.entries[castle]
        .scene()
        .expect("built")
        .clone();
    let icon = st::icon_model(&built).expect("the castle's icon");
    assert_eq!(*icon.parts()[0].volume, *body.volume);
    assert_eq!(f.corner, run.aim_corner([600, 600, 500]));
    assert_eq!(run.world.voxel_hash(), hash, "a ghost writes no voxel");
}

mod reading {
    use super::*;

    /// The ghost's body: per case its size, `k`, pool bytes
    /// (`vxe_render::EntityMirror::volume_bytes`) and make time (`structures::coarse_volume`,
    /// the median of 5): the chapel at scales 1 and 4, the castle at scale 1. Printed, not
    /// asserted. Run by hand:
    /// `--run-ignored only -E 'test(=run::structure_tests::reading::the_ghost_body_costs)'`.
    #[test]
    #[ignore = "a reading"]
    fn the_ghost_body_costs() {
        let mut run = structure_run();
        let table = run.world.materials().clone();
        let (chapel, castle) = (structure(&run, "chapel"), structure(&run, st::CASTLE_NAME));
        let lib = &mut run.modes.structures;
        lib.build_castle(castle, &table);
        lib.wait();
        let scenes = [
            (
                "chapel x1",
                lib.scene_at(chapel, 1, &table).expect("chapel"),
            ),
            (
                "chapel x4",
                lib.scene_at(chapel, 4, &table).expect("chapel x4"),
            ),
            (
                "castle x1",
                lib.scene_at(castle, 1, &table).expect("castle"),
            ),
        ];
        for (name, scene) in scenes {
            let mut ms: Vec<f64> = Vec::new();
            let mut body = None;
            for _ in 0..5 {
                let t = crate::now();
                body = st::coarse_volume(&scene);
                ms.push((crate::now() - t).as_secs_f64() * 1e3);
            }
            ms.sort_by(f64::total_cmp);
            let (v, k) = body.expect("a body");
            println!(
                "ghost body {name}: {:?} at k {k}, {} B in the pool, made in {:.1} ms (median of \
                 5, min {:.1}, max {:.1})",
                v.size(),
                vxe_render::EntityMirror::volume_bytes(&v),
                ms[2],
                ms[0],
                ms[4]
            );
        }
    }
}
