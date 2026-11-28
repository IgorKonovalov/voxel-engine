//! The `slow frame:` block printed at the exit: every frame whose wall time is over
//! [`SLOW_MS`], with that frame's own stages joined to it, so a tail is attributed frame by
//! frame rather than from percentiles of separate series.
//!
//! A frame's wall is the `dt` the shell hands the next frame: the span from this frame's start
//! to the next one's, which holds this frame's work, its present and the next frame's acquire
//! ([`svet_app::ShellTimes`]). So a frame's record is filled while it runs and closed at the
//! start of the next. The stages are clock reads already taken on the render thread, plus one
//! pair around the live queue's step, the encode and the submit, and one at the frame's end;
//! on Linux one `getrusage` call a frame reads the render thread's counters. Nothing here
//! allocates after the start: the kept frames' buffer is made once.

use crate::app::ms_since;
use crate::now;
use crate::run::Run;
use std::time::Instant;
use svet_app::{Gpu, ShellTimes};
use svet_render::{SyncStats, wgpu};

/// A frame is slow when its wall time is over this, in ms: two frames at 60 Hz.
pub(crate) const SLOW_MS: f64 = 33.3;
/// The summary's second count: frames over this wall time, in ms.
pub(crate) const VERY_SLOW_MS: f64 = 100.0;
/// The slow frames whose lines are kept and printed, the slowest first in, printed in frame
/// order.
pub(crate) const KEPT: usize = 64;

/// The mirror sync's stage names in [`FrameStages::sync_parts`]' order (`SyncStats`' `_us`
/// fields): `place` and `note` are `light place` and `light note`.
pub(crate) const SYNC_PARTS: [&str; 8] = [
    "select", "size", "pack", "place", "note", "copy", "alloc", "lock",
];
/// The bake thread's render laps in [`FrameStages::laps`]' order (`Frame::bake_render_laps`
/// without its total).
pub(crate) const LAPS: [&str; 7] = ["build", "take", "issue", "apply", "collect", "send", "wait"];

/// One frame's stages in ms, on the render thread unless named.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct FrameStages {
    pub(crate) frame: u32,
    /// The path's clock at the frame (`--path`), else seconds since the run's start.
    pub(crate) at_s: f64,
    pub(crate) wall: f64,
    /// The shell's acquire of the next frame's surface texture, inside this frame's wall.
    pub(crate) acquire: f64,
    pub(crate) update: f64,
    pub(crate) poll: f64,
    /// Chunks the poll inserted.
    pub(crate) chunks: u32,
    /// Chunks the poll's events evicted and demoted (the mirror's drops).
    pub(crate) evicted: u32,
    pub(crate) demoted: u32,
    /// The poll's events applied to the mirror and the frame (`events`).
    pub(crate) events: f64,
    pub(crate) sync: f64,
    pub(crate) sync_parts: [f64; 8],
    /// Bricks the sync packed on the render thread.
    pub(crate) bricks: u64,
    /// The live queue's whole step on the render thread (`Run::step_bake_queue`).
    pub(crate) queue: f64,
    /// The bake thread's render laps in the frame, when it ran one.
    pub(crate) laps: Option<[f64; 7]>,
    /// The write lists the frame's step applied: the store's writes, the `write_buffer` calls
    /// they took, their bytes, the lists and of them the whole ones (units over the maker's
    /// cap; `Frame::bake_lists_applied`' difference across the step).
    pub(crate) applied: [u64; 5],
    /// The lists and bytes still waiting after the step's apply.
    pub(crate) waiting: [u64; 2],
    /// The frame's start to the encode, less the stages above: the keys, the path, the audio,
    /// the HUD's rows and the frame's settings.
    pub(crate) prepare: f64,
    /// The params to the command buffer's finish: the field pass, the frame's passes, the
    /// present pass and the HUD recorded.
    pub(crate) encode: f64,
    pub(crate) submit: f64,
    /// The submit to the frame's end: the exposure's and the timers' read-backs, the
    /// readings' bookkeeping, the stills.
    pub(crate) after: f64,
    /// The shell's present of this frame (headless, the wait for the device's idle).
    pub(crate) present: f64,
    /// The render thread's involuntary context switches, minor and major page faults over the
    /// frame's wall; `None` where they are not read.
    pub(crate) counters: Option<[u64; 3]>,
}

impl FrameStages {
    /// The named stages' sum: everything but `rest`.
    fn named(&self) -> f64 {
        self.acquire
            + self.update
            + self.poll
            + self.events
            + self.sync
            + self.queue
            + self.prepare
            + self.encode
            + self.submit
            + self.after
            + self.present
    }

    /// The wall less every named stage: the shell's loop outside its timed spans, and any
    /// span no timer brackets.
    pub(crate) fn rest(&self) -> f64 {
        self.wall - self.named()
    }

    fn sync_other(&self) -> f64 {
        self.sync - self.sync_parts.iter().sum::<f64>()
    }

    fn queue_other(&self) -> f64 {
        self.queue - self.laps.map_or(0.0, |l| l.iter().sum())
    }

    /// The leaf stages, [`LEAVES`]' order: a stage with parts counts by its parts and the
    /// share of it they do not cover.
    fn leaves(&self) -> [f64; LEAVES.len()] {
        let s = &self.sync_parts;
        let l = self.laps.unwrap_or([0.0; 7]);
        [
            self.acquire,
            self.update,
            self.poll,
            self.events,
            s[0],
            s[1],
            s[2],
            s[3],
            s[4],
            s[5],
            s[6],
            s[7],
            self.sync_other(),
            l[0],
            l[1],
            l[2],
            l[3],
            l[4],
            l[5],
            l[6],
            self.queue_other(),
            self.prepare,
            self.encode,
            self.submit,
            self.after,
            self.present,
            self.rest(),
        ]
    }

    /// The frame's `slow frame:` line.
    pub(crate) fn line(&self) -> String {
        let parts = |names: &[&str], v: &[f64]| {
            names
                .iter()
                .zip(v)
                .map(|(n, ms)| format!("{n} {ms:.3}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let [writes, calls, bytes, lists, whole] = self.applied;
        let [waiting, waiting_bytes] = self.waiting;
        let laps = self.laps.map_or_else(
            || "no laps".to_owned(),
            |l| {
                // `apply` carries the writes it applied, the calls they took, their bytes, the
                // lists and the whole ones, and what waits after it.
                let k = LAPS.iter().position(|n| *n == "apply").unwrap_or(LAPS.len());
                format!(
                    "{}, apply {:.3} ({writes} writes, {calls} calls, {bytes} bytes; {lists} lists, {whole} whole; waiting {waiting} lists, {waiting_bytes} bytes), {}, other {:.3}",
                    parts(&LAPS[..k], &l[..k]),
                    l[k],
                    parts(&LAPS[k + 1..], &l[k + 1..]),
                    self.queue_other()
                )
            },
        );
        let counters = self.counters.map_or_else(
            || "counters n/a".to_owned(),
            |[s, minor, major]| format!("switches {s}, faults {minor} minor {major} major"),
        );
        format!(
            "slow frame: {} at {:.1} s, wall {:.3} ms: acquire {:.3}, update {:.3}, poll {:.3} ({} chunks, {} evicted, {} demoted), events {:.3}, sync {:.3} ({}, other {:.3}; {} bricks), queue {:.3} ({laps}), prepare {:.3}, encode {:.3}, submit {:.3}, after {:.3}, present {:.3}, rest {:.3}; {counters}",
            self.frame,
            self.at_s,
            self.wall,
            self.acquire,
            self.update,
            self.poll,
            self.chunks,
            self.evicted,
            self.demoted,
            self.events,
            self.sync,
            parts(&SYNC_PARTS, &self.sync_parts),
            self.sync_other(),
            self.bricks,
            self.queue,
            self.prepare,
            self.encode,
            self.submit,
            self.after,
            self.present,
            self.rest(),
        )
    }
}

/// The leaf stages the summary compares, in the line's order.
pub(crate) const LEAVES: [&str; 27] = [
    "acquire",
    "update",
    "poll",
    "events",
    "select",
    "size",
    "pack",
    "place",
    "note",
    "copy",
    "alloc",
    "lock",
    "sync other",
    "build",
    "take",
    "issue",
    "apply",
    "collect",
    "send",
    "wait",
    "queue other",
    "prepare",
    "encode",
    "submit",
    "after",
    "present",
    "rest",
];

/// The run's frames by their stages: the frame in progress, the slowest [`KEPT`] slow frames,
/// and sums over every slow frame and every frame for the summary.
#[derive(Debug)]
pub(crate) struct SlowFrames {
    /// The frame being filled; `None` before the first world frame.
    pub(crate) pending: Option<FrameStages>,
    /// The shell's spans handed before the frame in progress.
    pub(crate) shell: ShellTimes,
    kept: Vec<FrameStages>,
    slow: u64,
    very_slow: u64,
    /// Slow frames whose `rest` is over a fifth of their wall.
    rest_over_fifth: u64,
    /// Per leaf, the slow frames it is the largest in, and its sum over every slow frame.
    largest: [u64; LEAVES.len()],
    sums: [f64; LEAVES.len()],
    /// The counters summed over the slow frames and over every frame, with the frames read.
    slow_counters: ([u64; 3], u64),
    all_counters: ([u64; 3], u64),
    /// The thread's counters at the last frame's close.
    last_counters: Option<[u64; 3]>,
    /// The frame's last mark ([`SlowFrames::mark`]): the span to the next one is a stage.
    mark: Option<Instant>,
}

impl Default for SlowFrames {
    fn default() -> Self {
        Self {
            pending: None,
            shell: ShellTimes::default(),
            kept: Vec::with_capacity(KEPT),
            slow: 0,
            very_slow: 0,
            rest_over_fifth: 0,
            largest: [0; LEAVES.len()],
            sums: [0.0; LEAVES.len()],
            slow_counters: ([0; 3], 0),
            all_counters: ([0; 3], 0),
            last_counters: None,
            mark: None,
        }
    }
}

impl SlowFrames {
    /// Closes the frame in progress with its `wall` (the `dt` the next frame was handed),
    /// the shell's spans handed with it and the thread's counters now, and files it; then
    /// opens frame `frame` at `at_s`, begun at `start`.
    pub(crate) fn next_frame(&mut self, wall_ms: f64, frame: u32, at_s: f64, start: Instant) {
        let now = thread_counters();
        if let Some(mut f) = self.pending.take() {
            f.wall = wall_ms;
            f.acquire = self.shell.acquire_ms;
            f.present = self.shell.present_ms;
            f.counters = match (self.last_counters, now) {
                (Some(a), Some(b)) => Some(std::array::from_fn(|k| b[k].saturating_sub(a[k]))),
                _ => None,
            };
            self.file(f);
        }
        self.last_counters = now;
        self.mark = Some(start);
        self.pending = Some(FrameStages {
            frame,
            at_s,
            ..FrameStages::default()
        });
    }

    /// Marks the frame's next stage boundary, each the end of the span from the one before:
    /// the encode's start (its `prepare` is the span from the frame's start less the stages
    /// already filed), the submit's, the submit's end, and the frame's end. One clock read.
    pub(crate) fn mark(&mut self, at: Mark) {
        let t = now();
        let since = self.mark.map_or(0.0, |m| (t - m).as_secs_f64() * 1e3);
        self.mark = Some(t);
        let Some(p) = self.pending.as_mut() else {
            return;
        };
        match at {
            Mark::Encode => {
                p.prepare = since - (p.update + p.poll + p.events + p.sync + p.queue);
            }
            Mark::Submit => p.encode = since,
            Mark::Submitted => p.submit = since,
            Mark::End => p.after = since,
        }
    }

    /// Finishes `enc` and submits it on `queue`, the finish filed in the `encode` and the
    /// submit call its own stage.
    pub(crate) fn submit(&mut self, queue: &wgpu::Queue, enc: wgpu::CommandEncoder) {
        let cmd = enc.finish();
        self.mark(Mark::Submit);
        queue.submit([cmd]);
        self.mark(Mark::Submitted);
    }

    /// Files a closed frame: into the sums, and when slow into the counts and the kept lines.
    pub(crate) fn file(&mut self, f: FrameStages) {
        if let Some(c) = f.counters {
            add(&mut self.all_counters, c);
        }
        if f.wall <= SLOW_MS {
            return;
        }
        self.slow += 1;
        self.very_slow += u64::from(f.wall > VERY_SLOW_MS);
        self.rest_over_fifth += u64::from(f.rest() > f.wall / 5.0);
        if let Some(c) = f.counters {
            add(&mut self.slow_counters, c);
        }
        let leaves = f.leaves();
        let top = (0..leaves.len()).fold(0, |m, k| if leaves[k] > leaves[m] { k } else { m });
        self.largest[top] += 1;
        for (s, v) in self.sums.iter_mut().zip(leaves) {
            *s += v;
        }
        if self.kept.len() < KEPT {
            self.kept.push(f);
        } else if let Some(min) =
            (0..KEPT).min_by(|&a, &b| self.kept[a].wall.total_cmp(&self.kept[b].wall))
            && self.kept[min].wall < f.wall
        {
            self.kept[min] = f;
        }
    }

    /// The block: the kept frames' lines in frame order, then the `slow frames:` summary;
    /// empty with no frame closed.
    pub(crate) fn lines(&self) -> Vec<String> {
        let mut kept = self.kept.clone();
        kept.sort_by_key(|f| f.frame);
        let mut out: Vec<String> = kept.iter().map(FrameStages::line).collect();
        if self.all_counters.1 > 0 || self.slow > 0 {
            out.push(self.summary());
        }
        out
    }

    /// The `slow frames:` line: the counts over [`SLOW_MS`] and [`VERY_SLOW_MS`], each leaf
    /// stage's count of slow frames it is the largest in and its mean over the slow frames,
    /// the frames whose `rest` is over a fifth of the wall, and the counters' means over the
    /// slow frames beside every frame's.
    pub(crate) fn summary(&self) -> String {
        let n = self.slow.max(1) as f64;
        let stages = LEAVES
            .iter()
            .enumerate()
            .map(|(k, name)| format!("{name} {} (mean {:.3})", self.largest[k], self.sums[k] / n))
            .collect::<Vec<_>>()
            .join(", ");
        let mean = |(c, frames): ([u64; 3], u64)| {
            let f = frames.max(1) as f64;
            format!(
                "switches {:.2}, faults {:.1} minor {:.2} major",
                c[0] as f64 / f,
                c[1] as f64 / f,
                c[2] as f64 / f
            )
        };
        let counters = if self.all_counters.1 == 0 {
            "counters n/a".to_owned()
        } else {
            format!(
                "a slow frame's {}; any frame's {}",
                mean(self.slow_counters),
                mean(self.all_counters)
            )
        };
        format!(
            "slow frames: {} over {SLOW_MS} ms, {} over {VERY_SLOW_MS} ms; largest stage: {stages}; rest over a fifth of the wall {}; {counters}",
            self.slow, self.very_slow, self.rest_over_fifth
        )
    }
}

/// The frame's stage boundaries [`SlowFrames::mark`] reads the clock at, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mark {
    Encode,
    Submit,
    Submitted,
    End,
}

impl FrameStages {
    /// The mirror sync's time and its stages from its `s`.
    pub(crate) fn sync_from(&mut self, ms: f64, s: Option<&SyncStats>) {
        self.sync = ms;
        if let Some(s) = s {
            let us = [
                s.select_us,
                s.size_us,
                s.pack_us,
                s.light_place_us,
                s.light_note_us,
                s.copy_us,
                s.alloc_us,
                s.lock_us,
            ];
            self.sync_parts = us.map(|v| v as f64 / 1e3);
            self.bricks = s.packed_bricks;
        }
    }
}

impl Run {
    /// [`Run::step_bake_queue`], its whole call timed as the frame's `queue` stage, with the
    /// bake thread's render laps when it ran a frame.
    pub(crate) fn step_bake_queue_timed(
        &mut self,
        gpu: &Gpu,
        enc: wgpu::CommandEncoder,
    ) -> wgpu::CommandEncoder {
        let laps_before = self
            .gfx
            .as_ref()
            .map_or(0, |g| g.frame.bake_render_laps().len());
        let sums = |a: svet_render::ListsApplied| [a.writes, a.calls, a.bytes, a.lists, a.whole];
        let applied_before = self
            .gfx
            .as_ref()
            .map_or([0; 5], |g| sums(g.frame.bake_lists_applied()));
        let t = now();
        let enc = self.step_bake_queue(gpu, enc);
        let ms = ms_since(t);
        let (applied, waiting) = self.gfx.as_ref().map_or(([0; 5], [0; 2]), |g| {
            let a = g.frame.bake_lists_applied();
            let after = sums(a);
            (
                std::array::from_fn(|k| after[k].saturating_sub(applied_before[k])),
                [a.waiting_lists, a.waiting_bytes],
            )
        });
        let laps = self.gfx.as_ref().and_then(|g| {
            let l = g.frame.bake_render_laps();
            (l.len() > laps_before)
                .then(|| l.last().map(|l| std::array::from_fn(|k| f64::from(l[k]))))
                .flatten()
        });
        if let Some(p) = self.hud.run.slow.pending.as_mut() {
            p.queue = ms;
            p.laps = laps;
            p.applied = applied;
            p.waiting = waiting;
        }
        enc
    }

    /// Closes the frame before (its wall is `dt`) and opens this one: frame `self.frame` at
    /// the path's clock, else the run's.
    pub(crate) fn next_slow_frame(&mut self, dt: f32, start: Instant) {
        let at_s = self
            .player
            .as_ref()
            .map_or_else(|| ms_since(self.start) / 1e3, |p| f64::from(p.clock_s));
        let wall = if self.frame > 0 {
            f64::from(dt) * 1e3
        } else {
            0.0
        };
        self.hud.run.slow.next_frame(wall, self.frame, at_s, start);
    }
}

fn add(sum: &mut ([u64; 3], u64), c: [u64; 3]) {
    for (s, v) in sum.0.iter_mut().zip(c) {
        *s += v;
    }
    sum.1 += 1;
}

/// The calling thread's involuntary context switches, minor and major page faults since it
/// started (`getrusage(RUSAGE_THREAD)`): one system call, no allocation, no lock.
#[cfg(all(target_os = "linux", target_pointer_width = "64"))]
fn thread_counters() -> Option<[u64; 3]> {
    /// `struct timeval` on 64-bit Linux.
    #[repr(C)]
    #[derive(Default)]
    struct Timeval {
        sec: i64,
        usec: i64,
    }
    /// `struct rusage` on 64-bit Linux: two `timeval`s, then fourteen `long`s.
    #[repr(C)]
    #[derive(Default)]
    struct Rusage {
        utime: Timeval,
        stime: Timeval,
        rest: [i64; 14],
    }
    const RUSAGE_THREAD: i32 = 1;
    const MINFLT: usize = 4;
    const MAJFLT: usize = 5;
    const NIVCSW: usize = 13;
    unsafe extern "C" {
        fn getrusage(who: i32, usage: *mut Rusage) -> i32;
    }
    let mut u = Rusage::default();
    // SAFETY: `u` is a valid, writable `struct rusage` for the call's whole duration, laid out
    // as the 64-bit Linux ABI's (`repr(C)`), and the call writes nothing else.
    let r = unsafe { getrusage(RUSAGE_THREAD, &raw mut u) };
    let at = |k: usize| u64::try_from(u.rest[k]).unwrap_or(0);
    (r == 0).then(|| [at(NIVCSW), at(MINFLT), at(MAJFLT)])
}

#[cfg(not(all(target_os = "linux", target_pointer_width = "64")))]
fn thread_counters() -> Option<[u64; 3]> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame whose named stages sum to 31.5 ms, at `wall`, its largest stage `alloc`.
    fn frame(n: u32, wall: f64) -> FrameStages {
        let mut sync_parts = [0.0; 8];
        sync_parts[6] = 12.0; // alloc
        sync_parts[5] = 3.0; // copy
        FrameStages {
            frame: n,
            at_s: 2.5,
            wall,
            acquire: 0.5,
            update: 0.25,
            poll: 1.0,
            chunks: 7,
            evicted: 5,
            demoted: 3,
            events: 0.25,
            sync: 16.0,
            sync_parts,
            bricks: 900,
            queue: 6.0,
            laps: Some([4.0, 0.5, 0.5, 0.25, 0.25, 0.0, 0.0]),
            applied: [300, 2, 98_304, 3, 1],
            waiting: [4, 5000],
            prepare: 2.0,
            encode: 3.0,
            submit: 0.5,
            after: 1.0,
            present: 1.0,
            counters: Some([2, 300, 0]),
        }
    }

    #[test]
    fn a_slow_frame_line_names_every_stage_and_the_rest() {
        let f = frame(9, 40.0);
        assert_eq!(f.named(), 31.5);
        assert_eq!(
            f.line(),
            "slow frame: 9 at 2.5 s, wall 40.000 ms: acquire 0.500, update 0.250, poll 1.000 (7 chunks, 5 evicted, 3 demoted), events 0.250, sync 16.000 (select 0.000, size 0.000, pack 0.000, place 0.000, note 0.000, copy 3.000, alloc 12.000, lock 0.000, other 1.000; 900 bricks), queue 6.000 (build 4.000, take 0.500, issue 0.500, apply 0.250 (300 writes, 2 calls, 98304 bytes; 3 lists, 1 whole; waiting 4 lists, 5000 bytes), collect 0.250, send 0.000, wait 0.000, other 0.500), prepare 2.000, encode 3.000, submit 0.500, after 1.000, present 1.000, rest 8.500; switches 2, faults 300 minor 0 major"
        );

        // A frame of exactly 33.3 ms is not slow: no line, and only every frame's counters.
        let mut s = SlowFrames::default();
        s.file(frame(1, 33.3));
        assert_eq!(s.lines().len(), 1);
        assert!(s.lines()[0].starts_with("slow frames: 0 over 33.3 ms, 0 over 100 ms;"));

        // Three made-up frames whose largest stages are alloc, encode (with alloc cut) and the
        // rest (a 120 ms wall); filed out of order, printed in frame order.
        let mut s = SlowFrames::default();
        let mut b = frame(30, 34.0);
        b.sync_parts[6] = 1.0;
        b.sync = 5.0;
        b.encode = 14.0;
        s.file(frame(20, 40.0));
        s.file(b);
        s.file(frame(10, 120.0));
        let lines = s.lines();
        assert_eq!(lines.len(), 4);
        let order: Vec<&str> = lines[..3]
            .iter()
            .map(|l| l.split(" at ").next().unwrap_or(""))
            .collect();
        assert_eq!(
            order,
            ["slow frame: 10", "slow frame: 20", "slow frame: 30"]
        );
        let summary = &lines[3];
        assert!(summary.starts_with("slow frames: 3 over 33.3 ms, 1 over 100 ms; largest stage: "));
        assert!(summary.contains(", alloc 1 (mean 8.333),"), "{summary}");
        assert!(summary.contains(", encode 1 (mean 6.667),"), "{summary}");
        assert!(summary.contains(", rest 1 (mean "), "{summary}");
        assert!(summary.contains(", copy 0 (mean 3.000),"), "{summary}");
        // Frame 20's rest is 8.5 ms of 40, over the fifth (8.0); frame 30's 2.5 of 34 is not.
        assert!(
            summary.contains("; rest over a fifth of the wall 2;"),
            "{summary}"
        );
        assert!(
            summary.ends_with(
                "a slow frame's switches 2.00, faults 300.0 minor 0.00 major; any frame's switches 2.00, faults 300.0 minor 0.00 major"
            ),
            "{summary}"
        );
    }

    #[test]
    fn the_kept_lines_are_the_slowest_and_the_pending_frame_closes_on_the_next_dt() {
        let mut s = SlowFrames::default();
        for k in 0..(KEPT as u32 + 10) {
            s.file(frame(k, 40.0 + f64::from(k)));
        }
        let lines = s.lines();
        assert_eq!(lines.len(), KEPT + 1);
        assert!(lines[0].starts_with("slow frame: 10 at "), "{}", lines[0]);
        assert!(lines[KEPT].starts_with(&format!("slow frames: {} over", KEPT + 10)));

        let mut s = SlowFrames::default();
        s.next_frame(0.0, 0, 0.0, now());
        s.shell = ShellTimes {
            acquire_ms: 1.5,
            present_ms: 2.5,
        };
        if let Some(p) = s.pending.as_mut() {
            p.encode = 30.0;
        }
        s.next_frame(50.0, 1, 0.1, now());
        let lines = s.lines();
        assert!(
            lines[0].starts_with("slow frame: 0 at 0.0 s, wall 50.000 ms: acquire 1.500,"),
            "{}",
            lines[0]
        );
        assert!(
            lines[0].contains(", present 2.500, rest 16.000;"),
            "{}",
            lines[0]
        );
        assert_eq!(s.pending.map(|p| p.frame), Some(1));
    }
}
