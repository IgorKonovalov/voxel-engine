//! The run's tests of the entities page (Plan 0077 Phase 3): the preview at the crosshair and
//! its placement, the remove key, the one panel, the caps, the pad's way to the page, and the
//! tester's filter. A rock box with the walker's rows around the camera, the committed
//! library scanned against the cave's table (the walker builds; the spider is in error).

use super::*;
use crate::entities::library;
use crate::hud::menu::{Activated, PageId};
use crate::run::hud_feed::Overlay;
use crate::test_support::bare_run;
use vxe_app::{Input, KeyCode, PadButton};
use vxe_world::{ChunkExtent, EditOp, FIXED_ONE, Fixed3, Mode, World};

/// A rock box with the cave's rows and the walker's, hollow over `[-8, 60) x [-30, 30) x
/// [-8, 30)`: the camera at 0.5 looks along +x at the far wall, 59.5 voxels away.
fn walker_box() -> World {
    let table = crate::walker::with_walker(&crate::cave_materials());
    let world = World::generate(
        ChunkExtent::new([-1, -1, -1], [2, 1, 1]),
        &vxe_world::UniformSource(crate::cave::ROCK),
        &table,
    );
    world
        .apply(&EditOp::box_(
            Fixed3([-8, -30, -8].map(|v: i64| v * FIXED_ONE)),
            Fixed3([60, 30, 30].map(|v: i64| v * FIXED_ONE)),
            Mode::Carve,
        ))
        .expect("resident");
    world
}

/// A run in [`walker_box`] with the committed library built against its table.
fn picker_run() -> Run {
    let mut run = bare_run(walker_box());
    let lib = library::scan(std::path::Path::new(library::DEFAULT_DIR));
    run.modes.walkers = crate::entities::Walkers::default().with_library(lib);
    run.poll_viewer(0.0);
    run
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

fn pad(b: PadButton) -> Input {
    let mut i = Input::default();
    i.pad_down(b);
    i
}

/// The notice row's words, when one is shown.
fn notice(run: &Run) -> Option<&str> {
    run.notice.as_ref().map(|(l, _)| l.as_str())
}

/// Phase 3 done-when: `` ` `` shows the entities page and one preview (the selected walker)
/// at the crosshair's world surface facing the camera, not one of the placements; `Enter`
/// leaves that entity placed under the same id on the entry's clip, the page open and a new
/// preview standing; `Esc` removes the preview and leaves the placements as they were. The
/// placement point is the world's hit with the preview standing there.
#[test]
fn the_preview_is_placed_where_it_stands() {
    let mut run = picker_run();
    let walker = run.modes.walkers.library.selected;
    assert_eq!(run.modes.walkers.library.entries[walker].name, "walker");
    let (feet, eye) = run.aim_feet();
    assert!((feet[0] - 60.0).abs() < 1e-3, "the far wall: {feet:?}");
    frame_keys(&mut run, &pressed(KeyCode::Backquote));
    assert_eq!(run.menu.page(), Some(PageId::Entities));
    let p = run.modes.walkers.preview.clone().expect("a preview");
    assert_eq!((p.entry, p.feet), (Some(walker), feet));
    assert_eq!(p.yaw, crate::walker::facing(feet, eye));
    assert!(run.modes.walkers.placed.is_empty());
    assert_eq!(run.modes.walkers.set.len(), 1);
    assert_eq!(run.aim_feet().0, feet, "the place ray is the world's");

    frame_keys(&mut run, &pressed(KeyCode::Enter));
    let w = &run.modes.walkers;
    assert_eq!(w.placed.len(), 1);
    assert_eq!((w.placed[0].id, w.placed[0].feet), (p.id, feet));
    assert_eq!(w.placed[0].play.clip, 0, "the entry's clip, idle");
    assert_eq!(notice(&run), Some("placed walker #1 (idle)"));
    assert_eq!(
        run.menu.page(),
        Some(PageId::Entities),
        "the page stays open"
    );
    let next = w.preview.as_ref().expect("a new preview").id;
    assert_ne!(next, p.id);
    assert_eq!(w.set.len(), 2);

    frame_keys(&mut run, &pressed(KeyCode::Escape));
    assert!(!run.menu.is_open());
    let w = &run.modes.walkers;
    assert!(w.preview.is_none() && w.set.get(next).is_none());
    assert_eq!((w.placed.len(), w.set.len()), (1, 1));

    // `` ` `` `` ` ``: the page, then a placement that closes it.
    frame_keys(&mut run, &pressed(KeyCode::Backquote));
    frame_keys(&mut run, &pressed(KeyCode::Backquote));
    assert!(!run.menu.is_open());
    let w = &run.modes.walkers;
    assert_eq!((w.placed.len(), w.set.len()), (2, 2));
    assert!(w.preview.is_none());
}

/// Three walkers standing on the floor across the view, the second dead ahead; `4`'s marks
/// pushed as a placement pushes them.
fn three_in_a_row(run: &mut Run) -> Vec<vxe_entity::EntityId> {
    let table = run.world.materials().clone();
    let walker = run.modes.walkers.library.selected;
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

/// Phase 3 done-when: with three placed and the crosshair on the second, `Delete` removes
/// the second only, with the menu closed and with the entities page open; `4` afterwards
/// removes the third.
#[test]
fn the_remove_key_takes_the_aimed_entity() {
    for open in [false, true] {
        let mut run = picker_run();
        let ids = three_in_a_row(&mut run);
        if open {
            run.act(crate::toggles::Act::Picker);
            assert!(run.modes.walkers.preview.is_some());
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
        assert_eq!(placed(&run), [ids[0], ids[2]], "open {open}");
        assert!(run.modes.walkers.set.get(ids[1]).is_none());
        assert_eq!(run.menu.is_open(), open);
        if open {
            // `4` types into the open menu's search: close it first.
            frame_keys(&mut run, &pressed(KeyCode::Escape));
        }
        frame_keys(&mut run, &pressed(KeyCode::Digit4));
        assert_eq!(placed(&run), [ids[0]], "open {open}");
    }
}

/// Phase 3 done-when: from the Keys page `` ` `` shows the entities page with its preview;
/// `F1` from it shows the Keys page and removes the preview; `Esc` closes, and the stats
/// overlay is as `F2` left it, in both states of it.
#[test]
fn the_overlay_shows_one_panel() {
    for stats in [false, true] {
        let mut run = picker_run();
        run.hud.visible = stats;
        frame_keys(&mut run, &pressed(KeyCode::F1));
        assert_eq!(run.menu.page(), Some(PageId::Keys));
        frame_keys(&mut run, &pressed(KeyCode::Backquote));
        assert_eq!(run.menu.page(), Some(PageId::Entities));
        assert!(run.modes.walkers.preview.is_some());
        assert_eq!(Overlay::of(false, run.menu.is_open(), stats), Overlay::Menu);
        frame_keys(&mut run, &pressed(KeyCode::F1));
        assert_eq!(run.menu.page(), Some(PageId::Keys));
        assert!(run.modes.walkers.preview.is_none());
        assert_eq!(run.modes.walkers.set.len(), 0);
        frame_keys(&mut run, &pressed(KeyCode::Escape));
        assert!(!run.menu.is_open());
        assert_eq!(run.hud.visible, stats);
    }
}

/// Phase 3 done-when: with 64 entities placed the page shows no preview and the notice row
/// reads `entities 64/64 full`; `Enter` places nothing.
#[test]
fn the_caps_hold_with_a_preview() {
    let mut run = picker_run();
    let table = run.world.materials().clone();
    let walker = run.modes.walkers.library.selected;
    for i in 0..64 {
        let feet = [(i % 8) as f32 * 6.0, (i / 8) as f32 * 6.0 - 24.0, -8.0];
        run.modes
            .walkers
            .place_entry(&table, Some(walker), feet, 0.0)
            .expect("placed");
    }
    run.act(crate::toggles::Act::Picker);
    assert_eq!(run.menu.page(), Some(PageId::Entities));
    assert!(run.modes.walkers.preview.is_none());
    assert_eq!(notice(&run), Some("entities 64/64 full"));
    frame_keys(&mut run, &pressed(KeyCode::Enter));
    assert_eq!(run.modes.walkers.placed.len(), 64);
    assert_eq!(run.modes.walkers.set.len(), 64);
    assert_eq!(notice(&run), Some("entities 64/64 full"));
}

/// Phase 3 done-when: on the pad, `Start` opens the root, the D-pad walks to the `entities`
/// row and `South` opens the page with its preview, `South` places, `East` goes back to the
/// root (the preview gone) and `East` again closes.
#[test]
fn the_pad_places_from_the_root() {
    let mut run = picker_run();
    frame_keys(&mut run, &pad(PadButton::Start));
    assert_eq!(run.menu.page(), Some(PageId::Root));
    let at = crate::hud::menu::Group::ALL
        .iter()
        .position(|&g| g == crate::hud::menu::Group::Entities)
        .expect("a group");
    for _ in 0..at {
        frame_keys(&mut run, &pad(PadButton::DpadDown));
    }
    frame_keys(&mut run, &pad(PadButton::South));
    assert_eq!(run.menu.page(), Some(PageId::Entities));
    assert!(run.modes.walkers.preview.is_some());
    frame_keys(&mut run, &pad(PadButton::South));
    assert_eq!(run.modes.walkers.placed.len(), 1);
    frame_keys(&mut run, &pad(PadButton::East));
    assert_eq!(run.menu.page(), Some(PageId::Root));
    assert!(run.modes.walkers.preview.is_none());
    frame_keys(&mut run, &pad(PadButton::East));
    assert!(!run.menu.is_open());
    assert_eq!(run.modes.walkers.placed.len(), 1);
}

/// Phase 3 done-when: under `--tester` `` ` `` maps to no tester action, so the tester has no
/// picker; and an entry in error refuses `Enter` with its error in the notice row.
#[test]
fn the_tester_has_no_picker_and_an_entry_in_error_refuses() {
    use crate::path::{TesterInput, tester_action};
    assert_eq!(tester_action(TesterInput::Key(KeyCode::Backquote)), None);
    let mut run = picker_run();
    let spider = run
        .modes
        .walkers
        .library
        .entries
        .iter()
        .position(|e| e.name == "spider")
        .expect("the spider");
    run.act(crate::toggles::Act::Picker);
    run.menu_act(Activated::Place {
        entry: spider,
        close: false,
    });
    let words = notice(&run).expect("the refusal");
    assert!(
        words.starts_with("spider: ") && words.contains("crystal"),
        "{words}"
    );
    assert!(run.modes.walkers.placed.is_empty());
}

mod reading {
    use super::*;

    /// The entities page's format time (`hud.format_us`, the menu's `write_with`) over 60
    /// frames with the page open and its preview standing; printed, not asserted. Run by
    /// hand: `--run-ignored only -E 'test(=run::picker_tests::reading::the_page_formats)'`.
    #[test]
    #[ignore = "a reading"]
    fn the_page_formats() {
        let mut run = picker_run();
        frame_keys(&mut run, &pressed(KeyCode::Backquote));
        let mut us: Vec<u32> = (0..60)
            .map(|_| {
                frame_keys(&mut run, &Input::default());
                run.write_menu();
                run.hud.format_us
            })
            .collect();
        us.sort_unstable();
        println!(
            "entities page format_us over 60 frames: median {} min {} max {}",
            us[30], us[0], us[59]
        );
    }
}
