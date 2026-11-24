//! The one capture rule (ADR-0061): what the menu takes from a frame's input and what passes
//! on to the run. Closed, everything passes and only the menu's keys (`Tab`, `F1`, the pad's
//! `Start`) open it.
//! Open, the arrows, `Enter`, `Backspace`, `Esc`, the typed keys and the punctuation (`` ` ``
//! shows the entities' page, and places from it), the wheel, both clicks, the
//! D-pad, `South`, `East` and `Start` are the menu's; the movement keys, the look, the
//! function keys and the other pad buttons pass, so a function-key shortcut acts in place and
//! its row follows. The movement letters and `Space` do both: their press types, their hold
//! moves. The triggers (the dig and the fill) pause while it is open.

use super::page::{Activated, Menu, MenuInput};
use super::{FIXED_KEYS, RunView, bound_keys};
use crate::hud::Modes;
use svet_app::{Axis, Input, KeyCode, MouseButton, PadButton};

/// An input the rule sorts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteInput {
    Key(KeyCode),
    Pad(PadButton),
    Click(MouseButton),
    Wheel,
    /// Mouse motion.
    MouseLook,
    /// Either stick.
    Stick,
    /// `L2` or `R2`.
    Trigger,
}

/// Where an input goes: to the menu, on to the run, both, or (`menu` and `pass` both false)
/// nowhere this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Taken {
    pub menu: bool,
    pub pass: bool,
}

const MENU: Taken = Taken {
    menu: true,
    pass: false,
};
const PASS: Taken = Taken {
    menu: false,
    pass: true,
};

/// The movement keys: they pass while the menu is open (their hold moves).
const MOVE_KEYS: [KeyCode; 7] = [
    KeyCode::KeyW,
    KeyCode::KeyA,
    KeyCode::KeyS,
    KeyCode::KeyD,
    KeyCode::Space,
    KeyCode::KeyC,
    KeyCode::ShiftLeft,
];

/// The keys that move the selection, act, go back and erase while open.
// keys:typed-begin
pub const MENU_KEYS: [KeyCode; 7] = [
    KeyCode::ArrowUp,
    KeyCode::ArrowDown,
    KeyCode::ArrowLeft,
    KeyCode::ArrowRight,
    KeyCode::Enter,
    KeyCode::Escape,
    KeyCode::Backspace,
];
// keys:typed-end

/// The pad buttons that walk the menu while open.
const MENU_PAD: [PadButton; 7] = [
    PadButton::DpadUp,
    PadButton::DpadDown,
    PadButton::DpadLeft,
    PadButton::DpadRight,
    PadButton::South,
    PadButton::East,
    PadButton::Start,
];

/// The character a typed key adds to the filter: a letter, a digit, a space or `-`. The
/// keys here are text, not binds (the source scan skips them).
pub fn typed(key: KeyCode) -> Option<char> {
    // keys:typed-begin
    const TYPED: [(KeyCode, char); 38] = [
        (KeyCode::KeyA, 'a'),
        (KeyCode::KeyB, 'b'),
        (KeyCode::KeyC, 'c'),
        (KeyCode::KeyD, 'd'),
        (KeyCode::KeyE, 'e'),
        (KeyCode::KeyF, 'f'),
        (KeyCode::KeyG, 'g'),
        (KeyCode::KeyH, 'h'),
        (KeyCode::KeyI, 'i'),
        (KeyCode::KeyJ, 'j'),
        (KeyCode::KeyK, 'k'),
        (KeyCode::KeyL, 'l'),
        (KeyCode::KeyM, 'm'),
        (KeyCode::KeyN, 'n'),
        (KeyCode::KeyO, 'o'),
        (KeyCode::KeyP, 'p'),
        (KeyCode::KeyQ, 'q'),
        (KeyCode::KeyR, 'r'),
        (KeyCode::KeyS, 's'),
        (KeyCode::KeyT, 't'),
        (KeyCode::KeyU, 'u'),
        (KeyCode::KeyV, 'v'),
        (KeyCode::KeyW, 'w'),
        (KeyCode::KeyX, 'x'),
        (KeyCode::KeyY, 'y'),
        (KeyCode::KeyZ, 'z'),
        (KeyCode::Digit0, '0'),
        (KeyCode::Digit1, '1'),
        (KeyCode::Digit2, '2'),
        (KeyCode::Digit3, '3'),
        (KeyCode::Digit4, '4'),
        (KeyCode::Digit5, '5'),
        (KeyCode::Digit6, '6'),
        (KeyCode::Digit7, '7'),
        (KeyCode::Digit8, '8'),
        (KeyCode::Digit9, '9'),
        (KeyCode::Space, ' '),
        (KeyCode::Minus, '-'),
    ];
    // keys:typed-end
    TYPED.iter().find(|(k, _)| *k == key).map(|&(_, c)| c)
}

/// Whether a key is printable punctuation: the filter's while open, though it types nothing.
fn punctuation(key: KeyCode) -> bool {
    let name = super::key_name(key);
    name.len() == 1 && name.chars().all(|c| c.is_ascii_punctuation())
}

/// The capture rule: where `i` goes with the menu `open` or closed.
pub fn taken(i: RouteInput, open: bool) -> Taken {
    if !open {
        return PASS;
    }
    match i {
        RouteInput::Key(k) if MOVE_KEYS.contains(&k) => Taken {
            menu: typed(k).is_some(),
            pass: true,
        },
        RouteInput::Key(k) if MENU_KEYS.contains(&k) || typed(k).is_some() || punctuation(k) => {
            MENU
        }
        RouteInput::Key(_) => PASS,
        RouteInput::Pad(b) if MENU_PAD.contains(&b) => MENU,
        RouteInput::Pad(_) => PASS,
        RouteInput::Click(_) | RouteInput::Wheel => MENU,
        RouteInput::MouseLook | RouteInput::Stick => PASS,
        RouteInput::Trigger => Taken {
            menu: false,
            pass: false,
        },
    }
}

/// The menu's inputs in `input`, with the menu `open` or closed: closed, `Tab`, `F1` and the
/// pad's `Start` alone; open, every input [`taken`] gives the menu, in a fixed order.
pub fn menu_inputs(input: &Input, open: bool) -> Vec<MenuInput> {
    let mut out = Vec::new();
    if input.pressed(KeyCode::Tab) {
        out.push(MenuInput::Menu);
    }
    if input.pressed(KeyCode::F1) {
        out.push(MenuInput::Keys);
    }
    if !open {
        if input.pad_pressed(PadButton::Start) {
            out.push(MenuInput::Menu);
        }
        return out;
    }
    // keys:typed-begin
    let keys = [
        (KeyCode::ArrowUp, MenuInput::Up),
        (KeyCode::ArrowDown, MenuInput::Down),
        (KeyCode::ArrowLeft, MenuInput::Left),
        (KeyCode::ArrowRight, MenuInput::Right),
        (KeyCode::Backspace, MenuInput::Erase),
        (KeyCode::Enter, MenuInput::Activate),
        (KeyCode::Escape, MenuInput::Back),
    ];
    // keys:typed-end
    let pads = [
        (PadButton::DpadUp, MenuInput::Up),
        (PadButton::DpadDown, MenuInput::Down),
        (PadButton::DpadLeft, MenuInput::Left),
        (PadButton::DpadRight, MenuInput::Right),
        (PadButton::South, MenuInput::Activate),
        (PadButton::East, MenuInput::Back),
        (PadButton::Start, MenuInput::Menu),
    ];
    for k in all_keys() {
        if let Some(c) = typed(k)
            && input.pressed(k)
        {
            out.push(MenuInput::Type(c));
        }
    }
    let lines = input.scroll().round() as i32;
    if lines != 0 {
        out.push(MenuInput::Scroll(lines));
    }
    out.extend(
        keys.iter()
            .filter(|(k, _)| input.pressed(*k))
            .map(|&(_, i)| i),
    );
    if input.pressed(crate::toggles::KEY_WALKER) {
        out.push(MenuInput::Entities);
    }
    out.extend(
        pads.iter()
            .filter(|(b, _)| input.pad_pressed(*b))
            .map(|&(_, i)| i),
    );
    if input.clicked(MouseButton::Left) {
        out.push(MenuInput::Activate);
    }
    if input.clicked(MouseButton::Right) {
        out.push(MenuInput::Back);
    }
    out
}

/// Every key the rule may meet: the tables' binds, the fixed keys, the menu's own keys and
/// the typed ones, each once.
pub fn all_keys() -> Vec<KeyCode> {
    let mut keys = bound_keys();
    keys.extend(FIXED_KEYS);
    keys.extend(MENU_KEYS);
    // keys:typed-begin
    keys.extend([
        KeyCode::KeyA,
        KeyCode::KeyB,
        KeyCode::KeyC,
        KeyCode::KeyD,
        KeyCode::KeyE,
        KeyCode::KeyF,
        KeyCode::KeyG,
        KeyCode::KeyH,
        KeyCode::KeyI,
        KeyCode::KeyJ,
        KeyCode::KeyK,
        KeyCode::KeyL,
        KeyCode::KeyM,
        KeyCode::KeyN,
        KeyCode::KeyO,
        KeyCode::KeyP,
        KeyCode::KeyQ,
        KeyCode::KeyR,
        KeyCode::KeyS,
        KeyCode::KeyT,
        KeyCode::KeyU,
        KeyCode::KeyV,
        KeyCode::KeyW,
        KeyCode::KeyX,
        KeyCode::KeyY,
        KeyCode::KeyZ,
        KeyCode::Digit0,
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
        KeyCode::Minus,
    ]);
    // keys:typed-end
    let mut seen = Vec::with_capacity(keys.len());
    for k in keys {
        if !seen.contains(&k) {
            seen.push(k);
        }
    }
    seen
}

/// What passes on to the run: the frame's input as it came, or a copy holding only what the
/// rule passes.
pub enum Passed<'a> {
    All(&'a Input),
    Filtered(Box<Input>),
}

impl Passed<'_> {
    pub fn input(&self) -> &Input {
        match self {
            Self::All(i) => i,
            Self::Filtered(i) => i,
        }
    }
}

/// A copy of `input` with only what `pass` keeps of the keys `keys`, the pad buttons, the
/// sticks, the triggers, the mouse's motion, its clicks and the wheel: held keys and buttons
/// stay held, this frame's presses stay presses.
pub fn keep(input: &Input, keys: &[KeyCode], pass: impl Fn(RouteInput) -> bool) -> Input {
    let keys: Vec<KeyCode> = keys
        .iter()
        .copied()
        .filter(|&k| pass(RouteInput::Key(k)))
        .collect();
    let pads: Vec<PadButton> = PadButton::ALL
        .into_iter()
        .filter(|&b| pass(RouteInput::Pad(b)))
        .collect();
    let mut out = Input::default();
    for &k in keys.iter().filter(|&&k| input.held(k)) {
        out.key_down(k);
    }
    for &b in pads.iter().filter(|&&b| input.pad_held(b)) {
        out.pad_down(b);
    }
    // The holds stay; the presses are written again below.
    out.end_frame();
    for &k in keys.iter().filter(|&&k| input.pressed(k)) {
        if input.held(k) {
            out.key_up(k);
            out.key_down(k);
        } else {
            out.key_down(k);
            out.key_up(k);
        }
    }
    for &b in pads.iter().filter(|&&b| input.pad_pressed(b)) {
        if input.pad_held(b) {
            out.pad_up(b);
            out.pad_down(b);
        } else {
            out.pad_down(b);
            out.pad_up(b);
        }
    }
    if pass(RouteInput::Stick) {
        for a in [Axis::MoveX, Axis::MoveY, Axis::LookX, Axis::LookY] {
            out.set_axis(a, input.axis(a));
        }
    }
    if pass(RouteInput::Trigger) {
        for a in [Axis::TriggerL, Axis::TriggerR] {
            out.set_axis(a, input.axis(a));
        }
    }
    if pass(RouteInput::MouseLook) {
        let [dx, dy] = input.mouse_delta();
        out.add_mouse_delta(dx, dy);
    }
    for b in [MouseButton::Left, MouseButton::Right, MouseButton::Middle] {
        if input.clicked(b) && pass(RouteInput::Click(b)) {
            out.button_down(b);
        }
    }
    if pass(RouteInput::Wheel) {
        out.add_scroll(input.scroll());
    }
    out
}

/// What a frame's step gives the run: the menu's activations, in order, and the input that
/// passes on.
pub struct Routed<'a> {
    pub acts: Vec<Activated>,
    pub pass: Passed<'a>,
}

/// The frame's menu step: reads the menu's inputs from `input` (with the menu as it was at
/// the frame's start), applies them at `m` and `v`, and gives the activations and what passes
/// on. A menu open at the frame's start captures this frame's input by [`taken`]; one opened
/// this frame does not.
pub fn step<'a>(menu: &mut Menu, input: &'a Input, m: &Modes, v: &RunView) -> Routed<'a> {
    step_at(menu, input, m, v, 0.0)
}

/// Milliseconds an arrow is held in a number's edit mode before it repeats, and between two
/// repeats (Dear ImGui's defaults).
pub const REPEAT_DELAY_MS: f64 = 275.0;
pub const REPEAT_RATE_MS: f64 = 50.0;

/// The repeats an arrow held for `held_ms` has given: one at [`REPEAT_DELAY_MS`] and one every
/// [`REPEAT_RATE_MS`] after, the press's own step not counted.
pub fn repeats_due(held_ms: f64) -> u32 {
    if held_ms < REPEAT_DELAY_MS {
        0
    } else {
        ((held_ms - REPEAT_DELAY_MS) / REPEAT_RATE_MS) as u32 + 1
    }
}

/// [`step`] at `now_ms` on the run's clock: in a number's edit mode an arrow (or the D-pad's
/// up or down) held past [`REPEAT_DELAY_MS`] steps again every [`REPEAT_RATE_MS`], each step
/// due by `now_ms` given once; released, the hold ends, and the next press steps at once.
pub fn step_at<'a>(
    menu: &mut Menu,
    input: &'a Input,
    m: &Modes,
    v: &RunView,
    now_ms: f64,
) -> Routed<'a> {
    let open = menu.is_open();
    let mut acts = Vec::new();
    for i in menu_inputs(input, open) {
        let arrow = match i {
            MenuInput::Up => Some(1),
            MenuInput::Down => Some(-1),
            _ => None,
        };
        let editing = menu.editing();
        if let Some(a) = menu.input(i, m, v) {
            acts.push(a);
        }
        if let Some(d) = arrow.filter(|_| editing && menu.editing()) {
            menu.hold = Some((d, now_ms, 0));
        }
    }
    if let Some((d, since, given)) = menu.hold {
        // keys:typed-begin
        let held = if d > 0 {
            input.held(KeyCode::ArrowUp) || input.pad_held(PadButton::DpadUp)
        } else {
            input.held(KeyCode::ArrowDown) || input.pad_held(PadButton::DpadDown)
        };
        // keys:typed-end
        if held && menu.editing() {
            let due = repeats_due(now_ms - since);
            let step = if d > 0 {
                MenuInput::Up
            } else {
                MenuInput::Down
            };
            for _ in given..due {
                if let Some(a) = menu.input(step, m, v) {
                    acts.push(a);
                }
            }
            menu.hold = Some((d, since, due.max(given)));
        } else {
            menu.hold = None;
        }
    }
    let pass = if open {
        Passed::Filtered(Box::new(keep(input, &all_keys(), |i| taken(i, true).pass)))
    } else {
        Passed::All(input)
    };
    Routed { acts, pass }
}
