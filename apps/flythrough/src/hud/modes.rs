//! The banner's rows and the modes it names: the pose banner, the mode keys, the adaptation,
//! [`Modes`], and the path and help rows.

use super::*;
use std::fmt::Write as _;

/// `--pose`'s banner row, shown until the camera first moves: `pose vein`.
pub fn pose_banner(name: &str) -> String {
    format!("pose {name}")
}

/// How long the mode banner shows after the start or a change, in milliseconds.
pub const BANNER_MS: f64 = 3000.0;
/// The banner's widest row in characters: it fits a 1280-pixel window at the HUD's scale 2
/// (80 cells) and the golden's 320 pixels at scale 1 (40 cells).
pub const BANNER_COLS: usize = 40;
/// Rows the banner wraps into at most: the look's words in three (the look and the view) and the modes' line in six.
pub const BANNER_ROWS: usize = 9;

/// A key that changes the lighting mode: `F3`, `F5` to `F8`, `M` and `Q`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeKey {
    /// `F3`: full or coarse shadow walk.
    ShadowWalk,
    /// `F5`: edit boxes, or edit boxes and the gradient.
    History,
    /// `F6`: full or half lighting resolution.
    Resolution,
    /// `F7`: the headlamp.
    Lamp,
    /// `F8`: the lamp's bounce from the probe volume.
    LampBounce,
    /// `M`: the ore's emission pulse.
    Pulse,
    /// `Q`: the exposure: auto with the start's preset, auto symmetric, manual.
    Exposure,
    /// `Comma`: the store read by face or by the shading normal (ADR-0049).
    StoreRead,
    /// `Semicolon`: the light's filters weigh by the shading normal or the face normal.
    LightWeights,
}

/// The auto exposure's preset, as `--adaptation` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Adaptation {
    /// `AutoExposure::eye()`.
    Eye,
    /// `AutoExposure::symmetric(1.5)`: one rate both ways, 0.67 s.
    Symmetric,
    /// `AutoExposure::instant()`.
    Instant,
}

impl Adaptation {
    pub const ALL: [Self; 3] = [Self::Eye, Self::Symmetric, Self::Instant];

    pub fn name(self) -> &'static str {
        match self {
            Self::Eye => "eye",
            Self::Symmetric => "symmetric",
            Self::Instant => "instant",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.name() == name)
    }

    pub fn preset(self) -> vxe_render::AutoExposure {
        match self {
            Self::Eye => vxe_render::AutoExposure::eye(),
            Self::Symmetric => vxe_render::AutoExposure::symmetric(1.5),
            Self::Instant => vxe_render::AutoExposure::instant(),
        }
    }
}

impl ModeKey {
    pub const ALL: [Self; 5] = [
        Self::ShadowWalk,
        Self::History,
        Self::Resolution,
        Self::Lamp,
        Self::Pulse,
    ];
}

/// The lighting modes the window's keys switch, and the banner that names them: shown for
/// [`BANNER_MS`] after the start and after every change, by the frame loop's clock (`now_ms`),
/// so a test drives it without a window.
#[derive(Clone, Debug, PartialEq)]
pub struct Modes {
    pub lighting: vxe_render::Lighting,
    pub resolution: vxe_render::LightingResolution,
    /// The ore's emission scale pulses.
    pub pulse: bool,
    /// The world source the line names first (`source showcase`), when it names one.
    pub source: Option<&'static str>,
    banner_at: f64,
    /// The auto exposure `Q` returns to, under its preset's name.
    auto: (Adaptation, vxe_render::AutoExposure),
    /// The exposure state as last read back.
    readout: Option<vxe_render::ExposureReadout>,
    /// The look's words, from the first [`Modes::show_look`]: the banner's first row on.
    look_words: Option<String>,
    /// The view's name after the look's words, from [`Modes::show_view`] with any view but
    /// `shaded`.
    view: Option<&'static str>,
    /// Voxels between the probe volume's probes: the line names it (`probes 16`) while the
    /// lamp's bounce is on.
    probe_spacing: u32,
    /// The look the frame takes: `N` steps its preset's normal mode and tuning, `V` its far
    /// normal, `F9` its view.
    pub(crate) look: vxe_render::LookParams,
    /// The preset `N` last chose, or `--look`'s with its part flags.
    pub(crate) preset: crate::LookPreset,
    /// The run's mirror holds the field's band slots, so `N` steps through the field look.
    pub(crate) field_surface: bool,
    /// The upscaler `F10` cycles and `Z`'s sharpening, with their banner.
    pub(crate) upscaler: UpscalerBanner,
    /// The internal scale's name the upscaler runs at when it is not off.
    pub(crate) scale: &'static str,
    /// `--pan`'s yaw, which `Period` stops and starts.
    pub(crate) pan: Option<Pan>,
    /// The cutaway's mode, `X` (or the pad's `West`) cycles it.
    pub(crate) cutaway: crate::fly::CutawayMode,
    /// The lamp's shadow ray, `L` toggles it.
    pub(crate) shadow_ray: bool,
    /// The dig's radius in fixed point; the scroll wheel and the D-pad step it.
    pub(crate) dig_radius: i64,
    /// The live queue's modes, keys `1` to `8`.
    pub(crate) queue: QueueModes,
}

/// The live queue's modes the number keys step: `1` the order, `2` and `3` the score's angle
/// weight, `4` the withdrawal, `5` the budget, `6` the full-first radius, `7` the missing
/// light, `8` the reach.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QueueModes {
    /// The class and the distance first, rather than the score.
    pub distance: bool,
    /// The class as the score's leading key, rather than a weight.
    pub tier: bool,
    pub angle: f32,
    pub refine: f32,
    pub withdraw: bool,
    pub budget: vxe_render::Budget,
    pub full_first: Option<i32>,
    pub missing_light: vxe_render::MissingLight,
    pub reach: vxe_render::ReachMode,
}

impl Default for QueueModes {
    fn default() -> Self {
        QueueModes {
            distance: false,
            tier: true,
            angle: vxe_render::QUEUE_ANGLE_WEIGHT,
            refine: vxe_render::QUEUE_REFINE_WEIGHT,
            withdraw: true,
            budget: vxe_render::Budget::default(),
            full_first: Some(vxe_render::DEFAULT_NEAR),
            missing_light: vxe_render::MissingLight::Chain,
            reach: vxe_render::ReachMode::Full,
        }
    }
}

impl QueueModes {
    /// The live queue's order.
    pub fn order(&self) -> vxe_render::QueueOrder {
        use vxe_render::{QueueOrder, Refine};
        if self.distance {
            QueueOrder::Distance
        } else {
            QueueOrder::Score {
                angle: self.angle,
                refine: if self.tier {
                    Refine::Tier
                } else {
                    Refine::Weight(self.refine)
                },
            }
        }
    }
}

/// A row of the HUD's mode block ([`Modes::write_rows`]); each toggle names the row that
/// shows its mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HudRow {
    /// `look face, far normal [2, 6], view shaded`.
    Look,
    /// The lighting modes' line ([`Modes::line`]).
    Lighting,
    /// `upscaler fsr2 at 1.5, sharpen 0.50, pan off`.
    Upscaler,
    /// `queue order score, ...; withdraw on; budget ...; full-first 2; missing light chain;
    /// reach full`.
    Queue,
    /// `cutaway auto, shadow ray on, dig radius 0.5 m`.
    Controls,
}

impl HudRow {
    pub const ALL: [Self; 5] = [
        Self::Look,
        Self::Lighting,
        Self::Upscaler,
        Self::Queue,
        Self::Controls,
    ];

    /// The first word of the row, which no other HUD row starts with.
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Look => "look ",
            Self::Lighting => "lighting ",
            Self::Upscaler => "upscaler ",
            Self::Queue => "queue ",
            Self::Controls => "cutaway ",
        }
    }
}

impl Modes {
    /// The modes at the start, the banner shown from `now_ms`.
    pub fn new(
        lighting: vxe_render::Lighting,
        resolution: vxe_render::LightingResolution,
        now_ms: f64,
    ) -> Self {
        Self {
            lighting,
            resolution,
            pulse: false,
            source: None,
            banner_at: now_ms,
            auto: match lighting.exposure {
                vxe_render::Exposure::Auto(a) => (Adaptation::Eye, a),
                vxe_render::Exposure::Manual(_) => (Adaptation::Eye, Adaptation::Eye.preset()),
            },
            readout: None,
            look_words: None,
            view: None,
            probe_spacing: vxe_render::DEFAULT_PROBE_SPACING,
            look: vxe_render::LookParams::default(),
            preset: crate::LookPreset::DEFAULT,
            field_surface: false,
            upscaler: UpscalerBanner::new(UpscalerMode::Off, "1"),
            scale: "1.5",
            pan: None,
            cutaway: crate::fly::CutawayMode::Auto,
            shadow_ray: true,
            dig_radius: crate::edits::DIG_RADIUS,
            queue: QueueModes::default(),
        }
    }

    /// These modes with `--pan`'s yaw at `deg_per_s`, started at `now_ms`.
    pub fn with_pan(self, deg_per_s: f32, now_ms: f64) -> Self {
        Self {
            pan: Some(Pan::new(deg_per_s, now_ms)),
            ..self
        }
    }

    /// Writes the mode block, one row per [`HudRow`] in order, each ending in a line break.
    pub fn write_rows(&self, out: &mut String) {
        for row in HudRow::ALL {
            self.write_row(row, out);
            out.push('\n');
        }
    }

    /// The row `row` of the mode block, as the HUD shows it and a key press prints it.
    pub fn row(&self, row: HudRow) -> String {
        let mut out = String::new();
        self.write_row(row, &mut out);
        out
    }

    /// Appends the row `row` of the mode block to `out`.
    pub fn write_row(&self, row: HudRow, out: &mut String) {
        let on = |b: bool| if b { "on" } else { "off" };
        match row {
            HudRow::Look => {
                // Written in place, as `LookPreset::far_normal_words` words it: the HUD
                // allocates nothing after its first frame.
                let _ = write!(out, "look {}, far normal ", self.preset.name);
                let [lo, hi] = self.look.far_normal;
                if self.look.normal_mode == vxe_render::NormalMode::Face && hi > lo {
                    let _ = write!(out, "{lo}..{hi}");
                } else {
                    out.push_str("off");
                }
                let _ = write!(out, ", view {}", self.look.view.name());
            }
            HudRow::Lighting => self.write_line(out),
            HudRow::Upscaler => {
                let _ = write!(
                    out,
                    "upscaler {}, sharpen {:.2}, pan ",
                    self.upscaler.mode.name(),
                    self.upscaler.sharpen
                );
                match &self.pan {
                    Some(p) => out.push_str(on(p.on)),
                    None => out.push_str("none"),
                }
            }
            HudRow::Queue => {
                let q = &self.queue;
                // `QueueOrder::words`, written in place.
                match q.order() {
                    vxe_render::QueueOrder::Score { angle, refine } => {
                        let _ = write!(out, "queue order score, angle {angle}, refine ");
                        match refine {
                            vxe_render::Refine::Weight(r) => {
                                let _ = write!(out, "{r}");
                            }
                            vxe_render::Refine::Tier => out.push_str("tier"),
                        }
                    }
                    vxe_render::QueueOrder::Distance => out.push_str("queue order distance"),
                }
                let _ = write!(
                    out,
                    "; withdraw {}; budget {}; full-first ",
                    on(q.withdraw),
                    q.budget
                );
                match q.full_first {
                    Some(r) => {
                        let _ = write!(out, "{r}");
                    }
                    None => out.push_str("off"),
                }
                let _ = write!(
                    out,
                    "; missing light {}; reach {}",
                    q.missing_light.name(),
                    q.reach.name()
                );
            }
            HudRow::Controls => {
                let _ = write!(
                    out,
                    "cutaway {}, shadow ray {}, dig radius {:.1} m",
                    self.cutaway.name(),
                    on(self.shadow_ray),
                    self.dig_radius as f64 / (10 * vxe_world::FIXED_ONE) as f64
                );
            }
        }
    }

    /// These modes with the line starting `source NAME` when `source` is given.
    pub fn with_source(self, source: Option<&'static str>) -> Self {
        Self { source, ..self }
    }

    /// These modes with the probe volume's spacing `spacing` voxels (`probes N` in the line).
    pub fn with_probe_spacing(self, spacing: u32) -> Self {
        Self {
            probe_spacing: spacing,
            ..self
        }
    }

    /// The auto exposure `Q` returns to and its preset's name; the current exposure too while
    /// it is auto.
    pub fn set_auto(&mut self, adaptation: Adaptation, auto: vxe_render::AutoExposure) {
        self.auto = (adaptation, auto);
        if let vxe_render::Exposure::Auto(_) = self.lighting.exposure {
            self.lighting.exposure = vxe_render::Exposure::Auto(auto);
        }
    }

    /// Keeps the latest exposure read-back for the banner, the row and `Q`'s manual hold.
    pub fn set_readout(&mut self, readout: Option<vxe_render::ExposureReadout>) {
        if readout.is_some() {
            self.readout = readout;
        }
    }

    /// The auto symmetric preset `Q` passes through, with the start's metering.
    fn symmetric(&self) -> vxe_render::AutoExposure {
        vxe_render::AutoExposure {
            metering: self.auto.1.metering,
            ..Adaptation::Symmetric.preset()
        }
    }

    /// Shows the banner from `now_ms` with the look in words, `look field, far normal off`:
    /// the first row of every banner from then on. `far_normal` is the run's
    /// (`LookParams::far_normal`), which `N` leaves alone; it is named in face mode only.
    pub fn show_look(&mut self, preset: &crate::LookPreset, far_normal: [f32; 2], now_ms: f64) {
        self.look_words = Some(format!(
            "look {}, {}",
            preset.name,
            crate::LookPreset::far_normal_words(preset.normal_mode, far_normal)
        ));
        self.banner_at = now_ms;
    }

    /// Shows the banner from `now_ms` with `view NAME` after the look's words, or without it
    /// for the shaded view.
    pub fn show_view(&mut self, view: vxe_render::ViewMode, now_ms: f64) {
        self.view = (view != vxe_render::ViewMode::Shaded).then(|| view.name());
        self.banner_at = now_ms;
    }

    /// Switches the mode `key` changes and shows the banner from `now_ms`.
    pub fn press(&mut self, key: ModeKey, now_ms: f64) {
        use vxe_render::{HistoryKill, LightingResolution, ShadowWalk};
        let symmetric = self.symmetric();
        let l = &mut self.lighting;
        match key {
            ModeKey::ShadowWalk => {
                l.shadow_walk = match l.shadow_walk {
                    ShadowWalk::Full => ShadowWalk::Coarse,
                    ShadowWalk::Coarse => ShadowWalk::Full,
                }
            }
            ModeKey::History => {
                l.history = match l.history {
                    HistoryKill::Edits => HistoryKill::Gradient,
                    HistoryKill::Gradient => HistoryKill::Edits,
                }
            }
            ModeKey::Resolution => {
                self.resolution = match self.resolution {
                    LightingResolution::Full => LightingResolution::Half,
                    LightingResolution::Half => LightingResolution::Full,
                }
            }
            ModeKey::StoreRead => {
                l.store_read = match l.store_read {
                    vxe_render::StoreRead::Face => vxe_render::StoreRead::Normal,
                    vxe_render::StoreRead::Normal => vxe_render::StoreRead::Face,
                }
            }
            ModeKey::LightWeights => {
                l.light_weights = match l.light_weights {
                    vxe_render::LightWeights::Normal => vxe_render::LightWeights::Face,
                    vxe_render::LightWeights::Face => vxe_render::LightWeights::Normal,
                }
            }
            ModeKey::Lamp => l.lamp = !l.lamp,
            ModeKey::LampBounce => l.lamp_bounce = !l.lamp_bounce,
            ModeKey::Pulse => self.pulse = !self.pulse,
            ModeKey::Exposure => {
                use vxe_render::Exposure;
                l.exposure = match l.exposure {
                    Exposure::Auto(a)
                        if a == self.auto.1 && self.auto.0 != Adaptation::Symmetric =>
                    {
                        Exposure::Auto(symmetric)
                    }
                    Exposure::Auto(_) => Exposure::Manual(self.readout.map_or(1.0, |r| r.exposure)),
                    Exposure::Manual(_) => Exposure::Auto(self.auto.1),
                };
            }
        }
        self.banner_at = now_ms;
    }

    /// The exposure's mode in words, as the banner and the exposure row name it: `auto eye`,
    /// `auto symmetric`, `auto instant` or `manual`.
    pub fn exposure_mode(&self) -> String {
        let mut out = String::new();
        self.write_exposure_mode(&mut out);
        out
    }

    /// Appends [`Modes::exposure_mode`]'s words to `out`.
    pub fn write_exposure_mode(&self, out: &mut String) {
        match self.lighting.exposure {
            vxe_render::Exposure::Manual(_) => out.push_str("manual"),
            vxe_render::Exposure::Auto(a) if a == self.auto.1 => {
                out.push_str("auto ");
                out.push_str(self.auto.0.name());
            }
            vxe_render::Exposure::Auto(_) => out.push_str("auto symmetric"),
        }
    }

    /// The exposure row: the mode, the exposure in EV, the metered scene, and in auto the
    /// exposure the meter settles at.
    pub fn exposure_row(&self) -> ExposureRow {
        let mut row = ExposureRow::default();
        self.fill_exposure_row(&mut row);
        row
    }

    /// Writes [`Modes::exposure_row`] into `row` in place, reusing its mode's string.
    pub fn fill_exposure_row(&self, row: &mut ExposureRow) {
        let scene = self.readout.filter(|r| !r.held).map(|r| r.metered_log2);
        row.mode.clear();
        self.write_exposure_mode(&mut row.mode);
        row.ev = self.exposure_ev();
        row.scene_ev = scene;
        row.target_ev = match self.lighting.exposure {
            vxe_render::Exposure::Auto(a) => scene.map(|t| a.exposure(f64::from(t)).log2() as f32),
            vxe_render::Exposure::Manual(_) => None,
        };
    }

    /// The light's resolution and its filters in words, as the aliasing row names them:
    /// `store read face, weights normal, lighting half`.
    pub fn write_filter_words(&self, out: &mut String) {
        let _ = write!(
            out,
            "store read {}, weights {}, lighting {}",
            self.lighting.store_read.name(),
            self.lighting.light_weights.name(),
            self.resolution.name()
        );
    }

    /// The modes in words: `lighting half, store read face, weights normal, shadow walk coarse, history edits, lamp on,
    /// lamp bounce on at 64 rays 1/8, probes 16, filter on, ore pulse off, exposure auto eye +1.53
    /// EV` (the probe rays per update, the share of probes refreshed a frame, while the lamp's
    /// bounce is on the voxels between probes, and the exposure in EV, `log2 e`), after
    /// `source NAME, ` when the modes name a source.
    pub fn line(&self) -> String {
        let mut out = String::new();
        self.write_line(&mut out);
        out
    }

    /// Appends [`Modes::line`] to `out`.
    pub fn write_line(&self, out: &mut String) {
        let l = &self.lighting;
        let on = |b: bool| if b { "on" } else { "off" };
        if let Some(s) = self.source {
            let _ = write!(out, "source {s}, ");
        }
        let _ = write!(
            out,
            "lighting {}, store read {}, weights {}, shadow walk {}, history {}, lamp {}, lamp bounce {} at {} rays 1/{}",
            self.resolution.name(),
            l.store_read.name(),
            l.light_weights.name(),
            l.shadow_walk.name(),
            l.history.name(),
            on(l.lamp),
            on(l.lamp_bounce),
            l.rays(),
            l.slices(),
        );
        if l.lamp_bounce {
            let _ = write!(out, ", probes {}", self.probe_spacing);
        }
        let _ = write!(
            out,
            ", filter {}, ore pulse {}, exposure ",
            on(l.filter),
            on(self.pulse)
        );
        self.write_exposure_mode(out);
        let _ = write!(out, " {:+.2} EV", self.exposure_ev());
    }

    /// The exposure in EV the banner names: `log2` of the manual exposure, or in auto of the
    /// last read-back's (1.0 before the first).
    pub fn exposure_ev(&self) -> f32 {
        match self.lighting.exposure {
            vxe_render::Exposure::Manual(e) => e.log2(),
            vxe_render::Exposure::Auto(_) => self.readout.map_or(0.0, |r| r.exposure.log2()),
        }
    }

    /// The banner at `now_ms`: the look's words once [`Modes::show_look`] gave them,
    /// `view NAME` once [`Modes::show_view`] gave a view other than shaded, then the modes'
    /// line, within [`BANNER_MS`] of the start or the last change; each wrapped at its commas
    /// into rows of at most [`BANNER_COLS`] characters, one per text line; else `None`.
    pub fn banner(&self, now_ms: f64) -> Option<String> {
        let mut out = String::new();
        self.write_banner(now_ms, &mut out).then_some(out)
    }

    /// Appends [`Modes::banner`]'s rows to `out` as banner rows ([`start_row`]); `false`, and
    /// nothing written, when the banner does not show at `now_ms`.
    pub fn write_banner(&self, now_ms: f64, out: &mut String) -> bool {
        if now_ms - self.banner_at >= BANNER_MS {
            return false;
        }
        if self.look_words.is_some() || self.view.is_some() {
            start_row(out);
            let from = out.len();
            if let Some(look) = &self.look_words {
                out.push_str(look);
            }
            if let Some(v) = self.view {
                if out.len() > from {
                    out.push_str(", ");
                }
                out.push_str("view ");
                out.push_str(v);
            }
            wrap_from(out, from);
        }
        start_row(out);
        let from = out.len();
        self.write_line(out);
        wrap_from(out, from);
        true
    }
}

/// Starts a banner row in `out`: a line break unless `out` is empty.
pub fn start_row(out: &mut String) {
    if !out.is_empty() {
        out.push('\n');
    }
}

/// Appends a path run's mode row to `out` as a banner row: `benchmark 23 s / 37 s`, `tour -
/// move or look to fly`, `free flight - T resumes the tour`; nothing once the run ended. Under
/// `--tester` a second row names the help key.
pub fn write_path_row(out: &mut String, player: &crate::path::PathPlayer, tester: bool) {
    use crate::path::{Drive, PathMode};
    start_row(out);
    match (player.mode, player.drive) {
        (PathMode::Bench, Drive::Path { t_s }) => {
            let _ = write!(
                out,
                "benchmark {:.0} s / {:.0} s",
                t_s.floor(),
                player.path.duration_s().ceil()
            );
        }
        (PathMode::Tour, Drive::Path { .. }) => out.push_str("tour - move or look to fly"),
        (_, Drive::Free) => out.push_str("free flight - T resumes the tour"),
        (_, Drive::Aborted | Drive::Done) => return,
    }
    if tester {
        start_row(out);
        out.push_str("F1 help, Esc ends the run");
    }
}

/// Appends the tester mode's help panel to `out` as banner rows: the scene and mode, then the
/// keys the mode takes.
pub fn write_help_panel(out: &mut String, player: &crate::path::PathPlayer) {
    use crate::path::PathMode;
    start_row(out);
    let _ = write!(
        out,
        "scene {}, {}",
        player.path.scene.name(),
        match player.mode {
            PathMode::Bench => "benchmark",
            PathMode::Tour => "tour",
        }
    );
    let rows: &[&str] = match player.mode {
        PathMode::Bench => &[
            "the camera flies the path by itself",
            "Esc aborts the benchmark",
        ],
        PathMode::Tour => &[
            "W A S D move, Space C up and down",
            "Shift faster, the mouse looks",
            "left click digs in free flight",
            "T resumes the tour",
            "F1 hides this, Esc ends the run",
        ],
    };
    for r in rows {
        start_row(out);
        out.push_str(r);
    }
}

/// Wraps `out` from byte `from` on, parts joined by `, `, into rows of at most [`BANNER_COLS`]
/// characters where they fit: each `, ` before a part that would overrun its row becomes a line
/// break. In place; the text only shrinks, so nothing is allocated.
fn wrap_from(out: &mut String, from: usize) {
    let mut row_start = from;
    let mut search = from;
    while let Some(i) = out[search..].find(", ").map(|i| search + i) {
        let part_end = out[i + 2..].find(", ").map_or(out.len(), |j| i + 2 + j);
        if part_end - row_start > BANNER_COLS {
            out.replace_range(i..i + 2, "\n");
            row_start = i + 1;
            search = i + 1;
        } else {
            search = i + 2;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `parts` joined by `, ` into rows of at most [`BANNER_COLS`] characters where they fit,
    /// row by row: the reference [`wrap_from`] is held to.
    fn wrap(parts: &[&str]) -> Vec<String> {
        let mut rows = Vec::new();
        let mut row = String::new();
        for part in parts {
            if !row.is_empty() && row.len() + 2 + part.len() > BANNER_COLS {
                rows.push(std::mem::take(&mut row));
            }
            if !row.is_empty() {
                row.push_str(", ");
            }
            row.push_str(part);
        }
        if !row.is_empty() {
            rows.push(row);
        }
        rows
    }

    /// Wrapping in place after a prefix gives the reference's rows, for parts that fill a row
    /// exactly, overrun it by one, a part longer than a row, and one part alone.
    #[test]
    fn wrapping_in_place_matches_the_row_by_row_wrap() {
        let exact = "x".repeat(BANNER_COLS - 4);
        let long = "y".repeat(BANNER_COLS + 5);
        let cases: [Vec<&str>; 5] = [
            vec![
                "lighting half",
                "shadow walk coarse",
                "history edits",
                "lamp on",
            ],
            vec![&exact, "ab", "c"],
            vec![&exact, "abc", "d"],
            vec!["a", &long, "b", "c"],
            vec!["alone"],
        ];
        for parts in cases {
            let mut out = "prefix\n".to_owned();
            let from = out.len();
            out.push_str(&parts.join(", "));
            wrap_from(&mut out, from);
            assert_eq!(out[from..], wrap(&parts).join("\n"), "{parts:?}");
            assert_eq!(&out[..from], "prefix\n");
        }
    }

    /// Each of `F3`, `F5` to `F8`, `M`, `Comma` and `Semicolon` changes the banner's text to the new mode; the banner is
    /// empty 3 s after the last change and shows again on the next one.
    #[test]
    fn the_banner_names_each_change_and_fades_after_three_seconds() {
        let mut m = Modes::new(
            vxe_render::Lighting::default(),
            vxe_render::LightingResolution::default(),
            0.0,
        );
        let start = m.banner(0.0).expect("the banner shows at the start");
        assert_eq!(
            start,
            "lighting half, store read face\nweights normal, shadow walk coarse\nhistory edits, lamp on\nlamp bounce off at 64 rays 1/8\nfilter on, ore pulse off\nexposure manual +0.00 EV"
        );
        assert!(start.lines().all(|l| l.len() <= BANNER_COLS));
        let mut t = 1000.0;
        let mut last = start;
        for (key, word) in [
            (ModeKey::ShadowWalk, "shadow walk full"),
            (ModeKey::History, "history gradient"),
            (ModeKey::Resolution, "lighting full"),
            (ModeKey::Lamp, "lamp off"),
            (ModeKey::LampBounce, "lamp bounce on"),
            (ModeKey::Pulse, "ore pulse on"),
            (ModeKey::StoreRead, "store read normal"),
            (ModeKey::LightWeights, "weights face"),
            (ModeKey::StoreRead, "store read face"),
            (ModeKey::LightWeights, "weights normal"),
        ] {
            m.press(key, t);
            let now = m.banner(t).expect("a change shows the banner");
            assert_ne!(now, last, "{key:?} changes the text");
            assert!(now.contains(word), "{key:?}: {now}");
            last = now;
            t += 500.0;
        }
        let changed = t - 500.0;
        assert!(m.banner(changed + BANNER_MS - 1.0).is_some());
        assert_eq!(
            m.banner(changed + BANNER_MS),
            None,
            "empty 3 s after the last change"
        );
        // `Q` from the default auto eye: auto symmetric, then manual holding the read-back's
        // exposure, then auto eye again, each with its EV.
        let mut q = Modes::new(
            vxe_render::Lighting {
                exposure: vxe_render::Exposure::Auto(vxe_render::AutoExposure::eye()),
                ..vxe_render::Lighting::default()
            },
            vxe_render::LightingResolution::default(),
            0.0,
        );
        q.set_readout(Some(vxe_render::ExposureReadout {
            exposure: 11.52,
            adapted_log2: -6.0,
            metered_log2: -6.0,
            frame: 9,
            held: false,
        }));
        assert!(
            q.banner(0.0)
                .unwrap()
                .contains("exposure auto eye +3.53 EV")
        );
        for (i, want) in [
            "exposure auto symmetric +3.53 EV",
            "exposure manual +3.53 EV",
            "exposure auto eye +3.53 EV",
        ]
        .into_iter()
        .enumerate()
        {
            let at = 100.0 * (i + 1) as f64;
            q.press(ModeKey::Exposure, at);
            let b = q.banner(at).expect("a change shows the banner");
            assert!(b.contains(want), "{b}");
            assert!(b.lines().all(|l| l.len() <= BANNER_COLS), "{b}");
        }
        assert_eq!(
            q.exposure_row().to_string(),
            "exposure auto eye +3.53 EV  scene -6.0 EV  settles +3.53 EV"
        );
        m.press(ModeKey::Lamp, changed + 10_000.0);
        let again = m
            .banner(changed + 10_000.0)
            .expect("the next change shows it again");
        assert!(again.contains("lamp on"), "{again}");
    }

    /// `N` names each preset in turn on the banner's first row, then the far normal.
    #[test]
    fn the_banner_names_each_look_preset() {
        use crate::LookPreset;
        let mut m = Modes::new(
            vxe_render::Lighting::default(),
            vxe_render::LightingResolution::default(),
            0.0,
        );
        let lighting = m.banner(0.0).expect("the banner shows at the start");
        let mut preset = LookPreset::FACE;
        let mut t = 5000.0;
        let mut names = Vec::new();
        for _ in 0..LookPreset::ALL.len() {
            preset = preset.next(true);
            m.show_look(&preset, [0.0; 2], t);
            let b = m.banner(t).expect("a press shows the banner");
            assert_eq!(
                b,
                format!("look {}, far normal off\n{lighting}", preset.name)
            );
            assert!(b.lines().count() <= BANNER_ROWS && b.lines().all(|l| l.len() <= BANNER_COLS));
            assert_eq!(m.banner(t + BANNER_MS), None);
            names.push(preset.name);
            t += BANNER_MS;
        }
        assert_eq!(names, ["field", "trilinear8", "gradient27", "face"]);
        assert_eq!(
            LookPreset::FACE.next(false).name,
            "trilinear8",
            "without the field's band slots N skips the field"
        );
        assert_eq!(LookPreset::GRADIENT27.next(false), LookPreset::FACE);
        let first = m.banner(t - BANNER_MS).unwrap();
        assert!(first.starts_with("look face, far normal off\n"), "{first}");
        m.show_look(&LookPreset::FACE, [2.0, 6.0], t);
        assert!(
            m.banner(t)
                .unwrap()
                .starts_with("look face, far normal 2..6\n"),
            "the knob's setting shows"
        );
        m.show_look(&LookPreset::GRADIENT27, [2.0, 6.0], t);
        assert!(
            m.banner(t)
                .unwrap()
                .starts_with("look gradient27, far normal off\n"),
            "a mode that does not read the far normal does not name it"
        );
    }

    /// `F9`'s view shows on the banner after the look's words, `view cracks`, from the press
    /// for `BANNER_MS`; the shaded view adds nothing.
    #[test]
    fn the_banner_names_the_view() {
        use vxe_render::ViewMode;
        let mut m = Modes::new(
            vxe_render::Lighting::default(),
            vxe_render::LightingResolution::default(),
            0.0,
        );
        let lighting = m.banner(0.0).expect("the banner shows at the start");
        m.show_look(&crate::LookPreset::FIELD, [0.0; 2], 0.0);
        m.show_view(ViewMode::Cracks, 5000.0);
        assert_eq!(
            m.banner(5000.0).as_deref(),
            Some(
                format!(
                    "{}\n{lighting}",
                    wrap(&["look field", "far normal off", "view cracks"]).join("\n")
                )
                .as_str()
            )
        );
        assert_eq!(m.banner(5000.0 + BANNER_MS), None);
        m.show_view(ViewMode::Shaded, 9000.0);
        assert_eq!(
            m.banner(9000.0).as_deref(),
            Some(format!("look field, far normal off\n{lighting}").as_str())
        );
    }
}
