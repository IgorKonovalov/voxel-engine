//! The light-stability rules of ADR-0076 as plain functions over per-frame display shots
//! (rgba8, one shot per frame, every shot the same size). Rule 1: a still pixel changes by at
//! most `T_STILL` between frames. Rule 2: the frame at a return to a settled pose is within
//! `T_RETURN` of the settled image. Rule 3: a pixel whose value changes takes one monotonic fade
//! of `F` frames, classed by [`trace_class`]. Rule 4: outside an event's region rule 1 holds
//! across the event's frames. No baseline and no ratchet live here; a pop counter wraps these.
//!
//! A pixel's level is the largest of its three colour channels' values; a change between two
//! shots is the largest of the three channels' absolute differences. The thresholds are starting
//! guesses (Plan 0090 calibrates them).
#![allow(dead_code, reason = "shared across test binaries; each uses a subset")]

/// Rule 1's line: a still pixel's change between frames, in levels of 255.
pub const T_STILL: u8 = 4;
/// Rule 3's band around a fade's old and new values, in levels.
pub const T_FADE: u8 = 4;
/// Rule 2's line: the return frame against the settled image, in levels.
pub const T_RETURN: u8 = 8;
/// Rule 3's fade length in frames (ADR-0046's `FADE`).
pub const F: usize = 16;
/// The frames before an event whose mean is a trace's `old`, and the frames after `F + 8`
/// whose mean is its `new`.
pub const MEAN_FRAMES: usize = 8;

/// The change of pixel `i` between two rgba8 shots: the largest channel difference.
pub fn change(a: &[u8], b: &[u8], i: usize) -> u8 {
    (0..3)
        .map(|c| a[i * 4 + c].abs_diff(b[i * 4 + c]))
        .max()
        .unwrap_or(0)
}

/// Pixel `i`'s level in an rgba8 shot: its largest colour channel.
pub fn level(shot: &[u8], i: usize) -> u8 {
    shot[i * 4..i * 4 + 3].iter().copied().max().unwrap_or(0)
}

/// Rule 1: the pixels (inside `region` where one is given, one flag per pixel) whose change
/// between `a` and `b` exceeds `t_still`.
pub fn still_count(a: &[u8], b: &[u8], t_still: u8, region: Option<&[bool]>) -> usize {
    let n = a.len() / 4;
    (0..n)
        .filter(|&i| region.is_none_or(|r| r[i]) && change(a, b, i) > t_still)
        .count()
}

/// Rule 2: the pixels of the return frame more than `t_return` from the settled image.
pub fn return_count(settled: &[u8], returned: &[u8], t_return: u8) -> usize {
    still_count(settled, returned, t_return, None)
}

/// Rule 4: across an event's consecutive shots, the pixels outside the event's region (`inside`
/// false) whose change exceeds `t_still` on any pair, counted once each.
pub fn outside_count(shots: &[&[u8]], t_still: u8, inside: &[bool]) -> usize {
    let n = inside.len();
    (0..n)
        .filter(|&i| !inside[i] && shots.windows(2).any(|w| change(w[0], w[1], i) > t_still))
        .count()
}

/// Rule 3's classes of one pixel's trace across an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraceClass {
    /// Inside the band, reaching `new` by frame `F`, staying there: the rule holds.
    Fade,
    /// A frame above `max(old, new) + T_FADE`.
    Flash,
    /// A frame below `min(old, new) - T_FADE`.
    Dip,
    /// Not within `T_FADE` of `new` by frame `F` after the event.
    Slow,
    /// After first reaching `new`, a later frame more than `T_FADE` from it.
    Overshoot,
}

impl TraceClass {
    pub const ALL: [TraceClass; 5] = [
        TraceClass::Fade,
        TraceClass::Flash,
        TraceClass::Dip,
        TraceClass::Slow,
        TraceClass::Overshoot,
    ];

    /// The class's index in [`TraceClass::ALL`].
    pub fn index(self) -> usize {
        self as usize
    }
}

fn mean(v: &[u8]) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.iter().map(|&x| f32::from(x)).sum::<f32>() / v.len() as f32
}

/// Rule 3's class of one pixel's trace. `before` is the frames before the event (their mean is
/// `old`), `event` the frames from the event's frame (index 0) up to frame `f + 8` exclusive,
/// `after` frames `f + 8` to `f + 16` (their mean is `new`). The band is `[min(old, new) -
/// t_fade, max(old, new) + t_fade]` over `event` and `after`; a frame above it is `Flash`, below
/// it `Dip` (the first such frame names the class); a trace first within `t_fade` of `new` past
/// frame `f` is `Slow`; a trace that leaves `t_fade` of `new` after first reaching it is
/// `Overshoot`.
pub fn trace_class(before: &[u8], event: &[u8], after: &[u8], t_fade: u8, f: usize) -> TraceClass {
    let (old, new) = (mean(before), mean(after));
    let t = f32::from(t_fade);
    let (lo, hi) = (old.min(new) - t, old.max(new) + t);
    let all = || event.iter().chain(after.iter()).map(|&x| f32::from(x));
    for v in all() {
        if v > hi {
            return TraceClass::Flash;
        }
        if v < lo {
            return TraceClass::Dip;
        }
    }
    let Some(reach) = all().position(|v| (v - new).abs() <= t) else {
        return TraceClass::Slow;
    };
    if reach > f {
        return TraceClass::Slow;
    }
    if all().skip(reach).any(|v| (v - new).abs() > t) {
        return TraceClass::Overshoot;
    }
    TraceClass::Fade
}

/// Rule 3 over a region: `shots` run from `MEAN_FRAMES` frames before the event to `f + 16`
/// frames after it (the event's frame at index `MEAN_FRAMES`); per pixel inside `region`, its
/// level's trace classed, counted per class in [`TraceClass::ALL`]'s order.
pub fn trace_classes(shots: &[&[u8]], region: &[bool], t_fade: u8, f: usize) -> [usize; 5] {
    let need = MEAN_FRAMES + f + 16;
    assert!(
        shots.len() >= need,
        "rule 3 needs {need} shots ({} given)",
        shots.len()
    );
    let mut counts = [0usize; 5];
    let mut trace = vec![0u8; need];
    for (i, _) in region.iter().enumerate().filter(|(_, r)| **r) {
        for (k, s) in shots[..need].iter().enumerate() {
            trace[k] = level(s, i);
        }
        let (before, rest) = trace.split_at(MEAN_FRAMES);
        let (event, after) = rest.split_at(f + 8);
        counts[trace_class(before, event, after, t_fade, f).index()] += 1;
    }
    counts
}
