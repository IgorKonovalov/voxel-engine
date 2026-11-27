//! A whole dig (a box into a chunk that held no block) on the bake half's CPU rig: under
//! `DigEstimate::Depth` it takes a provisional block with held records at the edit, and its job
//! lands on that block through the dig's fade; under `Surface` it holds no block until its job.
//!
//! The rig's window is one chunk column two chunks high: rock fills the lower chunk `LOW` whole
//! (no air in it or beside it, so it lists no face and holds no block) and the upper one's bottom
//! slab, under which the dig's box reaches into `LOW`.

use super::*;
use crate::DigEstimate;

/// The chunk that holds no block until the dig.
const LOW: [i32; 3] = [0, 0, 0];
/// The chunk above it, whose slab's top faces the air.
const HIGH: [i32; 3] = [0, 0, 1];
/// The rock's top: `HIGH`'s slab, `SLAB_TOP` voxels over `LOW`'s top (a box's bounds hold the
/// voxels whose centres lie within them, so the rock is the voxels under it).
const ROCK_TOP: i64 = 64 + SLAB_TOP as i64;
/// The dig's box: down through `HIGH`'s slab and 8 voxels into `LOW`.
const DIG: ([i32; 3], [i32; 3]) = ([24, 24, 56], [40, 40, ROCK_TOP as i32]);

/// The rig two chunks high, rock to [`ROCK_TOP`], under `estimate`, settled: `HIGH` holds its
/// job's block, nothing in a job, in flight or fading.
fn whole_rig(estimate: DigEstimate) -> Rig {
    let mut rig = Rig::sized(None, 1, 1);
    let mut t = MaterialTable::new();
    let rock = t.push(Material::named("rock"));
    let w = World::generate(
        ChunkExtent::new([0, 0, 0], [1, 1, 2]),
        &UniformSource(AIR),
        &t,
    );
    // One box a chunk: an edit's extent is bounded.
    for (lo, hi) in [(0, 64), (64, ROCK_TOP)] {
        w.apply(&box_op([0, 0, lo], [64, 64, hi], Mode::Fill(rock)))
            .expect("resident");
    }
    rig.world = w;
    rig.mirror.dims = [1, 2];
    rig.mirror.lowest = ROCK_TOP as i32;
    rig.store = Store::new(64 << 20);
    rig.store
        .start_queue(rig.mirror.window(), rig.queue.settings.cell);
    let _ = rig.store.take_direct();
    rig.view.eye = [32.0, 32.0, 100.0];
    rig.settings.dig_estimate = estimate;
    rig.until("HIGH holds its block, the queue quiet", |r| {
        r.store.block(ChunkCoord(HIGH)).is_some()
            && r.queue.lp.open.is_empty()
            && r.queue.lp.pending.is_empty()
            && r.half.flights.is_empty()
            && r.queue.fades.is_empty()
    });
    let classes = rig.queue.classes(rig.world.materials());
    let faces = rig.queue.list(&rig.world, ChunkCoord(LOW), &classes).0;
    assert_eq!(faces.faces(), 0, "LOW lists no face");
    assert!(
        rig.store.block(ChunkCoord(LOW)).is_none(),
        "LOW holds no block"
    );
    assert!(rig.queue.dig_store_hold, "the hold on (the default)");
    rig
}

/// Each listed brick's cell records in `block`: whether it holds [`VIS_BAKED`] and whether
/// [`SKY_STALE`], a pair a cell.
fn vis_flags(block: &[u32], record: usize) -> Vec<(bool, bool)> {
    let mut out = Vec::new();
    for &dir in &block[..CHUNK_BRICKS] {
        if dir == 0 {
            continue;
        }
        let run = (dir >> 11) as usize;
        for at in (run - record..run).step_by(BRICK_WORDS) {
            let w = block[at + VIS_WORD + 1];
            out.push((w & VIS_BAKED != 0, w & SKY_STALE != 0));
        }
    }
    out
}

/// Plan 0105 Phase 5, done-when (the queue's CPU test). Under `Depth` a box into a chunk that
/// held no block leaves, in the edit's step, a block whose touched bricks (every brick it lists:
/// the rock had no face before the dig) hold records with [`VIS_BAKED`] and no [`SKY_STALE`],
/// counted as a whole dig; its job bakes every listed brick (keeps none of the held records) and
/// lands on that block through the dig's fade ([`DIG_FADE`] frames, `dig` set, counted as a
/// refade). Under `Surface` the same box leaves `LOW` with no block in the edit's step.
#[test]
fn a_box_into_a_chunk_with_no_block_holds_records_under_depth() {
    let record = |rig: &Rig| record_words(rig.queue.settings.cell);

    let mut rig = whole_rig(DigEstimate::Surface);
    rig.carve(DIG.0, DIG.1);
    rig.step();
    assert_eq!(rig.queue.stats.digs_whole, 1, "the dig into LOW is whole");
    assert!(
        rig.store.block_words(ChunkCoord(LOW)).is_none(),
        "surface: LOW holds no block at the edit"
    );

    let mut rig = whole_rig(DigEstimate::Depth);
    let refaded = rig.queue.stats.landings[LANDED_REFADED];
    rig.carve(DIG.0, DIG.1);
    rig.step();
    assert_eq!(rig.queue.stats.digs_whole, 1, "the dig into LOW is whole");
    assert_eq!(rig.queue.stats.pool_full, 0);
    let held = rig
        .store
        .block_words(ChunkCoord(LOW))
        .expect("depth: LOW holds a block at the edit")
        .to_vec();
    let flags = vis_flags(&held, record(&rig));
    assert!(!flags.is_empty(), "LOW lists the dig's faces");
    assert!(
        flags.iter().all(|&(baked, stale)| baked && !stale),
        "every held cell VIS_BAKED, none SKY_STALE: {flags:?}"
    );

    // The job: every listed brick baked, none kept, then one dig fade from the held block.
    let job = |r: &Rig| {
        r.queue
            .lp
            .open
            .iter()
            .find(|j| j.coord == ChunkCoord(LOW))
            .map(|j| (j.kept_bricks.len(), j.faces.bricks.len()))
    };
    rig.until("LOW's job open", |r| job(r).is_some());
    let (kept, bricks) = job(&rig).expect("open");
    assert_eq!(kept, 0, "a whole dig keeps no held record");
    assert_eq!(bricks, flags.len() / (record(&rig) / BRICK_WORDS));
    rig.until("LOW's job lands", |r| {
        r.queue.stats.landings[LANDED_REFADED] > refaded
    });
    let fade = rig
        .queue
        .fades
        .get(&LOW)
        .expect("the landing fades from the held block");
    assert!(fade.dig, "a dig's fade");
    assert_eq!(fade.frames, DIG_FADE);
    assert_eq!(fade.from, held, "it fades from the held block");
}

/// The ground rig's one chunk.
const GROUND: [i32; 3] = [0, 0, 0];

/// The ground rig: one chunk, rock in voxels z 0..=7 across it (the ground, its top faces at
/// z 7) and a kerb of rock floating over its `x < 16` strip in voxels z 10..=13 (its `+x`
/// faces at x 15), under `Depth`. With `hold` the render half holds every read-back, so no job
/// lands and `GROUND` lists faces and holds no block (ground the queue never reached);
/// without, it is stepped until `GROUND` holds its job's block and the queue is quiet.
fn ground_rig(hold: bool) -> Rig {
    let mut rig = Rig::sized(None, 1, 1);
    let mut t = MaterialTable::new();
    let rock = t.push(Material::named("rock"));
    let w = World::generate(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &UniformSource(AIR),
        &t,
    );
    for (lo, hi) in [([0, 0, 0], [64, 64, 8]), ([0, 0, 10], [16, 64, 14])] {
        w.apply(&box_op(lo, hi, Mode::Fill(rock)))
            .expect("resident");
    }
    rig.world = w;
    rig.half.hold = hold;
    rig.settings.dig_estimate = DigEstimate::Depth;
    if hold {
        for _ in 0..8 {
            rig.step();
        }
        assert!(
            rig.store.block(ChunkCoord(GROUND)).is_none(),
            "GROUND holds no block"
        );
    } else {
        rig.until("GROUND holds its block, the queue quiet", |r| {
            r.store.block(ChunkCoord(GROUND)).is_some()
                && r.queue.lp.open.is_empty()
                && r.queue.lp.pending.is_empty()
                && r.half.flights.is_empty()
                && r.queue.fades.is_empty()
        });
    }
    assert!(rig.queue.dig_store_hold, "the hold on (the default)");
    rig
}

/// Carves the box `min..max` (voxel bounds, max exclusive) as [`Rig::carve`] does and returns
/// the changed voxels' inclusive bounds (the box the queue reads).
fn carve_box(rig: &mut Rig, min: [i32; 3], max: [i32; 3]) -> [[i32; 3]; 2] {
    let r = rig
        .world
        .apply(&box_op(min.map(i64::from), max.map(i64::from), Mode::Carve))
        .expect("resident");
    let (lo, hi) = r.aabb.expect("voxels changed");
    rig.mirror.changed.push(ChunkCoord(GROUND));
    rig.inbox.push_box(lo.0, hi.0);
    [lo.0, hi.0]
}

/// `block`'s record of brick `slot` (one cell of 8 voxels), `None` where it lists no record.
fn brick_record(block: &[u32], slot: usize) -> Option<&[u32]> {
    let d = block[slot];
    (d != 0).then(|| {
        let run = (d >> 11) as usize;
        &block[run - BRICK_WORDS..run]
    })
}

/// `block`'s summary record's flags: whether it holds [`VIS_BAKED`] and whether [`SKY_STALE`].
fn summary_flags(block: &[u32]) -> (bool, bool) {
    let w = block[SUMMARY_AT as usize + VIS_WORD + 1];
    (w & VIS_BAKED != 0, w & SKY_STALE != 0)
}

/// Plan 0105 Phase 7a4a, step 1: under `Depth` the provisional block of a whole dig answers no
/// sky from its summary until its job lands. On the ground rig (faces, no block), one box: after
/// the edit the chunk's summary holds no fresh sky (no [`VIS_BAKED`], or [`SKY_STALE`]); it still
/// holds none after a second box in the same chunk and after a [`Store::show`] of the block; a
/// brick the first box (dilated by one voxel) does not touch reads [`record_visibility`] `None`
/// in six directions; once the chunk's job lands, its summary is fresh.
#[test]
fn a_whole_digs_block_answers_no_sky_outside_its_bricks() {
    let mut rig = ground_rig(true);
    let whole = rig.queue.stats.digs_whole;
    let [lo, hi] = carve_box(&mut rig, [24, 24, 4], [40, 40, 8]);
    rig.step();
    assert_eq!(rig.queue.stats.digs_whole, whole + 1, "the dig is whole");
    let block = rig
        .store
        .block_words(ChunkCoord(GROUND))
        .expect("depth: GROUND holds a block at the edit")
        .to_vec();
    let fresh = |b: &[u32]| {
        let (baked, stale) = summary_flags(b);
        baked && !stale
    };
    assert!(
        !fresh(&block),
        "after the edit: the summary (VIS_BAKED, SKY_STALE) {:?}",
        summary_flags(&block)
    );
    // A listed brick the dilated box does not touch: no visibility in any direction.
    let classes = rig.queue.classes(rig.world.materials());
    let faces = rig.queue.list(&rig.world, ChunkCoord(GROUND), &classes).0;
    let mut untouched = 0;
    for (slot, _) in &faces.bricks {
        let b = brick_of_slot(usize::from(*slot));
        if (0..3).all(|k| b[k] * 8 + 7 >= lo[k] - 1 && b[k] * 8 <= hi[k] + 1) {
            continue;
        }
        untouched += 1;
        let r = brick_record(&block, usize::from(*slot)).expect("a listed brick's record");
        let vis: Vec<_> = (0..6).map(|f| record_visibility(r, f)).collect();
        assert!(
            vis.iter().all(Option::is_none),
            "untouched brick {slot}: {vis:?}"
        );
    }
    assert!(
        untouched > 0,
        "the chunk lists bricks the box does not touch"
    );
    let first_held = block;

    // A second box in the same chunk, away from the first.
    carve_box(&mut rig, [48, 48, 4], [56, 56, 8]);
    rig.step();
    let block = rig
        .store
        .block_words(ChunkCoord(GROUND))
        .expect("GROUND's block after the second box")
        .to_vec();
    assert!(
        !fresh(&block),
        "after a second box: the summary {:?}",
        summary_flags(&block)
    );

    // A show of the block rewrites its summary in the words it shows.
    let (first, _) = rig.store.blocks[&GROUND];
    let mut shown = block.clone();
    rig.store.show(ChunkCoord(GROUND), first, &mut shown);
    assert!(
        !fresh(&shown),
        "after a show: the summary {:?}",
        summary_flags(&shown)
    );

    // The job lands: the summary is made from baked cells.
    let refaded = rig.queue.stats.landings[LANDED_REFADED];
    rig.half.hold = false;
    rig.until("GROUND's job lands", |r| {
        r.queue.stats.landings[LANDED_REFADED] > refaded
    });
    let landed = rig
        .store
        .block_words(ChunkCoord(GROUND))
        .expect("GROUND's landed block")
        .to_vec();
    // The rig's render half echoes no sky bake, so the landed cells hold no VIS_BAKED: the
    // landed summary is its cells' own, not stale; and a show of words whose cells hold baked
    // visibilities, none stale (the first box's held block), now writes a fresh summary.
    let made = crate::bake::summary::block_summary(&landed, record_words(rig.queue.settings.cell))
        .unwrap_or_default();
    let at = SUMMARY_AT as usize + VIS_WORD;
    assert_eq!(
        landed[at..at + 2],
        made[VIS_WORD..VIS_WORD + 2],
        "after the landing: the summary is the landed cells' ({:?})",
        summary_flags(&landed)
    );
    let mut shown = first_held;
    rig.store.show(ChunkCoord(GROUND), first, &mut shown);
    assert!(
        fresh(&shown),
        "after the landing, a show of baked cells: the summary {:?}",
        summary_flags(&shown)
    );
}

/// Plan 0105 Phase 7a2, step 1: under `Depth` a held record moves only the dig's faces.
/// (1) A box cut at one place into ground the queue never baked (faces, no block, a whole
/// dig): in each brick the box dilated by one voxel touches, a direction with a face outside
/// that dilated box (a face the dig did not make) reads no visibility in the held record
/// ([`record_visibility`] `None`: the light pass reads it as it read no block); (2) a direction
/// all of whose faces lie inside it holds a value with [`VIS_BAKED`] and no [`SKY_STALE`].
/// (3) In a chunk with a block whose summary holds no up visibility (the summary's up and the
/// kerb cell's up cleared by hand, so the kerb's cell has no base: no record's up at or within
/// [`super::super::super::held::HELD_CELLS`] cells above it, no summary's), a box beside the
/// kerb: the kerb cell's `+x` direction, every face of it kept, keeps its old byte.
///
/// Red first: the `should_panic` mark pins the clauses the tree misses before the fix, so the
/// quick tier stays green on that tree and turns red once the fix meets any of them; the fix
/// removes the mark.
#[test]
#[should_panic(expected = "failed clauses [1, 3]")]
fn a_whole_dig_moves_no_face_it_did_not_make() {
    // (1) and (2): a whole dig into ground that was never baked.
    let mut rig = ground_rig(true);
    let whole = rig.queue.stats.digs_whole;
    let [lo, hi] = carve_box(&mut rig, [24, 24, 4], [40, 40, 8]);
    rig.step();
    assert_eq!(rig.queue.stats.digs_whole, whole + 1, "the dig is whole");
    let classes = rig.queue.classes(rig.world.materials());
    let faces = rig.queue.list(&rig.world, ChunkCoord(GROUND), &classes).0;
    let held = rig
        .store
        .block_words(ChunkCoord(GROUND))
        .expect("depth: GROUND holds a block at the edit")
        .to_vec();
    let inside = |g: [i32; 3]| (0..3).all(|k| (lo[k] - 1..=hi[k] + 1).contains(&g[k]));
    // Per touched brick and direction: (slot, direction, kept faces, dig faces, the record's
    // visibility, VIS_BAKED, SKY_STALE).
    let mut rows = Vec::new();
    for (slot, keys) in &faces.bricks {
        let b = brick_of_slot(usize::from(*slot));
        let touched = (0..3).all(|k| b[k] * 8 + 7 >= lo[k] - 1 && b[k] * 8 <= hi[k] + 1);
        if !touched {
            continue;
        }
        let r = brick_record(&held, usize::from(*slot)).expect("a touched brick's record");
        for f in 0..6 {
            let (mut kept, mut dug) = (0, 0);
            for &key in keys.iter().filter(|&&k| usize::from(k & 7) == f) {
                let v = i32::from(key >> 3);
                let g = [b[0] * 8 + v % 8, b[1] * 8 + (v / 8) % 8, b[2] * 8 + v / 64];
                if inside(g) {
                    dug += 1;
                } else {
                    kept += 1;
                }
            }
            if kept + dug > 0 {
                let w = r[VIS_WORD + 1];
                rows.push((
                    *slot,
                    f,
                    kept,
                    dug,
                    record_visibility(r, f),
                    w & VIS_BAKED != 0,
                    w & SKY_STALE != 0,
                ));
            }
        }
    }
    let mixed: Vec<_> = rows.iter().filter(|r| r.2 > 0).collect();
    let all_dug: Vec<_> = rows.iter().filter(|r| r.2 == 0).collect();
    assert!(
        !mixed.is_empty() && !all_dug.is_empty(),
        "both kinds: {rows:?}"
    );
    let moved: Vec<_> = mixed.iter().filter(|r| r.4.is_some()).collect();
    let unheld: Vec<_> = all_dug
        .iter()
        .filter(|r| r.4.is_none() || !r.5 || r.6)
        .collect();

    // (3): a chunk with a block, its summary and the kerb cell's up cleared.
    let mut rig = ground_rig(false);
    let kerb = brick_slot([1, 2, 1]);
    let old = 0.5f32;
    {
        let block = rig.store.words.get_mut(&GROUND).expect("GROUND's block");
        let at = SUMMARY_AT as usize;
        block[at + 6] &= !(1 << (16 + 5));
        let d = block[kerb];
        assert_ne!(d, 0, "the kerb's brick holds a record");
        let run = (d >> 11) as usize;
        let r = &mut block[run - BRICK_WORDS..run];
        r[6] = (r[6] & 0xFFFF) | 0b00_0011 << 16;
        r[VIS_WORD..VIS_WORD + 2]
            .copy_from_slice(&visibility_words([0.0, old, 0.0, 0.0, 0.0, 0.0]));
    }
    let [lo, hi] = carve_box(&mut rig, [16, 20, 4], [20, 24, 8]);
    let classes = rig.queue.classes(rig.world.materials());
    let faces = rig.queue.list(&rig.world, ChunkCoord(GROUND), &classes).0;
    let keys = &faces
        .bricks
        .iter()
        .find(|(s, _)| usize::from(*s) == kerb)
        .expect("the kerb's brick lists faces")
        .1;
    let kerb_x: Vec<[i32; 3]> = keys
        .iter()
        .filter(|&&k| k & 7 == 1)
        .map(|&k| {
            let v = i32::from(k >> 3);
            [8 + v % 8, 16 + (v / 8) % 8, 8 + v / 64]
        })
        .collect();
    assert!(!kerb_x.is_empty(), "the kerb's +x faces");
    assert!(
        kerb_x
            .iter()
            .all(|g| !(0..3).all(|k| (lo[k] - 1..=hi[k] + 1).contains(&g[k]))),
        "every +x face of the kerb's cell is kept"
    );
    assert!(
        8 <= hi[2] + 1 && 15 >= lo[0] - 1,
        "the box dilated by one voxel touches the kerb's brick: {lo:?} {hi:?}"
    );
    rig.step();
    let block = rig
        .store
        .block_words(ChunkCoord(GROUND))
        .expect("GROUND's block");
    let r = brick_record(block, kerb).expect("the kerb's record");
    let kept_x = record_visibility(r, 1);
    let old_byte = (old * 255.0).round() / 255.0;

    eprintln!(
        "whole dig: {} touched directions, {} with a kept face, {} all the dig's",
        rows.len(),
        mixed.len(),
        all_dug.len()
    );
    eprintln!("(1) kept-face directions with a held value: {moved:?}");
    eprintln!("(2) all-dig directions not held (None, not VIS_BAKED, or SKY_STALE): {unheld:?}");
    eprintln!("(3) the kerb cell's +x: {kept_x:?}, its old byte {old_byte}");
    // (1) a face the dig did not make reads no held value; (2) the dig's own directions hold a
    // VIS_BAKED value, not SKY_STALE; (3) a kept face keeps its old byte.
    let failed: Vec<u8> = [
        (1, moved.is_empty()),
        (2, unheld.is_empty()),
        (3, kept_x == Some(old_byte)),
    ]
    .into_iter()
    .filter(|&(_, ok)| !ok)
    .map(|(n, _)| n)
    .collect();
    assert!(
        failed.is_empty(),
        "failed clauses {failed:?} (printed above)"
    );
}
