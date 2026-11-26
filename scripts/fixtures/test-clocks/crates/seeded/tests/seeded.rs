//! Seeded for check-test-clocks.mjs: rules 1 and 2, and the silences beside them.
//! A comment naming std::thread::sleep(..) or t.elapsed() < LIMIT is not a use.

use std::time::{Duration, Instant};

const LIMIT: Duration = Duration::from_secs(1);

/// Rule 1: a frame count paced by a sleep.
#[test]
fn paced_by_a_sleep() {
    let mut frames = 0;
    while frames < 10 {
        frames += 1;
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn bound_by_a_constant() {
    let t = Instant::now();
    assert!(t.elapsed() < LIMIT, "within a second");
}

#[test]
fn bound_through_a_name() {
    let ms = Instant::now().elapsed().as_secs_f64() * 1e3;
    assert!(ms < 1000.0, "under 1000 ms");
}

/// Silent: a duration against zero, and against a value that is not a constant.
#[test]
fn compared_with_zero_or_a_value() {
    let t = Instant::now();
    let other = t.elapsed();
    assert!(t.elapsed().as_secs_f64() >= 0.0);
    assert!(t.elapsed() >= other);
}

/// Silent: the allow-list's live row names this site.
#[test]
fn the_claims_own_delay() {
    std::thread::sleep(Duration::from_millis(100));
}

/// Silent: tokens inside string literals.
#[test]
fn tokens_in_strings() {
    println!("std::thread::sleep(1) and t.elapsed() < LIMIT");
}

/// Rule 1 in a soak test, and in a helper only it calls: no tier mark is exempt.
#[test]
#[ignore = "soak"]
fn a_long_series() {
    paced_helper();
    std::thread::sleep(Duration::from_millis(5));
}

fn paced_helper() {
    std::thread::yield_now();
}

mod reading {
    use std::time::Duration;

    /// Silent: a reading may time anything.
    #[test]
    #[ignore = "reading"]
    fn a_pace_reading() {
        std::thread::sleep(Duration::from_millis(5));
    }
}
