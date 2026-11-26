//! The teardown's lines on the edit's frames of the live queue (a reading): the bake call by
//! the queue's state, and with the bake thread on the wait for the dig's step by its parts.

use super::{DigFrame, p50_p99_max};

/// The edit's frames by the queue's state (a reading, the teardown's `bake dig call:` lines):
/// per state (while chunks in view wait for their tier, and after), the dig frames in the run's
/// mode and their bake call's p50 / p99 / max in ms (lockstep's step, or the thread's frame
/// call with its wait), beside the thread's wait.
pub(crate) fn dig_call_lines(frames: &[DigFrame]) -> Vec<String> {
    [(true, "while in view waits"), (false, "after")]
        .iter()
        .filter_map(|&(waiting, name)| {
            let f: Vec<&DigFrame> = frames.iter().filter(|d| d.waiting == waiting).collect();
            let first = f.first()?;
            let call = p50_p99_max(f.iter().map(|d| d.call_ms));
            let wait = p50_p99_max(f.iter().map(|d| d.wait));
            Some(format!(
                "bake dig call: {name}, {} dig frames ({}), call p50/p99/max {:.3}/{:.3}/{:.3} ms, wait p50/p99/max {:.3}/{:.3}/{:.3} ms",
                f.len(),
                if first.thread { "thread" } else { "lockstep" },
                call[0],
                call[1],
                call[2],
                wait[0],
                wait[1],
                wait[2],
            ))
        })
        .collect()
}

/// The edit's frames with the bake thread on (a reading, the teardown's `bake thread dig wait:`
/// lines, one per state while chunks in view wait for their tier and after): the dig frames, the
/// wait for the dig's step p50 / p99 / max in ms, the wait less its rest p50 / p99 (per frame:
/// its parts' span less its own rest) and its parts' p50 / p99 (the rest of a step in
/// progress, the pick-up, the step to its `boxes` stage's end, the step's rest, the snapshot,
/// the hand-back), the frame's issue and apply laps' p50 / p99, the dig frames by the submits
/// they issued (1, 2, 3 or more), and the frames that applied bytes over the frame's cap (the
/// edit's list takes every list made before it) with those bytes' p50 and max.
pub(crate) fn dig_wait_lines(frames: &[DigFrame]) -> Vec<String> {
    const PARTS: [&str; 6] = [
        "rest",
        "pickup",
        "boxes",
        "step_rest",
        "snapshot",
        "handback",
    ];
    [(true, "while in view waits"), (false, "after")]
        .iter()
        .filter_map(|&(waiting, name)| {
            let f: Vec<&DigFrame> = frames
                .iter()
                .filter(|d| d.thread && d.waiting == waiting)
                .collect();
            if f.is_empty() {
                return None;
            }
            let wait = p50_p99_max(f.iter().map(|d| d.wait));
            let pair = |v: [f64; 3]| format!("{:.3}/{:.3}", v[0], v[1]);
            let less_rest = pair(p50_p99_max(f.iter().map(|d| d.span - d.parts[0])));
            let parts: Vec<String> = PARTS
                .iter()
                .enumerate()
                .map(|(k, n)| format!("{n} {}", pair(p50_p99_max(f.iter().map(|d| d.parts[k])))))
                .collect();
            let by = |lo: u64, hi: u64| f.iter().filter(|d| d.submits >= lo && d.submits <= hi).count();
            let over = p50_p99_max(f.iter().filter(|d| d.over > 0).map(|d| d.over as f64));
            Some(format!(
                "bake thread dig wait: {name}, {} dig frames, wait p50 {:.3} p99 {:.3} max {:.3} ms, less rest p50/p99 {less_rest} ms, parts p50/p99 {} ms, issue {} apply {} ms, submits 1 {} 2 {} 3+ {}, over the cap {} frames p50 {:.0} max {:.0} bytes",
                f.len(),
                wait[0],
                wait[1],
                wait[2],
                parts.join(" "),
                pair(p50_p99_max(f.iter().map(|d| d.issue_ms))),
                pair(p50_p99_max(f.iter().map(|d| d.apply_ms))),
                by(1, 1),
                by(2, 2),
                by(3, u64::MAX),
                f.iter().filter(|d| d.over > 0).count(),
                if over[0].is_nan() { 0.0 } else { over[0] },
                if over[2].is_nan() { 0.0 } else { over[2] },
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Done when (3) of Plan 0099 Phase 3g. Three made dig frames with the bake thread on, while
    /// chunks in view wait: spans 30, 32 and 40 ms, rests 1, 20 and 2. The wait less its rest
    /// per frame is 29, 12 and 38 (p50 29); the difference of the two p50s is 32 - 2 = 30. The
    /// line prints the per-frame figure.
    #[test]
    fn the_dig_wait_line_reads_the_wait_less_its_rest_per_frame() {
        let frame = |span: f64, rest: f64| DigFrame {
            thread: true,
            waiting: true,
            call_ms: span,
            wait: span,
            parts: [rest, 0.0, 1.0, span - rest - 1.0, 0.0, 0.0],
            span,
            issue_ms: 0.1,
            apply_ms: 0.2,
            over: 0,
            submits: 1,
        };
        let frames = [frame(30.0, 1.0), frame(32.0, 20.0), frame(40.0, 2.0)];
        let lines = dig_wait_lines(&frames);
        assert_eq!(lines.len(), 1, "one state: {lines:?}");
        let line = &lines[0];
        assert!(line.contains("wait p50 32.000 "), "{line}");
        assert!(line.contains("parts p50/p99 rest 2.000/"), "{line}");
        assert!(line.contains(", less rest p50/p99 29.000/"), "{line}");
        assert!(!line.contains("less rest p50/p99 30.000/"), "{line}");
    }
}
