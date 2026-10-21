//! A streamed voice's source: a single-producer, single-consumer ring of mono frames at
//! [`crate::RATE`], allocated once.
//!
//! The writer (a microphone's callback, a network decoder) pushes slices without blocking; what
//! does not fit is dropped and counted. The reader is a voice on the mixer's callback: each block
//! it takes what is there and plays silence for the rest. Neither side locks or allocates.
//!
//! `head` counts frames written and `tail` frames read, both only growing; the writer alone
//! stores `head` and the reader alone stores `tail`. Each slot is an atomic holding the frame's
//! bits, written relaxed; the writer publishes its frames with a release store of `head` that
//! the reader's acquire load sees before reading them, and the reader frees slots the same way
//! through `tail`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

/// Frames a stream holds: 100 ms at [`crate::RATE`].
pub const STREAM_CAPACITY_FRAMES: usize = 4_800;

struct Ring {
    /// Frames as `f32` bits.
    buf: Box<[AtomicU32]>,
    head: AtomicUsize,
    tail: AtomicUsize,
    dropped: AtomicU64,
}

/// A stream's two ends.
pub struct Stream;

impl Stream {
    /// A new empty stream of [`STREAM_CAPACITY_FRAMES`]: its writer and its reader.
    #[allow(
        clippy::new_ret_no_self,
        reason = "a stream is only ever held as its two ends"
    )]
    pub fn new() -> (StreamWriter, StreamReader) {
        let ring = Arc::new(Ring {
            buf: (0..STREAM_CAPACITY_FRAMES)
                .map(|_| AtomicU32::new(0))
                .collect(),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            dropped: AtomicU64::new(0),
        });
        (
            StreamWriter {
                ring: Arc::clone(&ring),
            },
            StreamReader { ring },
        )
    }
}

/// The producing end.
pub struct StreamWriter {
    ring: Arc<Ring>,
}

impl StreamWriter {
    /// Appends as many of `frames` as fit and returns how many were dropped.
    pub fn push(&mut self, frames: &[f32]) -> usize {
        let r = &*self.ring;
        let head = r.head.load(Ordering::Relaxed);
        let tail = r.tail.load(Ordering::Acquire);
        let room = STREAM_CAPACITY_FRAMES - (head - tail);
        let n = frames.len().min(room);
        for (k, &x) in frames[..n].iter().enumerate() {
            // Slot `head + k` lies outside `[tail, head)`: the reader does not read it until
            // the release store below publishes it.
            r.buf[(head + k) % STREAM_CAPACITY_FRAMES].store(x.to_bits(), Ordering::Relaxed);
        }
        r.head.store(head + n, Ordering::Release);
        let dropped = frames.len() - n;
        if dropped > 0 {
            r.dropped.fetch_add(dropped as u64, Ordering::Relaxed);
        }
        dropped
    }

    /// Frames that can be pushed now without dropping any.
    pub fn space(&self) -> usize {
        let r = &*self.ring;
        STREAM_CAPACITY_FRAMES - (r.head.load(Ordering::Relaxed) - r.tail.load(Ordering::Acquire))
    }

    /// A handle that reads the stream's dropped-frame count from anywhere.
    pub fn monitor(&self) -> StreamMonitor {
        StreamMonitor {
            ring: Arc::clone(&self.ring),
        }
    }
}

/// Reads how many frames a stream's writer has dropped.
#[derive(Clone)]
pub struct StreamMonitor {
    ring: Arc<Ring>,
}

impl StreamMonitor {
    /// Frames dropped because the ring was full, since the stream began.
    pub fn dropped(&self) -> u64 {
        self.ring.dropped.load(Ordering::Relaxed)
    }
}

/// The consuming end, played by a voice ([`crate::Command::PlayStream`]).
pub struct StreamReader {
    ring: Arc<Ring>,
}

impl StreamReader {
    /// Fills `out` with the oldest frames and zeros past what is there; returns how many
    /// frames were read.
    pub fn read(&mut self, out: &mut [f32]) -> usize {
        let r = &*self.ring;
        let tail = r.tail.load(Ordering::Relaxed);
        let head = r.head.load(Ordering::Acquire);
        let n = out.len().min(head - tail);
        for (k, y) in out[..n].iter_mut().enumerate() {
            // Slot `tail + k` lies inside `[tail, head)`, published by the writer's release
            // store of `head`; the writer does not reuse it until `tail` passes it.
            *y = f32::from_bits(r.buf[(tail + k) % STREAM_CAPACITY_FRAMES].load(Ordering::Relaxed));
        }
        out[n..].fill(0.0);
        r.tail.store(tail + n, Ordering::Release);
        n
    }
}

impl std::fmt::Debug for StreamReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamReader").finish_non_exhaustive()
    }
}

impl PartialEq for StreamReader {
    /// The same stream.
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.ring, &other.ring)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_come_out_in_order_and_overflow_is_dropped() {
        let (mut w, mut r) = Stream::new();
        let monitor = w.monitor();
        let data: Vec<f32> = (0..3000).map(|i| i as f32).collect();
        assert_eq!(w.push(&data), 0);
        assert_eq!(w.space(), STREAM_CAPACITY_FRAMES - 3000);
        assert_eq!(w.push(&data), 1200);
        assert_eq!(monitor.dropped(), 1200);
        let mut out = vec![-1.0; 4000];
        assert_eq!(r.read(&mut out), 4000);
        assert_eq!(out[2999], 2999.0);
        assert_eq!(out[3000], 0.0);
        assert_eq!(out[3999], 999.0);
        let mut rest = vec![-1.0; 1000];
        assert_eq!(r.read(&mut rest), 800);
        assert_eq!(rest[799], 1799.0);
        assert!(rest[800..].iter().all(|&x| x == 0.0));
        // Wraps.
        assert_eq!(w.push(&data), 0);
        let mut again = vec![0.0; 3000];
        assert_eq!(r.read(&mut again), 3000);
        assert_eq!(again, data);
    }
}
