//! The listing pool (Plan 0099 Phase 4, lever (b)) on the device-less rig: the openings hand
//! their batches to the pool and push what it finished in a later step.

use super::*;

/// Steps `rig` until its queue is empty, each step followed by the wait for every listing it
/// sent to the pool ([`BakeQueue::wait_listed`]; at once with no pool), so the next step finds
/// them done; the frames it took.
fn drain(rig: &mut Rig) -> u32 {
    for frame in 0..4000 {
        if rig.read().empty() && rig.queue.listing.is_empty() {
            return frame;
        }
        rig.step();
        rig.queue.wait_listed();
    }
    panic!("the queue drains within 4000 frames");
}

/// Every chunk's block words, by chunk.
fn blocks(rig: &Rig) -> Vec<([i32; 3], Vec<u32>)> {
    let mut out: Vec<_> = rig
        .world
        .store()
        .coords()
        .into_iter()
        .filter_map(|c| rig.store.block_words(c).map(|w| (c.0, w.to_vec())))
        .collect();
    out.sort();
    out
}

/// A sliced rig drained with its listings handed to the pool holds every chunk's block as the
/// rig that lists in its own step does, and pushes no chunk twice: the pool changes when a job
/// opens, not what it bakes.
#[test]
fn the_listing_pool_drains_to_the_blocks_the_step_lists() {
    let mut step = Rig::new(None);
    let mut pool = Rig::new(None);
    pool.queue
        .start_lister(Arc::new(world_of(CHUNKS[0])))
        .expect("the pool's threads start");
    assert!(pool.queue.lists_off_the_loop());
    let (a, b) = (drain(&mut step), drain(&mut pool));
    let (sa, sb) = (blocks(&step), blocks(&pool));
    assert!(!sa.is_empty(), "the rig bakes something");
    assert_eq!(
        sa.iter().map(|(c, w)| (*c, w.len())).collect::<Vec<_>>(),
        sb.iter().map(|(c, w)| (*c, w.len())).collect::<Vec<_>>(),
        "the same chunks hold blocks of the same size (drained in {a} and {b} frames)"
    );
    assert_eq!(sa, sb, "every block's words are the same");
    pool.queue.stop_lister();
    assert!(!pool.queue.lists_off_the_loop());
}

/// The world of `n` chunks along x, each holding a lone rock voxel at the middle of each of its
/// first `k` bricks: every such brick lists six directions of one face each, so a job's samples
/// are `6 k` times its count, and the listing bound from the chunk's prepared masks is exact.
fn lone_voxels(n: i32, k: usize) -> World {
    let mut t = MaterialTable::new();
    let rock = t.push(Material::named("rock"));
    let w = World::generate(
        ChunkExtent::new([0, 0, 0], [n, 1, 1]),
        &UniformSource(AIR),
        &t,
    );
    for x in 0..n {
        for slot in 0..k {
            let b = svet_world::brick_of_slot(slot);
            let v = [64 * x + b[0] * 8 + 3, b[1] * 8 + 3, b[2] * 8 + 3].map(i64::from);
            // A box holds the voxels whose centres lie in `min..max`.
            w.apply(&box_op(v, v.map(|c| c + 1), Mode::Fill(rock)))
                .expect("resident");
        }
    }
    w
}

/// A sliced rig over [`lone_voxels`]`(n, k)` listing through the pool, its jobs at `count` a
/// brick in rounds of one face's count, no coarse pass, a slice of `slice_ms` a frame; with
/// `masks`, every chunk's prepared masks held as the stream workers would hand them.
fn pooled_rig(n: i32, k: usize, count: u32, slice_ms: f64, masks: bool) -> Rig {
    let mut rig = sliced_rig(n, k, count, slice_ms, masks);
    rig.queue
        .start_lister(Arc::new(lone_voxels(n, k)))
        .expect("the pool's threads start");
    rig
}

/// [`pooled_rig`] in lockstep: the openings list inside the step.
fn sliced_rig(n: i32, k: usize, count: u32, slice_ms: f64, masks: bool) -> Rig {
    let mut rig = Rig::sized(None, n, n);
    rig.world = lone_voxels(n, k);
    rig.queue.settings.brick_samples = count;
    rig.queue.settings.round = count;
    rig.queue.coarse_samples = 0;
    rig.queue.budget = Budget::PerFrame { ms: slice_ms };
    if masks {
        let classes = rig.queue.classes(rig.world.materials());
        for c in rig.world.store().coords() {
            let chunk = rig.world.store().get(c).expect("resident");
            let chunk = chunk.read().expect("unpoisoned");
            let m = PreparedMasks {
                classes: classes.clone(),
                masks: ChunkMasks::of(&chunk, &classes),
                ns: 0,
                seal_ns: 0,
                verdicts: Vec::new(),
                shells: Default::default(),
            };
            rig.queue.prepared_bytes += m.masks.bytes();
            rig.queue.prepared.insert(c.0, Arc::new(m));
        }
    }
    rig
}

/// One frame at a price of 2 ns a sample and no fixed part, then the wait for every listing the
/// step sent: the candidates it sent and whether it opened.
fn pooled_step(rig: &mut Rig) -> (u64, bool) {
    rig.queue.cost = SampleCost::new(2.0);
    let (seq, openings) = (rig.queue.listing_seq, rig.queue.stats.openings);
    rig.step();
    rig.queue.wait_listed();
    (
        rig.queue.listing_seq - seq,
        rig.queue.stats.openings > openings,
    )
}

/// A lone-voxel chunk of 53 bricks at 625 a brick: 198 750 samples, its bound exact.
const LONE: (usize, u32) = (53, 625);

/// Done when (1) of Plan 0099 Phase 4b. Candidates of 198 750 samples (the rig's nearest to
/// 200 000 with exact bounds: a job's samples are six faces a lone voxel), a price of 2 ns and
/// no fixed part. Under a 4.0 ms slice (cap 2 000 000, level 6 000 000) an empty loop's opening
/// sends 31 (30 x 198 750 is under the level), more than `QUEUE_CHECKS`, in the walk's order
/// from its start; each later opening, the loop's samples left one cap lower, sends 10 or 11.
/// Under 1.0 ms (cap 500 000, level 1 500 000) the first sends 8 or 9 and the next ten send 2
/// to 4 each and 25 or 26 together.
#[test]
fn the_opening_admits_until_the_projected_samples_cover_the_slice() {
    let (k, count) = LONE;
    let samples = 6 * k as u64 * u64::from(count);
    assert_eq!(samples, 198_750);
    // Enough chunks that the walk never runs out in these steps.
    let mut rig = pooled_rig(120, k, count, 4.0, true);
    let (sent, opened) = pooled_step(&mut rig);
    assert!(opened, "an empty loop opens");
    assert!((30..=31).contains(&sent), "the first opening sends {sent}");
    assert!(sent > QUEUE_CHECKS as u64);
    let order = rig.queue.listing_order();
    let walk: Vec<[i32; 3]> = (0..sent as i32).map(|x| [x, 0, 0]).collect();
    assert_eq!(
        order, walk,
        "the candidates sent are the walk's order from its start"
    );
    let mut later = Vec::new();
    for _ in 0..6 {
        let (sent, opened) = pooled_step(&mut rig);
        if opened {
            later.push(sent);
        }
    }
    assert!(later.len() >= 3, "later openings {later:?}");
    assert!(
        later.iter().all(|s| (10..=11).contains(s)),
        "each later opening sends 10 or 11: {later:?}"
    );
    assert_eq!(rig.queue.fill.of(true), 1.0, "exact bounds fill 1");
    rig.queue.stop_lister();

    let mut rig = pooled_rig(48, k, count, 1.0, true);
    let (sent, opened) = pooled_step(&mut rig);
    assert!(
        opened && (8..=9).contains(&sent),
        "the first opening at 1.0 ms sends {sent}"
    );
    let mut later = Vec::new();
    for _ in 0..40 {
        if later.len() == 10 {
            break;
        }
        let (sent, opened) = pooled_step(&mut rig);
        if opened {
            later.push(sent);
        }
    }
    assert_eq!(later.len(), 10, "ten later openings: {later:?}");
    assert!(later.iter().all(|s| (2..=4).contains(s)), "{later:?}");
    let total: u64 = later.iter().sum();
    assert!(
        (25..=26).contains(&total),
        "the ten send {total}: {later:?}"
    );
    rig.queue.stop_lister();
}

/// Done when (2) of Plan 0099 Phase 4b. Chunks listed from the world (no masks) count
/// `512 x 6` directions at 261 a brick: a bound of 801 792. Before 16 listings have returned,
/// an empty loop under a cap of 2 000 000 sends 8 (7 x 801 792 is under the level of
/// 6 000 000). Chunks of 128 lone voxels list a quarter of that bound; once 64 of them have
/// returned through the pool, an empty loop holding that fill sends 30 (30 x 801 792 x 0.25
/// covers the level, 29 do not).
#[test]
fn a_pooled_candidate_counts_its_bound_times_the_fill() {
    const COUNT: u32 = 261;
    let bound = CHUNK_BRICKS as u64 * 6 * u64::from(COUNT);
    assert_eq!(bound, 801_792);
    let mut rig = pooled_rig(100, 128, COUNT, 4.0, false);
    let (sent, opened) = pooled_step(&mut rig);
    assert!(opened);
    assert_eq!(sent, 8, "before 16 have returned the bound counts whole");
    for _ in 0..40 {
        if rig.queue.listing_seq >= 64 + 8 {
            break;
        }
        pooled_step(&mut rig);
    }
    // One step more pushes the last of them.
    rig.queue.cost = SampleCost::new(2.0);
    rig.step();
    assert!(rig.queue.listing_seq >= 64, "64 listings returned");
    assert_eq!(
        rig.queue.fill.of(false),
        0.25,
        "each listed a quarter of its bound"
    );
    rig.queue.stop_lister();
    let mut empty = pooled_rig(48, 128, COUNT, 4.0, false);
    empty.queue.fill = std::mem::take(&mut rig.queue.fill);
    let (sent, opened) = pooled_step(&mut empty);
    assert!(opened);
    assert_eq!(sent, 30, "at the measured fill");
    empty.queue.stop_lister();
}

/// Done when (3) of Plan 0099 Phase 4b. An empty idle loop, the mirror unchanged, an opening
/// that sends its four candidates to the pool and pushes nothing: it is not quiet, and their
/// jobs are in the loop after the next step, not eight steps on.
#[test]
fn an_opening_that_only_sends_is_not_quiet() {
    let mut rig = Rig::sized(None, 4, 4);
    rig.queue
        .start_lister(Arc::new(world_of(4)))
        .expect("the pool's threads start");
    let mut sent = 0;
    for _ in 0..20 {
        let before = rig.queue.listing_seq;
        rig.step();
        sent = rig.queue.listing_seq - before;
        if sent > 0 {
            break;
        }
    }
    assert_eq!(sent, 4, "the opening sends the four");
    assert_eq!(rig.queue.lp.chunks().count(), 0, "and pushes nothing");
    assert!(rig.queue.quiet_until <= rig.queue.steps, "not quiet");
    rig.queue.wait_listed();
    rig.step();
    assert_eq!(
        rig.queue.lp.chunks().count(),
        4,
        "the jobs after the next step"
    );
    rig.queue.stop_lister();
}

/// Done when (2) of Plan 0099 Phase 4c. With every read-back withheld, the bake thread's half
/// (the listing pool on) plans as many submits as its depth and counts each step after the
/// last as blocked, every slot unread; lockstep's plans two whatever the depth is set to. The
/// slice is 1.0 ms (a cap of 500 000), so the 16 table slots hold more than four caps.
#[test]
fn the_thread_plans_to_its_depth_and_lockstep_to_two() {
    let (k, count) = LONE;
    const STEPS: u64 = 12;
    // The steps a rig takes to plan its first `bound` submits, then its submits and blocked
    // steps after `STEPS`.
    let run = |rig: &mut Rig, bound: usize| {
        rig.half.hold = true;
        let mut full_at = None;
        for step in 1..=STEPS {
            pooled_step(rig);
            if full_at.is_none() && rig.half.issued.len() == bound {
                full_at = Some(step);
            }
        }
        (
            full_at,
            rig.half.issued.len(),
            rig.queue.stats.in_flight_full,
        )
    };
    for depth in [2, 3, 4] {
        let mut rig = pooled_rig(40, k, count, 1.0, true);
        rig.queue.set_thread_depth(depth);
        assert_eq!(rig.queue.in_flight_bound(), depth);
        let (full_at, issued, blocked) = run(&mut rig, depth);
        let full_at = full_at.expect("the depth is planned");
        assert_eq!(issued, depth, "depth {depth}: as many submits as the depth");
        assert_eq!(
            blocked,
            STEPS - full_at,
            "depth {depth}: every step after the last is blocked"
        );
        rig.queue.stop_lister();
        assert_eq!(rig.queue.in_flight_bound(), IN_FLIGHT, "lockstep again");
    }
    let mut rig = sliced_rig(40, k, count, 1.0, true);
    rig.queue.set_thread_depth(4);
    let (full_at, issued, blocked) = run(&mut rig, IN_FLIGHT);
    assert_eq!(issued, 2, "lockstep plans two");
    assert_eq!(blocked, STEPS - full_at.expect("two are planned"));
}

/// Plan 0101 Phase 4, done-when (1). The rule's rig (`C` buried, light radius 1) listing through
/// the pool: an opening rules `C` and hands its listing to the pool, which finishes it. Before the
/// step that pushes it, a message lowers a footprint column's top (column (2, 2)) to `C`'s reach
/// box, where the rule does not hold. After that step `C` is not zeroed and holds no zero record, and
/// it settles with one job's block.
#[test]
fn a_ruled_verdict_is_read_again_where_its_record_is_written() {
    use super::rule::{BOX_TOP, C, jobs, rock_world, rule_rig, settle};
    let mut rig = rule_rig(3);
    rig.queue
        .start_lister(Arc::new(rock_world([3, 3, 3])))
        .expect("the pool's threads start");
    rig.until("C in the pool's hands", |r| r.queue.in_listing(C));
    assert_eq!(
        rig.queue.listing_ruled(C),
        Some(true),
        "the opening ruled C"
    );
    rig.queue.wait_listed();
    assert_eq!(rig.queue.stats.zeroed, 0, "not pushed yet");

    rig.mirror.tops.insert([2, 2], BOX_TOP);
    rig.mirror.changed.push(ChunkCoord([2, 2, 2]));
    rig.step();
    assert_eq!(rig.queue.stats.zeroed, 0, "no zero record was written");
    assert!(!rig.queue.zeroed(ChunkCoord(C)), "C is not zeroed");

    settle(&mut rig, "C's job");
    assert_eq!(jobs(&rig), 1, "C settles with one job's block");
    assert_eq!(rig.queue.stats.zeroed, 0);
    assert!(!rig.queue.zeroed(ChunkCoord(C)));
    rig.queue.stop_lister();
}

/// The rule's rig listing through the pool, stepped until an opening has ruled `C` and the pool
/// has finished its listing.
fn ruled_in_the_pools_hands() -> Rig {
    use super::rule::{C, rock_world, rule_rig};
    let mut rig = rule_rig(3);
    rig.queue
        .start_lister(Arc::new(rock_world([3, 3, 3])))
        .expect("the pool's threads start");
    rig.until("C in the pool's hands", |r| r.queue.in_listing(C));
    assert_eq!(
        rig.queue.listing_ruled(C),
        Some(true),
        "the opening ruled C"
    );
    rig.queue.wait_listed();
    assert_eq!(rig.queue.stats.zeroed, 0, "not pushed yet");
    rig
}

/// Plan 0101 Phase 5, done-when (1). A step with no change pushes `C`'s ruled listing and opens:
/// between the push and the submit that places its block `C` has no block, no job and no flag, and
/// the opening's sync reads the kept pool against a whole reading. After the step `C` is zeroed
/// once, out of the pool's hands and not a candidate; it settles with no job and the zero record.
#[test]
fn a_pooled_ruled_push_keeps_the_pool_a_whole_reading() {
    use super::rule::{C, jobs, settle, zero_of_c};
    let mut rig = ruled_in_the_pools_hands();

    let openings = rig.queue.stats.openings;
    rig.step();
    assert!(
        rig.queue.stats.openings > openings,
        "the pushing step opened"
    );
    assert_eq!(
        rig.queue.stats.zeroed, 1,
        "C's zero record was written once"
    );
    assert!(rig.queue.zeroed(ChunkCoord(C)), "C is zeroed");
    assert!(!rig.queue.in_listing(C), "C is out of the pool's hands");
    assert!(!rig.queue.pool.contains_key(&C), "C is not a candidate");

    settle(&mut rig, "C's zero record");
    assert_eq!(jobs(&rig), 0, "C settles with no job");
    assert_eq!(rig.queue.stats.zeroed, 1);
    let zero = zero_of_c(&mut rig);
    assert_eq!(
        rig.store.block_words(ChunkCoord(C)).map(<[u32]>::to_vec),
        Some(zero),
        "C holds the zero record"
    );
    rig.queue.stop_lister();
}

/// Plan 0101 Phase 5, done-when (2). Before the step that pushes `C`'s ruled listing, a message
/// names `C` among the mirror's changes with the column tops as they were (the rule still holds):
/// the pushing step's opening reads `C` again. After that step `C` is zeroed and out of the
/// pool's hands, and it settles listed once: one zero record, no job.
#[test]
fn a_pooled_ruled_chunk_is_listed_once_under_a_change_that_keeps_the_rule() {
    use super::rule::{C, jobs, settle};
    let mut rig = ruled_in_the_pools_hands();

    rig.mirror.changed.push(ChunkCoord(C));
    rig.step();
    assert!(rig.queue.zeroed(ChunkCoord(C)), "C is zeroed");
    assert!(!rig.queue.in_listing(C), "C is out of the pool's hands");

    settle(&mut rig, "C's zero record");
    rig.queue.wait_listed();
    for _ in 0..4 {
        rig.step();
        rig.queue.wait_listed();
    }
    assert_eq!(rig.queue.stats.zeroed, 1, "C was listed and zeroed once");
    assert_eq!(jobs(&rig), 0, "no job");
    rig.queue.stop_lister();
}

/// One step of [`slots_run`]: the samples of the submit it sent (0 for none), whether it was
/// counted blocked, whether it was counted neither blocked nor sent, and the jobs open after it.
#[derive(Debug, PartialEq)]
struct SlotsStep {
    sent: u64,
    blocked: bool,
    idle: bool,
    open: usize,
}

/// `steps` frames of the rig, its read-backs withheld (a lone-voxel face draws one round of
/// `count`), each step's [`SlotsStep`]; and the plans out of rounds with no table slot free.
fn slots_run(rig: &mut Rig, steps: usize, count: u32) -> (Vec<SlotsStep>, u64) {
    rig.half.hold = true;
    let mut out = Vec::new();
    for _ in 0..steps {
        let before = rig.half.issued.len();
        let (blocked, idle) = (rig.queue.stats.in_flight_full, rig.queue.stats.idle_steps);
        if rig.queue.lists_off_the_loop() {
            pooled_step(rig);
        } else {
            rig.queue.cost = SampleCost::new(2.0);
            rig.step();
        }
        let issued = &rig.half.issued[before..];
        assert!(issued.len() <= 1, "one submit a step");
        out.push(SlotsStep {
            sent: issued
                .iter()
                .map(|(_, slots)| slots.len() as u64 * u64::from(count))
                .sum(),
            blocked: rig.queue.stats.in_flight_full > blocked,
            idle: rig.queue.stats.idle_steps > idle,
            open: rig.queue.lp.open.len(),
        });
    }
    (out, rig.queue.lp.slotless_stops)
}

/// The submits of `steps`, each its samples and the jobs open after it.
fn sent(steps: &[SlotsStep]) -> Vec<(u64, usize)> {
    steps
        .iter()
        .filter(|s| s.sent > 0)
        .map(|s| (s.sent, s.open))
        .collect()
}

/// [`pooled_rig`] over 64 chunks at a 4.0 ms slice, its bake thread's table slots `slots` and
/// its depth `depth`.
fn slots_rig(slots: usize, depth: usize) -> Rig {
    let (k, count) = LONE;
    let mut rig = sliced_rig(64, k, count, 4.0, true);
    rig.queue.set_thread_slots(slots);
    rig.queue.set_thread_depth(depth);
    rig.queue
        .start_lister(Arc::new(lone_voxels(64, k)))
        .expect("the pool's threads start");
    rig
}

/// Done when (1) of Plan 0099 Phase 4c2. The pooled rig, read-backs withheld, jobs of 198 750
/// samples in 318 faces of 625 ([`LONE`]), a 4.0 ms slice at 2 ns (cap 2 000 000), depth 3. At
/// 16 table slots: two submits, 2 000 000 (ten jobs whole and 20 faces of the eleventh) and
/// 1 180 000 (the eleventh's 186 250 and five jobs), the second out of rounds with no slot
/// free, and the step after sends nothing and is not counted blocked. At 32: three submits of
/// 2 000 000, 21 jobs open after the second and 31 after the third, none stopped for a slot. At
/// 32 and depth 4 the fourth is 360 000 (the thirty-first's 161 250 and one job), out of rounds
/// with no slot free; at 48 it is 2 000 000 with 41 open. The rig's first step hands its
/// candidates to the pool and sends nothing; its submits start at the second.
#[test]
fn the_threads_slots_hold_its_depth_in_caps() {
    let count = LONE.1;
    let (steps, slotless) = slots_run(&mut slots_rig(16, 3), 6, count);
    assert_eq!(sent(&steps), [(2_000_000, 11), (1_180_000, 16)], "16 slots");
    assert_eq!(
        slotless, 1,
        "16 slots: the second out of rounds with no slot free"
    );
    let second = steps.iter().rposition(|s| s.sent > 0).expect("two sent");
    assert_eq!(
        steps[second + 1],
        SlotsStep {
            sent: 0,
            blocked: false,
            idle: true,
            open: 16
        },
        "16 slots: the step after the second sends nothing, not blocked: {steps:?}"
    );

    let (steps, slotless) = slots_run(&mut slots_rig(32, 3), 6, count);
    assert_eq!(
        sent(&steps),
        [(2_000_000, 11), (2_000_000, 21), (2_000_000, 31)],
        "32 slots"
    );
    assert_eq!(slotless, 0, "32 slots: none stopped for a slot");

    let (steps, slotless) = slots_run(&mut slots_rig(32, 4), 7, count);
    assert_eq!(
        sent(&steps),
        [
            (2_000_000, 11),
            (2_000_000, 21),
            (2_000_000, 31),
            (360_000, 32)
        ],
        "32 slots, depth 4"
    );
    assert_eq!(
        slotless, 1,
        "32 slots, depth 4: the fourth with no slot free"
    );

    let (steps, slotless) = slots_run(&mut slots_rig(48, 4), 7, count);
    assert_eq!(
        sent(&steps),
        [
            (2_000_000, 11),
            (2_000_000, 21),
            (2_000_000, 31),
            (2_000_000, 41)
        ],
        "48 slots, depth 4"
    );
    assert_eq!(slotless, 0, "48 slots, depth 4");
}

/// Done when (2) of Plan 0099 Phase 4c2: the same jobs in lockstep (no pool), the bake
/// thread's count set to 64, plan 2 000 000 then 1 180 000 and hold 16 jobs, every slot in the
/// last 16 table rows.
#[test]
fn lockstep_keeps_sixteen_slots_whatever_the_count() {
    let (k, count) = LONE;
    let mut rig = sliced_rig(64, k, count, 4.0, true);
    rig.queue.set_thread_slots(64);
    let (steps, slotless) = slots_run(&mut rig, 6, count);
    assert_eq!(sent(&steps), [(2_000_000, 11), (1_180_000, 16)]);
    assert_eq!(slotless, 1, "the second out of rounds with no slot free");
    let lockstep = OPEN_JOBS - QUEUE_SLOTS..OPEN_JOBS;
    assert_eq!(rig.queue.lp.open.len(), 16);
    assert!(
        rig.queue.lp.open.iter().all(|j| lockstep.contains(&j.slot)),
        "every slot in the last 16 rows"
    );
    let held: Vec<u32> = rig
        .half
        .issued
        .iter()
        .flat_map(|(_, slots)| slots.iter().copied())
        .collect();
    assert!(held.iter().all(|&s| lockstep.contains(&(s as usize))));
}

/// Done when (3) of Plan 0099 Phase 4c2: the pool stopped with 31 jobs open (32 slots, depth
/// 3); the read-backs released, every job opened after the stop takes one of the last 16 rows,
/// and once the 31 have closed the loop's free rows are those 16 less those the later jobs hold.
#[test]
fn a_join_returns_the_loop_to_its_sixteen() {
    let count = LONE.1;
    let mut rig = slots_rig(32, 3);
    let (steps, _) = slots_run(&mut rig, 6, count);
    assert_eq!(sent(&steps).last(), Some(&(2_000_000, 31)));
    rig.queue.stop_lister();
    let lockstep = OPEN_JOBS - QUEUE_SLOTS..OPEN_JOBS;
    let first: std::collections::HashSet<[i32; 3]> =
        rig.queue.lp.open.iter().map(|j| j.coord.0).collect();
    assert_eq!(first.len(), 31);
    assert!(
        rig.queue.lp.free.iter().all(|s| lockstep.contains(s)),
        "no slot outside the 16 is free after the stop"
    );
    rig.half.hold = false;
    let mut closed_at = None;
    let mut later = std::collections::HashSet::new();
    for frame in 0..400 {
        rig.queue.cost = SampleCost::new(2.0);
        rig.step();
        let lp = &rig.queue.lp;
        for j in lp.open.iter().filter(|j| !first.contains(&j.coord.0)) {
            later.insert(j.coord.0);
            assert!(
                lockstep.contains(&j.slot),
                "a job opened after the stop in slot {}",
                j.slot
            );
        }
        assert!(lp.free.iter().all(|s| lockstep.contains(s)));
        let left = lp.open.iter().any(|j| first.contains(&j.coord.0))
            || lp.withdrawn.iter().any(|j| first.contains(&j.coord.0));
        if !left && closed_at.is_none() {
            closed_at = Some(frame);
            let mut rows: Vec<usize> = lp
                .free
                .iter()
                .copied()
                .chain(lp.open.iter().map(|j| j.slot))
                .chain(lp.withdrawn.iter().map(|j| j.slot))
                .collect();
            rows.sort_unstable();
            assert_eq!(
                rows,
                lockstep.clone().collect::<Vec<_>>(),
                "the free rows and the later jobs' are the 16"
            );
        }
        if rig.read().empty() {
            break;
        }
    }
    assert!(closed_at.is_some(), "the 31 close");
    assert!(
        later.len() >= QUEUE_SLOTS,
        "jobs opened after the stop: {}",
        later.len()
    );
    let mut free = rig.queue.lp.free.clone();
    free.sort_unstable();
    assert_eq!(free, lockstep.collect::<Vec<_>>(), "drained: the 16 free");
}
