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
