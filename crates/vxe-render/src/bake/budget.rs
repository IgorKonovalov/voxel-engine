//! The live queue's time budget and its slice constants.

/// The live queue's slice by default, in milliseconds of a frame.
pub const DEFAULT_BAKE_SLICE_MS: f64 = 1.5;

/// The least slice a per-second budget gives a step, in milliseconds: what a step with no
/// interval yet (the first) or a very short one bakes.
pub(crate) const BAKE_SLICE_FLOOR_MS: f64 = 0.25;

/// The live queue's bake budget (ADR-0052): a slice a frame, or milliseconds of GPU time a
/// second of wall time spent in slices sized by the frame's interval.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Budget {
    /// Milliseconds of bake a frame, whatever the frame's length.
    PerFrame { ms: f64 },
    /// Milliseconds of bake a second: a step's slice is `ms_per_s` times the last frame's
    /// interval in seconds, at most `cap` of that interval and at least
    /// [`BAKE_SLICE_FLOOR_MS`].
    PerSecond { ms_per_s: f64, cap: f64 },
}

impl Default for Budget {
    fn default() -> Self {
        Budget::PerFrame {
            ms: DEFAULT_BAKE_SLICE_MS,
        }
    }
}

impl Budget {
    /// The step's slice in milliseconds after a frame of `interval_s` seconds.
    pub fn slice_ms(&self, interval_s: f64) -> f64 {
        match *self {
            Budget::PerFrame { ms } => ms,
            Budget::PerSecond { ms_per_s, cap } => {
                let interval_ms = interval_s.max(0.0) * 1e3;
                (ms_per_s * interval_s.max(0.0))
                    .min(cap * interval_ms)
                    .max(BAKE_SLICE_FLOOR_MS)
            }
        }
    }

    /// Parses `frame:MS` or `second:MS,CAP` (the form [`std::fmt::Display`] writes).
    pub fn parse(s: &str) -> Result<Self, String> {
        let num = |v: &str| {
            v.trim()
                .parse::<f64>()
                .ok()
                .filter(|x| x.is_finite() && *x >= 0.0)
                .ok_or_else(|| format!("bake budget: {v:?} is not a non-negative number"))
        };
        match s.split_once(':') {
            Some(("frame", ms)) => Ok(Budget::PerFrame { ms: num(ms)? }),
            Some(("second", rest)) => {
                let (ms, cap) = rest
                    .split_once(',')
                    .ok_or_else(|| format!("bake budget: {s:?} wants second:MS,CAP"))?;
                let cap = num(cap)?;
                if cap <= 0.0 || cap > 1.0 {
                    return Err(format!("bake budget: the cap {cap} is not in (0, 1]"));
                }
                Ok(Budget::PerSecond {
                    ms_per_s: num(ms)?,
                    cap,
                })
            }
            _ => Err(format!(
                "bake budget: {s:?} is neither frame:MS nor second:MS,CAP"
            )),
        }
    }
}

impl std::fmt::Display for Budget {
    /// `frame:1.5` or `second:150,0.25`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Budget::PerFrame { ms } => write!(f, "frame:{ms}"),
            Budget::PerSecond { ms_per_s, cap } => write!(f, "second:{ms_per_s},{cap}"),
        }
    }
}

/// Milliseconds a frame the live queue's slice grows by while a dig's bricks wait (ADR-0046's
/// edit cap, by time on the slice's own ns a sample).
pub const DEFAULT_BAKE_EDIT_MS: f64 = 3.0;

#[cfg(test)]
mod tests {
    use super::*;

    /// At 150 ms a second a 10 ms interval gives a 1.5 ms slice and a 66 ms one 9.9 ms; at 300
    /// ms a second and a cap of 0.25 the 66 ms interval gives the cap's 16.5 ms; a 0 interval
    /// gives the floor. A per-frame budget ignores the interval; both forms parse back from
    /// their text.
    #[test]
    fn a_per_second_budget_sizes_the_slice_by_the_interval() {
        let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
        let s150 = Budget::PerSecond {
            ms_per_s: 150.0,
            cap: 0.25,
        };
        assert!(near(s150.slice_ms(0.010), 1.5), "{}", s150.slice_ms(0.010));
        assert!(near(s150.slice_ms(0.066), 9.9), "{}", s150.slice_ms(0.066));
        let s300 = Budget::PerSecond {
            ms_per_s: 300.0,
            cap: 0.25,
        };
        assert!(near(s300.slice_ms(0.066), 16.5), "{}", s300.slice_ms(0.066));
        assert_eq!(s150.slice_ms(0.0), BAKE_SLICE_FLOOR_MS);
        assert_eq!(s300.slice_ms(0.0), BAKE_SLICE_FLOOR_MS);
        let f = Budget::PerFrame { ms: 1.5 };
        assert_eq!(f.slice_ms(0.0), 1.5);
        assert_eq!(f.slice_ms(0.066), 1.5);
        assert_eq!(f.to_string(), "frame:1.5");
        assert_eq!(s150.to_string(), "second:150,0.25");
        assert_eq!(Budget::parse("frame:1.5"), Ok(f));
        assert_eq!(Budget::parse("second:150,0.25"), Ok(s150));
        assert!(Budget::parse("second:150").is_err());
        assert!(Budget::parse("second:150,0").is_err());
        assert!(Budget::parse("minute:3").is_err());
    }
}
