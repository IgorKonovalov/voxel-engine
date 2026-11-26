//! The blocked marks stepped by count: a candidate refused at a check is checked again at the
//! first opening after what refused it changes in the mirror's notes, and at the latest
//! [`QUEUE_RECHECK`] steps after its refusal. The geometry is
//! `a_chunk_absent_high_in_a_column_holds_the_bake`'s: a slab of chunks along x at z 0, a light
//! radius small enough that a chunk four up stands in a column over a candidate's reach
//! footprint and outside its reach, a window of z -2..6. No device: the render half takes the
//! submits and reads none back, so every job opened stays open.

use super::reach::{Blocked, Blocker};
use super::shadow::{ChunkNote, LightNote, MirrorRead, notes};
use super::*;
use svet_world::{AIR, EditOp, FIXED_ONE, Fixed3, Material, MaterialTable, Mode, UniformSource};

const SLAB_TOP: i32 = 8;

/// The mirror's light radius: a reach of the chunks within one chunk (Chebyshev), a footprint of
/// the columns within one.
const LR: i32 = 1;

/// A chunk waiting to upload: four up, so in a footprint column and outside every reach.
const PENDING: ChunkNote = ChunkNote {
    mirrored: false,
    lod: false,
    pending: true,
};

/// What an upload of a chunk outside the world leaves: held LOD-only, so no candidate.
const UPLOADED: ChunkNote = ChunkNote {
    mirrored: true,
    lod: true,
    pending: false,
};

/// A slab of `n` chunks along x, rock over z 0 ..= 7: every chunk a candidate with faces.
fn slab(n: i32) -> World {
    let mut t = MaterialTable::new();
    let rock = t.push(Material::named("rock"));
    let w = World::generate(
        ChunkExtent::new([0, 0, 0], [n, 1, 1]),
        &UniformSource(AIR),
        &t,
    );
    for x in 0..i64::from(n) {
        let (min, max) = ([64 * x, 0, 0], [64 * x + 63, 63, i64::from(SLAB_TOP) - 1]);
        let op = EditOp::box_(
            Fixed3(min.map(|v| v * FIXED_ONE)),
            Fixed3(max.map(|v| v * FIXED_ONE)),
            Mode::Fill(rock),
        );
        w.apply(&op).expect("resident");
    }
    w
}

/// The mirror as the render half reads it: the slab's chunks held full, the chunks `notes`
/// names in their state, and `lights` with its key.
struct Mirror {
    n: i32,
    notes: std::collections::HashMap<[i32; 3], ChunkNote>,
    changed: Vec<ChunkCoord>,
    lights: Vec<([i32; 3], LightNote)>,
    key: (u64, u64),
}

impl MirrorRead for Mirror {
    fn window(&self) -> ChunkExtent {
        ChunkExtent::new([-1, -1, -2], [self.n + 3, 4, 8])
    }
    fn light_radius(&self) -> i32 {
        LR
    }
    fn chunk_note(&self, c: ChunkCoord) -> ChunkNote {
        let slab = (0..self.n).contains(&c.0[0]) && c.0[1] == 0 && c.0[2] == 0;
        self.notes.get(&c.0).copied().unwrap_or(ChunkNote {
            mirrored: slab,
            lod: false,
            pending: false,
        })
    }
    fn chunks(&self) -> Vec<ChunkCoord> {
        let mut out: Vec<ChunkCoord> = (0..self.n).map(|x| ChunkCoord([x, 0, 0])).collect();
        out.extend(
            self.notes
                .iter()
                .filter(|(_, n)| **n != ChunkNote::default())
                .map(|(c, _)| ChunkCoord(*c)),
        );
        out
    }
    fn column_lowest(&self, _: i32, _: i32) -> i32 {
        SLAB_TOP
    }
    fn light_key(&self) -> (u64, u64) {
        self.key
    }
    fn light_notes(&self) -> Vec<([i32; 3], LightNote)> {
        self.lights.clone()
    }
}

/// A render half that issues and reads nothing back.
struct Half;

impl RenderHalf for Half {
    fn poll(&mut self) {}
    fn tables(&mut self, _: &[(usize, ChunkCoord)]) {}
    fn refresh_tables(&mut self, _: &[(usize, ChunkCoord)]) {}
    fn write_params(&mut self) {}
    fn issue(&mut self, _: Prepared) {}
    fn collect(&mut self, _: usize) -> Option<ReadBack> {
        None
    }
    fn cap(&self) -> Option<usize> {
        None
    }
    fn apply(&mut self, _: Vec<WriteList>, _: u64) -> Vec<([i32; 3], u32)> {
        Vec::new()
    }
}

struct Rig {
    world: World,
    mirror: Mirror,
    shadow: Shadow,
    seen: (Option<ChunkExtent>, Option<(u64, u64)>),
    queue: BakeQueue,
    store: Store,
    settings: StepSettings,
    inbox: Inbox,
    frame: u64,
}

impl Rig {
    /// The slab of `n` chunks, each chunk of `pending` waiting to upload.
    fn new(n: i32, pending: &[[i32; 3]]) -> Self {
        let settings = BakeSettings {
            round: 64,
            brick_samples: 64,
            ..BakeSettings::default()
        };
        let mirror = Mirror {
            n,
            notes: pending.iter().map(|&c| (c, PENDING)).collect(),
            changed: Vec::new(),
            lights: Vec::new(),
            key: (0, 0),
        };
        let mut store = Store::new(64 << 20);
        store.start_queue(mirror.window(), settings.cell);
        let _ = store.take_direct();
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
            world: slab(n),
            mirror,
            shadow: Shadow::default(),
            seen: (None, None),
            queue,
            store,
            settings: step_settings,
            inbox: Inbox::default(),
            frame: 0,
        }
    }

    /// `c` changes to `note` before the next step's message.
    fn set(&mut self, c: [i32; 3], note: ChunkNote) {
        self.mirror.notes.insert(c, note);
        self.mirror.changed.push(ChunkCoord(c));
    }

    /// One step. One more chunk of row y 2, outside every candidate's footprint, arrives
    /// LOD-only each step: the mirror's count of chunks changes every step, as it does on a
    /// streamed world, so every step opens (a step with no change after an opening that found
    /// nothing ready sorts nothing for [`QUEUE_QUIET`] steps), and no change names a blocker.
    fn step(&mut self) -> QueueStats {
        let (k, w) = (self.frame as i32, self.mirror.n + 3);
        let noise = [-1 + k % w, 2, -2 + k / w];
        assert!(noise[2] < 6, "the row's chunks of the window are spent");
        self.set(noise, UPLOADED);
        let changed = std::mem::take(&mut self.mirror.changed);
        let (window, key) = &mut self.seen;
        let first = window.is_none();
        let n = notes(&self.mirror, (!first).then_some(changed), *window, key);
        *window = n.window;
        self.frame += 1;
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
            frame: self.frame,
            start: StepStart::now(),
            view,
            notes: n,
            sky: 0,
            key: None,
            settings: self.settings,
            arrivals: self.inbox.take(self.frame),
            landed: Vec::new(),
        };
        let stats = self
            .queue
            .step(
                &mut self.shadow,
                &self.world,
                &mut self.store,
                msg,
                &mut Half,
            )
            .expect("the step runs");
        self.inbox.answered(self.frame);
        stats
    }

    fn job(&self, c: [i32; 3]) -> bool {
        self.queue.lp.chunks().any(|j| j.0 == c)
    }
}

const C: [i32; 3] = [0, 0, 0];

/// (a) Candidate (0, 0, 0)'s reach is ready (nothing of it waits to upload) and its only
/// blocker is chunk (1, 0, 4), waiting to upload in a column over its reach footprint and
/// outside its reach (ADR-0070's amendment refuses it). Refused at step 1 and marked with that
/// chunk; the chunk uploads before step `k` (3, then 17); the candidate's job opens at the
/// opening of step `k`, not at step 1 + [`QUEUE_RECHECK`].
#[test]
fn a_candidate_blocked_by_a_column_outside_its_reach_opens_when_that_chunk_uploads() {
    let p = [1, 0, 4];
    for k in [3, 17] {
        let mut rig = Rig::new(1, &[p]);
        let s = rig.step();
        assert_eq!(
            (s.openings, s.waiting_reach),
            (1, 1),
            "k {k}: refused at step 1"
        );
        assert_eq!(
            rig.queue.blocked.get(&C),
            Some(&Blocked {
                at: 1,
                by: Blocker::Chunk(p)
            }),
            "k {k}: marked with the pending chunk"
        );
        for step in 2..k {
            rig.step();
            assert!(!rig.job(C), "k {k}: no job at step {step}");
        }
        rig.set(p, UPLOADED);
        rig.step();
        assert_eq!(rig.queue.steps, k);
        assert!(rig.job(C), "k {k}: the job opens at step {k}");
    }
}

/// (b) Two blockers: chunk (0, 0, 1), within the candidate's reach, waits to upload, and a
/// light range near it, chunk (0, 0, 1)'s, is not current. The chunk uploads before step 3:
/// the candidate is checked at step 3 and refused by the lights, its mark now the lights'; no
/// job. The range turns current before step 9 (the light key moves): the job opens at step 9.
#[test]
fn a_candidate_with_two_blockers_is_checked_as_each_clears() {
    let r = [0, 0, 1];
    let mut rig = Rig::new(1, &[r]);
    rig.mirror.lights = vec![(
        r,
        LightNote {
            emitter: false,
            current: false,
        },
    )];
    let s = rig.step();
    assert_eq!(s.waiting_reach, 1, "refused by the reach at step 1");
    assert_eq!(
        rig.queue.blocked.get(&C).map(|b| b.by),
        Some(Blocker::Chunk(r))
    );
    rig.step();
    rig.set(r, UPLOADED);
    let s = rig.step();
    assert_eq!(
        (s.waiting_reach, s.waiting_lights),
        (0, 1),
        "checked at step 3 and refused by the lights"
    );
    assert_eq!(
        rig.queue.blocked.get(&C),
        Some(&Blocked {
            at: 3,
            by: Blocker::Lights
        })
    );
    assert!(!rig.job(C));
    for step in 4..9 {
        let s = rig.step();
        assert_eq!(s.waiting_lights, 0, "not checked at step {step}");
        assert!(!rig.job(C));
    }
    rig.mirror.lights[0].1.current = true;
    rig.mirror.key.0 += 1;
    rig.step();
    assert_eq!(rig.queue.steps, 9);
    assert!(rig.job(C), "the job opens at step 9");
}

/// (c) No event: the candidate's blocker, chunk (1, 0, 4) waiting to upload, stays pending,
/// and every step changes the mirror elsewhere. Steps 2 to 30 each open, and none checks it:
/// no reach walk ([`QueueStats::reach_checks`]) and no refusal counted. Step 31, the mark's
/// age at [`QUEUE_RECHECK`], checks it and refuses it again.
#[test]
fn a_candidate_whose_blocker_does_not_change_is_checked_at_the_age() {
    let p = [1, 0, 4];
    let mut rig = Rig::new(1, &[p]);
    let s = rig.step();
    assert_eq!((s.reach_checks, s.waiting_reach), (1, 1));
    for step in 2..=30 {
        let s = rig.step();
        assert_eq!(s.openings, step, "step {step} opens");
        assert_eq!(
            (s.reach_checks, s.waiting_reach, s.waiting_lights),
            (0, 0, 0),
            "not checked at step {step}"
        );
    }
    let s = rig.step();
    assert_eq!(s.openings, 31);
    assert_eq!(s.waiting_reach, 1, "checked and refused at step 31");
    assert_eq!(rig.queue.blocked.get(&C).map(|b| b.at), Some(31));
    assert!(!rig.job(C));
}

/// (d) The check budget. Chunks (x, 0, 4) for x 0 to 30 wait to upload: candidates 0 to 31 are
/// each held by a column over their footprint, candidate 32 is ready and last in the order.
/// Step 1's opening spends its [`QUEUE_CHECKS`] on candidates 0 to 15; before step 2 chunk
/// (0, 0, 4), the blocker of candidates 0 and 1, uploads. Candidate 32's job opens at step 3,
/// the step read on the tree before the blocked marks carried their blocker (61ddaaa1): the two
/// candidates the event frees are checked once each within the opening's checks, and the walk
/// behind them is not set back.
#[test]
fn an_event_that_clears_a_blocker_does_not_delay_the_ready_candidate_behind() {
    let pending: Vec<[i32; 3]> = (0..=30).map(|x| [x, 0, 4]).collect();
    let mut rig = Rig::new(33, &pending);
    rig.step();
    assert_eq!(
        rig.queue.blocked.len(),
        QUEUE_CHECKS,
        "step 1 spends its checks"
    );
    assert!(
        rig.queue.blocked.contains_key(&[0, 0, 0]) && rig.queue.blocked.contains_key(&[1, 0, 0])
    );
    rig.set([0, 0, 4], UPLOADED);
    let ready = [32, 0, 0];
    let mut opened = None;
    for _ in 2..=40 {
        rig.step();
        if rig.job(ready) {
            opened = Some(rig.queue.steps);
            break;
        }
    }
    assert_eq!(opened, Some(3), "candidate 32's job opens at step 3");
}
