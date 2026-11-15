//! The fly-through's list of analytic lights (`vxe_render::Light`, ADR-0044): `1` drops one at
//! the looked-at point, `2` carries the newest in front of the camera, `3` cycles the picked
//! colour and recolours the newest, `6` fires one along the view ray, `4` removes the newest,
//! `5` flips the newest's bounce flag;
//! drops and shots take the picked colour; `--lights N` starts with a ring. The run hands
//! [`TestLights::list`] to the frame every frame (`Frame::set_lights`). Not the mirror's light
//! samples (`--light-radius`, the emissive voxels' candidates).

use std::fmt::Write as _;
use vxe_render::{Light, MAX_LIGHTS};

/// A dropped or fired light's intensity (owner, 2026-11-14: half the headlamp's 1200), reaching
/// 77.5 voxels at the default light floor.
pub const DROP_INTENSITY: f32 = 600.0;

/// `--lights N`'s ring lights' intensity.
pub const RING_INTENSITY: f32 = 600.0;

/// `--lights N`'s ring radius in voxels (3 m).
pub const RING_RADIUS: f32 = 30.0;

/// A dropped light sits this many voxels off the surface along its normal, and a shot stops
/// this short of the first surface on its ray.
pub const SURFACE_GAP: f32 = 2.0;

/// A drop whose ray hits nothing within reach lands this many voxels ahead.
pub const MISS_DROP: f32 = 20.0;

/// A shot's speed in voxels a second (20 m/s).
pub const SHOT_SPEED: f32 = 200.0;

/// A shot that hits nothing stops after this many voxels.
pub const SHOT_RANGE: f32 = 400.0;

/// The carried light sits this far ahead of the eye (2 m) and this far below it (0.3 m).
pub const CARRY_AHEAD: f32 = 20.0;
pub const CARRY_DOWN: f32 = 3.0;

/// The line a drop or a shot into a full list prints.
pub const FULL_LINE: &str =
    "lights: the list is full (Plan 0058: 16 slots, the headlamp's among them while it is on)";

/// The colours `3` cycles through, in order, with their names: white first.
pub const COLOURS: [(&str, [f32; 3]); 5] = [
    ("white", [1.0, 1.0, 1.0]),
    ("warm", [1.0, 0.7, 0.4]),
    ("red", [1.0, 0.0, 0.0]),
    ("green", [0.0, 1.0, 0.0]),
    ("blue", [0.0, 0.0, 1.0]),
];

/// A shot's flight: its unit direction and the voxels it has left to travel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flight {
    pub dir: [f32; 3],
    pub left: f32,
}

/// One light of the testbed's list, and its flight while it flies.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TestLight {
    pub light: Light,
    pub flight: Option<Flight>,
}

/// The testbed's lights, oldest first, whether the newest is carried, and the picked colour
/// (an index into [`COLOURS`]) that drops and shots take.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TestLights {
    pub lights: Vec<TestLight>,
    pub carry: bool,
    pub colour: usize,
}

fn add(a: [f32; 3], b: [f32; 3], s: f32) -> [f32; 3] {
    std::array::from_fn(|k| a[k] + b[k] * s)
}

impl TestLights {
    /// `n` white lights of [`RING_INTENSITY`] on a horizontal ring of [`RING_RADIUS`] around
    /// `centre` (the spawn's eye).
    pub fn ring(n: u32, centre: [f32; 3]) -> Self {
        let lights = (0..n)
            .map(|i| {
                let a = std::f32::consts::TAU * i as f32 / n as f32;
                let pos = [
                    centre[0] + RING_RADIUS * a.cos(),
                    centre[1] + RING_RADIUS * a.sin(),
                    centre[2],
                ];
                TestLight {
                    light: Light::point(pos, RING_INTENSITY),
                    flight: None,
                }
            })
            .collect();
        Self {
            lights,
            carry: false,
            colour: 0,
        }
    }

    /// The lights with their [`Light::BOUNCE`] flag set when `bounce` (`--lights-bounce`).
    pub fn bouncing_all(mut self, bounce: bool) -> Self {
        if bounce {
            for l in &mut self.lights {
                l.light.flags |= Light::BOUNCE;
            }
        }
        self
    }

    /// The picked colour's name and value.
    pub fn picked(&self) -> (&'static str, [f32; 3]) {
        COLOURS[self.colour % COLOURS.len()]
    }

    /// `3`: the next colour of [`COLOURS`] is picked, and the newest light (in flight or
    /// still) takes it. Returns its name.
    pub fn cycle_colour(&mut self) -> &'static str {
        self.colour = (self.colour + 1) % COLOURS.len();
        let (name, c) = self.picked();
        if let Some(l) = self.lights.last_mut() {
            l.light.colour = c;
        }
        name
    }

    /// A point light of the picked colour at `pos` of [`DROP_INTENSITY`].
    fn coloured(&self, pos: [f32; 3]) -> Light {
        Light {
            colour: self.picked().1,
            ..Light::point(pos, DROP_INTENSITY)
        }
    }

    /// The lights the frame shades beside the headlamp: fifteen while it is on, sixteen while
    /// off.
    pub fn cap(lamp: bool) -> usize {
        MAX_LIGHTS - usize::from(lamp)
    }

    /// The lights in flight.
    pub fn in_flight(&self) -> usize {
        self.lights.iter().filter(|l| l.flight.is_some()).count()
    }

    fn push(&mut self, lamp: bool, l: TestLight) -> Result<(), &'static str> {
        if self.lights.len() >= Self::cap(lamp) {
            return Err(FULL_LINE);
        }
        self.lights.push(l);
        Ok(())
    }

    /// `1`: a point light of the picked colour and [`DROP_INTENSITY`] at the looked-at hit, `hit` its distance
    /// along `fwd` from `eye` in voxels and its surface normal, [`SURFACE_GAP`] off the
    /// surface; [`MISS_DROP`] ahead with no hit. Refused when the list is full.
    pub fn drop_at(
        &mut self,
        lamp: bool,
        eye: [f32; 3],
        fwd: [f32; 3],
        hit: Option<(f32, [f32; 3])>,
    ) -> Result<(), &'static str> {
        let pos = match hit {
            Some((t, n)) => add(add(eye, fwd, t), n, SURFACE_GAP),
            None => add(eye, fwd, MISS_DROP),
        };
        let light = self.coloured(pos);
        self.push(
            lamp,
            TestLight {
                light,
                flight: None,
            },
        )
    }

    /// `6`: a point light of the picked colour and [`DROP_INTENSITY`] leaving `eye` along `fwd` at
    /// [`SHOT_SPEED`], to stop [`SURFACE_GAP`] short of the first surface, `hit_t` voxels
    /// along the ray (cast once, now), or after [`SHOT_RANGE`]. Its bounce flag is clear.
    /// Refused when the list is full.
    pub fn fire(
        &mut self,
        lamp: bool,
        eye: [f32; 3],
        fwd: [f32; 3],
        hit_t: Option<f32>,
    ) -> Result<(), &'static str> {
        let left = hit_t.map_or(SHOT_RANGE, |t| (t - SURFACE_GAP).max(0.0));
        let light = self.coloured(eye);
        self.push(
            lamp,
            TestLight {
                light,
                flight: Some(Flight { dir: fwd, left }),
            },
        )
    }

    /// `2`: carry the newest light (ending its flight), or set it down where it is. Carry
    /// switched on over an empty list takes the next light added.
    pub fn toggle_carry(&mut self) {
        self.carry = !self.carry;
        if self.carry
            && let Some(l) = self.lights.last_mut()
        {
            l.flight = None;
        }
    }

    /// `5`: flips the newest light's [`Light::BOUNCE`] flag (in flight or still), so it feeds
    /// the probe channel while the lamp's bounce is on. Returns the flag, `None` over an
    /// empty list.
    pub fn toggle_bounce(&mut self) -> Option<bool> {
        let l = self.lights.last_mut()?;
        l.light.flags ^= Light::BOUNCE;
        Some(l.light.flags & Light::BOUNCE != 0)
    }

    /// The lights with [`Light::BOUNCE`] set.
    pub fn bouncing(&self) -> usize {
        self.lights
            .iter()
            .filter(|l| l.light.flags & Light::BOUNCE != 0)
            .count()
    }

    /// `4`: removes the newest light; carry ends with it.
    pub fn remove_newest(&mut self) {
        self.lights.pop();
        self.carry = false;
    }

    /// One frame of `dt` seconds: each flight advances by [`SHOT_SPEED`] `dt` up to its stop;
    /// a carried light sits [`CARRY_AHEAD`] along `fwd` and [`CARRY_DOWN`] along `-up` from
    /// `eye`.
    pub fn step(&mut self, dt: f32, eye: [f32; 3], fwd: [f32; 3], up: [f32; 3]) {
        for l in &mut self.lights {
            if let Some(f) = l.flight.as_mut() {
                let go = (SHOT_SPEED * dt).min(f.left);
                l.light.pos = add(l.light.pos, f.dir, go);
                f.left -= go;
                if f.left <= 0.0 {
                    l.flight = None;
                }
            }
        }
        if self.carry
            && let Some(l) = self.lights.last_mut()
        {
            l.light.pos = add(add(eye, fwd, CARRY_AHEAD), up, -CARRY_DOWN);
        }
    }

    /// The lights as the frame takes them, oldest first.
    pub fn list(&self) -> Vec<Light> {
        self.lights.iter().map(|l| l.light).collect()
    }

    /// The `lights` row: `lights <n>/16 [bounce *..] [carry] [fly <m>] over <k> colour <name>`,
    /// `n` the lights shaded with the headlamp counted while it is on, one `*` for each list
    /// light flagged to bounce, `m` those in flight, `k` those past the cap, `name` the picked
    /// colour.
    pub fn write_row(&self, lamp: bool, out: &mut String) {
        let cap = Self::cap(lamp);
        let shaded = self.lights.len().min(cap) + usize::from(lamp);
        let _ = write!(out, "lights {shaded}/{MAX_LIGHTS}");
        let b = self.bouncing();
        if b > 0 {
            let _ = write!(out, " bounce {}", "*".repeat(b));
        }
        if self.carry {
            out.push_str(" carry");
        }
        let fly = self.in_flight();
        if fly > 0 {
            let _ = write!(out, " fly {fly}");
        }
        let _ = write!(
            out,
            " over {} colour {}",
            self.lights.len().saturating_sub(cap),
            self.picked().0
        );
    }

    /// The banner's count, `lights <n>/16`.
    pub fn count_words(&self, lamp: bool) -> String {
        let shaded = self.lights.len().min(Self::cap(lamp)) + usize::from(lamp);
        format!("lights {shaded}/{MAX_LIGHTS}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drop_sits_off_the_surface_and_a_miss_lands_ahead() {
        let mut l = TestLights::default();
        l.drop_at(
            false,
            [0.0; 3],
            [1.0, 0.0, 0.0],
            Some((10.0, [-1.0, 0.0, 0.0])),
        )
        .unwrap();
        assert_eq!(l.lights[0].light.pos, [8.0, 0.0, 0.0]);
        l.drop_at(false, [0.0; 3], [0.0, 1.0, 0.0], None).unwrap();
        assert_eq!(l.lights[1].light.pos, [0.0, MISS_DROP, 0.0]);
        assert_eq!(l.lights[1].light.intensity, DROP_INTENSITY);
        assert_eq!(l.lights[1].light.flags & Light::BOUNCE, 0);
    }

    /// Owner, 2026-11-14: drops and shots at 600, the ring's intensity.
    #[test]
    fn a_drop_and_a_shot_take_intensity_600() {
        let mut l = TestLights::default();
        l.drop_at(false, [0.0; 3], [1.0, 0.0, 0.0], None).unwrap();
        l.fire(false, [0.0; 3], [1.0, 0.0, 0.0], None).unwrap();
        assert_eq!(l.lights[0].light.intensity, 600.0);
        assert_eq!(l.lights[1].light.intensity, 600.0);
    }

    #[test]
    fn a_full_list_refuses_a_drop_and_a_shot() {
        let mut l = TestLights::default();
        for _ in 0..15 {
            l.drop_at(true, [0.0; 3], [1.0, 0.0, 0.0], None).unwrap();
        }
        assert_eq!(
            l.drop_at(true, [0.0; 3], [1.0, 0.0, 0.0], None),
            Err(FULL_LINE)
        );
        assert_eq!(
            l.fire(true, [0.0; 3], [1.0, 0.0, 0.0], None),
            Err(FULL_LINE)
        );
        assert!(FULL_LINE.contains("0058"));
        // With the lamp off a sixteenth fits, and a seventeenth is refused.
        l.fire(false, [0.0; 3], [1.0, 0.0, 0.0], None).unwrap();
        assert_eq!(l.lights.len(), 16);
        assert_eq!(
            l.drop_at(false, [0.0; 3], [1.0, 0.0, 0.0], None),
            Err(FULL_LINE)
        );
    }

    #[test]
    fn a_shot_flies_at_its_speed_and_stops_short_of_the_surface() {
        let mut l = TestLights::default();
        l.fire(false, [0.0; 3], [1.0, 0.0, 0.0], Some(12.0))
            .unwrap();
        l.step(0.025, [0.0; 3], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
        assert_eq!(l.lights[0].light.pos, [5.0, 0.0, 0.0]);
        assert_eq!(l.in_flight(), 1);
        l.step(0.1, [0.0; 3], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
        assert_eq!(l.lights[0].light.pos, [10.0, 0.0, 0.0]);
        assert_eq!(l.in_flight(), 0);
        // A miss flies SHOT_RANGE.
        l.fire(false, [0.0; 3], [0.0, 1.0, 0.0], None).unwrap();
        for _ in 0..200 {
            l.step(1.0 / 60.0, [0.0; 3], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
        }
        assert!((l.lights[1].light.pos[1] - SHOT_RANGE).abs() < 1e-3);
    }

    #[test]
    fn carry_holds_the_newest_ahead_and_ends_its_flight() {
        let mut l = TestLights::default();
        l.fire(false, [0.0; 3], [1.0, 0.0, 0.0], None).unwrap();
        l.toggle_carry();
        assert!(l.carry);
        assert_eq!(l.in_flight(), 0);
        l.step(
            1.0 / 60.0,
            [1.0, 2.0, 3.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        );
        assert_eq!(l.lights[0].light.pos, [1.0, 22.0, 0.0]);
        l.toggle_carry();
        l.step(1.0 / 60.0, [0.0; 3], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
        assert_eq!(l.lights[0].light.pos, [1.0, 22.0, 0.0]);
        l.remove_newest();
        assert!(l.lights.is_empty());
    }

    #[test]
    fn a_shot_takes_the_picked_colour() {
        let mut l = TestLights::default();
        assert_eq!(l.picked().0, "white");
        l.drop_at(false, [0.0; 3], [1.0, 0.0, 0.0], None).unwrap();
        assert_eq!(l.cycle_colour(), "warm");
        assert_eq!(l.lights[0].light.colour, [1.0, 0.7, 0.4]);
        assert_eq!(l.cycle_colour(), "red");
        l.fire(false, [0.0; 3], [1.0, 0.0, 0.0], None).unwrap();
        l.drop_at(false, [0.0; 3], [1.0, 0.0, 0.0], None).unwrap();
        assert_eq!(l.lights[1].light.colour, [1.0, 0.0, 0.0]);
        assert_eq!(l.lights[2].light.colour, [1.0, 0.0, 0.0]);
        // A light in flight takes a recolour too.
        l.fire(false, [0.0; 3], [1.0, 0.0, 0.0], None).unwrap();
        assert_eq!(l.cycle_colour(), "green");
        assert!(l.lights[3].flight.is_some());
        assert_eq!(l.lights[3].light.colour, [0.0, 1.0, 0.0]);
        assert_eq!(l.lights[2].light.colour, [1.0, 0.0, 0.0]);
        assert_eq!(l.cycle_colour(), "blue");
        assert_eq!(l.cycle_colour(), "white");
        let mut row = String::new();
        l.write_row(false, &mut row);
        assert_eq!(row, "lights 4/16 fly 2 over 0 colour white");
    }

    #[test]
    fn five_flips_the_newest_lights_bounce_flag() {
        let mut l = TestLights::default();
        assert_eq!(l.toggle_bounce(), None);
        l.drop_at(false, [0.0; 3], [1.0, 0.0, 0.0], None).unwrap();
        l.fire(false, [0.0; 3], [1.0, 0.0, 0.0], None).unwrap();
        assert_eq!(l.toggle_bounce(), Some(true));
        assert_eq!(l.lights[1].light.flags & Light::BOUNCE, Light::BOUNCE);
        assert_eq!(l.lights[0].light.flags & Light::BOUNCE, 0);
        let mut row = String::new();
        l.write_row(false, &mut row);
        assert_eq!(row, "lights 2/16 bounce * fly 1 over 0 colour white");
        assert_eq!(l.toggle_bounce(), Some(false));
        assert_eq!(l.bouncing(), 0);
    }

    #[test]
    fn the_ring_holds_n_lights_at_its_radius() {
        let r = TestLights::ring(4, [10.0, 10.0, 5.0]);
        assert_eq!(r.lights.len(), 4);
        for l in &r.lights {
            let p = l.light.pos;
            let d = ((p[0] - 10.0).powi(2) + (p[1] - 10.0).powi(2)).sqrt();
            assert!((d - RING_RADIUS).abs() < 1e-3);
            assert_eq!(p[2], 5.0);
            assert_eq!(l.light.intensity, RING_INTENSITY);
        }
    }
}
