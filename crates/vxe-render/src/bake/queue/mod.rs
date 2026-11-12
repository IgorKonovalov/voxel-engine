//! The live bake queue: its state, its settings and its views.

use super::*;

mod digs;
mod list;
mod open;
mod step;

/// Frames a re-baked block's values move over, from what it showed to its new values.
pub const FADE: u32 = 16;

/// Frames a dig job's block moves over, from its provisional block: short, so a dug hole's light
/// is seen settling for a moment, not dissolving.
pub const DIG_FADE: u32 = 4;

/// Samples a brick direction draws in the live queue: the owner's pick (Plan 0059 Phase 7b).
pub const QUEUE_BRICK_SAMPLES: u32 = 4096;

/// Candidates whose reach the queue checks at most in a frame that opens jobs.
pub(super) const QUEUE_CHECKS: usize = 16;

/// The same in a step without a slice (the pre-roll), where the coarse pass opens many small jobs.
pub(super) const QUEUE_CHECKS_UNSLICED: usize = 64;

/// A brick direction's samples in the live queue's coarse pass by default (`--coarse`).
pub const DEFAULT_COARSE_SAMPLES: u32 = 256;

/// Chunks (Chebyshev) around the camera's whose first pass is the full count, not the coarse one
/// (`--near`).
pub const DEFAULT_NEAR: i32 = 2;

/// Frames before a candidate whose reach or lights were not ready is checked again.
pub(super) const QUEUE_RECHECK: u64 = 30;

/// Steps an opening that found nothing ready waits before sorting the candidates again.
pub(super) const QUEUE_QUIET: u64 = 8;

/// A hasher for chunk coordinates in the queue step's per-change sets: one multiply and rotate
/// a word (FxHash's mix), where std's SipHash costs several times as much a key, and a flight
/// hashes about a thousand mirror changes a step. No key comes from outside the process and no
/// set's iteration order is read.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ChunkHasher(u64);

impl ChunkHasher {
    pub(super) fn add(&mut self, w: u64) {
        self.0 = (self.0.rotate_left(5) ^ w).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

impl std::hash::Hasher for ChunkHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for w in bytes.chunks(4) {
            let mut b = [0u8; 4];
            b[..w.len()].copy_from_slice(w);
            self.add(u64::from(u32::from_le_bytes(b)));
        }
    }

    fn write_u32(&mut self, i: u32) {
        self.add(u64::from(i));
    }

    fn write_i32(&mut self, i: i32) {
        self.add(u64::from(i as u32));
    }

    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }
}

/// A set of chunk coordinates under [`ChunkHasher`].
pub(super) type ChunkSet =
    std::collections::HashSet<[i32; 3], std::hash::BuildHasherDefault<ChunkHasher>>;

/// A map from chunk coordinates under [`ChunkHasher`].
pub(super) type ChunkMap<V> =
    std::collections::HashMap<[i32; 3], V, std::hash::BuildHasherDefault<ChunkHasher>>;
/// Bytes of prepared masks the queue holds at most; the farthest from the camera go first.
pub(super) const PREPARED_BYTES_MAX: usize = 64 << 20;

/// The live queue's settings: the default's brick level ([`BRICK_CELL`], [`QUEUE_BRICK_SAMPLES`]
/// a brick direction), a face's round one entry's lanes.
pub fn queue_settings() -> BakeSettings {
    BakeSettings {
        round: QUEUE_ROUND,
        ..BakeSettings::default()
    }
}

/// Samples a face draws in one entry of the live queue's slices: one a lane of the entry's
/// workgroup ([`BAKE_LANES`]).
pub(crate) const QUEUE_ROUND: u32 = BAKE_LANES;

/// Samples a face draws in one entry of a step without a slice ([`QueueView::samples`]).
pub(crate) const UNSLICED_ROUND: u32 = BAKE_LANES;

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
    /// ([`QueueView::full_radius`]): their distance in chunks plus the light radius exceeds it,
    /// so a neighbour within their reach is not resident full while the camera stands still.
    pub reach_bound: u64,
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
}

/// What [`BakeQueue::push_batch`] pushed: the jobs' samples, the chunks with no block the pool
/// refused, and whether a job was a full first pass on a chunk with no block.
#[derive(Default)]
pub(super) struct Pushed {
    pub(super) samples: u64,
    pub(super) refused0: usize,
    pub(super) full_blank: bool,
}

/// A block fading from what it showed to its target over `frames` ([`FADE`], or [`DIG_FADE`]
/// for a dig job's).
pub(super) struct Fade {
    pub(super) first: u64,
    pub(super) from: Vec<u32>,
    pub(super) frame: u32,
    pub(super) frames: u32,
}

/// A chunk's dig bricks (slots), and whether one of them is in ring 1.
#[derive(Clone, Debug, Default)]
pub(super) struct DigChunk {
    pub(super) slots: std::collections::BTreeSet<u16>,
    pub(super) near: bool,
}

/// What the frame hands the queue's step.
pub struct QueueView<'a> {
    /// The camera's position and forward direction, in voxels.
    pub eye: [f32; 3],
    pub forward: [f32; 3],
    /// Whether a chunk is resident full (a streamed world); `None` for a bounded world, whose
    /// mirror holds every chunk it has and whose absent chunks are empty.
    pub resident: Option<&'a dyn Fn(ChunkCoord) -> bool>,
    /// Samples this step draws at most; `None` sizes it by [`BakeQueue::budget`].
    pub samples: Option<u64>,
    /// The last frame's interval in seconds, which a per-second budget sizes the slice by
    /// ([`Budget::slice_ms`]); 0 when unknown (the first step), which gives its floor.
    pub interval_s: f64,
    /// Chunks from the focus the light-sample ranges are held out to; `None` the window's far
    /// corner plus the light radius.
    pub hold: Option<i32>,
    /// The wider of the view's horizontal and vertical half-angles, in radians
    /// ([`view_half_angle`]); the queue's view cone adds [`QUEUE_VIEW_MARGIN_DEG`] to it.
    pub half_angle: f32,
    /// The view's own horizontal and vertical half-angles, in radians, for the reading of the
    /// chunks the cone holds outside the frustum widened by the same margin
    /// ([`QueueStats::cone_only`]); `None` reads none. The frustum's up is the camera's, with
    /// no roll: `right = forward x Z`, `up = right x forward`.
    pub frustum: Option<[f32; 2]>,
    /// The streamer's full radius in chunks about the camera's chunk, for the census's count
    /// of the candidates whose reach crosses it ([`QueueStats::reach_bound`]); `None` (a
    /// bounded world) counts none.
    pub full_radius: Option<i32>,
}

/// The live queue (ADR-0046's scheduler, Plan 0059 Phase 8 as re-read at Phase 7b): the brick
/// level of every full chunk in the mirror, baked a slice a frame on the render thread. A chunk
/// is opened in [`BakeQueue::order`] (ADR-0048's score of distance and angle to the view by
/// default; [`QueueOrder`]), once every chunk within the light radius of it is mirrored or known
/// empty and every emitter range its table reads is held. Its faces are
/// listed from the stream workers' masks ([`FacePrep`]) where they arrived, else inline. A first
/// block is written directly; a block that replaces one of the same layout fades over [`FADE`]
/// frames. A chunk leaving the mirror frees its block.
///
/// **Coarse first** (Plan 0059 Phase 8c). With `coarse_samples` above 0, a chunk with no block
/// beyond `near` chunks (Chebyshev) of the camera's is first baked at `coarse_samples` a brick
/// direction, its block flagged coarse ([`TABLE_COARSE`]); one within `near` at the full count.
/// Under the distance order and the score's tier ([`Refine::Tier`]) every chunk with no block
/// comes before every coarse one, so a chunk the mirror gains is lit coarse at the head of the
/// queue, and the coarse ones are then refined in order; under the score's refine weight a
/// coarse chunk takes its place in the one list, its score times the weight. A
/// refinement lists the same faces, so its block has the coarse one's layout and fades from it.
///
/// **Digs** (ADR-0046's rings, Plan 0059 Phase 9 as re-read at Phase 7b). An edit's box
/// ([`BakeQueue::invalidate`]) is read at the next step against the store as it is then: the
/// bricks within [`DIG_REACH`] voxels of it ([`dig_ring`]) re-bake at the brick level ahead of
/// every other job, the chunks holding a brick the box dilated by one voxel touches first (ring
/// 1), the rest by distance (ring 2); each chunk is listed again and keeps every other brick's
/// record. While a dig's bricks wait the slice grows by [`BakeQueue::edit_ms`]. A chunk with no
/// block is left to its first pass. When a touched brick held light before the edit, every chunk
/// with a block within the light radius of the box re-bakes whole (ring 3), after the first
/// passes and with the refinements, fading. A chunk re-dug while its dig job runs throws the
/// job's block away and re-bakes the union of its bricks.
pub struct BakeQueue {
    pub settings: BakeSettings,
    /// The bake budget: the slice a frame, or milliseconds a second sized by the view's
    /// interval ([`QueueView::interval_s`]).
    pub budget: Budget,
    /// A brick direction's samples in the coarse pass; 0 bakes every chunk at the full count.
    pub coarse_samples: u32,
    /// Chunks (Chebyshev) around the camera's that the score's angle term and the withdrawal
    /// treat as the near shell.
    pub near: i32,
    /// Chunks (Chebyshev) around the camera's within which a chunk with no block takes the full
    /// count directly; beyond it, or everywhere when `None`, its first pass is the coarse pass
    /// and a chunk refines as class 1. [`Frame::set_bake_coarse`](crate::Frame::set_bake_coarse)
    /// sets it to `Some(near)`.
    pub full_first: Option<i32>,
    /// Milliseconds a frame the slice grows by while a dig's bricks wait.
    pub edit_ms: f64,
    /// Whether a dig that touched lit bricks re-bakes the chunks within the light radius (ring 3).
    pub reach_ring: bool,
    /// The order candidates open in.
    pub order: QueueOrder,
    /// Whether the queue withdraws the jobs of chunks the camera has passed (ADR-0048): at a
    /// step with a candidate in the near shell or the view cone ready, a job that is not a
    /// dig's and not a whole re-bake of a full block, whose chunk lies outside the near shell
    /// and outside the keep cone (the view cone's margin doubled), and that opened before the
    /// camera's chunk last changed, is taken out of the loop, its place freed; its chunk keeps
    /// the block it had and is a candidate again. A still camera withdraws nothing. A step with
    /// a sample count (the pre-roll) withdraws nothing.
    pub withdraw: bool,
    /// Whether a chunk whose reach is not resident full bakes from the chunks that are
    /// ([`ReachMode`]; [`BakeQueue::set_reach_mode`]).
    pub(super) reach_mode: ReachMode,
    /// Edit boxes since the last step.
    pub(super) boxes: Vec<[[i32; 3]; 2]>,
    /// Chunks with dig bricks to re-bake, not yet in a job; and those in a job, with the bricks
    /// the job re-bakes.
    pub(super) digs: BTreeMap<[i32; 3], DigChunk>,
    pub(super) dig_open: std::collections::HashMap<[i32; 3], DigChunk>,
    /// Chunks with a block that re-bake whole (ring 3, or a job thrown away by an edit).
    pub(super) reach: std::collections::BTreeSet<[i32; 3]>,
    /// Dug chunks' faces, listed at the edit for their provisional block, for their dig job.
    pub(super) listed: std::collections::HashMap<[i32; 3], ChunkFaces>,
    /// The jobs pending or open, and whether each is a coarse pass.
    pub(super) job_coarse: std::collections::HashMap<[i32; 3], bool>,
    pub(super) lp: JobLoop,
    pub(super) prepared: std::collections::HashMap<[i32; 3], Arc<PreparedMasks>>,
    pub(super) prepared_bytes: usize,
    /// The camera's chunk at the last step.
    pub(super) eye: [i32; 3],
    /// The steps at which the camera's chunk changed, counted since the queue started.
    pub(super) eye_moves: u64,
    /// Each listed job's [`BakeQueue::eye_moves`] when it opened: a job is withdrawn only once
    /// the camera's chunk has changed since (ADR-0048's Outcome), so a still camera withdraws
    /// nothing and the queue drains.
    pub(super) job_eye: std::collections::HashMap<[i32; 3], u64>,
    pub(super) faceless: std::collections::HashSet<[i32; 3]>,
    /// Chunks an edit touched while their job ran: the job's block is thrown away.
    pub(super) stale: std::collections::HashSet<[i32; 3]>,
    /// Candidates not ready at a check, and the step that checked them.
    pub(super) blocked: std::collections::HashMap<[i32; 3], u64>,
    pub(super) fades: BTreeMap<[i32; 3], Fade>,
    pub(super) steps: u64,
    /// The step before which no candidate is sorted: set when none was ready.
    pub(super) quiet_until: u64,
    /// Whether the last count of the depth ran with no job pending or open: a job read back
    /// after the count can leave a candidate it did not see (a coarse block to refine, a chunk
    /// whose job an edit threw away).
    pub(super) counted_idle: bool,
    /// The mirror's window and chunk count at the last step: a change counts the depth again.
    pub(super) mirror_seen: (Option<ChunkExtent>, usize),
    /// [`face_classes`] of the world's table, and each row's translucency it was made from.
    pub(super) classes: Option<(Vec<bool>, Arc<Vec<u8>>)>,
    /// The block a fade shows this step, kept between steps.
    pub(super) fade_scratch: Vec<u32>,
    /// Chunks that entered the view cone or the near shell with no full block: the step they
    /// entered at, and whether a block has landed since. It can hold most of the mirror's full
    /// chunks, so it is pruned to the chunks the mirror holds full at each census, not at every
    /// step: an entry whose chunk left and came back between two censuses keeps its old step.
    pub(super) entered: std::collections::HashMap<[i32; 3], (u64, bool)>,
    /// Steps from entering to the first block and to the full block, since the queue started.
    pub(super) ages_first: Vec<u32>,
    pub(super) ages_full: Vec<u32>,
    /// For each candidate found ready since the last [`BakeQueue::take_angle_needs`], the angle
    /// weight at which the nearest candidate in the cone or shell would have outranked it.
    pub(super) angle_needs: Vec<f32>,
    /// The step of the last reach census of the view ([`QueueStats::view_reach`]).
    pub(super) reach_census_at: u64,
    /// Whether the next opening reads the view's census ([`BakeQueue::request_census`]).
    pub(super) census_due: bool,
    /// Every opening reads the view's census, the in-view ages and the angle weights, not only
    /// one after a [`BakeQueue::request_census`]: a reading, which costs the step about a
    /// census (0.8 ms over the streamed cave's 17 000 candidates) at each opening.
    pub census_every_opening: bool,
    pub(super) stats: QueueStats,
    /// The current step's stage laps.
    pub(super) laps: Laps,
    /// The candidates, kept between steps: every full chunk in the mirror with no block, a
    /// coarse one or a whole re-bake waiting (ring 3), not known faceless, not in a job and
    /// not a dig's, with its class (1 when it holds a block) and whether its block is coarse.
    /// Re-read for the chunks the mirror, the store or the queue changed ([`BakeQueue::touched`]).
    pub(super) pool: std::collections::HashMap<[i32; 3], (u8, bool)>,
    /// Chunks whose candidacy is read again at the next opening.
    pub(super) touched: ChunkSet,
    /// The pool is read again whole at the next opening: at the start, and when the mirror's
    /// or the store's changes went untracked.
    pub(super) pool_stale: bool,
    /// The jobs' chunks at the last reading of the pool: a chunk joining or leaving a job is
    /// read again.
    pub(super) pool_jobs: std::collections::HashSet<[i32; 3]>,
    /// The pool's candidates in their order, and the view they were scored under; dirty when
    /// a candidate joined the pool or changed class.
    pub(super) ordered: Ordered,
    pub(super) order_view: Option<OrderView>,
    pub(super) order_dirty: bool,
    /// Each checked candidate's reach verdict ([`reach_ready`]), dropped when a chunk within
    /// the light radius and one of it changes in the mirror.
    pub(super) reach_cache: ChunkMap<Reach>,
    /// Chunks whose block was baked from a partial reach ([`ReachMode::Partial`]).
    pub(super) partial: ChunkSet,
    /// The jobs pending or open that bake from a partial reach.
    pub(super) job_partial: ChunkSet,
    /// Chunks the opening's checks found bakeable from a partial reach, read when listed.
    pub(super) partial_checked: ChunkSet,
    /// Flagged chunks (block or job) a chunk within whose reach arrived full: candidates again,
    /// as a refinement.
    pub(super) partial_due: ChunkSet,
}

/// Whether every chunk whose box lies within `lr` chunks of `coord`'s (the candidate table's
/// reach) is mirrored full and current, or known empty: in a bounded world (`resident` `None`) a
/// chunk the mirror does not wait to upload; in a streamed one a chunk resident full whose
/// record is current.
pub(super) fn reach_ready(
    run: &BakeRun<'_>,
    coord: ChunkCoord,
    lr: i32,
    resident: Option<&dyn Fn(ChunkCoord) -> bool>,
) -> bool {
    reach_of(run, coord, lr, resident, true) == Reach::Ready
}

/// `coord`'s reach as [`reach_ready`] reads it: [`Reach::Waiting`] at the first chunk within it
/// waiting to upload, else [`Reach::Partial`] if a chunk within it is not resident full (a
/// streamed world only), else [`Reach::Ready`]. With `first`, the walk stops at the first chunk
/// that is not ready either way and reads it as [`Reach::Partial`]: a caller that asks only
/// whether the reach is ready ([`ReachMode::Full`], the census) walks no further than that.
pub(super) fn reach_of(
    run: &BakeRun<'_>,
    coord: ChunkCoord,
    lr: i32,
    resident: Option<&dyn Fn(ChunkCoord) -> bool>,
    first: bool,
) -> Reach {
    let gpu = run.gpu;
    if resident.is_none() && gpu.pending_chunks() == 0 {
        return Reach::Ready;
    }
    let r = lr + 1;
    let mut partial = false;
    for z in -r..=r {
        for y in -r..=r {
            for x in -r..=r {
                if !in_reach([x, y, z], lr) {
                    continue;
                }
                let n = ChunkCoord([coord.0[0] + x, coord.0[1] + y, coord.0[2] + z]);
                if gpu.is_pending(n) {
                    return Reach::Waiting;
                }
                if let Some(res) = resident
                    && !(res(n) && gpu.is_mirrored(n) && !gpu.is_lod(n))
                {
                    if first {
                        return Reach::Partial;
                    }
                    partial = true;
                }
            }
        }
    }
    if partial {
        Reach::Partial
    } else {
        Reach::Ready
    }
}

impl BakeQueue {
    /// A queue whose budget is `slice_ms` a frame ([`Budget::PerFrame`]).
    pub fn new(settings: BakeSettings, slice_ms: f64) -> Self {
        Self {
            settings,
            budget: Budget::PerFrame { ms: slice_ms },
            coarse_samples: DEFAULT_COARSE_SAMPLES,
            near: DEFAULT_NEAR,
            full_first: Some(DEFAULT_NEAR),
            edit_ms: DEFAULT_BAKE_EDIT_MS,
            reach_ring: true,
            order: QueueOrder::default(),
            withdraw: true,
            boxes: Vec::new(),
            digs: BTreeMap::new(),
            dig_open: Default::default(),
            reach: Default::default(),
            listed: Default::default(),
            job_coarse: Default::default(),
            lp: {
                let mut lp = JobLoop::new(Vec::new(), LOOP_SLOTS..OPEN_JOBS).timed();
                lp.ended = Some(Vec::new());
                lp
            },
            prepared: Default::default(),
            prepared_bytes: 0,
            eye: [0; 3],
            eye_moves: 0,
            job_eye: Default::default(),
            faceless: Default::default(),
            stale: Default::default(),
            blocked: Default::default(),
            fades: BTreeMap::new(),
            steps: 0,
            quiet_until: 0,
            counted_idle: false,
            mirror_seen: (None, 0),
            classes: None,
            fade_scratch: Vec::new(),
            entered: Default::default(),
            ages_first: Vec::new(),
            ages_full: Vec::new(),
            angle_needs: Vec::new(),
            reach_census_at: 0,
            census_due: true,
            census_every_opening: false,
            stats: QueueStats {
                ns_per_sample: 20.0,
                ..QueueStats::default()
            },
            laps: Laps::new(stopwatch()),
            pool: Default::default(),
            touched: Default::default(),
            pool_stale: true,
            pool_jobs: Default::default(),
            ordered: Ordered::default(),
            order_view: None,
            order_dirty: true,
            reach_cache: Default::default(),
            reach_mode: ReachMode::Full,
            partial: Default::default(),
            job_partial: Default::default(),
            partial_checked: Default::default(),
            partial_due: Default::default(),
        }
    }

    /// Whether `c` is a candidate ([`BakeQueue::pool`]): its class and whether its block is
    /// coarse.
    pub(super) fn candidacy(
        &self,
        store: &Store,
        gpu: &GpuWorld,
        in_job: &std::collections::HashSet<[i32; 3]>,
        c: [i32; 3],
    ) -> Option<(u8, bool)> {
        let cc = ChunkCoord(c);
        if !gpu.is_mirrored(cc) || gpu.is_lod(cc) {
            return None;
        }
        let has = store.blocks.contains_key(&c);
        let coarse = has && store.is_coarse(cc);
        let wants = !has
            || coarse
            || self.reach.contains(&c)
            || (!self.partial_due.is_empty() && self.partial_due.contains(&c));
        (wants
            && !self.faceless.contains(&c)
            && !in_job.contains(&c)
            && !self.digs.contains_key(&c))
        .then_some((u8::from(has), coarse))
    }

    /// Brings the pool up to date: whole when it is stale, else for the chunks touched since,
    /// the store's changed blocks and the chunks that joined or left a job. A candidate that
    /// joins or changes class leaves the kept order dirty; one that leaves is skipped by the
    /// walk.
    pub(super) fn sync_pool(&mut self, run: &BakeRun<'_>, store: &mut Store) {
        let gpu = run.gpu;
        let in_job: std::collections::HashSet<[i32; 3]> = self.lp.chunks().map(|c| c.0).collect();
        match store.take_touched() {
            Some(t) => self.touched.extend(t),
            None => self.pool_stale = true,
        }
        if self.pool_stale {
            let pool: std::collections::HashMap<[i32; 3], (u8, bool)> = gpu
                .full_chunks()
                .filter_map(|c| self.candidacy(store, gpu, &in_job, c.0).map(|v| (c.0, v)))
                .collect();
            self.stats.scanned += gpu.mirrored_chunks() as u64;
            self.pool = pool;
            self.pool_stale = false;
            self.order_dirty = true;
            self.touched.clear();
        } else {
            let moved: Vec<[i32; 3]> = self
                .pool_jobs
                .symmetric_difference(&in_job)
                .copied()
                .collect();
            self.touched.extend(moved);
            for c in std::mem::take(&mut self.touched) {
                let now = self.candidacy(store, gpu, &in_job, c);
                let was = self.pool.get(&c).copied();
                if now == was {
                    continue;
                }
                match now {
                    Some(v) => {
                        self.pool.insert(c, v);
                        self.order_dirty = true;
                    }
                    None => {
                        self.pool.remove(&c);
                    }
                }
            }
        }
        self.pool_jobs = in_job;
        #[cfg(debug_assertions)]
        {
            let jobs = &self.pool_jobs;
            let whole: std::collections::HashMap<[i32; 3], (u8, bool)> = gpu
                .full_chunks()
                .filter_map(|c| self.candidacy(store, gpu, jobs, c.0).map(|v| (c.0, v)))
                .collect();
            debug_assert_eq!(self.pool, whole, "the kept pool against a whole reading");
        }
    }

    /// [`reach_ready`] through the cache of verdicts; a bounded world with nothing waiting to
    /// upload is ready without a walk. A miss counts in [`QueueStats::reach_checks`].
    pub(super) fn reach_cached(
        &mut self,
        run: &BakeRun<'_>,
        coord: ChunkCoord,
        lr: i32,
        resident: Option<&dyn Fn(ChunkCoord) -> bool>,
    ) -> Reach {
        if resident.is_none() && run.gpu.pending_chunks() == 0 {
            return Reach::Ready;
        }
        if let Some(&v) = self.reach_cache.get(&coord.0) {
            // A streamer's residency can move a frame ahead of the mirror; a bounded world's
            // verdict reads the mirror alone.
            debug_assert!(
                resident.is_some()
                    || v == reach_of(run, coord, lr, resident, self.reach_mode == ReachMode::Full),
                "the cached reach of {coord:?}"
            );
            return v;
        }
        self.stats.reach_checks += 1;
        let v = reach_of(run, coord, lr, resident, self.reach_mode == ReachMode::Full);
        self.reach_cache.insert(coord.0, v);
        v
    }

    /// Whether a chunk whose reach is not resident full bakes from the chunks that are.
    pub fn reach_mode(&self) -> ReachMode {
        self.reach_mode
    }

    /// Sets [`BakeQueue::reach_mode`]. A change drops the cached reach verdicts: under
    /// [`ReachMode::Full`] a walk stops at the first chunk not resident full, so its
    /// [`Reach::Partial`] does not say that nothing in the reach waits to upload.
    pub fn set_reach_mode(&mut self, mode: ReachMode) {
        if mode != self.reach_mode {
            self.reach_mode = mode;
            self.reach_cache.clear();
        }
    }

    /// Whether `coord` may bake under [`BakeQueue::reach`], and whether from a partial reach:
    /// `Some(false)` when its reach is ready, `Some(true)` under [`ReachMode::Partial`] when
    /// nothing in its reach waits to upload and the reach crosses the streamer's full radius
    /// about `eye`: a chunk within it lies past the full sphere (`d2 > r^2` in chunks, the
    /// streamer's rule), so a still camera never makes it whole. Else `None`.
    pub(super) fn bakeable(
        &mut self,
        run: &BakeRun<'_>,
        coord: ChunkCoord,
        lr: i32,
        view: &QueueView<'_>,
        eye: [i32; 3],
    ) -> Option<bool> {
        match self.reach_cached(run, coord, lr, view.resident) {
            Reach::Ready => Some(false),
            Reach::Partial
                if self.reach_mode == ReachMode::Partial
                    && view.full_radius.is_some_and(|r| {
                        let o = [0, 1, 2].map(|k| coord.0[k] - eye[k]);
                        reach_offsets(lr).any(|n| {
                            (0..3).map(|k| i64::from(o[k] + n[k]).pow(2)).sum::<i64>()
                                > i64::from(r).pow(2)
                        })
                    }) =>
            {
                Some(true)
            }
            _ => None,
        }
    }

    /// Re-queues as a refinement each chunk flagged partial (its block, or its job's) within
    /// whose reach a chunk of `changed` is now held full: a chunk arriving, or a held one
    /// changing. `None` (the changes went untracked) re-queues every flagged chunk. Nothing to
    /// do under [`ReachMode::Full`], which flags none.
    pub(super) fn requeue_partial(&mut self, gpu: &GpuWorld, changed: Option<&[ChunkCoord]>) {
        if self.partial.is_empty() && self.job_partial.is_empty() {
            return;
        }
        let due: Vec<[i32; 3]> = match changed {
            None => self
                .partial
                .iter()
                .chain(self.job_partial.iter())
                .copied()
                .collect(),
            Some(ch) => {
                let lr = gpu.light_radius();
                let held: ChunkSet = ch
                    .iter()
                    .filter(|c| gpu.is_mirrored(**c) && !gpu.is_lod(**c))
                    .map(|c| c.0)
                    .collect();
                if held.is_empty() {
                    return;
                }
                let flagged =
                    |c: &[i32; 3]| self.partial.contains(c) || self.job_partial.contains(c);
                let mut due = Vec::new();
                // Each side walks its reach over the other's set: the smaller side walks.
                if self.partial.len() + self.job_partial.len() <= held.len() {
                    for p in self.partial.iter().chain(self.job_partial.iter()) {
                        let near = reach_offsets(lr)
                            .any(|o| held.contains(&[p[0] + o[0], p[1] + o[1], p[2] + o[2]]));
                        if near {
                            due.push(*p);
                        }
                    }
                } else {
                    for n in &held {
                        for o in reach_offsets(lr) {
                            let p = [n[0] + o[0], n[1] + o[1], n[2] + o[2]];
                            if flagged(&p) {
                                due.push(p);
                            }
                        }
                    }
                }
                due
            }
        };
        for p in due {
            if self.partial_due.insert(p) {
                self.stats.partial_requeued += 1;
                self.touched.insert(p);
                self.counted_idle = false;
                self.quiet_until = self.steps;
            }
        }
    }

    /// Drops the cached reach verdicts within `lr + 1` chunks (Chebyshev) of a chunk of
    /// `changed`; too many pairs to compare drop them all.
    pub(super) fn drop_reach(&mut self, changed: &[ChunkCoord], lr: i32) {
        if changed.is_empty() || self.reach_cache.is_empty() {
            return;
        }
        if changed.len().saturating_mul(self.reach_cache.len()) > 1 << 22 {
            self.reach_cache.clear();
            return;
        }
        let r = lr + 1;
        // The verdicts within `r` (Chebyshev) of a changed chunk go. Removing each changed
        // chunk's neighbourhood by key costs `(2 r + 1)^3` lookups a change; a scan of the cache
        // costs its length times the changes. In a flight a few chunks change a step against
        // thousands of cached verdicts, so the keys are removed directly unless the scan is
        // smaller. Either way the same verdicts go.
        let side = (2 * r + 1) as usize;
        if changed.len().saturating_mul(side * side * side) < self.reach_cache.len() {
            for n in changed {
                for dz in -r..=r {
                    for dy in -r..=r {
                        for dx in -r..=r {
                            self.reach_cache
                                .remove(&[n.0[0] + dx, n.0[1] + dy, n.0[2] + dz]);
                        }
                    }
                }
            }
            return;
        }
        self.reach_cache.retain(|c, _| {
            !changed
                .iter()
                .any(|n| (0..3).all(|k| (n.0[k] - c[k]).abs() <= r))
        });
    }

    pub fn stats(&self) -> QueueStats {
        self.stats
    }

    /// Steps (frames) from a chunk entering the view cone or the near shell with no full block
    /// to its first block, and to its full block, one entry a chunk since the queue started. A
    /// chunk that entered with a coarse block reads 0 to its first. Entry is read at the
    /// openings that read the census ([`BakeQueue::census_every_opening`] for all of them), so
    /// it lags by the steps between two.
    pub fn view_ages(&self) -> (&[u32], &[u32]) {
        (&self.ages_first, &self.ages_full)
    }

    /// The pool words the places of the jobs pending and open hold: with the store's blocks,
    /// its header and its free words, the whole pool.
    pub fn job_words(&self) -> u64 {
        self.lp.place_words(self.record_words())
    }

    /// The angle weights read since the last call: for each candidate found ready, the weight
    /// above which the nearest candidate in the view cone or the near shell at that opening
    /// (ready or not) would have outranked it under the score at a refine weight of 1; 0 for
    /// one in the cone or shell, or with no nearer such candidate.
    pub fn take_angle_needs(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.angle_needs)
    }

    /// Asks the next opening for the view's census: [`QueueStats::view_waiting`],
    /// [`QueueStats::weight_census`], [`QueueStats::cone_only`], [`QueueStats::view_median`]
    /// and [`QueueStats::view_reach`] keep the last census's values until then. A caller that
    /// shows them once a second asks once a second; [`BakeQueue::census_every_opening`] reads
    /// them at every opening.
    pub fn request_census(&mut self) {
        self.census_due = true;
    }

    /// Where `c` lies against `view` from the camera's chunk `eye`: [`VIEW_SHELL`],
    /// [`VIEW_IN`] or [`VIEW_OUT`].
    pub(super) fn view_class(&self, c: [i32; 3], eye: [i32; 3], view: &QueueView<'_>) -> usize {
        let o = [0, 1, 2].map(|k| c[k] - eye[k]);
        if in_near_shell(o, self.near) {
            VIEW_SHELL
        } else if angle_term(o, view.forward, view.half_angle, self.near) <= 0.0 {
            VIEW_IN
        } else {
            VIEW_OUT
        }
    }

    /// The chunks of the jobs open and then pending, each part in its order: the order the
    /// queue opened them in, but where a dig was promoted ahead.
    pub fn jobs(&self) -> Vec<ChunkCoord> {
        let open = self.lp.open.iter().map(|j| j.coord);
        open.chain(self.lp.pending.iter().map(|j| j.coord))
            .collect()
    }

    /// The names of the per-chunk sets that hold `c`: what a chunk leaving the mirror must not
    /// leave behind.
    pub fn tracks(&self, c: ChunkCoord) -> Vec<&'static str> {
        let c = c.0;
        [
            ("fades", self.fades.contains_key(&c)),
            ("digs", self.digs.contains_key(&c)),
            ("dig_open", self.dig_open.contains_key(&c)),
            ("reach", self.reach.contains(&c)),
            ("listed", self.listed.contains_key(&c)),
            ("faceless", self.faceless.contains(&c)),
            ("blocked", self.blocked.contains_key(&c)),
            ("job_coarse", self.job_coarse.contains_key(&c)),
            ("stale", self.stale.contains(&c)),
            ("partial", self.partial.contains(&c)),
            ("job_partial", self.job_partial.contains(&c)),
            ("partial_due", self.partial_due.contains(&c)),
        ]
        .into_iter()
        .filter_map(|(name, held)| held.then_some(name))
        .collect()
    }

    /// No job pending or open, no block fading, and no candidate at the last count, which ran
    /// with no job in the loop.
    pub fn empty(&self) -> bool {
        self.lp.idle()
            && self.fades.is_empty()
            && self.counted_idle
            && self.stats.depth == 0
            && self.boxes.is_empty()
            && self.digs.is_empty()
            && self.reach.is_empty()
            && self.partial_due.is_empty()
    }

    /// Whether an edit box waits for the next step.
    pub fn boxes_waiting(&self) -> bool {
        !self.boxes.is_empty()
    }

    /// No edit box, dig brick or ring-3 chunk waits, and no dig job runs.
    pub fn digs_settled(&self) -> bool {
        self.boxes.is_empty()
            && self.digs.is_empty()
            && self.dig_open.is_empty()
            && self.reach.is_empty()
    }

    /// Whether `c` holds its block at the full count or is known to have none: no face, or
    /// (`absent`) no chunk.
    pub fn settled(&self, store: &Store, c: ChunkCoord, absent: bool) -> bool {
        ((store.blocks.contains_key(&c.0) && !store.is_coarse(c))
            || self.faceless.contains(&c.0)
            || absent)
            && !self.lp.chunks().any(|j| j == c)
            && !self.digs.contains_key(&c.0)
            && !self.reach.contains(&c.0)
            && !self.partial_due.contains(&c.0)
    }

    /// Whether `c` has had its first pass: it holds a block (coarse or full), is known to have
    /// no face, or its reach or lights were not ready at its last check.
    pub fn passed(&self, store: &Store, c: ChunkCoord) -> bool {
        store.blocks.contains_key(&c.0)
            || self.faceless.contains(&c.0)
            || self.blocked.contains_key(&c.0)
    }

    /// [`face_classes`] of `materials`, made again only when a row's translucency changed.
    pub(super) fn classes(&mut self, materials: &MaterialTable) -> Arc<Vec<u8>> {
        let rows = materials.rows();
        let same = self.classes.as_ref().is_some_and(|(k, _)| {
            k.len() == rows.len() && k.iter().zip(rows).all(|(t, m)| *t == m.is_translucent())
        });
        if !same {
            let key = rows.iter().map(|m| m.is_translucent()).collect();
            self.classes = Some((key, Arc::new(face_classes(materials))));
        }
        self.classes.as_ref().expect("made above").1.clone()
    }

    /// Takes the stream workers' results; one made under other classes than `materials`' is
    /// dropped.
    pub fn take_prepared(
        &mut self,
        prepared: &[(ChunkCoord, vxe_world::Prepared)],
        materials: &MaterialTable,
    ) {
        let classes = self.classes(materials);
        let classes = classes.as_slice();
        for (c, p) in prepared {
            let Ok(m) = p.0.clone().downcast::<PreparedMasks>() else {
                continue;
            };
            self.stats.hook_chunks += 1;
            self.stats.hook_ns += m.ns;
            self.stats.hook_max_ns = self.stats.hook_max_ns.max(m.ns);
            if m.classes.as_slice() != classes {
                continue;
            }
            self.prepared_bytes += m.masks.bytes();
            if let Some(old) = self.prepared.insert(c.0, m) {
                self.prepared_bytes -= old.masks.bytes();
            }
            self.faceless.remove(&c.0);
            self.touched.insert(c.0);
            self.blocked.remove(&c.0);
        }
        if self.prepared_bytes > PREPARED_BYTES_MAX {
            // The farthest from the last step's eye go first, to nine tenths of the cap.
            let eye = self.eye;
            let mut by: Vec<(i64, [i32; 3])> = self
                .prepared
                .keys()
                .map(|c| {
                    let d: i64 = (0..3).map(|k| i64::from(c[k] - eye[k]).pow(2)).sum();
                    (d, *c)
                })
                .collect();
            by.sort_unstable();
            while self.prepared_bytes > PREPARED_BYTES_MAX / 10 * 9 {
                let Some((_, c)) = by.pop() else { break };
                if let Some(m) = self.prepared.remove(&c) {
                    self.prepared_bytes -= m.masks.bytes();
                }
            }
        }
    }

    /// An edit's box (`min ..= max`, voxels): the chunks within a voxel of it lose their prepared
    /// masks and their known facelessness, and a running job's block for them is thrown away; the
    /// box's rings are read at the next step.
    pub fn invalidate(&mut self, min: [i32; 3], max: [i32; 3]) {
        self.boxes.push([min, max]);
        let lo = min.map(|v| (v - 1).div_euclid(64));
        let hi = max.map(|v| (v + 1).div_euclid(64));
        for z in lo[2]..=hi[2] {
            for y in lo[1]..=hi[1] {
                for x in lo[0]..=hi[0] {
                    let c = [x, y, z];
                    if let Some(m) = self.prepared.remove(&c) {
                        self.prepared_bytes -= m.masks.bytes();
                    }
                    self.faceless.remove(&c);
                    self.touched.insert(c);
                    self.blocked.remove(&c);
                    if self.lp.chunks().any(|j| j.0 == c) {
                        self.stale.insert(c);
                    }
                }
            }
        }
    }
}
