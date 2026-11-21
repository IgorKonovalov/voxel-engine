//! The live bake queue: its state, its settings and its views.

use super::*;

mod digs;
mod keys;
mod list;
mod open;
mod reach;
mod rule;
mod sealed;
mod stats;
mod step;
mod tiers;

pub use self::open::in_view_census;
use self::reach::reach_of;
pub use self::stats::{Band, Bands, InViewWaiting, ListingCensus, QueueStats, chunk_nearest};
pub use self::tiers::DEFAULT_TIERS;

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

/// A chunk's dig bricks (slots), and whether one of them is in ring 1. `whole`: a chunk the
/// edit's box touched that held no block (a stamp into air), whose every listed brick is a dig
/// brick, its slots filled at its opening.
#[derive(Clone, Debug, Default)]
pub(super) struct DigChunk {
    pub(super) slots: std::collections::BTreeSet<u16>,
    pub(super) near: bool,
    pub(super) whole: bool,
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
    /// The streamer's LOD radius in chunks about the camera's chunk: the chunks it holds or
    /// will hold, which a column must have mirrored before the bake reads its top
    /// ([`reach_columns_final`]); `None` waits only for the chunks pending upload.
    pub lod_radius: Option<i32>,
    /// The output's focal length in pixels ([`Bands::focal_px`]), which with
    /// [`BakeQueue::far_normal`] sets each chunk's [`Band`]; `None` reads every chunk as near.
    pub focal_px: Option<f32>,
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
    /// The slice's two values (ADR-0080 rule 3): the drain value while a chunk of
    /// [`BakeQueue::view_waiting`] waits, the line's once none does; `None` is the budget alone.
    pub drain: Option<Drain>,
    /// The chunks in view the last census found waiting for their tier's record, each removed
    /// as its record lands at its band's count, or as it leaves the mirror or turns out to hold
    /// no face.
    pub(super) view_waiting: ChunkSet,
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
    /// The stream workers' no-face verdicts ([`crate::bake::seal`]), each with its point of the
    /// shells' order, and their handle once a result brought it ([`sealed::Sealed`]).
    pub(super) sealed: sealed::Sealed,
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
    /// Each verdict with whether its walk stopped at the first chunk not resident full
    /// ([`reach_of`]'s `first`), whose [`Reach::Partial`] does not say that nothing in the reach
    /// waits to upload.
    pub(super) reach_cache: ChunkMap<(Reach, bool)>,
    /// Chunks whose block was baked from a partial reach ([`ReachMode::Partial`]).
    pub(super) partial: ChunkSet,
    /// The jobs pending or open that bake from a partial reach.
    pub(super) job_partial: ChunkSet,
    /// Chunks the opening's checks found bakeable from a partial reach, read when listed.
    pub(super) partial_checked: ChunkSet,
    /// Flagged chunks (block or job) a chunk within whose reach arrived full: candidates again,
    /// as a refinement.
    pub(super) partial_due: ChunkSet,
    /// The step's verdicts of [`GpuWorld::column_final`] by chunk column, emptied each step.
    pub(super) columns_final: std::collections::HashMap<[i32; 2], bool>,
    /// [`crate::Lighting::column_wait`], set by the frame each step.
    pub column_wait: bool,
    /// [`crate::Lighting::key_step`] in degrees, set by the frame each step: a block re-bakes
    /// its key visibilities alone once the key light has turned past it and its disc's radius
    /// from the light they were baked under ([`BakeQueue::mark_keys`]).
    pub key_step: f32,
    /// The key-only re-bakes waiting and in a job.
    pub(super) keys: keys::KeyJobs,
    /// Whether a candidate the rule answers ([`rule::buried`], no exposed emitter within the
    /// light radius) takes a zero record written on the CPU instead of a GPU job (ADR-0080).
    pub zero_rule: bool,
    /// Chunks whose block is the rule's zero record: baked again when a change near them
    /// breaks the rule ([`BakeQueue::relist_zeroed`]).
    pub(super) zeroed: ChunkSet,
    /// Chunks the opening's checks found answered by the rule, read when listed.
    pub(super) ruled_checked: ChunkSet,
    /// Ruled chunks listed this step, their place and faces: their zero blocks are placed at
    /// the step's submit.
    pub(super) ruled: Vec<(ChunkCoord, u64, ChunkFaces, u8)>,
    /// The sample tiers a brick direction by band, near, mid and far (ADR-0080 rule 2); with a
    /// view's focal length a chunk's job draws its band's count past its coarse pass, else
    /// [`BakeSettings::brick_samples`].
    pub tiers: [u32; 3],
    /// Each block's samples a brick direction ([`BakeQueue::tiers`]), and each job's.
    pub(super) block_count: std::collections::HashMap<[i32; 3], u32>,
    pub(super) job_count: std::collections::HashMap<[i32; 3], u32>,
    /// The bands the last step read ([`tiers`]), `None` without a focal length, and the centre
    /// of the camera's chunk in voxels they are read from.
    pub(super) bands: Option<Bands>,
    pub(super) eye_c: [f32; 3],
    /// The step's lowest column tops by chunk column ([`GpuWorld::column_lowest`]), emptied
    /// each step.
    pub(super) columns_lowest: std::collections::HashMap<[i32; 2], i32>,
    /// `LookParams::far_normal` (`[lo, hi]` pixels a voxel), set by the frame each step: with
    /// [`QueueView::focal_px`] the band a chunk's size on screen puts it in ([`Bands`]).
    pub far_normal: [f32; 2],
    /// Chunks whose block was baked while a column over its reach was not final (the wait
    /// off): baked again once when those columns turn final ([`BakeQueue::catch_up`]).
    pub(super) provisional: ChunkSet,
    /// The jobs pending or open that bake under a column not yet final.
    pub(super) job_provisional: ChunkSet,
    /// Chunks the opening's checks found bakeable only provisionally, read when listed.
    pub(super) provisional_checked: ChunkSet,
    /// Provisional chunks whose columns turned final: candidates again, for their catch-up.
    pub(super) provisional_due: ChunkSet,
}

/// Whether every chunk whose box lies within `lr` chunks of `coord`'s (the candidate table's
/// reach) is mirrored full and current, or known empty: in a bounded world (`resident` `None`) a
/// chunk the mirror does not wait to upload; in a streamed one a chunk resident full whose
/// record is current. And every column over the reach is final ([`reach_columns_final`]).
pub(super) fn reach_ready(
    run: &BakeRun<'_>,
    coord: ChunkCoord,
    lr: i32,
    view: &QueueView<'_>,
) -> bool {
    let held = held_by_streamer(view.eye, view.lod_radius, view.resident);
    let held = held.as_ref().map(|h| h as &dyn Fn(ChunkCoord) -> bool);
    reach_of(run, coord, lr, view.resident, true) == Reach::Ready
        && reach_columns_final(coord, lr, |x, y| run.gpu.column_final(x, y, held))
}

/// Whether every chunk column over `coord`'s reach footprint (the columns of the chunks within
/// `lr` of it, [`in_reach`]) is final by `final_column(cx, cy)`: ADR-0070's amendment, under
/// which the bake's sky rays read the column map, so a chunk bakes only once the tops over its
/// reach are the tops it will keep.
pub(super) fn reach_columns_final(
    coord: ChunkCoord,
    lr: i32,
    mut final_column: impl FnMut(i32, i32) -> bool,
) -> bool {
    let r = lr + 1;
    for y in -r..=r {
        for x in -r..=r {
            if (-r..=r).any(|z| in_reach([x, y, z], lr))
                && !final_column(coord.0[0] + x, coord.0[1] + y)
            {
                return false;
            }
        }
    }
    true
}

/// A reading hook: the chunks that hold a column over `coord`'s reach footprint from final
/// under `view` ([`reach_columns_final`], [`GpuWorld::column_final`]), each with whether it is
/// pending upload (else the streamer's sphere names it and it is not mirrored). Empty when
/// every column is final.
pub fn finality_blockers(
    gpu: &GpuWorld,
    coord: ChunkCoord,
    lr: i32,
    view: &QueueView<'_>,
) -> Vec<(ChunkCoord, bool)> {
    let held = held_by_streamer(view.eye, view.lod_radius, view.resident);
    let w = gpu.window();
    let mut out = Vec::new();
    reach_columns_final(coord, lr, |x, y| {
        for z in w.min.0[2]..w.min.0[2] + w.size()[2].max(1) {
            let c = ChunkCoord([x, y, z]);
            let pending = gpu.is_pending(c);
            if pending || held.as_ref().is_some_and(|h| h(c) && !gpu.is_mirrored(c)) {
                out.push((c, pending));
            }
        }
        true
    });
    out
}

/// Whether `coord`'s reach at light radius `lr` crosses the streamer's full radius `r_full`
/// about the eye's chunk `eye`: a chunk within the reach ([`reach_offsets`]) lies past the full
/// sphere (`d2 > r_full^2` in chunks, the streamer's rule), so a still camera never makes the
/// reach whole and [`ReachMode::Full`] never bakes `coord`.
pub fn reach_crosses(coord: ChunkCoord, eye: [i32; 3], lr: i32, r_full: i32) -> bool {
    let o = [0, 1, 2].map(|k| coord.0[k] - eye[k]);
    reach_offsets(lr).any(|n| {
        (0..3).map(|k| i64::from(o[k] + n[k]).pow(2)).sum::<i64>() > i64::from(r_full).pow(2)
    })
}

/// The chunks a column waits for before it is final ([`GpuWorld::column_final`]): in a streamed
/// world with `lod_radius`, those within it of `eye`'s chunk (the streamer's sphere, `d2 <=
/// r^2`); else none beyond the pending ones.
pub(super) fn held_by_streamer(
    eye: [f32; 3],
    lod_radius: Option<i32>,
    resident: Option<&dyn Fn(ChunkCoord) -> bool>,
) -> Option<impl Fn(ChunkCoord) -> bool> {
    let r = lod_radius.filter(|_| resident.is_some())?;
    let e = eye.map(|v| (v.floor() as i32).div_euclid(64));
    Some(move |c: ChunkCoord| {
        (0..3).map(|k| i64::from(c.0[k] - e[k]).pow(2)).sum::<i64>() <= i64::from(r).pow(2)
    })
}

impl BakeQueue {
    /// A queue whose budget is `slice_ms` a frame ([`Budget::PerFrame`]).
    pub fn new(settings: BakeSettings, slice_ms: f64) -> Self {
        Self {
            settings,
            budget: Budget::PerFrame { ms: slice_ms },
            drain: None,
            view_waiting: Default::default(),
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
            columns_final: Default::default(),
            column_wait: false,
            key_step: crate::frame::DEFAULT_KEY_STEP,
            keys: Default::default(),
            far_normal: [0.0; 2],
            zero_rule: true,
            sealed: Default::default(),
            zeroed: Default::default(),
            ruled_checked: Default::default(),
            ruled: Vec::new(),
            tiers: DEFAULT_TIERS,
            block_count: Default::default(),
            job_count: Default::default(),
            bands: None,
            eye_c: [32.0; 3],
            columns_lowest: Default::default(),
            provisional: Default::default(),
            job_provisional: Default::default(),
            provisional_checked: Default::default(),
            provisional_due: Default::default(),
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
        let coarse = has && self.lesser(store, c);
        let wants = !has
            || coarse
            || self.reach.contains(&c)
            || (!self.partial_due.is_empty() && self.partial_due.contains(&c))
            || (!self.provisional_due.is_empty() && self.provisional_due.contains(&c));
        (wants
            && !self.faceless.contains(&c)
            && !self.sealed.holds(c)
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

    /// Whether `coord` may bake under [`BakeQueue::reach`] and the column wait, whether from a
    /// partial reach and whether provisionally (a column over its reach not final, the wait
    /// off: [`BakeQueue::column_wait`]). The reach's verdict:
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
    ) -> Option<(bool, bool)> {
        let partial = self.bakeable_reach(run, coord, lr, view, eye)?;
        let fin = self.columns_final_at(run, coord, lr, view);
        if fin {
            return Some((partial, false));
        }
        if self.column_wait {
            return None;
        }
        // The wait off: no chunk of the columns may be pending upload. The block is
        // provisional where the bake reads the sky (the column map's tops may still rise).
        let gpu = run.gpu;
        reach_columns_final(coord, lr, |x, y| gpu.column_final(x, y, None))
            .then_some((partial, run.sky != 0))
    }

    /// Whether every column over `coord`'s reach footprint is final under `view`, the step's
    /// verdicts memoised by column.
    pub(super) fn columns_final_at(
        &mut self,
        run: &BakeRun<'_>,
        coord: ChunkCoord,
        lr: i32,
        view: &QueueView<'_>,
    ) -> bool {
        let held = held_by_streamer(view.eye, view.lod_radius, view.resident);
        let held = held.as_ref().map(|h| h as &dyn Fn(ChunkCoord) -> bool);
        let memo = &mut self.columns_final;
        reach_columns_final(coord, lr, |x, y| {
            *memo
                .entry([x, y])
                .or_insert_with(|| run.gpu.column_final(x, y, held))
        })
    }

    /// The catch-up (ADR-0070's amendment of the second window): each provisional block whose
    /// columns turned final is due for one more bake, behind the first bakes. Runs when the
    /// mirror changed (`changed`, or untracked) and a block is provisional.
    pub(super) fn catch_up(&mut self, run: &BakeRun<'_>, view: &QueueView<'_>, changed: bool) {
        if !changed || self.provisional.is_empty() {
            return;
        }
        let lr = run.gpu.light_radius();
        let flagged: Vec<[i32; 3]> = self
            .provisional
            .iter()
            .filter(|c| !self.provisional_due.contains(*c))
            .copied()
            .collect();
        for c in flagged {
            if self.columns_final_at(run, ChunkCoord(c), lr, view) && self.provisional_due.insert(c)
            {
                self.stats.provisional_requeued += 1;
                self.touched.insert(c);
                self.counted_idle = false;
                self.quiet_until = self.steps;
            }
        }
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

    /// The samples `c` still needs (class 1 with a block, `coarse` a coarse one): its brick
    /// directions ([`ChunkMasks::dirs_bound`] from its prepared masks, else six a brick) at its
    /// band's count ([`BakeQueue::want`]), and at the coarse count too where its first pass is
    /// still to come and is the coarse one.
    pub(super) fn need_samples(&self, c: [i32; 3], class: u8, eye: [i32; 3]) -> u64 {
        let dirs = self
            .prepared
            .get(&c)
            .map_or(CHUNK_BRICKS as u64 * 6, |m| m.masks.dirs_bound());
        let want = self.want(c);
        let first = self.count_for(c, class, eye).0;
        let per = u64::from(want) + if first < want { u64::from(first) } else { 0 };
        dirs * per
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
            ("sealed", self.sealed.holds(c)),
            ("blocked", self.blocked.contains_key(&c)),
            ("job_coarse", self.job_coarse.contains_key(&c)),
            ("stale", self.stale.contains(&c)),
            ("partial", self.partial.contains(&c)),
            ("job_partial", self.job_partial.contains(&c)),
            ("partial_due", self.partial_due.contains(&c)),
            ("provisional", self.provisional.contains(&c)),
            ("job_provisional", self.job_provisional.contains(&c)),
            ("provisional_due", self.provisional_due.contains(&c)),
            ("keys", self.keys.holds(&c)),
            ("zeroed", self.zeroed.contains(&c)),
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
            && self.provisional_due.is_empty()
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
        ((store.blocks.contains_key(&c.0) && !self.lesser(store, c.0))
            || self.faceless.contains(&c.0)
            || self.sealed.holds(c.0)
            || absent)
            && !self.lp.chunks().any(|j| j == c)
            && !self.digs.contains_key(&c.0)
            && !self.reach.contains(&c.0)
            && !self.partial_due.contains(&c.0)
            && !self.provisional_due.contains(&c.0)
    }

    /// Whether `c` has had its first pass: it holds a block (coarse or full), is known to have
    /// no face, or its reach or lights were not ready at its last check.
    pub fn passed(&self, store: &Store, c: ChunkCoord) -> bool {
        store.blocks.contains_key(&c.0)
            || self.faceless.contains(&c.0)
            || self.sealed.holds(c.0)
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
            self.stats.seal_ns += m.seal_ns;
            self.stats.seal_max_ns = self.stats.seal_max_ns.max(m.seal_ns);
            if m.classes.as_slice() != classes {
                continue;
            }
            let changed = self.sealed.take(&m);
            self.touched.extend(changed);
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
        self.sealed.forget(min, max);
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

#[cfg(test)]
mod tests;
