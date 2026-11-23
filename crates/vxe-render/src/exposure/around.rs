//! The floor's sky share smoothed over time before it reaches the exposure (ADR-0081).

/// The sky share over the camera's floor as the exposure takes it: the per-frame answer of
/// `sky_over_floor` smoothed at a time constant `tau`, `f += (t - f) (1 - exp(-dt / tau))`
/// (`tau` 0 jumps). The first answer after none was ever had is taken as is. A frame with no
/// answer holds the last value; once more than `tau` seconds pass with no answer the share
/// reads absent, and the next answer is smoothed from the held value.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SkyAround {
    /// The smoothed share; meaningful once an answer arrived.
    value: f32,
    /// An answer arrived at some frame.
    had: bool,
    /// Seconds since the last answer.
    since: f64,
}

impl SkyAround {
    /// One frame of `dt` seconds with this frame's answer `answer` at time constant `tau`:
    /// the smoothed share, `None` while absent.
    pub fn step(&mut self, answer: Option<f32>, dt: f32, tau: f32) -> Option<f32> {
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        match answer {
            Some(t) => {
                if !self.had || tau <= 0.0 {
                    self.value = t;
                } else {
                    let k = 1.0 - (-f64::from(dt) / f64::from(tau)).exp();
                    self.value += ((f64::from(t) - f64::from(self.value)) * k) as f32;
                }
                self.had = true;
                self.since = 0.0;
            }
            None => self.since += f64::from(dt),
        }
        self.read(tau)
    }

    /// The share as last stepped under time constant `tau`, `None` while absent: no answer
    /// yet, or none for more than `tau` seconds (plus a microsecond for the frames' summed
    /// steps).
    pub fn read(&self, tau: f32) -> Option<f32> {
        (self.had && self.since <= f64::from(tau.max(0.0)) + 1e-6).then_some(self.value)
    }

    /// Forgets every answer (a new world or a teleport).
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plan 0097 Phase 2 bar 2: a step of the floor's share from 0 to 0.5 at `dt` 1/60 and
    /// `tau` 1 rises monotonically and reaches `0.5 (1 - 1/e)` within 1/256 at frame 60; with
    /// no answer the share holds for 60 frames and reads absent at frame 61; `tau` 0 jumps.
    #[test]
    fn the_floor_share_settles_at_tau() {
        let dt = 1.0 / 60.0;
        let mut s = SkyAround::default();
        assert_eq!(s.read(1.0), None);
        s.step(Some(0.0), dt, 1.0);
        assert_eq!(s.read(1.0), Some(0.0));
        let mut last = 0.0;
        for k in 1..=60 {
            s.step(Some(0.5), dt, 1.0);
            let v = s.read(1.0).expect("present");
            assert!(v > last, "frame {k}: {v} after {last}");
            last = v;
        }
        let want = 0.5 * (1.0 - (-1.0f32).exp());
        assert!((last - want).abs() <= 1.0 / 256.0, "{last} against {want}");
        // No answer: held for 60 frames, absent at 61.
        for k in 1..=60 {
            s.step(None, dt, 1.0);
            assert_eq!(s.read(1.0), Some(last), "frame {k}");
        }
        s.step(None, dt, 1.0);
        assert_eq!(s.read(1.0), None);
        // The next answer is smoothed from the held value.
        s.step(Some(0.5), dt, 1.0);
        let v = s.read(1.0).expect("present");
        assert!(v > last && v < 0.5, "{v}");
        // `tau` 0 jumps, and reads absent at the first frame with no answer.
        let mut j = SkyAround::default();
        j.step(Some(0.1), dt, 0.0);
        j.step(Some(0.7), dt, 0.0);
        assert_eq!(j.read(0.0), Some(0.7));
        j.step(None, dt, 0.0);
        assert_eq!(j.read(0.0), None);
    }
}
