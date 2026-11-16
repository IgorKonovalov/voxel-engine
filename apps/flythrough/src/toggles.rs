//! The fly-through's toggles: one table of key, name, HUD row, getter and flip over
//! [`hud::Modes`], which owns no GPU object. The run dispatches its keys through the table
//! (`run::keys`), the HUD's mode block shows each toggle's mode in the row the table names
//! ([`hud::Modes::write_rows`]), and a press prints that row.
//!
//! The keys that are actions with no mode are [`ACTIONS`]: binds, a label, a menu page, an
//! optional mode in words and the [`Act`] the run calls. Both tables are the menu's source
//! (`hud::menu::entries`): its pages, its palette, its Keys page and `flythrough --keys`.
//! `T` is the tour's resume in a path run and the audio's pulse outside one, so the pulse's
//! entry names no bind of its own.

use crate::cli::next_far_normal;
use crate::edits::{DIG_RANGE, DIG_STEP};
use crate::hud::menu::{AudioView, Group, RunView};
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
    /// No key: the toggle is the menu's only.
    Menu,
}

/// One toggle: what presses it, its name, the HUD row that shows its mode, the mode in words
/// as that row shows it, and the change a press makes, its banner shown from the time given.
pub struct Toggle {
    pub press: Press,
    pub name: &'static str,
    /// The menu's page the toggle is an entry of.
    pub group: Group,
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
            Press::Scroll | Press::Menu => false,
        }
    }
}

/// Flips `t` on `m` at `now` and gives the line a press prints: the toggle's HUD row with the
/// new mode. Its key's press and the menu's `Enter` both come here.
pub fn press(t: &Toggle, m: &mut Modes, now: f64) -> String {
    (t.flip)(m, now);
    m.row(t.row)
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
            group: Group::Lighting,
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

/// The budgets key `Backspace` steps through: the default slice, 150 ms a second and 300 ms a
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

/// `1`: drop a light at the looked-at surface (`run::keys`).
pub const KEY_DROP: KeyCode = KeyCode::Digit1;
/// `4`: remove the newest light.
pub const KEY_REMOVE: KeyCode = KeyCode::Digit4;
/// `6`: fire a light along the view ray.
pub const KEY_FIRE: KeyCode = KeyCode::Digit6;
/// `` ` `` (Backquote): place a walker on the looked-at surface (`crate::walker`).
pub const KEY_WALKER: KeyCode = KeyCode::Backquote;
/// `F12`: the walkers' eye lights on or off.
pub const KEY_EYE_LIGHTS: KeyCode = KeyCode::F12;
/// `F11`: the walkers walk or hold still (`crate::walker`).
pub const KEY_WALK: KeyCode = KeyCode::F11;
/// `Left` and `Right`: the viewer's clip before and after (`--model`, `crate::viewer`).
pub const KEY_CLIP_BACK: KeyCode = KeyCode::ArrowLeft;
pub const KEY_CLIP_ON: KeyCode = KeyCode::ArrowRight;
/// `Down`: the viewer's clip paused or playing.
pub const KEY_CLIP_PAUSE: KeyCode = KeyCode::ArrowDown;

/// What an action does: the run's method its entry and its binds call (`Run::act`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    /// Toggles walk mode (the body under the camera) or flight.
    Walk,
    /// Resumes a tour path from its point nearest the camera (`crate::path`).
    ResumePath,
    /// Moves the camera to the next castle shot.
    CastleShot,
    /// Drops a light at the looked-at surface.
    DropLight,
    /// Removes the newest light or walker.
    Remove,
    /// Fires a light along the view ray.
    FireLight,
    /// Places a walker (or `--model`'s model) at the looked-at surface.
    PlaceWalker,
    /// The walkers walk or hold still.
    WalkersWalk,
    /// The walkers' eye lights on or off.
    EyeLights,
    /// The viewer's clip before.
    ClipBack,
    /// The viewer's clip after.
    ClipOn,
    /// The viewer's clip paused or playing.
    ClipPause,
    /// Plays a knock at the dig point.
    Knock,
    /// Plays a looping pulse.
    Pulse,
    /// Plays a looping hum, the next note each time.
    Hum,
    /// The early reflections on or off.
    Echoes,
    /// The reverb tail on or off.
    Tail,
    /// The echoes' trim, 3 dB down or up.
    EchoTrim(i8),
    /// The tail's trim, 3 dB down or up.
    TailTrim(i8),
    /// The tail's pre-delay, 10 ms down or up.
    PreDelay(i8),
    /// The tail's lines: glide or handover.
    Glide,
    /// The direct path's propagation delay on or off.
    DirectDelay,
    /// Takes the frame readings over the next frames and prints their line.
    Readings,
    /// Runs the dig benchmark.
    Bench,
    /// Stamps the castle prefab at the looked-at surface.
    Stamp,
    /// Sends a crumble input to the collapse.
    Crumble,
    /// Prints the coarse grid's line.
    CoarsePrint,
}

/// A bind of an action: a key, or the pad button that does the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bind {
    Key(KeyCode),
    Pad(PadButton),
}

/// One action: its binds (none for a menu-only one), its label, its menu page, its mode in
/// words when it has one (read from the modes and the few run flags in [`RunView`]), and the
/// run's method it calls.
pub struct Action {
    pub binds: &'static [Bind],
    pub label: &'static str,
    pub group: Group,
    pub mode: Option<fn(&Modes, &RunView) -> String>,
    pub act: Act,
}

impl Action {
    /// Whether `input` presses one of this action's binds this frame.
    pub fn pressed(&self, input: &Input) -> bool {
        self.binds.iter().any(|b| match *b {
            Bind::Key(k) => input.pressed(k),
            Bind::Pad(p) => input.pad_pressed(p),
        })
    }
}

/// The audio's mode in words, or `no audio` in a run without it.
fn audio_words(v: &RunView, words: fn(&AudioView) -> String) -> String {
    v.audio
        .as_ref()
        .map_or_else(|| "no audio".to_owned(), words)
}

/// An action with no mode, bound to `binds`.
macro_rules! action {
    ($label:literal, $group:ident, $act:expr, [$($bind:expr),*]) => {
        Action {
            binds: &[$($bind),*],
            label: $label,
            group: Group::$group,
            mode: None,
            act: $act,
        }
    };
}

/// Every action of the fly-through, in its pages' order: walking, the paths and the castle
/// shot, the lights' drop, removal and shot, the walkers and the viewer's clips, the audio's
/// sounds and mix, the readings, and the debug actions. The run dispatches the binds
/// (`run::keys`); the menu calls the same [`Act`]. The movement keys, `Esc` and `F2` are the
/// fixed keys (`hud::menu::FIXED_KEYS`), not actions.
pub static ACTIONS: &[Action] = &[
    Action {
        binds: &[Bind::Key(KeyCode::KeyG), Bind::Pad(PadButton::North)],
        label: "walk",
        group: Group::Moving,
        mode: Some(|_, v| if v.walking { "walk" } else { "fly" }.to_owned()),
        act: Act::Walk,
    },
    action!(
        "resume the tour",
        Camera,
        Act::ResumePath,
        [Bind::Key(KeyCode::KeyT)]
    ),
    action!(
        "next castle shot",
        Camera,
        Act::CastleShot,
        [Bind::Key(KeyCode::KeyI)]
    ),
    action!(
        "drop a light",
        Lights,
        Act::DropLight,
        [Bind::Key(KEY_DROP)]
    ),
    action!(
        "remove the newest",
        Lights,
        Act::Remove,
        [Bind::Key(KEY_REMOVE)]
    ),
    action!(
        "fire a light",
        Lights,
        Act::FireLight,
        [Bind::Key(KEY_FIRE)]
    ),
    action!(
        "place a walker",
        Entities,
        Act::PlaceWalker,
        [Bind::Key(KEY_WALKER)]
    ),
    Action {
        binds: &[Bind::Key(KEY_WALK)],
        label: "walkers walk",
        group: Group::Entities,
        mode: Some(|m, _| format!("walkers walk {}", on(m.walkers.walking))),
        act: Act::WalkersWalk,
    },
    Action {
        binds: &[Bind::Key(KEY_EYE_LIGHTS)],
        label: "eye lights",
        group: Group::Entities,
        mode: Some(|m, _| format!("eye lights {}", on(!m.walkers.eye_lights_off))),
        act: Act::EyeLights,
    },
    action!(
        "clip before",
        Entities,
        Act::ClipBack,
        [Bind::Key(KEY_CLIP_BACK)]
    ),
    action!(
        "clip after",
        Entities,
        Act::ClipOn,
        [Bind::Key(KEY_CLIP_ON)]
    ),
    action!(
        "clip pause",
        Entities,
        Act::ClipPause,
        [Bind::Key(KEY_CLIP_PAUSE)]
    ),
    action!(
        "knock",
        Audio,
        Act::Knock,
        [Bind::Key(KeyCode::KeyH), Bind::Pad(PadButton::East)]
    ),
    action!("pulse", Audio, Act::Pulse, []),
    action!("hum", Audio, Act::Hum, [Bind::Key(KeyCode::KeyJ)]),
    Action {
        binds: &[Bind::Key(KeyCode::KeyE)],
        label: "echoes",
        group: Group::Audio,
        mode: Some(|_, v| audio_words(v, |a| format!("echoes {}", on(a.echoes)))),
        act: Act::Echoes,
    },
    Action {
        binds: &[Bind::Key(KeyCode::KeyY)],
        label: "tail",
        group: Group::Audio,
        mode: Some(|_, v| audio_words(v, |a| format!("tail {}", on(a.tail)))),
        act: Act::Tail,
    },
    Action {
        binds: &[Bind::Key(KeyCode::BracketLeft)],
        label: "echo trim down",
        group: Group::Audio,
        mode: Some(|_, v| audio_words(v, |a| format!("echo trim {:+.0} dB", a.echo_db))),
        act: Act::EchoTrim(-1),
    },
    Action {
        binds: &[Bind::Key(KeyCode::BracketRight)],
        label: "echo trim up",
        group: Group::Audio,
        mode: Some(|_, v| audio_words(v, |a| format!("echo trim {:+.0} dB", a.echo_db))),
        act: Act::EchoTrim(1),
    },
    Action {
        binds: &[Bind::Key(KeyCode::Minus)],
        label: "tail trim down",
        group: Group::Audio,
        mode: Some(|_, v| audio_words(v, |a| format!("tail trim {:+.0} dB", a.tail_db))),
        act: Act::TailTrim(-1),
    },
    Action {
        binds: &[Bind::Key(KeyCode::Equal)],
        label: "tail trim up",
        group: Group::Audio,
        mode: Some(|_, v| audio_words(v, |a| format!("tail trim {:+.0} dB", a.tail_db))),
        act: Act::TailTrim(1),
    },
    Action {
        binds: &[Bind::Key(KeyCode::Digit9)],
        label: "pre-delay down",
        group: Group::Audio,
        mode: Some(|_, v| audio_words(v, |a| format!("pre-delay {:.0} ms", a.predelay_ms))),
        act: Act::PreDelay(-1),
    },
    Action {
        binds: &[Bind::Key(KeyCode::Digit0)],
        label: "pre-delay up",
        group: Group::Audio,
        mode: Some(|_, v| audio_words(v, |a| format!("pre-delay {:.0} ms", a.predelay_ms))),
        act: Act::PreDelay(1),
    },
    Action {
        binds: &[Bind::Key(KeyCode::KeyO)],
        label: "tail lines",
        group: Group::Audio,
        mode: Some(|_, v| {
            audio_words(v, |a| {
                format!("tail lines {}", if a.glide { "glide" } else { "handover" })
            })
        }),
        act: Act::Glide,
    },
    Action {
        binds: &[Bind::Key(KeyCode::KeyU)],
        label: "direct delay",
        group: Group::Audio,
        mode: Some(|_, v| audio_words(v, |a| format!("direct delay {}", on(a.direct_delay)))),
        act: Act::DirectDelay,
    },
    action!(
        "frame readings",
        Readings,
        Act::Readings,
        [Bind::Key(KeyCode::Enter)]
    ),
    action!("bench", Debug, Act::Bench, [Bind::Key(KeyCode::KeyB)]),
    action!("stamp", Debug, Act::Stamp, [Bind::Key(KeyCode::KeyP)]),
    action!("crumble", Debug, Act::Crumble, [Bind::Key(KeyCode::KeyR)]),
    action!(
        "coarse print",
        Debug,
        Act::CoarsePrint,
        [Bind::Key(KeyCode::KeyK)]
    ),
];

/// The `lights` row in words.
fn lights_words(m: &Modes) -> String {
    m.row(HudRow::Lights)
}

/// The toggle table: every key of the fly-through that switches a mode.
pub static TOGGLES: &[Toggle] = &[
    lighting!(F3, "shadow walk", ShadowWalk, |m| format!(
        "shadow walk {}",
        m.lighting.shadow_walk.name()
    )),
    lighting!(F4, "filter iterations", Atrous, |m| {
        let mut out = String::new();
        m.write_filter_count(&mut out);
        out
    }),
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
    lighting!(Delete, "young filter", YoungFilter, |m| format!(
        "young {}",
        m.lighting.young_filter.name()
    )),
    lighting!(Insert, "entity light", EntityLight, |m| format!(
        "entity light {}",
        m.lighting.entity_light.name()
    )),
    Toggle {
        press: Press::Key(KeyCode::KeyN, None),
        name: "look",
        group: Group::Look,
        row: HudRow::Look,
        get: |m| format!("look {}", m.preset.name),
        flip: next_look,
    },
    Toggle {
        press: Press::Key(KeyCode::KeyV, None),
        name: "far normal",
        group: Group::Look,
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
        group: Group::Debug,
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
        group: Group::Upscaler,
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
        group: Group::Upscaler,
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
        group: Group::Upscaler,
        row: HudRow::Upscaler,
        get: |m| format!("pan {}", m.pan.as_ref().map_or("none", |p| on(p.on))),
        flip: |m, now| {
            if let Some(p) = m.pan.as_mut() {
                p.toggle(now);
            }
        },
    },
    Toggle {
        press: Press::Key(KeyCode::KeyF, None),
        name: "queue order",
        group: Group::Queue,
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
        press: Press::Menu,
        name: "queue angle halved",
        group: Group::Queue,
        row: HudRow::Queue,
        get: order_words,
        flip: |m, _| m.queue.angle *= 0.5,
    },
    Toggle {
        press: Press::Key(KeyCode::Slash, None),
        name: "queue angle doubled",
        group: Group::Queue,
        row: HudRow::Queue,
        get: order_words,
        flip: |m, _| {
            let a = m.queue.angle;
            m.queue.angle = if a > 0.0 { a * 2.0 } else { 0.25 };
        },
    },
    Toggle {
        press: Press::Key(KeyCode::Quote, None),
        name: "queue withdraw",
        group: Group::Queue,
        row: HudRow::Queue,
        get: |m| format!("withdraw {}", on(m.queue.withdraw)),
        flip: |m, _| m.queue.withdraw = !m.queue.withdraw,
    },
    Toggle {
        press: Press::Key(KeyCode::Backspace, None),
        name: "bake budget",
        group: Group::Queue,
        row: HudRow::Queue,
        get: |m| format!("budget {}", m.queue.budget),
        flip: |m, _| m.queue.budget = next_budget(m.queue.budget),
    },
    Toggle {
        press: Press::Key(KeyCode::Backslash, None),
        name: "full-first",
        group: Group::Queue,
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
        group: Group::Queue,
        row: HudRow::Queue,
        get: |m| format!("missing light {}", m.queue.missing_light.name()),
        flip: |m, _| m.queue.missing_light = next_missing_light(m.queue.missing_light),
    },
    Toggle {
        press: Press::Key(KeyCode::Digit8, None),
        name: "reach",
        group: Group::Queue,
        row: HudRow::Queue,
        get: |m| format!("reach {}", m.queue.reach.name()),
        flip: |m, _| m.queue.reach = next_reach(m.queue.reach),
    },
    Toggle {
        press: Press::Key(KeyCode::Digit2, None),
        name: "carry",
        group: Group::Lights,
        row: HudRow::Lights,
        get: lights_words,
        flip: |m, now| {
            m.lights.toggle_carry();
            let words = format!("carry {}", on(m.lights.carry));
            m.show_lights(words, now);
        },
    },
    Toggle {
        press: Press::Key(KeyCode::Digit3, None),
        name: "colour",
        group: Group::Lights,
        row: HudRow::Lights,
        get: lights_words,
        flip: |m, now| {
            let name = m.lights.cycle_colour();
            m.show_lights(format!("colour {name}"), now);
        },
    },
    Toggle {
        press: Press::Key(KeyCode::Digit5, None),
        name: "bounce",
        group: Group::Lights,
        row: HudRow::Lights,
        get: lights_words,
        flip: |m, now| {
            let words = match m.lights.toggle_bounce() {
                Some(b) => format!("bounce {}", on(b)),
                None => "bounce: no light".to_string(),
            };
            m.show_lights(words, now);
        },
    },
    Toggle {
        press: Press::Key(KeyCode::KeyX, Some(PadButton::West)),
        name: "cutaway",
        group: Group::Debug,
        row: HudRow::Controls,
        get: |m| format!("cutaway {}", m.cutaway.name()),
        flip: |m, _| m.cutaway = m.cutaway.next(),
    },
    Toggle {
        press: Press::Key(KeyCode::KeyL, None),
        name: "shadow ray",
        group: Group::Debug,
        row: HudRow::Controls,
        get: |m| format!("shadow ray {}", on(m.shadow_ray)),
        flip: |m, _| m.shadow_ray = !m.shadow_ray,
    },
    Toggle {
        press: Press::Scroll,
        name: "dig radius",
        group: Group::Moving,
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
