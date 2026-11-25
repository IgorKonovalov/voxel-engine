//! The listing pool (Plan 0099 Phase 4, lever (b)) on the device-less rig: the openings hand
//! their batches to the pool and push what it finished in a later step.

use super::*;

/// Steps `rig` until its queue is empty, sleeping a little while the listing pool holds
/// candidates (its threads run beside the rig's steps); the frames it took.
fn drain(rig: &mut Rig) -> u32 {
    for frame in 0..4000 {
        if rig.read().empty() && rig.queue.listing.is_empty() {
            return frame;
        }
        if !rig.queue.listing.is_empty() {
            std::thread::sleep(std::time::Duration::from_micros(200));
        }
        rig.step();
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
