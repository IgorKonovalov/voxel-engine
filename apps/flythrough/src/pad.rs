//! The fly-through's gamepad: `gilrs` drained once a frame into the shell's [`Input`], whose
//! axes and pad buttons name no gamepad library (ADR-0019). The mapping is two plain tables
//! over SDL's controller names, which `gilrs` reports for a mapped pad: [`AXES`] for the
//! sticks and the analog triggers, [`BUTTONS`] for the digital buttons.

use gilrs::{EventType, GamepadId, Gilrs};
use vxe_app::{Axis, Input, PadButton};

/// A `gilrs` source and the `Input` axis it feeds. Sticks report y positive up, which is
/// forward on the move stick and up on the look stick, so no row flips a sign. The analog
/// triggers (`L2`, `R2`) arrive as button values in `[0, 1]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AxisSource {
    Axis(gilrs::Axis),
    Button(gilrs::Button),
}

pub const AXES: [(AxisSource, Axis); 6] = [
    (AxisSource::Axis(gilrs::Axis::LeftStickX), Axis::MoveX),
    (AxisSource::Axis(gilrs::Axis::LeftStickY), Axis::MoveY),
    (AxisSource::Axis(gilrs::Axis::RightStickX), Axis::LookX),
    (AxisSource::Axis(gilrs::Axis::RightStickY), Axis::LookY),
    (
        AxisSource::Button(gilrs::Button::LeftTrigger2),
        Axis::TriggerL,
    ),
    (
        AxisSource::Button(gilrs::Button::RightTrigger2),
        Axis::TriggerR,
    ),
];

/// `gilrs`'s digital buttons and the pad buttons they press. `LeftTrigger` and `RightTrigger`
/// are the shoulder buttons (`L1`, `R1`); on a DualSense `South` is Cross, `East` Circle,
/// `West` Square, `North` Triangle, `Start` Options and `Select` Create.
pub const BUTTONS: [(gilrs::Button, PadButton); 14] = [
    (gilrs::Button::South, PadButton::South),
    (gilrs::Button::East, PadButton::East),
    (gilrs::Button::West, PadButton::West),
    (gilrs::Button::North, PadButton::North),
    (gilrs::Button::LeftTrigger, PadButton::L1),
    (gilrs::Button::RightTrigger, PadButton::R1),
    (gilrs::Button::LeftThumb, PadButton::L3),
    (gilrs::Button::RightThumb, PadButton::R3),
    (gilrs::Button::DPadUp, PadButton::DpadUp),
    (gilrs::Button::DPadDown, PadButton::DpadDown),
    (gilrs::Button::DPadLeft, PadButton::DpadLeft),
    (gilrs::Button::DPadRight, PadButton::DpadRight),
    (gilrs::Button::Start, PadButton::Start),
    (gilrs::Button::Select, PadButton::Select),
];

fn axis_of(source: AxisSource) -> Option<Axis> {
    AXES.iter().find(|(s, _)| *s == source).map(|&(_, a)| a)
}

fn button_of(b: gilrs::Button) -> Option<PadButton> {
    BUTTONS.iter().find(|(g, _)| *g == b).map(|&(_, p)| p)
}

/// One `gilrs` event applied to `input`: a mapped axis sets its value, a mapped button
/// presses or releases. Everything else changes nothing.
fn apply(event: EventType, input: &mut Input) {
    match event {
        EventType::AxisChanged(a, v, _) => set(AxisSource::Axis(a), v, input),
        EventType::ButtonChanged(b, v, _) => set(AxisSource::Button(b), v, input),
        EventType::ButtonPressed(b, _) => press(b, true, input),
        EventType::ButtonReleased(b, _) => press(b, false, input),
        _ => {}
    }
}

fn set(source: AxisSource, v: f32, input: &mut Input) {
    if let Some(axis) = axis_of(source) {
        input.set_axis(axis, v);
    }
}

fn press(b: gilrs::Button, down: bool, input: &mut Input) {
    match (button_of(b), down) {
        (Some(p), true) => input.pad_down(p),
        (Some(p), false) => input.pad_up(p),
        (None, _) => {}
    }
}

/// The gamepad source: `gilrs` and the pad whose events feed `Input`. The first pad
/// connected at startup is active; with none, the first pad to send an event becomes it.
pub struct Pad {
    gilrs: Gilrs,
    active: Option<GamepadId>,
}

impl Pad {
    /// `gilrs` on this machine, or `None` when it cannot start (no udev, as in a container),
    /// which the fly-through treats as no pad.
    pub fn open() -> Option<Self> {
        match Gilrs::new() {
            Ok(gilrs) => {
                let active = gilrs.gamepads().next().map(|(id, _)| id);
                Some(Self { gilrs, active })
            }
            Err(e) => {
                log::warn!("gamepad: gilrs did not start: {e}");
                None
            }
        }
    }

    /// Drains the events since the last call into `input`. Events of a pad other than the
    /// active one are dropped; the active pad's disconnect zeroes its axes and buttons.
    pub fn poll(&mut self, input: &mut Input) {
        while let Some(ev) = self.gilrs.next_event() {
            match ev.event {
                EventType::Disconnected if self.active == Some(ev.id) => {
                    self.active = None;
                    input.release_pad();
                }
                EventType::Connected | EventType::Disconnected => {}
                e => {
                    let active = *self.active.get_or_insert(ev.id);
                    if active == ev.id {
                        apply(e, input);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The plan's mapping, row by row: every `gilrs` axis and button it names feeds the
    /// `Input` axis or pad button its table gives.
    #[test]
    fn the_tables_map_each_named_source() {
        use AxisSource as S;
        use gilrs::{Axis as G, Button as B};
        let axes = [
            (S::Axis(G::LeftStickX), Axis::MoveX),
            (S::Axis(G::LeftStickY), Axis::MoveY),
            (S::Axis(G::RightStickX), Axis::LookX),
            (S::Axis(G::RightStickY), Axis::LookY),
            (S::Button(B::LeftTrigger2), Axis::TriggerL),
            (S::Button(B::RightTrigger2), Axis::TriggerR),
        ];
        for (s, a) in axes {
            assert_eq!(axis_of(s), Some(a), "{s:?}");
        }
        let buttons = [
            (B::South, PadButton::South),
            (B::East, PadButton::East),
            (B::West, PadButton::West),
            (B::North, PadButton::North),
            (B::LeftTrigger, PadButton::L1),
            (B::RightTrigger, PadButton::R1),
            (B::LeftThumb, PadButton::L3),
            (B::RightThumb, PadButton::R3),
            (B::DPadUp, PadButton::DpadUp),
            (B::DPadDown, PadButton::DpadDown),
            (B::DPadLeft, PadButton::DpadLeft),
            (B::DPadRight, PadButton::DpadRight),
            (B::Start, PadButton::Start),
            (B::Select, PadButton::Select),
        ];
        for (g, p) in buttons {
            assert_eq!(button_of(g), Some(p), "{g:?}");
        }
        // Each pad button is fed by exactly one row; the analog triggers are not buttons.
        for p in PadButton::ALL {
            assert_eq!(BUTTONS.iter().filter(|(_, q)| *q == p).count(), 1, "{p:?}");
        }
        assert_eq!(button_of(B::RightTrigger2), None);
        assert_eq!(button_of(B::Mode), None);
        assert_eq!(axis_of(S::Axis(G::LeftZ)), None);
    }

    /// Mapped sources land in `Input` with the table's sign: left stick up is `MoveY`
    /// positive, `R2` half pulled is `TriggerR` 0.5, Cross presses `South`; an unmapped
    /// source changes nothing.
    #[test]
    fn sources_feed_the_input() {
        let mut input = Input::default();
        set(AxisSource::Axis(gilrs::Axis::LeftStickY), 0.75, &mut input);
        set(
            AxisSource::Button(gilrs::Button::RightTrigger2),
            0.5,
            &mut input,
        );
        set(AxisSource::Axis(gilrs::Axis::LeftZ), 0.9, &mut input);
        press(gilrs::Button::South, true, &mut input);
        press(gilrs::Button::Mode, true, &mut input);
        assert_eq!(input.axis(Axis::MoveY), 0.75);
        assert_eq!(input.axis(Axis::TriggerR), 0.5);
        assert_eq!(
            Axis::ALL.map(|a| input.axis(a)),
            [0.0, 0.75, 0.0, 0.0, 0.0, 0.5]
        );
        assert!(input.pad_pressed(PadButton::South));
        assert_eq!(
            PadButton::ALL
                .iter()
                .filter(|&&p| input.pad_held(p))
                .count(),
            1
        );
        press(gilrs::Button::South, false, &mut input);
        assert!(!input.pad_held(PadButton::South));
    }
}
