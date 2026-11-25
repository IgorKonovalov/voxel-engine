//! The bake half's step with no device (ADR-0083): a recording render half stands in for the
//! GPU, echoing each submit's entries back as drawn when the test lets it.

use super::shadow::{ChunkNote, LightNote, MirrorRead, notes};
use super::*;
use svet_world::{AIR, EditOp, FIXED_ONE, Fixed3, Material, MaterialTable, Mode, UniformSource};

/// Chunks along x; a rock slab fills z 0 ..= 7 across them.
const CHUNKS: [i32; 3] = [3, 1, 1];
const SLAB_TOP: i32 = 8;

fn box_op(min: [i64; 3], max: [i64; 3], mode: Mode) -> EditOp {
    EditOp::box_(
        Fixed3(min.map(|v| v * FIXED_ONE)),
        Fixed3(max.map(|v| v * FIXED_ONE)),
        mode,
    )
}

fn world() -> World {
    let mut t = MaterialTable::new();
    let rock = t.push(Material::named("rock"));
    let w = World::generate(ChunkExtent::new([0, 0, 0], CHUNKS), &UniformSource(AIR), &t);
    let max = [64 * i64::from(CHUNKS[0]) - 1, 63, i64::from(SLAB_TOP) - 1];
    w.apply(&box_op([0, 0, 0], max, Mode::Fill(rock)))
        .expect("resident");
    w
}

/// The mirror as the render half would read it: every chunk of the world held full, its uploads
/// done, no light.
struct Mirror {
    changed: Vec<ChunkCoord>,
}

impl MirrorRead for Mirror {
    fn window(&self) -> ChunkExtent {
        ChunkExtent::new([0, 0, 0], CHUNKS)
    }
    fn light_radius(&self) -> i32 {
        2
    }
    fn chunk_note(&self, _: ChunkCoord) -> ChunkNote {
        ChunkNote {
            mirrored: true,
            lod: false,
            pending: false,
        }
    }
    fn chunks(&self) -> Vec<ChunkCoord> {
        let mut out = Vec::new();
        for x in 0..CHUNKS[0] {
            out.push(ChunkCoord([x, 0, 0]));
        }
        out
    }
    fn column_lowest(&self, _: i32, _: i32) -> i32 {
        SLAB_TOP
    }
    fn light_key(&self) -> (u64, u64) {
        (0, 0)
    }
    fn light_notes(&self) -> Vec<([i32; 3], LightNote)> {
        Vec::new()
    }
}

/// One applied write: the frame it was applied in, its offset, kind and bytes.
#[derive(Clone, Debug)]
struct Applied {
    offset: u64,
    kind: usize,
    bytes: usize,
}

struct Recorder<'a> {
    out: &'a mut Vec<Applied>,
}

impl Applier for Recorder<'_> {
    fn write(&mut self, offset: u64, data: &[u8], kind: usize) {
        self.out.push(Applied {
            offset,
            kind,
            bytes: data.len(),
        });
    }
}

/// The recording render half: submits issued in order, read back as drawn unless `hold`.
#[derive(Default)]
struct Half {
    frame: u64,
    hold: bool,
    flights: VecDeque<Vec<[u32; FACE_WORDS]>>,
    /// Each submit's entries' slots in order, by the frame it was issued in.
    issued: Vec<(u64, Vec<u32>)>,
    lists: Lists,
    applied: Vec<Applied>,
}

impl RenderHalf for Half {
    fn poll(&mut self) {}
    fn tables(&mut self, _: &[(usize, ChunkCoord)]) {}
    fn refresh_tables(&mut self, _: &[(usize, ChunkCoord)]) {}
    fn write_params(&mut self) {}
    fn issue(&mut self, p: Prepared) {
        self.issued
            .push((self.frame, p.input.iter().map(|r| r[17]).collect()));
        self.flights.push_back(p.input);
    }
    fn collect(&mut self, unread: usize) -> Option<ReadBack> {
        if self.flights.is_empty() || (self.hold && self.flights.len() <= unread) {
            return None;
        }
        let input = self.flights.pop_front()?;
        Some(ReadBack {
            words: input.iter().flatten().copied().collect(),
            mapped: true,
            seconds: 0.001,
            gpu_ms: Some(0.1),
            timer_ms: None,
        })
    }
    fn cap(&self) -> Option<usize> {
        self.lists.cap
    }
    fn apply(&mut self, lists: Vec<WriteList>, frame: u64) -> Vec<([i32; 3], u32)> {
        self.lists
            .take(lists)
            .expect("no landing list over the cap");
        let mut out = Recorder {
            out: &mut self.applied,
        };
        self.lists.apply(frame, &mut out).1
    }
}

/// The bake half and its frame: the queue, the store, the shadow, and the recording half.
struct Rig {
    world: World,
    mirror: Mirror,
    shadow: Shadow,
    seen: (Option<ChunkExtent>, Option<(u64, u64)>),
    queue: BakeQueue,
    store: Store,
    half: Half,
    /// The settings each message carries: the queue's own at its start.
    settings: StepSettings,
}

impl Rig {
    fn new(cap: Option<usize>) -> Self {
        let settings = BakeSettings {
            round: 64,
            brick_samples: 64,
            ..BakeSettings::default()
        };
        let mut store = Store::new(64 << 20);
        let mirror = Mirror {
            changed: Vec::new(),
        };
        store.start_queue(mirror.window(), settings.cell);
        let _ = store.take_direct();
        let mut half = Half::default();
        half.lists.cap = cap;
        let queue = BakeQueue::new(settings, 4.0);
        let step_settings = StepSettings {
            column_wait: queue.column_wait,
            key_step: queue.key_step,
            landing_fade: queue.landing_fade,
            dig_store_hold: queue.dig_store_hold,
            landed_word: queue.landed_word,
            far_normal: queue.far_normal,
        };
        Self {
            world: world(),
            mirror,
            shadow: Shadow::default(),
            seen: (None, None),
            queue,
            store,
            half,
            settings: step_settings,
        }
    }

    /// One frame: the message (the mirror's changes since the last), then the bake half's step.
    fn step(&mut self) {
        let changed = std::mem::take(&mut self.mirror.changed);
        let (window, key) = &mut self.seen;
        let first = window.is_none();
        let n = notes(&self.mirror, (!first).then_some(changed), *window, key);
        *window = n.window;
        self.half.frame += 1;
        let view = QueueView {
            eye: [32.0, 32.0, 32.0],
            forward: [1.0, 0.0, 0.0],
            resident: None,
            samples: None,
            interval_s: 1.0 / 60.0,
            hold: None,
            half_angle: 1.0,
            frustum: None,
            full_radius: None,
            lod_radius: None,
            focal_px: None,
        };
        let msg = FrameMessage {
            start: StepStart::now(),
            view,
            notes: n,
            sky: 0,
            key: None,
            settings: self.settings,
            arrivals: self.queue.inbox.take(),
        };
        self.queue
            .step(
                &mut self.shadow,
                &self.world,
                &mut self.store,
                msg,
                &mut self.half,
            )
            .expect("the step runs");
    }

    /// An edit's box, mirrored and handed to the queue as the frame does.
    fn carve(&mut self, min: [i32; 3], max: [i32; 3]) {
        let r = self
            .world
            .apply(&box_op(min.map(i64::from), max.map(i64::from), Mode::Carve))
            .expect("resident");
        let (lo, hi) = r.aabb.expect("voxels changed");
        for x in lo.0[0].div_euclid(64)..=hi.0[0].div_euclid(64) {
            self.mirror.changed.push(ChunkCoord([x, 0, 0]));
        }
        self.queue.invalidate(lo.0, hi.0);
    }

    /// Steps until `pred` holds, at most 400 frames.
    fn until(&mut self, what: &str, pred: impl Fn(&Self) -> bool) {
        for _ in 0..400 {
            if pred(self) {
                return;
            }
            self.step();
        }
        panic!("{what} within 400 frames");
    }
}

const A: [i32; 3] = [0, 0, 0];

/// A box in chunk 1 within a dig's reach of chunk 0's surface bricks but more than a voxel from
/// them: chunk 0 is in its ring and not withdrawn.
const NEAR_A: ([i32; 3], [i32; 3]) = ([76, 20, 4], [79, 23, 7]);

/// Done-when (2) of Plan 0099 Phase 2, with its amended case. A job takes the stamp of the frame
/// message it joined the loop at; a change carried by a later message stales its result, which
/// is released and its chunk queued again; a change in an earlier message leaves it placed. The
/// render half holds the read-back over the later message (a submit planned from one message
/// and issued, or mapped, frames on).
#[test]
fn a_change_after_a_submit_stales_its_result_and_one_before_does_not() {
    // After: chunk 0's job is in flight, held, when the box's message arrives.
    let mut rig = Rig::new(None);
    rig.until("chunk 0's job issued", |r| {
        r.queue.lp.stamps.contains_key(&A) && !r.half.flights.is_empty()
    });
    rig.half.hold = true;
    let stamp = rig.queue.lp.stamps[&A];
    assert!(rig.store.block(ChunkCoord(A)).is_none());
    let (lo, hi) = NEAR_A;
    rig.carve(lo, hi);
    rig.step();
    let at = rig
        .queue
        .stale
        .get(&A)
        .copied()
        .expect("chunk 0 marked by the box");
    assert!(at > stamp, "the box's message {at} after the job's {stamp}");
    assert!(
        rig.queue.lp.stamps.get(&A) == Some(&stamp),
        "chunk 0 is not withdrawn"
    );
    rig.half.hold = false;
    rig.until("the held job read back", |r| {
        !r.queue.lp.stamps.get(&A).is_some_and(|&s| s == stamp)
    });
    assert!(
        rig.store.block(ChunkCoord(A)).is_none(),
        "the stale block is released"
    );
    assert!(!rig.queue.stale.contains_key(&A));
    rig.until("chunk 0 queued again", |r| {
        r.queue.lp.stamps.get(&A).is_some_and(|&s| s > at)
    });
    rig.until("chunk 0's new block", |r| {
        r.store.block(ChunkCoord(A)).is_some()
    });

    // Before: the same box's message comes before chunk 0's job joins the loop.
    let mut rig = Rig::new(None);
    rig.carve(lo, hi);
    rig.step();
    let mut stamp = None;
    for _ in 0..400 {
        if let Some(&s) = rig.queue.lp.stamps.get(&A) {
            stamp = Some(s);
        }
        if rig.store.block(ChunkCoord(A)).is_some() {
            break;
        }
        rig.step();
    }
    let stamp = stamp.expect("chunk 0's job joined the loop");
    // The box's message is frame 1: a job listed in that step is listed after the box is taken.
    assert!(stamp >= 1, "a job stamped at or after the box's message");
    assert!(!rig.queue.stale.contains_key(&A));
    assert!(
        rig.store.block(ChunkCoord(A)).is_some(),
        "a job stamped after the change is placed"
    );
}

/// Done-when (7) of Plan 0099 Phase 2: with landings over the frame's cap already waiting, the
/// step that takes a dig's box makes one edit list holding, for each touched chunk, its
/// sky-stale marks and its held block's words with its table word, all applied in that frame;
/// that step's submit plans ring 1's rounds ahead of every other job's; the landings applied
/// are the ones carried.
#[test]
fn a_dig_is_served_whole_in_the_edits_step() {
    let mut rig = Rig::new(Some(64 << 10));
    rig.until("every chunk holds a block", |r| {
        (0..CHUNKS[0]).all(|x| r.store.block(ChunkCoord([x, 0, 0])).is_some())
            && r.half.flights.is_empty()
    });
    // Two landings of 40 KiB wait under the 64 KiB cap.
    let landing = |step| WriteList {
        step,
        writes: vec![Write {
            offset: 1 << 30,
            data: vec![0; 40 << 10],
            kind: W_POOL,
            landing: None,
        }],
        bytes: 40 << 10,
        edit: false,
    };
    rig.half.lists.waiting.extend([landing(901), landing(902)]);
    let before: BTreeMap<[i32; 3], Vec<u32>> = rig.store.words.clone();
    // A hole in chunk 0's slab top.
    rig.carve([20, 20, 4], [23, 23, 7]);
    let applied_at = rig.half.applied.len();
    let issued_at = rig.half.issued.len();
    rig.step();
    // A step applies once: every write after `applied_at` is this frame's.
    let applied: Vec<Applied> = rig.half.applied[applied_at..].to_vec();
    // The dug chunk's held block at its place and its table word.
    let (first, words) = rig
        .store
        .block(ChunkCoord(A))
        .expect("chunk 0 keeps a block");
    let table = rig.store.table_word(ChunkCoord(A)) * 4;
    assert!(
        applied
            .iter()
            .any(|a| a.kind == W_POOL && a.offset == first * 4 && a.bytes as u64 == words * 4),
        "the held block's words"
    );
    assert!(
        applied
            .iter()
            .any(|a| a.kind == W_TABLE && a.offset == table && a.bytes == 4),
        "its table word"
    );
    // Every ring chunk the marks changed: its block written in the same frame.
    for (c, w) in &rig.store.words {
        if before.get(c).is_some_and(|b| b != w) {
            let (f, _) = rig.store.block(ChunkCoord(*c)).expect("a block");
            assert!(
                applied
                    .iter()
                    .any(|a| a.kind == W_POOL && a.offset == f * 4),
                "chunk {c:?}'s marks"
            );
        }
    }
    // The edit's writes come first; the landings applied after them are the carried ones, in
    // order, and what the cap leaves waits.
    let carried: Vec<&Applied> = applied.iter().filter(|a| a.offset == 1 << 30).collect();
    let n = carried.len();
    let waiting: Vec<u64> = rig.half.lists.waiting.iter().map(|l| l.step).collect();
    assert_eq!(
        waiting.iter().take(2 - n).copied().collect::<Vec<_>>(),
        [901, 902][n..],
        "the carried landings wait ahead of this step's"
    );
    if n < 2 {
        let edit_end = applied
            .iter()
            .position(|a| a.offset == 1 << 30)
            .unwrap_or(applied.len());
        assert!(
            applied[edit_end..].iter().all(|a| a.offset == 1 << 30),
            "no landing of this step ahead of a carried one"
        );
    }
    assert!(
        applied.iter().take_while(|a| a.offset != 1 << 30).count() >= 2,
        "the edit's writes first"
    );
    // The step's submit: ring 1's rounds first.
    let ring1: Vec<u32> = rig
        .queue
        .lp
        .open
        .iter()
        .filter(|j| rig.queue.dig_open.get(&j.coord.0).is_some_and(|d| d.near))
        .map(|j| j.slot as u32)
        .collect();
    assert!(!ring1.is_empty(), "ring 1 opened in the edit's step");
    let (_, slots) = rig
        .half
        .issued
        .get(issued_at)
        .expect("the edit's step issued a submit");
    let first_other = slots.iter().position(|s| !ring1.contains(s));
    let last_ring = slots.iter().rposition(|s| ring1.contains(s));
    assert!(last_ring.is_some(), "ring 1 in the submit");
    assert!(
        first_other.is_none_or(|o| last_ring.is_some_and(|r| r < o)),
        "ring 1's rounds ahead of every other job's: {slots:?}"
    );
}

/// The public reads between an edit and the next step answer as they did when the box went into
/// the books at the edit: a queue whose box waits in the mailbox against the same queue with the
/// box taken at once, over every chunk within two of the box. A faceless chunk the box touches
/// reads unsettled and not passed until the step, and the dig reads waiting.
#[test]
fn an_edit_reads_unsettled_before_the_next_step() {
    let ready = |r: &Rig| {
        (0..CHUNKS[0]).all(|x| r.store.block(ChunkCoord([x, 0, 0])).is_some())
            && r.half.flights.is_empty()
            && r.queue.digs_settled()
    };
    let (mut waits, mut eager) = (Rig::new(None), Rig::new(None));
    waits.until("every chunk holds a block", ready);
    eager.until("every chunk holds a block", ready);
    // A chunk beside the slab, known faceless and blocked: the box reaches it within a voxel.
    let beside = [-1, 0, 0];
    for r in [&mut waits, &mut eager] {
        r.queue.faceless.insert(beside);
        r.queue.blocked.insert(beside, Default::default());
    }
    assert!(waits.queue.settled(&waits.store, ChunkCoord(beside), false));
    let (lo, hi) = ([0, 20, 4], [3, 23, 7]);
    waits.carve(lo, hi);
    eager.carve(lo, hi);
    let arrivals = eager.queue.inbox.take();
    eager
        .queue
        .take_message(eager.settings, arrivals, eager.world.materials());
    let (w, e) = (&waits.queue, &eager.queue);
    assert_eq!(w.digs_settled(), e.digs_settled());
    assert_eq!(w.boxes_waiting(), e.boxes_waiting());
    assert_eq!(w.empty(), e.empty());
    assert!(!w.digs_settled() && w.boxes_waiting(), "the dig waits");
    for z in -2..=2 {
        for y in -2..=3 {
            for x in -2..=4 {
                let c = ChunkCoord([x, y, z]);
                for absent in [false, true] {
                    assert_eq!(
                        w.settled(&waits.store, c, absent),
                        e.settled(&eager.store, c, absent),
                        "settled {c:?} {absent}"
                    );
                }
                assert_eq!(
                    w.passed(&waits.store, c),
                    e.passed(&eager.store, c),
                    "passed {c:?}"
                );
                assert_eq!(w.tracks(c), e.tracks(c), "tracks {c:?}");
            }
        }
    }
    assert!(
        !w.settled(&waits.store, ChunkCoord(beside), false),
        "the box's faceless neighbour reads unsettled"
    );
    assert!(!w.passed(&waits.store, ChunkCoord(beside)));
    // The step takes the box: the books then stand as the eager queue's.
    waits.step();
    assert!(waits.queue.inbox.take().is_empty());
    assert!(!waits.queue.faceless.contains(&beside) && !waits.queue.blocked.contains_key(&beside));
    eager.step();
    assert!(
        waits.queue.digs.keys().eq(eager.queue.digs.keys()),
        "the same dig bricks"
    );
}
