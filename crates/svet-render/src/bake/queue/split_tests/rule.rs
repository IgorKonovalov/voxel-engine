//! The rule at every opening (ADR-0085) on the bake half's CPU rig: a buried chunk is answered
//! by the zero record whatever its class, its reach or its columns' finality, the record is
//! final, and only the re-list undoes it.
//!
//! The rig is a 3 x 3 window of chunk columns at light radius 1, rock only in the middle chunk
//! `C` one chunk up (every other chunk is air, so faceless: every job is `C`'s). The mirror's
//! column map is the rig's own (`Mirror::tops`, `Mirror::lowest`): it stands in for the sheets
//! the render half builds, whose mip rule `gpu_world::columns` tests.

use super::*;
use crate::bake::job::BAKE_SKY_ON;

/// The rig's one chunk with rock (a slab at the bottom of it).
pub(super) const C: [i32; 3] = [1, 1, 1];
/// `C`'s reach box's top voxel at light radius 1: `(1 + 1 + 1) * 64 - 1`. A column top at it
/// or under it is not over the box ([`super::super::rule::buried`]).
pub(super) const BOX_TOP: i32 = 191;

/// The window `dims` chunks with a slab of rock in `C`.
pub(super) fn rock_world(dims: [i32; 3]) -> World {
    let mut t = MaterialTable::new();
    let rock = t.push(Material::named("rock"));
    let w = World::generate(ChunkExtent::new([0, 0, 0], dims), &UniformSource(AIR), &t);
    let min = C.map(|v| i64::from(v) * 64);
    let max = [min[0] + 63, min[1] + 63, min[2] + i64::from(SLAB_TOP) - 1];
    w.apply(&box_op(min, max, Mode::Fill(rock)))
        .expect("resident");
    w
}

/// The rule's rig: a streamed world (the residency the mirror's full chunks) `z` chunks high,
/// every chunk mirrored, the camera in `C`, the sky on, every column top over the reach boxes.
pub(super) fn rule_rig(z: i32) -> Rig {
    let mut rig = Rig::sized(None, 3, 3);
    rig.world = rock_world([3, 3, z]);
    rig.mirror.dims = [3, z];
    rig.mirror.lr = 1;
    rig.mirror.lowest = BURIED;
    rig.store = Store::new(64 << 20);
    rig.store
        .start_queue(rig.mirror.window(), rig.queue.settings.cell);
    let _ = rig.store.take_direct();
    rig.view.eye = C.map(|v| v as f32 * 64.0 + 32.0);
    rig.view.streamed = true;
    rig.view.sky = BAKE_SKY_ON;
    rig
}

/// The chunk at the top of `C`'s column in [`provisional_rig`], held by the streamer and not
/// yet mirrored: above the reach box, so `C`'s reach is ready and its column is not final.
const WAITING: [i32; 3] = [1, 1, 4];

/// [`rule_rig`] five chunks high whose streamer holds every chunk within 8 of the camera, with
/// [`WAITING`] not mirrored: `C`'s first opening is under a column not final.
fn provisional_rig() -> Rig {
    let mut rig = rule_rig(5);
    rig.view.lod_radius = Some(8);
    rig.mirror.absent.insert(WAITING);
    rig
}

/// [`rule_rig`] four chunks high with no streamer hold (every column final) and a full radius
/// of 0 under the partial reach mode: `C`'s reach crosses the full radius, and it bakes from the
/// chunks resident once one in its reach is not.
pub(super) fn partial_rig() -> Rig {
    let mut rig = rule_rig(4);
    rig.view.full_radius = Some(0);
    rig.queue.reach_mode = ReachMode::Partial;
    rig
}

/// The jobs placed so far: every block placed but the zero records (only `C` has faces).
pub(super) fn jobs(rig: &Rig) -> u64 {
    rig.queue.stats.baked - rig.queue.stats.zeroed
}

fn in_job(rig: &Rig) -> bool {
    rig.queue.jobs().contains(&ChunkCoord(C))
}

fn landings(rig: &Rig) -> u64 {
    rig.queue.stats.landings.iter().sum()
}

/// Steps until `C` holds a block, nothing is in a job or in flight and no fade runs.
pub(super) fn settle(rig: &mut Rig, what: &str) {
    rig.until(what, |r| {
        r.store.block(ChunkCoord(C)).is_some()
            && r.queue.lp.open.is_empty()
            && r.queue.lp.pending.is_empty()
            && r.half.flights.is_empty()
            && r.queue.fades.is_empty()
    });
}

/// The rule's zero record of `C`'s faces as the rig's step writes it (the sky on, no key light),
/// with the light summary the store writes from its records at the put (the sun stamp kept).
pub(super) fn zero_of_c(rig: &mut Rig) -> Vec<u32> {
    let classes = rig.queue.classes(rig.world.materials());
    let faces = rig.queue.list(&rig.world, ChunkCoord(C), &classes).0;
    let s = &rig.queue.settings;
    let mut zero =
        super::super::rule::zero_block(ChunkCoord(C), &faces, s.cell, s.clamp, true, None, 0);
    let summary = crate::bake::summary::block_summary(&zero, record_words(s.cell));
    let (at, stamp) = (SUMMARY_AT as usize, zero[KEY_SUN_AT as usize]);
    zero[at..at + BRICK_WORDS].copy_from_slice(&summary.unwrap_or_default());
    zero[KEY_SUN_AT as usize] = stamp;
    zero
}

pub(super) fn flags_of_c(rig: &Rig) -> [bool; 4] {
    [
        rig.queue.provisional.contains(&C),
        rig.queue.partial.contains(&C),
        rig.queue.job_provisional.contains(&C),
        rig.queue.job_partial.contains(&C),
    ]
}

/// Plan 0101 Phase 2, done-when (1). `C`'s column top lies in its reach box while
/// [`WAITING`] is not mirrored: the rule does not hold, `C` bakes by a job, flagged provisional.
/// `WAITING` arrives and the column's top rises over the box: the catch-up opens `C` again as a
/// class 1 candidate, the rule answers it, and its block becomes the zero record with no second
/// job, landing once more with one fade from the block it held.
#[test]
fn a_catch_up_the_rule_answers_writes_the_zero_record_without_a_job() {
    let mut rig = provisional_rig();
    rig.mirror.tops.insert([1, 1], BOX_TOP);
    settle(&mut rig, "C's provisional block");
    assert_eq!(jobs(&rig), 1, "C's first bake is a job");
    assert_eq!(rig.queue.stats.zeroed, 0);
    assert!(rig.queue.provisional.contains(&C), "flagged provisional");
    let first = rig.queue.stats.landings;
    assert_eq!(landings(&rig), 1, "{first:?}");

    rig.mirror.absent.remove(&WAITING);
    rig.mirror.tops.remove(&[1, 1]);
    rig.mirror.changed.push(ChunkCoord(WAITING));
    rig.until("C's catch-up ruled", |r| r.queue.stats.zeroed == 1);
    settle(&mut rig, "the catch-up's landing");
    for _ in 0..30 {
        rig.step();
    }
    assert_eq!(rig.queue.stats.provisional_requeued, 1, "C caught up once");
    assert_eq!(jobs(&rig), 1, "no second job");
    assert_eq!(rig.queue.stats.zeroed, 1);
    assert!(rig.queue.zeroed.contains(&C));
    assert_eq!(flags_of_c(&rig), [false; 4], "the zero record is final");
    let zero = zero_of_c(&mut rig);
    assert_eq!(
        rig.store.words.get(&C),
        Some(&zero),
        "C's block is the zero record"
    );
    let l = rig.queue.stats.landings;
    assert_eq!(landings(&rig), 2, "{l:?}");
    assert_eq!(
        l[LANDED_REFADED],
        first[LANDED_REFADED] + 1,
        "one fade from the block C held: {l:?}"
    );
}

/// Plan 0101 Phase 2, done-when (2). `C` is buried and [`WAITING`], above the tops it is under,
/// is not mirrored at its first opening: the rule answers it with the zero record and no job,
/// flagged neither provisional nor partial; `WAITING` arrives and `C` takes no job, no catch-up
/// and no re-list.
#[test]
fn a_first_opening_under_a_column_not_final_is_ruled_for_good() {
    let mut rig = provisional_rig();
    settle(&mut rig, "C's zero record");
    assert_eq!(rig.queue.stats.zeroed, 1, "C ruled");
    assert_eq!(jobs(&rig), 0, "no job");
    assert_eq!(rig.queue.stats.provisional_opened, 0);
    assert!(rig.queue.zeroed.contains(&C));
    assert_eq!(
        flags_of_c(&rig),
        [false; 4],
        "neither provisional nor partial"
    );
    let zero = zero_of_c(&mut rig);
    assert_eq!(rig.store.words.get(&C), Some(&zero));

    rig.mirror.absent.remove(&WAITING);
    rig.mirror.changed.push(ChunkCoord(WAITING));
    for _ in 0..60 {
        rig.step();
        assert!(!in_job(&rig), "no job when the waiting chunk arrives");
    }
    assert_eq!(jobs(&rig), 0);
    assert_eq!(rig.queue.stats.provisional_requeued, 0, "no catch-up");
    assert_eq!(rig.queue.stats.zero_relisted, 0, "no re-list");
    assert_eq!(rig.queue.stats.zeroed, 1);
    assert_eq!(rig.store.words.get(&C), Some(&zero));
}

/// The re-list of a ruled `C` on one change: `change` is made, and the step that reads it
/// re-lists `C` (out of the zeroed set, due to bake whole); its job's block lands with one fade over
/// the zero record, and no other job follows.
fn relists_on(rig: &mut Rig, what: &str, change: impl FnOnce(&mut Mirror)) {
    settle(rig, "C's zero record");
    assert_eq!(rig.queue.stats.zeroed, 1, "{what}: C ruled");
    assert_eq!(jobs(rig), 0, "{what}: no job");
    assert_eq!(flags_of_c(rig), [false; 4], "{what}: final");
    let before = rig.queue.stats.landings;
    change(&mut rig.mirror);
    rig.step();
    assert_eq!(rig.queue.stats.zero_relisted, 1, "{what}: re-listed");
    assert!(
        !rig.queue.zeroed.contains(&C),
        "{what}: out of the zeroed set"
    );
    assert!(
        rig.queue.reach.contains(&C) || in_job(rig),
        "{what}: due to bake whole, or its job open"
    );
    rig.until(what, |r| jobs(r) == 1);
    settle(rig, what);
    for _ in 0..30 {
        rig.step();
    }
    assert_eq!(jobs(rig), 1, "{what}: one job");
    assert!(!rig.queue.zeroed.contains(&C), "{what}");
    let l = rig.queue.stats.landings;
    assert_eq!(landings(rig), 2, "{what}: {l:?}");
    assert_eq!(
        l[LANDED_REFADED],
        before[LANDED_REFADED] + 1,
        "{what}: one fade over the zero record: {l:?}"
    );
}

/// Plan 0101 Phase 2, done-when (3): a chunk ruled under a column not final or a partial reach
/// is re-listed with a job, landing as one fade, by each of the four changes that end the rule.
#[test]
fn a_ruled_chunk_is_relisted_when_the_rule_stops_holding() {
    // A column carved from the surface into the reach box: column (2, 2)'s top falls to it.
    let mut rig = provisional_rig();
    relists_on(&mut rig, "a carved column", |m| {
        m.tops.insert([2, 2], BOX_TOP);
        m.changed.push(ChunkCoord([2, 2, 4]));
    });

    // A chunk with an emitter arriving within the reach: the hole `H` beside `C` (its reach
    // partial until then) arrives with an exposed emitter.
    const H: [i32; 3] = [0, 1, 1];
    let mut rig = partial_rig();
    rig.mirror.absent.insert(H);
    relists_on(&mut rig, "an emitter's chunk arriving", |m| {
        m.absent.remove(&H);
        m.lights.push((
            H,
            LightNote {
                emitter: true,
                current: true,
            },
        ));
        m.light_gen += 1;
        m.changed.push(ChunkCoord(H));
    });

    // The chunk over `C`'s reach in its column evicted, the column's lowest top falling to the
    // box.
    let mut rig = provisional_rig();
    relists_on(&mut rig, "the chunk over it evicted", |m| {
        m.absent.insert([1, 1, 3]);
        m.tops.insert([1, 1], BOX_TOP);
        m.changed.push(ChunkCoord([1, 1, 3]));
    });

    // A full chunk arriving over its LOD version (the chunk over `C`, in its reach, so the
    // reach partial until then): the LOD sheet tops the box by one voxel (a 2-voxel cell counted
    // solid up to its top voxel), the full sheet does not.
    const L: [i32; 3] = [1, 1, 2];
    let mut rig = partial_rig();
    rig.mirror.lod.insert(L);
    rig.mirror.tops.insert([1, 1], BOX_TOP + 1);
    relists_on(&mut rig, "a full chunk over its LOD version", |m| {
        m.lod.remove(&L);
        m.tops.insert([1, 1], BOX_TOP);
        m.changed.push(ChunkCoord(L));
    });
}

/// The chunk over `C` in [`partial_rig`]'s window, given a slab of rock at its bottom by
/// [`border_rig`]: its six neighbours are held from the start.
const D: [i32; 3] = [1, 1, 2];
/// `C`'s neighbour across its negative x face: air, so `C`'s slab lists faces toward it once it
/// is held.
const N: [i32; 3] = [0, 1, 1];

/// `c`'s faces as a listing reads the world now.
fn faces_of(rig: &mut Rig, c: [i32; 3]) -> usize {
    let classes = rig.queue.classes(rig.world.materials());
    rig.queue
        .list(&rig.world, ChunkCoord(c), &classes)
        .0
        .faces()
}

/// [`partial_rig`] with a slab of rock at the bottom of [`D`] as well as `C`'s, and [`N`] in
/// neither the mirror nor the world's store; the chunk it took out of the store.
fn border_rig() -> (Rig, Chunk) {
    let mut rig = partial_rig();
    let rock = rig
        .world
        .material_at([C[0] * 64, C[1] * 64, C[2] * 64])
        .expect("C's slab is resident");
    let min = D.map(|v| i64::from(v) * 64);
    let max = [min[0] + 63, min[1] + 63, min[2] + i64::from(SLAB_TOP) - 1];
    rig.world
        .apply(&box_op(min, max, Mode::Fill(rock)))
        .expect("resident");
    let n = rig
        .world
        .store()
        .remove(ChunkCoord(N))
        .expect("N is in the store");
    let n = n.read().expect("unpoisoned").clone();
    rig.mirror.absent.insert(N);
    (rig, n)
}

/// Plan 0101 Phase 4, done-when (2). `C`'s slab meets the air of [`N`] across its negative x
/// face, and `N` is in neither the world's store nor the mirror at `C`'s opening: `C` is ruled
/// with 0 jobs, its faces listed with `N` read as rock. `N` arrives: `C` is listed again and
/// ruled again, its block the zero record of its faces listed with `N` present (more faces than
/// before), with 0 jobs, still zeroed, the four flags clear. [`D`], ruled with its six neighbours
/// held, is not listed again by `N`'s arrival (`N` is not one of them).
#[test]
fn a_ruled_chunk_lists_its_border_faces_when_the_neighbour_arrives() {
    let (mut rig, n) = border_rig();
    rig.until("C and D ruled", |r| r.queue.stats.zeroed == 2);
    settle(&mut rig, "C's and D's zero records");
    assert!(rig.queue.zeroed.contains(&C) && rig.queue.zeroed.contains(&D));
    assert_eq!(jobs(&rig), 0, "no job");
    let before = faces_of(&mut rig, C);
    let zero = zero_of_c(&mut rig);
    assert_eq!(rig.store.words.get(&C), Some(&zero), "C's record, N absent");
    let d = rig.store.words.get(&D).cloned().expect("D's record");

    rig.world.store().insert(ChunkCoord(N), n);
    rig.mirror.absent.remove(&N);
    rig.mirror.changed.push(ChunkCoord(N));
    rig.until("C ruled again", |r| r.queue.stats.zeroed == 3);
    settle(&mut rig, "C's second zero record");
    for _ in 0..30 {
        rig.step();
        assert!(!in_job(&rig), "no job for C");
    }
    let after = faces_of(&mut rig, C);
    assert!(
        after > before,
        "C lists {after} faces with N, {before} without"
    );
    assert_eq!(jobs(&rig), 0, "no job");
    assert_eq!(rig.queue.stats.zeroed, 3, "C ruled twice, D once");
    assert_eq!(
        rig.queue.stats.zero_relisted, 0,
        "not by the rule's re-list"
    );
    assert_eq!(
        rig.queue.stats.zero_border_relisted, 1,
        "C listed again once, D not"
    );
    assert!(rig.queue.zeroed.contains(&C), "C still zeroed");
    assert_eq!(flags_of_c(&rig), [false; 4], "the four flags clear");
    let zero = zero_of_c(&mut rig);
    assert_eq!(
        rig.store.words.get(&C),
        Some(&zero),
        "C's record is the zero record of its faces with N present"
    );
    assert!(rig.queue.zeroed.contains(&D), "D still zeroed");
    assert_eq!(rig.store.words.get(&D), Some(&d), "D not listed again");
}

/// Plan 0101 Phase 4 (c), the hash line's reading ([`RuleHashes`]). `C` is ruled on the rule's
/// rig: the zeroed chunks' hashes name `C` alone, its hash FNV-1a 64 over its block's words but
/// the landing word, the set's over its coordinate and that hash; a landing word changed moves no
/// hash, a record word changed moves `C`'s, the set's and the store's. The bake thread's answer
/// is read once per newer request.
#[test]
fn the_rule_hashes_name_each_zeroed_block_and_skip_its_landing_word() {
    use svet_world::hash::Fnv1a;
    let mut rig = rule_rig(3);
    settle(&mut rig, "C ruled");
    assert_eq!(jobs(&rig), 0, "C ruled with no job");
    assert!(rig.queue.zeroed.contains(&C));
    let words = rig.store.words.get(&C).expect("C's block").clone();
    let landed = LANDED_AT as usize;
    let mut f = Fnv1a::default();
    for (i, w) in words.iter().enumerate() {
        if i != landed {
            f.write(&w.to_le_bytes());
        }
    }
    let mut set = Fnv1a::default();
    for v in C {
        set.write(&v.to_le_bytes());
    }
    set.write(&f.finish().to_le_bytes());

    let h = rig.queue.rule_hashes_of(&rig.store, 7);
    assert_eq!(h.asked, 7);
    assert_eq!(h.unplaced, 0);
    assert_eq!(
        h.chunks,
        vec![(C, f.finish())],
        "C alone, its words but the landing word"
    );
    assert_eq!(h.zeroed, set.finish());
    assert_eq!(h.blocks, 1, "only C has faces");
    assert_eq!(h.store, h.zeroed, "the store holds C's block alone");

    rig.store.landed(C, words[landed] ^ 0x5a5a);
    assert_ne!(
        rig.store.words.get(&C).map(|w| w[landed]),
        Some(words[landed])
    );
    assert_eq!(
        rig.queue.rule_hashes_of(&rig.store, 7),
        h,
        "the landing word is not hashed"
    );
    let last = words.len() - 1;
    assert_ne!(last, landed);
    rig.store.words.get_mut(&C).expect("C's block")[last] ^= 1;
    let moved = rig.queue.rule_hashes_of(&rig.store, 7);
    assert_ne!(moved.chunks[0].1, h.chunks[0].1, "a record word is hashed");
    assert_ne!(moved.zeroed, h.zeroed);
    assert_ne!(moved.store, h.store);

    rig.queue.answer_rule_hashes(&rig.store);
    assert!(
        rig.queue.rule_hashes.is_none(),
        "nothing asked, nothing read"
    );
    rig.queue.ask_rule_hashes(1);
    rig.queue.answer_rule_hashes(&rig.store);
    let first = rig.queue.rule_hashes.clone().expect("request 1 answered");
    assert_eq!((first.asked, &first.chunks), (1, &moved.chunks));
    rig.queue.ask_rule_hashes(1);
    rig.queue.answer_rule_hashes(&rig.store);
    let again = rig.queue.rule_hashes.clone().expect("held");
    assert!(
        std::sync::Arc::ptr_eq(&first, &again),
        "an answered request is not read again"
    );
    rig.queue.ask_rule_hashes(2);
    rig.queue.answer_rule_hashes(&rig.store);
    assert_eq!(rig.queue.rule_hashes.as_ref().map(|h| h.asked), Some(2));
}
