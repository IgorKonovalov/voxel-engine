//! The render settings rows that change nothing under the settings that hold them, and why:
//! one table the window and the settings-rows walk (`tests/gpu_settings_rows.rs`) both read.
//! The walk proves each row inert under the fly-through's defaults and live under the
//! settings its `live_under` gives; the menu marks an entry whose row is inert under the run's
//! live settings (`penumbra disc (inert: key far column)`) and the notice of its change says
//! the same. Rows inert for what the run cannot tell (a rebuild, the live queue, motion) stay
//! in the walk's own list.

use svet_render::{KeyFar, NormalMode, RenderSettings};

/// One row and the condition under which it changes nothing.
pub struct InertRow {
    /// The row's name in `RenderSettings::FIELDS`.
    pub row: &'static str,
    /// The menu entry that changes the row, if one does.
    pub entry: Option<&'static str>,
    /// Why, in a few words: the window prints it after the entry's.
    pub reason: &'static str,
    /// Whether the row changes nothing under `s`.
    pub holds: fn(&RenderSettings) -> bool,
    /// Rows to set so it acts (the walk renders it there); `None` when no arrangement is
    /// known to.
    pub live_under: Option<fn(&mut RenderSettings)>,
}

/// A cube's face is hit: the field look (`NormalMode::Contour`) moves the hit off it.
fn cubes(s: &RenderSettings) -> bool {
    s.look.normal_mode != NormalMode::Contour
}

/// Under key far column a cube face's key light is the column map's: no key ray is cast.
fn key_column(s: &RenderSettings) -> bool {
    s.lighting.key_far == KeyFar::Column && cubes(s)
}

/// With no light history a face's sky term is its record's or the column's: no sky ray shows.
fn no_light_history(s: &RenderSettings) -> bool {
    !s.lighting.light_history && cubes(s)
}

/// Only key far baked blends the store's key visibility in the band.
fn key_not_baked(s: &RenderSettings) -> bool {
    s.lighting.key_far != KeyFar::Baked && cubes(s)
}

fn key_ray(s: &mut RenderSettings) {
    s.lighting.key_far = KeyFar::Ray;
}

fn light_history(s: &mut RenderSettings) {
    s.lighting.light_history = true;
}

fn key_baked(s: &mut RenderSettings) {
    s.lighting.key_far = KeyFar::Baked;
}

/// A row of the key ray's, inert under key far column.
const fn key(row: &'static str, entry: Option<&'static str>) -> InertRow {
    InertRow {
        row,
        entry,
        reason: "key far column",
        holds: key_column,
        live_under: Some(key_ray),
    }
}

/// A row of the sky rays', inert with no light history.
const fn sky(row: &'static str, entry: Option<&'static str>) -> InertRow {
    InertRow {
        row,
        entry,
        reason: "light history off",
        holds: no_light_history,
        live_under: Some(light_history),
    }
}

/// The table: one line per row.
pub static INERT_WHEN: &[InertRow] = &[
    key("lighting.penumbra", Some("penumbra")),
    key("lighting.key_cap", None),
    key("lighting.key_history", None),
    sky("lighting.sky_rays", Some("sky rays")),
    sky("lighting.sky_history", None),
    sky("lighting.sky_down_skip", None),
    InertRow {
        row: "lighting.key_far_mix",
        entry: Some("key far mix"),
        reason: "key far not baked",
        holds: key_not_baked,
        live_under: Some(key_baked),
    },
    // Under the auto exposure its preset's own weights meter; under a manual one the meter
    // feeds the exposure readout alone.
    InertRow {
        row: "metering",
        entry: None,
        reason: "the meter feeds the readout only",
        holds: |_| true,
        live_under: None,
    },
];

const _: () = assert!(INERT_WHEN.len() <= 32, "Inert holds a bit per row");

/// The rows inert under `s`, with their reasons.
pub fn inert_rows(s: &RenderSettings) -> Vec<(&'static str, &'static str)> {
    INERT_WHEN
        .iter()
        .filter(|r| (r.holds)(s))
        .map(|r| (r.row, r.reason))
        .collect()
}

/// The rows of [`INERT_WHEN`] inert under one run's live settings, a bit each: what the menu
/// reads to mark an entry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inert(u32);

impl Inert {
    /// The rows inert under `s`.
    pub fn of(s: &RenderSettings) -> Self {
        Self(
            INERT_WHEN
                .iter()
                .enumerate()
                .filter(|(_, r)| (r.holds)(s))
                .fold(0, |bits, (i, _)| bits | 1 << i),
        )
    }

    /// Why the menu entry `label` changes nothing now, when its row is inert.
    pub fn entry(self, label: &str) -> Option<&'static str> {
        INERT_WHEN
            .iter()
            .enumerate()
            .find(|(i, r)| self.0 & 1 << i != 0 && r.entry == Some(label))
            .map(|(_, r)| r.reason)
    }
}

/// `words` with the mark of an inert row, `words (inert: why)`.
pub fn marked(words: &str, why: Option<&str>) -> String {
    match why {
        Some(why) => format!("{words} (inert: {why})"),
        None => words.to_owned(),
    }
}
