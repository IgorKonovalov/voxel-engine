//! The record by rule (ADR-0080 rule 1) through the live queue on the software adapter: the
//! zero record of a chunk buried under the column map with no emitter in reach, a chunk with no
//! exposed face, the stream workers' no-face verdict, and the re-listing of a zeroed chunk by a
//! dig to the sky.
//!
//! Every world here is the buried rig's 7 x 7 x 7 chunks at test counts; the rigs are
//! `support::queue`'s.

mod support;

use flythrough::cave::ROCK;
use vxe_world::{ChunkCoord, Mode};

/// A sky over the buried rig whose sun is at elevation `el` degrees (the moon under the
/// horizon).
fn sun_sky(el: f32) -> vxe_render::sky::Sky {
    let e = el.to_radians();
    vxe_render::sky::Sky::new(
        vxe_render::sky::Body::sun([0.3 * e.cos(), -(0.91f32).sqrt() * e.cos(), e.sin()]),
        vxe_render::sky::Body::moon([0.0, 0.0, -1.0]),
    )
}

/// A 7 x 7 x 7-chunk world of rock under air from z 320 (a flat surface), with a cave of air
/// over chunks (2..=4, 3, 1) under it, no emitter; the live queue at light radius 2 over a
/// sun at elevation 40, the camera above the surface; `rule` sets the queue's zero rule.
fn buried_rig(rule: bool) -> Option<support::queue::Q> {
    use support::queue::{Setup, box_op, rig, world_of};
    let world = world_of([7, 7, 7], ROCK);
    for (min, max) in [
        ([0, 0, 320], [448, 448, 448]),
        ([140, 200, 80], [300, 250, 120]),
    ] {
        world
            .apply(&box_op(min, max, Mode::Carve))
            .expect("the box is resident");
    }
    let mut q = rig(world, &Setup::at([224.0, 224.0, 400.0]))?;
    q.frame.set_sky(Some(sun_sky(40.0)));
    q.frame
        .bake_queue_mut()
        .expect("the queue started")
        .zero_rule = rule;
    Some(q)
}

/// The cave's chunks, under the surface with their reach boxes (top z 255) under every
/// column's top (319).
const CAVE: [[i32; 3]; 3] = [[2, 3, 1], [3, 3, 1], [4, 3, 1]];

/// Plan 0096 Phase 2 bar 1 (ADR-0080): the cave's chunks, buried under the column map with no
/// emitter in reach, take the rule's zero record with no GPU job, and those records equal byte
/// for byte the records the same queue writes with the rule off (each a GPU job at the full
/// count), under a sun (the sky's and the key light's flags set).
#[test]
fn a_buried_chunk_with_no_emitter_in_reach_gets_the_bakes_bytes_without_a_job() {
    let Some(mut ruled) = buried_rig(true) else {
        return;
    };
    ruled.settle();
    let Some(mut baked) = buried_rig(false) else {
        return;
    };
    baked.settle();
    for c in CAVE {
        assert!(ruled.q().zeroed(ChunkCoord(c)), "{c:?} took no zero record");
        assert!(!baked.q().zeroed(ChunkCoord(c)));
        let (a, b) = (ruled.target(c), baked.target(c));
        assert!(a.is_some(), "{c:?} holds no block");
        assert_eq!(a, b, "{c:?}: the rule's record against the GPU's");
    }
    assert!(ruled.q().stats().zeroed >= CAVE.len() as u64);
    assert_eq!(baked.q().stats().zeroed, 0);
}

/// Plan 0096 Phase 2 bar 4: a uniform rock chunk inside the rock (5, 5, 2), every neighbour
/// rock, and a uniform air chunk over the surface (3, 3, 6) hold no record: the rock is marked faceless, and the store
/// holds no block of either. (A lavapipe test: the store needs a device.)
#[test]
fn a_chunk_with_no_exposed_face_holds_no_record() {
    let Some(mut q) = buried_rig(true) else {
        return;
    };
    q.settle();
    let (rock, air) = ([5, 5, 2], [3, 3, 6]);
    assert!(q.target(rock).is_none(), "the rock chunk's record");
    assert!(q.target(air).is_none(), "the air chunk's record");
    assert!(
        q.q().tracks(ChunkCoord(rock)).contains(&"faceless"),
        "{:?}",
        q.q().tracks(ChunkCoord(rock))
    );
    assert!(!q.q().zeroed(ChunkCoord(rock)));
}

/// Plan 0096 Phase 2 as amended: the stream workers' no-face verdict. Every chunk of the buried
/// rig's world goes through the workers' hook ([`vxe_render::internals::FacePrep`]) before the
/// queue's first step, in slot order, so a chunk's neighbours arrive before and after it. The
/// rock chunk (5, 5, 2), all six neighbours rock, is sealed by the verdicts and never listed
/// (no `faceless` mark, no record); the corner chunk (0, 0, 0), whose three outer neighbours
/// never stream, reads open there and is listed (the listing finds no face: absent is rock);
/// a cave chunk is neither. A carve into the sealed chunk clears its verdict and it takes a
/// record.
#[test]
fn the_workers_seal_a_chunk_with_no_face_and_an_edit_opens_it() {
    use vxe_world::ChunkPrep;
    let Some(mut q) = buried_rig(true) else {
        return;
    };
    let prep = vxe_render::internals::FacePrep::new(std::sync::Arc::new(
        vxe_render::internals::face_classes(q.world.materials()),
    ));
    let mut results = Vec::new();
    for z in 0..7 {
        for y in 0..7 {
            for x in 0..7 {
                let c = ChunkCoord([x, y, z]);
                let chunk = q.world.store().get(c).expect("resident");
                let chunk = chunk.read().expect("unpoisoned");
                results.push((c, prep.prepare(c, &chunk)));
            }
        }
    }
    q.frame.take_prepared(&results, &q.world);
    q.settle();
    let (rock, corner, cave) = ([5, 5, 2], [0, 0, 0], CAVE[1]);
    let tracks = |q: &support::queue::Q, c| q.q().tracks(ChunkCoord(c));
    assert_eq!(tracks(&q, rock), vec!["sealed"], "the rock chunk");
    assert!(q.target(rock).is_none());
    assert_eq!(tracks(&q, corner), vec!["faceless"], "the corner chunk");
    assert!(!tracks(&q, cave).contains(&"sealed"));
    assert!(q.target(cave).is_some(), "the cave's record");
    // A carve into the rock chunk's middle: its verdict goes and the dig bakes it.
    q.edit(&support::queue::box_op(
        [340, 340, 150],
        [350, 350, 160],
        Mode::Carve,
    ));
    q.settle();
    assert!(
        !tracks(&q, rock).contains(&"sealed"),
        "{:?}",
        tracks(&q, rock)
    );
    assert!(q.target(rock).is_some(), "the dug chunk's record");
}

/// ADR-0080 rule 1's re-listing, through the queue's steps: the buried rig with the rule on,
/// stepped until the step whose opening lists the last of the cave's chunks by the rule (the
/// rule read the column tops over their footprints at that opening). In the very next step a
/// shaft carved from the cave's roof to the sky over chunk column (4, 3) lowers that column's
/// top under the cave's reach boxes: the step re-lists all three cave chunks
/// (`zero_relisted` 3), reading the column tops after the edit, not the opening's.
#[test]
fn a_dig_to_the_sky_the_step_after_the_rule_was_read_relists_the_cave() {
    let Some(mut q) = buried_rig(true) else {
        return;
    };
    let mut steps = 0;
    while !CAVE.iter().all(|&c| q.q().zeroed(ChunkCoord(c))) {
        q.step();
        steps += 1;
        assert!(steps < support::queue::MAX_STEPS, "the cave is zeroed");
    }
    assert_eq!(q.q().stats().zero_relisted, 0);
    println!("the cave's last zero record at step {steps}");
    q.edit(&support::queue::box_op(
        [260, 210, 110],
        [270, 220, 330],
        Mode::Carve,
    ));
    let st = q.step();
    assert_eq!(st.zero_relisted, 3, "{st:?}");
    for c in CAVE {
        assert!(!q.q().zeroed(ChunkCoord(c)), "{c:?} still zeroed");
    }
}
