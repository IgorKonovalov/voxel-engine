//! The fly-through's menu (`flythrough::hud::menu`): its entries over the key tables, the
//! source scan that holds the tables to the keys the code reads, and the tree `--keys`
//! prints. CPU only, no device.

use flythrough::cli::parse_args_from;
use flythrough::hud::menu::{
    Activated, MENU_COLS, MENU_ROWS, Menu, PageId, Panel, RouteInput, RunView, matches,
    panel_scale, place, step, taken,
};
use flythrough::hud::menu::{Group, entries, keys_text, unlisted_keys};
use flythrough::hud::{HudRow, Modes};
use flythrough::toggles::{ACTIONS, TOGGLES};
use flythrough::toggles::{Press, Toggle, press};
use svet_app::{Axis, Input, KeyCode, MouseButton, PadButton};
use svet_render::{HudBox, HudColour, Lighting, LightingResolution};

fn argv(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| (*s).to_owned()).collect()
}

/// The entries' `(label, group)` pairs cover every `TOGGLES` row and every `ACTIONS` row once
/// each, and no entry has an empty label or a group the root page does not list.
#[test]
fn every_toggle_and_action_is_an_entry() {
    let pairs: Vec<(&str, Group)> = entries().map(|e| (e.label, e.group)).collect();
    let mut want: Vec<(&str, Group)> = TOGGLES.iter().map(|t| (t.name, t.group)).collect();
    want.extend(ACTIONS.iter().map(|a| (a.label, a.group)));
    assert_eq!(pairs.len(), want.len(), "{pairs:?}");
    for w in &want {
        let n = pairs.iter().filter(|p| *p == w).count();
        assert_eq!(n, 1, "{w:?} is an entry {n} times");
    }
    for (label, group) in &pairs {
        assert!(!label.is_empty(), "an entry of {group:?} has no label");
        assert!(Group::ALL.contains(group), "{label}: {group:?}");
    }
}

/// Every `KeyCode::<Name>` token in the `.rs` files under `apps/flythrough/src/` names a key
/// `TOGGLES`, `ACTIONS` or `FIXED_KEYS` binds; and the scan bites: fed a read of `ArrowUp`,
/// which nothing binds, it reports `ArrowUp`.
#[test]
fn the_source_reads_no_key_the_tables_miss() {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).expect("the source directory reads") {
            let p = e.expect("an entry").path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    assert!(files.len() > 20, "{files:?}");
    for f in &files {
        let text = std::fs::read_to_string(f).expect("the file reads");
        let missed = unlisted_keys(&text);
        assert!(missed.is_empty(), "{}: {missed:?}", f.display());
    }
    assert_eq!(
        unlisted_keys("if input.pressed(KeyCode::ArrowUp) {}"),
        ["ArrowUp"]
    );
    assert_eq!(
        unlisted_keys("// keys:typed-begin\nKeyCode::ArrowUp,\n// keys:typed-end\n"),
        Vec::<String>::new()
    );
}

/// The `--keys` tree's row for `label` under the group header `group`.
fn tree_row<'a>(tree: &'a str, group: &str, label: &str) -> &'a str {
    let mut in_group = false;
    for line in tree.lines() {
        if !line.starts_with(' ') {
            in_group = line == group;
        } else if in_group && line[2..].split("  ").next() == Some(label) {
            return line;
        }
    }
    panic!("no {label} under {group}:\n{tree}");
}

/// `--keys` at the start modes prints `lamp on` under `lighting` with `F7` in its key column;
/// with `--lamp off` it prints `lamp off`. Every group's header is a line of its own; the
/// `structures` group lists `clear box`, `structure scale` and `choose a structure`.
#[test]
fn the_tree_prints_live_values() {
    let start = parse_args_from(argv(&["--keys"])).expect("--keys parses");
    assert!(start.keys);
    let tree = keys_text(&start);
    let row = tree_row(&tree, "lighting", "lamp");
    let cols: Vec<&str> = row.split("  ").filter(|c| !c.trim().is_empty()).collect();
    assert_eq!(
        cols.iter().map(|c| c.trim()).collect::<Vec<_>>(),
        ["lamp", "lamp on", "F7"],
        "{row}"
    );
    for g in Group::ALL {
        assert!(tree.lines().any(|l| l == g.name()), "{g:?}:\n{tree}");
    }
    // Plan 0083 Phase 6: the `structures` group's three entries, menu-only.
    for (label, value) in [
        ("clear box", "clear box on"),
        ("structure scale", "structure scale 1"),
        ("choose a structure", "-"),
    ] {
        let row = tree_row(&tree, "structures", label);
        assert!(row.contains(&format!(" {value} ")), "{row}");
        assert!(row.trim_end().ends_with(" menu"), "{row}");
    }
    let off = keys_text(&parse_args_from(argv(&["--keys", "--lamp", "off"])).unwrap());
    let row = tree_row(&off, "lighting", "lamp");
    assert!(row.contains(" lamp off "), "{row}");
    assert!(row.trim_end().ends_with(" F7"), "{row}");
}

/// `flythrough --keys` exits before a device: the parser sets `keys` (which `main` reads
/// beside `--help`, before the window or a device), and a run without it does not.
#[test]
fn keys_is_an_early_exit_beside_help() {
    let a = parse_args_from(argv(&["--keys"])).unwrap();
    assert!(a.keys && !a.help);
    assert!(!parse_args_from(argv(&[])).unwrap().keys);
    let main = include_str!("../src/main.rs");
    let keys_at = main.find("if args.keys").expect("main reads --keys");
    let device_at = main.find("setup::flythrough").expect("main starts the app");
    assert!(keys_at < device_at, "--keys exits before the app starts");
}

/// The modes over the default lighting with the lamp as given and its bounce on, as a run
/// starts it.
fn modes(lamp: bool) -> Modes {
    Modes::new(
        Lighting {
            lamp,
            lamp_bounce: true,
            ..Lighting::default()
        },
        LightingResolution::Half,
        0.0,
    )
}

fn toggle(name: &str) -> &'static Toggle {
    TOGGLES
        .iter()
        .find(|t| t.name == name)
        .unwrap_or_else(|| panic!("{name} is a toggle"))
}

/// One frame with `key` pressed through the menu's step at `m`; the activations it gives.
fn key(menu: &mut Menu, m: &Modes, k: KeyCode) -> Vec<Activated> {
    let mut input = Input::default();
    input.key_down(k);
    step(menu, &input, m, &RunView::default()).acts
}

/// The menu's text and colours at `m`.
fn drawn(menu: &Menu, m: &Modes) -> (String, Vec<HudColour>) {
    let (mut text, mut colours) = (String::new(), Vec::new());
    menu.write(m, &RunView::default(), &mut text, &mut colours);
    assert_eq!(
        text.lines().count(),
        colours.len(),
        "a colour per row:\n{text}"
    );
    assert!(text.lines().count() <= MENU_ROWS, "{text}");
    for l in text.lines() {
        assert!(l.len() <= MENU_COLS, "{l}");
    }
    (text, colours)
}

/// An entry's row on a group page: its words from column 2, its binds from column 60.
fn entry_row(words: &str, keys: &str) -> String {
    format!("  {words:<58}{keys}").trim_end().to_owned()
}

/// With the lamp off and the filter at 2 iterations, `Tab` draws the root page: the title, a
/// row per group with the first selected (yellow) and `audio` grey with `needs --audio` (the
/// run has no audio), a blank row and the footer. `Down` five
/// times and `Enter` open `lighting`: its twelve toggles' words and binds, `lamp off` and
/// `filter 2` among them, `F7` in the lamp's key column and the menu-only rows with none.
#[test]
fn the_root_and_a_group_draw_their_rows() {
    let mut m = modes(false);
    press(toggle("filter iterations"), &mut m, 0.0);
    let mut menu = Menu::default();
    key(&mut menu, &m, KeyCode::Tab);
    assert_eq!(menu.page(), Some(PageId::Root));
    let (text, colours) = drawn(&menu, &m);
    // With no audio in the run the audio group is grey with its reason in the key column.
    let audio = entry_row("audio", "needs --audio");
    let want = format!(
        "menu\n> moving\n  camera and paths\n  world and sky\n  look\n  upscaler\n  lighting\n  lights\n  \
         light queue\n  entities\n  structures\n{audio}\n  readings\n  debug\n\nEnter opens  \
         Esc closes  Tab closes  type to search"
    );
    assert_eq!(text, want);
    let mut want_colours = vec![HudColour::White; 16];
    want_colours[1] = HudColour::Yellow;
    want_colours[11] = HudColour::Grey;
    assert_eq!(colours, want_colours);
    for _ in 0..5 {
        key(&mut menu, &m, KeyCode::ArrowDown);
    }
    assert!(key(&mut menu, &m, KeyCode::Enter).is_empty());
    assert_eq!(menu.page(), Some(PageId::Group(Group::Lighting)));
    let (text, colours) = drawn(&menu, &m);
    let rows = [
        ("shadow walk coarse", ""),
        ("filter iterations: filter 2", ""),
        ("history edits", ""),
        ("lighting resolution: lighting half", ""),
        ("lamp off", "F7"),
        ("lamp bounce on", ""),
        ("ore pulse off", ""),
        ("exposure manual", ""),
        ("store read face", ""),
        ("key far column", ""),
        ("key far mix step", ""),
        ("light weights normal", ""),
        ("young filter: young luma", ""),
        ("entity light world", ""),
    ];
    let mut want = vec!["menu  >  lighting".to_owned()];
    want.extend(rows.iter().map(|(w, k)| entry_row(w, k)));
    want[1].replace_range(0..1, ">");
    want.push(String::new());
    want.push("Enter acts  Esc back  Tab closes  type to search".to_owned());
    assert_eq!(text, want.join("\n"));
    let mut want_colours = vec![HudColour::White; 17];
    want_colours[1] = HudColour::Yellow;
    assert_eq!(colours, want_colours);
}

/// Typing `l`, `a`, `m` on the root page gives exactly the entries whose label, value or group
/// contains `lam`, in the tree's order, each after its group; `Backspace` three times gives the
/// root page back; `Esc` with a filter clears it and leaves the menu open, a second closes it.
#[test]
fn typing_filters_the_whole_tree() {
    let m = modes(true);
    let v = RunView::default();
    let mut menu = Menu::default();
    key(&mut menu, &m, KeyCode::Tab);
    let (root, _) = drawn(&menu, &m);
    for k in [KeyCode::KeyL, KeyCode::KeyA, KeyCode::KeyM] {
        key(&mut menu, &m, k);
    }
    assert_eq!(menu.filter(), "lam");
    let want: Vec<String> = entries()
        .filter(|e| {
            let value = e.value(&m, &v).unwrap_or_default();
            e.label.contains("lam") || value.contains("lam") || e.group.name().contains("lam")
        })
        .map(|e| format!("{}  {}", e.group.name(), e.words(&m, &v)))
        .collect();
    assert!(want.len() >= 2, "{want:?}");
    let (text, _) = drawn(&menu, &m);
    let rows: Vec<&str> = text.lines().skip(1).take(want.len()).collect();
    assert_eq!(text.lines().next(), Some("menu  search: lam"));
    for (row, w) in rows.iter().zip(&want) {
        assert!(row[2..].starts_with(w.as_str()), "{row} is {w}");
    }
    assert_eq!(text.lines().nth(want.len() + 1), Some(""), "{text}");
    let lamp = rows
        .iter()
        .find(|r| r[2..].starts_with("lighting  lamp on"))
        .expect("the lamp matches");
    assert_eq!(
        lamp[2..].split_whitespace().collect::<Vec<_>>(),
        ["lighting", "lamp", "on", "F7"]
    );
    assert!(
        rows.iter()
            .any(|r| r[2..].starts_with("lighting  lamp bounce on")),
        "{text}"
    );
    assert!(!text.contains("look face"), "{text}");
    assert!(!matches(
        &entries().find(|e| e.label == "look").unwrap(),
        "lam",
        &m,
        &v
    ));
    for _ in 0..3 {
        key(&mut menu, &m, KeyCode::Backspace);
    }
    assert_eq!(drawn(&menu, &m).0, root);
    key(&mut menu, &m, KeyCode::KeyL);
    key(&mut menu, &m, KeyCode::Escape);
    assert!(menu.is_open() && menu.filter().is_empty());
    assert_eq!(drawn(&menu, &m).0, root);
    key(&mut menu, &m, KeyCode::Escape);
    assert!(!menu.is_open());
}

/// `Tab`, `l`, `a`, `Enter` from the start modes flips the lamp: the lamp is off, the entry's
/// row reads `lamp off`, and the lighting HUD row and the printed line are what an `F7` press
/// gives.
#[test]
fn enter_acts_and_the_row_follows() {
    let mut m = modes(true);
    let mut menu = Menu::default();
    for k in [KeyCode::Tab, KeyCode::KeyL, KeyCode::KeyA] {
        assert!(key(&mut menu, &m, k).is_empty());
    }
    let acts = key(&mut menu, &m, KeyCode::Enter);
    let [Activated::Toggle(t)] = acts[..] else {
        panic!("{acts:?}")
    };
    let line = press(t, &mut m, 1.0);
    assert!(!m.lighting.lamp);
    let (text, _) = drawn(&menu, &m);
    let row = text.lines().nth(1).expect("the first match");
    assert!(row.starts_with("> lighting  lamp off"), "{text}");
    let mut by_key = modes(true);
    let f7 = TOGGLES
        .iter()
        .find(|t| t.press == Press::Key(KeyCode::F7, None))
        .expect("F7 is a toggle");
    assert_eq!(press(f7, &mut by_key, 1.0), line);
    assert_eq!(by_key.row(HudRow::Lighting), m.row(HudRow::Lighting));
}

/// The capture rule over every key the tables and the fixed set bind, the menu's keys, the
/// wheel, both clicks and every pad button: closed, everything passes; open, the arrows,
/// `Enter`, `Backspace`, `Esc`, the typed and punctuation keys, the wheel, both clicks, the
/// D-pad, `South`, `East` and `Start` are the menu's and nothing else is; the movement keys
/// pass as well (their press types, their hold moves); `F7` and the sticks pass. Through the
/// step, an open menu passes on no click and no wheel (no dig, no dig radius), and passes `W`
/// held, the mouse's motion, `F7` and the move stick.
#[test]
fn the_route_is_one_function() {
    use flythrough::hud::menu::{MENU_KEYS, all_keys, key_name, typed};
    let menu_pad = [
        PadButton::DpadUp,
        PadButton::DpadDown,
        PadButton::DpadLeft,
        PadButton::DpadRight,
        PadButton::South,
        PadButton::East,
        PadButton::Start,
    ];
    let moves = [
        KeyCode::KeyW,
        KeyCode::KeyA,
        KeyCode::KeyS,
        KeyCode::KeyD,
        KeyCode::Space,
        KeyCode::KeyC,
        KeyCode::ShiftLeft,
    ];
    let mut inputs: Vec<RouteInput> = all_keys().into_iter().map(RouteInput::Key).collect();
    inputs.extend(PadButton::ALL.map(RouteInput::Pad));
    inputs.extend([
        RouteInput::Wheel,
        RouteInput::Click(MouseButton::Left),
        RouteInput::Click(MouseButton::Right),
        RouteInput::MouseLook,
        RouteInput::Stick,
    ]);
    for &i in &inputs {
        let closed = taken(i, false);
        assert!(closed.pass && !closed.menu, "{i:?} closed: {closed:?}");
        let open = taken(i, true);
        let menus = match i {
            RouteInput::Key(k) => {
                let name = key_name(k);
                MENU_KEYS.contains(&k)
                    || typed(k).is_some()
                    || (name.len() == 1 && name.chars().all(|c| c.is_ascii_punctuation()))
            }
            RouteInput::Pad(b) => menu_pad.contains(&b),
            RouteInput::Wheel | RouteInput::Click(_) => true,
            _ => false,
        };
        assert_eq!(open.menu, menus, "{i:?} open: {open:?}");
        let passes = match i {
            RouteInput::Key(k) => moves.contains(&k) || !menus,
            _ => !menus,
        };
        assert_eq!(open.pass, passes, "{i:?} open: {open:?}");
    }
    for k in [KeyCode::KeyW, KeyCode::F7, KeyCode::F1, KeyCode::Tab] {
        assert!(taken(RouteInput::Key(k), true).pass, "{k:?}");
    }
    // Placement mode is the run's with the menu closed: its wheel, digits, `R`, `Esc` and
    // both clicks pass to the run.
    let mut placing: Vec<RouteInput> = flythrough::toggles::BELT_KEYS
        .into_iter()
        .chain([KeyCode::KeyR, KeyCode::Escape])
        .map(RouteInput::Key)
        .collect();
    placing.extend([
        RouteInput::Wheel,
        RouteInput::Click(MouseButton::Left),
        RouteInput::Click(MouseButton::Right),
    ]);
    for i in placing {
        assert_eq!(
            taken(i, false),
            flythrough::hud::menu::Taken {
                menu: false,
                pass: true
            },
            "{i:?} in placement mode"
        );
    }
    assert_eq!(
        taken(RouteInput::Key(KeyCode::KeyL), true),
        flythrough::hud::menu::Taken {
            menu: true,
            pass: false
        }
    );
    let m = modes(true);
    let mut menu = Menu::default();
    menu.open_at(PageId::Root);
    let mut input = Input::default();
    input.key_down(KeyCode::KeyW);
    input.key_down(KeyCode::F7);
    input.key_down(KeyCode::KeyL);
    input.button_down(MouseButton::Left);
    input.add_scroll(1.0);
    input.add_mouse_delta(3.0, -2.0);
    input.set_axis(Axis::MoveY, 0.5);
    input.set_axis(Axis::TriggerR, 1.0);
    let routed = step(&mut menu, &input, &m, &RunView::default());
    let pass = routed.pass.input();
    assert!(pass.held(KeyCode::KeyW) && pass.pressed(KeyCode::F7));
    assert!(!pass.pressed(KeyCode::KeyL));
    assert!(!pass.clicked(MouseButton::Left), "no dig while open");
    assert_eq!(pass.scroll(), 0.0, "no dig radius while open");
    assert_eq!(pass.axis(Axis::TriggerR), 0.0, "no trigger dig while open");
    assert_eq!(pass.mouse_delta(), [3.0, -2.0]);
    assert_eq!(pass.axis(Axis::MoveY), 0.5);
    // `W`'s press types as well as its hold moving; a frame's letters come in no order.
    let mut typed: Vec<char> = menu.filter().chars().collect();
    typed.sort_unstable();
    assert_eq!(typed, ['l', 'w']);
    let mut closed = Menu::default();
    let routed = step(&mut closed, &input, &m, &RunView::default());
    let pass = routed.pass.input();
    assert!(pass.clicked(MouseButton::Left) && pass.scroll() == 1.0 && pass.pressed(KeyCode::KeyL));
    assert!(!closed.is_open());
}

/// `F1` opens a flat page of every entry of every group with its binds under the fixed keys'
/// block; `F1` again closes it; `Tab` from the Keys page shows the root.
#[test]
fn f1_opens_the_keys_page() {
    use flythrough::hud::menu::{FIXED_ROWS, Row};
    let m = modes(true);
    let v = RunView::default();
    let mut menu = Menu::default();
    key(&mut menu, &m, KeyCode::F1);
    assert_eq!(menu.page(), Some(PageId::Keys));
    let rows = menu.rows(&m, &v);
    let fixed: Vec<(&str, &str)> = rows
        .iter()
        .take_while(|r| matches!(r, Row::Fixed(..)))
        .map(|r| match r {
            Row::Fixed(k, d) => (*k, *d),
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(fixed, FIXED_ROWS);
    let listed: Vec<(&str, Group)> = rows
        .iter()
        .filter_map(|r| match r {
            Row::Entry(e) => Some((e.label, e.group)),
            _ => None,
        })
        .collect();
    let all: Vec<(&str, Group)> = entries().map(|e| (e.label, e.group)).collect();
    assert_eq!(listed, all);
    let (text, _) = drawn(&menu, &m);
    assert!(text.starts_with("menu  >  keys"), "{text}");
    assert!(text.contains("W A S D"), "{text}");
    assert!(
        text.lines()
            .any(|l| l.split_whitespace().collect::<Vec<_>>() == ["F7", "lamp", "on"]),
        "{text}"
    );
    key(&mut menu, &m, KeyCode::F1);
    assert!(!menu.is_open());
    key(&mut menu, &m, KeyCode::F1);
    key(&mut menu, &m, KeyCode::Tab);
    assert_eq!(menu.page(), Some(PageId::Root));
}

/// The shortcut rows of ADR-0061's key table as the owner answered it (Plan 0080's "The
/// keys"), the render profile's `F4` (ADR-0055) and the belt's (Plan 0077's placement mode;
/// the dig radius is the menu's only; the digits are the belt's slots while the belt is shown,
/// a fixed row, and enter no mode): each entry that keeps a bind and its binds as the menu
/// shows them.
const SHORTCUTS: [(&str, &str); 20] = [
    ("profile", "F4"),
    ("walk", "G / pad North"),
    ("resume the tour", "T"),
    ("upscaler", "F10"),
    ("lamp", "F7"),
    ("drop a light", "1"),
    ("remove the newest", "4"),
    ("fire a light", "6"),
    ("choose an entity", "`"),
    ("belt next", "pad D-pad right"),
    ("belt before", "pad D-pad left"),
    ("turn the ghost", "R"),
    ("remove aimed entity", "Delete"),
    ("clip before", "Left"),
    ("clip after", "Right"),
    ("clip pause", "Down"),
    ("knock", "pad East"),
    ("frame readings", "Enter"),
    ("view", "F9"),
    ("snap", "F / pad D-pad up"),
];

/// `FIXED_KEYS` is the table's Fixed rows (movement, look's keys, `Tab`, `F1`, `F2`, `Esc`,
/// placement mode's `E`) and
/// no toggle or action binds one of them.
#[test]
fn the_fixed_set_is_the_table() {
    use flythrough::hud::menu::{FIXED_KEYS, bound_keys};
    assert_eq!(
        FIXED_KEYS,
        [
            KeyCode::KeyW,
            KeyCode::KeyA,
            KeyCode::KeyS,
            KeyCode::KeyD,
            KeyCode::Space,
            KeyCode::KeyC,
            KeyCode::ShiftLeft,
            KeyCode::Tab,
            KeyCode::F1,
            KeyCode::F2,
            KeyCode::Escape,
            KeyCode::KeyE,
        ]
    );
    let bound = bound_keys();
    for k in FIXED_KEYS {
        assert_eq!(bound.iter().filter(|b| *b == k).count(), 1, "{k:?}");
    }
}

/// The entries that keep a bind are exactly the table's shortcut rows (with `X / pad West` on
/// the cutaway), each with its binds, and `--keys` prints each in its key column; every other
/// entry has none and prints `menu`.
#[test]
fn the_shortcuts_are_the_table() {
    let mut want: Vec<(&str, &str)> = SHORTCUTS.to_vec();
    want.push(("cutaway", "X / pad West"));
    let mut have: Vec<(&str, String)> = entries()
        .map(|e| (e.label, e.keys()))
        .filter(|(_, k)| !k.is_empty())
        .collect();
    have.sort();
    want.sort();
    let have: Vec<(&str, &str)> = have.iter().map(|(l, k)| (*l, k.as_str())).collect();
    assert_eq!(have, want);
    let tree = keys_text(&parse_args_from(argv(&["--keys"])).unwrap());
    for e in entries() {
        let row = tree_row(&tree, e.group.name(), e.label);
        let keys = e.keys();
        let keys = if keys.is_empty() {
            "menu".to_owned()
        } else {
            keys
        };
        let row = row.split("  unavailable: ").next().unwrap_or(row);
        assert!(row.trim_end().ends_with(&format!(" {keys}")), "{row}");
    }
    // Plan 0077 Phase 9 (1): the fixed rows say the digits are the belt's slots while the
    // belt is shown; no entry's keys are a digit other than the lights' `1`, `4`, `6`.
    assert!(
        tree.lines()
            .any(|l| l.contains("1-9") && l.contains("the belt's slots while the belt is shown")),
        "{tree}"
    );
    assert!(
        tree.lines()
            .all(|l| l.trim_start().split("  ").next() != Some("placement mode")),
        "{tree}"
    );
    for (label, keys) in SHORTCUTS {
        let digit = keys
            .split(" / ")
            .any(|k| k.len() == 1 && k.chars().all(|c| c.is_ascii_digit()));
        assert!(!digit || ["1", "4", "6"].contains(&keys), "{label}: {keys}");
    }
}

/// A view of a run that has everything an entry can need: audio, a path, the castle, a
/// model, a bounded world with its coarse grid.
fn full_view() -> RunView {
    RunView {
        audio: Some(flythrough::hud::menu::AudioView {
            echoes: true,
            tail: true,
            echo_db: 0.0,
            tail_db: 0.0,
            predelay_ms: 0.0,
            glide: false,
            direct_delay: false,
            echo_range: (-24.0, 12.0),
            tail_range: (-24.0, 12.0),
        }),
        path: true,
        castle: true,
        model: true,
        coarse: true,
        ..RunView::default()
    }
}

/// One frame with `key` pressed through the menu's step at `m` and `v`.
fn key_in(menu: &mut Menu, m: &Modes, v: &RunView, k: KeyCode) -> Vec<Activated> {
    let mut input = Input::default();
    input.key_down(k);
    step(menu, &input, m, v).acts
}

/// Every menu-only entry is reached from a closed menu by `Tab`, its label typed and `Down`
/// to it among the matches (in a run that has what every entry needs: `--pan`, the outdoor
/// source and a sky among the modes, the rest in the view), and `Enter` hands the run that entry's own flip or action,
/// the change its former key made; on a number `Enter` opens its edit mode and `Up` hands the
/// run its step up.
#[test]
fn every_menu_only_entry_is_reached_by_its_words() {
    use flythrough::hud::menu::{EntryKind, Row, typed};
    let mut m = modes(true)
        .with_pan(10.0, 0.0)
        .with_source(Some("outdoor, trees sparse"));
    m.sky = true;
    let v = full_view();
    let keys: Vec<KeyCode> = flythrough::hud::menu::all_keys();
    for e in entries().filter(|e| e.keys().is_empty()) {
        let mut menu = Menu::default();
        key_in(&mut menu, &m, &v, KeyCode::Tab);
        for c in e.label.chars() {
            let k = *keys
                .iter()
                .find(|&&k| typed(k) == Some(c))
                .unwrap_or_else(|| panic!("{}: {c:?} types", e.label));
            key_in(&mut menu, &m, &v, k);
        }
        assert_eq!(menu.filter(), e.label);
        let at = menu
            .rows(&m, &v)
            .iter()
            .filter(|r| r.selectable())
            .position(|r| matches!(r, Row::Entry(x) if x.label == e.label && x.group == e.group))
            .unwrap_or_else(|| panic!("{} matches its label", e.label));
        for _ in 0..at {
            key_in(&mut menu, &m, &v, KeyCode::ArrowDown);
        }
        let mut acts = key_in(&mut menu, &m, &v, KeyCode::Enter);
        if e.is_number() {
            assert!(acts.is_empty() && menu.editing(), "{}", e.label);
            acts = key_in(&mut menu, &m, &v, KeyCode::ArrowUp);
        }
        match (e.kind, &acts[..]) {
            (EntryKind::Toggle(t), [Activated::Step(got, 1)]) => {
                assert!(std::ptr::eq(t, *got) && t.number.is_some(), "{}", e.label);
            }
            (EntryKind::Toggle(t), [Activated::Toggle(got)]) => {
                assert!(std::ptr::eq(t, *got), "{}", e.label);
                let (mut a, mut b) = (modes(true), modes(true));
                assert_eq!(press(got, &mut a, 1.0), press(t, &mut b, 1.0));
            }
            (EntryKind::Action(a), [Activated::Action(got)]) => assert_eq!(a.act, *got),
            _ => panic!("{}: {acts:?}", e.label),
        }
    }
}

/// The menu's glyph scale is the largest at which its 80 x 45 cells fit the window, at least
/// 2.
#[test]
fn the_panel_scale_fits_the_window() {
    for (w, h, want) in [
        (1280, 720, 2),
        (1920, 1080, 3),
        (2530, 1378, 3),
        (2560, 1440, 4),
        (800, 600, 2),
    ] {
        assert_eq!(panel_scale(w, h), want, "{w} x {h}");
        assert_eq!(Panel::of(w, h).scale, want, "{w} x {h}");
    }
}

/// At 2530 x 1378 (105 x 57 whole cells at scale 3) the root page's title starts at column
/// 12, row 6, and the backdrop covers columns 11 to 92 of the title row through one row
/// below the footer and no cell outside; at 1280 x 720 the panel starts at (0, 0) and the
/// backdrop covers the page's rows, cut to the window's 80 x 45 cells.
#[test]
fn the_panel_is_centred() {
    let m = modes(true);
    let mut menu = Menu::default();
    key(&mut menu, &m, KeyCode::Tab);
    let (page, colours) = drawn(&menu, &m);
    let used = page.lines().count();

    let panel = Panel::of(2530, 1378);
    assert_eq!((panel.cols, panel.rows), (105, 57));
    let (mut text, mut out) = (String::new(), Vec::new());
    let b = place(&page, &colours, panel, "", &mut text, &mut out);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 6 + used, "{text}");
    assert_eq!(out.len(), lines.len());
    assert!(lines[..6].iter().all(|l| l.trim().is_empty()), "{text}");
    assert_eq!(lines[6].find("menu"), Some(12), "{text}");
    for (i, l) in page.lines().enumerate() {
        assert_eq!(lines[6 + i].get(12..).unwrap_or(""), l, "row {i}");
        assert_eq!(out[6 + i], colours[i], "row {i}");
    }
    let footer = 6 + used - 1;
    for row in 0..57u32 {
        for col in 0..105u32 {
            let want = (11..=92).contains(&col) && (6..=footer as u32 + 1).contains(&row);
            assert_eq!(b.contains(col, row), want, "cell ({col}, {row}) in {b:?}");
        }
    }

    let panel = Panel::of(1280, 720);
    assert_eq!(
        (panel.col, panel.row, panel.cols, panel.rows),
        (0, 0, 80, 45)
    );
    let b = place(&page, &colours, panel, "", &mut text, &mut out);
    assert_eq!(text, page);
    assert_eq!(
        b,
        HudBox {
            col: 0,
            row: 0,
            cols: 80,
            rows: used as u32 + 1,
        }
    );
}

/// In a number's edit mode, `Up` held from 0 to 600 ms in 60 Hz frames steps once at the
/// press and once at 275 ms and every 50 ms after (`REPEAT_DELAY_MS`, `REPEAT_RATE_MS`):
/// 275, 325, ..., 575, eight steps in all; released and pressed again it steps at once.
#[test]
fn a_held_arrow_repeats() {
    use flythrough::hud::menu::{REPEAT_DELAY_MS, REPEAT_RATE_MS, step_at};
    assert_eq!((REPEAT_DELAY_MS, REPEAT_RATE_MS), (275.0, 50.0));
    let m = modes(true);
    let v = RunView::default();
    let mut menu = Menu::default();
    key(&mut menu, &m, KeyCode::Tab);
    key(&mut menu, &m, KeyCode::Enter);
    assert_eq!(menu.page(), Some(PageId::Group(Group::Moving)));
    assert!(key(&mut menu, &m, KeyCode::Enter).is_empty());
    assert!(menu.editing(), "Enter on the dig radius edits it");
    let steps = |acts: &[Activated]| {
        acts.iter()
            .filter(|a| matches!(a, Activated::Step(t, 1) if t.name == "dig radius"))
            .count()
    };
    let mut press = Input::default();
    press.key_down(KeyCode::ArrowUp);
    let mut total = steps(&step_at(&mut menu, &press, &m, &v, 0.0).acts);
    assert_eq!(total, 1, "the press steps");
    let mut held = Input::default();
    held.key_down(KeyCode::ArrowUp);
    held.end_frame();
    let mut at = Vec::new();
    for frame in 1..=36 {
        let t = f64::from(frame) * 1000.0 / 60.0;
        let n = steps(&step_at(&mut menu, &held, &m, &v, t).acts);
        if n > 0 {
            at.push(frame);
        }
        total += n;
    }
    assert_eq!(total, 8);
    // The frames whose time first passes 275, 325, ..., 575 ms.
    assert_eq!(at, [17, 20, 23, 26, 29, 32, 35]);
    assert_eq!(
        steps(&step_at(&mut menu, &Input::default(), &m, &v, 616.7).acts),
        0
    );
    assert_eq!(steps(&step_at(&mut menu, &press, &m, &v, 700.0).acts), 1);
    assert!(menu.editing());
}

/// Plan 0095 Phase 4: the `key far` row cycles baked, column, ray and back, its words naming
/// each, and the banner names the two that are not the default (column, Plan 0098 Phase 4).
#[test]
fn the_key_far_row_cycles_its_three_sources() {
    let t = toggle("key far");
    let mut m = modes(false);
    m.sky = true;
    let mut seen = Vec::new();
    for _ in 0..3 {
        press(t, &mut m, 1.0);
        seen.push(m.lighting.key_far);
        let mut words = String::new();
        m.write_key_far(&mut words);
        assert_eq!(
            words,
            match m.lighting.key_far {
                svet_render::KeyFar::Column => String::new(),
                k => format!("key far {}, ", k.name()),
            }
        );
    }
    use svet_render::KeyFar;
    assert_eq!(seen, [KeyFar::Ray, KeyFar::Baked, KeyFar::Column]);
}

/// Plan 0100 Phase 9: over each profile's lighting the `key at`, `key horizon` and `key cover`
/// rows read the profile's values (`hit`, `march`, `off`), and a whole cycle of each visits
/// only the values that stand (no `centre`, no `ray-record`) and returns to the profile's.
#[test]
fn the_key_rows_start_at_the_profiles_and_cycle_the_standing_values() {
    use svet_render::{Profile, RenderSettings};
    for p in Profile::ALL {
        let mut m = Modes::new(
            RenderSettings::profile(p).lighting,
            LightingResolution::Half,
            0.0,
        );
        for (name, cycle) in [
            (
                "key at",
                &["key at hit", "key at face", "key at face-smooth"][..],
            ),
            (
                "key horizon",
                &["key horizon march", "key horizon walk"][..],
            ),
            (
                "key cover",
                &["key cover off", "key cover ray", "key cover record"][..],
            ),
        ] {
            let t = toggle(name);
            let mut seen = vec![(t.get)(&m)];
            for _ in 1..cycle.len() {
                press(t, &mut m, 1.0);
                seen.push((t.get)(&m));
            }
            assert_eq!(seen, cycle, "{} under {}", name, p.name());
            press(t, &mut m, 1.0);
            assert_eq!((t.get)(&m), cycle[0], "{} under {}", name, p.name());
        }
    }
}
