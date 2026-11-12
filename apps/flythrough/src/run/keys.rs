//! The run's key and pad dispatch, and the cycles its keys step through.

use crate::app::ms_since;
use crate::castle;
use crate::cli::walk_allowed;
use crate::edits::scroll_dig_radius;
use crate::fly::Walk;
use crate::hud::{self};
use crate::logic::DigInput;
use crate::run::Run;
use crate::run::hud_feed::Samples;
use crate::setup::{castle_banner, castle_camera};
use crate::toggles::TOGGLES;
use vxe_app::{Axis, Input, KeyCode, MouseButton, PadButton};
use vxe_audio::{Command, PlayParams};
use vxe_render::Camera;

/// A trigger presses (`R2` digs, `L2` fills) once pulled past this, and lets go under
/// [`TRIGGER_RELEASE`], so a trigger resting near a threshold does not repeat.
pub(crate) const TRIGGER_PRESS: f32 = 0.5;

pub(crate) const TRIGGER_RELEASE: f32 = 0.3;

/// Whether a trigger at `pull` presses this frame, with `held` its state since the last.
pub(crate) fn trigger_pressed(held: &mut bool, pull: f32) -> bool {
    if *held {
        *held = pull >= TRIGGER_RELEASE;
        false
    } else {
        *held = pull >= TRIGGER_PRESS;
        *held
    }
}

/// The ore's emission scale `ms` milliseconds into the run while `M` pulses it: between 0.25
/// and 1.75, one cycle every 1.5 s, about [`ORE_PULSE_MEAN`].
pub(crate) fn ore_pulse(ms: f64) -> f32 {
    (f64::from(ORE_PULSE_MEAN) + 0.75 * (ms / 1500.0 * std::f64::consts::TAU).sin()) as f32
}

/// The ore pulse's time average, the scale the probe rays see the ore at, pulsing or not.
pub(crate) const ORE_PULSE_MEAN: f32 = 1.0;

impl Run {
    /// `G`: walk mode with the body under the camera, or back to flight.
    pub(crate) fn toggle_walk(&mut self) {
        if self.walk.take().is_some() {
            println!("fly mode");
            return;
        }
        if let Err(e) = walk_allowed(self.stream.is_some()) {
            println!("walk: {e}; still flying");
            return;
        }
        match Walk::at_camera(&self.world, &self.fly.camera) {
            Some(w) => {
                self.walk = Some(w);
                println!("walk mode");
            }
            None => println!("walk: no room for the body here; still flying"),
        }
    }

    /// Brings the run up to the modes after a toggle changed them from `before`: the
    /// upscaler and its sharpening at the next frame, the shadow ray, and the live queue's
    /// order, withdrawal, budget, full-first radius, missing light and reach, kept in `args`
    /// for the frame and the readings.
    pub(crate) fn apply_modes(&mut self, before: &hud::Modes) {
        let m = &self.modes;
        self.upscaler_changed |= m.upscaler.mode != before.upscaler.mode;
        if m.upscaler.sharpen != before.upscaler.sharpen {
            self.args.look.sharpen = m.upscaler.sharpen;
            self.sharpen_changed = true;
        }
        self.light.shadows = m.shadow_ray;
        let (q, b) = (m.queue, before.queue);
        let bake = &mut self.args.bake;
        (bake.queue_distance, bake.queue_tier) = (q.distance, q.tier);
        (bake.queue_angle, bake.queue_refine, bake.queue_withdraw) =
            (q.angle, q.refine, q.withdraw);
        bake.budget = q.budget;
        bake.full_first = Some(q.full_first);
        bake.missing_light = q.missing_light;
        bake.reach = q.reach;
        let Some(g) = self.gfx.as_mut() else { return };
        if q.reach != b.reach {
            g.frame.set_bake_reach(q.reach);
        }
        if q.full_first != b.full_first {
            g.frame.set_bake_full_first(q.full_first);
        }
        if q.budget != b.budget {
            g.frame.set_bake_budget(q.budget);
        }
        if (q.order(), q.withdraw) != (b.order(), b.withdraw)
            && let Some(queue) = g.frame.bake_queue_mut()
        {
            queue.order = q.order();
            queue.withdraw = q.withdraw;
        }
    }

    /// The toggles' keys, through [`TOGGLES`]: each pressed one flips its mode and prints its
    /// HUD row; then the run takes the changes ([`Run::apply_modes`]).
    pub(crate) fn toggle_keys(&mut self, input: &Input) {
        let mut before = None;
        let now = ms_since(self.start);
        for t in TOGGLES.iter().filter(|t| t.pressed(input)) {
            before.get_or_insert_with(|| self.modes.clone());
            (t.flip)(&mut self.modes, now);
            println!("{}", self.modes.row(t.row));
        }
        if let Some(before) = before {
            self.apply_modes(&before);
        }
    }

    /// The keys, buttons and pad buttons of interactive flight; the autopilot takes none of
    /// them, so a stray click cannot change the world it measures. The toggles go through
    /// [`TOGGLES`] ([`Run::toggle_keys`]); the rest are actions. On a pad `R2` digs, `L2`
    /// fills, the D-pad's up and down step the dig radius as a scroll line does, `North`
    /// toggles walking, `West` cycles the cutaway as `X` does, and `East` knocks.
    pub(crate) fn handle_input(&mut self, input: &Input) {
        self.toggle_keys(input);
        let scroll = input.scroll() + f32::from(u8::from(input.pad_pressed(PadButton::DpadUp)))
            - f32::from(u8::from(input.pad_pressed(PadButton::DpadDown)));
        if scroll != 0.0
            && let Some(r) = scroll_dig_radius(self.modes.dig_radius, &mut self.scroll, scroll)
        {
            self.modes.dig_radius = r;
            println!("{}", self.modes.row(hud::HudRow::Controls));
        }
        let r2 = trigger_pressed(&mut self.triggers[1], input.axis(Axis::TriggerR));
        if input.clicked(MouseButton::Left) || r2 {
            self.dig();
        }
        let l2 = trigger_pressed(&mut self.triggers[0], input.axis(Axis::TriggerL));
        if input.clicked(MouseButton::Right) || l2 {
            self.fill();
        }
        if input.pressed(KeyCode::KeyB) {
            self.bench();
        }
        if input.pressed(KeyCode::KeyP) {
            self.stamp();
        }
        if input.pressed(KeyCode::KeyG) || input.pad_pressed(PadButton::North) {
            self.toggle_walk();
        }
        if input.pressed(KeyCode::KeyK) {
            println!("{}", self.coarse_line());
        }
        if input.pressed(KeyCode::KeyR) {
            match self.coarse {
                Some(_) => self.send(DigInput::Crumble),
                None => println!("collapse: not kept while streaming"),
            }
        }
        if input.pressed(KeyCode::KeyI)
            && let Some(shot) = self.castle_shot.map(castle::CastleShot::next)
        {
            self.castle_shot = Some(shot);
            self.fly.camera = Camera {
                vfov: self.fly.camera.vfov,
                ..castle_camera(shot)
            };
            let line = castle_banner(shot);
            println!("{line}");
            self.scene = Some((line, ms_since(self.start)));
        }
        if input.pressed(KeyCode::F1) {
            self.samples = Some(Samples::default());
        }
        self.audio_keys(input);
    }

    /// The audio's keys, when it runs: `H` (or the pad's `East`) knocks, `T` a looping pulse,
    /// `J` a looping hum, `E` and `Y` the echoes and the tail, and the mix's trims, pre-delay,
    /// glide and direct delay, each change sent to the output and printed.
    pub(crate) fn audio_keys(&mut self, input: &Input) {
        if let Some(audio) = self.audio.as_mut() {
            if input.pressed(KeyCode::KeyH) || input.pad_pressed(PadButton::East) {
                audio.play("knock", PlayParams::at);
            }
            if input.pressed(KeyCode::KeyT) {
                audio.play("pulse", |p| PlayParams {
                    looping: true,
                    ..PlayParams::at(p)
                });
            }
            if input.pressed(KeyCode::KeyE) {
                let on = !audio.acoustics.reflections_on();
                audio.acoustics.set_reflections(on);
                println!("echoes {}", if on { "on" } else { "off" });
            }
            if input.pressed(KeyCode::KeyY) {
                let on = !audio.acoustics.reverb_on();
                audio.acoustics.set_reverb(on);
                println!("tail {}", if on { "on" } else { "off" });
            }
            if input.pressed(KeyCode::KeyJ) {
                audio.play("hum", |p| PlayParams {
                    looping: true,
                    ..PlayParams::at(p)
                });
            }
            let steps = [
                (KeyCode::BracketLeft, -3.0, 0.0, 0.0),
                (KeyCode::BracketRight, 3.0, 0.0, 0.0),
                (KeyCode::Minus, 0.0, -3.0, 0.0),
                (KeyCode::Equal, 0.0, 3.0, 0.0),
                (KeyCode::Digit9, 0.0, 0.0, -10.0),
                (KeyCode::Digit0, 0.0, 0.0, 10.0),
            ];
            let mut mix = audio.mix;
            let mut touched = false;
            for (key, echo, tail, pre) in steps {
                if input.pressed(key) {
                    mix = mix.step(echo, tail, pre);
                    touched = true;
                }
            }
            if input.pressed(KeyCode::KeyO) {
                mix.glide = !mix.glide;
                touched = true;
            }
            if input.pressed(KeyCode::KeyU) {
                mix.direct_delay = !mix.direct_delay;
                touched = true;
            }
            if touched {
                audio.mix = mix;
                // A full channel drops it; the next key sends the whole balance again.
                if let Err(e) = audio.output.send(Command::SetMix(mix.settings())) {
                    println!("mix: {e}");
                }
                println!("{}", mix.line());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trigger_presses_past_half_and_lets_go_under_three_tenths() {
        let mut held = false;
        let pulls = [0.4, 0.5, 0.9, 0.35, 0.6, 0.25, 0.55];
        let presses = pulls.map(|p| trigger_pressed(&mut held, p));
        assert_eq!(presses, [false, true, false, false, false, false, true]);
    }
}
