//! The run's tests of the chooser and placement mode (Plan 0077 Phases 3 and 5): the wheel,
//! the digits and the pad on the belt, the chooser's hand-over, where the ghost stands, the
//! clicks, the remove key, the one panel, the caps and the tester's filter. A rock box with
//! the walker's rows around the camera, the committed library scanned against the cave's table
//! (the walker builds; the spider is in error), unless a test names another.

use super::*;
use crate::entities::belt::{self, BELT_ROWS};
use crate::entities::library;
use crate::hud::menu::{Activated, PageId, Row};
use crate::run::hud_feed::Overlay;
use crate::test_support::bare_run;
use vxe_app::{Axis, Flow, Input, KeyCode, MouseButton, PadButton};
use vxe_world::{ChunkExtent, EditOp, FIXED_ONE, Fixed3, MaterialTable, Mode, OpResult, World};

/// A rock box of `table` hollow over `[-8, x_end) x [-30, 30) x [-8, 30)`: the floor's top at
/// z = -8, the far wall's face at x = `x_end`.
fn rock_box(table: &MaterialTable, x_end: i64) -> World {
    let world = World::generate(
        ChunkExtent::new([-1, -1, -1], [2, 1, 1]),
        &vxe_world::UniformSource(crate::cave::ROCK),
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
    run.fly.camera.position = vxe_render::glam::Vec3::from_array(at);
    run.fly.camera.yaw = 0.0;
    run.fly.camera.pitch = pitch_deg.to_radians();
}

/// Phase 5 done-when: closed, one wheel step enters the mode at slot 1 with the walker's ghost
/// standing and the dig radius unchanged; a second selects slot 2 (the spider, in error
/// under the cave's table: no ghost, its reason in the notice); eight more wrap to slot 1;
/// `3` selects the empty slot 3 (no ghost, the name row names it); `Esc` ends the mode and
/// the run goes on; a second `Esc` is the quit.
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
        &w.belt,
        &w.library,
        now,
        crate::hud::BANNER_MS,
    );
    let name_row = text.lines().nth(45 - BELT_ROWS).expect("the name row");
    assert_eq!(name_row.trim(), "slot 3 empty  (choose with `)");

    assert_eq!(step(&mut run, &pressed(KeyCode::Escape)), None);
    let w = &run.modes.walkers;
    assert!(!w.belt.on && w.preview.is_none() && w.set.is_empty());
    assert_eq!(step(&mut run, &pressed(KeyCode::Escape)), Some(Flow::Exit));
}

/// Phase 5 done-when: `` ` `` opens the chooser on the walker; `Down` to the spider, `2` with
/// the page open puts the spider in slot 2 and the page stays; `5` puts it in slot 5 (out of
/// slot 2); `Enter` on the walker selects the slot that holds it, closes the page and enters
/// the mode with the walker's ghost; `` ` `` in the mode opens the chooser again, the mode,
/// the belt and the ghost gone.
#[test]
fn the_chooser_fills_the_belt_and_hands_over() {
    let mut run = picker_run();
    let (walker, spider) = (entry(&run, "walker"), entry(&run, "spider"));
    frame_keys(&mut run, &pressed(KeyCode::Backquote));
    assert_eq!(run.menu.page(), Some(PageId::Entities));
    let v = run.run_view();
    let row = |run: &Run| run.menu.selected_row(&run.modes, &v);
    assert!(matches!(row(&run), Some(Row::Item(i)) if i == walker));
    frame_keys(&mut run, &pressed(KeyCode::ArrowDown));
    assert!(matches!(row(&run), Some(Row::Item(i)) if i == spider));
    frame_keys(&mut run, &pressed(KeyCode::Digit2));
    assert_eq!(run.modes.walkers.belt.slots[1], Some(spider));
    assert_eq!(run.menu.page(), Some(PageId::Entities));
    assert_eq!(notice(&run), Some("spider in slot 2"));
    frame_keys(&mut run, &pressed(KeyCode::Digit5));
    let b = &run.modes.walkers.belt;
    assert_eq!((b.slots[1], b.slots[4]), (None, Some(spider)));
    assert_eq!(run.menu.filter(), "");
    assert!(!b.on && run.modes.walkers.preview.is_none());

    // keys:typed-begin
    frame_keys(&mut run, &pressed(KeyCode::ArrowUp));
    // keys:typed-end
    frame_keys(&mut run, &pressed(KeyCode::Enter));
    assert!(!run.menu.is_open());
    let w = &run.modes.walkers;
    assert!(w.belt.on);
    assert_eq!((w.belt.selected, w.belt.entry()), (0, Some(walker)));
    assert_eq!(w.preview.as_ref().and_then(|p| p.entry), Some(walker));

    frame_keys(&mut run, &pressed(KeyCode::Backquote));
    assert_eq!(run.menu.page(), Some(PageId::Entities));
    let w = &run.modes.walkers;
    assert!(!w.belt.on && w.preview.is_none() && w.set.is_empty());
}

/// The ghost's feet with the camera at `at`, `pitch_deg` above the horizon along +x, in
/// placement mode at the walker's slot (or the slot holding `name`).
fn ghost_feet(run: &mut Run, name: &str, at: [f32; 3], pitch_deg: f32) -> [f32; 3] {
    look(run, at, pitch_deg);
    let i = entry(run, name);
    run.fill_belt();
    let s = run.modes.walkers.belt.slot_of(i).expect("a slot");
    run.act(crate::toggles::Act::BeltSlot(s as u8 + 1));
    run.modes.walkers.preview.as_ref().expect("a ghost").feet
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
    let mark = |colour| vxe_render::Mark {
        colour,
        outline: true,
        tint: 0.35,
    };
    let g = ghost(&run);
    run.modes.walkers.refresh_displays();
    assert_eq!(
        run.modes.walkers.marks(),
        [(g, mark(vxe_render::MARK_COLOURS[0]))]
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

/// Phase 5 done-when: in the mode at 1280 x 720 (80 x 45 cells at the HUD's scale 2) the
/// belt's rows are the overlay's last seven, centred: the name row, slot 1's frame top
/// (yellow), the nine slots' digits, their names, two blank rows, the frame's bottom; slot 1
/// framed by `+ - |`, the backdrop's box under the slot rows; the name row reads `walker` for
/// `BANNER_MS` after a wheel step and is blank after; out of the mode the overlay has no belt
/// rows and the stats overlay is as `F2` left it.
#[test]
fn the_belt_draws_its_slots() {
    let mut run = picker_run();
    let stats = run.hud.visible;
    frame_keys(&mut run, &wheel_down());
    let w = &run.modes.walkers;
    let at = w.belt.changed_at.expect("a change");
    let draw = |now: f64| {
        let mut text = String::from("row a\nrow b");
        let mut colours = vec![vxe_render::HudColour::White; 2];
        let b = belt::append_belt(
            &mut text,
            &mut colours,
            (80, 45),
            &w.belt,
            &w.library,
            now,
            crate::hud::BANNER_MS,
        );
        (text, colours, b)
    };
    let (text, colours, b) = draw(at + crate::hud::BANNER_MS - 1.0);
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
    use vxe_render::HudColour::{White, Yellow};
    assert_eq!(
        &colours[45 - BELT_ROWS..],
        [Yellow, Yellow, White, White, White, White, Yellow]
    );
    assert_eq!(
        b,
        vxe_render::HudBox {
            col: 3,
            row: 39,
            cols: 74,
            rows: 6
        }
    );
    let (text, _, _) = draw(at + crate::hud::BANNER_MS + 1.0);
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
/// chooser open; `4` afterwards removes the third (in the mode too).
#[test]
fn the_remove_key_takes_the_aimed_entity() {
    for state in ["closed", "placing", "open"] {
        let mut run = picker_run();
        let ids = three_in_a_row(&mut run);
        match state {
            "placing" => run.act(crate::toggles::Act::BeltSlot(3)),
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
        assert_eq!(placed(&run), [ids[0]], "{state}");
    }
}

/// Three walkers standing on the floor across the view, the second dead ahead; `4`'s marks
/// pushed as a placement pushes them.
fn three_in_a_row(run: &mut Run) -> Vec<vxe_entity::EntityId> {
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
    run.menu_act(Activated::Choose { entry: spider });
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
