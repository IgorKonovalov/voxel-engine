//! The fly-through's gamepad: `gilrs` drained once a frame into the shell's [`Input`], whose
//! axes and pad buttons name no gamepad library (ADR-0019). The mapping is two plain tables
//! over SDL's controller names, which `gilrs` reports for a mapped pad: [`AXES`] for the
//! sticks and the analog triggers, [`BUTTONS`] for the digital buttons.

use gilrs::{EventType, GamepadId, Gilrs, PowerInfo};
use vxe_app::{Axis, Input, PadButton};

use flythrough::hud::{PadPower, PadRow};

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

/// What `gilrs` reports of a pad's power, as the HUD shows it.
pub fn power_of(p: PowerInfo) -> PadPower {
    match p {
        PowerInfo::Unknown => PadPower::Unknown,
        PowerInfo::Wired => PadPower::Wired,
        PowerInfo::Discharging(n) => PadPower::Discharging(n),
        PowerInfo::Charging(n) => PadPower::Charging(n),
        PowerInfo::Charged => PadPower::Charged,
    }
}

/// The connected pads, oldest first, by an id the caller assigns; the newest is active. Kept
/// apart from `gilrs` so connects and disconnects are tested without a device.
#[derive(Debug, Default)]
pub struct PadState {
    pads: Vec<(usize, String)>,
}

impl PadState {
    /// The active pad's id and name.
    pub fn active(&self) -> Option<(usize, &str)> {
        self.pads.last().map(|(id, n)| (*id, n.as_str()))
    }

    /// Pad `id` connected and is now active; the line to print, or `None` for an id already
    /// connected.
    pub fn connected(&mut self, id: usize, name: &str, power: PadPower) -> Option<String> {
        if self.pads.iter().any(|(i, _)| *i == id) {
            return None;
        }
        self.pads.push((id, name.to_owned()));
        Some(format!("pad: {name} connected ({power})"))
    }

    /// Pad `id` disconnected; the line to print, naming the pad now active or `pad: none`,
    /// or `None` for an id not connected.
    pub fn disconnected(&mut self, id: usize) -> Option<String> {
        let k = self.pads.iter().position(|(i, _)| *i == id)?;
        let (_, name) = self.pads.remove(k);
        Some(match self.active() {
            Some((_, now)) => format!("pad: {name} disconnected; now {now}"),
            None => format!("pad: {name} disconnected; pad: none"),
        })
    }

    /// The startup line: the active pad and its power, or `pad: none`.
    pub fn startup_line(&self, power: PadPower) -> String {
        match self.active() {
            Some((_, name)) => format!("pad: {name} ({power})"),
            None => "pad: none".to_owned(),
        }
    }

    /// The HUD row's values: the active pad's name and power, empty and `None` without one.
    pub fn write_row(&self, power: PadPower, row: &mut PadRow) {
        match self.active() {
            Some((_, name)) => {
                if row.name != name {
                    row.name.clear();
                    row.name.push_str(name);
                }
                row.power = Some(power);
            }
            None => {
                row.name.clear();
                row.power = None;
            }
        }
    }
}

/// Polls between two reads of the active pad's power (two seconds at 60 Hz); reading it
/// opens sysfs files on Linux.
const POWER_EVERY: u32 = 120;

/// The gamepad source: `gilrs` and the pads it has connected. Only the active pad (the one
/// connected last) feeds `Input`.
pub struct Pad {
    gilrs: Gilrs,
    state: PadState,
    /// The active pad's power, read every [`POWER_EVERY`] polls and on a connect.
    power: PadPower,
    polls: u32,
}

impl Pad {
    /// `gilrs` on this machine with the pads already connected, or `None` when it cannot
    /// start (no udev, as in a container), which the fly-through treats as no pad.
    pub fn open() -> Option<Self> {
        let gilrs = match Gilrs::new() {
            Ok(g) => g,
            Err(e) => {
                log::warn!("gamepad: gilrs did not start: {e}");
                return None;
            }
        };
        let mut state = PadState::default();
        for (id, g) in gilrs.gamepads() {
            state.connected(usize::from(id), g.name(), power_of(g.power_info()));
        }
        let mut pad = Self {
            gilrs,
            state,
            power: PadPower::Unknown,
            polls: 0,
        };
        pad.read_power();
        Some(pad)
    }

    /// The startup line: the active pad and its power, or `pad: none`.
    pub fn startup_line(&self) -> String {
        self.state.startup_line(self.power)
    }

    fn gamepad_id(&self, id: usize) -> Option<GamepadId> {
        self.gilrs
            .gamepads()
            .map(|(g, _)| g)
            .find(|&g| usize::from(g) == id)
    }

    fn read_power(&mut self) {
        if let Some(id) = self.state.active().and_then(|(id, _)| self.gamepad_id(id)) {
            self.power = power_of(self.gilrs.gamepad(id).power_info());
        }
    }

    /// Drains the events since the last call into `input` and returns the lines a connect or
    /// disconnect prints. Events of a pad other than the active one are dropped; a change of
    /// active pad zeroes the axes and releases the buttons.
    pub fn poll(&mut self, input: &mut Input) -> Vec<String> {
        let mut lines = Vec::new();
        while let Some(ev) = self.gilrs.next_event() {
            let id = usize::from(ev.id);
            match ev.event {
                EventType::Connected => {
                    let g = self.gilrs.gamepad(ev.id);
                    let power = power_of(g.power_info());
                    if let Some(line) = self.state.connected(id, g.name(), power) {
                        self.power = power;
                        input.release_pad();
                        lines.push(line);
                    }
                }
                EventType::Disconnected => {
                    let was_active = self.state.active().map(|(a, _)| a) == Some(id);
                    if let Some(line) = self.state.disconnected(id) {
                        if was_active {
                            input.release_pad();
                            self.read_power();
                        }
                        lines.push(line);
                    }
                }
                e => {
                    if self.state.active().map(|(a, _)| a) == Some(id) {
                        apply(e, input);
                    }
                }
            }
        }
        self.polls = self.polls.wrapping_add(1);
        if self.polls.is_multiple_of(POWER_EVERY) {
            self.read_power();
        }
        lines
    }

    /// The HUD's pad row: the active pad's name and its last power reading.
    pub fn write_row(&self, row: &mut PadRow) {
        self.state.write_row(self.power, row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The DualSense mapping, row by row: every `gilrs` axis and button it names feeds the
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

    /// Connects and disconnects swap the active pad and print one line each: the newest
    /// pad is active, its disconnect hands back to the one before, the last one leaves
    /// `pad: none`; a repeated connect or an unknown disconnect prints nothing. The HUD row
    /// follows.
    #[test]
    fn connects_and_disconnects_swap_the_active_pad() {
        let mut s = PadState::default();
        let mut row = PadRow::default();
        assert_eq!(s.startup_line(PadPower::Unknown), "pad: none");
        s.write_row(PadPower::Unknown, &mut row);
        assert_eq!(row.to_string(), "pad none");

        let line = s.connected(3, "PS5 Controller", PadPower::Charging(5));
        assert_eq!(
            line.as_deref(),
            Some("pad: PS5 Controller connected (charging 5%)")
        );
        assert_eq!(
            s.connected(3, "PS5 Controller", PadPower::Charging(5)),
            None
        );
        assert_eq!(
            s.startup_line(PadPower::Charging(5)),
            "pad: PS5 Controller (charging 5%)"
        );
        s.write_row(PadPower::Charging(5), &mut row);
        assert_eq!(row.to_string(), "pad PS5 Controller, charging 5%");

        let line = s.connected(7, "Xbox Controller", PadPower::Wired);
        assert_eq!(
            line.as_deref(),
            Some("pad: Xbox Controller connected (wired)")
        );
        assert_eq!(s.active(), Some((7, "Xbox Controller")));
        s.write_row(PadPower::Wired, &mut row);
        assert_eq!(row.to_string(), "pad Xbox Controller, wired");

        assert_eq!(
            s.disconnected(7).as_deref(),
            Some("pad: Xbox Controller disconnected; now PS5 Controller")
        );
        assert_eq!(s.active(), Some((3, "PS5 Controller")));
        assert_eq!(s.disconnected(7), None);
        assert_eq!(
            s.disconnected(3).as_deref(),
            Some("pad: PS5 Controller disconnected; pad: none")
        );
        assert_eq!(s.active(), None);
        s.write_row(PadPower::Charged, &mut row);
        assert_eq!(row.to_string(), "pad none");
    }

    #[test]
    fn power_reads_as_the_hud_shows_it() {
        let shown = [
            PowerInfo::Unknown,
            PowerInfo::Wired,
            PowerInfo::Discharging(40),
            PowerInfo::Charging(5),
            PowerInfo::Charged,
        ]
        .map(|p| power_of(p).to_string());
        assert_eq!(
            shown,
            [
                "power unknown",
                "wired",
                "battery 40%",
                "charging 5%",
                "charged"
            ]
        );
    }
}
