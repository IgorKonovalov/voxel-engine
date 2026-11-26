//! The record by rule (ADR-0080 rule 1) through the live queue on the software adapter: the
//! zero record of a chunk buried under the column map with no emitter in reach, a chunk with no
//! exposed face, the stream workers' no-face verdict, and the re-listing of a zeroed chunk by a
//! dig to the sky. Beside them the bake's key ray at its end (ADR-0084): a key ray that ends in
//! air under a roof is not lit, one that leaves the window's side above every top is.
//!
//! Every world here is the buried rig's 7 x 7 x 7 chunks at test counts; the rigs are
//! `support::queue`'s.

mod support;

use std::collections::BTreeMap;

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
    buried_rig_of(buried_world(), rule)
}

/// [`buried_rig`]'s world.
fn buried_world() -> svet_world::World {
    use support::queue::{box_op, world_of};
    let world = world_of([7, 7, 7], ROCK);
    for (min, max) in [
        ([0, 0, 320], [448, 448, 448]),
        ([140, 200, 80], [300, 250, 120]),
    ] {
        world
            .apply(&box_op(min, max, Mode::Carve))
            .expect("the box is resident");
    }
    world
}

/// [`buried_rig`] over `world` (the buried world, changed before the mirror's first sync).
fn buried_rig_of(world: svet_world::World, rule: bool) -> Option<support::queue::Q> {
    use support::queue::{Setup, rig};
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

/// Steps `q` until `done` holds after a step (at most [`support::queue::MAX_STEPS`]), asserting
/// after every step that no cave chunk is in a GPU job (a job's chunk holds its `job_coarse` mark
/// from its listing to its placement, a step later at the earliest; a ruled record is placed in
/// its listing's step).
fn step_with_no_cave_job(
    q: &mut support::queue::Q,
    what: &str,
    mut done: impl FnMut(&support::queue::Q) -> bool,
) {
    for _ in 0..support::queue::MAX_STEPS {
        q.step();
        for c in CAVE {
            let t = q.q().tracks(ChunkCoord(c));
            assert!(!t.contains(&"job_coarse"), "{what}: {c:?} in a job: {t:?}");
        }
        if done(q) {
            return;
        }
    }
    panic!(
        "{what}: not done in {} steps: {:?}",
        support::queue::MAX_STEPS,
        q.q().stats()
    );
}

/// [`step_with_no_cave_job`] for [`AFTER_STEPS`] more steps, to a queue with no job open or
/// pending. (The streamed rigs never empty: the chunks by the window's top and sides, whose
/// reach leaves the window inside the full radius, wait for chunks no streamer brings.)
fn steps_on_with_no_cave_job(q: &mut support::queue::Q, what: &str) {
    let mut n = 0;
    step_with_no_cave_job(q, what, |q| {
        n += 1;
        n >= AFTER_STEPS && q.q().stats().jobs == 0
    });
}

/// The steps a streamed rig runs on after the cave's records, for a job that would follow them.
const AFTER_STEPS: usize = 120;

/// The cave's records in `ruled` against the whole world's GPU bake with the rule off: each
/// zeroed, flagged neither provisional nor partial, and equal byte for byte.
fn assert_cave_ruled_as_baked(ruled: &support::queue::Q, what: &str) {
    let Some(mut baked) = buried_rig(false) else {
        return;
    };
    baked.settle();
    for c in CAVE {
        let t = ruled.q().tracks(ChunkCoord(c));
        assert!(
            t.contains(&"zeroed"),
            "{what}: {c:?} took no zero record: {t:?}"
        );
        for flag in ["provisional", "job_provisional", "partial", "job_partial"] {
            assert!(!t.contains(&flag), "{what}: {c:?} is {flag}: {t:?}");
        }
        let (a, b) = (ruled.target(c), baked.target(c));
        assert!(a.is_some(), "{what}: {c:?} holds no block");
        assert_eq!(a, b, "{what}: {c:?}: the rule's record against the GPU's");
    }
    assert_eq!(ruled.q().stats().zero_relisted, 0, "{what}");
}

/// The chunk over the cave's middle column left out of the world (air, above the cave's reach
/// boxes and under the camera's chunk) in [`the_cave_under_a_column_waiting_to_upload_is_ruled`].
const WAITING: [i32; 3] = [3, 3, 5];

fn not_waiting(c: ChunkCoord) -> bool {
    c.0 != WAITING
}

fn every(_: ChunkCoord) -> bool {
    true
}

/// Plan 0101 Phase 2, done-when (4), the column not final: the buried rig with the rule on as a
/// streamed world whose streamer holds every chunk within 8 of the camera's, [`WAITING`] not yet
/// in the world (so neither mirrored nor pending) at the openings that list the cave, the column
/// wait off and the partial reach (the cave's reach crosses the window's floor, past a full
/// radius of 6). The cave's chunks take the zero record with no GPU job; [`WAITING`] arrives and
/// no job follows; their records equal the whole world's GPU bake with the rule off, and they
/// are neither provisional nor partial. Soak: 25.2 s on lavapipe at load 19 (two settles of the
/// 7 x 7 x 7-chunk rig and 120 steps on), over the gate's 10 s line.
#[test]
#[ignore = "soak"]
fn the_cave_under_a_column_waiting_to_upload_is_ruled() {
    let w = ChunkCoord(WAITING);
    let world = buried_world();
    let chunk = world.store().remove(w).expect("the rig's chunk");
    let Some(mut q) = buried_rig_of(world, true) else {
        return;
    };
    q.resident = Some(not_waiting);
    q.lod_radius = Some(8);
    q.full_radius = Some(6);
    q.frame.set_bake_reach(svet_render::ReachMode::Partial);
    let cave_zeroed = |q: &support::queue::Q| CAVE.iter().all(|&c| q.q().zeroed(ChunkCoord(c)));
    step_with_no_cave_job(&mut q, "waiting", cave_zeroed);
    assert!(
        !q.gpu.is_mirrored(w) && !q.gpu.is_pending(w),
        "the chunk over the cave is still waiting"
    );
    let provisional = q.q().stats().provisional_opened;
    println!(
        "waiting: the cave zeroed, {provisional} jobs opened provisional, {:?}",
        q.q().stats()
    );
    assert!(provisional > 0, "columns not final over the rig's chunks");
    q.world
        .store()
        .insert(w, chunk.read().expect("the chunk's lock").clone());
    q.world.store().mark_dirty(w);
    q.resident = Some(every);
    step_with_no_cave_job(&mut q, "arrived", |q| q.gpu.is_mirrored(w));
    steps_on_with_no_cave_job(&mut q, "arrived");
    assert_cave_ruled_as_baked(&q, "waiting");
}

/// The full radius [`the_cave_whose_reach_crosses_the_full_radius_is_ruled`] shrinks to, in
/// chunks about the camera's chunk (3, 3, 6): the cave's chunks lie within it (at most 26 in
/// squared distance), chunks of their reach past it (the window's floor's (1, 3, 0) at 40).
const FULL_RADIUS: i32 = 6;

fn within_full_radius(c: ChunkCoord) -> bool {
    let d = [c.0[0] - 3, c.0[1] - 3, c.0[2] - 6];
    d.iter().map(|v| v * v).sum::<i32>() <= FULL_RADIUS * FULL_RADIUS
}

/// Plan 0101 Phase 2, done-when (4), the partial reach: the buried rig with the rule on as a
/// streamed world whose full radius is [`FULL_RADIUS`], every chunk past it LOD-only, under the
/// partial reach mode, so the cave's reach crosses the full radius and is not whole. The cave's
/// chunks take the zero record with no GPU job; their records equal the whole world's GPU bake
/// with the rule off, and they are neither provisional nor partial. Soak: 16.1 s on lavapipe at
/// load 19, over the gate's 10 s line.
#[test]
#[ignore = "soak"]
fn the_cave_whose_reach_crosses_the_full_radius_is_ruled() {
    let world = buried_world();
    support::make_lod(&world, |c| !within_full_radius(c));
    let Some(mut q) = buried_rig_of(world, true) else {
        return;
    };
    q.resident = Some(within_full_radius);
    q.full_radius = Some(FULL_RADIUS);
    q.frame.set_bake_reach(svet_render::ReachMode::Partial);
    let cave_zeroed = |q: &support::queue::Q| CAVE.iter().all(|&c| q.q().zeroed(ChunkCoord(c)));
    step_with_no_cave_job(&mut q, "partial", cave_zeroed);
    steps_on_with_no_cave_job(&mut q, "partial");
    let st = q.q().stats();
    println!("partial: {st:?}");
    assert!(
        st.partial_opened > 0,
        "chunks of the rig bake from partial reaches"
    );
    for c in CAVE {
        assert!(q.gpu.is_mirrored(ChunkCoord(c)) && !q.gpu.is_lod(ChunkCoord(c)));
    }
    assert_cave_ruled_as_baked(&q, "partial");
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

/// The chunk whose floor the key ray's tests read: wholly inside the tall cave's footprint, its
/// air from z 80, the rock floor's `+z` faces on voxels z 79.
const FLOOR: [i32; 3] = [3, 4, 1];

/// The 7 x 7 x 7-chunk world of rock under air from z 320, with a cave of air over x [190, 290),
/// y [150, 322), z [80, 230) under a roof 90 voxels thick (`roof`; else the cave is open to the
/// sky, its air up to the surface), no emitter; the live queue at light radius 2 (a key ray's
/// cap, `job.radius`, 128 voxels) over a sun at elevation 40. From [`FLOOR`]'s floor a key ray
/// toward the sun rises 82 voxels and runs 93 along -y and 29 along +x within its cap, so every
/// one ends in the cave's air: under the roof and under every column's top (319), or with no
/// roof above the floor's columns' tops (79). `rule` sets the queue's zero rule.
fn tall_cave_rig(rule: bool, roof: bool) -> Option<support::queue::Q> {
    use support::queue::{Setup, box_op, rig, world_of};
    let world = world_of([7, 7, 7], ROCK);
    let top = if roof { 230 } else { 320 };
    for (min, max) in [
        ([0, 0, 320], [448, 448, 448]),
        ([190, 150, 80], [290, 322, top]),
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

/// The `+z` faces of chunk block `block`: their count, and how many read each key visibility
/// (u8; `None` where the record holds no key words).
fn up_faces_by_key(block: &[u32]) -> (usize, BTreeMap<Option<u8>, usize>) {
    use svet_render::internals::{BRICK_WORDS, record_key_visibility};
    let mut faces = 0;
    let mut by_key = BTreeMap::new();
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
    }
    (faces, by_key)
}

/// ADR-0084 (Plan 0096 Phase 10c's defect): a key ray that ends in air at its cap under a roof
/// is not lit. In the closed tall cave the GPU job's record (rule off) holds a key visibility of
/// 0 on every one of [`FLOOR`]'s 64 `+z` faces, and every chunk the rule zeroes holds the job's
/// bytes (Plan 0096 Phase 2 bar 1 with a cave taller than the radius: none differs, the floor
/// among them). With no roof the same floor reads 255 on all 64: the clause does not darken an
/// open sky. Soak: three settles of the 7 x 7 x 7-chunk rig, 19.7 s on lavapipe at load 14.
#[test]
#[ignore = "soak"]
fn a_key_ray_that_ends_in_air_under_a_roof_is_not_lit() {
    let Some(mut ruled) = tall_cave_rig(true, true) else {
        return;
    };
    ruled.settle();
    let Some(mut baked) = tall_cave_rig(false, true) else {
        return;
    };
    baked.settle();
    let floor = baked.target(FLOOR).expect("the floor chunk holds a block");
    assert_eq!(
        up_faces_by_key(&floor),
        (64, BTreeMap::from([(Some(0), 64)]))
    );
    assert!(
        ruled.q().zeroed(ChunkCoord(FLOOR)),
        "the floor takes the rule"
    );
    let mut zeroed = Vec::new();
    let mut differ = Vec::new();
    for c in ruled.world.extent().coords() {
        if ruled.q().zeroed(c) {
            zeroed.push(c.0);
            if ruled.target(c.0) != baked.target(c.0) {
                differ.push(c.0);
            }
        }
    }
    println!("the rule zeroed {} chunks: {zeroed:?}", zeroed.len());
    assert!(zeroed.contains(&FLOOR));
    assert_eq!(
        differ,
        Vec::<[i32; 3]>::new(),
        "the rule's records against the job's"
    );
    let Some(mut open) = tall_cave_rig(false, false) else {
        return;
    };
    open.settle();
    let floor = open.target(FLOOR).expect("the open floor holds a block");
    assert_eq!(
        up_faces_by_key(&floor),
        (64, BTreeMap::from([(Some(255), 64)]))
    );
}

/// The slab rig's floor chunk: x [64, 128), y [0, 64), its rock floor's `+z` faces on voxels z
/// 63, the window's -y side at y 0.
const SLAB_FLOOR: [i32; 3] = [1, 0, 0];

/// The `+z` faces of [`SLAB_FLOOR`] that read lit (255) on the tree before the key ray took the
/// column clause (ADR-0084), read there by this test with the count's assertion off.
const SLAB_LIT: usize = 64;

/// A 3 x 2 x 2-chunk world: rock floor to z 64, air over it, and a roof slab of rock over z
/// [100, 110) on every column from y 8 on, so the floor is covered but along the window's -y
/// side; no emitter, the live queue at light radius 2 (cap 128 voxels), the rule off, a sun at
/// elevation 10 from -y. From [`SLAB_FLOOR`]'s floor a key ray runs at most 64 voxels across
/// the ground (11.3 up) to the window's -y side, under the roof's edge, and leaves the window
/// there at most at z 75, above the edge columns' top (63) and under the roof: every floor face
/// of the chunk stays lit, [`SLAB_LIT`] of them, the count before the change.
#[test]
fn a_key_ray_that_leaves_the_windows_side_above_every_top_stays_lit() {
    use support::queue::{Setup, box_op, fill, rig, world_of};
    let world = world_of([3, 2, 2], ROCK);
    world
        .apply(&box_op([0, 0, 64], [192, 128, 128], Mode::Carve))
        .expect("the box is resident");
    fill(&world, [0, 8, 100], [192, 128, 110], ROCK);
    let Some(mut q) = rig(world, &Setup::at([96.0, 4.0, 80.0])) else {
        return;
    };
    let e = 10f32.to_radians();
    q.frame.set_sky(Some(svet_render::sky::Sky::new(
        svet_render::sky::Body::sun([0.0, -e.cos(), e.sin()]),
        svet_render::sky::Body::moon([0.0, 0.0, -1.0]),
    )));
    q.frame.set_bake_queue(move |q| q.zero_rule = false);
    q.settle();
    let block = q
        .target(SLAB_FLOOR)
        .expect("the slab's floor holds a block");
    let (faces, by_key) = up_faces_by_key(&block);
    let lit = by_key.get(&Some(255)).copied().unwrap_or(0);
    println!("the slab's floor: {faces} +z faces, by key u8 {by_key:?}");
    assert_eq!(faces, 64);
    assert_eq!(
        lit, SLAB_LIT,
        "lit faces against the count before the change"
    );
}
