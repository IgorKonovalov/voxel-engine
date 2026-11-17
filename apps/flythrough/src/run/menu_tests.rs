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
    assert_eq!(frame_keys(&mut run, &pressed(KeyCode::KeyA)), None);
    assert_eq!(run.menu.filter(), "a");
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
    assert!(dropped > 30, "{dropped}");
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

/// The keys the menu-only entries had before the minimal key set (ADR-0061's table, the Menu
/// rows): text here, bound to nothing now.
// keys:typed-begin
const FORMER_KEYS: [KeyCode; 41] = [
    KeyCode::F3,
    KeyCode::F4,
    KeyCode::F5,
    KeyCode::F6,
    KeyCode::F8,
    KeyCode::KeyM,
    KeyCode::KeyQ,
    KeyCode::Comma,
    KeyCode::Semicolon,
    KeyCode::Delete,
    KeyCode::Insert,
    KeyCode::KeyN,
    KeyCode::KeyV,
    KeyCode::KeyZ,
    KeyCode::Period,
    KeyCode::KeyF,
    KeyCode::Slash,
    KeyCode::Quote,
    KeyCode::Backspace,
    KeyCode::Backslash,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit5,
    KeyCode::KeyL,
    KeyCode::KeyI,
    KeyCode::KeyB,
    KeyCode::KeyP,
    KeyCode::KeyR,
    KeyCode::KeyK,
    KeyCode::F11,
    KeyCode::F12,
    KeyCode::KeyH,
    KeyCode::KeyJ,
    KeyCode::KeyE,
    KeyCode::KeyY,
    KeyCode::BracketLeft,
    KeyCode::BracketRight,
    KeyCode::Minus,
    KeyCode::Equal,
];
// keys:typed-end

/// Everything a menu-only entry's former key could change: the rows and lighting, the menu,
/// the readings, walking, the castle shot, the walkers' switches, the lights' count and the
/// world (an edit).
fn state(
    run: &mut Run,
) -> (
    Vec<String>,
    bool,
    bool,
    bool,
    Option<&'static str>,
    [bool; 2],
    u32,
) {
    let w = [run.modes.walkers.walking, run.modes.walkers.eye_lights_off];
    (
        rows(run),
        run.menu.is_open(),
        run.samples.is_some(),
        run.walk.is_some(),
        run.castle_shot.map(|c| c.name()),
        w,
        tick_ops(run) as u32,
    )
}

/// With the menu closed, a key step with a menu-only entry's former key pressed changes
/// nothing (the modes, the menu, the readings, walking, the castle shot, the walkers, the
/// world); the same entry through `Tab`, its words and `Enter` (the run's `menu_act`) changes
/// what the key did.
#[test]
fn a_menu_only_entry_has_no_key() {
    let mut run = bare_run(carved_rock());
    run.castle_shot = Some(crate::castle::CastleShot::Gate);
    let before = state(&mut run);
    for k in FORMER_KEYS {
        assert_eq!(frame_keys(&mut run, &pressed(k)), None, "{k:?}");
        assert_eq!(state(&mut run), before, "{k:?} acted");
    }
    let lamp_before = rows(&run);
    // `Tab`, `shadow walk`'s first letters, `Enter`: the first match is the shadow walk.
    // keys:typed-begin
    for k in [
        KeyCode::Tab,
        KeyCode::KeyS,
        KeyCode::KeyH,
        KeyCode::KeyA,
        KeyCode::KeyD,
    ] {
        frame_keys(&mut run, &pressed(k));
    }
    // keys:typed-end
    assert_eq!(run.menu.filter(), "shad");
    frame_keys(&mut run, &pressed(KeyCode::Enter));
    assert_ne!(rows(&run), lamp_before, "the menu flipped the shadow walk");
    assert!(
        run.modes.row(HudRow::Lighting).contains("shadow walk full"),
        "{}",
        run.modes.row(HudRow::Lighting)
    );
}

/// The keys that stay: closed, `F7` flips the lamp, `F9` the view, `F10` the upscaler, `X`
/// the cutaway, `1` drops a light, `6` fires one, `4` removes the newest, `` ` `` tries a
/// walker, `T` resumes a tour in free flight and `Enter` starts the readings. Open, the
/// function keys still act and the open page's row shows the new value; a letter or digit
/// shortcut is the filter's and acts on nothing.
#[test]
fn a_shortcut_still_acts_and_shows() {
    let mut run = bare_run(carved_rock());
    for (k, row, closed_word) in [
        (KeyCode::F7, HudRow::Lighting, "lamp off"),
        (KeyCode::F9, HudRow::Look, "view"),
        (KeyCode::F10, HudRow::Upscaler, "upscaler own"),
        (KeyCode::KeyX, HudRow::Controls, "cutaway on"),
    ] {
        let before = run.modes.row(row);
        frame_keys(&mut run, &pressed(k));
        let after = run.modes.row(row);
        assert_ne!(after, before, "{k:?} closed");
        assert!(after.contains(closed_word), "{k:?}: {after}");
    }
    let count = |run: &Run| run.modes.lights.count_words(true);
    let start = count(&run);
    frame_keys(&mut run, &pressed(KeyCode::Digit1));
    let dropped = count(&run);
    assert_ne!(dropped, start, "1 drops a light");
    frame_keys(&mut run, &pressed(KeyCode::Digit6));
    assert_ne!(count(&run), dropped, "6 fires a light");
    frame_keys(&mut run, &pressed(KeyCode::Digit4));
    assert_eq!(count(&run), dropped, "4 removes the newest");
    frame_keys(&mut run, &pressed(KeyCode::Backquote));
    let banner = run.banner().unwrap();
    assert!(banner.contains("walker"), "` places a walker: {banner}");
    frame_keys(&mut run, &pressed(KeyCode::Enter));
    assert!(run.samples.is_some(), "Enter starts the readings");
    run.samples = None;
    let mut player = crate::path::PathPlayer::new(&crate::path::CAVE, crate::path::PathMode::Tour);
    player.drive = crate::path::Drive::Free;
    run.player = Some(player);
    frame_keys(&mut run, &pressed(KeyCode::KeyT));
    assert!(
        matches!(run.player.unwrap().drive, crate::path::Drive::Path { .. }),
        "T resumes the tour"
    );
    run.player = None;

    frame_keys(&mut run, &pressed(KeyCode::Tab));
    frame_keys(&mut run, &pressed(KeyCode::F7));
    assert!(run.modes.lighting.lamp, "F7 flips the lamp back while open");
    // keys:typed-begin
    for k in [KeyCode::KeyL, KeyCode::KeyA] {
        frame_keys(&mut run, &pressed(k));
    }
    // keys:typed-end
    run.write_menu();
    assert!(
        run.hud
            .text
            .lines()
            .nth(1)
            .unwrap()
            .starts_with("> lighting  lamp on"),
        "{}",
        run.hud.text
    );
    frame_keys(&mut run, &pressed(KeyCode::F7));
    run.write_menu();
    assert!(
        run.hud
            .text
            .lines()
            .nth(1)
            .unwrap()
            .starts_with("> lighting  lamp off"),
        "the row follows F7 while open: {}",
        run.hud.text
    );
    let lights = count(&run);
    let cutaway = run.modes.row(HudRow::Controls);
    for k in [KeyCode::KeyX, KeyCode::Digit1] {
        frame_keys(&mut run, &pressed(k));
    }
    assert_eq!(
        run.modes.row(HudRow::Controls),
        cutaway,
        "X is the filter's while open"
    );
    assert_eq!(count(&run), lights, "1 is the filter's while open");
    assert_eq!(run.menu.filter(), "lax1");
    frame_keys(&mut run, &pressed(KeyCode::Enter));
    assert!(run.samples.is_none(), "Enter acts on the menu while open");
}

/// On a pad: `Start` opens the root; `DpadDown` and `South` open the second group; `East`
/// returns to the root; `Start` closes. Closed, `Select` shows or hides the stats overlay and
/// `DpadUp` steps the dig radius as it did.
#[test]
fn the_pad_walks_the_menu() {
    use crate::hud::menu::Group;
    use vxe_app::PadButton;
    let pad = |b: PadButton| {
        let mut i = Input::default();
        i.pad_down(b);
        i
    };
    let mut run = bare_run(carved_rock());
    frame_keys(&mut run, &pad(PadButton::Start));
    assert_eq!(run.menu.page(), Some(PageId::Root));
    frame_keys(&mut run, &pad(PadButton::DpadDown));
    frame_keys(&mut run, &pad(PadButton::South));
    assert_eq!(run.menu.page(), Some(PageId::Group(Group::ALL[1])));
    frame_keys(&mut run, &pad(PadButton::East));
    assert_eq!(run.menu.page(), Some(PageId::Root));
    frame_keys(&mut run, &pad(PadButton::Start));
    assert!(!run.menu.is_open());
    let visible = run.hud.visible;
    frame_keys(&mut run, &pad(PadButton::Select));
    assert_eq!(run.hud.visible, !visible);
    let radius = run.modes.dig_radius;
    frame_keys(&mut run, &pad(PadButton::DpadUp));
    assert_eq!(run.modes.dig_radius, radius + crate::edits::DIG_STEP);
}

/// `Enter` starts the readings; `F1` opens the Keys page and starts none.
#[test]
fn the_readings_moved_to_enter() {
    let mut run = bare_run(carved_rock());
    frame_keys(&mut run, &pressed(KeyCode::F1));
    assert_eq!(run.menu.page(), Some(PageId::Keys));
    assert!(run.samples.is_none());
    frame_keys(&mut run, &pressed(KeyCode::F1));
    frame_keys(&mut run, &pressed(KeyCode::Enter));
    assert!(run.samples.is_some());
}

/// The banner at a run's start holds the row `Tab menu, F1 keys` and loses it after
/// `hud::BANNER_MS`; under `--tester` it never shows.
#[test]
fn the_start_banner_names_the_menu_keys() {
    let mut run = bare_run(carved_rock());
    run.start = crate::now();
    let has = |run: &Run| {
        run.banner()
            .unwrap()
            .lines()
            .any(|l| l == "Tab menu, F1 keys")
    };
    assert!(has(&run), "{}", run.banner().unwrap());
    run.start =
        crate::now() - std::time::Duration::from_secs_f64(crate::hud::BANNER_MS / 1e3 + 0.001);
    assert!(!has(&run));
    let mut tester = bare_run(carved_rock());
    tester.args.tester = true;
    tester.start = crate::now();
    tester.player = Some(crate::path::PathPlayer::new(
        &crate::path::CAVE,
        crate::path::PathMode::Tour,
    ));
    assert!(!has(&tester), "{}", tester.banner().unwrap());
}

/// With sixty 16.7 ms frames recorded and the menu open, the overlay's row 0 ends at the
/// window's last whole column with `fps 59.9  frame 16.7 ms p99 16.7`, white, whether the
/// stats are shown or hidden, and the panel's title is on row 6 at 2530 x 1378; at 1280 x
/// 720 the readout shares row 0 with the title. With the menu closed and the stats off the
/// overlay is the banner, which has no readout.
#[test]
fn the_readout_shows_with_the_menu_open() {
    const READOUT: &str = "fps 59.9  frame 16.7 ms p99 16.7";
    for stats in [true, false] {
        let mut run = bare_run(carved_rock());
        run.hud = super::hud_feed::Hud::new(stats);
        for _ in 0..60 {
            run.hud.frames.record(0.0167);
        }
        frame_keys(&mut run, &pressed(KeyCode::Tab));
        assert_eq!(
            Overlay::of(false, run.menu.is_open(), run.hud.visible),
            Overlay::Menu
        );
        run.write_menu();
        run.place_menu([2530, 1378]);
        let row0 = run.hud.overlay.lines().next().unwrap();
        assert!(row0.ends_with(READOUT), "{row0:?}");
        assert_eq!(row0.len(), 105, "ends at the last whole column: {row0:?}");
        assert_eq!(row0.trim_start(), READOUT);
        assert_eq!(run.hud.overlay_colours[0], vxe_render::HudColour::White);
        assert_eq!(
            run.hud.overlay.lines().nth(6).map(str::trim_start),
            Some("menu")
        );
        run.place_menu([1280, 720]);
        let row0 = run.hud.overlay.lines().next().unwrap();
        assert!(row0.starts_with("menu "), "{row0:?}");
        assert!(row0.ends_with(READOUT), "{row0:?}");
        assert_eq!(row0.len(), 80);
    }
    let mut run = bare_run(carved_rock());
    run.hud = super::hud_feed::Hud::new(false);
    assert_eq!(Overlay::of(false, false, false), Overlay::Banner);
    run.write_banner();
    assert!(!run.hud.banner.contains("fps"), "{}", run.hud.banner);
}
