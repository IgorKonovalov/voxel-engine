//! The live queue (ADR-0046's scheduler), the fly-through's light, its order: stepped headless on
//! the software adapter as the window's frame steps it (the mirror synced, then
//! `Frame::bake_queue_step` with no sample count, so each step is sized by its slice and waits on
//! nothing), its store read back against one-shot bakes of the same world.
//!
//! Every world here is a few hand-built boxes at test counts (64 samples a brick direction, a
//! coarse pass of 16); the rigs are `support::queue`'s.
//!
//! This binary reads the order: settling to a direct bake, near first, the score, a turn's re-sort,
//! and what is withdrawn and what is not.

mod support;

use support::SyncNow;
use support::queue::*;

use flythrough::cave::{ORE, ROCK};
use vxe_render::BakeSettings;
use vxe_world::{AIR, ChunkCoord, Mode};

/// The two-chunk box through the live queue, coarse pass on (chunk 1 lies beyond the near ring
/// of 0, so it is lit coarse first and refined), until the queue is empty: every block is
/// bit-identical to a one-shot bake at the same counts. Then 32 more frames: the queue submits
/// nothing and reads nothing back, and the last two frames are bit-identical.
#[test]
fn the_queue_settles_to_a_direct_bake() {
    let (world, eye) = border_box();
    let mut q = match rig(
        world,
        &Setup {
            coarse: COARSE,
            near: 0,
            ..Setup::at(eye)
        },
    ) {
        Some(q) => q,
        None => return,
    };
    let mut coarse_seen = 0;
    for n in 1..=MAX_STEPS {
        let st = q.step();
        coarse_seen = coarse_seen.max(st.coarse);
        if q.q().empty() {
            println!("empty after {n} steps: {st:?}");
            break;
        }
    }
    assert!(q.q().empty(), "the queue settles");
    assert_eq!(coarse_seen, 1, "chunk 1 is lit coarse first");
    let got = q.blocks();
    assert_eq!(got.len(), 2, "both chunks hold a block");
    let fresh = q.fresh();
    for (c, b) in &got {
        assert_eq!(Some(b), fresh.get(c), "{c:?} against a one-shot bake");
        assert_eq!(
            q.target(*c).as_ref(),
            Some(b),
            "{c:?}: the pool shows the store's copy"
        );
    }
    let mut frames = Vec::new();
    for _ in 0..32 {
        let st = q.step();
        assert_eq!(
            (st.submitted, st.samples, st.jobs, st.fading),
            (0, 0, 0, 0),
            "an empty queue dispatches nothing"
        );
        frames.push(q.render());
    }
    assert_eq!(frames[30], frames[31], "two frames at rest");
}

/// [`the_queue_settles_to_a_direct_bake`] with the camera turned to look -x half-way: once
/// chunk 1's coarse block lands. Every block is still bit-identical to a one-shot bake.
#[test]
fn the_queue_settles_to_a_direct_bake_after_a_turn() {
    let (world, eye) = border_box();
    let mut q = match rig(
        world,
        &Setup {
            coarse: COARSE,
            near: 0,
            ..Setup::at(eye)
        },
    ) {
        Some(q) => q,
        None => return,
    };
    let mut turned = false;
    for n in 1..=MAX_STEPS {
        let st = q.step();
        if !turned && st.coarse > 0 {
            q.camera.yaw = std::f32::consts::PI;
            turned = true;
            println!("turned after {n} steps");
        }
        if q.q().empty() {
            println!("empty after {n} steps: {st:?}");
            break;
        }
    }
    assert!(turned, "chunk 1 was lit coarse");
    assert!(q.q().empty(), "the queue settles");
    let got = q.blocks();
    assert_eq!(got.len(), 2, "both chunks hold a block");
    let fresh = q.fresh();
    for (c, b) in &got {
        assert_eq!(Some(b), fresh.get(c), "{c:?} against a one-shot bake");
        assert_eq!(
            q.target(*c).as_ref(),
            Some(b),
            "{c:?}: the pool shows the store's copy"
        );
    }
}

/// Six chunks in a row, a rock block in each and the camera in chunk 2 looking along +x, coarse
/// off: the jobs open by squared chunk distance from the camera's chunk, ties by the view (the
/// chunk ahead first), so 2, 3, 1, 4, 0, 5. At the smallest slice no step's submit plans more
/// than the step's cap but for the one entry (a round, 64 samples) that crosses it.
#[test]
fn the_queue_runs_near_first_and_within_its_samples() {
    let Some(mut q) = rig(
        row(&[0, 1, 2, 3, 4, 5]),
        &Setup {
            slice_ms: 1e-6,
            ..Setup::at(ROW_EYE)
        },
    ) else {
        return;
    };
    let mut order: Vec<i32> = Vec::new();
    let mut steps = 0;
    loop {
        let st = q.step();
        steps += 1;
        assert!(steps <= MAX_STEPS, "the queue settles");
        assert!(
            st.submitted < st.cap + 64,
            "step {steps}: {} samples planned against the cap {}",
            st.submitted,
            st.cap
        );
        for c in q.q().jobs() {
            if !order.contains(&c.0[0]) {
                order.push(c.0[0]);
            }
        }
        if q.q().empty() {
            break;
        }
    }
    println!("{steps} steps, opened {order:?}");
    assert!(steps > 3, "the cap spreads the work over steps");
    assert_eq!(order, [2, 3, 1, 4, 0, 5]);
}

/// Steps until the queue is empty: the chunks' x in the order they first appear in `jobs()`.
fn open_order(q: &mut Q) -> Vec<i32> {
    let mut order = Vec::new();
    for _ in 0..MAX_STEPS {
        q.step();
        for c in q.q().jobs() {
            if !order.contains(&c.0[0]) {
                order.push(c.0[0]);
            }
        }
        if q.q().empty() {
            return order;
        }
    }
    panic!("the queue is not empty after {MAX_STEPS} steps");
}

/// ADR-0048's score on the row, camera in chunk 2 looking +x, `near` 0, coarse pass on, refine
/// weight 2: the chunks first open by `|o| x (1 + W x a)`, behind the camera `a` 0.632. At
/// angle weight 4: ahead 1, 2, 3, behind 1 x 3.53 and 2 x 3.53, so 2, 3, 4, 5, 1, 0; at 1:
/// behind 1.63 and 3.26, so 2, 3, 1, 4, 5, 0; at 0 the distance, ties ahead first, so 2, 3, 1,
/// 4, 0, 5, as under the distance order. About 15 s on lavapipe: soak.
#[test]
#[ignore = "soak"]
fn the_score_blends_distance_and_the_view() {
    for (order, want) in [
        (score(4.0), [2, 3, 4, 5, 1, 0]),
        (score(1.0), [2, 3, 1, 4, 5, 0]),
        (score(0.0), [2, 3, 1, 4, 0, 5]),
        (vxe_render::QueueOrder::Distance, [2, 3, 1, 4, 0, 5]),
    ] {
        let Some(mut q) = rig(
            row(&[0, 1, 2, 3, 4, 5]),
            &Setup {
                coarse: COARSE,
                near: 0,
                slice_ms: 1e-6,
                order: Some(order),
                ..Setup::at(ROW_EYE)
            },
        ) else {
            return;
        };
        let got = open_order(&mut q);
        println!("{order:?}: {got:?}");
        assert_eq!(got, want, "{order:?}");
    }
}

/// The three orders the blank-chunk tests run under: the score at angle weight 4 and refine
/// weight 2, the score with the class as a tier, and the distance.
fn three_orders() -> [vxe_render::QueueOrder; 3] {
    [
        score(4.0),
        vxe_render::QueueOrder::Score {
            angle: 4.0,
            refine: vxe_render::Refine::Tier,
        },
        vxe_render::QueueOrder::Distance,
    ]
}

/// The row of [`row`] with the chunks `absent` taken out of the world before it is mirrored:
/// no mirror, queue or store has seen them.
fn row_without(rocks: &[i64], absent: &[i32]) -> vxe_world::World {
    let world = row(rocks);
    for &x in absent {
        world.store().remove(ChunkCoord([x, 0, 0]));
    }
    world
}

impl Q {
    /// Chunk `x` of the row arrives as the streamer brings one: a rock block 16 voxels from its
    /// borders, inserted into the world and mirrored, with no edit box (no `invalidate`).
    fn arrive(&mut self, x: i32) {
        let source = row(&[i64::from(x)]);
        let c = ChunkCoord([x, 0, 0]);
        let chunk = source
            .store()
            .remove(c)
            .expect("the source holds the chunk");
        let chunk = chunk.read().expect("the chunk's lock").clone();
        self.world.store().insert(c, chunk);
        self.gpu
            .sync_now(&self.world, &self.device, &self.queue)
            .expect("the mirror fits");
    }

    /// Steps until `c` holds a coarse block and no job: its refinement pending.
    fn step_until_coarse(&mut self, c: ChunkCoord) {
        let mut steps = 0;
        while !(self.frame.store().is_coarse(c) && !self.q().jobs().contains(&c)) {
            self.step();
            steps += 1;
            assert!(
                steps < MAX_STEPS,
                "chunk {c:?}'s coarse block lands: block {:?}, coarse {}, tracks {:?}, mirrored {}",
                self.frame.store().block(c),
                self.frame.store().is_coarse(c),
                self.q().tracks(c),
                self.gpu.is_mirrored(c)
            );
        }
    }
}

/// The row with rock in chunks 0, 1 and 2, chunk 3 blank and chunks 4 and 5 not in the world,
/// camera in chunk 2 looking +x, `near` 0, coarse pass on: chunk 4 arrives (inserted and
/// mirrored, no edit box) and is stepped until its coarse block lands, so its refinement is
/// pending; then in one frame chunk 5 arrives the same way (a blank chunk's first pass pending)
/// and an edit fills chunk 3. Under all three
/// orders the edited chunk is a dig (ADR-0070's whole dig of a chunk that held no block) after
/// the edit's step, opens before chunk 4's refinement and chunk 5's first pass, and lands a
/// full block with no coarse one. About 20 s on lavapipe: soak; the gate keeps the score's
/// order alone (`a_chunk_an_edit_fills_is_a_dig_under_the_score`).
#[test]
#[ignore = "soak"]
fn a_chunk_an_edit_fills_is_a_dig_in_every_order() {
    for order in three_orders() {
        edit_fills_is_a_dig(order);
    }
}

/// `a_chunk_an_edit_fills_is_a_dig_in_every_order` under the fly-through's order, the score at
/// angle weight 4, alone: the edited chunk's claim in the gate.
#[test]
fn a_chunk_an_edit_fills_is_a_dig_under_the_score() {
    edit_fills_is_a_dig(score(4.0));
}

/// The rig and the claims of `a_chunk_an_edit_fills_is_a_dig_in_every_order` under `order`.
fn edit_fills_is_a_dig(order: vxe_render::QueueOrder) {
    {
        let Some(mut q) = rig(
            row_without(&[0, 1, 2], &[4, 5]),
            &Setup {
                coarse: COARSE,
                near: 0,
                slice_ms: 1e-6,
                order: Some(order),
                ..Setup::at(ROW_EYE)
            },
        ) else {
            return;
        };
        q.settle();
        let (three, four, five) = (
            ChunkCoord([3, 0, 0]),
            ChunkCoord([4, 0, 0]),
            ChunkCoord([5, 0, 0]),
        );
        q.arrive(4);
        q.step_until_coarse(four);
        q.arrive(5);
        q.edit(&box_op([208, 16, 16], [240, 48, 48], Mode::Fill(ROCK)));
        q.step();
        let tracks = q.q().tracks(three);
        assert!(
            tracks.contains(&"digs") || tracks.contains(&"dig_open"),
            "{order:?}: the edited chunk is a dig after the edit's step: {tracks:?}"
        );
        let mut seen: Vec<i32> = Vec::new();
        let mut coarse_three = false;
        for _ in 0..MAX_STEPS {
            for c in q.q().jobs() {
                if [three, four, five].contains(&c) && !seen.contains(&c.0[0]) {
                    seen.push(c.0[0]);
                }
            }
            coarse_three |= q.frame.store().is_coarse(three);
            if seen.len() == 3 && q.frame.store().block(three).is_some() {
                break;
            }
            q.step();
        }
        q.settle();
        coarse_three |= q.frame.store().is_coarse(three);
        println!("{order:?}: opened {seen:?}");
        assert_eq!(seen.first(), Some(&3), "{order:?}: {seen:?}");
        assert_eq!(seen.len(), 3, "{order:?}: all three open: {seen:?}");
        assert!(
            q.frame.store().block(three).is_some(),
            "{order:?}: the dig's block lands"
        );
        assert!(!coarse_three, "{order:?}: the dig lands no coarse block");
    }
}

/// The row with chunk 3 holding a coarse block and chunk 5 none, the rest full, camera in chunk
/// 2 looking +x, `near` 0, refine weight 2: under the score chunk 3's refinement (1 x 2 = 2)
/// opens before chunk 5's first pass (3); with the class as a tier, and under the distance
/// order, chunk 5 opens first. Chunks 3 and 5 start out of the world and arrive with their rock
/// as the streamer brings a chunk (inserted and mirrored, no `invalidate`): 3 first, stepped
/// until its coarse block lands, then 5. About 19 s on lavapipe: soak.
#[test]
#[ignore = "soak"]
fn a_blank_chunk_that_arrives_waits_behind_a_nearer_refinement_only_under_the_score() {
    for (order, first) in three_orders().into_iter().zip([3, 5, 5]) {
        let Some(mut q) = rig(
            row_without(&[0, 1, 2, 4], &[3, 5]),
            &Setup {
                coarse: COARSE,
                near: 0,
                slice_ms: 1e-6,
                order: Some(order),
                ..Setup::at(ROW_EYE)
            },
        ) else {
            return;
        };
        q.settle();
        q.arrive(3);
        q.step_until_coarse(ChunkCoord([3, 0, 0]));
        q.arrive(5);
        let mut seen: Vec<i32> = Vec::new();
        for _ in 0..MAX_STEPS {
            q.step();
            for c in q.q().jobs() {
                if (c.0[0] == 3 || c.0[0] == 5) && !seen.contains(&c.0[0]) {
                    seen.push(c.0[0]);
                }
            }
            if seen.len() == 2 {
                break;
            }
        }
        println!("{order:?}: {seen:?}");
        assert_eq!(seen.first(), Some(&first), "{order:?}: {seen:?}");
    }
}

/// The row at `near` 0 and angle weight 4, the camera turned to look -x once its own chunk
/// opens: of the chunks not yet opened at the turn, 1 and 0 (now ahead) open before 4 and 5.
/// Coarse off and 4096 samples a brick direction, so a chunk's job (about 3.9 x 10^5 samples)
/// fills the loop's two slices of 16 384 alone and the chunks open one at a time.
#[test]
fn a_turn_resorts_the_queue() {
    let Some(mut q) = rig(
        row(&[0, 1, 2, 3, 4, 5]),
        &Setup {
            near: 0,
            slice_ms: 1e-6,
            order: Some(score(4.0)),
            counts: BakeSettings {
                brick_samples: 4096,
                ..counts()
            },
            ..Setup::at(ROW_EYE)
        },
    ) else {
        return;
    };
    let mut before: Vec<i32> = Vec::new();
    while !before.contains(&2) {
        q.step();
        before.extend(q.q().jobs().iter().map(|c| c.0[0]));
        assert!(before.len() < 64, "chunk 2 opens");
    }
    q.camera.yaw = std::f32::consts::PI;
    let mut after: Vec<i32> = Vec::new();
    for _ in 0..MAX_STEPS {
        q.step();
        for c in q.q().jobs() {
            let x = c.0[0];
            if !before.contains(&x) && !after.contains(&x) {
                after.push(x);
            }
        }
        if q.q().empty() {
            break;
        }
    }
    println!("opened before the turn {before:?}, after {after:?}");
    let at = |x: i32| after.iter().position(|&v| v == x);
    for behind in [4, 5] {
        for ahead in [1, 0] {
            if let Some(b) = at(behind) {
                let a = at(ahead).expect("a chunk ahead opens");
                assert!(a < b, "{ahead} before {behind}: {after:?}");
            }
        }
    }
    assert!(at(1).is_some() && at(0).is_some(), "{before:?} {after:?}");
}

/// Eight chunks in a row, an 8^3 rock cube on one brick of each, its top layer ore (6 brick
/// directions, 24 576 samples a chunk at 4096 a brick direction), and a rig on it at `near` 0,
/// coarse off, the smallest slice (a step draws the cap's floor, 16 384 samples, and an opening
/// admits jobs while under two caps are left), the camera in chunk 0 looking +x.
fn cubes_rig() -> Option<Q> {
    let world = world_of([8, 1, 1], AIR);
    for x in 0..8 {
        fill(&world, [x * 64 + 24, 24, 24], [x * 64 + 32, 32, 31], ROCK);
        fill(&world, [x * 64 + 24, 24, 31], [x * 64 + 32, 32, 32], ORE);
    }
    rig(
        world,
        &Setup {
            near: 0,
            slice_ms: 1e-6,
            counts: BakeSettings {
                brick_samples: 4096,
                ..counts()
            },
            ..Setup::at([32.5, 32.5, 50.5])
        },
    )
}

/// The cubes' row: the camera steps in chunk 0 until chunk 2's job is in the loop (with chunk
/// 1's, open and partly drawn), then moves to chunk 5 looking +x. At the next step the loop
/// holds no job of chunks 0 to 4, the withdrawn count and samples are above 0, each withdrawn
/// chunk holds no block it did not hold before and nothing of it is left but its listing, and
/// every pool word is accounted for. Run to empty, every block is bit-identical to a one-shot
/// bake.
#[test]
fn a_passed_chunk_is_withdrawn() {
    let Some(mut q) = cubes_rig() else {
        return;
    };
    let steps = q.step_until_job(2);
    let held: Vec<[i32; 3]> = q.q().jobs().iter().map(|c| c.0).collect();
    let had: Vec<bool> = (0..5)
        .map(|x| q.frame.store().block(ChunkCoord([x, 0, 0])).is_some())
        .collect();
    println!("chunk 2 opened after {steps} steps; jobs {held:?}, blocks {had:?}");
    assert!(
        held.contains(&[1, 0, 0]) && held.contains(&[2, 0, 0]),
        "{held:?}"
    );
    q.camera.position.x = 5.0 * 64.0 + 32.5;
    let st = q.step();
    let jobs: Vec<i32> = q.q().jobs().iter().map(|c| c.0[0]).collect();
    println!("after the move: jobs {jobs:?}, {st:?}");
    assert!(jobs.iter().all(|x| !(0..5).contains(x)), "{jobs:?}");
    assert!(st.withdrawn > 0, "{st:?}");
    assert!(st.withdrawn_samples > 0, "{st:?}");
    for c in &held {
        if (0..5).contains(&c[0]) {
            let block = q.frame.store().block(ChunkCoord(*c));
            assert!(
                block.is_none() || had[c[0] as usize],
                "{c:?} holds a block it did not hold"
            );
            let tracks = q.q().tracks(ChunkCoord(*c));
            assert!(
                tracks.iter().all(|t| *t == "listed"),
                "{c:?} left behind {tracks:?}"
            );
        }
    }
    q.assert_pool_accounted();
    q.settle();
    let got = q.blocks();
    assert_eq!(got.len(), 8, "every chunk holds a block");
    let fresh = q.fresh();
    for (c, b) in &got {
        assert_eq!(Some(b), fresh.get(c), "{c:?} against a one-shot bake");
    }
    q.assert_pool_accounted();
}

/// The cubes' row as in [`a_passed_chunk_is_withdrawn`], but the step after the move has a
/// sample count, as the pre-roll's steps do: it withdraws nothing, and chunk 1's and 2's jobs
/// that did not end in it are still in the loop.
#[test]
fn a_pre_roll_step_withdraws_nothing() {
    let Some(mut q) = cubes_rig() else {
        return;
    };
    q.step_until_job(2);
    let held: Vec<[i32; 3]> = q.q().jobs().iter().map(|c| c.0).collect();
    assert!(
        held.contains(&[1, 0, 0]) && held.contains(&[2, 0, 0]),
        "{held:?}"
    );
    q.camera.position.x = 5.0 * 64.0 + 32.5;
    q.samples = Some(16_384);
    let st = q.step();
    let jobs: Vec<[i32; 3]> = q.q().jobs().iter().map(|c| c.0).collect();
    println!("after the move: jobs {jobs:?}, {st:?}");
    assert_eq!(st.withdrawn, 0, "{st:?}");
    for c in &held {
        assert!(
            jobs.contains(c) || q.frame.store().block(ChunkCoord(*c)).is_some(),
            "{c:?} left the loop with no block"
        );
    }
    q.assert_pool_accounted();
}

/// The cubes' row with no extra slice for digs and a carve in chunk 1 once its block lands,
/// before the move to chunk 5: the dig's job stays in the loop after the move, and the dig's
/// chunk settles to a one-shot bake of the edited world.
#[test]
fn a_dig_is_never_withdrawn() {
    let Some(mut q) = cubes_rig() else {
        return;
    };
    // No extra slice for digs: the dig's job draws the cap's floor a step, so it is still in
    // the loop at the step after the move.
    q.frame.bake_queue_mut().expect("the queue started").edit_ms = 0.0;
    let one = ChunkCoord([1, 0, 0]);
    let mut n = 0;
    while q.frame.store().block(one).is_none() {
        q.step();
        n += 1;
        assert!(n < MAX_STEPS, "chunk 1 is baked");
    }
    q.edit(&box_op([88, 24, 24], [92, 28, 32], Mode::Carve));
    q.step();
    assert!(
        q.q().tracks(one).contains(&"dig_open"),
        "{:?}",
        q.q().tracks(one)
    );
    q.camera.position.x = 5.0 * 64.0 + 32.5;
    let st = q.step();
    println!("after the move: jobs {:?}, {st:?}", q.q().jobs());
    assert!(q.q().jobs().contains(&one), "the dig's job stays");
    assert!(q.q().tracks(one).contains(&"dig_open"));
    q.settle();
    assert_eq!(
        q.blocks().get(&one.0),
        q.fresh().get(&one.0),
        "the dig's chunk against a one-shot bake of the edited world"
    );
}

/// A 4 x 4 chunk floor of 32^3 rock blocks at 4096 samples a brick direction (a job of about
/// 3.9 x 10^5 samples, many steps), the camera in chunk [0, 0, 0] yawing by plus and minus 5
/// degrees a step about the view cone's edge on +x: the chunks along +x leave the view cone
/// and come back, and stay in the keep cone. Over 64 steps nothing is withdrawn, while a job's
/// chunk lies out of the view cone at some step.
#[test]
fn the_edge_of_the_view_does_not_churn() {
    let world = world_of([4, 4, 1], AIR);
    for x in 0..4 {
        for y in 0..4 {
            fill(
                &world,
                [x * 64 + 16, y * 64 + 16, 16],
                [x * 64 + 48, y * 64 + 48, 48],
                ROCK,
            );
        }
    }
    fill(&world, [20, 20, 48], [30, 30, 50], ORE);
    let Some(mut q) = rig(
        world,
        &Setup {
            near: 0,
            slice_ms: 1e-6,
            counts: BakeSettings {
                brick_samples: 4096,
                ..counts()
            },
            ..Setup::at([32.5, 32.5, 32.5])
        },
    ) else {
        return;
    };
    let edge = vxe_render::internals::view_half_angle(70f32.to_radians(), 16.0 / 9.0)
        + vxe_render::internals::QUEUE_VIEW_MARGIN_DEG.to_radians();
    let mut out_of_view = 0;
    for n in 0..64 {
        let sign = if n % 2 == 0 { 1.0 } else { -1.0 };
        q.camera.yaw = edge + sign * 5f32.to_radians();
        let st = q.step();
        assert_eq!(st.withdrawn, 0, "step {n}: {st:?}");
        let forward = q.camera.basis().0.to_array();
        out_of_view += q
            .q()
            .jobs()
            .iter()
            .filter(|c| {
                vxe_render::internals::angle_term(
                    c.0,
                    forward,
                    edge - vxe_render::internals::QUEUE_VIEW_MARGIN_DEG.to_radians(),
                    0,
                ) > 0.0
            })
            .count();
    }
    println!("job-steps out of the view cone: {out_of_view}");
    assert!(out_of_view > 0, "a job's chunk left the view cone");
}

/// The rock cubes of [`still_rig`]: chunks 0, 1, 2, 6 and 7 of an eight-chunk row.
const STILL_ROCKS: [i32; 5] = [0, 1, 2, 6, 7];

/// An eight-chunk row with a 32^3 rock cube in chunks 0, 1, 2, 6 and 7 (3 to 5 air), the camera
/// in chunk 2 looking +x, `near` 0, the coarse pass on, the default order (angle weight 4,
/// class as a tier), 1024 samples a brick direction and the smallest slice. Once the coarse
/// passes land, the order's first refinement is chunk 1 behind the camera (score 1 x 3.53)
/// ahead of chunk 6 in the view (4): a job outside the near shell and the keep cone while a
/// candidate in the view cone is ready, the patch's state at rest. Steps until chunk 1's
/// refinement is in the loop and chunk 6 holds its coarse block outside a job, and asserts that
/// state.
fn still_rig() -> Option<Q> {
    let world = world_of([8, 1, 1], AIR);
    for x in STILL_ROCKS.map(i64::from) {
        fill(&world, [x * 64 + 16, 16, 16], [x * 64 + 48, 48, 48], ROCK);
    }
    fill(&world, [140, 28, 48], [150, 36, 50], ORE);
    let mut q = rig(
        world,
        &Setup {
            coarse: COARSE,
            near: 0,
            slice_ms: 1e-6,
            counts: BakeSettings {
                brick_samples: 1024,
                ..counts()
            },
            ..Setup::at(ROW_EYE)
        },
    )?;
    let (one, six) = (ChunkCoord([1, 0, 0]), ChunkCoord([6, 0, 0]));
    let mut n = 0;
    loop {
        let st = q.step_with(false);
        assert_eq!(st.withdrawn, 0, "step {n}: {st:?}");
        let jobs = q.q().jobs();
        let store = q.frame.store();
        if jobs.contains(&one)
            && store.is_coarse(one)
            && store.is_coarse(six)
            && !jobs.contains(&six)
        {
            break;
        }
        n += 1;
        assert!(n < MAX_STEPS, "chunk 1's refinement opens: jobs {jobs:?}");
    }
    let half = {
        let [w, h] = q.frame.output_size();
        vxe_render::internals::view_half_angle(q.camera.vfov, w as f32 / h as f32)
    };
    let keep = half + vxe_render::internals::QUEUE_VIEW_MARGIN_DEG.to_radians();
    let forward = q.camera.basis().0.to_array();
    assert!(
        !vxe_render::internals::in_near_shell([-1, 0, 0], 0)
            && vxe_render::internals::angle_term([-1, 0, 0], forward, keep, 0) > 0.0,
        "chunk 1 lies outside the near shell and the keep cone"
    );
    assert_eq!(
        vxe_render::internals::angle_term([4, 0, 0], forward, half, 0),
        0.0,
        "chunk 6 lies in the view cone"
    );
    assert!(
        q.gpu.is_mirrored(six) && !q.gpu.is_lod(six),
        "chunk 6 is resident full: its reach is ready in the bounded row"
    );
    Some(q)
}

/// [`still_rig`]'s state, the camera still: every step withdraws nothing. Within `N` steps,
/// `N` the refinements' samples (at most four chunks of 56 surface bricks, three directions
/// each, at 1024 samples) over the step's cap floor (16 384), plus 8 steps a job for the
/// read-back's lag and the quiet box, every chunk of the row holds a full block; the 100 steps
/// after push no job and scan nothing. A second rig moved one chunk along +x from that state
/// withdraws chunk 1's job at its next step: the flight's withdrawal is kept.
#[test]
fn a_still_camera_drains_and_withdraws_nothing() {
    let Some(mut q) = still_rig() else {
        return;
    };
    let refinements = (STILL_ROCKS.len() - 1) as u64;
    let n = (refinements * 56 * 3 * 1024).div_ceil(16_384) + 8 * refinements;
    let full = |q: &Q| {
        STILL_ROCKS.iter().all(|&x| {
            let c = ChunkCoord([x, 0, 0]);
            q.frame.store().block(c).is_some() && !q.frame.store().is_coarse(c)
        })
    };
    let mut steps = 0;
    while !(full(&q) && q.q().empty()) {
        let st = q.step_with(false);
        assert_eq!(st.withdrawn, 0, "still step {steps}: {st:?}");
        steps += 1;
        assert!(
            steps <= n,
            "drained within {n} steps: jobs {:?}, {st:?}",
            q.q().jobs()
        );
    }
    println!("drained after {steps} still steps of at most {n}");
    for k in 0..100 {
        let st = q.step_with(false);
        assert_eq!(
            (st.withdrawn, st.jobs, st.scanned),
            (0, 0, 0),
            "drained step {k}: {st:?}"
        );
        assert!(q.q().jobs().is_empty(), "drained step {k} pushes no job");
    }
    let Some(mut q) = still_rig() else {
        return;
    };
    q.camera.position.x += 64.0;
    let st = q.step_with(false);
    println!("after the move: jobs {:?}, {st:?}", q.q().jobs());
    assert!(st.withdrawn > 0, "{st:?}");
    assert!(
        !q.q().jobs().contains(&ChunkCoord([1, 0, 0])),
        "chunk 1's job is withdrawn"
    );
}

/// Plan 0096 Phase 5 bar 4: the kept order walked from the camera outward changes when a chunk
/// is lit, not what it holds. A 5 x 5 x 5 chunk world (a rock floor, four pillars through it and
/// an ore slab on two of them) settled through the live queue from its centre while the camera
/// turns a quarter every 8 steps, so the walks restart, is bit-identical block by block to a
/// one-shot bake of the same world at the same counts, whose order is the job list's.
#[test]
fn images_are_unchanged_by_the_incremental_order() {
    let world = world_of([5, 5, 5], AIR);
    fill(&world, [0, 0, 0], [319, 319, 70], ROCK);
    for (x, y) in [(60, 60), (60, 250), (250, 60), (250, 250)] {
        fill(&world, [x, y, 71], [x + 12, y + 12, 260], ROCK);
    }
    fill(&world, [60, 60, 200], [262, 72, 212], ORE);
    let eye = [160.5, 160.5, 140.5];
    let Some(mut q) = rig(world, &Setup::at(eye)) else {
        return;
    };
    let mut scored = 0;
    for n in 0..MAX_STEPS {
        q.camera.yaw = ((n / 8) % 4) as f32 * std::f32::consts::FRAC_PI_2;
        scored += q.step().visited;
        if n > 32 && q.q().empty() {
            break;
        }
    }
    assert!(q.q().empty(), "the queue settles: {:?}", q.q().stats());
    let got = q.blocks();
    println!("{} blocks, {scored} candidates scored", got.len());
    assert!(
        got.len() >= 25,
        "the floor's chunks hold blocks: {}",
        got.len()
    );
    let fresh = q.fresh();
    assert_eq!(got.len(), fresh.len());
    for (c, b) in &got {
        assert_eq!(Some(b), fresh.get(c), "{c:?} against a one-shot bake");
    }
}

/// ADR-0080 rule 3 through the frame's own step, with no `request_census`: sixteen chunks in a
/// row, a 16^3 rock cube over eight bricks of each (24 brick directions, 98 304 samples a chunk
/// at 4096 a brick direction), the camera in chunk 8 looking +x, `near` 0, coarse off, the
/// smallest budget and a drain of 0.2 ms over the line's 0.1 (both under the cap's floor of
/// 16 384 samples, so a chunk takes about six steps). The queue drains the first view (the
/// slice the drain's while chunks 8 to 15 wait) and reaches the line with chunks behind the
/// camera still to bake; the camera then turns to look -x, and within `WAITING_EVERY` (8) steps
/// the step reads `draining` and the drain's slice, from the queue's own read of its in-view
/// set.
#[test]
fn a_turn_onto_unbaked_chunks_drains_without_a_census_request() {
    let world = world_of([16, 1, 1], AIR);
    for x in 0..16 {
        fill(&world, [x * 64 + 16, 16, 16], [x * 64 + 32, 32, 32], ROCK);
    }
    let Some(mut q) = rig(
        world,
        &Setup {
            near: 0,
            slice_ms: 1e-6,
            counts: BakeSettings {
                brick_samples: 4096,
                ..counts()
            },
            ..Setup::at([8.0 * 64.0 + 32.5, 32.5, 50.5])
        },
    ) else {
        return;
    };
    let drain = vxe_render::Drain {
        drain_ms: 0.2,
        line_ms: 0.1,
    };
    q.frame.bake_queue_mut().expect("the queue started").drain = Some(drain);
    let unbaked_behind = |q: &Q| {
        (0..8)
            .filter(|&x| q.target([x, 0, 0]).is_none())
            .collect::<Vec<i32>>()
    };
    let (mut drained, mut landed) = (false, None);
    for n in 0..MAX_STEPS {
        let st = q.step();
        if st.draining {
            assert_eq!(st.slice_ms, drain.drain_ms, "step {n}: {st:?}");
            drained = true;
        } else if drained {
            assert_eq!(st.slice_ms, drain.line_ms, "step {n}: {st:?}");
            landed = Some(n);
            break;
        }
    }
    let landed = landed.expect("the first view drains and lands");
    let behind = unbaked_behind(&q);
    println!("the first view landed at step {landed}; behind still unbaked {behind:?}");
    assert!(
        !behind.is_empty(),
        "chunks behind the camera wait at the turn"
    );
    for x in 8..16 {
        assert!(q.target([x, 0, 0]).is_some(), "chunk {x} in the first view");
    }
    q.camera.yaw = std::f32::consts::PI;
    let mut at = None;
    for k in 1..=8 {
        let st = q.step();
        if st.draining {
            assert_eq!(st.slice_ms, drain.drain_ms, "{st:?}");
            assert!(st.drain_waiting > 0, "{st:?}");
            at = Some(k);
            break;
        }
    }
    println!("draining {at:?} steps after the turn");
    assert!(
        at.is_some(),
        "the turn drains within 8 steps: {:?}",
        q.q().stats()
    );
}

/// A block's brick records by directory slot, at the brick cell (`BRICK_WORDS` words each, the
/// directory word's run end above bit 11).
fn slot_records(block: &[u32]) -> std::collections::BTreeMap<usize, Vec<u32>> {
    use vxe_render::internals::{BRICK_WORDS, SUMMARY_AT};
    block[..SUMMARY_AT as usize]
        .iter()
        .enumerate()
        .filter(|(_, d)| **d != 0)
        .map(|(slot, &d)| {
            let end = (d >> 11) as usize;
            (slot, block[end - BRICK_WORDS..end].to_vec())
        })
        .collect()
}

/// Whether the brick with lowest voxel `b` lies within a voxel of the box `lo ..= hi`.
fn in_ring1(b: [i32; 3], lo: [i64; 3], hi: [i64; 3]) -> bool {
    (0..3).all(|k| i64::from(b[k]) + 7 >= lo[k] - 1 && i64::from(b[k]) <= hi[k] + 1)
}

/// Plan 0098 Phase 16: a dig's faces jump the queue. Forty-seven chunks of an 8^3 cube each and
/// a 64 x 64 x 8 slab in chunk 0 (the camera's), at 4096 samples a brick direction, no coarse
/// pass: stepped at a pre-roll's sample count until chunk 0 holds a block, every table slot holds
/// an open job that is not a dig's and jobs pend behind them; then sliced at the smallest slice.
/// A carve in the slab: the step that reads it plans the dig's ring-1 job (every brick it draws
/// within a voxel of the box) first in the open list, its rounds in flight; the chunk's ring-2
/// bricks open as a second job only once that job's block is placed. A second carve in the slab
/// the step after that placement: its step holds no stale job for the chunk (the first dig's
/// ring-2 job is withdrawn, not run to its end), plans the new ring-1 job first, and the first
/// dig's ring-1 records the second box does not touch keep their landed words (but the ring's
/// stale flag). Settled, chunk 0 is bit-identical to a one-shot bake of the edited world.
#[test]
fn a_dig_jumps_every_open_job_and_withdraws_its_stale_one() {
    let world = world_of([8, 6, 1], AIR);
    for x in 0..8i64 {
        for y in 0..6i64 {
            if (x, y) != (0, 0) {
                fill(
                    &world,
                    [x * 64 + 24, y * 64 + 24, 24],
                    [x * 64 + 32, y * 64 + 32, 32],
                    ROCK,
                );
            }
        }
    }
    fill(&world, [0, 0, 24], [64, 64, 32], ROCK);
    let Some(mut q) = rig(
        world,
        &Setup {
            near: 0,
            slice_ms: 1e-6,
            counts: BakeSettings {
                brick_samples: 4096,
                ..counts()
            },
            ..Setup::at([32.5, 32.5, 50.5])
        },
    ) else {
        return;
    };
    let zero = ChunkCoord([0, 0, 0]);
    q.samples = Some(1 << 20);
    let mut n = 0;
    loop {
        q.step();
        n += 1;
        let t = q.q().dig_trace();
        if q.frame.store().block(zero).is_some() && t.open == 16 && t.pending > 0 {
            break;
        }
        assert!(n < MAX_STEPS, "the slots fill: {t:?}");
    }
    println!("slots full after {n} steps");
    q.samples = None;
    let carve = |q: &mut Q, lo: [i64; 3], hi: [i64; 3]| {
        q.edit(&box_op(lo, [hi[0] + 1, hi[1] + 1, hi[2] + 1], Mode::Carve));
        let w0 = q.q().stats().dig_withdrawn;
        q.step();
        let t = q.q().dig_trace();
        println!("after the carve: {t:?}");
        let first = t
            .jobs
            .iter()
            .find(|j| j.open && j.at == 0)
            .expect("a watched job is first in the open list");
        assert_eq!(first.coord, zero.0, "the dig's job is planned first: {t:?}");
        assert!(first.dig && !first.stale, "{first:?}");
        assert!(first.totals()[1] > 0, "its rounds are in flight: {first:?}");
        assert!(
            first.bricks.iter().all(|b| in_ring1(b.0, lo, hi)),
            "the first job draws ring 1 alone: {first:?}"
        );
        assert!(
            !t.jobs.iter().any(|j| j.coord == zero.0 && j.stale),
            "no stale job for the chunk: {t:?}"
        );
        q.q().stats().dig_withdrawn - w0
    };
    let (lo1, hi1) = ([8, 8, 28], [11, 11, 31]);
    carve(&mut q, lo1, hi1);
    // Until ring 1's block is placed no other job of the chunk runs; then ring 2's opens.
    let mut ring2 = false;
    for _ in 0..MAX_STEPS {
        q.step();
        let jobs: Vec<_> = q
            .q()
            .dig_trace()
            .jobs
            .into_iter()
            .filter(|j| j.coord == zero.0)
            .collect();
        match jobs.first() {
            Some(j) if j.bricks.iter().all(|b| in_ring1(b.0, lo1, hi1)) => {}
            Some(j) => {
                assert!(j.dig, "{j:?}");
                ring2 = true;
                break;
            }
            None => {}
        }
    }
    assert!(ring2, "ring 2's job opens after ring 1's is placed");
    let landed = slot_records(q.frame.store().block_words(zero).expect("chunk 0's block"));
    let (lo2, hi2) = ([40, 40, 28], [43, 43, 31]);
    let withdrawn = carve(&mut q, lo2, hi2);
    assert!(withdrawn > 0, "the first dig's ring-2 job is withdrawn");
    let now = slot_records(q.frame.store().block_words(zero).expect("chunk 0's block"));
    let mut kept = 0;
    for (slot, r) in &landed {
        let b = [slot % 8, slot / 8 % 8, slot / 64].map(|v| v as i32 * 8);
        if !in_ring1(b, lo1, hi1) || in_ring1(b, lo2, hi2) {
            continue;
        }
        let mut a = now
            .get(slot)
            .expect("the landed brick keeps its record")
            .clone();
        let mut r = r.clone();
        // The ring's stale flag (the second sky word's bit 17) is the edit's mark.
        a[8] &= !(1 << 17);
        r[8] &= !(1 << 17);
        assert_eq!(a, r, "ring-1 brick {slot} keeps its landed record");
        kept += 1;
    }
    assert!(
        kept > 0,
        "some first ring-1 brick lies outside the second box"
    );
    q.settle();
    assert_eq!(
        q.blocks().get(&zero.0),
        q.fresh().get(&zero.0),
        "chunk 0 against a one-shot bake of the edited world"
    );
}
