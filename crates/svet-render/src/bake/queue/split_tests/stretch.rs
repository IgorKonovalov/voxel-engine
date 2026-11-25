//! The ring-1 stretch ([`BakeQueue::ring1_stretch`], Plan 0099 Phase 2e): a submit whose ring-1
//! dig jobs have more samples left than the cap, and at most the stretch times it, plans them
//! whole and nothing else.

use super::step::ring1_cap;
use super::*;
use crate::bake::job_loop::JobCtx;

const ROUND: u32 = 1000;
const CAP: u64 = 200_000;
const OTHER: [i32; 3] = [2, 0, 0];

/// A job of `faces` faces (four directions a brick), each drawing 2 000 samples.
fn job(c: [i32; 3], faces: u16) -> Job {
    let faces = ChunkFaces {
        bricks: (0..faces / 4).map(|b| (b, (0..4u16).collect())).collect(),
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
}

/// Ring-1 jobs of `ring1` samples each (chunks `[0, 0, 0]`, `[1, 0, 0]`, ...) open ahead of an
/// open job that is not a dig's with 4 000 000 left, planned by one dispatch at `stretch` over
/// [`CAP`]: the cap sent, whether it stretched, each ring-1 job's planned samples and the other
/// job's.
fn plan(ring1: &[u64], stretch: f64) -> (u64, bool, Vec<u64>, u64) {
    let mut rig = Rig::new(None);
    rig.queue.settings.round = ROUND;
    rig.queue.settings.brick_samples = 2000;
    rig.queue.ring1_stretch = stretch;
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
    let mut open = Vec::new();
    for (i, &n) in ring1.iter().enumerate() {
        let c = [i as i32, 0, 0];
        let mut j = job(c, (n / 2000) as u16);
        assert_eq!(j.samples, n);
        j.class = JobClass::Ring1;
        open.push(ctx.open(j, i + 1));
        rig.queue.dig_open.insert(
            c,
            DigChunk {
                near: true,
                ..DigChunk::default()
            },
        );
    }
    let other = job(OTHER, 2000);
    assert_eq!(other.samples, 4_000_000);
    open.push(ctx.open(other, 0));
    rig.queue.lp.open = open;
    rig.queue.lp.free.clear();
    rig.queue.lp.pending.clear();
    let chunks = rig.queue.ring1_chunks();
    let (cap, stretched) = ring1_cap(&rig.queue.lp, rig.queue.ring1_stretch, CAP, &chunks);
    rig.queue.lp.dispatch(
        &run,
        &mut rig.half,
        &mut rig.store,
        &settings,
        cap,
        &mut |_: &mut Store, _, _, _, _| {},
        &mut |_| {},
    );
    let drawn = |c: [i32; 3]| {
        rig.queue
            .lp
            .open
            .iter()
            .find(|j| j.coord.0 == c)
            .map_or(0, |j| j.states.iter().map(|s| u64::from(s.queued)).sum())
    };
    let ring = (0..ring1.len()).map(|i| drawn([i as i32, 0, 0])).collect();
    (cap, stretched, ring, drawn(OTHER))
}

/// Done when (1) of Plan 0099 Phase 2e. A ring-1 job of 240 000 samples ahead of an open job
/// that is not a dig's with 4 000 000 left, a cap of 200 000: at 1.0 and at 1.125 (225 000)
/// the submit plans 200 000 of ring 1, at most one round over, and none of the other job; at
/// 1.5 (300 000) all 240 000 and none of the other. Two ring-1 jobs of 160 000 and 96 000 at
/// 1.5: both whole, 256 000. A ring-1 job with 144 000 left at 1.5: the cap stays 200 000 and
/// the other job takes 56 000, at most one round over.
#[test]
fn ring_one_within_the_stretch_goes_whole() {
    let round = u64::from(ROUND);
    for stretch in [1.0, 1.125] {
        let (cap, stretched, ring, other) = plan(&[240_000], stretch);
        assert_eq!((cap, stretched), (CAP, false), "{stretch}: the priced cap");
        assert!(
            (CAP..=CAP + round).contains(&ring[0]),
            "{stretch}: ring 1 planned {}",
            ring[0]
        );
        assert_eq!(other, 0, "{stretch}: the other job takes no round");
    }
    let (cap, stretched, ring, other) = plan(&[240_000], 1.5);
    assert_eq!(
        (cap, stretched),
        (240_000, true),
        "1.5: the cap is ring 1's"
    );
    assert_eq!((ring[0], other), (240_000, 0), "1.5: ring 1 whole, alone");

    let (cap, stretched, ring, other) = plan(&[160_000, 96_000], 1.5);
    assert_eq!(
        (cap, stretched),
        (256_000, true),
        "two jobs: the cap is both"
    );
    assert_eq!((ring, other), (vec![160_000, 96_000], 0), "two jobs whole");

    let (cap, stretched, ring, other) = plan(&[144_000], 1.5);
    assert_eq!((cap, stretched), (CAP, false), "under the cap: no stretch");
    assert_eq!(ring[0], 144_000, "ring 1 whole under the cap");
    assert!(
        (CAP - 144_000..=CAP - 144_000 + round).contains(&other),
        "the other job takes the rest of the cap: {other}"
    );
}

/// Done when (2) of Plan 0099 Phase 2e, lockstep. The rig starts at 1.0 (set by name: the
/// default is 1.5 since Phase 3b, under which this ring goes whole unset). The stretch set to 1.5
/// between frames 3 and 4 waits in the mailbox (the books keep 1.0 until the step), and the step
/// that reads message 4 plans by it: its dig's ring 1 (280 samples in this rig) against a cap of
/// 200 goes whole, at 280, where the same rig without the set sends the cap.
#[test]
fn the_stretch_reaches_the_bake_side_in_the_next_message() {
    for set in [false, true] {
        let mut rig = Rig::new(None);
        rig.queue.ring1_stretch = 1.0;
        for _ in 0..3 {
            rig.step();
        }
        assert_eq!(rig.half.frame, 3);
        if set {
            rig.inbox
                .push_set(Box::new(|q: &mut BakeQueue| q.ring1_stretch = 1.5));
        }
        assert_eq!(
            rig.queue.ring1_stretch, 1.0,
            "the set writes nothing before the step"
        );
        rig.half.hold = true;
        rig.carve([0, 20, 4], [3, 23, 7]);
        rig.samples = Some(200);
        let msg = rig.message();
        assert_eq!(msg.frame, 4);
        rig.run(msg);
        let st = rig.queue.stats;
        assert!(!rig.queue.ring1_chunks().is_empty(), "a ring-1 job is open");
        if set {
            assert_eq!(rig.queue.ring1_stretch, 1.5);
            assert_eq!((st.cap, st.stretched), (280, true), "the step plans by 1.5");
            assert_eq!(st.submitted, 280, "ring 1 whole");
        } else {
            assert_eq!(
                (st.cap, st.stretched),
                (200, false),
                "the priced cap at 1.0"
            );
            assert!(st.submitted < 280, "ring 1 cut: {}", st.submitted);
        }
    }
}
