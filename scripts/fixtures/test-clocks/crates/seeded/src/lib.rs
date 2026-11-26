//! Seeded for check-test-clocks.mjs: a `#[cfg(test)]` module (rule 2 by a clock function and
//! an `if` that panics) beside production code that may sleep.

use std::time::Duration;

/// Silent: production code, not a test.
pub fn back_off() {
    std::thread::sleep(Duration::from_millis(1));
}

/// Silent: production code comparing a duration.
pub fn late(t: std::time::Instant) -> bool {
    t.elapsed().as_secs_f64() > 2.0
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    fn seconds_since(t: Instant) -> f64 {
        t.elapsed().as_secs_f64()
    }

    /// Rule 2: a duration past a constant before a panic, through a clock function.
    #[test]
    fn panics_past_a_bound() {
        let t = Instant::now();
        super::back_off();
        // The bound below is the finding.
        if seconds_since(t) > 2.0 {
            panic!("over two seconds");
        }
    }
}
