//! The menu marks an entry whose settings row is inert under the run's live settings, and
//! the notice of its change says so: every condition of `crate::inert::INERT_WHEN`, held and
//! not held.

use super::hud_live_tests::{hold, run_of, sky_args};
use super::menu_tests::{frame_keys, pressed, select, selected_row};
use super::*;
use crate::inert::INERT_WHEN;
use svet_app::KeyCode;

/// The run under the sky started with the settings `live_under` gives: its live settings
/// hold them.
fn run_under(live_under: Option<fn(&mut svet_render::RenderSettings)>) -> Run {
    let mut s = run_of(sky_args()).live_settings();
    if let Some(f) = live_under {
        f(&mut s);
    }
    let mut args = sky_args();
    hold(&mut args, &s);
    run_of(args)
}

/// The menu entry labelled `label`.
fn entry(label: &str) -> crate::hud::menu::Entry {
    crate::hud::menu::entries()
        .find(|e| e.label == label)
        .unwrap_or_else(|| panic!("{label} is a menu entry"))
}

/// For every row of the table with a menu entry: under the sky run's defaults the condition
/// holds and the entry's selected row ends `(inert: <reason>)`; under the row's `live_under`
/// it does not hold and the row carries no mark. A row with no `live_under` holds under the
/// defaults alone.
#[test]
fn an_inert_row_s_entry_is_marked_while_its_condition_holds() {
    let mut checked = 0;
    for r in INERT_WHEN {
        let base = run_under(None);
        assert!(
            (r.holds)(&base.live_settings()),
            "{} holds at the start",
            r.row
        );
        let Some(label) = r.entry else { continue };
        let mark = format!("(inert: {})", r.reason);
        let mut run = base;
        select(&mut run, &entry(label));
        let (row, _) = selected_row(&mut run);
        assert!(row.contains(&mark), "{label}: {row:?} lacks {mark:?}");
        let Some(live) = r.live_under else { continue };
        let mut run = run_under(Some(live));
        assert!(
            !(r.holds)(&run.live_settings()),
            "{} holds under its live_under",
            r.row
        );
        select(&mut run, &entry(label));
        let (row, _) = selected_row(&mut run);
        assert!(!row.contains("(inert:"), "{label}: {row:?} is marked");
        checked += 1;
    }
    assert!(checked > 0, "no row with an entry and a live_under");
}

/// Every menu entry, activated on the sky run (`Enter`; a number then `Up`, or `Down` at its
/// top): when it changes a row of the table that row names it as its `entry`, and every
/// `entry` of the table changes its row; an entry marked inert says the mark in the notice.
#[test]
fn the_table_names_the_entry_that_changes_each_row() {
    let mut named = Vec::new();
    for e in crate::hud::menu::entries() {
        let mut run = run_under(None);
        if e.available(&run.modes, &run.run_view()).is_err() {
            continue;
        }
        select(&mut run, &e);
        let before = run.live_settings();
        run.notice = None;
        // The menu's own keys, typed as a player types them.
        // keys:typed-begin
        frame_keys(&mut run, &pressed(KeyCode::Enter));
        if e.is_number() {
            frame_keys(&mut run, &pressed(KeyCode::ArrowUp));
            if run.live_settings() == before {
                frame_keys(&mut run, &pressed(KeyCode::ArrowDown));
            }
        }
        // keys:typed-end
        let changed = before.changed_fields(&run.live_settings());
        for r in INERT_WHEN.iter().filter(|r| changed.contains(&r.row)) {
            assert_eq!(r.entry, Some(e.label), "{} changes {}", e.label, r.row);
            named.push(e.label);
            if (r.holds)(&run.live_settings()) {
                let mark = format!("(inert: {})", r.reason);
                let notice = run.notice.as_ref().map(|(l, _)| l.as_str());
                assert!(
                    notice.is_some_and(|l| l.ends_with(&mark)),
                    "{}: notice {notice:?} lacks {mark:?}",
                    e.label
                );
            }
        }
    }
    for r in INERT_WHEN {
        if let Some(label) = r.entry {
            assert!(named.contains(&label), "{label} does not change {}", r.row);
        }
    }
}

/// `inert_rows` lists the rows `Inert` marks, each a row of the settings.
#[test]
fn inert_rows_are_settings_rows() {
    let s = run_under(None).live_settings();
    let rows = crate::inert::inert_rows(&s);
    assert_eq!(rows.len(), INERT_WHEN.len(), "all hold at the start");
    for (row, _) in rows {
        assert!(
            svet_render::RenderSettings::class_of(row).is_some(),
            "{row} is no row"
        );
    }
}
