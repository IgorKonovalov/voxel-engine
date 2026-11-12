//! The fly-through's toggles: one table of key, name, HUD row, getter and flip over
//! [`hud::Modes`], which owns no GPU object. The run dispatches its keys through the table
//! (`run::keys`), the HUD's mode block shows each toggle's mode in the row the table names
//! ([`hud::Modes::write_rows`]), and a press prints that row.
//!
//! The keys that are actions with no mode are not rows: the dig and the fill (the mouse
//! buttons, `R2` and `L2`), `B` (the bench), `P` (a stamp), `G` (walk mode, which needs the
//! world for its body; the HUD's position row names `fly` or `walk`), `K` (prints the coarse
//! grid), `R` (a crumble input), `I` (the next castle shot, a camera move), `F1` (the frame
//! readings), `F2` (the stats HUD), `T` (resumes a path) and the audio's keys (`H`, `T`, `J`
//! play sounds; `E`, `Y`, `O`, `U` and the trims change the mix and print its line, with no
//! HUD row while the audio has none).

use crate::cli::next_far_normal;
use crate::edits::{DIG_RANGE, DIG_STEP};
use crate::hud::{self, HudRow, Modes};
use vxe_app::{Input, KeyCode, PadButton};

/// What presses a toggle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    /// A key, and the pad button that does the same, if any.
    Key(KeyCode, Option<PadButton>),
    /// A scroll line up (or the pad's D-pad up); down steps back. The run banks fractional
    /// lines (`edits::scroll_dig_radius`); [`Toggle::flip`] is one whole line up.
    Scroll,
}

/// One toggle: what presses it, its name, the HUD row that shows its mode, the mode in words
/// as that row shows it, and the change a press makes, its banner shown from the time given.
pub struct Toggle {
    pub press: Press,
    pub name: &'static str,
    pub row: HudRow,
    pub get: fn(&Modes) -> String,
    pub flip: fn(&mut Modes, f64),
}

impl Toggle {
    /// Whether `input` presses this toggle's key or pad button this frame (a scroll row is
    /// stepped by the run's scroll banking, not here).
    pub fn pressed(&self, input: &Input) -> bool {
        match self.press {
            Press::Key(key, pad) => input.pressed(key) || pad.is_some_and(|b| input.pad_pressed(b)),
            Press::Scroll => false,
        }
    }
}

fn on(b: bool) -> &'static str {
    if b { "on" } else { "off" }
}

/// A lighting toggle: `hud::Modes::press` of `key`.
macro_rules! lighting {
    ($key:ident, $name:literal, $mode:ident, $get:expr) => {
        Toggle {
            press: Press::Key(KeyCode::$key, None),
            name: $name,
            row: HudRow::Lighting,
            get: $get,
            flip: |m, now| m.press(hud::ModeKey::$mode, now),
        }
    };
}

/// The missing-light mode after `m` in [`vxe_render::MissingLight::ALL`] (key `7`).
pub(crate) fn next_missing_light(m: vxe_render::MissingLight) -> vxe_render::MissingLight {
    use vxe_render::MissingLight;
    let i = MissingLight::ALL.iter().position(|&x| x == m).unwrap_or(0);
    MissingLight::ALL[(i + 1) % MissingLight::ALL.len()]
}

/// The reach mode after `m` in [`vxe_render::ReachMode::ALL`] (key `8`).
pub(crate) fn next_reach(m: vxe_render::ReachMode) -> vxe_render::ReachMode {
    use vxe_render::ReachMode;
    let i = ReachMode::ALL.iter().position(|&x| x == m).unwrap_or(0);
    ReachMode::ALL[(i + 1) % ReachMode::ALL.len()]
}

/// The budgets key `5` steps through: the default slice, 150 ms a second and 300 ms a
/// second, each capped at a quarter of the frame.
pub(crate) const BUDGET_CYCLE: [vxe_render::Budget; 3] = [
    vxe_render::Budget::PerFrame {
        ms: vxe_render::DEFAULT_BAKE_SLICE_MS,
    },
    vxe_render::Budget::PerSecond {
        ms_per_s: 150.0,
        cap: 0.25,
    },
    vxe_render::Budget::PerSecond {
        ms_per_s: 300.0,
        cap: 0.25,
    },
];

/// The budget after `b` in [`BUDGET_CYCLE`]; one not in it goes to the first.
pub(crate) fn next_budget(b: vxe_render::Budget) -> vxe_render::Budget {
    let i = BUDGET_CYCLE.iter().position(|&x| x == b);
    BUDGET_CYCLE[i.map_or(0, |i| (i + 1) % BUDGET_CYCLE.len())]
}

/// `N`: the next preset, its normal mode and tuning in the look. The far normal stays, so
/// returning to face mode uses it again.
fn next_look(m: &mut Modes, now: f64) {
    let next = m.preset.next(m.field_surface);
    m.look.normal_mode = next.normal_mode;
    m.look.tuning = next.tuning;
    m.preset = next;
    m.show_look(&next, m.look.far_normal, now);
}

/// The queue's order in words.
fn order_words(m: &Modes) -> String {
    m.queue.order().words()
}

/// The toggle table: every key of the fly-through that switches a mode.
pub static TOGGLES: &[Toggle] = &[
    lighting!(F3, "shadow walk", ShadowWalk, |m| format!(
        "shadow walk {}",
        m.lighting.shadow_walk.name()
    )),
    lighting!(F5, "history", History, |m| format!(
        "history {}",
        m.lighting.history.name()
    )),
    lighting!(F6, "lighting resolution", Resolution, |m| format!(
        "lighting {}",
        m.resolution.name()
    )),
    lighting!(F7, "lamp", Lamp, |m| format!(
        "lamp {}",
        on(m.lighting.lamp)
    )),
    lighting!(F8, "lamp bounce", LampBounce, |m| format!(
        "lamp bounce {}",
        on(m.lighting.lamp_bounce)
    )),
    lighting!(KeyM, "ore pulse", Pulse, |m| format!(
        "ore pulse {}",
        on(m.pulse)
    )),
    lighting!(KeyQ, "exposure", Exposure, |m| format!(
        "exposure {}",
        m.exposure_mode()
    )),
    lighting!(Comma, "store read", StoreRead, |m| format!(
        "store read {}",
        m.lighting.store_read.name()
    )),
    lighting!(Semicolon, "light weights", LightWeights, |m| format!(
        "weights {}",
        m.lighting.light_weights.name()
    )),
    Toggle {
        press: Press::Key(KeyCode::KeyN, None),
        name: "look",
        row: HudRow::Look,
        get: |m| format!("look {}", m.preset.name),
        flip: next_look,
    },
    Toggle {
        press: Press::Key(KeyCode::KeyV, None),
        name: "far normal",
        row: HudRow::Look,
        get: |m| crate::LookPreset::far_normal_words(m.look.normal_mode, m.look.far_normal),
        flip: |m, now| {
            m.look.far_normal = next_far_normal(m.look.far_normal);
            m.show_look(&m.preset.clone(), m.look.far_normal, now);
        },
    },
    Toggle {
        press: Press::Key(KeyCode::F9, None),
        name: "view",
        row: HudRow::Look,
        get: |m| format!("view {}", m.look.view.name()),
        flip: |m, now| {
            m.look.view = m.look.view.next();
            m.show_view(m.look.view, now);
        },
    },
    Toggle {
        press: Press::Key(KeyCode::F10, None),
        name: "upscaler",
        row: HudRow::Upscaler,
        get: |m| format!("upscaler {}", m.upscaler.mode.name()),
        flip: |m, now| {
            let mode = m.upscaler.mode.next();
            let scale = if mode == hud::UpscalerMode::Off {
                "1"
            } else {
                m.scale
            };
            m.upscaler.change(mode, scale, now);
        },
    },
    Toggle {
        press: Press::Key(KeyCode::KeyZ, None),
        name: "sharpen",
        row: HudRow::Upscaler,
        get: |m| format!("sharpen {:.2}", m.upscaler.sharpen),
        flip: |m, now| {
            let s = hud::next_sharpen(m.upscaler.sharpen);
            m.upscaler.set_sharpen(s, now);
        },
    },
    Toggle {
        press: Press::Key(KeyCode::Period, None),
        name: "pan",
        row: HudRow::Upscaler,
        get: |m| format!("pan {}", m.pan.as_ref().map_or("none", |p| on(p.on))),
        flip: |m, now| {
            if let Some(p) = m.pan.as_mut() {
                p.toggle(now);
            }
        },
    },
    Toggle {
        press: Press::Key(KeyCode::Digit1, None),
        name: "queue order",
        row: HudRow::Queue,
        get: order_words,
        flip: |m, _| {
            let q = &mut m.queue;
            (q.distance, q.tier) = match (q.distance, q.tier) {
                (false, false) => (false, true),
                (false, true) => (true, false),
                (true, _) => (false, false),
            };
        },
    },
    Toggle {
        press: Press::Key(KeyCode::Digit2, None),
        name: "queue angle halved",
        row: HudRow::Queue,
        get: order_words,
        flip: |m, _| m.queue.angle *= 0.5,
    },
    Toggle {
        press: Press::Key(KeyCode::Digit3, None),
        name: "queue angle doubled",
        row: HudRow::Queue,
        get: order_words,
        flip: |m, _| {
            let a = m.queue.angle;
            m.queue.angle = if a > 0.0 { a * 2.0 } else { 0.25 };
        },
    },
    Toggle {
        press: Press::Key(KeyCode::Digit4, None),
        name: "queue withdraw",
        row: HudRow::Queue,
        get: |m| format!("withdraw {}", on(m.queue.withdraw)),
        flip: |m, _| m.queue.withdraw = !m.queue.withdraw,
    },
    Toggle {
        press: Press::Key(KeyCode::Digit5, None),
        name: "bake budget",
        row: HudRow::Queue,
        get: |m| format!("budget {}", m.queue.budget),
        flip: |m, _| m.queue.budget = next_budget(m.queue.budget),
    },
    Toggle {
        press: Press::Key(KeyCode::Digit6, None),
        name: "full-first",
        row: HudRow::Queue,
        get: |m| match m.queue.full_first {
            Some(r) => format!("full-first {r}"),
            None => "full-first off".to_owned(),
        },
        flip: |m, _| {
            m.queue.full_first = match m.queue.full_first {
                Some(_) => None,
                None => Some(vxe_render::DEFAULT_NEAR),
            };
        },
    },
    Toggle {
        press: Press::Key(KeyCode::Digit7, None),
        name: "missing light",
        row: HudRow::Queue,
        get: |m| format!("missing light {}", m.queue.missing_light.name()),
        flip: |m, _| m.queue.missing_light = next_missing_light(m.queue.missing_light),
    },
    Toggle {
        press: Press::Key(KeyCode::Digit8, None),
        name: "reach",
        row: HudRow::Queue,
        get: |m| format!("reach {}", m.queue.reach.name()),
        flip: |m, _| m.queue.reach = next_reach(m.queue.reach),
    },
    Toggle {
        press: Press::Key(KeyCode::KeyX, Some(PadButton::West)),
        name: "cutaway",
        row: HudRow::Controls,
        get: |m| format!("cutaway {}", m.cutaway.name()),
        flip: |m, _| m.cutaway = m.cutaway.next(),
    },
    Toggle {
        press: Press::Key(KeyCode::KeyL, None),
        name: "shadow ray",
        row: HudRow::Controls,
        get: |m| format!("shadow ray {}", on(m.shadow_ray)),
        flip: |m, _| m.shadow_ray = !m.shadow_ray,
    },
    Toggle {
        press: Press::Scroll,
        name: "dig radius",
        row: HudRow::Controls,
        get: |m| {
            format!(
                "dig radius {:.1} m",
                m.dig_radius as f64 / (10 * vxe_world::FIXED_ONE) as f64
            )
        },
        flip: |m, _| m.dig_radius = (m.dig_radius + DIG_STEP).min(DIG_RANGE.1),
    },
];
