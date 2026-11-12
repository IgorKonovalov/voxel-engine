//! The run's key and pad dispatch, and the cycles its keys step through.

use crate::LookPreset;
use crate::app::ms_since;
use crate::castle;
use crate::cli::next_far_normal;
use crate::cli::walk_allowed;
use crate::edits::scroll_dig_radius;
use crate::fly::Walk;
use crate::hud::{self};
use crate::logic::DigInput;
use crate::run::Run;
use crate::run::hud_feed::{FullFirstWords, Samples, look_line};
use crate::setup::{castle_banner, castle_camera};
use vxe_app::{Axis, Input, KeyCode, MouseButton, PadButton};
use vxe_audio::{Command, PlayParams};
use vxe_render::{Camera, LookParams};
use vxe_world::FIXED_ONE;

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

/// `N`: the next preset, its normal mode and tuning in `look`. The run's far normal stays in
/// `look`, so returning to face mode uses it again.
pub(crate) fn next_look(preset: LookPreset, look: &mut LookParams, contour: bool) -> LookPreset {
    let next = preset.next(contour);
    look.normal_mode = next.normal_mode;
    look.tuning = next.tuning;
    next
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

/// The budgets key `5` steps through (Phase 6's arms): today's slice, 150 ms a second and 300
/// ms a second, each capped at a quarter of the frame.
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

    /// The live queue's keys: `1` cycles the order (score, score with the class as a tier,
    /// distance), `2` halves the score's angle weight and `3` doubles it (from 0, to 0.25), `4`
    /// toggles the withdrawal, `5` steps the bake budget through [`BUDGET_CYCLE`], `6` toggles
    /// the full-first radius between `off` and `2`, `7` cycles the missing light through
    /// [`vxe_render::MissingLight::ALL`], `8` toggles the reach between `full` and
    /// `partial`. A change goes to the queue at once (the missing light at the next frame) and
    /// prints the order and the withdrawal, the budget, the radius, the missing light or the
    /// reach.
    pub(crate) fn queue_keys(&mut self, input: &Input) {
        if input.pressed(KeyCode::Digit8) {
            self.args.bake.reach = next_reach(self.args.bake.reach);
            println!("bake queue: reach {}", self.args.bake.reach.name());
            if let Some(g) = self.gfx.as_mut() {
                g.frame.set_bake_reach(self.args.bake.reach);
            }
        }
        if input.pressed(KeyCode::Digit7) {
            self.args.bake.missing_light = next_missing_light(self.args.bake.missing_light);
            println!(
                "bake queue: missing light {}",
                self.args.bake.missing_light.name()
            );
        }
        if input.pressed(KeyCode::Digit6) {
            let r = match self.args.full_first() {
                Some(_) => None,
                None => Some(vxe_render::DEFAULT_NEAR),
            };
            self.args.bake.full_first = Some(r);
            println!("bake queue: full-first {}", FullFirstWords(r));
            if let Some(g) = self.gfx.as_mut() {
                g.frame.set_bake_full_first(r);
            }
        }
        if input.pressed(KeyCode::Digit5) {
            self.args.bake.budget = next_budget(self.args.bake.budget);
            let budget = self.args.bake.budget;
            println!("bake queue: budget {budget}");
            if let Some(g) = self.gfx.as_mut() {
                g.frame.set_bake_budget(budget);
            }
        }
        let a = &mut self.args;
        let mut changed = false;
        if input.pressed(KeyCode::Digit1) {
            (a.bake.queue_distance, a.bake.queue_tier) =
                match (a.bake.queue_distance, a.bake.queue_tier) {
                    (false, false) => (false, true),
                    (false, true) => (true, false),
                    (true, _) => (false, false),
                };
            changed = true;
        }
        if input.pressed(KeyCode::Digit2) {
            a.bake.queue_angle *= 0.5;
            changed = true;
        }
        if input.pressed(KeyCode::Digit3) {
            a.bake.queue_angle = if a.bake.queue_angle > 0.0 {
                a.bake.queue_angle * 2.0
            } else {
                0.25
            };
            changed = true;
        }
        if input.pressed(KeyCode::Digit4) {
            a.bake.queue_withdraw = !a.bake.queue_withdraw;
            changed = true;
        }
        if changed {
            let order = self.args.queue_order();
            println!("bake queue: {}", self.args.queue_words());
            let withdraw = self.args.bake.queue_withdraw;
            if let Some(q) = self.gfx.as_mut().and_then(|g| g.frame.bake_queue_mut()) {
                q.order = order;
                q.withdraw = withdraw;
            }
        }
    }

    /// The keys, buttons and pad buttons of interactive flight; the autopilot takes none of
    /// them, so a stray click cannot change the world it measures. On a pad `R2` digs, `L2`
    /// fills, the D-pad's up and down step the dig radius as a scroll line does, `North`
    /// toggles walking, `West` cycles the cutaway as `X` does, and `East` knocks.
    pub(crate) fn handle_input(&mut self, input: &Input) {
        self.queue_keys(input);
        for (key, mode) in [
            (KeyCode::F3, hud::ModeKey::ShadowWalk),
            (KeyCode::F5, hud::ModeKey::History),
            (KeyCode::F6, hud::ModeKey::Resolution),
            (KeyCode::F7, hud::ModeKey::Lamp),
            (KeyCode::F8, hud::ModeKey::LampBounce),
            (KeyCode::KeyM, hud::ModeKey::Pulse),
            (KeyCode::KeyQ, hud::ModeKey::Exposure),
            (KeyCode::Comma, hud::ModeKey::StoreRead),
            (KeyCode::Semicolon, hud::ModeKey::LightWeights),
        ] {
            if input.pressed(key) {
                self.modes.press(mode, ms_since(self.start));
                println!("{}", self.modes.line());
            }
        }
        if input.pressed(KeyCode::F10) {
            let mode = self.upscaler.mode.next();
            let (_, scale, _) = self.args.upscaler_settings(mode);
            self.upscaler
                .change(mode, scale.name(), ms_since(self.start));
            self.upscaler_changed = true;
            println!("{}", self.upscaler.line());
        }
        if input.pressed(KeyCode::KeyZ) {
            self.args.look.sharpen = hud::next_sharpen(self.args.look.sharpen);
            self.upscaler
                .set_sharpen(self.args.look.sharpen, ms_since(self.start));
            self.sharpen_changed = true;
            println!("{}", self.upscaler.line());
        }
        if input.pressed(KeyCode::KeyV) {
            self.look.far_normal = next_far_normal(self.look.far_normal);
            self.modes
                .show_look(&self.preset, self.look.far_normal, ms_since(self.start));
            println!("{}", look_line(&self.look, &self.preset));
        }
        if input.pressed(KeyCode::Period) {
            match self.pan.as_mut() {
                Some(p) => {
                    p.toggle(ms_since(self.start));
                    println!("{}", p.line());
                }
                None => println!("pan: start with --pan DEG_PER_S"),
            }
        }
        if input.pressed(KeyCode::KeyL) {
            self.light.shadows = !self.light.shadows;
            println!(
                "shadow ray {}",
                if self.light.shadows { "on" } else { "off" }
            );
        }
        let scroll = input.scroll() + f32::from(u8::from(input.pad_pressed(PadButton::DpadUp)))
            - f32::from(u8::from(input.pad_pressed(PadButton::DpadDown)));
        if scroll != 0.0
            && let Some(r) = scroll_dig_radius(self.dig_radius, &mut self.scroll, scroll)
        {
            self.dig_radius = r;
            println!("dig radius {:.1} m", r as f64 / (10 * FIXED_ONE) as f64);
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
        if input.pressed(KeyCode::KeyN) {
            let contour = self.gfx.as_ref().is_some_and(|g| g.mirror.field_surface());
            self.preset = next_look(self.preset, &mut self.look, contour);
            self.modes
                .show_look(&self.preset, self.look.far_normal, ms_since(self.start));
            println!("{}", look_line(&self.look, &self.preset));
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
        if input.pressed(KeyCode::KeyX) || input.pad_pressed(PadButton::West) {
            self.cutaway = self.cutaway.next();
            println!("cutaway {}", self.cutaway.name());
        }
        if input.pressed(KeyCode::F9) {
            self.look.view = self.look.view.next();
            self.modes.show_view(self.look.view, ms_since(self.start));
            println!("{}", look_line(&self.look, &self.preset));
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
