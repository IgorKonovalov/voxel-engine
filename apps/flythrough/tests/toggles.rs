//! The toggle table against the HUD: every toggle's mode shows in the HUD's text grid, in the
//! row the table names. CPU only, no device.

use flythrough::hud::{HudRow, HudStats, Modes};
use flythrough::toggles::{TOGGLES, Toggle};
use vxe_render::{Lighting, LightingResolution};

/// The modes a run starts with, a pan running so `Period` has one to stop, and one dropped
/// light so `5` has one to flag.
fn start() -> Modes {
    let mut m = Modes::new(Lighting::default(), LightingResolution::Half, 0.0).with_pan(10.0, 0.0);
    m.lights
        .drop_at(true, [0.0; 3], [1.0, 0.0, 0.0], None)
        .unwrap();
    m
}

/// The HUD's rows for `modes`, as `HudStats::write_rows` writes them each frame.
fn grid(modes: &Modes) -> Vec<String> {
    let mut stats = HudStats::default();
    modes.write_rows(&mut stats.modes);
    let (mut text, mut colours) = (String::new(), Vec::new());
    stats.write_rows(&mut text, &mut colours);
    assert_eq!(text.lines().count(), colours.len(), "a colour per row");
    text.lines().map(str::to_owned).collect()
}

/// The HUD row `row` names in `grid`: the one row of the mode block that starts with its
/// prefix.
fn row_of(grid: &[String], row: HudRow) -> &str {
    let rows: Vec<&String> = grid
        .iter()
        .filter(|l| l.starts_with(row.prefix()))
        .collect();
    assert_eq!(rows.len(), 1, "{row:?}: {grid:#?}");
    rows[0]
}

/// Flips `t` once on `m` at `now` and asserts that its mode changed and that the HUD's row the
/// table names shows the new mode; returns the new mode's words.
fn flip_and_read(t: &Toggle, m: &mut Modes, now: f64) -> String {
    let before = (t.get)(m);
    (t.flip)(m, now);
    let after = (t.get)(m);
    assert_ne!(before, after, "{}: one press changes the mode", t.name);
    let g = grid(m);
    let row = row_of(&g, t.row);
    assert!(
        row.contains(&after),
        "{}: the {:?} row `{row}` shows `{after}`",
        t.name,
        t.row
    );
    after
}

/// Each toggle of the table, flipped once from the start, changes its mode, and the HUD's
/// grid shows the new mode in the row the table names: `cutaway off` after the cutaway's
/// second press (auto, on, off), `shadow ray off` after `L`, the dig radius a step up.
#[test]
fn every_toggle_shows_its_mode_in_the_hud_grid() {
    for t in TOGGLES {
        flip_and_read(t, &mut start(), 1000.0);
    }
    let named = |name: &str| {
        TOGGLES
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("{name} is a toggle"))
    };
    let mut m = start();
    assert_eq!(flip_and_read(named("cutaway"), &mut m, 1.0), "cutaway on");
    assert_eq!(flip_and_read(named("cutaway"), &mut m, 2.0), "cutaway off");
    assert_eq!(
        flip_and_read(named("shadow ray"), &mut m, 3.0),
        "shadow ray off"
    );
    assert_eq!(
        flip_and_read(named("dig radius"), &mut m, 4.0),
        "dig radius 0.6 m"
    );
    let g = grid(&m);
    assert_eq!(
        row_of(&g, HudRow::Controls),
        "cutaway off, shadow ray off, dig radius 0.6 m"
    );
    for row in HudRow::ALL {
        assert!(
            TOGGLES.iter().any(|t| t.row == row),
            "{row:?} shows a toggle"
        );
    }
}

/// No two inputs share a key: every key and pad button of the toggle table, of the actions
/// (`toggles::ACTIONS`) and of the fixed set (`hud::menu::FIXED_KEYS`) is bound once. `1` to
/// `1`, `4` and `6` are the lights' drop, removal and shot; the lights' carry, colour and
/// bounce and the bake queue's six toggles are the menu's only (no key), each changing its
/// row's words.
#[test]
fn no_two_inputs_share_a_key() {
    use flythrough::hud::menu::FIXED_KEYS;
    use flythrough::toggles::{ACTIONS, Bind, KEY_DROP, KEY_FIRE, KEY_REMOVE, Press};
    use vxe_app::KeyCode;
    let mut keys: Vec<Bind> = TOGGLES
        .iter()
        .flat_map(|t| match t.press {
            Press::Key(k, pad) => std::iter::once(Bind::Key(k))
                .chain(pad.map(Bind::Pad))
                .collect(),
            Press::Scroll | Press::Menu => Vec::new(),
        })
        .collect();
    keys.extend(ACTIONS.iter().flat_map(|a| a.binds.iter().copied()));
    keys.extend(FIXED_KEYS.iter().map(|&k| Bind::Key(k)));
    for (i, k) in keys.iter().enumerate() {
        assert!(!keys[i + 1..].contains(k), "{k:?} is bound twice");
    }
    assert_eq!(
        [KEY_DROP, KEY_REMOVE, KEY_FIRE],
        [KeyCode::Digit1, KeyCode::Digit4, KeyCode::Digit6]
    );
    let named = |name: &str| {
        TOGGLES
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("{name} is a toggle"))
    };
    for name in ["carry", "colour", "bounce"] {
        assert_eq!(named(name).press, Press::Menu, "{name}");
        assert_eq!(named(name).row, HudRow::Lights, "{name}");
    }
    for name in [
        "queue order",
        "queue angle halved",
        "queue angle doubled",
        "queue withdraw",
        "bake budget",
        "full-first",
    ] {
        let t = named(name);
        assert_eq!(t.press, Press::Menu, "{name}");
        assert_eq!(t.row, HudRow::Queue);
        let mut m = start();
        let before = m.row(HudRow::Queue);
        (t.flip)(&mut m, 1.0);
        assert_ne!(m.row(HudRow::Queue), before, "{name} changes the queue row");
    }
}
