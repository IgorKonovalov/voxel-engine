//! The books' snapshot the bake thread publishes with each step's results (ADR-0083): what the
//! render thread's public reads ([`QueueRead`]) and the store's reads consult while the books
//! live on the bake thread. It is made on the bake thread, after the step, and swapped in by the
//! render thread when it takes the step's lists, so its answers stand as of the newest list
//! taken.

use super::*;
use std::collections::HashSet;

/// Voxels from the eye within which a neighbouring column's chunks join the snapshot's words:
/// the encode's eye may cross into it before the next snapshot.
const AROUND_MARGIN: f32 = 16.0;

/// A chunk's summary record, by chunk.
type Summary = ([i32; 3], [u32; BRICK_WORDS]);

/// The books as of one bake step, owned.
#[derive(Debug, Default)]
pub(crate) struct Books {
    /// The message's frame the step read.
    pub(crate) frame: u64,
    /// The step's eye (voxels), which [`Books::around`] was read under.
    pub(crate) eye: [f32; 3],
    pub(crate) stats: QueueStats,
    pub(crate) shells: (u64, u64),
    pub(crate) ages: (Vec<u32>, Vec<u32>),
    /// [`QueueRead::empty`] but for the mailbox: no job, fade, candidate, dig or due chunk.
    pub(crate) idle: bool,
    /// Edit boxes the books hold, not yet taken by a step.
    pub(crate) boxes: bool,
    /// No dig brick, dig job or ring-3 chunk.
    pub(crate) digs_quiet: bool,
    /// The store's blocks: first word and words, by chunk.
    pub(crate) blocks: BTreeMap<[i32; 3], (u64, u64)>,
    /// Chunks whose block is below its tier's count ([`BakeQueue::lesser`]).
    pub(crate) lesser: HashSet<[i32; 3]>,
    pub(crate) faceless: HashSet<[i32; 3]>,
    pub(crate) sealed: HashSet<[i32; 3]>,
    pub(crate) blocked: HashSet<[i32; 3]>,
    /// Chunks in a job, with dig bricks, in ring 3, or due a refinement or a catch-up.
    pub(crate) busy: HashSet<[i32; 3]>,
    /// The store's counts with no GPU words read ([`Store::stats`]).
    pub(crate) store_stats: StoreStats,
    /// The words of the blocks in the columns under the step's eye, its chunk's summary and
    /// the records' cell: what [`crate::bake::sky_over_floor`] reads.
    pub(crate) around: BTreeMap<[i32; 3], Vec<u32>>,
    pub(crate) around_summary: Option<Summary>,
    pub(crate) cell: u32,
}

impl Books {
    /// The snapshot of `q` and `store` after the step that read `frame`, with the words under
    /// `eye` (voxels).
    pub(crate) fn of(q: &BakeQueue, store: &Store, frame: u64, eye: [f32; 3]) -> Self {
        let busy =
            q.lp.chunks()
                .map(|c| c.0)
                .chain(q.digs.keys().copied())
                .chain(q.reach.iter().copied())
                .chain(q.partial_due.iter().copied())
                .chain(q.provisional_due.iter().copied())
                .collect();
        let lesser = store
            .blocks
            .keys()
            .copied()
            .filter(|&c| q.lesser(store, c))
            .collect();
        let (around, around_summary) = around(store, eye);
        Self {
            frame,
            eye,
            stats: q.stats,
            shells: q.shells(),
            ages: (q.ages_first.clone(), q.ages_full.clone()),
            idle: q.lp.idle()
                && q.fades.is_empty()
                && q.counted_idle
                && q.stats.depth == 0
                && q.digs.is_empty()
                && q.reach.is_empty()
                && q.partial_due.is_empty()
                && q.provisional_due.is_empty(),
            boxes: !q.boxes.is_empty(),
            digs_quiet: q.digs.is_empty() && q.dig_open.is_empty() && q.reach.is_empty(),
            blocks: store.blocks.clone(),
            lesser,
            faceless: q.faceless.iter().copied().collect(),
            sealed: q.sealed.held().collect(),
            blocked: q.blocked.keys().copied().collect(),
            busy,
            store_stats: store.stats(0, 0, 0),
            around,
            around_summary,
            cell: store.cell,
        }
    }

    /// About the bytes the snapshot holds, for the outbox's bound.
    pub(crate) fn bytes(&self) -> usize {
        let set = 12 + 8;
        std::mem::size_of::<Self>()
            + self.blocks.len() * (12 + 16 + 8)
            + (self.lesser.len()
                + self.faceless.len()
                + self.sealed.len()
                + self.blocked.len()
                + self.busy.len())
                * set
            + (self.ages.0.len() + self.ages.1.len()) * 4
            + self
                .around
                .values()
                .map(|w| w.len() * 4 + 12)
                .sum::<usize>()
    }

    /// Whether `eye` lies where the snapshot's words reach: within [`AROUND_MARGIN`] of the
    /// step's eye across, in its chunk's layer.
    pub(crate) fn reaches(&self, eye: [f32; 3]) -> bool {
        let layer = |v: f32| (v.floor() as i32).div_euclid(64);
        (0..2).all(|k| (eye[k] - self.eye[k]).abs() <= AROUND_MARGIN)
            && layer(eye[2]) == layer(self.eye[2])
    }

    /// [`crate::bake::sky_over_floor`] over the snapshot's words.
    pub(crate) fn sky_over_floor(&self, eye: [f32; 3]) -> Option<(f32, crate::bake::AroundSource)> {
        crate::bake::floor_sky(
            |c| self.around.get(&c).map(Vec::as_slice),
            |c| {
                self.around_summary
                    .as_ref()
                    .filter(|(at, _)| *at == c)
                    .map(|(_, s)| s)
            },
            self.cell,
            eye,
        )
    }
}

/// The blocks of the columns [`crate::bake::sky_over_floor`] can read from `eye` and its
/// neighbours within [`AROUND_MARGIN`], the eye's chunk and the two under it, and the eye
/// chunk's summary.
fn around(store: &Store, eye: [f32; 3]) -> (BTreeMap<[i32; 3], Vec<u32>>, Option<Summary>) {
    let mut words = BTreeMap::new();
    if !eye.iter().all(|v| v.is_finite()) {
        return (words, None);
    }
    let chunk = eye.map(|v| (v.floor() as i32).div_euclid(64));
    for dx in [-AROUND_MARGIN, 0.0, AROUND_MARGIN] {
        for dy in [-AROUND_MARGIN, 0.0, AROUND_MARGIN] {
            let cx = ((eye[0] + dx).floor() as i32).div_euclid(64);
            let cy = ((eye[1] + dy).floor() as i32).div_euclid(64);
            for dz in 0..=2 {
                let c = [cx, cy, chunk[2] - dz];
                if let Some(w) = store.words.get(&c) {
                    words.entry(c).or_insert_with(|| w.clone());
                }
            }
        }
    }
    let summary = store.light.summary(chunk).map(|s| (chunk, *s));
    (words, summary)
}

impl QueueStats {
    /// `self`, a step's, after `earlier` steps taken in the same frame: the newest state, the
    /// read-back's and the submit's counts of both.
    pub(crate) fn after(self, earlier: &QueueStats) -> QueueStats {
        QueueStats {
            samples: self.samples + earlier.samples,
            ms: self.ms + earlier.ms,
            read_submits: self.read_submits + earlier.read_submits,
            submitted: self.submitted + earlier.submitted,
            projected: if self.projected > 0.0 {
                self.projected
            } else {
                earlier.projected
            },
            ..self
        }
    }

    /// `self` as a frame that took no step reads it: the state, no read-back or submit.
    pub(crate) fn idle(self) -> QueueStats {
        QueueStats {
            samples: 0,
            ms: 0.0,
            read_submits: 0,
            submitted: 0,
            projected: 0.0,
            ..self
        }
    }
}

/// The encode's reads of the books' snapshot while the bake thread runs (a reading): frames,
/// frames whose eye lay past [`Books::reaches`], and frames whose sky over the floor read
/// nothing at the encode's eye but an answer at the step's.
/// The fourth count is the longest run of consecutive frames past the reach, the fifth the
/// current run.
#[derive(Default)]
pub(crate) struct SnapshotReach {
    counts: [std::sync::atomic::AtomicU64; 5],
}

impl SnapshotReach {
    /// One encode at `eye` against `books`.
    pub(crate) fn note(&self, books: &Books, eye: [f32; 3]) {
        use std::sync::atomic::Ordering::Relaxed;
        self.counts[0].fetch_add(1, Relaxed);
        if books.reaches(eye) {
            self.counts[4].store(0, Relaxed);
        } else {
            self.counts[1].fetch_add(1, Relaxed);
            let run = self.counts[4].fetch_add(1, Relaxed) + 1;
            self.counts[3].fetch_max(run, Relaxed);
        }
        if books.sky_over_floor(eye).is_none() && books.sky_over_floor(books.eye).is_some() {
            self.counts[2].fetch_add(1, Relaxed);
        }
    }

    pub(crate) fn read(&self) -> [u64; 4] {
        let c = self
            .counts
            .each_ref()
            .map(|c| c.load(std::sync::atomic::Ordering::Relaxed));
        [c[0], c[1], c[2], c[3]]
    }
}
