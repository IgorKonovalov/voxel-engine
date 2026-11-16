//! The run's tests of the menu's place in the frame: the key step's `Esc`, the capture of the
//! dig and the wheel, the tester's filter in the loop, and the overlay's choice.

use super::*;
use crate::hud::HudRow;
use crate::hud::menu::{PageId, all_keys};
use crate::path::{TesterInput, tester_action};
use crate::run::hud_feed::Overlay;
use crate::test_support::{bare_run, carved_rock};
use vxe_app::{Flow, Input, KeyCode, MouseButton};
use vxe_world::OpResult;

/// The ops one server tick of `run` applied.
fn tick_ops(run: &mut Run) -> usize {
    let net = run.net.as_mut().unwrap();
    assert_eq!(net.server.tick().logic_error, None);
    crate::test_support::applied(&net.client.poll())
        .into_iter()
        .filter(|r| matches!(r, OpResult::Edit(_)))
        .count()
}

/// One frame's input step on `run`: the key step, then interactive flight with what passed.
fn frame_keys(run: &mut Run, input: &Input) -> Option<Flow> {
    let (pass, exit) = run.key_step(input);
    if exit.is_none() {
        run.flight_step(pass.input(), 1.0 / 60.0);
    }
    exit
}

fn pressed(k: KeyCode) -> Input {
    let mut i = Input::default();
    i.key_down(k);
    i
}

/// Every HUD row's words and the lighting modes: what a key that acts would change.
fn rows(run: &Run) -> Vec<String> {
    let mut out: Vec<String> = HudRow::ALL.iter().map(|&r| run.modes.row(r)).collect();
    out.push(format!("{:?}", run.modes.lighting));
    out
}

/// A key step with `Esc` and the menu closed ends the run; with it open `Esc` closes the menu
/// (a filter first, then a page) and the run goes on. While open the left click does not dig and the wheel
/// does not change the dig radius; closed, both do.
#[test]
fn esc_closed_ends_the_run_and_open_never_does() {
    let mut run = bare_run(carved_rock());
    assert_eq!(
        frame_keys(&mut run, &pressed(KeyCode::Escape)),
        Some(Flow::Exit)
    );

    let mut run = bare_run(carved_rock());
    assert_eq!(frame_keys(&mut run, &pressed(KeyCode::Tab)), None);
    assert!(run.menu.is_open());
    assert_eq!(frame_keys(&mut run, &pressed(KeyCode::KeyL)), None);
    assert_eq!(run.menu.filter(), "l");
    assert_eq!(frame_keys(&mut run, &pressed(KeyCode::Escape)), None);
    assert!(run.menu.is_open() && run.menu.filter().is_empty());
    let radius = run.modes.dig_radius;
    let mut click = Input::default();
    click.button_down(MouseButton::Left);
    click.add_scroll(2.0);
    assert_eq!(frame_keys(&mut run, &click), None);
    assert_eq!(tick_ops(&mut run), 0, "no dig while open");
    assert_eq!(run.modes.dig_radius, radius, "no dig radius while open");
    // The click is the menu's `Enter`: it opened the first group.
    assert_eq!(
        run.menu.page(),
        Some(PageId::Group(crate::hud::menu::Group::Moving))
    );
    for _ in 0..2 {
        assert_eq!(frame_keys(&mut run, &pressed(KeyCode::Escape)), None);
    }
    assert!(!run.menu.is_open());
    assert_eq!(frame_keys(&mut run, &click), None);
    assert_eq!(tick_ops(&mut run), 1, "closed, the click digs");
    assert_ne!(
        run.modes.dig_radius, radius,
        "closed, the wheel steps the radius"
    );
    assert_eq!(
        frame_keys(&mut run, &pressed(KeyCode::Escape)),
        Some(Flow::Exit)
    );
}

/// Under `--tester` the frame loop's input step passes on only what `tester_action` maps:
/// every other key the fly-through knows (the debug keys, `Tab`, the typed letters) changes
/// no mode, opens no menu, starts no readings, shows no stats and edits nothing; the left
/// click still digs and `F1` still flips the tester's help.
#[test]
fn the_tester_loop_reaches_the_keys_step_with_the_filter_s_keys_only() {
    let mut run = bare_run(carved_rock());
    run.args.tester = true;
    let start = rows(&run);
    let visible = run.hud.visible;
    let mut dropped = 0;
    for k in all_keys() {
        if tester_action(TesterInput::Key(k)).is_some() {
            continue;
        }
        dropped += 1;
        assert_eq!(frame_keys(&mut run, &pressed(k)), None, "{k:?}");
        assert_eq!(rows(&run), start, "{k:?} changed a mode");
        assert!(!run.menu.is_open(), "{k:?} opened the menu");
        assert!(run.samples.is_none(), "{k:?} started the readings");
        assert!(run.walk.is_none(), "{k:?} walks");
        assert_eq!(run.hud.visible, visible, "{k:?}");
        assert_eq!(tick_ops(&mut run), 0, "{k:?} edited the world");
    }
    assert!(dropped > 50, "{dropped}");
    let mut click = Input::default();
    click.button_down(MouseButton::Left);
    frame_keys(&mut run, &click);
    assert_eq!(tick_ops(&mut run), 1, "the left click digs");
    assert!(!run.help);
    frame_keys(&mut run, &pressed(KeyCode::F1));
    assert!(run.help, "F1 shows the tester's help");
}

/// The overlay is one function of (tester, menu open, stats shown): outside `--tester` the
/// open menu's page alone whatever the stats; else the stats or the banner. `F1` twice opens
/// and closes the Keys page and leaves the stats as they were; `F2` with the menu open still
/// flips them.
#[test]
fn the_overlay_is_chosen_by_tester_menu_and_stats() {
    use Overlay::*;
    for (tester, menu, stats, want) in [
        (false, false, false, Banner),
        (false, false, true, Stats),
        (false, true, false, Menu),
        (false, true, true, Menu),
        (true, false, false, Banner),
        (true, false, true, Stats),
        (true, true, false, Banner),
        (true, true, true, Stats),
    ] {
        assert_eq!(
            Overlay::of(tester, menu, stats),
            want,
            "{tester} {menu} {stats}"
        );
    }
    let mut run = bare_run(carved_rock());
    run.hud = super::hud_feed::Hud::new(true);
    frame_keys(&mut run, &pressed(KeyCode::F1));
    assert_eq!(run.menu.page(), Some(PageId::Keys));
    run.write_menu();
    assert!(
        run.hud.text.starts_with("menu  >  keys"),
        "{}",
        run.hud.text
    );
    frame_keys(&mut run, &pressed(KeyCode::F1));
    assert!(!run.menu.is_open() && run.hud.visible);
    frame_keys(&mut run, &pressed(KeyCode::Tab));
    frame_keys(&mut run, &pressed(KeyCode::F2));
    assert!(run.menu.is_open() && !run.hud.visible);
}
