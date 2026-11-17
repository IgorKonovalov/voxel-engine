//! The testbed's time of day: a clock the frame loop advances by its `dt` (the engine holds
//! none), turned into the frame's sky through [`vxe_render::sky::body_direction`] at latitude
//! [`LATITUDE_DEG`] and declination [`DECLINATION`], an equinox: the sun rises at 06:00, sets
//! at 18:00 and stands at 45 degrees in the south at 12:00. The moon is full, 12 hours from
//! the sun, at `vxe_render::sky::MOON_OVER_SUN` of its illuminance.
//!
//! The keys: [`KEY_HOUR`] steps the clock an hour forward, and back with Shift held;
//! [`KEY_DAY_CYCLE`] runs or pauses it at `--day-length`; [`KEY_SOURCE`] switches the world
//! source in process (`run::switch`), keeping the clock, and [`KEY_TREES`] the outdoor trees.

use crate::hud::Modes;
use std::fmt::Write as _;
use vxe_app::{Input, KeyCode};
use vxe_render::sky::{Body, Sky, body_direction};

/// The testbed's latitude in degrees.
pub const LATITUDE_DEG: f32 = 45.0;
/// The sun's declination in radians: 0, an equinox.
pub const DECLINATION: f32 = 0.0;
/// `--day-length` by default: seconds for a whole day while the cycle runs.
pub const DEFAULT_DAY_LENGTH_S: f32 = 1200.0;
/// `--time` by default: 12:00, in minutes after midnight.
pub const DEFAULT_TIME_MIN: f64 = 720.0;
const DAY_MIN: f64 = 1440.0;
/// `--sun-radius` and `--moon-radius` by default, in degrees: the testbed draws both bodies
/// wider than the real ones (`vxe_render::sky::SUN_ANGULAR_RADIUS`, the engine's default) so
/// their discs read on screen and their shadows are soft.
pub const BODY_RADIUS_DEG: f32 = 3.0;

/// `Home`: the next world source, rebuilt in process (`run::switch`).
pub const KEY_SOURCE: KeyCode = KeyCode::Home;
/// `PageUp`: an hour forward; with Shift, an hour back.
pub const KEY_HOUR: KeyCode = KeyCode::PageUp;
/// `End`: the day cycle runs or pauses.
pub const KEY_DAY_CYCLE: KeyCode = KeyCode::End;
/// `CapsLock`: the outdoor hills' next `--trees`, rebuilt in process as [`KEY_SOURCE`] does.
pub const KEY_TREES: KeyCode = KeyCode::CapsLock;

/// The time of day and whether it runs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clock {
    /// Minutes after midnight, in `[0, 1440)`.
    pub minutes: f64,
    /// The cycle runs: [`Clock::advance`] moves the time.
    pub running: bool,
    /// Seconds for a whole day while it runs.
    pub day_length_s: f32,
    /// The sun's angular radius in radians (`--sun-radius`).
    pub sun_radius: f32,
    /// The moon's angular radius in radians (`--moon-radius`).
    pub moon_radius: f32,
    /// `--sun EL,AZ`: the sun's elevation and azimuth in degrees (azimuth clockwise from
    /// north), fixed whatever the time; the moon stands opposite.
    pub fixed_sun: Option<[f32; 2]>,
}

impl Default for Clock {
    fn default() -> Self {
        Self {
            minutes: DEFAULT_TIME_MIN,
            running: false,
            day_length_s: DEFAULT_DAY_LENGTH_S,
            sun_radius: BODY_RADIUS_DEG.to_radians(),
            moon_radius: BODY_RADIUS_DEG.to_radians(),
            fixed_sun: None,
        }
    }
}

impl Clock {
    /// The sun's hour angle in radians: 0 at 12:00, 15 degrees an hour, positive after noon.
    pub fn hour_angle(&self) -> f32 {
        ((self.minutes / 60.0 - 12.0) * 15.0).to_radians() as f32
    }

    /// The sky at this time: the default atmosphere, the sun at [`Clock::hour_angle`] and
    /// the full moon 12 hours from it, with altitude 0 at voxel z `altitude_zero_z`. With
    /// [`Clock::fixed_sun`] the sun stands there and the moon opposite it.
    pub fn sky(&self, altitude_zero_z: f32) -> Sky {
        let lat = LATITUDE_DEG.to_radians();
        let h = self.hour_angle();
        let (sun_dir, moon_dir) = match self.fixed_sun {
            Some(ea) => {
                let d = fixed_direction(ea);
                (d, d.map(|c| -c))
            }
            None => (
                body_direction(h, DECLINATION, lat),
                body_direction(h + std::f32::consts::PI, -DECLINATION, lat),
            ),
        };
        let sun = Body {
            angular_radius: self.sun_radius,
            ..Body::sun(sun_dir)
        };
        let moon = Body {
            angular_radius: self.moon_radius,
            ..Body::moon(moon_dir)
        };
        Sky {
            altitude_zero_z,
            ..Sky::new(sun, moon)
        }
    }

    /// Moves the time by `dt` seconds of the frame loop while the cycle runs.
    pub fn advance(&mut self, dt: f32) {
        if self.running {
            let step = f64::from(dt) * DAY_MIN / f64::from(self.day_length_s);
            self.minutes = (self.minutes + step).rem_euclid(DAY_MIN);
        }
    }

    /// Moves the time by `hours`, wrapping at midnight.
    pub fn step_hours(&mut self, hours: i32) {
        self.minutes = (self.minutes + 60.0 * f64::from(hours)).rem_euclid(DAY_MIN);
    }

    /// `time 12:00, day cycle off`; with a fixed sun `sun 10/180 fixed, day cycle off`.
    pub fn write_words(&self, out: &mut String) {
        let m = self.minutes.floor() as u32;
        let cycle = if self.running { "on" } else { "off" };
        let _ = match self.fixed_sun {
            Some([e, a]) => write!(out, "sun {e}/{a} fixed, day cycle {cycle}"),
            None => write!(out, "time {:02}:{:02}, day cycle {cycle}", m / 60, m % 60),
        };
    }
}

/// The unit direction toward elevation `e` and azimuth `a` degrees (clockwise from north),
/// in world axes (+x east, +y north, +z up).
pub fn fixed_direction([e, a]: [f32; 2]) -> [f32; 3] {
    let (e, a) = (f64::from(e).to_radians(), f64::from(a).to_radians());
    [a.sin() * e.cos(), a.cos() * e.cos(), e.sin()].map(|c| c as f32)
}

/// `--sun EL,AZ` in degrees: an elevation in [-90, 90] and an azimuth in [0, 360).
pub fn parse_sun(s: &str) -> Result<[f32; 2], String> {
    let bad = || {
        format!(
            "--sun takes EL,AZ degrees, elevation -90 to 90 and azimuth 0 to under 360, not {s}"
        )
    };
    let (e, a) = s.split_once(',').ok_or_else(bad)?;
    let (e, a): (f32, f32) = (e.parse().map_err(|_| bad())?, a.parse().map_err(|_| bad())?);
    if !(-90.0..=90.0).contains(&e) || !(0.0..360.0).contains(&a) {
        return Err(bad());
    }
    Ok([e, a])
}

/// `--time HH:MM` in minutes after midnight: hours 0 to 23, minutes 0 to 59.
pub fn parse_time(s: &str) -> Result<f64, String> {
    let bad = || format!("--time takes HH:MM from 00:00 to 23:59, not {s}");
    let (h, m) = s.split_once(':').ok_or_else(bad)?;
    let (h, m): (u32, u32) = (h.parse().map_err(|_| bad())?, m.parse().map_err(|_| bad())?);
    if h > 23 || m > 59 {
        return Err(bad());
    }
    Ok(f64::from(h * 60 + m))
}

/// The clock's keys on `m`: [`KEY_HOUR`] (Shift held: back) and [`KEY_DAY_CYCLE`]; each shows
/// the banner from `now_ms` and prints the lighting row. Whether one was pressed. They act
/// while a sky is set, as the banner names the time only then.
pub(crate) fn clock_keys(input: &Input, m: &mut Modes, now_ms: f64) -> bool {
    if !m.sky {
        return false;
    }
    let shift = input.held(KeyCode::ShiftLeft) || input.held(KeyCode::ShiftRight);
    let pressed = if input.pressed(KEY_HOUR) {
        m.clock.step_hours(if shift { -1 } else { 1 });
        true
    } else if input.pressed(KEY_DAY_CYCLE) {
        m.clock.running = !m.clock.running;
        true
    } else {
        false
    };
    if pressed {
        m.banner_at = now_ms;
    }
    pressed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sun stands at 45 degrees in the south at 12:00, on the horizon in the east at
    /// 06:00 and in the west at 18:00; the moon is opposite.
    #[test]
    fn the_clock_turns_the_sun_through_an_equinox_day() {
        let at = |h: f64| {
            Clock {
                minutes: h * 60.0,
                ..Clock::default()
            }
            .sky(0.0)
        };
        let noon = at(12.0);
        let el = noon.sun.elevation().to_degrees();
        assert!((el - 45.0).abs() < 1e-3, "{el}");
        assert!(
            noon.sun.direction[1] < 0.0,
            "south: {:?}",
            noon.sun.direction
        );
        assert!(noon.moon.elevation().to_degrees() < -44.99);
        let six = at(6.0).sun;
        assert!(
            six.elevation().abs() < 1e-5 && six.direction[0] > 0.99,
            "{six:?}"
        );
        let eighteen = at(18.0).sun;
        assert!(eighteen.elevation().abs() < 1e-5 && eighteen.direction[0] < -0.99);
    }

    #[test]
    fn the_clock_advances_only_while_it_runs_and_wraps() {
        let mut c = Clock::default();
        c.advance(600.0);
        assert_eq!(c.minutes, 720.0);
        c.running = true;
        c.advance(600.0);
        assert_eq!(c.minutes, 0.0, "half a 1200 s day from noon is midnight");
        c.step_hours(-1);
        assert_eq!(c.minutes, 23.0 * 60.0);
        let mut s = String::new();
        c.write_words(&mut s);
        assert_eq!(s, "time 23:00, day cycle on");
    }

    /// On the outdoor source the banner names the clock: `PageUp` steps 12:00 to 13:00,
    /// Shift+`PageUp` back to 12:00, `End` turns the cycle on; the banner reads empty
    /// `BANNER_MS` after the last press. Off a sky the keys do nothing.
    #[test]
    fn the_clock_keys_change_the_banner() {
        let args = crate::cli::parse_args_from(crate::test_support::argv(&["--source", "outdoor"]))
            .expect("parses");
        let mut m = args.start_modes(0.0);
        let mut input = Input::default();
        let mut press = |m: &mut Modes, keys: &[KeyCode], now: f64| {
            keys.iter().for_each(|&k| input.key_down(k));
            let pressed = clock_keys(&input, m, now);
            keys.iter().for_each(|&k| input.key_up(k));
            input.end_frame();
            (pressed, m.banner(now).unwrap_or_default())
        };
        // The banner wraps its line at commas; its rows joined read the line again.
        let row = |b: &str| b.replace('\n', ", ");
        assert!(
            row(&m.banner(0.0).expect("at the start"))
                .contains("source outdoor, trees sparse, time 12:00, day cycle off, sky rays 1")
        );
        let (_, b) = press(&mut m, &[KEY_HOUR], 5000.0);
        assert!(row(&b).contains("time 13:00"), "{b}");
        let (_, b) = press(&mut m, &[KeyCode::ShiftLeft, KEY_HOUR], 6000.0);
        assert!(row(&b).contains("time 12:00"), "{b}");
        let (_, b) = press(&mut m, &[KEY_DAY_CYCLE], 7000.0);
        assert!(row(&b).contains("day cycle on"), "{b}");
        assert!(m.banner(7000.0 + crate::hud::BANNER_MS).is_none());
        let mut cave = crate::cli::parse_args_from(Vec::new())
            .expect("parses")
            .start_modes(0.0);
        assert!(!press(&mut cave, &[KEY_HOUR], 5000.0).0);
        assert!(!cave.line().contains("time") && !cave.line().contains("sky rays"));
    }

    #[test]
    fn time_parses_hh_mm_and_refuses_the_rest() {
        assert_eq!(parse_time("12:00"), Ok(720.0));
        assert_eq!(parse_time("00:00"), Ok(0.0));
        assert_eq!(parse_time("17:45"), Ok(1065.0));
        for bad in ["25:00", "24:00", "12:60", "12", "ab:cd", "-1:00", "12:5x"] {
            assert!(parse_time(bad).is_err(), "{bad}");
        }
    }
}
