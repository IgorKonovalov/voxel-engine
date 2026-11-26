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
    /// The submits read back in the last step whose GPU time is in `ms` (0 where none was
    /// timed).
    pub read_submits: u64,
    /// The last sliced step's slice in milliseconds, from the budget ([`Budget::slice_ms`]),
    /// before a dig's growth.
    pub slice_ms: f64,
    /// Whether the last sliced step's slice was the drain value ([`crate::bake::Drain`]), and
    /// the chunks in view still waiting for their tier's record it was read from
    /// ([`BakeQueue::view_waiting`]).
    pub draining: bool,
    pub drain_waiting: u64,
    /// Reads of that set since the queue started ([`BakeQueue::refresh_waiting`]), their CPU
    /// milliseconds summed, and the longest read's.
    pub waiting_reads: u64,
    pub waiting_ms: f64,
    pub waiting_max_ms: f64,
    /// Since the queue started: the submits' plans that stopped at the sample cap, at the
    /// entries' batch, and out of rounds to plan; and the steps that sent no submit because
    /// the loop's bound were unread ([`crate::bake::IN_FLIGHT`] in lockstep, the bake thread's
    /// depth with it on); and of the plans out of rounds, those that found no table slot free.
    pub plan_stops: [u64; 3],
    pub in_flight_full: u64,
    pub slotless_stops: u64,
    /// Steps since the queue started that sent no submit with a submit slot free (neither
    /// blocked nor sent), and the jobs open after the last step's submit (0 where it sent none).
    pub idle_steps: u64,
    pub open_at_draw: u64,
    /// Steps since the queue started.
    pub steps: u64,
    /// The loop's samples read back: kept, and read into nothing (rounds past a face's stop,
    /// withdrawn jobs' rounds, maps that failed).
    pub read_back: [u64; 2],
    /// Samples since the queue started.
    pub total_samples: u64,
    /// Chunks whose block is the coarse pass, waiting for their refinement.
    pub coarse: u64,
    /// Of the depth, the chunks with no block yet, but those whose reach or lights were not ready
    /// at a recent check.
    pub unbaked: u64,
    /// The mean ns a sample of the submits read back.
    pub ns_per_sample: f64,
    /// The price the cap is sized by: a dispatch's fixed ms (0 until the submits' sizes first
    /// spread) and the ns a sample beyond it ([`SampleCost`]).
    pub fixed_ms: f64,
    pub cost_ns: f64,
    /// The samples the last step's read-backs drew, kept or not (`samples` counts those kept),
    /// and their rays' mean length in voxels (0 where none was read).
    pub drawn: u64,
    pub ray_len: f64,
    /// A dig's price by its first job's class: the ns a sample all in of ring-1 dig work and
    /// of other dig work, 0 until first read ([`DigCost`]).
    pub ring1_ns: f64,
    pub dig_ns: f64,
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
    /// The submits the last step planned, 0 or 1 (summed over the steps a frame of the bake
    /// thread took, as `submitted` is).
    pub submits: u64,
    /// Whether the last step's cap was stretched to its ring-1 dig jobs' samples left
    /// ([`BakeQueue::ring1_stretch`](crate::bake::BakeQueue::ring1_stretch)).
    pub stretched: bool,
    /// The stream workers' results taken since the queue started, and their time on the
    /// workers in total and at most, in nanoseconds ([`FacePrep`]).
    pub hook_chunks: u64,
    pub hook_ns: u64,
    pub hook_max_ns: u64,
    /// Of that time, the no-face verdict's in total and at most: the shell and the shared
    /// map's update under its lock ([`crate::bake::seal`]).
    pub seal_ns: u64,
    pub seal_max_ns: u64,
    /// Full chunks the workers' verdicts hold sealed (no exposed face, no candidate); the
    /// shells behind them are read by [`BakeQueue::shells`], which takes the workers' lock.
    pub sealed: u64,
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
    /// Jobs an edit withdrew from its ring ([`BakeQueue::withdraw_at`]) since the queue
    /// started, and the samples their faces had drawn: lost.
    pub dig_withdrawn: u64,
    pub dig_withdrawn_samples: u64,
    /// The last step's CPU time by stage ([`StepStages`]).
    pub stages: StepStages,
    /// Steps that ran an opening since the queue started.
    pub openings: u64,
    /// In the last step: the chunks the mirror filter and a whole reading of the candidates
    /// visited, the reach walks run ([`reach_ready`], not answered from the cache), and
    /// whether the order's index was read whole again (1, at a change of the order's leads)
    /// or kept (0).
    pub scanned: u64,
    pub reach_checks: u64,
    pub rebuilt: u64,
    /// In the last step: the candidates the order scored (its walks from the camera outward),
    /// 0 at a still camera with no candidate joining; and their sum since the queue started.
    pub visited: u64,
    pub visited_total: u64,
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
    /// Blocks landed since the queue started, by how ([`LANDED_REFADED`], [`LANDED_FALLBACK`],
    /// [`LANDED_UNFADED`], [`LANDED_BARE`]); key-only jobs are in [`QueueStats::keys_baked`].
    pub landings: [u64; 4],
    /// Of the faded landings since the queue started (refaded and over the fallback, key-only
    /// jobs among them), the cells whose words differ from what the pool showed as the fade
    /// started, by read ([`LANDING_DIRECTIONS`] to [`LANDING_TAIL`]); and every cell those
    /// landings hold, changed or not ([`LANDING_RECORDS`]).
    pub landing_cells: [u64; 7],
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
    /// Zeroed chunks listed again since the queue started because a neighbour their record was
    /// listed without (not held full) is held full now; each is ruled again or baked.
    pub zero_border_relisted: u64,
    /// With the bake thread's listing pool (Plan 0099 Phase 4b): the projection (the loop's
    /// samples left and the pool's estimates) over the cap at the last step's draw, 0 where
    /// the step drew nothing; the two fills of the listing bounds, listed from the world and
    /// from prepared masks; and the openings the pool's guard of 64 candidates stopped.
    pub projected: f64,
    pub fill: [f64; 2],
    pub pool_guard: u64,
    /// The jobs the rule would have answered, while [`BakeQueue::rule_reading`] is on.
    pub rule_missed: RuleMissed,
    /// The rule's audit, while [`BakeQueue::rule_audit`] is on.
    pub rule_audit: RuleAudit,
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

/// A reading of the GPU jobs the queue opened for chunks the rule would answer at that opening
/// (buried under the column tops as they stand with no exposed emitter near), by the first
/// exclusion that kept the rule from them: a column over the reach not final (`provisional`,
/// 0), the reach crossing the full radius (`partial`, 1), a block already held (class 1, 2).
/// Counted while [`BakeQueue::rule_reading`] is on, since it was set.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RuleMissed {
    /// The jobs and their samples by exclusion, every opening.
    pub jobs: [u64; 3],
    pub samples: [u64; 3],
    /// The same for the jobs opened before the in-view landing on chunks in the view cone or
    /// the near shell.
    pub before_jobs: [u64; 3],
    pub before_samples: [u64; 3],
    /// Every job opened and its samples, and those opened before the in-view landing.
    pub opened: [u64; 2],
    pub opened_before: [u64; 2],
    /// The step of the in-view landing: the first at which the drain's set is empty after a
    /// read found it not ([`BakeQueue::refresh_waiting`]); `None` until it lands.
    pub landed_at: Option<u64>,
}

impl RuleMissed {
    /// Counts a job opened with `samples`, and its exclusion and place in view where the
    /// reading noted the rule held for it.
    pub(super) fn note(&mut self, missed: Option<(usize, bool)>, samples: u64) {
        let before = self.landed_at.is_none();
        self.opened[0] += 1;
        self.opened[1] += samples;
        if before {
            self.opened_before[0] += 1;
            self.opened_before[1] += samples;
        }
        if let Some((why, cone)) = missed {
            self.jobs[why] += 1;
            self.samples[why] += samples;
            if before && cone {
                self.before_jobs[why] += 1;
                self.before_samples[why] += samples;
            }
        }
    }

    /// The jobs and samples the reading counted before the in-view landing, every exclusion,
    /// as shares of those opened before it.
    pub fn before_shares(&self) -> [f64; 2] {
        let j: u64 = self.before_jobs.iter().sum();
        let s: u64 = self.before_samples.iter().sum();
        [
            j as f64 / self.opened_before[0].max(1) as f64,
            s as f64 / self.opened_before[1].max(1) as f64,
        ]
    }
}

/// The audit's cases ([`RuleAudit`]): the rule held at the opening with none of the exclusions.
pub(crate) const AUDIT_FULL: usize = 0;
/// A column over the reach not final.
pub(crate) const AUDIT_PROVISIONAL: usize = 1;
/// The reach crossing the full radius.
pub(crate) const AUDIT_PARTIAL: usize = 2;
/// A block already held (class 1).
pub(crate) const AUDIT_CLASS1: usize = 3;

/// The differing blocks an audit keeps.
pub(crate) const AUDIT_KEPT: usize = 16;

/// One block an audit found differing from the rule's zero record: its chunk, its case, the
/// first differing word's index in the block, and the job's word and the rule's there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AuditDiffer {
    pub chunk: [i32; 3],
    pub case: u8,
    pub word: u32,
    pub job: u32,
    pub rule: u32,
}

/// An audit of the rule (while [`BakeQueue::rule_audit`] is on): every block a job placed, not
/// stale, for a chunk the rule held for at its opening, compared word for word with the zero
/// record the rule writes for the same faces, by the case at the opening ([`AUDIT_FULL`],
/// [`AUDIT_PROVISIONAL`], [`AUDIT_PARTIAL`], [`AUDIT_CLASS1`]; the first that applies, in the
/// reading's order). The records' tier bits are left out where the job's tier is not the
/// ruled record's (counted in `tier_masked`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RuleAudit {
    pub compared: [u64; 4],
    pub differ: [u64; 4],
    /// Blocks compared with the tier bits masked: a job at a lesser tier (ADR-0080 rule 2).
    pub tier_masked: u64,
    /// The first [`AUDIT_KEPT`] that differ, `kept` of them.
    pub first: [AuditDiffer; AUDIT_KEPT],
    pub kept: u8,
}

impl RuleAudit {
    /// Compares `job` (a block placed for `chunk`, case `case`) with `rule`, the zero record of
    /// its faces at the ruled tier, the tier bits of each cell record (`record` words a cell
    /// from the block's directory) masked where `mask_tier`.
    pub fn compare(
        &mut self,
        chunk: [i32; 3],
        case: usize,
        job: &[u32],
        rule: &[u32],
        record: usize,
        mask_tier: bool,
    ) {
        self.compared[case] += 1;
        self.tier_masked += u64::from(mask_tier);
        let tier_words: std::collections::HashSet<usize> = if mask_tier {
            tier_words(rule, record)
        } else {
            Default::default()
        };
        let mask = |i: usize, w: u32| {
            if tier_words.contains(&i) {
                w & !(3 << TIER_SHIFT)
            } else {
                w
            }
        };
        let n = job.len().max(rule.len());
        let at = (0..n).find(|&i| {
            let (a, b) = (job.get(i).copied(), rule.get(i).copied());
            a.map(|w| mask(i, w)) != b.map(|w| mask(i, w))
        });
        let Some(i) = at else {
            return;
        };
        self.differ[case] += 1;
        if usize::from(self.kept) < AUDIT_KEPT {
            self.first[usize::from(self.kept)] = AuditDiffer {
                chunk,
                case: case as u8,
                word: i as u32,
                job: job.get(i).copied().unwrap_or(u32::MAX),
                rule: rule.get(i).copied().unwrap_or(u32::MAX),
            };
            self.kept += 1;
        }
    }

    /// The differing blocks kept.
    pub fn kept(&self) -> &[AuditDiffer] {
        &self.first[..usize::from(self.kept)]
    }
}

/// The indices of the words holding the tier bits ([`TIER_SHIFT`]) in `block`: each cell
/// record's second visibility word, the records found from the directory.
fn tier_words(block: &[u32], record: usize) -> std::collections::HashSet<usize> {
    let mut out = std::collections::HashSet::new();
    for &dir in block.iter().take(CHUNK_BRICKS) {
        if dir == 0 {
            continue;
        }
        let run = (dir >> 11) as usize;
        for cell in (run - record..run).step_by(BRICK_WORDS) {
            out.insert(cell + VIS_WORD + 1);
        }
    }
    out
}

/// The hashes of the store's blocks a reading compares across runs ([`BakeQueue::rule_hashes`],
/// [`QueueRead::rule_hashes`]). A block's hash is FNV-1a 64 over its words as little-endian
/// `u32`, the landing word ([`LANDED_AT`], the frame the chunk's first block landed in) left
/// out; a set's is FNV-1a 64 over its chunks ascending, each as its coordinate (three
/// little-endian `i32`) then its block's hash (little-endian `u64`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuleHashes {
    /// The request this answers ([`BakeQueue::ask_rule_hashes`]); `u64::MAX` for a read made
    /// on the spot in lockstep.
    pub asked: u64,
    /// The queue's steps when it was read.
    pub step: u64,
    /// The zeroed chunks (the rule's records) holding a block, ascending, with their blocks'
    /// hashes.
    pub chunks: Vec<([i32; 3], u64)>,
    /// Zeroed chunks whose block the store does not hold yet (pushed, placed at the submit).
    pub unplaced: u64,
    /// The zeroed set's hash over `chunks`.
    pub zeroed: u64,
    /// The whole store's: every block, and its count. A job's record on a partial reach casts
    /// from what was resident (ADR-0080 rule 4), so this one depends on the camera's start.
    pub store: u64,
    pub blocks: u64,
}

/// A block's hash ([`RuleHashes`]): its words but the landing word.
pub(crate) fn block_hash(words: &[u32]) -> u64 {
    let mut h = svet_world::hash::Fnv1a::default();
    for (i, w) in words.iter().enumerate() {
        if i != LANDED_AT as usize {
            h.write(&w.to_le_bytes());
        }
    }
    h.finish()
}

/// Folds `(chunk, block hash)` pairs, in the order given, into a set's hash ([`RuleHashes`]).
fn set_hash<'a>(pairs: impl Iterator<Item = &'a ([i32; 3], u64)>) -> u64 {
    let mut h = svet_world::hash::Fnv1a::default();
    for (c, b) in pairs {
        for v in c {
            h.write(&v.to_le_bytes());
        }
        h.write(&b.to_le_bytes());
    }
    h.finish()
}

impl BakeQueue {
    /// The hashes of the zeroed chunks' blocks and of the whole store's in `store`'s copy, read
    /// now ([`RuleHashes`]; `asked` names the request). A reading: it hashes every block.
    pub(crate) fn rule_hashes_of(&self, store: &Store, asked: u64) -> RuleHashes {
        let all: Vec<([i32; 3], u64)> = store
            .words
            .iter()
            .map(|(&c, w)| (c, block_hash(w)))
            .collect();
        let chunks: Vec<([i32; 3], u64)> = all
            .iter()
            .filter(|(c, _)| self.zeroed.contains(c))
            .copied()
            .collect();
        RuleHashes {
            asked,
            step: self.steps,
            unplaced: self.zeroed.len() as u64 - chunks.len() as u64,
            zeroed: set_hash(chunks.iter()),
            chunks,
            store: set_hash(all.iter()),
            blocks: all.len() as u64,
        }
    }

    /// Asks the bake thread for [`RuleHashes`]: the first step after the message that carries
    /// request `n` reads them ([`BakeQueue::answer_rule_hashes`]), and the books it publishes
    /// carry them from then on. Requests are numbered upward; an older one is answered by a newer.
    pub fn ask_rule_hashes(&mut self, n: u64) {
        self.rule_hash_asked = self.rule_hash_asked.max(n);
    }

    /// Reads [`RuleHashes`] after a bake thread's step when a request newer than the last
    /// answer waits.
    pub(crate) fn answer_rule_hashes(&mut self, store: &Store) {
        let answered = self.rule_hashes.as_ref().map_or(0, |h| h.asked);
        if self.rule_hash_asked > answered {
            self.rule_hashes = Some(std::sync::Arc::new(
                self.rule_hashes_of(store, self.rule_hash_asked),
            ));
        }
    }
}

/// [`QueueStats::landings`]: a block over its chunk's block of the same layout, faded from
/// what that showed.
pub const LANDED_REFADED: usize = 0;
/// A first block over the missing-light chain, faded from the chain's values.
pub const LANDED_FALLBACK: usize = 1;
/// A block over its chunk's block of another layout, put with no fade.
pub const LANDED_UNFADED: usize = 2;
/// A first block with no chain to fade from, put with no fade.
pub const LANDED_BARE: usize = 3;

/// [`QueueStats::landing_cells`]: the six direction values (faded).
pub const LANDING_DIRECTIONS: usize = 0;
/// The sky visibilities, both baked alike (faded under [`crate::Lighting::landing_fade`]).
pub const LANDING_VIS: usize = 1;
/// The sky visibilities' flags (baked, stale) changed: not faded.
pub const LANDING_VIS_FLAG: usize = 2;
/// The key visibilities, both baked (faded).
pub const LANDING_KEY: usize = 3;
/// The key visibilities with either unbaked: not faded.
pub const LANDING_KEY_FLAG: usize = 4;
/// The tail word (material and direction mask): not faded.
pub const LANDING_TAIL: usize = 5;
/// Every cell record of the faded landings, changed or not: 0 for blocks with no surface brick.
pub const LANDING_RECORDS: usize = 6;

/// One job of a [`DigTrace`]: its chunk, whether it is open (else pending) and its place in
/// its list, whether it is a dig's ([`BakeQueue::dig_open`]) and whether an edit made it stale
/// (its block is thrown away when it ends), and per brick with a live face the brick's lowest
/// voxel, the samples its faces still have to submit, have in flight and have drawn.
#[derive(Clone, Debug, Default)]
pub struct DigTraceJob {
    pub coord: [i32; 3],
    pub open: bool,
    pub at: usize,
    pub dig: bool,
    pub stale: bool,
    pub bricks: Vec<([i32; 3], u64, u64, u64)>,
}

impl DigTraceJob {
    /// The job's samples left, in flight and drawn, over its bricks.
    pub fn totals(&self) -> [u64; 3] {
        self.bricks
            .iter()
            .fold([0; 3], |a, b| [a[0] + b.1, a[1] + b.2, a[2] + b.3])
    }
}

/// The queue's state a reading reads a dig's timeline from ([`BakeQueue::dig_trace`]): the
/// chunks waiting in `digs` (each with its ring-1 flag, its dig bricks, and whether a job holds
/// it), the jobs of chunks in `digs`, `dig_open` or `stale` (every other job is summed), the
/// free table slots, the pending and open jobs' counts, the submits in flight and the
/// withdrawn jobs whose rounds are.
#[derive(Clone, Debug, Default)]
pub struct DigTrace {
    pub waiting: Vec<([i32; 3], bool, usize, bool)>,
    pub jobs: Vec<DigTraceJob>,
    /// Every other job's samples left, in flight and drawn, summed.
    pub others: [u64; 3],
    pub free_slots: usize,
    pub pending: usize,
    pub open: usize,
    pub in_flight: usize,
    pub withdrawn: usize,
    /// The loop's report: the samples read back, those read into nothing, and the submits.
    pub read: [u64; 3],
}

impl BakeQueue {
    /// The queue's state for a dig's timeline ([`DigTrace`]); a reading, not read by the frame.
    pub fn dig_trace(&self) -> DigTrace {
        let lp = &self.lp;
        let in_job: std::collections::HashSet<[i32; 3]> = lp.chunks().map(|c| c.0).collect();
        let waiting = self
            .digs
            .iter()
            .map(|(c, d)| (*c, d.near, d.slots.len(), in_job.contains(c)))
            .collect();
        let watched = |c: &[i32; 3]| {
            self.digs.contains_key(c) || self.dig_open.contains_key(c) || self.stale.contains_key(c)
        };
        let mut t = DigTrace {
            waiting,
            free_slots: lp.free.len(),
            pending: lp.pending.len(),
            open: lp.open.len(),
            in_flight: lp.in_flight.len(),
            withdrawn: lp.withdrawn.len(),
            read: [lp.report.samples, lp.report.wasted, lp.report.submits],
            ..DigTrace::default()
        };
        let bricks_of = |states: &[crate::bake::job::FaceState]| {
            let mut by: BTreeMap<[i32; 3], (u64, u64, u64)> = BTreeMap::new();
            for s in states {
                let b = s.voxel.map(|v| v.div_euclid(8) * 8);
                let e = by.entry(b).or_default();
                if !s.done {
                    e.0 += u64::from(s.n_max.saturating_sub(s.n + s.queued));
                }
                e.1 += u64::from(s.queued);
                e.2 += u64::from(s.n);
            }
            by.into_iter()
                .filter(|(_, v)| *v != (0, 0, 0))
                .map(|(b, v)| (b, v.0, v.1, v.2))
                .collect::<Vec<_>>()
        };
        for (at, j) in lp.open.iter().enumerate() {
            let c = j.coord.0;
            let bricks = bricks_of(&j.states);
            if !watched(&c) {
                for b in &bricks {
                    t.others = [t.others[0] + b.1, t.others[1] + b.2, t.others[2] + b.3];
                }
                continue;
            }
            t.jobs.push(DigTraceJob {
                coord: c,
                open: true,
                at,
                dig: self.dig_open.contains_key(&c),
                stale: self.stale.contains_key(&c),
                bricks,
            });
        }
        for (at, j) in lp.pending.iter().enumerate() {
            let c = j.coord.0;
            if !watched(&c) {
                t.others[0] += j.samples;
                continue;
            }
            // A pending dig job's bricks: its dig bricks' directions at its count.
            let bricks = match self.dig_open.get(&c) {
                Some(d) => {
                    let n = u64::from(j.brick_samples.unwrap_or(self.settings.brick_samples));
                    j.faces
                        .bricks
                        .iter()
                        .filter(|(slot, _)| d.slots.contains(slot))
                        .map(|(slot, keys)| {
                            let b = brick_of_slot(usize::from(*slot));
                            let mut dirs = [false; 6];
                            for k in keys {
                                dirs[usize::from(k & 7)] = true;
                            }
                            let left = dirs.iter().filter(|d| **d).count() as u64 * n;
                            ([0, 1, 2].map(|k| c[k] * 64 + b[k] as i32 * 8), left, 0, 0)
                        })
                        .collect()
                }
                None => vec![(c.map(|v| v * 64), j.samples, 0, 0)],
            };
            t.jobs.push(DigTraceJob {
                coord: c,
                open: false,
                at,
                dig: self.dig_open.contains_key(&c),
                stale: self.stale.contains_key(&c),
                bricks,
            });
        }
        t
    }
}

/// The bake thread's submits on the render side (a reading, ADR-0083): each submit's round trip
/// in frames, from the frame that issued it to the frame whose message carried its read-back;
/// the submits issued; the frames that carried a submit past them (a frame issues at most one);
/// and the most readback buffers the issuer held at once.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BakeRoundTrip {
    /// Read-backs by their round trip in frames: index `k` counts those of `k` frames, the
    /// last index those of that many or more.
    pub frames: Vec<u64>,
    pub issued: u64,
    pub carried: u64,
    pub peak_buffers: usize,
}

impl BakeRoundTrip {
    /// The longest round trip [`BakeRoundTrip::frames`] tells apart.
    pub const FRAMES: usize = 64;

    /// Counts a read-back of `frames` frames.
    pub(crate) fn count(&mut self, frames: u64) {
        if self.frames.is_empty() {
            self.frames = vec![0; Self::FRAMES + 1];
        }
        self.frames[(frames as usize).min(Self::FRAMES)] += 1;
    }

    /// `other`'s counts added to `self`'s; the peak is the larger.
    pub(crate) fn merge(&mut self, other: &BakeRoundTrip) {
        if self.frames.is_empty() {
            self.frames = vec![0; Self::FRAMES + 1];
        }
        for (k, n) in other.frames.iter().enumerate() {
            self.frames[k] += n;
        }
        self.issued += other.issued;
        self.carried += other.carried;
        self.peak_buffers = self.peak_buffers.max(other.peak_buffers);
    }

    /// The round trip at percentile `p` (0 to 100), in frames; 0 with none read.
    pub fn percentile(&self, p: f64) -> u64 {
        let n: u64 = self.frames.iter().sum();
        if n == 0 {
            return 0;
        }
        let rank = ((p / 100.0) * n as f64).ceil().max(1.0) as u64;
        let mut seen = 0;
        for (k, c) in self.frames.iter().enumerate() {
            seen += c;
            if seen >= rank {
                return k as u64;
            }
        }
        Self::FRAMES as u64
    }
}

/// How well the frames fed the slice (a reading, the teardown's `queue fed:` line): over the
/// frames pushed, the samples the steps planned into submits over the caps granted, a frame
/// with no step counting the newest cap ([`QueueStats::idle`] keeps it). Its two factors are
/// the submits a frame and the drawn share a submit (the samples a submit planned over the
/// mean cap), whose product it is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct QueueFed {
    pub frames: u64,
    pub caps: u64,
    pub planned: u64,
    pub submits: u64,
}

impl QueueFed {
    /// Counts a frame whose stats (the steps it took, summed) are `st`.
    pub fn push(&mut self, st: &QueueStats) {
        self.frames += 1;
        self.caps += st.cap;
        self.planned += st.submitted;
        self.submits += st.submits;
    }

    /// The samples planned over the caps granted.
    pub fn fed(&self) -> f64 {
        self.planned as f64 / self.caps.max(1) as f64
    }

    /// The submits planned a frame.
    pub fn submits_a_frame(&self) -> f64 {
        self.submits as f64 / self.frames.max(1) as f64
    }

    /// The samples a submit planned over the mean cap granted a frame.
    pub fn drawn_a_submit(&self) -> f64 {
        let cap = self.caps as f64 / self.frames.max(1) as f64;
        self.planned as f64 / (self.submits.max(1) as f64 * cap.max(1.0))
    }

    /// The line's text after `queue fed: `; `None` with no frame pushed.
    pub fn line(&self) -> Option<String> {
        (self.frames > 0).then(|| {
            format!(
                "{} frames, fed {:.3} ({:.4e} samples planned into submits over {:.4e} granted), submits a frame {:.3}, drawn a submit {:.3}",
                self.frames,
                self.fed(),
                self.planned as f64,
                self.caps as f64,
                self.submits_a_frame(),
                self.drawn_a_submit(),
            )
        })
    }
}

/// The bake thread's cycle by its parts, in ms, over the steps pushed (a reading, the
/// teardown's `bake thread cycle:` line): the wait for a message, the step, the books'
/// snapshot, the hand-over (the step's results and the books put in the outbox), and their sum,
/// the time from one step's pick-up to the next's; beside them the wall of the frames pushed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CycleLine {
    pub parts: Vec<[f64; 4]>,
    pub walls: Vec<f64>,
}

impl CycleLine {
    /// The line's text after `bake thread cycle: `; `None` with no step pushed.
    pub fn line(&self) -> Option<String> {
        if self.parts.is_empty() {
            return None;
        }
        let p50_mean = |mut v: Vec<f64>| {
            v.sort_by(f64::total_cmp);
            let mean = v.iter().sum::<f64>() / v.len().max(1) as f64;
            let p50 = v.get(v.len() / 2).copied().unwrap_or(f64::NAN);
            format!("{p50:.3}/{mean:.3}")
        };
        let part = |k: usize| p50_mean(self.parts.iter().map(|p| p[k]).collect());
        let sum = p50_mean(self.parts.iter().map(|p| p.iter().sum()).collect());
        Some(format!(
            "{} steps over {} frames ({:.3} a frame), p50/mean ms: wait {}, step {}, snapshot {}, hand-over {}, sum {} against the frame's wall {}",
            self.parts.len(),
            self.walls.len(),
            self.parts.len() as f64 / self.walls.len().max(1) as f64,
            part(0),
            part(1),
            part(2),
            part(3),
            sum,
            p50_mean(self.walls.clone()),
        ))
    }
}

#[cfg(test)]
mod fed_tests {
    use super::*;

    /// Done when (1) of Plan 0099 Phase 4d. Ten frames at a cap of 1 000, six of them with a
    /// submit of 900 samples and four with none: fed 0.54, submits a frame 0.60, drawn a
    /// submit 0.90.
    #[test]
    fn the_fed_line_is_the_samples_planned_over_the_caps_granted() {
        let mut fed = QueueFed::default();
        for k in 0..10 {
            let sent = k % 5 < 3;
            fed.push(&QueueStats {
                cap: 1_000,
                submitted: if sent { 900 } else { 0 },
                submits: u64::from(sent),
                ..QueueStats::default()
            });
        }
        let line = fed.line().expect("ten frames");
        println!("queue fed: {line}");
        assert_eq!((fed.frames, fed.submits), (10, 6), "{line}");
        for want in ["fed 0.540", "submits a frame 0.600", "drawn a submit 0.900"] {
            assert!(line.contains(want), "{want} in {line}");
        }
    }
}
