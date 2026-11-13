//! The toggle table against the HUD: every toggle's mode shows in the HUD's text grid, in the
//! row the table names. CPU only, no device.

use flythrough::hud::{HudRow, HudStats, Modes};
use flythrough::toggles::{TOGGLES, Toggle};
use vxe_render::{Lighting, LightingResolution};

/// The modes a run starts with, a pan running so `Period` has one to stop.
fn start() -> Modes {
    Modes::new(Lighting::default(), LightingResolution::Half, 0.0).with_pan(10.0, 0.0)
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
