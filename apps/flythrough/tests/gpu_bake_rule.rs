//! The record by rule (ADR-0080 rule 1) through the live queue on the software adapter: the
//! zero record of a chunk buried under the column map with no emitter in reach, a chunk with no
//! exposed face, the stream workers' no-face verdict, and the re-listing of a zeroed chunk by a
//! dig to the sky.
//!
//! Every world here is the buried rig's 7 x 7 x 7 chunks at test counts; the rigs are
//! `support::queue`'s.

mod support;

use flythrough::cave::ROCK;
use svet_world::{ChunkCoord, Mode};

/// A sky over the buried rig whose sun is at elevation `el` degrees (the moon under the
/// horizon).
fn sun_sky(el: f32) -> svet_render::sky::Sky {
    let e = el.to_radians();
    svet_render::sky::Sky::new(
        svet_render::sky::Body::sun([0.3 * e.cos(), -(0.91f32).sqrt() * e.cos(), e.sin()]),
        svet_render::sky::Body::moon([0.0, 0.0, -1.0]),
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
    q.frame.set_bake_queue(move |q| q.zero_rule = rule);
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
/// rig's world goes through the workers' hook ([`svet_render::internals::FacePrep`]) before the
/// queue's first step, in slot order, so a chunk's neighbours arrive before and after it. The
/// rock chunk (5, 5, 2), all six neighbours rock, is sealed by the verdicts and never listed
/// (no `faceless` mark, no record); the corner chunk (0, 0, 0), whose three outer neighbours
/// never stream, reads open there and is listed (the listing finds no face: absent is rock);
/// a cave chunk is neither. A carve into the sealed chunk clears its verdict and it takes a
/// record.
#[test]
fn the_workers_seal_a_chunk_with_no_face_and_an_edit_opens_it() {
    use svet_world::ChunkPrep;
    let Some(mut q) = buried_rig(true) else {
        return;
    };
    let prep = svet_render::internals::FacePrep::new(std::sync::Arc::new(
        svet_render::internals::face_classes(q.world.materials()),
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

mod reading {
    use std::collections::BTreeMap;

    use super::*;

    /// The chunk whose floor the reading prints: wholly inside the tall cave's footprint, its
    /// air from z 80, the rock floor's `+z` faces on voxels z 79.
    const FLOOR: [i32; 3] = [3, 4, 1];

    /// The 7 x 7 x 7-chunk world of rock under air from z 320, with a closed cave of air over
    /// x [190, 290), y [150, 322), z [80, 230) under a roof 90 voxels thick, no emitter; the
    /// live queue at light radius 2 (a key ray's cap, `job.radius`, 128 voxels) over a sun at
    /// elevation 40. From [`FLOOR`]'s floor a key ray toward the sun rises 82 voxels and runs
    /// 93 along -y and 29 along +x within its cap, so every one ends in the cave's air, under
    /// the roof and under every column's top (319). `rule` sets the queue's zero rule.
    fn tall_cave_rig(rule: bool) -> Option<support::queue::Q> {
        use support::queue::{Setup, box_op, rig, world_of};
        let world = world_of([7, 7, 7], ROCK);
        for (min, max) in [
            ([0, 0, 320], [448, 448, 448]),
            ([190, 150, 80], [290, 322, 230]),
        ] {
            world
                .apply(&box_op(min, max, Mode::Carve))
                .expect("the box is resident");
        }
        let mut q = rig(world, &Setup::at([224.0, 224.0, 400.0]))?;
        q.frame.set_sky(Some(sun_sky(40.0)));
        q.frame.set_bake_queue(move |q| q.zero_rule = rule);
        Some(q)
    }

    /// [`FLOOR`]'s `+z` faces in `block`: their count, and how many read each key visibility
    /// (`None` where the record holds no key words), with the records' raw key and sky words.
    fn floor_key_words(block: &[u32]) -> (usize, BTreeMap<Option<u8>, usize>, Vec<[u32; 4]>) {
        use svet_render::internals::{BRICK_WORDS, record_key_visibility};
        let mut faces = 0;
        let mut by_key = BTreeMap::new();
        let mut raw = Vec::new();
        for slot in 0..512usize {
            let dir = block[slot];
            if dir == 0 {
                continue;
            }
            let rec = &block[(dir >> 11) as usize - BRICK_WORDS..(dir >> 11) as usize];
            if (rec[6] >> (16 + 5)) & 1 == 0 {
                continue;
            }
            faces += 1;
            let key = record_key_visibility(rec, 5).map(|v| (v * 255.0).round() as u8);
            *by_key.entry(key).or_insert(0) += 1;
            raw.push([rec[9], rec[10], rec[7], rec[8]]);
        }
        (faces, by_key, raw)
    }

    /// Plan 0096 Phase 10c: a key ray that ends in air at its cap under a roof. The tall cave's
    /// floor chunk [`FLOOR`] is buried under the column map with no emitter in reach, so the
    /// rule writes its zero record; with the rule off a GPU job bakes it. Prints, for both
    /// records, the floor's `+z` face count, the faces by key visibility (u8, `None` with no
    /// key words) and the distinct raw words (key 9, key 10, sky 7, sky 8). A reading: it
    /// asserts only that both records exist.
    #[test]
    #[ignore = "reading"]
    fn a_key_ray_that_ends_in_air_under_a_roof() {
        let Some(mut ruled) = tall_cave_rig(true) else {
            return;
        };
        let ruled_steps = ruled.settle();
        let Some(mut baked) = tall_cave_rig(false) else {
            return;
        };
        let baked_steps = baked.settle();
        for (name, q, steps) in [("job", &baked, baked_steps), ("rule", &ruled, ruled_steps)] {
            let block = q.target(FLOOR).expect("the floor chunk holds a block");
            let (faces, by_key, raw) = floor_key_words(&block);
            let mut distinct: Vec<[u32; 4]> = raw.clone();
            distinct.sort_unstable();
            distinct.dedup();
            println!(
                "{name}: zeroed {} after {steps} steps; {faces} floor +z faces; by key u8 \
                 {by_key:?}; distinct raw words [key9, key10, sky7, sky8] (hex): {}",
                q.q().zeroed(ChunkCoord(FLOOR)),
                distinct
                    .iter()
                    .take(8)
                    .map(|w| format!("{:08x} {:08x} {:08x} {:08x}", w[0], w[1], w[2], w[3]))
                    .collect::<Vec<_>>()
                    .join(" | ")
            );
        }
        println!(
            "the two blocks equal: {}",
            ruled.target(FLOOR) == baked.target(FLOOR)
        );
    }
}
