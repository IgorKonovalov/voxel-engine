//! The fly-through's toggles: one table of key, name, HUD row, getter and flip over
//! [`hud::Modes`], which owns no GPU object. The run dispatches its keys through the table
//! (`run::keys`), the HUD's mode block shows each toggle's mode in the row the table names
//! ([`hud::Modes::write_rows`]), and a press prints that row.
//!
//! The keys that are actions with no mode are [`ACTIONS`]: binds, a label, a menu page, an
//! optional mode in words and the [`Act`] the run calls. Both tables are the menu's source
//! (`hud::menu::entries`): its pages, its palette, its Keys page and `flythrough --keys`.
//! An entry with no key (`Press::Menu`, an action with no binds) is the menu's only; the keys
//! that stay are the shortcuts of ADR-0061's key table.

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
    /// What the toggle needs of the run to act; `None` for always.
    pub needs: Option<Needs>,
    /// A number's step and range: `Enter` edits it with the arrows instead of `flip`
    /// (`hud::menu`); `None` for a switch or a cycle, which `Enter` flips.
    pub number: Option<Number>,
}

/// A number an entry edits: one step up (`1`) or down (`-1`), held in its range, and the
/// range in words as the edited row shows it after the value.
#[derive(Clone, Copy)]
pub struct Number {
    pub step: fn(&mut Modes, i32),
    pub range: fn(&Modes, &RunView) -> String,
}

/// What an entry needs of the run to act, and the flag that gives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Needs {
    /// `--audio`: the output, the mixer and its trims.
    Audio,
    /// `--pan DEG`: the pan to switch.
    Pan,
    /// `--path NAME`: a camera path to resume.
    Path,
    /// `--castle`: the castle's shots.
    Castle,
    /// `--model PATH`: the viewer's clips.
    Model,
    /// A placed entity: its clip, cycled or paused.
    Placed,
    /// A bounded world (`--level`): walking, which a streamed world refuses.
    Bounded,
    /// A bounded world (`--level`): the coarse grid and the collapse it feeds.
    Coarse,
    /// A sky over the world (`--sky on`, the outdoor source's default): the clock.
    Sky,
    /// The outdoor hills (`--source outdoor`): their trees.
    Outdoor,
}

impl Needs {
    /// `Ok` when the run at `m` and `v` has what is needed, else the reason in words.
    pub fn check(self, m: &Modes, v: &RunView) -> Result<(), &'static str> {
        let (ok, why) = match self {
            Self::Audio => (v.audio.is_some(), "needs --audio"),
            Self::Pan => (m.pan.is_some(), "needs --pan"),
            Self::Path => (v.path, "needs --path"),
            Self::Castle => (v.castle, "needs --castle"),
            Self::Model => (v.model, "needs --model"),
            Self::Placed => (v.placed, "needs a placed entity"),
            Self::Bounded => (!v.streaming, "needs --level"),
            Self::Coarse => (v.coarse, "needs --level"),
            Self::Sky => (m.sky, "needs --sky on"),
            Self::Outdoor => (
                m.source.is_some_and(|s| s.starts_with("outdoor")),
                "needs --source outdoor",
            ),
        };
        if ok { Ok(()) } else { Err(why) }
    }
}

impl Toggle {
    /// Whether `input` presses this toggle's key or pad button this frame.
    pub fn pressed(&self, input: &Input) -> bool {
        match self.press {
            Press::Key(key, pad) => input.pressed(key) || pad.is_some_and(|b| input.pad_pressed(b)),
            Press::Menu => false,
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

/// A lighting toggle: `hud::Modes::press` of its mode, bound to `key` or (without one) the
/// menu's only.
macro_rules! lighting {
    ($key:ident, $name:literal, $mode:ident, $get:expr) => {
        lighting!(@ Press::Key(KeyCode::$key, None), $name, $mode, $get)
    };
    ($name:literal, $mode:ident, $get:expr) => {
        lighting!(@ Press::Menu, $name, $mode, $get)
    };
    (@ $press:expr, $name:literal, $mode:ident, $get:expr) => {
        Toggle {
            press: $press,
            name: $name,
            group: Group::Lighting,
            row: HudRow::Lighting,
            get: $get,
            flip: |m, now| m.press(hud::ModeKey::$mode, now),
            needs: None,
            number: None,
        }
    };
}

/// The missing-light mode after `m` in [`vxe_render::MissingLight::ALL`].
pub(crate) fn next_missing_light(m: vxe_render::MissingLight) -> vxe_render::MissingLight {
    use vxe_render::MissingLight;
    let i = MissingLight::ALL.iter().position(|&x| x == m).unwrap_or(0);
    MissingLight::ALL[(i + 1) % MissingLight::ALL.len()]
}

/// The reach mode after `m` in [`vxe_render::ReachMode::ALL`].
pub(crate) fn next_reach(m: vxe_render::ReachMode) -> vxe_render::ReachMode {
    use vxe_render::ReachMode;
    let i = ReachMode::ALL.iter().position(|&x| x == m).unwrap_or(0);
    ReachMode::ALL[(i + 1) % ReachMode::ALL.len()]
}

/// The budgets the menu's `bake budget` steps through: the default slice, 150 ms a second and 300 ms a
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

/// The menu's `look`: the next preset, its normal mode and tuning in the look. The far normal stays, so
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
/// `` ` `` (Backquote): the chooser, the entities page; on it, `Enter`'s hand-over to
/// placement mode (`crate::entities::picker`).
pub const KEY_WALKER: KeyCode = KeyCode::Backquote;
/// `R`: turn the ghost one step in placement mode (`crate::entities::belt`).
pub const KEY_TURN: KeyCode = KeyCode::KeyR;
/// The belt's slots' keys, `1`..`9`: while the belt is shown (placement mode, and the
/// chooser over it) each selects its slot, `4` included, and the lights' drop, removal and
/// shot rest; out of the mode `1`, `4` and `6` keep those binds and no digit enters the mode.
/// No action binds them: `hud::menu::bound_keys` counts them as read.
pub const BELT_KEYS: [KeyCode; 9] = [
    KEY_DROP,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KEY_REMOVE,
    KeyCode::Digit5,
    KEY_FIRE,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
];
/// `F4`: the next render profile (`run::profile`).
pub const KEY_PROFILE: KeyCode = KeyCode::F4;
/// `Delete`: remove the placed entity under the crosshair.
pub const KEY_REMOVE_AIMED: KeyCode = KeyCode::Delete;
/// `Left` and `Right`: the aimed (else the newest) placed entity's clip before and after
/// (`crate::entities`).
pub const KEY_CLIP_BACK: KeyCode = KeyCode::ArrowLeft;
pub const KEY_CLIP_ON: KeyCode = KeyCode::ArrowRight;
/// `Down`: that entity's clip paused or playing.
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
    /// Opens the chooser: the library's entries, each into a belt slot.
    Picker,
    /// Opens the structures' chooser: the structures library's entries, each into a slot.
    StructurePicker,
    /// The belt's selection one slot on (`1`) or back (`-1`), entering the mode.
    BeltStep(i8),
    /// Turns the ghost one step in placement mode.
    Turn,
    /// Removes the placed entity under the crosshair.
    RemoveAimed,
    /// The wander on or off: the bodies wander, the built-in walkers walk their circle.
    WalkersWalk,
    /// The walkers' eye lights on or off.
    EyeLights,
    /// The aimed (else the newest) placed entity's clip before.
    ClipBack,
    /// Its clip after.
    ClipOn,
    /// Its clip paused or playing.
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
    /// The echoes' trim, 3 dB down (`-1`) or up (`1`): a number the menu edits.
    EchoTrim(i8),
    /// The tail's trim, 3 dB down or up: a number.
    TailTrim(i8),
    /// The tail's pre-delay, 10 ms down or up: a number.
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
    /// The next render profile (ADR-0055).
    Profile,
    /// The next world source, rebuilt in process (`run::switch`).
    NextSource,
    /// The outdoor hills' next tree density, rebuilt in process as the source switch is.
    Trees,
    /// The upscaler's next internal scale (1.5, 1, 2), applied from the next frame
    /// (`Run::cycle_scale`).
    RenderScale,
    /// FSR 2's next jitter phase count (its own, 8, 32), applied from the next frame
    /// (`Run::cycle_jitter_phases`).
    JitterPhases,
}

impl Act {
    /// The step of a number's act in direction `sign` (`1` up, `-1` down); any other act as
    /// it is.
    pub fn signed(self, sign: i32) -> Self {
        let d = if sign < 0 { -1 } else { 1 };
        match self {
            Self::EchoTrim(_) => Self::EchoTrim(d),
            Self::TailTrim(_) => Self::TailTrim(d),
            Self::PreDelay(_) => Self::PreDelay(d),
            a => a,
        }
    }
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
    /// What the action needs of the run to act; `None` for always.
    pub needs: Option<Needs>,
    /// A number's range in words: `Enter` edits it with the arrows, each step the act
    /// [`Act::signed`]; `None` for an action `Enter` runs.
    pub range: Option<fn(&Modes, &RunView) -> String>,
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
        action!($label, $group, $act, [$($bind),*], None)
    };
    ($label:literal, $group:ident, $act:expr, [$($bind:expr),*], $needs:expr) => {
        Action {
            binds: &[$($bind),*],
            label: $label,
            group: Group::$group,
            mode: None,
            act: $act,
            needs: $needs,
            range: None,
        }
    };
}

/// Every action of the fly-through, in its pages' order: walking, the paths and the castle
/// shot, the lights' drop, removal and shot, the walkers and the viewer's clips, the audio's
/// sounds and mix, the readings, and the debug actions. The run dispatches the binds
/// (`run::keys`); the menu calls the same [`Act`]. The movement keys, `Tab`, `F1`, `F2` and
/// `Esc` are the fixed keys (`hud::menu::FIXED_KEYS`), not actions.
pub static ACTIONS: &[Action] = &[
    Action {
        binds: &[Bind::Key(KeyCode::KeyG), Bind::Pad(PadButton::North)],
        label: "walk",
        group: Group::Moving,
        mode: Some(|_, v| if v.walking { "walk" } else { "fly" }.to_owned()),
        act: Act::Walk,
        needs: Some(Needs::Bounded),
        range: None,
    },
    action!(
        "resume the tour",
        Camera,
        Act::ResumePath,
        [Bind::Key(KeyCode::KeyT)],
        Some(Needs::Path)
    ),
    action!(
        "next castle shot",
        Camera,
        Act::CastleShot,
        [],
        Some(Needs::Castle)
    ),
    Action {
        binds: &[],
        label: "source",
        group: Group::World,
        mode: Some(|m, _| {
            let s = m
                .source
                .map_or("none", |s| s.split(',').next().unwrap_or(s));
            format!("source {s}")
        }),
        act: Act::NextSource,
        needs: None,
        range: None,
    },
    Action {
        binds: &[],
        label: "trees",
        group: Group::World,
        mode: Some(|m, _| {
            m.source
                .and_then(|s| s.split_once(", "))
                .map_or("trees none", |(_, t)| t)
                .to_owned()
        }),
        act: Act::Trees,
        needs: Some(Needs::Outdoor),
        range: None,
    },
    Action {
        binds: &[],
        label: "render scale",
        group: Group::Debug,
        mode: Some(|m, _| format!("scale {}", m.scale)),
        act: Act::RenderScale,
        needs: None,
        range: None,
    },
    Action {
        binds: &[],
        label: "jitter phases",
        group: Group::Debug,
        mode: Some(|m, _| {
            let phases = m
                .upscaler
                .fsr2_knobs
                .split(", ")
                .find_map(|w| w.strip_prefix("phases "));
            format!("jitter phases {}", phases.unwrap_or("fsr2's"))
        }),
        act: Act::JitterPhases,
        needs: None,
        range: None,
    },
    Action {
        binds: &[Bind::Key(KEY_PROFILE)],
        label: "profile",
        group: Group::Upscaler,
        mode: Some(|m, _| m.profile.clone()),
        act: Act::Profile,
        needs: None,
        range: None,
    },
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
        "choose an entity",
        Entities,
        Act::Picker,
        [Bind::Key(KEY_WALKER)]
    ),
    action!(
        "belt next",
        Entities,
        Act::BeltStep(1),
        [Bind::Pad(PadButton::DpadRight)]
    ),
    action!(
        "belt before",
        Entities,
        Act::BeltStep(-1),
        [Bind::Pad(PadButton::DpadLeft)]
    ),
    action!("turn the ghost", Entities, Act::Turn, [Bind::Key(KEY_TURN)]),
    action!("choose a structure", Structures, Act::StructurePicker, []),
    action!(
        "remove aimed entity",
        Entities,
        Act::RemoveAimed,
        [Bind::Key(KEY_REMOVE_AIMED)],
        Some(Needs::Placed)
    ),
    Action {
        binds: &[],
        label: "wander",
        group: Group::Entities,
        mode: Some(|m, _| format!("wander {}", on(m.walkers.walking))),
        act: Act::WalkersWalk,
        needs: None,
        range: None,
    },
    Action {
        binds: &[],
        label: "eye lights",
        group: Group::Entities,
        mode: Some(|m, _| format!("eye lights {}", on(!m.walkers.eye_lights_off))),
        act: Act::EyeLights,
        needs: None,
        range: None,
    },
    action!(
        "clip before",
        Entities,
        Act::ClipBack,
        [Bind::Key(KEY_CLIP_BACK)],
        Some(Needs::Placed)
    ),
    action!(
        "clip after",
        Entities,
        Act::ClipOn,
        [Bind::Key(KEY_CLIP_ON)],
        Some(Needs::Placed)
    ),
    action!(
        "clip pause",
        Entities,
        Act::ClipPause,
        [Bind::Key(KEY_CLIP_PAUSE)],
        Some(Needs::Placed)
    ),
    action!(
        "knock",
        Audio,
        Act::Knock,
        [Bind::Pad(PadButton::East)],
        Some(Needs::Audio)
    ),
    action!("pulse", Audio, Act::Pulse, [], Some(Needs::Audio)),
    action!("hum", Audio, Act::Hum, [], Some(Needs::Audio)),
    Action {
        binds: &[],
        label: "echoes",
        group: Group::Audio,
        mode: Some(|_, v| audio_words(v, |a| format!("echoes {}", on(a.echoes)))),
        act: Act::Echoes,
        needs: Some(Needs::Audio),
        range: None,
    },
    Action {
        binds: &[],
        label: "tail",
        group: Group::Audio,
        mode: Some(|_, v| audio_words(v, |a| format!("tail {}", on(a.tail)))),
        act: Act::Tail,
        needs: Some(Needs::Audio),
        range: None,
    },
    Action {
        binds: &[],
        label: "echo trim",
        group: Group::Audio,
        mode: Some(|_, v| audio_words(v, |a| format!("echo trim {:+.0} dB", a.echo_db))),
        act: Act::EchoTrim(1),
        needs: Some(Needs::Audio),
        range: Some(|_, v| audio_words(v, |a| db_range(a.echo_range))),
    },
    Action {
        binds: &[],
        label: "tail trim",
        group: Group::Audio,
        mode: Some(|_, v| audio_words(v, |a| format!("tail trim {:+.0} dB", a.tail_db))),
        act: Act::TailTrim(1),
        needs: Some(Needs::Audio),
        range: Some(|_, v| audio_words(v, |a| db_range(a.tail_range))),
    },
    Action {
        binds: &[],
        label: "pre-delay",
        group: Group::Audio,
        mode: Some(|_, v| audio_words(v, |a| format!("pre-delay {:.0} ms", a.predelay_ms))),
        act: Act::PreDelay(1),
        needs: Some(Needs::Audio),
        range: Some(|_, _| {
            let r = crate::audio::Mix::PREDELAY_MS;
            format!("{:.0}..{:.0} ms", r.start(), r.end())
        }),
    },
    Action {
        binds: &[],
        label: "tail lines",
        group: Group::Audio,
        mode: Some(|_, v| {
            audio_words(v, |a| {
                format!("tail lines {}", if a.glide { "glide" } else { "handover" })
            })
        }),
        act: Act::Glide,
        needs: Some(Needs::Audio),
        range: None,
    },
    Action {
        binds: &[],
        label: "direct delay",
        group: Group::Audio,
        mode: Some(|_, v| audio_words(v, |a| format!("direct delay {}", on(a.direct_delay)))),
        act: Act::DirectDelay,
        needs: Some(Needs::Audio),
        range: None,
    },
    action!(
        "frame readings",
        Readings,
        Act::Readings,
        [Bind::Key(KeyCode::Enter)]
    ),
    action!("bench", Debug, Act::Bench, []),
    action!("stamp", Debug, Act::Stamp, []),
    action!("crumble", Debug, Act::Crumble, [], Some(Needs::Coarse)),
    action!(
        "coarse print",
        Debug,
        Act::CoarsePrint,
        [],
        Some(Needs::Coarse)
    ),
];

/// A trim's range in words, `-24..+12 dB`.
fn db_range((lo, hi): (f32, f32)) -> String {
    format!("{lo:+.0}..{hi:+.0} dB")
}

/// The `lights` row in words.
fn lights_words(m: &Modes) -> String {
    m.row(HudRow::Lights)
}

/// The largest angle weight `queue angle` steps up to.
pub const QUEUE_ANGLE_MAX: f32 = 8.0;

/// The queue's angle weight one step up (doubled, from 0 to 1/4, at most
/// [`QUEUE_ANGLE_MAX`]) or down (halved, to 0 under 1/64).
fn step_queue_angle(m: &mut Modes, d: i32) {
    let a = m.queue.angle;
    m.queue.angle = if d > 0 {
        if a > 0.0 {
            (a * 2.0).min(QUEUE_ANGLE_MAX)
        } else {
            0.25
        }
    } else if a * 0.5 < 1.0 / 64.0 {
        0.0
    } else {
        a * 0.5
    };
}

/// A diagnosis switch of the debug page (menu only, free when off): its mode in words on the
/// lighting row and in the banner, which a flip shows again from `now`.
macro_rules! diag {
    ($name:literal, $get:expr, $set:expr) => {
        Toggle {
            press: Press::Menu,
            name: $name,
            group: Group::Debug,
            row: HudRow::Lighting,
            get: $get,
            flip: |m, now| {
                let set: fn(&mut Modes) = $set;
                set(m);
                m.banner_at = now;
            },
            needs: None,
            number: None,
        }
    };
}

/// The toggle table: every key of the fly-through that switches a mode.
pub static TOGGLES: &[Toggle] = &[
    lighting!("shadow walk", ShadowWalk, |m| format!(
        "shadow walk {}",
        m.lighting.shadow_walk.name()
    )),
    lighting!("filter iterations", Atrous, |m| {
        let mut out = String::new();
        m.write_filter_count(&mut out);
        out
    }),
    lighting!("history", History, |m| format!(
        "history {}",
        m.lighting.history.name()
    )),
    lighting!("lighting resolution", Resolution, |m| format!(
        "lighting {}",
        m.resolution.name()
    )),
    lighting!(F7, "lamp", Lamp, |m| format!(
        "lamp {}",
        on(m.lighting.lamp)
    )),
    lighting!("lamp bounce", LampBounce, |m| format!(
        "lamp bounce {}",
        on(m.lighting.lamp_bounce)
    )),
    lighting!("ore pulse", Pulse, |m| format!("ore pulse {}", on(m.pulse))),
    lighting!("exposure", Exposure, |m| format!(
        "exposure {}",
        m.exposure_mode()
    )),
    lighting!("store read", StoreRead, |m| format!(
        "store read {}",
        m.lighting.store_read.name()
    )),
    lighting!("key far", KeyFar, |m| format!(
        "key far {}",
        m.lighting.key_far.name()
    )),
    lighting!("key far mix", KeyFarMix, |m| format!(
        "key far mix {}",
        m.lighting.key_far_mix.name()
    )),
    lighting!("light weights", LightWeights, |m| format!(
        "weights {}",
        m.lighting.light_weights.name()
    )),
    lighting!("young filter", YoungFilter, |m| format!(
        "young {}",
        m.lighting.young_filter.name()
    )),
    lighting!("entity light", EntityLight, |m| format!(
        "entity light {}",
        m.lighting.entity_light.name()
    )),
    Toggle {
        press: Press::Menu,
        name: "look",
        group: Group::Look,
        needs: None,
        number: None,
        row: HudRow::Look,
        get: |m| format!("look {}", m.preset.name),
        flip: next_look,
    },
    Toggle {
        press: Press::Menu,
        name: "far normal",
        group: Group::Look,
        needs: None,
        number: None,
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
        needs: None,
        number: None,
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
        needs: None,
        number: None,
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
        press: Press::Menu,
        name: "sharpen",
        group: Group::Upscaler,
        needs: None,
        number: None,
        row: HudRow::Upscaler,
        get: |m| format!("sharpen {:.2}", m.upscaler.sharpen),
        flip: |m, now| {
            let s = hud::next_sharpen(m.upscaler.sharpen);
            m.upscaler.set_sharpen(s, now);
        },
    },
    Toggle {
        press: Press::Menu,
        name: "pan",
        group: Group::Upscaler,
        needs: Some(Needs::Pan),
        number: None,
        row: HudRow::Upscaler,
        get: |m| format!("pan {}", m.pan.as_ref().map_or("none", |p| on(p.on))),
        flip: |m, now| {
            if let Some(p) = m.pan.as_mut() {
                p.toggle(now);
            }
        },
    },
    Toggle {
        press: Press::Menu,
        name: "queue order",
        group: Group::Queue,
        needs: None,
        number: None,
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
        name: "queue angle",
        group: Group::Queue,
        needs: None,
        number: Some(Number {
            step: step_queue_angle,
            range: |_, _| format!("0..{QUEUE_ANGLE_MAX}, doubled or halved"),
        }),
        row: HudRow::Queue,
        get: |m| format!("angle {}", m.queue.angle),
        flip: |m, _| step_queue_angle(m, 1),
    },
    Toggle {
        press: Press::Menu,
        name: "queue withdraw",
        group: Group::Queue,
        needs: None,
        number: None,
        row: HudRow::Queue,
        get: |m| format!("withdraw {}", on(m.queue.withdraw)),
        flip: |m, _| m.queue.withdraw = !m.queue.withdraw,
    },
    Toggle {
        press: Press::Menu,
        name: "bake budget",
        group: Group::Queue,
        needs: None,
        number: None,
        row: HudRow::Queue,
        get: |m| format!("budget {}", m.queue.budget),
        flip: |m, _| m.queue.budget = next_budget(m.queue.budget),
    },
    Toggle {
        press: Press::Menu,
        name: "full-first",
        group: Group::Queue,
        needs: None,
        number: None,
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
        press: Press::Menu,
        name: "missing light",
        group: Group::Queue,
        needs: None,
        number: None,
        row: HudRow::Queue,
        get: |m| format!("missing light {}", m.queue.missing_light.name()),
        flip: |m, _| m.queue.missing_light = next_missing_light(m.queue.missing_light),
    },
    Toggle {
        press: Press::Menu,
        name: "reach",
        group: Group::Queue,
        needs: None,
        number: None,
        row: HudRow::Queue,
        get: |m| format!("reach {}", m.queue.reach.name()),
        flip: |m, _| m.queue.reach = next_reach(m.queue.reach),
    },
    Toggle {
        press: Press::Menu,
        name: "carry",
        group: Group::Lights,
        needs: None,
        number: None,
        row: HudRow::Lights,
        get: lights_words,
        flip: |m, now| {
            m.lights.toggle_carry();
            let words = format!("carry {}", on(m.lights.carry));
            m.show_lights(words, now);
        },
    },
    Toggle {
        press: Press::Menu,
        name: "colour",
        group: Group::Lights,
        needs: None,
        number: None,
        row: HudRow::Lights,
        get: lights_words,
        flip: |m, now| {
            let name = m.lights.cycle_colour();
            m.show_lights(format!("colour {name}"), now);
        },
    },
    Toggle {
        press: Press::Menu,
        name: "bounce",
        group: Group::Lights,
        needs: None,
        number: None,
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
        needs: None,
        number: None,
        row: HudRow::Controls,
        get: |m| format!("cutaway {}", m.cutaway.name()),
        flip: |m, _| m.cutaway = m.cutaway.next(),
    },
    Toggle {
        press: Press::Menu,
        name: "shadow ray",
        group: Group::Debug,
        needs: None,
        number: None,
        row: HudRow::Controls,
        get: |m| format!("shadow ray {}", on(m.shadow_ray)),
        flip: |m, _| m.shadow_ray = !m.shadow_ray,
    },
    diag!(
        "penumbra",
        |m| format!("penumbra {}", m.lighting.penumbra.name()),
        |m| {
            use vxe_render::Penumbra;
            m.lighting.penumbra = match m.lighting.penumbra {
                Penumbra::Approach => Penumbra::Disc,
                Penumbra::Disc => Penumbra::Approach,
            };
        }
    ),
    diag!(
        "sky normal",
        |m| format!("sky normal {}", m.lighting.sky_normal.name()),
        |m| {
            use vxe_render::SkyNormal;
            m.lighting.sky_normal = match m.lighting.sky_normal {
                SkyNormal::Smoothed => SkyNormal::Face,
                SkyNormal::Face => SkyNormal::Smoothed,
            };
        }
    ),
    diag!(
        "filter",
        |m| format!("filter {}", on(m.lighting.filter)),
        |m| {
            m.lighting.filter = !m.lighting.filter;
        }
    ),
    diag!(
        "light history",
        |m| format!("light history {}", on(m.lighting.light_history)),
        |m| {
            m.lighting.light_history = !m.lighting.light_history;
        }
    ),
    diag!(
        "landing fade",
        |m| format!("landing fade {}", on(m.lighting.landing_fade)),
        |m| {
            m.lighting.landing_fade = !m.lighting.landing_fade;
        }
    ),
    diag!(
        "source fade",
        |m| format!("source fade {}", on(m.lighting.source_fade)),
        |m| {
            m.lighting.source_fade = !m.lighting.source_fade;
        }
    ),
    diag!(
        "far light wide",
        |m| format!("far light wide {}", on(m.lighting.far_light_wide)),
        |m| {
            m.lighting.far_light_wide = !m.lighting.far_light_wide;
        }
    ),
    diag!(
        "light history far",
        |m| format!("light history far {}", on(m.lighting.light_history_far)),
        |m| {
            m.lighting.light_history_far = !m.lighting.light_history_far;
        }
    ),
    diag!(
        "key jitter",
        |m| format!("key jitter {}", on(!m.diag.hooks.key_no_jitter)),
        |m| {
            m.diag.hooks.key_no_jitter = !m.diag.hooks.key_no_jitter;
        }
    ),
    diag!(
        "stair rules",
        |m| format!("stair rules {}", on(!m.diag.hooks.no_stair)),
        |m| {
            m.diag.hooks.no_stair = !m.diag.hooks.no_stair;
        }
    ),
    diag!(
        "queue freeze",
        |m| format!("queue freeze {}", on(m.diag.queue_frozen)),
        |m| {
            m.diag.queue_frozen = !m.diag.queue_frozen;
        }
    ),
    Toggle {
        press: Press::Menu,
        name: "time",
        group: Group::World,
        needs: Some(Needs::Sky),
        number: Some(Number {
            step: |m, d| m.clock.step_hours_held(d),
            range: |_, _| "00:00..23:59, an hour a step".to_owned(),
        }),
        row: HudRow::Lighting,
        get: |m| {
            let mut out = String::new();
            m.clock.write_time(&mut out);
            out
        },
        flip: |m, _| m.clock.step_hours_held(1),
    },
    Toggle {
        press: Press::Menu,
        name: "day cycle",
        group: Group::World,
        needs: Some(Needs::Sky),
        number: None,
        row: HudRow::Lighting,
        get: |m| format!("day cycle {}", on(m.clock.running)),
        flip: |m, now| {
            m.clock.running = !m.clock.running;
            m.banner_at = now;
        },
    },
    Toggle {
        press: Press::Menu,
        name: "sky rays",
        group: Group::World,
        needs: Some(Needs::Sky),
        number: Some(Number {
            step: |m, d| {
                let max = vxe_render::MAX_SKY_RAYS as i64;
                m.lighting.sky_rays =
                    (i64::from(m.lighting.sky_rays) + i64::from(d)).clamp(1, max) as u32;
            },
            range: |_, _| format!("1..{}", vxe_render::MAX_SKY_RAYS),
        }),
        row: HudRow::Lighting,
        get: |m| format!("sky rays {}", m.lighting.sky_rays),
        flip: |m, _| {
            m.lighting.sky_rays = m.lighting.sky_rays % vxe_render::MAX_SKY_RAYS + 1;
        },
    },
    Toggle {
        press: Press::Menu,
        name: "dig radius",
        group: Group::Moving,
        needs: None,
        number: Some(Number {
            step: |m, d| {
                m.dig_radius =
                    (m.dig_radius + i64::from(d) * DIG_STEP).clamp(DIG_RANGE.0, DIG_RANGE.1);
            },
            range: |_, _| {
                let m = |r: i64| r as f64 / (10 * vxe_world::FIXED_ONE) as f64;
                format!("{:.1}..{:.1} m", m(DIG_RANGE.0), m(DIG_RANGE.1))
            },
        }),
        row: HudRow::Controls,
        get: |m| {
            format!(
                "dig radius {:.1} m",
                m.dig_radius as f64 / (10 * vxe_world::FIXED_ONE) as f64
            )
        },
        flip: |m, _| m.dig_radius = (m.dig_radius + DIG_STEP).min(DIG_RANGE.1),
    },
    Toggle {
        press: Press::Menu,
        name: "clear box",
        group: Group::Structures,
        needs: None,
        number: None,
        row: HudRow::Structures,
        get: |m| format!("clear box {}", on(m.clear_box)),
        flip: |m, _| m.clear_box = !m.clear_box,
    },
    Toggle {
        press: Press::Menu,
        name: "structure scale",
        group: Group::Structures,
        needs: None,
        number: Some(Number {
            step: |m, d| {
                m.structure_scale = (m.structure_scale as i32 + d).clamp(1, MAX_SCALE) as u32;
            },
            range: |_, _| format!("1..{MAX_SCALE}"),
        }),
        row: HudRow::Structures,
        get: |m| format!("structure scale {}", m.structure_scale),
        flip: |m, _| m.structure_scale = m.structure_scale % MAX_SCALE as u32 + 1,
    },
    Toggle {
        press: Press::Key(KeyCode::KeyF, Some(PadButton::DpadUp)),
        name: SNAP,
        group: Group::Structures,
        needs: None,
        number: None,
        row: HudRow::Structures,
        get: |m| format!("snap {}", m.snap.name()),
        flip: |m, _| m.snap = m.snap.other(),
    },
];

/// The `structures` group's snap toggle's name: its key's press says its row in the notice.
pub const SNAP: &str = "snap";

/// A structure's largest scale (`vxe_assets::MAX_SCALE`).
const MAX_SCALE: i32 = vxe_assets::MAX_SCALE as i32;
