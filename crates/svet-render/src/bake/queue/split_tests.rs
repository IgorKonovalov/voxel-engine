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

/// The slab world `n` chunks along x.
fn world_of(n: i32) -> World {
    let mut t = MaterialTable::new();
    let rock = t.push(Material::named("rock"));
    let w = World::generate(
        ChunkExtent::new([0, 0, 0], [n, 1, 1]),
        &UniformSource(AIR),
        &t,
    );
    // One box a chunk: an edit's extent is bounded.
    for x in 0..i64::from(n) {
        let max = [64 * x + 63, 63, i64::from(SLAB_TOP) - 1];
        w.apply(&box_op([64 * x, 0, 0], max, Mode::Fill(rock)))
            .expect("resident");
    }
    w
}

/// The mirror as the render half would read it: every chunk of the world held full, its uploads
/// done, no light.
struct Mirror {
    changed: Vec<ChunkCoord>,
    /// The window's chunks along x.
    size: i32,
    /// The chunks held, `0 .. held` along x; the rest not yet streamed in.
    held: i32,
    /// Every column's lowest top: the slab's, or one over the chunks' reach (buried).
    lowest: i32,
    /// The light radius: 2, or 0 where the rule's footprint (the columns within it) must lie in
    /// the one-chunk-wide window.
    lr: i32,
}

impl MirrorRead for Mirror {
    fn window(&self) -> ChunkExtent {
        ChunkExtent::new([0, 0, 0], [self.size, 1, 1])
    }
    fn light_radius(&self) -> i32 {
        self.lr
    }
    fn chunk_note(&self, c: ChunkCoord) -> ChunkNote {
        ChunkNote {
            mirrored: c.0[0] < self.held,
            lod: false,
            pending: false,
        }
    }
    fn chunks(&self) -> Vec<ChunkCoord> {
        let mut out = Vec::new();
        for x in 0..self.held {
            out.push(ChunkCoord([x, 0, 0]));
        }
        out
    }
    fn column_lowest(&self, _: i32, _: i32) -> i32 {
        self.lowest
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
    /// The pool's bytes as the writes leave them, in the order applied, where a test keeps one.
    pool: Option<&'a mut Vec<u8>>,
}

impl Applier for Recorder<'_> {
    fn write(&mut self, offset: u64, data: &[u8], kind: usize) {
        self.out.push(Applied {
            offset,
            kind,
            bytes: data.len(),
        });
        if let Some(pool) = self.pool.as_mut() {
            let at = offset as usize;
            if pool.len() < at + data.len() {
                pool.resize(at + data.len(), 0);
            }
            pool[at..at + data.len()].copy_from_slice(data);
        }
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
    /// The frame each list taken was named by ([`WriteList::frame`]), in order.
    named: Vec<u64>,
    applied: Vec<Applied>,
    /// Each read-back's GPU milliseconds, 0.1 where unset.
    ms: Option<f64>,
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
            gpu_ms: Some(self.ms.unwrap_or(0.1)),
            timer_ms: None,
        })
    }
    fn cap(&self) -> Option<usize> {
        self.lists.cap
    }
    fn apply(&mut self, lists: Vec<WriteList>, frame: u64) -> Vec<([i32; 3], u32)> {
        self.named.extend(lists.iter().map(|l| l.frame));
        self.lists
            .take(lists)
            .expect("no landing list over the cap");
        let mut out = Recorder {
            out: &mut self.applied,
            pool: None,
        };
        let landed = self.lists.apply(frame, &mut out).1;
        self.lists.confirm_all();
        landed
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
    /// The render half's mailbox.
    inbox: Inbox,
    /// The sample cap each message's view sets (`QueueView::samples`); `None` prices it.
    samples: Option<u64>,
}

impl Rig {
    fn new(cap: Option<usize>) -> Self {
        Self::sized(cap, CHUNKS[0], CHUNKS[0])
    }

    /// The rig over [`world_of`]`(size)`, its first `held` chunks in the mirror.
    fn sized(cap: Option<usize>, size: i32, held: i32) -> Self {
        let settings = BakeSettings {
            round: 64,
            brick_samples: 64,
            ..BakeSettings::default()
        };
        let mut store = Store::new(64 << 20);
        let mirror = Mirror {
            changed: Vec::new(),
            size,
            held,
            lowest: SLAB_TOP,
            lr: 2,
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
            world: world_of(size),
            mirror,
            shadow: Shadow::default(),
            seen: (None, None),
            queue,
            store,
            half,
            settings: step_settings,
            inbox: Inbox::default(),
            samples: None,
        }
    }

    /// One frame: the message (the mirror's changes since the last), then the bake half's step.
    fn step(&mut self) {
        let msg = self.message();
        self.run(msg);
    }

    /// The next frame's message (the mirror's changes since the last), sent: its arrivals are
    /// counted by the mailbox until a list made from it is applied.
    fn message(&mut self) -> FrameMessage {
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
            samples: self.samples,
            interval_s: 1.0 / 60.0,
            hold: None,
            half_angle: 1.0,
            frustum: None,
            full_radius: None,
            lod_radius: None,
            focal_px: None,
        };
        let frame = self.half.frame;
        FrameMessage {
            frame,
            start: StepStart::now(),
            view,
            notes: n,
            sky: 0,
            key: None,
            settings: self.settings,
            arrivals: self.inbox.take(frame),
            landed: Vec::new(),
        }
    }

    /// The bake half's step on `msg` (the newest, or several coalesced); its lists apply in the
    /// same call, which answers every message up to it.
    fn run(&mut self, msg: FrameMessage) {
        let frame = msg.frame;
        self.queue
            .step(
                &mut self.shadow,
                &self.world,
                &mut self.store,
                msg,
                &mut self.half,
            )
            .expect("the step runs");
        self.inbox.answered(frame);
    }

    /// The public reads as the frame makes them.
    fn read(&self) -> QueueRead<'_> {
        QueueRead {
            src: ReadSource::Live {
                queue: &self.queue,
                store: &self.store,
            },
            inbox: &self.inbox,
        }
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
        self.inbox.push_box(lo.0, hi.0);
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

/// Done-when (7) of Plan 0099 Phase 2, under Phase 3c's order: with landings over the frame's
/// cap already waiting, the step that takes a dig's box makes one edit list holding, for each
/// touched chunk, its sky-stale marks and its held block's words with its table word, all
/// applied in that frame behind the landings made before it; that step's submit plans ring 1's
/// rounds ahead of every other job's.
#[test]
fn a_dig_is_served_whole_in_the_edits_step() {
    let mut rig = Rig::new(Some(64 << 10));
    rig.until("every chunk holds a block", |r| {
        (0..CHUNKS[0]).all(|x| r.store.block(ChunkCoord([x, 0, 0])).is_some())
            && r.half.flights.is_empty()
    });
    // Two landings of 40 KiB wait under the 64 KiB cap.
    let landing = |frame| WriteList {
        frame,
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
    // Lists apply in the order made (ADR-0083, Plan 0099 Phase 3c): the carried landings were
    // made before the edit, so both apply in its frame, ahead of its writes, whatever the cap.
    let carried: Vec<usize> = applied
        .iter()
        .enumerate()
        .filter(|(_, a)| a.offset == 1 << 30)
        .map(|(k, _)| k)
        .collect();
    assert_eq!(carried, [0, 1], "the carried landings first, both applied");
    assert!(
        rig.half
            .lists
            .waiting
            .iter()
            .all(|l| l.frame != 901 && l.frame != 902),
        "no carried landing waits behind the edit"
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
            && r.read().digs_settled()
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
    assert!(waits.read().settled(ChunkCoord(beside), false));
    let (lo, hi) = ([0, 20, 4], [3, 23, 7]);
    waits.carve(lo, hi);
    eager.carve(lo, hi);
    let arrivals = eager.inbox.take(1_000);
    eager.inbox.answered(1_000);
    eager
        .queue
        .take_message(1_000, eager.settings, arrivals, eager.world.materials());
    let (w, e) = (waits.read(), eager.read());
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
                        w.settled(c, absent),
                        e.settled(c, absent),
                        "settled {c:?} {absent}"
                    );
                }
                assert_eq!(w.passed(c), e.passed(c), "passed {c:?}");
                assert_eq!(w.tracks(c), e.tracks(c), "tracks {c:?}");
            }
        }
    }
    assert!(
        !w.settled(ChunkCoord(beside), false),
        "the box's faceless neighbour reads unsettled"
    );
    assert!(!w.passed(ChunkCoord(beside)));
    // The step takes the box: the books then stand as the eager queue's.
    waits.step();
    assert!(waits.inbox.take(u64::MAX).is_empty() && !waits.inbox.has_box());
    assert!(!waits.queue.faceless.contains(&beside) && !waits.queue.blocked.contains_key(&beside));
    eager.step();
    assert!(
        waits.queue.digs.keys().eq(eager.queue.digs.keys()),
        "the same dig bricks"
    );
}

/// Done when (1) of Plan 0099 Phase 2c. One open job that is not a dig's with 4 000 000 samples
/// left, 15 table slots free, a ring-2 dig job of 100 000 samples made in this step, a cap of
/// 250 000: the dig job opens into a free slot in the step and the submit plans its 100 000
/// first, then no more than 150 000 and one round of the other's. With no slot free the dig job
/// stays first of the pending ones and no job is withdrawn.
#[test]
fn a_dig_job_is_planned_ahead_of_an_open_job_that_is_not_a_digs() {
    use crate::bake::job_loop::JobCtx;
    const ROUND: u32 = 1000;
    const CAP: u64 = 250_000;
    const DIG: [i32; 3] = [0, 0, 0];
    const OTHER: [i32; 3] = [1, 0, 0];
    // `bricks` bricks of five directions, one face each: each face draws `brick_samples`.
    let job = |c: [i32; 3], bricks: u16| {
        let faces = ChunkFaces {
            bricks: (0..bricks).map(|b| (b, (0..5u16).collect())).collect(),
            glass: Vec::new(),
        };
        let samples = faces.faces() as u64 * 2000;
        Job {
            coord: ChunkCoord(c),
            faces,
            first_word: 0,
            kept_bricks: KeptBricks::default(),
            samples,
            brick_samples: None,
            key_only: false,
            tier: 0,
            class: JobClass::Other,
        }
    };
    let samples_of = |j: &OpenJob| j.states.iter().map(|s| u64::from(s.queued)).sum::<u64>();
    for free in [15usize, 0] {
        let mut rig = Rig::new(None);
        rig.queue.settings.round = ROUND;
        rig.queue.settings.brick_samples = 2000;
        let settings = rig.queue.settings.clone();
        let run = BakeRun {
            mirror: &rig.shadow,
            world: &rig.world,
            sky: 0,
            key: None,
        };
        let ctx = JobCtx {
            run: &run,
            settings: &settings,
            round: ROUND,
            rounds_cap: 0,
        };
        let other = job(OTHER, 400);
        assert_eq!(other.samples, 4_000_000);
        rig.queue.lp.open = vec![ctx.open(other, 0)];
        rig.queue.lp.free = (1..=free).rev().collect();
        rig.queue.lp.pending.clear();
        let dig = job(DIG, 10);
        assert_eq!(dig.samples, 100_000);
        rig.queue.dig_open.insert(
            DIG,
            DigChunk {
                slots: (0..10).collect(),
                ..DigChunk::default()
            },
        );
        rig.queue.open_pushed(&run, &mut rig.store, vec![dig], 0);
        assert_eq!(
            rig.queue.stats.dig_withdrawn, 0,
            "free {free}: no job withdrawn"
        );
        assert!(
            rig.queue.lp.open.iter().any(|j| j.coord.0 == OTHER),
            "free {free}: the other job stays open"
        );
        if free == 0 {
            assert_eq!(
                rig.queue.lp.pending.front().map(|j| j.coord.0),
                Some(DIG),
                "no slot free: the dig job is first of the pending ones"
            );
            assert_eq!(rig.queue.lp.open.len(), 1);
            continue;
        }
        assert!(
            rig.queue.lp.pending.is_empty(),
            "the dig job opened in its step"
        );
        assert_eq!(rig.queue.lp.free.len(), free - 1);
        let slot = |c: [i32; 3]| {
            rig.queue
                .lp
                .open
                .iter()
                .find(|j| j.coord.0 == c)
                .map(|j| j.slot as u32)
        };
        let (dig_slot, other_slot) = (slot(DIG).expect("dig open"), slot(OTHER).expect("open"));
        rig.queue.lp.dispatch(
            &run,
            &mut rig.half,
            &mut rig.store,
            &settings,
            CAP,
            &mut |_: &mut Store, _, _, _, _| {},
            &mut |_| {},
        );
        let drawn = |c: [i32; 3]| {
            rig.queue
                .lp
                .open
                .iter()
                .find(|j| j.coord.0 == c)
                .map_or(0, samples_of)
        };
        assert_eq!(drawn(DIG), 100_000, "the dig job's samples whole");
        let other_drawn = drawn(OTHER);
        assert!(
            other_drawn <= CAP - 100_000 + u64::from(ROUND),
            "the other job drew {other_drawn}"
        );
        let (_, slots) = rig.half.issued.last().expect("a submit");
        let last_dig = slots.iter().rposition(|s| *s == dig_slot);
        let first_other = slots.iter().position(|s| *s == other_slot);
        assert!(last_dig.is_some(), "the dig job in the submit");
        assert!(
            first_other.is_none_or(|o| last_dig.is_some_and(|d| d < o)),
            "the dig job's rounds ahead of the other's"
        );
    }
}

/// Done when (1) of Plan 0099 Phase 2d. A submit of one job's 200 000 samples and another's
/// 20 000, the first withdrawn before the read-back, read back in 4.0 ms: `samples` counts the
/// 20 000 kept and the drawn count is 220 000, which the estimate reads, so from 20 ns its mean
/// is `0.8 x 20 + 0.2 x 4.0 ms / 220 000` = 19.64 ns, where the kept samples would give 56.0.
/// With both withdrawn nothing is kept and the read-back is read all the same, to 19.64 again.
#[test]
fn a_withdrawn_round_counts_in_what_the_estimate_reads() {
    use crate::bake::job_loop::JobCtx;
    const GONE_AT: [i32; 3] = [0, 0, 0];
    const KEPT_AT: [i32; 3] = [1, 0, 0];
    // `bricks` bricks of five directions, one face each and one round a face.
    let job = |c: [i32; 3], bricks: u16| {
        let faces = ChunkFaces {
            bricks: (0..bricks).map(|b| (b, (0..5u16).collect())).collect(),
            glass: Vec::new(),
        };
        Job {
            coord: ChunkCoord(c),
            samples: faces.faces() as u64 * 2000,
            faces,
            first_word: 0,
            kept_bricks: KeptBricks::default(),
            brick_samples: None,
            key_only: false,
            tier: 0,
            class: JobClass::Other,
        }
    };
    let kept_mean = {
        let mut c = SampleCost::new(20.0);
        c.read(1, 20_000, 4.0);
        c.mean_ns
    };
    assert!((kept_mean - 56.0).abs() < 1e-9, "{kept_mean}");
    for both in [false, true] {
        let mut rig = Rig::new(None);
        rig.queue.settings.round = 2000;
        rig.queue.settings.brick_samples = 2000;
        rig.half.ms = Some(4.0);
        let settings = rig.queue.settings.clone();
        let run = BakeRun {
            mirror: &rig.shadow,
            world: &rig.world,
            sky: 0,
            key: None,
        };
        let ctx = JobCtx {
            run: &run,
            settings: &settings,
            round: 2000,
            rounds_cap: 0,
        };
        let (big, small) = (job(GONE_AT, 20), job(KEPT_AT, 2));
        assert_eq!((big.samples, small.samples), (200_000, 20_000));
        rig.queue.lp.open = vec![ctx.open(big, 0), ctx.open(small, 1)];
        rig.queue.lp.free.clear();
        rig.queue.lp.pending.clear();
        let mut sink = |_: &mut Store, _, _, _, _| {};
        assert!(rig.queue.lp.dispatch(
            &run,
            &mut rig.half,
            &mut rig.store,
            &settings,
            1_000_000,
            &mut sink,
            &mut |_| {},
        ));
        assert_eq!(rig.queue.lp.last_samples, 220_000, "both jobs whole");
        let record = record_words(settings.cell);
        let gone = rig
            .queue
            .lp
            .withdraw(|c| c.0 == GONE_AT || (both && c.0 == KEPT_AT), record);
        assert_eq!(gone.len(), 1 + usize::from(both));
        let d = rig.queue.lp.collect(
            &run,
            &mut rig.half,
            &mut rig.store,
            &settings,
            &mut sink,
            &mut |_| {},
            0,
        );
        assert_eq!(d.gpu_ms, Some(4.0));
        let kept = if both { 0 } else { 20_000 };
        assert_eq!(d.samples, kept, "both {both}: the samples kept");
        assert_eq!(d.drawn, 220_000, "both {both}: the samples drawn");
        rig.queue.read_stats(&d, 1_000_000, 220_000, false);
        assert_eq!(rig.queue.stats.samples, kept);
        assert_eq!(rig.queue.stats.drawn, 220_000);
        let mean = rig.queue.cost.mean_ns;
        assert!(
            (mean - 19.636_363).abs() < 1e-3,
            "both {both}: the estimate's mean {mean}"
        );
    }
}

/// The column top the audit's rig sets, at light radius 0 (each chunk's footprint its own
/// column, inside the rig's window): over every chunk's reach box (z 0's tops at 63), so the
/// rule holds for every chunk.
const BURIED: i32 = 4096;

/// Plan 0101 Phase 1, done-when (1). Under the audit the rule writes no record: each of the three
/// buried chunks takes a job, and its block, read back with no light (the recording half echoes
/// each entry's zero sums), equals the zero record for its faces: compared 3 under `full`, none
/// differs. The same block with one sky word changed counts one that differs, kept with its
/// chunk, its case and that word's index. A job whose result is placed stale (an edit's message
/// after its stamp) counts in neither: across every step, the blocks compared are the listing
/// jobs' blocks placed ([`QueueStats::baked`] less the digs').
#[test]
fn the_audit_counts_a_block_that_differs_from_the_zero_record() {
    let audited = |rig: &Rig| {
        let a = &rig.queue.stats.rule_audit;
        (a.compared.iter().sum::<u64>(), a.differ.iter().sum::<u64>())
    };
    let mut rig = Rig::new(None);
    rig.mirror.lowest = BURIED;
    rig.mirror.lr = 0;
    rig.queue.rule_audit = true;
    rig.queue.rule_reading = true;
    rig.until("every chunk holds a block", |r| {
        (0..CHUNKS[0]).all(|x| r.store.block(ChunkCoord([x, 0, 0])).is_some())
            && r.half.flights.is_empty()
    });
    let a = rig.queue.stats.rule_audit;
    assert_eq!(a.compared, [3, 0, 0, 0], "{a:?}");
    assert_eq!(a.differ, [0; 4], "{:?}", a.kept());
    assert_eq!(rig.queue.stats.zeroed, 0, "the rule wrote no record");
    assert!(rig.queue.zeroed.is_empty());

    // The same block with one sky word changed: chunk A's zero record (equal to its job's block
    // above), its first cell record's first visibility word set.
    let classes = rig.queue.classes(rig.world.materials());
    let faces = rig.queue.list(&rig.world, ChunkCoord(A), &classes).0;
    let s = &rig.queue.settings;
    let rule = super::rule::zero_block(ChunkCoord(A), &faces, s.cell, s.clamp, false, None, 0);
    let run = (rule[..CHUNK_BRICKS]
        .iter()
        .find(|&&d| d != 0)
        .expect("a surface brick")
        >> 11) as usize;
    let record = record_words(s.cell);
    let word = run - record + VIS_WORD;
    let mut job = rule.clone();
    job[word] ^= 0x00FF;
    let mut audit = RuleAudit::default();
    audit.compare(A, AUDIT_PARTIAL, &rule, &rule, record, false);
    audit.compare(A, AUDIT_PARTIAL, &job, &rule, record, false);
    assert_eq!(audit.compared, [0, 0, 2, 0]);
    assert_eq!(audit.differ, [0, 0, 1, 0]);
    assert_eq!(
        audit.kept(),
        [AuditDiffer {
            chunk: A,
            case: AUDIT_PARTIAL as u8,
            word: word as u32,
            job: job[word],
            rule: rule[word],
        }]
    );

    // Stale: chunk A's job in flight, held, when an edit's message arrives.
    let mut rig = Rig::new(None);
    rig.mirror.lowest = BURIED;
    rig.mirror.lr = 0;
    rig.queue.rule_audit = true;
    rig.until("chunk 0's job issued", |r| {
        r.queue.lp.stamps.contains_key(&A) && !r.half.flights.is_empty()
    });
    assert!(rig.queue.audit_jobs.contains_key(&A), "an audited job");
    rig.half.hold = true;
    let stamp = rig.queue.lp.stamps[&A];
    let (lo, hi) = NEAR_A;
    rig.carve(lo, hi);
    rig.step();
    assert!(
        rig.queue.stale.contains_key(&A),
        "chunk 0 marked by the box"
    );
    rig.half.hold = false;
    let placed = |r: &Rig| r.queue.stats.baked - r.queue.stats.digs_baked;
    let mut released = false;
    for _ in 0..400 {
        if !rig.queue.lp.stamps.get(&A).is_some_and(|&s| s == stamp) {
            break;
        }
        let (before, blocks) = (audited(&rig), placed(&rig));
        rig.step();
        let (after, now) = (audited(&rig), placed(&rig));
        assert_eq!(
            after.0 - before.0,
            now - blocks,
            "compared: the blocks placed, no other"
        );
        released |= rig.store.block(ChunkCoord(A)).is_none()
            && !rig.queue.lp.stamps.get(&A).is_some_and(|&s| s == stamp);
    }
    assert!(released, "the stale block is released");
    assert!(
        !rig.queue.audit_jobs.contains_key(&A),
        "its audit entry with it"
    );
    assert_eq!(
        audited(&rig).0,
        placed(&rig),
        "a stale block counts in neither"
    );
    assert_eq!(audited(&rig).1, 0);
}

mod lister;
mod routing;
mod stretch;
mod thread;
