//! The run's tests of the menu's place in the frame: the key step's `Esc`, the capture of the
//! dig and the wheel, the tester's filter in the loop, and the overlay's choice.

use super::*;
use crate::hud::HudRow;
use crate::hud::menu::{Activated, PageId, RunView, all_keys};
use crate::path::{TesterInput, tester_action};
use crate::run::hud_feed::Overlay;
use crate::test_support::{bare_run, carved_rock};
use crate::toggles::Act;
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
/// rows; the profile's `PageDown`, the source's `Home`, the hour's `PageUp`, the day cycle's
/// `End` and the trees' `CapsLock`): text here, bound to nothing now.
// keys:typed-begin
const FORMER_KEYS: [KeyCode; 45] = [
    KeyCode::F3,
    KeyCode::PageDown,
    KeyCode::Home,
    KeyCode::PageUp,
    KeyCode::End,
    KeyCode::CapsLock,
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
/// the readings, walking, the castle shot, the walkers' switches, the lights' count, the world
/// (an edit), the profile and a source or trees switch asked for.
type State = (
    Vec<String>,
    bool,
    bool,
    bool,
    Option<&'static str>,
    [bool; 2],
    u32,
    (String, bool, bool),
);

fn state(run: &mut Run) -> State {
    let w = [run.modes.walkers.walking, run.modes.walkers.eye_lights_off];
    (
        rows(run),
        run.menu.is_open(),
        run.samples.is_some(),
        run.walk.is_some(),
        run.castle_shot.map(|c| c.name()),
        w,
        tick_ops(run) as u32,
        (run.modes.profile.clone(), run.switch, run.retree),
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
/// the cutaway, `F4` cycles the profile, `1` drops a light, `6` fires one, `4` removes the newest, `` ` `` tries a
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
    assert_eq!(run.modes.profile, "profile: floor");
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
    // `F4` is a function key: it cycles the profile open, and closed.
    frame_keys(&mut run, &pressed(KeyCode::F4));
    assert!(
        run.modes.profile.starts_with("profile: portable"),
        "F4 cycles the profile while open: {}",
        run.modes.profile
    );
    frame_keys(&mut run, &pressed(KeyCode::Tab));
    assert!(!run.menu.is_open());
    frame_keys(&mut run, &pressed(KeyCode::F4));
    assert!(
        run.modes.profile.starts_with("profile: performance"),
        "F4 cycles the profile closed: {}",
        run.modes.profile
    );
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

// The menu's numbers, unavailable entries and answers. The keys these tests press are the open
// menu's text and navigation, not binds.
// keys:typed-begin

/// One frame with the pad button `b` pressed.
fn pad(b: vxe_app::PadButton) -> Input {
    let mut i = Input::default();
    i.pad_down(b);
    i
}

/// The key that types `c` into the palette.
fn key_for(c: char) -> KeyCode {
    *all_keys()
        .iter()
        .find(|&&k| crate::hud::menu::typed(k) == Some(c))
        .unwrap_or_else(|| panic!("{c:?} types"))
}

/// `Tab` (when closed) and `words` typed: the palette at `words`.
fn search(run: &mut Run, words: &str) {
    if !run.menu.is_open() {
        frame_keys(run, &pressed(KeyCode::Tab));
    }
    for c in words.chars() {
        frame_keys(run, &pressed(key_for(c)));
    }
    assert_eq!(run.menu.filter(), words);
}

/// The palette at `e`'s label with the selection on `e`.
fn select(run: &mut Run, e: &crate::hud::menu::Entry) {
    use crate::hud::menu::Row;
    search(run, e.label);
    let v = run.run_view();
    let at = run
        .menu
        .rows(&run.modes, &v)
        .iter()
        .filter(|r| r.selectable())
        .position(|r| matches!(r, Row::Entry(x) if x.label == e.label && x.group == e.group))
        .unwrap_or_else(|| panic!("{} matches its label", e.label));
    for _ in 0..at {
        frame_keys(run, &pressed(KeyCode::ArrowDown));
    }
}

/// The open menu's text: its rows and their colours.
fn menu_text(run: &mut Run) -> (Vec<String>, Vec<vxe_render::HudColour>) {
    run.write_menu();
    let rows = run.hud.text.lines().map(str::to_owned).collect();
    (rows, run.hud.colours.clone())
}

/// The selected row's text and colour.
fn selected_row(run: &mut Run) -> (String, vxe_render::HudColour) {
    let (rows, colours) = menu_text(run);
    let i = rows
        .iter()
        .position(|r| r.starts_with("> "))
        .expect("a selected row");
    (rows[i].clone(), colours[i])
}

/// The menu's footer: its last row.
fn footer_row(run: &mut Run) -> String {
    menu_text(run).0.last().cloned().unwrap_or_default()
}

/// `dig radius` in metres as its row shows it.
fn dig_m(run: &Run) -> i64 {
    run.modes.dig_radius / vxe_world::FIXED_ONE
}

/// `Tab`, `dig`, `Enter` edits the dig radius: its row reads `< 0.5 m >` and its range, the
/// footer names the arrows; `Up` twice gives 0.7 m (7 voxels), `Down` 0.6 m; `Esc` leaves the
/// mode with 0.6 m kept, and on the `moving` page the next `Down` moves the selection. At
/// 3.0 m `Up` holds. Outside the mode `Right` on the row changes nothing and `Left` goes back
/// to the root; the wheel scrolls the page.
#[test]
fn enter_edits_a_number_and_the_arrows_step_it() {
    let mut run = bare_run(carved_rock());
    search(&mut run, "dig");
    frame_keys(&mut run, &pressed(KeyCode::Enter));
    assert!(run.menu.editing());
    let (row, colour) = selected_row(&mut run);
    assert_eq!(row, "> dig radius  < 0.5 m >  0.1..3.0 m");
    assert_eq!(colour, vxe_render::HudColour::Yellow);
    assert_eq!(footer_row(&mut run), "Up Down change  Enter Esc done");
    frame_keys(&mut run, &pressed(KeyCode::ArrowUp));
    frame_keys(&mut run, &pressed(KeyCode::ArrowUp));
    assert_eq!(dig_m(&run), 7);
    assert!(run.modes.row(HudRow::Controls).contains("dig radius 0.7 m"));
    frame_keys(&mut run, &pressed(KeyCode::ArrowDown));
    assert_eq!(dig_m(&run), 6);
    assert_eq!(
        selected_row(&mut run).0,
        "> dig radius  < 0.6 m >  0.1..3.0 m"
    );
    frame_keys(&mut run, &pressed(KeyCode::Escape));
    assert!(!run.menu.editing() && run.menu.is_open());
    assert_eq!(run.menu.filter(), "dig", "Esc leaves the mode first");
    assert_eq!(dig_m(&run), 6);
    assert!(!selected_row(&mut run).0.contains('<'));

    // On the `moving` page: the dig radius, then `walk`.
    frame_keys(&mut run, &pressed(KeyCode::Escape));
    frame_keys(&mut run, &pressed(KeyCode::Enter));
    assert_eq!(
        run.menu.page(),
        Some(PageId::Group(crate::hud::menu::Group::Moving))
    );
    assert_eq!(run.menu.selected(), 0);
    frame_keys(&mut run, &pressed(KeyCode::ArrowDown));
    assert_eq!(
        run.menu.selected(),
        1,
        "Down outside the mode moves the selection"
    );
    assert_eq!(dig_m(&run), 6);
    frame_keys(&mut run, &pressed(KeyCode::ArrowUp));
    frame_keys(&mut run, &pressed(KeyCode::Enter));
    for _ in 0..30 {
        frame_keys(&mut run, &pressed(KeyCode::ArrowUp));
    }
    assert_eq!(dig_m(&run), 30, "3.0 m");
    frame_keys(&mut run, &pressed(KeyCode::ArrowUp));
    assert_eq!(dig_m(&run), 30, "Up holds at the top");
    frame_keys(&mut run, &pressed(KeyCode::Enter));
    assert!(!run.menu.editing(), "Enter leaves the mode");

    let before = (rows(&run), run.menu.page(), run.menu.selected());
    frame_keys(&mut run, &pressed(KeyCode::ArrowRight));
    assert_eq!((rows(&run), run.menu.page(), run.menu.selected()), before);
    frame_keys(&mut run, &pressed(KeyCode::ArrowLeft));
    assert_eq!(run.menu.page(), Some(PageId::Root));
    let mut wheel = Input::default();
    wheel.add_scroll(-2.0);
    frame_keys(&mut run, &wheel);
    assert_eq!(run.menu.selected(), 2, "the wheel scrolls the page");
    assert_eq!(dig_m(&run), 30);
}

/// On `missing light` `Enter` gives the next of `MissingLight::ALL` and opens no edit mode
/// (the footer is the palette's, the row has no `<` `>`); the arrows leave it so; on `lamp`
/// `Enter` flips it.
#[test]
fn a_cycle_keeps_enter_as_next() {
    let mut run = bare_run(carved_rock());
    let start = run.modes.queue.missing_light;
    search(&mut run, "missing");
    frame_keys(&mut run, &pressed(KeyCode::Enter));
    let next = crate::toggles::next_missing_light(start);
    assert_eq!(run.modes.queue.missing_light, next);
    assert!(!run.menu.editing());
    assert!(footer_row(&mut run).starts_with("Enter acts  Backspace erases"));
    for k in [
        KeyCode::ArrowUp,
        KeyCode::ArrowDown,
        KeyCode::ArrowLeft,
        KeyCode::ArrowRight,
    ] {
        frame_keys(&mut run, &pressed(k));
        assert!(!run.menu.editing(), "{k:?}");
        assert_eq!(run.modes.queue.missing_light, next, "{k:?}");
    }
    let (row, _) = selected_row(&mut run);
    assert!(!row[2..].contains(['<', '>']), "{row}");
    frame_keys(&mut run, &pressed(KeyCode::Escape));
    search(&mut run, "lamp");
    let lamp = run.modes.lighting.lamp;
    frame_keys(&mut run, &pressed(KeyCode::Enter));
    assert_eq!(run.modes.lighting.lamp, !lamp);
    assert!(!run.menu.editing());
}

/// The entries a bare run cannot act on, each with its reason: grey with the reason on its
/// own row, in the palette and on its group's page (selected or not), the footer naming the
/// page's keys only, `Enter` changing nothing and saying the reason in the notice; the
/// root's `audio` row grey with `needs --audio`; `--keys` printing `unavailable: needs --pan`
/// on the pan row. With `--pan 10` the pan acts (`pan off`). A streamed run without the
/// coarse grid refuses `coarse print`, `crumble` and `walk` with `needs --level`; the bare
/// (bounded) run walks.
#[test]
fn an_unavailable_entry_says_why() {
    use crate::hud::menu::{Group, entries};
    let mut want: Vec<(&str, &str)> = vec![
        ("resume the tour", "needs --path"),
        ("next castle shot", "needs --castle"),
        ("pan", "needs --pan"),
        ("clip before", "needs a placed entity"),
        ("clip after", "needs a placed entity"),
        ("clip pause", "needs a placed entity"),
    ];
    let audio: Vec<&str> = entries()
        .filter(|e| e.group == Group::Audio)
        .map(|e| e.label)
        .collect();
    assert_eq!(audio.len(), 10, "{audio:?}");
    want.extend(audio.iter().map(|&l| (l, "needs --audio")));
    for (label, why) in want {
        let e = entries().find(|e| e.label == label).unwrap();
        let mut run = bare_run(carved_rock());
        assert_eq!(
            e.available(&run.modes, &run.run_view()),
            Err(why),
            "{label}"
        );
        run.menu.open_at(crate::hud::menu::PageId::Group(e.group));
        let words = format!("{}  {why}", e.words(&run.modes, &run.run_view()));
        let (rows, colours) = menu_text(&mut run);
        let i = rows
            .iter()
            .position(|r| r.get(2..).is_some_and(|r| r.starts_with(&words)))
            .unwrap_or_else(|| panic!("{label} on its page: {rows:?}"));
        assert_eq!(
            colours[i],
            vxe_render::HudColour::Grey,
            "{label}: {}",
            rows[i]
        );
        run.menu.close();
        select(&mut run, &e);
        let (row, colour) = selected_row(&mut run);
        assert_eq!(colour, vxe_render::HudColour::Grey, "{label}: {row}");
        assert!(
            row.starts_with(&format!("> {}  ", e.group.name()))
                && row.contains(&format!("  {why}")),
            "{label}: the palette row {row}"
        );
        assert!(!footer_row(&mut run).contains(why), "{label}");
        let before = state(&mut run);
        frame_keys(&mut run, &pressed(KeyCode::Enter));
        assert_eq!(state(&mut run), before, "{label} acted");
        assert!(!run.menu.editing(), "{label}");
        assert_eq!(run.notice.as_ref().map(|(l, _)| l.as_str()), Some(why));
        let (rows, colours) = menu_text(&mut run);
        let n = rows.len();
        assert_eq!(
            (rows[n - 2].as_str(), colours[n - 2]),
            (why, vxe_render::HudColour::Yellow),
            "{label}: the notice row"
        );
    }

    let mut run = bare_run(carved_rock());
    frame_keys(&mut run, &pressed(KeyCode::Tab));
    let (rows, colours) = menu_text(&mut run);
    let i = rows
        .iter()
        .position(|r| r.trim_start().starts_with("audio"))
        .unwrap();
    assert!(rows[i].ends_with("needs --audio"), "{}", rows[i]);
    assert_eq!(colours[i], vxe_render::HudColour::Grey);
    let tree = crate::hud::menu::keys_text(
        &crate::cli::parse_args_from(vec!["--keys".to_owned()]).unwrap(),
    );
    let pan_row = tree
        .lines()
        .find(|l| l.trim_start().starts_with("pan "))
        .unwrap();
    assert!(pan_row.ends_with("unavailable: needs --pan"), "{pan_row}");
    let tree = crate::hud::menu::keys_text(
        &crate::cli::parse_args_from(["--keys", "--pan", "10"].map(str::to_owned).to_vec())
            .unwrap(),
    );
    let pan_row = tree
        .lines()
        .find(|l| l.trim_start().starts_with("pan "))
        .unwrap();
    assert!(!pan_row.contains("unavailable"), "{pan_row}");

    let mut run = bare_run(carved_rock());
    run.modes = run.modes.clone().with_pan(10.0, 0.0);
    let pan = entries().find(|e| e.label == "pan").unwrap();
    assert_eq!(pan.available(&run.modes, &run.run_view()), Ok(()));
    select(&mut run, &pan);
    frame_keys(&mut run, &pressed(KeyCode::Enter));
    assert!(run.modes.row(HudRow::Upscaler).contains("pan off"));

    let run = bare_run(carved_rock());
    let bounded = run.run_view();
    let streamed = RunView {
        streaming: true,
        coarse: false,
        ..bounded
    };
    for label in ["coarse print", "crumble", "walk"] {
        let e = entries().find(|e| e.label == label).unwrap();
        assert_eq!(
            e.available(&run.modes, &streamed),
            Err("needs --level"),
            "{label}"
        );
    }
    let walk = entries().find(|e| e.label == "walk").unwrap();
    assert_eq!(walk.available(&run.modes, &bounded), Ok(()));
}

/// No entry is a silent no-op: on a bare run each available entry, reached by `Tab`, its
/// words and `Enter` (a number then `Up`, or `Down` at its top), changes `state` (the HUD's
/// rows, the lighting, the readings, walking, the walkers, the world) or says a line in the
/// notice in the same key step.
#[test]
fn every_live_entry_answers() {
    use crate::hud::menu::entries;
    let mut live = 0;
    for e in entries() {
        let mut run = bare_run(carved_rock());
        if e.available(&run.modes, &run.run_view()).is_err() {
            continue;
        }
        live += 1;
        select(&mut run, &e);
        let before = state(&mut run);
        run.notice = None;
        frame_keys(&mut run, &pressed(KeyCode::Enter));
        if e.is_number() {
            assert!(run.menu.editing(), "{}", e.label);
            frame_keys(&mut run, &pressed(KeyCode::ArrowUp));
            if state(&mut run) == before {
                frame_keys(&mut run, &pressed(KeyCode::ArrowDown));
            }
        }
        let said = run.notice.as_ref().is_some_and(|(l, _)| !l.is_empty());
        assert!(
            said || state(&mut run) != before,
            "{} is a silent no-op",
            e.label
        );
    }
    assert!(live > 30, "{live} live entries");
}

/// `frame readings` says `readings: sampling 30 frames` at once and, after 30 frames with no
/// GPU times, `frame: cpu 1.000 ms, gpu n/a (median of 30)`; `stamp` looking out of the world
/// says the logic's `stamped: no surface within 10 m` after one server tick; `coarse print`
/// with the coarse grid built says the coarse line.
#[test]
fn an_action_answers_in_the_window() {
    let mut run = bare_run(carved_rock());
    run.menu_act(Activated::Action(Act::Readings));
    let notice = |run: &Run| run.notice.as_ref().map(|(l, _)| l.clone());
    assert_eq!(
        notice(&run).as_deref(),
        Some("readings: sampling 30 frames")
    );
    for i in 0..30 {
        let done = run.file_samples(1.0, None, None, None);
        assert_eq!(done.is_some(), i == 29, "frame {i}");
    }
    assert_eq!(
        notice(&run).as_deref(),
        Some("frame: cpu 1.000 ms, gpu n/a (median of 30)")
    );
    assert!(run.samples.is_none());

    run.fly.camera.position = vxe_render::glam::Vec3::new(5000.0, 5000.0, 5000.0);
    run.menu_act(Activated::Action(Act::Stamp));
    assert_eq!(notice(&run).as_deref(), Some("stamp: sent to the logic"));
    run.tick_server();
    assert_eq!(
        notice(&run).as_deref(),
        Some("stamped: no surface within 10 m")
    );

    run.coarse = Some(crate::edits::Coarse::build(&run.world));
    run.menu_act(Activated::Action(Act::CoarsePrint));
    assert_eq!(notice(&run), Some(run.coarse_line()));
}

/// On a pad: `Start`, `South` on `moving`, `South` on the dig radius edits it; `DpadUp` steps
/// to 0.6 m; `East` leaves the mode; `DpadRight` on the row outside it changes nothing.
#[test]
fn the_pad_edits_a_number() {
    use vxe_app::PadButton;
    let mut run = bare_run(carved_rock());
    frame_keys(&mut run, &pad(PadButton::Start));
    frame_keys(&mut run, &pad(PadButton::South));
    frame_keys(&mut run, &pad(PadButton::South));
    assert!(run.menu.editing());
    frame_keys(&mut run, &pad(PadButton::DpadUp));
    assert_eq!(dig_m(&run), 6);
    frame_keys(&mut run, &pad(PadButton::East));
    assert!(!run.menu.editing() && run.menu.is_open());
    let before = (rows(&run), run.menu.page(), run.menu.selected());
    frame_keys(&mut run, &pad(PadButton::DpadRight));
    assert_eq!((rows(&run), run.menu.page(), run.menu.selected()), before);
}
// keys:typed-end
