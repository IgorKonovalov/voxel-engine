//! The live queue's counters.

use super::*;

/// The live queue's counters, for the HUD and stdout.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct QueueStats {
    /// Full chunks in the mirror with no block, not known faceless and not in a job: the depth.
    pub depth: u64,
    /// Chunks in a job, pending or open.
    pub jobs: u64,
    /// Of the candidates checked at the last opening, those whose reach was not ready, and those
    /// whose emitters' ranges were not.
    pub waiting_reach: u64,
    pub waiting_lights: u64,
    /// Chunks baked, and of them listed from a prepared result.
    pub baked: u64,
    pub prepared: u64,
    /// Blocks fading.
    pub fading: u64,
    /// The samples read back in the last step and their milliseconds on the GPU's timestamps
    /// (submit to readback where there are none).
    pub samples: u64,
    pub ms: f64,
    /// The last sliced step's slice in milliseconds, from the budget ([`Budget::slice_ms`]),
    /// before a dig's growth.
    pub slice_ms: f64,
    /// Whether the last sliced step's slice was the drain value ([`crate::bake::Drain`]), and
    /// the chunks in view still waiting for their tier's record it was read from
    /// ([`BakeQueue::view_waiting`]).
    pub draining: bool,
    pub drain_waiting: u64,
    /// Since the queue started: the submits' plans that stopped at the sample cap, at the
    /// entries' batch, and out of rounds to plan; and the steps that sent no submit because
    /// [`crate::bake::IN_FLIGHT`] were unread.
    pub plan_stops: [u64; 3],
    pub in_flight_full: u64,
    /// Samples since the queue started.
    pub total_samples: u64,
    /// Chunks whose block is the coarse pass, waiting for their refinement.
    pub coarse: u64,
    /// Of the depth, the chunks with no block yet, but those whose reach or lights were not ready
    /// at a recent check.
    pub unbaked: u64,
    /// The ns a sample the slice is sized by.
    pub ns_per_sample: f64,
    /// Chunks with dig bricks waiting or in a job, and chunks waiting to re-bake whole (ring 3).
    pub digs: u64,
    pub reach: u64,
    /// Dig jobs ended, and the bricks they re-baked, since the queue started.
    pub digs_baked: u64,
    pub dig_bricks: u64,
    /// Block-less chunks an edit's box touched, taken as whole digs since the queue started.
    pub digs_whole: u64,
    /// Places the pool refused since the queue started: each a job or a provisional block left
    /// waiting for an eviction to free room.
    pub pool_full: u64,
    /// The last step's sample cap, and the samples its one submit planned (0 for none): a submit
    /// passes the cap by less than one entry's round.
    pub cap: u64,
    pub submitted: u64,
    /// The stream workers' results taken since the queue started, and their time on the
    /// workers in total and at most, in nanoseconds ([`FacePrep`]).
    pub hook_chunks: u64,
    pub hook_ns: u64,
    pub hook_max_ns: u64,
    /// Of that time, the no-face verdict's in total and at most: the shell and the shared
    /// map's update under its lock ([`crate::bake::seal`]).
    pub seal_ns: u64,
    pub seal_max_ns: u64,
    /// Full chunks the workers' verdicts hold sealed (no exposed face, no candidate), and the
    /// shells the workers' map holds with their bytes.
    pub sealed: u64,
    pub shells: u64,
    pub shell_bytes: u64,
    /// The candidates at the last opening by where they lie against the view ([`VIEW_SHELL`],
    /// [`VIEW_IN`], [`VIEW_OUT`]): those with no block, and those with a coarse one.
    pub view_waiting: [[u64; 2]; 3],
    /// Of the candidates in the view cone beyond the shell at the last opening, those outside
    /// the frustum widened by the same margin ([`QueueView::frustum`]).
    pub cone_only: u64,
    /// The median distance in chunks of the candidates in the view cone beyond the shell at the
    /// last opening, with no block and coarse (0 for none).
    pub view_median: [f32; 2],
    /// The samples read back for the jobs ended since the queue started, by where their chunk
    /// lay against the view as the job ended.
    pub view_samples: [u64; 3],
    /// Of them, the samples of jobs whose chunk was out of view and behind the camera (its
    /// centre's view cosine under 0) as the job ended: passed.
    pub passed_samples: u64,
    /// The candidates at the last opening by class (no block, coarse), distance band (chunks 0
    /// to 2, 3 to 5, 6 to 10, over 10) and angle band (`a` 0, under 0.2, 0.2 to 0.4, over 0.4).
    pub weight_census: [[[u32; 4]; 4]; 2],
    /// Of the candidates in the view cone beyond the shell, the ones read at the last reach
    /// census ([`VIEW_REACH_SAMPLE`] at most, spread by distance, every [`VIEW_REACH_EVERY`]
    /// steps) and those whose reach was not ready.
    pub view_reach: [u64; 2],
    /// The CPU milliseconds of the view's counters at the last opening.
    pub census_ms: f64,
    /// Jobs withdrawn since the queue started ([`BakeQueue::withdraw`]), and the samples their
    /// faces had drawn: lost.
    pub withdrawn: u64,
    pub withdrawn_samples: u64,
    /// The last step's CPU time by stage ([`StepStages`]).
    pub stages: StepStages,
    /// Steps that ran an opening since the queue started.
    pub openings: u64,
    /// In the last step: the chunks the mirror filter and a whole reading of the candidates
    /// visited, the reach walks run ([`reach_ready`], not answered from the cache), and
    /// whether the candidates' order was scored again (1) or walked as kept (0).
    pub scanned: u64,
    pub reach_checks: u64,
    pub rebuilt: u64,
    /// Of the candidates in the view cone beyond the shell at the last census, waiting for
    /// their light (blank or coarse), those whose reach crosses the streamer's full radius
    /// ([`QueueView::full_radius`]): a chunk within their reach lies past it ([`reach_crosses`]),
    /// so a neighbour within their reach is not resident full while the camera stands still:
    /// near-band ones (0), which wait for good at a still camera under [`ReachMode::Full`], and
    /// mid- or far-band ones (1), which bake from the chunks resident (ADR-0080 rule 4).
    pub reach_bound: [u64; 2],
    /// Openings since the queue started that pushed a full job on a chunk with no block while a
    /// blank candidate outside [`BakeQueue::full_first`] was ready and left unpushed: the near
    /// shell starving the coarse front.
    pub starved_openings: u64,
    /// Coarse-pass jobs pushed since the queue started.
    pub coarse_opened: u64,
    /// Chunks whose block was baked from a partial reach ([`ReachMode::Partial`]).
    pub partial: u64,
    /// Jobs pushed from a partial reach since the queue started.
    pub partial_opened: u64,
    /// Flagged chunks re-queued since the queue started, a chunk within their reach having
    /// arrived full.
    pub partial_requeued: u64,
    /// Chunks whose block is provisional (baked under a column not yet final), now.
    pub provisional: u64,
    /// Jobs opened provisional since the queue started.
    pub provisional_opened: u64,
    /// Provisional chunks re-queued for their catch-up since the queue started.
    pub provisional_requeued: u64,
    /// Key-only re-bakes (ADR-0079) waiting or in a job, now: blocks whose key visibilities
    /// were baked under a key light turned past [`crate::Lighting::key_step`] since.
    pub keys: u64,
    /// Key-only re-bakes landed since the queue started.
    pub keys_baked: u64,
    /// The chunks in view (the view cone and the near shell) waiting for their light at the
    /// last census, by band and class, and the samples they still need ([`InViewWaiting`]).
    pub in_view: InViewWaiting,
    /// The samples read back since the queue started of jobs at the full count and of jobs at
    /// their own count (the coarse pass), and the GPU milliseconds of their submits split
    /// between them by those samples: an attribution, not a measurement of each class.
    pub class_samples: [u64; 2],
    pub class_ms: [f64; 2],
    /// The mirror's changes the steps read since the queue started, each chunk once a step:
    /// what the step's `filter` stages walk.
    pub mirror_changes: u64,
    /// Chunks given the rule's zero record since the queue started, with no GPU job
    /// ([`BakeQueue::zero_rule`]), and zeroed chunks baked again since, a change near them
    /// having broken the rule.
    pub zeroed: u64,
    pub zero_relisted: u64,
    /// Blocks whose band changed as the camera's chunk changed (their candidacy read again),
    /// and pending jobs withdrawn above their new band's count, since the queue started.
    pub retiered: u64,
    pub withdrawn_band: u64,
}

/// A chunk's band by its size on screen: where a voxel of its nearest point projects over the
/// far-normal band's top (near), within the band (mid), or under its bottom (far) ([`Bands`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Band {
    Near,
    Mid,
    Far,
}

impl Band {
    /// The bands nearest first, in the order of [`InViewWaiting::chunks`].
    pub const ALL: [Band; 3] = [Band::Near, Band::Mid, Band::Far];

    /// The band's index in [`Band::ALL`].
    pub fn index(self) -> usize {
        self as usize
    }

    /// The band's name on the lines: `near`, `mid`, `far`.
    pub fn name(self) -> &'static str {
        ["near", "mid", "far"][self.index()]
    }
}

/// The far-normal band (`[lo, hi]` pixels a voxel, `LookParams::far_normal`) at a view's focal
/// length in pixels (`focal_px`: half the output's height over the tangent of half the
/// vertical field, which for square pixels equals the width's over the horizontal's, the
/// shader's `far_normal_weight`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bands {
    pub focal_px: f32,
    pub lo: f32,
    pub hi: f32,
}

impl Bands {
    /// The focal length in pixels of an output `height` pixels tall under a vertical field of
    /// `vfov` radians.
    pub fn focal_px(height: u32, vfov: f32) -> f32 {
        height as f32 * 0.5 / (vfov * 0.5).tan()
    }

    /// The band of a voxel `px` pixels across: over `hi` near, from `lo` to `hi` mid, under `lo`
    /// far. With the band off (`hi <= lo`) every voxel is near.
    pub fn of_px(&self, px: f32) -> Band {
        if self.hi <= self.lo || px > self.hi {
            Band::Near
        } else if px >= self.lo {
            Band::Mid
        } else {
            Band::Far
        }
    }

    /// The pixels a voxel spans at `d` voxels from the eye.
    pub fn px_at(&self, d: f32) -> f32 {
        self.focal_px / d.max(1e-3)
    }

    /// The band of chunk `c` seen from `eye` (voxels): by its point nearest the eye (the eye's
    /// own chunk is near).
    pub fn of_chunk(&self, c: [i32; 3], eye: [f32; 3]) -> Band {
        self.of_px(self.px_at(chunk_nearest(c, eye)))
    }
}

/// The distance in voxels from `eye` to the nearest point of chunk `c`'s box (0 inside it).
pub fn chunk_nearest(c: [i32; 3], eye: [f32; 3]) -> f32 {
    (0..3)
        .map(|k| {
            let lo = (c[k] * 64) as f32;
            let hi = lo + 64.0;
            let d = (lo - eye[k]).max(eye[k] - hi).max(0.0);
            d * d
        })
        .sum::<f32>()
        .sqrt()
}

/// The chunks in view waiting for their light, by band ([`Band::ALL`]'s order) and class (no
/// record, a coarse record), and the samples they still need: each chunk's brick directions
/// (bounded from its prepared masks, else six a brick) at the count of every pass it has left.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InViewWaiting {
    pub chunks: [[u64; 2]; 3],
    pub samples: u64,
}

impl InViewWaiting {
    /// Counts a waiting chunk of `band`, with a coarse record or none, needing `samples`.
    pub fn note(&mut self, band: Band, coarse: bool, samples: u64) {
        self.chunks[band.index()][usize::from(coarse)] += 1;
        self.samples += samples;
    }

    /// The chunks waiting in every band and class.
    pub fn total(&self) -> u64 {
        self.chunks.iter().flatten().sum()
    }

    /// The chunks waiting in each band, both classes.
    pub fn by_band(&self) -> [u64; 3] {
        self.chunks.map(|[a, b]| a + b)
    }

    /// The chunks waiting with no record and with a coarse one, every band.
    pub fn by_class(&self) -> [u64; 2] {
        [0, 1].map(|k| self.chunks.iter().map(|b| b[k]).sum())
    }

    /// The seconds the need takes at `samples_per_s` (the last second's rate): `None` while the
    /// queue draws nothing and something waits, 0 when nothing does.
    pub fn drain_s(&self, samples_per_s: f64) -> Option<f64> {
        if self.samples == 0 {
            Some(0.0)
        } else if samples_per_s > 0.0 {
            Some(self.samples as f64 / samples_per_s)
        } else {
            None
        }
    }
}

/// A reading of the chunks the queue lists ([`BakeQueue::listing_census`]): every candidate and
/// every chunk in a job with no full block, by what the rules would answer of it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ListingCensus {
    /// The chunks read.
    pub listed: u64,
    /// Of them, those whose listing finds no exposed face.
    pub no_face: u64,
    /// Of those, the ones with a neighbour the mirror does not hold full (the residency's
    /// edge, read open by the workers' verdict and as rock by the listing).
    pub no_face_edge: u64,
    /// Of the rest, those whose reach box (the chunk dilated by the light radius) lies under
    /// the column map's lowest top over the box's footprint, with no exposed emitter within the
    /// light radius and every column over the reach final; and those that would be but for a
    /// column not yet final.
    pub buried: u64,
    pub buried_unfinal: u64,
    /// The rest (surface chunks) by band ([`Band::ALL`]'s order) and out of view (0) or in
    /// view (1: the view cone or the near shell).
    pub surface: [[u64; 2]; 3],
    /// Of the surface chunks, by band, those whose reach crosses the streamer's full radius
    /// ([`reach_crosses`]); none in a bounded world.
    pub reach_bound: [u64; 3],
    /// The census's CPU milliseconds.
    pub ms: f64,
}
