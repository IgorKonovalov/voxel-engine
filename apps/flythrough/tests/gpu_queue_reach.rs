//! The live queue (ADR-0046's scheduler), the fly-through's light, its records: stepped headless on
//! the software adapter as the window's frame steps it (the mirror synced, then
//! `Frame::bake_queue_step` with no sample count, so each step is sized by its slice and waits on
//! nothing), its store read back against one-shot bakes of the same world.
//!
//! Every world here is a few hand-built boxes at test counts (64 samples a brick direction, a
//! coarse pass of 16); the rigs are `support::queue`'s.
//!
//! This binary reads the records and the step: the fades, the reach, eviction, digs and emitters, a
//! full pool, the step's stages and the flight's opening order.

mod support;

use support::SyncNow;
use support::queue::*;

use flythrough::cave::{ORE, ROCK};
use std::collections::{BTreeMap, BTreeSet};
use vxe_render::Window;
use vxe_render::{BakeSettings, QueueStats};
use vxe_world::{AIR, ChunkCoord, Mode, World};

/// The record words of `block` that hold a direction's value: each surface brick's six
/// direction words.
fn value_words(block: &[u32]) -> impl Iterator<Item = usize> + '_ {
    block[..512].iter().filter(|&&d| d != 0).flat_map(|&d| {
        let end = (d >> 11) as usize;
        end - vxe_render::internals::BRICK_WORDS..end - vxe_render::internals::BRICK_WORDS + 6
    })
}

/// The `rgb9e5` step of word `w`'s exponent.
fn step_of(w: u32) -> f32 {
    2f32.powi((w >> 27) as i32 - 24)
}

/// A record fading from `from` to `to` over `frames` steps, shown each step: every value word
/// moves by at most `|to - from| / frames` a step plus one `rgb9e5` step, and the last shows `to`.
fn assert_fade(what: &str, from: &[u32], shown: &[Vec<u32>], to: &[u32], frames: u32) {
    assert!(
        shown.len() <= frames as usize,
        "{what}: {} steps to reach the target, FADE {frames}",
        shown.len()
    );
    assert_eq!(
        shown.last().map(Vec::as_slice),
        Some(to),
        "{what}: ends at the target"
    );
    let mut prev = from;
    for (k, now) in shown.iter().enumerate() {
        for w in value_words(to) {
            let (a, b) = (
                vxe_render::internals::rgb9e5_unpack(prev[w]),
                vxe_render::internals::rgb9e5_unpack(now[w]),
            );
            let (x, y) = (
                vxe_render::internals::rgb9e5_unpack(from[w]),
                vxe_render::internals::rgb9e5_unpack(to[w]),
            );
            let ulp = step_of(prev[w]).max(step_of(now[w]));
            for ch in 0..3 {
                let share = (y[ch] - x[ch]).abs() / frames as f32;
                assert!(
                    (b[ch] - a[ch]).abs() <= share + ulp * 1.0001,
                    "{what}, step {k}, word {w} channel {ch}: moved {} for a share {share} and a step {ulp}",
                    (b[ch] - a[ch]).abs()
                );
            }
        }
        prev = now;
    }
}

/// A fade as [`record_fades`] saw it: the block shown before it, each step's, and the target.
type Fade = (Vec<u32>, Vec<Vec<u32>>, Vec<u32>);

/// Steps the queue until it is empty, recording `c`'s pool block each step from the step its
/// target changes: each fade's `from` (the block shown the step before), the blocks shown and
/// its target.
fn record_fades(q: &mut Q, c: [i32; 3]) -> Vec<Fade> {
    let mut fades = Vec::new();
    let mut shown_before = q.shown(c);
    let mut target_before = q.target(c);
    let mut open: Option<Fade> = None;
    for _ in 0..MAX_STEPS {
        q.step();
        let (shown, target) = (q.shown(c), q.target(c));
        if target != target_before {
            if let Some(f) = open.take() {
                fades.push(f);
            }
            if let (Some(from), Some(s), Some(t)) = (&shown_before, &shown, &target)
                && from.len() == t.len()
                && from[..512] == t[..512]
                && s != t
            {
                open = Some((from.clone(), Vec::new(), t.clone()));
            }
        }
        if let (Some(f), Some(s)) = (open.as_mut(), &shown) {
            f.1.push(s.clone());
            if Some(s) == target.as_ref() {
                fades.push(open.take().expect("open"));
            }
        }
        shown_before = shown;
        target_before = target;
        if q.q().empty() && open.is_none() {
            break;
        }
    }
    assert!(open.is_none(), "a fade still runs when the queue is empty");
    fades
}

/// The coarse block's refinement fades over `FADE` steps, and a dig's job (from the dig's
/// provisional block) over `DIG_FADE`: each record moves at most its share a step plus one
/// `rgb9e5` step, and ends at the new block.
#[test]
fn a_fade_moves_a_record_by_its_share_a_frame() {
    let (world, eye) = border_box();
    let Some(mut q) = rig(
        world,
        &Setup {
            coarse: COARSE,
            near: 0,
            ..Setup::at(eye)
        },
    ) else {
        return;
    };
    let refined = record_fades(&mut q, [1, 0, 0]);
    assert_eq!(refined.len(), 1, "chunk 1's coarse block is refined once");
    let (from, shown, to) = &refined[0];
    assert!(
        value_words(to).any(|w| from[w] != to[w]),
        "the refinement changes a record"
    );
    assert_fade(
        "the refinement",
        from,
        shown,
        to,
        vxe_render::internals::FADE,
    );
    // A carve in the lit top of the box, inside chunk 1.
    q.edit(&box_op([70, 26, 37], [74, 30, 40], Mode::Carve));
    let dug = record_fades(&mut q, [1, 0, 0]);
    assert!(!dug.is_empty(), "the dig's job fades in");
    for (k, (from, shown, to)) in dug.iter().enumerate() {
        assert_fade(
            &format!("dig fade {k}"),
            from,
            shown,
            to,
            vxe_render::internals::DIG_FADE,
        );
    }
}

/// ADR-0053's arrival. Chunk 0 holds a small rock slab under an ore cube, chunk 1 a large rock
/// floor (at 4096 samples a brick direction, many submits' work), so chunk 0's block lands
/// first (asserted). Under `chain`, chunk 1's first block fades from the chain's value at its
/// landing, chunk 0's summary through its face neighbour: on the step it lands, every value word
/// of its records shows `fallback + (block - fallback) / FADE` within one `rgb9e5` step, not the
/// block's value. Under `black` that step shows the block itself.
#[test]
fn a_first_block_fades_from_its_fallback() {
    use vxe_render::{MissingLight, debug::LightSummaries};
    for mode in [MissingLight::Chain, MissingLight::Black] {
        let world = world_of([2, 1, 1], AIR);
        fill(&world, [16, 12, 16], [28, 16, 28], ROCK);
        fill(&world, [20, 22, 20], [24, 26, 24], ORE);
        fill(&world, [68, 12, 4], [124, 16, 60], ROCK);
        let Some(mut q) = rig(
            world,
            &Setup {
                counts: BakeSettings {
                    brick_samples: 4096,
                    ..counts()
                },
                ..Setup::at([10.5, 30.5, 22.5])
            },
        ) else {
            return;
        };
        q.frame.set_missing_light(&q.queue, mode);
        let mut landed = false;
        for _ in 0..MAX_STEPS {
            let before = q.target([0, 0, 0]);
            q.step();
            if q.target([1, 0, 0]).is_some() {
                assert!(
                    before.is_some(),
                    "{mode:?}: chunk 0's block lands before chunk 1's (the rig's premise)"
                );
                landed = true;
                break;
            }
        }
        assert!(landed, "{mode:?}: chunk 1's block lands");
        let to = q.target([1, 0, 0]).expect("chunk 1's target");
        let shown = q.shown([1, 0, 0]).expect("chunk 1's pool block");
        if mode == MissingLight::Black {
            assert_eq!(shown, to, "black: the first block is written directly");
            continue;
        }
        let a0 = *q
            .frame
            .store()
            .light()
            .summary([0, 0, 0])
            .expect("chunk 0's summary");
        let mut only_a0 = LightSummaries::default();
        only_a0.set([0, 0, 0], Some(a0));
        let fallback = only_a0
            .chain([1, 0, 0], mode)
            .expect("the chain has chunk 0's value");
        let (mut checked, mut differ, mut moved) = (0, 0, 0);
        for &d in &to[..512] {
            if d == 0 {
                continue;
            }
            let start = (d >> 11) as usize - vxe_render::internals::BRICK_WORDS;
            for f in 0..6 {
                let w = start + f;
                let from = if fallback[6] >> (16 + f) & 1 != 0 {
                    vxe_render::internals::rgb9e5_unpack(fallback[f])
                } else {
                    [0.0; 3]
                };
                let target = vxe_render::internals::rgb9e5_unpack(to[w]);
                let got = vxe_render::internals::rgb9e5_unpack(shown[w]);
                let ulp = step_of(shown[w]);
                for k in 0..3 {
                    let want = from[k] + (target[k] - from[k]) / vxe_render::internals::FADE as f32;
                    assert!(
                        (got[k] - want).abs() <= ulp * 1.0001 + want.abs() * 1e-6,
                        "word {w} channel {k}: {} against {want} (fallback {}, block {})",
                        got[k],
                        from[k],
                        target[k]
                    );
                }
                checked += 1;
                // A word whose fallback and block differ: the fade has not reached the block.
                if from != target {
                    differ += 1;
                    moved += usize::from(shown[w] != to[w]);
                }
            }
        }
        assert!(checked > 100, "{checked} value words");
        assert!(
            differ > 20,
            "{differ} words whose fallback and block differ"
        );
        assert!(
            moved > differ / 2,
            "{moved} of {differ} differing words not yet at the block"
        );
    }
}

/// Chunk 0's wall faces an ore block in chunk 1 across their border. With the mirror taking one
/// chunk a sync, nearest the focus first, chunk 0 is mirrored and chunk 1 waits: chunk 0 is not
/// baked (its reach holds a chunk waiting to upload) and is held as blocked. Once chunk 1 is
/// mirrored, both bake, bit-identical to a one-shot bake with both present from the start, and
/// chunk 0's wall is lit.
#[test]
fn a_chunk_waits_for_its_reach() {
    let world = world_of([2, 1, 1], AIR);
    fill(&world, [40, 8, 8], [48, 56, 56], ROCK);
    fill(&world, [70, 28, 28], [74, 36, 36], ORE);
    let Some(mut q) = rig(
        world,
        &Setup {
            sync_budget: 1,
            ..Setup::at([20.5, 32.5, 32.5])
        },
    ) else {
        return;
    };
    let (c0, c1) = (ChunkCoord([0, 0, 0]), ChunkCoord([1, 0, 0]));
    assert!(
        q.gpu.is_mirrored(c0) && q.gpu.is_pending(c1),
        "one chunk a sync"
    );
    for _ in 0..40 {
        q.step_with(false);
    }
    assert!(q.gpu.is_pending(c1));
    assert_eq!(
        q.frame.store().block(c0),
        None,
        "chunk 0 waits for its reach"
    );
    assert_eq!(q.q().tracks(c0), ["blocked"]);
    let n = q.settle();
    println!("settled {n} steps after chunk 1 arrived");
    let got = q.blocks();
    assert_eq!(got.len(), 2);
    assert_eq!(got, q.fresh(), "against a one-shot bake with both present");
    let lit = value_words(&got[&c0.0])
        .filter(|&w| got[&c0.0][w] != 0)
        .count();
    assert!(lit > 0, "chunk 0's wall is lit by chunk 1's ore");
}

/// A 3 x 3 x 3 world at light radius 1, the camera in the centre chunk: the centre's rock wall
/// faces an ore block in chunk (2, 1, 1), which is LOD-only, as a streamer leaves a chunk past
/// its full radius (0 chunks here, so the centre's reach crosses it). Under `full` the centre
/// waits over 50 steps; under `partial` it bakes from its resident reach, flagged partial.
/// Loading the neighbour full re-queues it as a refinement: settled, its block equals a
/// one-shot bake of the whole rig, its wall is lit by the ore, and its flag is gone.
#[test]
fn a_partial_reach_chunk_bakes_and_is_rebaked_when_its_reach_arrives() {
    fn but_lod(c: ChunkCoord) -> bool {
        c != ChunkCoord([2, 1, 1])
    }
    fn every(_: ChunkCoord) -> bool {
        true
    }
    let scene = || {
        let world = world_of([3, 3, 3], AIR);
        fill(&world, [104, 72, 72], [112, 120, 120], ROCK);
        fill(&world, [134, 92, 92], [138, 100, 100], ORE);
        world
    };
    let (centre, lod) = (ChunkCoord([1, 1, 1]), ChunkCoord([2, 1, 1]));
    let world = scene();
    support::make_lod(&world, |c| c == lod);
    let Some(mut q) = rig(
        world,
        &Setup {
            light_radius: 1,
            ..Setup::at([96.5, 96.5, 96.5])
        },
    ) else {
        return;
    };
    q.resident = Some(but_lod);
    q.full_radius = Some(0);
    assert!(q.gpu.is_lod(lod), "the neighbour is LOD-only");
    assert!(q.gpu.is_mirrored(centre) && !q.gpu.is_lod(centre));
    for _ in 0..50 {
        q.step();
    }
    assert_eq!(
        q.frame.store().block(centre),
        None,
        "under full the centre waits for its reach"
    );
    assert_eq!(q.q().stats().partial_opened, 0);
    q.frame.set_bake_reach(vxe_render::ReachMode::Partial);
    let n = q.settle();
    println!("partial: settled in {n} steps, {:?}", q.q().stats());
    assert!(
        q.frame.store().block(centre).is_some(),
        "under partial the centre bakes"
    );
    assert!(
        q.q().tracks(centre).contains(&"partial"),
        "flagged partial: {:?}",
        q.q().tracks(centre)
    );
    let requeued = q.q().stats().partial_requeued;
    // The neighbour arrives full.
    let full = scene()
        .store()
        .remove(lod)
        .expect("the twin holds the chunk");
    let full = full.read().expect("the chunk's lock").clone();
    q.world.remove_lod(lod);
    q.world.store().insert(lod, full);
    q.world.store().mark_dirty(lod);
    q.resident = Some(every);
    q.step();
    assert!(
        q.gpu.is_mirrored(lod) && !q.gpu.is_lod(lod),
        "the neighbour is full"
    );
    assert!(
        q.q().stats().partial_requeued > requeued,
        "the arrival re-queues the centre: {:?}",
        q.q().stats()
    );
    let n = q.settle();
    println!("re-queued: settled in {n} steps, {:?}", q.q().stats());
    assert!(
        !q.q().tracks(centre).contains(&"partial"),
        "its reach is whole: {:?}",
        q.q().tracks(centre)
    );
    let got = q.blocks();
    let fresh = q.fresh();
    assert_eq!(
        got.get(&centre.0),
        fresh.get(&centre.0),
        "against a one-shot bake of the whole rig"
    );
    let block = &got[&centre.0];
    let lit = value_words(block).filter(|&w| block[w] != 0).count();
    assert!(lit > 0, "the centre's wall is lit by the ore");
}

/// A window of 3 chunks following the focus over four chunks in a row (chunk 3 air): with the
/// focus at chunk 1 the queue bakes chunks 0 to 2. Moving the focus to chunk 2 drops chunk 0
/// from the mirror: its block is gone, no per-chunk set of the queue holds it, chunk 3 (air)
/// is known faceless, and once the queue is empty every pool word is a block's, the header's or
/// free, so chunk 0's place was not leaked. Moving back to chunk 1 drops chunk 3 the same way.
#[test]
fn an_evicted_chunk_frees_its_block_and_its_bookkeeping() {
    let world = world_of([4, 1, 1], AIR);
    for x in 0..3 {
        fill(&world, [x * 64 + 16, 16, 16], [x * 64 + 48, 48, 48], ROCK);
    }
    fill(&world, [80, 28, 48], [90, 36, 50], ORE);
    let Some(mut q) = rig(
        world,
        &Setup {
            window: Some(Window::Follow { side: 3 }),
            ..Setup::at([96.5, 32.5, 32.5])
        },
    ) else {
        return;
    };
    let all: Vec<[i32; 3]> = (-1..5).map(|x| [x, 0, 0]).collect();
    let accounted = |q: &Q| {
        let store = q.frame.store();
        let used = store.stats(0, 0, 0).bytes / 4;
        assert_eq!(
            used + store.free_words(),
            store.pool().size() / 4,
            "every pool word is the header's, a block's or free"
        );
    };
    q.settle();
    assert!(q.frame.store().block(ChunkCoord([0, 0, 0])).is_some());
    accounted(&q);
    q.camera.position.x = 160.5;
    q.step();
    let gone = ChunkCoord([0, 0, 0]);
    assert!(!q.gpu.is_mirrored(gone));
    assert_eq!(
        q.frame.store().block(gone),
        None,
        "the evicted block is dropped"
    );
    q.settle();
    assert_eq!(
        q.q().tracks(gone),
        Vec::<&str>::new(),
        "chunk 0 left behind"
    );
    assert_eq!(q.q().tracks(ChunkCoord([3, 0, 0])), ["faceless"]);
    q.assert_places_apart(&all);
    accounted(&q);
    q.camera.position.x = 96.5;
    q.settle();
    let gone = ChunkCoord([3, 0, 0]);
    assert!(!q.gpu.is_mirrored(gone));
    assert_eq!(
        q.q().tracks(gone),
        Vec::<&str>::new(),
        "chunk 3 left behind"
    );
    assert!(
        q.frame.store().block(ChunkCoord([0, 0, 0])).is_some(),
        "chunk 0 baked again"
    );
    q.assert_places_apart(&all);
    accounted(&q);
}

// ---- Digs on the queue: rings 1 to 3 ----

/// A rock bar of two chunks with a tunnel along x: an ore block at its west end, a plug at
/// x 30..32, and the tunnel's far end past x 72. The camera sits in the tunnel west of the plug.
fn tunnel() -> (World, [f32; 3]) {
    let world = world_of([2, 1, 1], ROCK);
    world
        .apply(&box_op([8, 28, 28], [110, 34, 34], Mode::Carve))
        .expect("the tunnel");
    fill(&world, [10, 30, 30], [12, 32, 32], ORE);
    fill(&world, [30, 28, 28], [32, 34, 34], ROCK);
    (world, [20.5, 31.5, 31.5])
}

/// The records of bricks at world brick x 9 and beyond (voxel x from 72): the tunnel's far end.
fn far_end(blocks: &BTreeMap<[i32; 3], Vec<u32>>) -> BTreeMap<([i32; 3], usize), Vec<u32>> {
    let mut out = BTreeMap::new();
    for (&c, block) in blocks {
        for (slot, &d) in block[..512].iter().enumerate() {
            let b = vxe_world::brick_of_slot(slot);
            if d == 0 || c[0] * 8 + b[0] < 9 {
                continue;
            }
            let end = (d >> 11) as usize;
            out.insert(
                (c, slot),
                block[end - vxe_render::internals::BRICK_WORDS..end].to_vec(),
            );
        }
    }
    out
}

/// The tunnel's far end, dark behind the plug. A window carved in the plug (a lit brick
/// touched, so ring 3 re-bakes the chunks within the light radius): once the queue is empty
/// the far end's records equal a one-shot bake's of the opened world, and some are lit. The
/// plug put back: once the queue is empty they equal the first bake's again.
#[test]
fn a_break_through_reaches_the_far_wall() {
    let (world, eye) = tunnel();
    let Some(mut q) = rig(world, &Setup::at(eye)) else {
        return;
    };
    q.settle();
    let dark = far_end(&q.blocks());
    assert!(dark.len() >= 20, "{} far bricks", dark.len());
    assert_eq!(
        dark,
        far_end(&q.fresh()),
        "the first bake against a one-shot bake"
    );
    q.edit(&box_op([30, 30, 30], [32, 32, 32], Mode::Carve));
    let mut reach = 0;
    while !q.q().empty() {
        reach = reach.max(q.step().reach);
    }
    assert!(reach > 0, "the window opens ring 3");
    let open = far_end(&q.blocks());
    assert_eq!(
        open,
        far_end(&q.fresh()),
        "the far end against a one-shot bake"
    );
    let lit = open
        .values()
        .filter(|r| r[..6].iter().any(|&w| w != 0))
        .count();
    let lit_before = dark
        .values()
        .filter(|r| r[..6].iter().any(|&w| w != 0))
        .count();
    println!(
        "far bricks {}, lit {lit_before} before, {lit} through the window",
        open.len()
    );
    assert!(lit > lit_before, "the window lights the far end");
    q.edit(&box_op([30, 30, 30], [32, 32, 32], Mode::Fill(ROCK)));
    q.settle();
    assert_eq!(far_end(&q.blocks()), dark, "the plug put back");
}

/// A carve in the tunnel's dark far end, which no emitter reaches past the plug: the queue
/// re-bakes rings 1 and 2 and opens no ring-3 chunk, and its blocks equal a one-shot bake's.
#[test]
fn a_dark_carve_opens_no_reach() {
    let (world, eye) = tunnel();
    let Some(mut q) = rig(world, &Setup::at(eye)) else {
        return;
    };
    q.settle();
    let before = q.frame.store().block(ChunkCoord([1, 0, 0]));
    q.edit(&box_op([100, 30, 30], [104, 34, 37], Mode::Carve));
    let (mut digs, mut reach, mut baked) = (0, 0, BTreeSet::new());
    while !q.q().empty() {
        let st = q.step();
        digs = digs.max(st.digs);
        reach = reach.max(st.reach);
        baked.extend(q.q().jobs().into_iter().map(|c| c.0));
        assert!(baked.len() < 1000);
    }
    println!("digs {digs}, reach {reach}, jobs {baked:?}, chunk 1 at {before:?}");
    assert!(digs > 0, "the carve's rings re-bake");
    assert_eq!(reach, 0, "a dark carve opens no ring 3");
    assert_eq!(q.blocks(), q.fresh(), "against a one-shot bake");
}

/// A rock slab under one ore block, in one chunk. Once the queue is empty the slab's top is
/// lit; the ore carved away, once the queue is empty every record of the chunk reads 0.
#[test]
fn an_emitter_dug_away_darkens_what_it_lit() {
    let world = world_of([1, 1, 1], AIR);
    fill(&world, [8, 8, 20], [40, 40, 21], ROCK);
    fill(&world, [22, 22, 30], [24, 24, 32], ORE);
    let Some(mut q) = rig(world, &Setup::at([30.5, 30.5, 50.5])) else {
        return;
    };
    q.settle();
    let block = &q.blocks()[&[0, 0, 0]];
    let lit = value_words(block).filter(|&w| block[w] != 0).count();
    // The slab's 16 bricks face the ore with their +z.
    assert!(lit >= 16, "{lit} lit directions");
    q.edit(&box_op([22, 22, 30], [24, 24, 32], Mode::Carve));
    q.settle();
    let block = &q.blocks()[&[0, 0, 0]];
    let still: Vec<usize> = value_words(block).filter(|&w| block[w] != 0).collect();
    assert!(still.is_empty(), "{} words still lit", still.len());
}

/// Four chunks in a row: rock blocks of 2 x 2 x 3 bricks in chunk 1, 3 x 3 x 3 in chunk 2 and
/// 2 x 2 x 2 in chunk 3, so chunk 1's block is a little larger than chunk 3's, and chunk 2's
/// grows when a carve opens its centre brick.
fn pool_world() -> World {
    let world = world_of([4, 1, 1], AIR);
    fill(&world, [80, 16, 16], [96, 32, 40], ROCK);
    fill(&world, [136, 16, 16], [160, 40, 40], ROCK);
    fill(&world, [208, 16, 16], [224, 32, 32], ROCK);
    world
}

/// [`pool_world`] under a window of 3 chunks following the focus, the pool sized so that, with
/// the camera at chunk 2, chunks 2 (full count) and 3 (coarse) take places and chunk 1's block
/// does not fit. While the pool refuses chunk 1 the queue goes on: it refines chunk 3 (a place
/// of chunk 3's size fits), reads it back and fades it, and no step fails. A carve into chunk
/// 2's centre brick asks for a larger provisional block, which the pool refuses: chunk 2 keeps
/// its block where the chunk table points and its dig waits. No two chunks ever hold the same
/// pool word. The camera moved to chunk 1 drops chunk 3, whose freed place merges with the room
/// left: the dig re-bakes and chunk 1 bakes.
#[test]
fn a_full_pool_keeps_the_queue_alive() {
    let eye = [168.5, 28.5, 56.5];
    let setup = Setup {
        window: Some(Window::Follow { side: 3 }),
        coarse: COARSE,
        near: 0,
        ..Setup::at(eye)
    };
    // The blocks' sizes, from a queue with room.
    let Some(mut probe) = rig(pool_world(), &setup) else {
        return;
    };
    let size = |q: &Q, x: i32| {
        q.frame
            .store()
            .block(ChunkCoord([x, 0, 0]))
            .expect("a block")
            .1
    };
    probe.settle();
    let (w2, w3) = (size(&probe, 2), size(&probe, 3));
    probe.camera.position.x = 96.5;
    probe.settle();
    let w1 = size(&probe, 1);
    let head = probe.frame.store().stats(0, 0, 0).bytes / 4 - w1 - w2;
    println!("words: header {head}, chunk 1 {w1}, 2 {w2}, 3 {w3}");
    drop(probe);
    let spare = w3 + 4;
    assert!(
        w1 > spare && w1 <= spare + w3,
        "chunk 1's block fits only with chunk 3's place"
    );
    assert!(w2 >= w1, "chunk 2's place would hold chunk 1's block");
    let Some(mut q) = rig(
        pool_world(),
        &Setup {
            pool_bytes: (head + w2 + w3 + spare) * 4,
            ..setup
        },
    ) else {
        return;
    };
    let all: Vec<[i32; 3]> = (0..5).map(|x| [x, 0, 0]).collect();
    let (c1, c2, c3) = (
        ChunkCoord([1, 0, 0]),
        ChunkCoord([2, 0, 0]),
        ChunkCoord([3, 0, 0]),
    );
    let (mut faded, mut st) = (0, QueueStats::default());
    for _ in 0..200 {
        st = q.step();
        q.assert_places_apart(&all);
        faded = faded.max(st.fading);
        if st.jobs == 0 && st.fading == 0 && !q.frame.store().is_coarse(c3) {
            break;
        }
    }
    println!(
        "refused {}, faded {faded}, baked {}",
        st.pool_full, st.baked
    );
    assert!(st.pool_full > 0, "the pool refuses chunk 1");
    assert_eq!(q.frame.store().block(c1), None);
    assert!(faded > 0, "chunk 3's refinement fades while chunk 1 waits");
    assert!(!q.frame.store().is_coarse(c3), "chunk 3 refined");
    assert_eq!(
        st.baked, 3,
        "chunk 2, chunk 3's coarse pass and its refinement"
    );
    let place = q.frame.store().block(c2);
    let refused = st.pool_full;
    q.edit(&box_op([146, 26, 30], [150, 30, 41], Mode::Carve));
    for _ in 0..20 {
        st = q.step();
        q.assert_places_apart(&all);
    }
    assert!(st.pool_full > refused, "the dig's larger block is refused");
    assert_eq!(st.digs, 1, "chunk 2's dig waits");
    assert_eq!(q.frame.store().block(c2), place, "chunk 2 keeps its block");
    q.camera.position.x = 96.5;
    for _ in 0..200 {
        st = q.step();
        q.assert_places_apart(&all);
        if q.frame.store().block(c1).is_some() && st.jobs == 0 && st.digs == 0 {
            break;
        }
    }
    assert_eq!(q.frame.store().block(c3), None, "chunk 3 evicted");
    assert_eq!(st.digs, 0, "the dig re-baked once chunk 3's place was free");
    assert!(
        q.frame.store().block(c1).is_some(),
        "chunk 1 bakes once the dig's old place is free"
    );
}

/// 20 steps of the row with the coarse pass on from the start, timed from outside each
/// `bake_queue_step` call: each step's stages and opening flag, its own time, and the queue's
/// openings before and after.
fn staged_steps() -> Option<(Vec<(vxe_render::debug::StepStages, f64)>, u64)> {
    let mut q = rig(
        row(&[0, 1, 2, 3, 4, 5]),
        &Setup {
            coarse: COARSE,
            ..Setup::at(ROW_EYE)
        },
    )?;
    let before = q.q().stats().openings;
    let mut steps = Vec::new();
    for _ in 0..20 {
        let st = q.step();
        steps.push((st.stages, q.last_step_ms));
    }
    let openings = q.q().stats().openings - before;
    Some((steps, openings))
}

/// Over 20 steps the stages' sum is within 5 % of the steps' own time (each call timed from
/// outside), and no stage reads negative.
#[test]
fn the_step_s_stages_sum_to_its_time() {
    let Some((steps, _)) = staged_steps() else {
        return;
    };
    let sum: f64 = steps.iter().map(|(s, _)| s.sum()).sum();
    let own: f64 = steps.iter().map(|(_, ms)| ms).sum();
    println!("stages {sum:.4} ms, own {own:.4} ms over 20 steps");
    for (s, _) in &steps {
        for (k, ms) in s.ms.iter().enumerate() {
            assert!(
                *ms >= 0.0,
                "{} reads {ms}",
                vxe_render::debug::STEP_STAGES[k]
            );
        }
    }
    assert!(
        (sum - own).abs() <= 0.05 * own,
        "stages {sum} ms against the steps' {own} ms"
    );
}

/// Over 20 steps of which some open and some do not, the opening and other counts sum to 20
/// and match the steps whose opening ran (the queue's count of openings), and each side's
/// stage sum is within 5 % of its steps' own time.
#[test]
fn the_stages_split_by_opening() {
    let Some((steps, openings)) = staged_steps() else {
        return;
    };
    let (open, not): (Vec<_>, Vec<_>) = steps.iter().partition(|(s, _)| s.opened);
    println!(
        "{} opened, {} not, {openings} openings",
        open.len(),
        not.len()
    );
    assert!(
        !open.is_empty() && !not.is_empty(),
        "the rig has both kinds of step"
    );
    assert_eq!(open.len() + not.len(), 20);
    assert_eq!(open.len() as u64, openings);
    for (what, side) in [("opening", &open), ("other", &not)] {
        let sum: f64 = side.iter().map(|(s, _)| s.sum()).sum();
        let own: f64 = side.iter().map(|(_, ms)| ms).sum();
        println!("{what}: stages {sum:.4} ms, own {own:.4} ms");
        assert!(
            (sum - own).abs() <= 0.05 * own,
            "{what}: stages {sum} ms against the steps' {own} ms"
        );
        assert!(
            side.iter().all(|(s, _)| s.opened == (what == "opening")),
            "{what}: the flag"
        );
    }
}

/// `n` chunks in a row along x, a rock block in each and an ore plate in chunk 2, mirrored one
/// chunk a sync nearest the camera (in chunk 2) first, `syncs` times: 7 syncs of 8, chunk 7
/// waits to upload and the chunks within the light radius of it (5 and 6) wait for their
/// reach.
fn row_n(n: i64, syncs: usize, setup: Setup) -> Option<Q> {
    let world = world_of([n as i32, 1, 1], AIR);
    for x in 0..n {
        fill(&world, [x * 64 + 16, 16, 16], [x * 64 + 48, 48, 48], ROCK);
    }
    fill(&world, [140, 28, 48], [150, 36, 50], ORE);
    let mut q = rig(
        world,
        &Setup {
            sync_budget: 1,
            ..setup
        },
    )?;
    for _ in 1..syncs {
        q.gpu
            .sync_now(&q.world, &q.device, &q.queue)
            .expect("the mirror fits");
    }
    Some(q)
}

/// Mirrors one chunk: one sync at the rig's budget.
fn sync_one(q: &mut Q) {
    q.gpu
        .sync_now(&q.world, &q.device, &q.queue)
        .expect("the mirror fits");
}

/// Nine chunks in a row, chunks 7 and 8 waiting to upload: once the queue has baked chunks 0
/// to 4 and checked 5 and 6, 100 further steps on a still mirror scan nothing, walk no reach
/// and score nothing. Evicting chunk 3 makes the next step scan exactly that chunk and free
/// its block, no other. Mirroring chunk 7 re-checks exactly the candidates within the light
/// radius and one of it, chunks 5, 6 and 7, each once: 5 bakes, 6 and 7 wait for chunk 8.
#[test]
fn a_still_step_scans_nothing() {
    let Some(mut q) = row_n(9, 7, Setup::at(ROW_EYE)) else {
        return;
    };
    let blocks = |q: &Q| -> Vec<i32> {
        (0..9)
            .filter(|&x| q.frame.store().block(ChunkCoord([x, 0, 0])).is_some())
            .collect()
    };
    for _ in 0..300 {
        q.step_with(false);
    }
    let st = q.q().stats();
    assert_eq!((st.jobs, st.fading), (0, 0), "drained: {st:?}");
    assert_eq!(
        blocks(&q),
        [0, 1, 2, 3, 4],
        "chunks 5 and 6 wait for their reach"
    );
    for n in 0..100 {
        let st = q.step_with(false);
        assert_eq!(
            (st.scanned, st.reach_checks, st.rebuilt),
            (0, 0, 0),
            "still step {n}: {st:?}"
        );
    }
    q.gpu.apply_events(&vxe_world::StreamEvents {
        evicted: vec![ChunkCoord([3, 0, 0])],
        ..Default::default()
    });
    let st = q.step_with(false);
    assert_eq!(st.scanned, 1, "the eviction's step scans its chunk");
    assert_eq!(
        blocks(&q),
        [0, 1, 2, 4],
        "chunk 3's block is freed, no other"
    );
    sync_one(&mut q);
    assert!(
        q.gpu.is_mirrored(ChunkCoord([7, 0, 0])),
        "chunk 7 is mirrored"
    );
    assert!(
        q.gpu.is_pending(ChunkCoord([8, 0, 0])),
        "chunk 8 still waits"
    );
    let mut checks = 0;
    for _ in 0..40 {
        checks += q.step_with(false).reach_checks;
    }
    assert_eq!(checks, 3, "chunks 5, 6 and 7 are checked again, once each");
    assert_eq!(blocks(&q), [0, 1, 2, 4, 5], "chunk 5 bakes");
}

/// The eight-chunk row with chunk 7 waiting to upload, at 1024 samples a brick direction and
/// the smallest slice: one job opens at a time, nearest first with the view's tie (2, 3, 1,
/// 4, 0), as the start commit opens them. Every opening after the first, the mirror still and
/// jobs ending between them, scores nothing again, and each candidate's reach is walked once
/// over the run (0 to 6).
/// Mirroring chunk 7 makes the next opening score once; 7 opens, then 5 and 6 once their
/// last check is old enough to be read again.
#[test]
fn an_opening_without_a_mirror_event_rebuilds_nothing() {
    let Some(mut q) = row_n(
        8,
        7,
        Setup {
            slice_ms: 1e-6,
            counts: BakeSettings {
                brick_samples: 1024,
                ..counts()
            },
            ..Setup::at(ROW_EYE)
        },
    ) else {
        return;
    };
    let mut order: Vec<i32> = Vec::new();
    let (mut checks, mut openings, mut rebuilt) = (0, 0, 0);
    let mut steps = 0;
    while !(q.q().jobs().is_empty() && order.len() == 5) {
        steps += 1;
        assert!(steps <= MAX_STEPS, "the five ready chunks bake");
        let before = q.q().stats().openings;
        let st = q.step_with(false);
        checks += st.reach_checks;
        if st.openings > before {
            openings += 1;
            rebuilt += st.rebuilt;
            assert!(
                openings == 1 || st.rebuilt == 0,
                "opening {openings} scores again: {st:?}"
            );
        }
        for c in q.q().jobs() {
            if !order.contains(&c.0[0]) {
                order.push(c.0[0]);
            }
        }
    }
    for _ in 0..40 {
        checks += q.step_with(false).reach_checks;
    }
    println!("order {order:?}, {openings} openings, {rebuilt} scored, {checks} reach walks");
    assert_eq!(order, [2, 3, 1, 4, 0]);
    assert!(openings >= 5, "one opening a job at least");
    assert_eq!(rebuilt, 1, "only the first opening scores");
    assert_eq!(checks, 7, "chunks 0 to 6 each walked once");
    sync_one(&mut q);
    let mut rebuilt = None;
    for _ in 0..20 {
        let before = q.q().stats().openings;
        let st = q.step_with(false);
        if st.openings > before {
            rebuilt = Some(st.rebuilt);
            break;
        }
    }
    assert_eq!(rebuilt, Some(1), "chunk 7's arrival scores the order once");
    for _ in 0..MAX_STEPS {
        q.step_with(false);
        for c in q.q().jobs() {
            if !order.contains(&c.0[0]) {
                order.push(c.0[0]);
            }
        }
        if q.q().empty() {
            break;
        }
    }
    // 5 and 6 were found blocked at the last openings and wait out the recheck: 7 first.
    assert_eq!(order, [2, 3, 1, 4, 0, 7, 5, 6]);
}

/// The order of a flight: the six-chunk row, the coarse pass on, the near shell 1 and the
/// smallest slice, the camera flown along +x from chunk 0 six voxels a step for 60 steps,
/// then held until the queue is empty. Each job's chunk is recorded as it first appears in
/// the jobs (a refinement appears again), against the start commit's recording.
#[test]
fn a_flight_opens_in_the_start_commit_s_order() {
    let Some(mut q) = rig(
        row(&[0, 1, 2, 3, 4, 5]),
        &Setup {
            coarse: COARSE,
            near: 1,
            slice_ms: 1e-6,
            ..Setup::at([20.5, 32.5, 60.5])
        },
    ) else {
        return;
    };
    let mut seen: Vec<i32> = Vec::new();
    let mut prev: Vec<ChunkCoord> = Vec::new();
    for n in 0..MAX_STEPS {
        if n < 60 {
            q.camera.position.x += 6.0;
        }
        q.step();
        let jobs = q.q().jobs();
        for c in &jobs {
            if !prev.contains(c) {
                seen.push(c.0[0]);
            }
        }
        prev = jobs;
        if n >= 60 && q.q().empty() {
            break;
        }
    }
    println!("flight order {seen:?}");
    assert_eq!(seen, FLIGHT_ORDER);
}

/// [`a_flight_opens_in_the_start_commit_s_order`]'s recording at the start commit (4bc8100).
const FLIGHT_ORDER: [i32; 10] = [0, 1, 2, 3, 4, 5, 2, 3, 4, 5];

/// The samples of `c`'s job at `brick_samples` a brick direction: its listed bricks' distinct
/// directions times the count, as the queue sizes a job.
fn job_samples(world: &World, c: [i32; 3], brick_samples: u32) -> u64 {
    let classes = vxe_render::internals::face_classes(world.materials());
    let faces = vxe_render::internals::list_world_chunk(world, ChunkCoord(c), &classes)
        .expect("the chunk is held");
    faces
        .bricks
        .iter()
        .map(|(_, keys)| {
            let mut dirs = [false; 6];
            for k in keys {
                dirs[usize::from(k & 7)] = true;
            }
            dirs.iter().filter(|d| **d).count() as u64 * u64::from(brick_samples)
        })
        .sum()
}

/// A slab of `n` x `w` chunks, a 32^3 rock cube in each and an ore patch on chunk 0's.
fn rock_slab(n: i32, w: i32) -> World {
    let world = world_of([n, w, 1], AIR);
    for x in 0..i64::from(n) {
        for y in 0..i64::from(w) {
            fill(
                &world,
                [x * 64 + 16, y * 64 + 16, 16],
                [x * 64 + 48, y * 64 + 48, 48],
                ROCK,
            );
        }
    }
    fill(&world, [24, 28, 48], [34, 36, 50], ORE);
    world
}

/// A row of `n` chunks, a 32^3 rock cube in each and an ore patch on chunk 0's.
fn rock_row(n: i32) -> World {
    rock_slab(n, 1)
}

/// Steps between two advances of [`a_starved_queue_recovers`]'s camera.
const ADVANCE_STEPS: usize = 5;

/// A slab of 24 x 5 chunks of rock cubes at 1024 samples a brick direction and a coarse pass of 64 (the
/// live queue's 4096 and 256 scaled down by the same 16), the smallest slice (a step draws the
/// cap's floor, 16 384 samples), `near` and the full-first radius 2. The camera starts in chunk
/// [0, 2, 0] looking +x and advances one chunk every [`ADVANCE_STEPS`] steps; the test asserts
/// that a full job is over two steps' samples, the five chunks entering the near shell at an
/// advance over the advance's at full and under half of it at coarse, and a coarse job under
/// one step's. Under `Some(2)`, after 10
/// advances the near shell holds more chunks with no block than after the first, the last
/// five advances open no coarse job and some opening is counted starved. Then, from that state
/// under `None` (the camera held still until the jobs the loop held at the switch end, since
/// the coarse jobs are planned behind them): the near shell's blank count falls then and at
/// every advance until 0 and stays 0 for the rest, every opening while a blank candidate waits pushes a coarse job, and no opening
/// is counted starved.
#[test]
fn a_starved_queue_recovers() {
    const CAP: u64 = 16_384;
    let Some(mut q) = rig(
        rock_slab(24, 5),
        &Setup {
            coarse: 64,
            near: 2,
            slice_ms: 1e-6,
            pool_bytes: 32 << 20,
            counts: BakeSettings {
                brick_samples: 1024,
                ..counts()
            },
            ..Setup::at([32.5, 160.5, 60.5])
        },
    ) else {
        return;
    };
    let full = job_samples(&q.world, [3, 0, 0], 1024);
    let coarse = job_samples(&q.world, [3, 0, 0], 64);
    let k = ADVANCE_STEPS as u64;
    println!("a job: full {full}, coarse {coarse} samples; a step {CAP}");
    assert!(full > 2 * CAP, "a full job outlasts two steps");
    assert!(coarse < CAP, "a coarse job fits a step");
    assert!(
        5 * full > k * CAP && 2 * 5 * coarse < k * CAP,
        "an advance's entering chunks outlast it at full and take under half of it at coarse"
    );
    let shell_blank = |q: &Q| {
        let cx = (q.camera.position.x / 64.0).floor() as i32;
        (cx - 2..=cx + 2)
            .filter(|x| (0..24).contains(x))
            .flat_map(|x| (0..5).map(move |y| ChunkCoord([x, y, 0])))
            .filter(|&c| q.frame.store().block(c).is_none())
            .count()
    };
    // A blank candidate: a chunk with no block and no job.
    let blank_waiting = |q: &Q| {
        let jobs = q.q().jobs();
        (0..24)
            .flat_map(|x| (0..5).map(move |y| ChunkCoord([x, y, 0])))
            .any(|c| q.frame.store().block(c).is_none() && !jobs.contains(&c))
    };
    let mut first = None;
    let mut coarse_late = 0;
    for a in 1..=10 {
        q.camera.position.x += 64.0;
        let before = q.q().stats().coarse_opened;
        for _ in 0..ADVANCE_STEPS {
            q.step_with(false);
        }
        if a > 5 {
            coarse_late += q.q().stats().coarse_opened - before;
        }
        let n = shell_blank(&q);
        println!("full-first 2, advance {a}: {n} blank in the shell");
        first.get_or_insert(n);
    }
    let starved = shell_blank(&q);
    let st = q.q().stats();
    assert!(
        starved > first.unwrap_or(0),
        "the shell's blank count grows"
    );
    assert_eq!(coarse_late, 0, "the last five advances open no coarse job");
    assert!(
        st.starved_openings > 0,
        "an opening is counted starved: {st:?}"
    );
    // The switch to `None`, the camera still until the jobs the loop held at the switch
    // (pushed under `Some(2)`, coarse jobs planned behind them) have ended.
    q.frame.set_bake_full_first(None);
    let held = q.q().jobs();
    for n in 0.. {
        assert!(n < MAX_STEPS, "the jobs held at the switch end");
        if !q.q().jobs().iter().any(|c| held.contains(c)) {
            break;
        }
        q.step_with(false);
    }
    let after_switch = shell_blank(&q);
    println!("full-first off, the switch's jobs ended: {after_switch} blank in the shell");
    assert!(
        after_switch < starved,
        "the shell's blank count falls after the switch"
    );
    let (starved_before, mut last) = (st.starved_openings, after_switch);
    let mut zeros = 0;
    for a in 1..=10 {
        q.camera.position.x += 64.0;
        for s in 0..ADVANCE_STEPS {
            let waiting = blank_waiting(&q);
            let before = q.q().stats();
            let after = q.step_with(false);
            if waiting && after.openings > before.openings {
                assert!(
                    after.coarse_opened > before.coarse_opened,
                    "off, advance {a} step {s}: an opening with a blank candidate waiting pushes a coarse job: {after:?}"
                );
            }
        }
        let n = shell_blank(&q);
        println!("full-first off, advance {a}: {n} blank in the shell");
        if last > 0 {
            assert!(
                n < last,
                "off, advance {a}: the shell's blank count falls ({last} to {n})"
            );
        } else {
            assert_eq!(
                n, 0,
                "off, advance {a}: the shell stays without a blank chunk"
            );
        }
        zeros += usize::from(n == 0);
        last = n;
    }
    assert!(
        zeros >= 5,
        "the shell holds no blank chunk for the later advances"
    );
    assert_eq!(
        q.q().stats().starved_openings,
        starved_before,
        "no opening is starved under off"
    );
}

/// An eight-chunk row of rock cubes, the camera in chunk 2 looking +x and still, the coarse
/// pass on, `near` 2: the order in which the chunks take their first job is the same under the
/// full-first radius `Some(2)` and `None`; under `Some(2)` it, and which chunks' first block is
/// coarse (0 to 4 full, 5 to 7 coarse), equal the start commit's recording
/// ([`FULL_FIRST_ORDER`]); under `None` every first block is coarse.
#[test]
fn the_full_first_radius_leaves_the_order_alone() {
    let mut seen = Vec::new();
    for r in [Some(2), None] {
        let Some(mut q) = rig(
            rock_row(8),
            &Setup {
                coarse: COARSE,
                near: 2,
                slice_ms: 1e-6,
                ..Setup::at(ROW_EYE)
            },
        ) else {
            return;
        };
        q.frame.set_bake_full_first(r);
        let mut order: Vec<i32> = Vec::new();
        let mut first_coarse: BTreeMap<i32, bool> = BTreeMap::new();
        for _ in 0..MAX_STEPS {
            q.step_with(false);
            for c in q.q().jobs() {
                if !order.contains(&c.0[0]) {
                    order.push(c.0[0]);
                }
            }
            for x in 0..8 {
                let c = ChunkCoord([x, 0, 0]);
                if q.frame.store().block(c).is_some() {
                    first_coarse
                        .entry(x)
                        .or_insert_with(|| q.frame.store().is_coarse(c));
                }
            }
            if first_coarse.len() == 8 {
                break;
            }
        }
        let coarse: Vec<bool> = first_coarse.values().copied().collect();
        println!("full-first {r:?}: order {order:?}, first block coarse {coarse:?}");
        seen.push((order, coarse));
    }
    assert_eq!(
        seen[0].0, seen[1].0,
        "the same order under Some(2) and None"
    );
    assert_eq!(
        (seen[0].0.as_slice(), seen[0].1.as_slice()),
        (FULL_FIRST_ORDER.0.as_slice(), FULL_FIRST_ORDER.1.as_slice()),
        "Some(2) against the start commit"
    );
    assert!(
        seen[1].1.iter().all(|&c| c),
        "every first block coarse under None"
    );
}

/// [`the_full_first_radius_leaves_the_order_alone`]'s recording at the start commit (4bc8100):
/// the order of first jobs and whether each chunk's first block was coarse.
const FULL_FIRST_ORDER: ([i32; 8], [bool; 8]) = (
    [2, 3, 1, 4, 0, 5, 6, 7],
    [false, false, false, false, false, true, true, true],
);
