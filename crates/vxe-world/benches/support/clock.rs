//! The benches' wall clock: the one allowed `Instant::now` of a bench. The code under test
//! reads no clock; a bench times it from outside. Other crates' benches take this file by
//! `#[path]`.

use std::time::{Duration, Instant};

/// The wall clock now.
#[allow(
    clippy::disallowed_methods,
    reason = "a bench timing code that reads no clock itself"
)]
pub fn now() -> Instant {
    Instant::now()
}

/// `f`'s output and the wall time it took.
#[allow(
    clippy::disallowed_methods,
    reason = "a bench timing code that reads no clock itself"
)]
pub fn timed<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let t = now();
    let out = f();
    (out, t.elapsed())
}
