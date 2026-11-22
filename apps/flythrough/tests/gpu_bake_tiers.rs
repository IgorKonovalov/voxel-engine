//! The live queue's tiers by size on screen (ADR-0080 rules 2 and 4) on the software adapter: a
//! far chunk's record at the far tier's count and its promotion as the camera comes near, the
//! tier's bytes against two runs and two slicings, a far chunk's partial record cast from a
//! LOD-only chunk's mips, and a far dig's ring at the near tier.
//!
//! Every world here is a row of hand-built boxes at test counts (tiers 64, 32 and 16 samples a
//! brick direction, no coarse pass); the rigs are `support::queue`'s, the view's focal length
//! the fly-through's at 1080p (70 degrees vertical) and the far-normal band `[2, 6]` px a voxel.

mod support;

use support::queue::*;

use flythrough::cave::{ORE, ROCK};
use vxe_render::LookParams;
use vxe_render::internals::FADE;
use vxe_world::{AIR, ChunkCoord, World};

/// The test tiers a brick direction: near, mid, far.
const TIERS: [u32; 3] = [64, 32, 16];

/// The fly-through's focal length at 1080p under a 70-degree vertical field.
fn focal_1080() -> f32 {
    540.0 / 35f32.to_radians().tan()
}

/// `rig` with the bands on: the focal length at 1080p, the far-normal band `[2, 6]`, [`TIERS`].
fn tiered(world: World, setup: &Setup) -> Option<Q> {
    let mut q = rig(world, setup)?;
    q.focal_px = Some(focal_1080());
    q.frame.set_look(LookParams {
        far_normal: [2.0, 6.0],
        ..LookParams::default()
    });
    q.frame.set_bake_tiers(TIERS);
    Some(q)
}

/// The tier bits of every brick record of `block` (`record_tier`), each once.
fn tiers_of(block: &[u32]) -> Vec<u8> {
    let w = vxe_render::internals::BRICK_WORDS;
    let mut t: Vec<u8> = block[..512]
        .iter()
        .filter(|&&d| d != 0)
        .map(|&d| {
            let end = (d >> 11) as usize;
            vxe_render::internals::record_tier(&block[end - w..end])
        })
        .collect();
    t.sort_unstable();
    t.dedup();
    t
}

/// The sum of the luminance of `block`'s direction values (each surface brick's six words).
fn light_of(block: &[u32]) -> f64 {
    let w = vxe_render::internals::BRICK_WORDS;
    block[..512]
        .iter()
        .filter(|&&d| d != 0)
        .flat_map(|&d| {
            let end = (d >> 11) as usize;
            end - w..end - w + 6
        })
        .map(|i| {
            let [r, g, b] = vxe_render::internals::rgb9e5_unpack(block[i]);
            f64::from(0.2126 * r + 0.7152 * g + 0.0722 * b)
        })
        .sum()
}

/// Ten chunks in a row along x: a rock block in chunk 8 (x 520 to 560) and an ore plate beside
/// it in chunk 8, the rest air.
fn far_row() -> World {
    let world = world_of([10, 1, 1], AIR);
    fill(&world, [520, 16, 16], [560, 48, 48], ROCK);
    fill(&world, [566, 28, 28], [570, 36, 36], ORE);
    world
}

/// Plan 0096 Phase 3 bar 2 (ADR-0080 rules 2 and 3): chunk 8 of [`far_row`] seen from chunk 0
/// (its nearest point 480 voxels from the eye's chunk's centre, 1.6 px a voxel) is baked at the
/// far tier, its records' tier bits far (2). The camera steps to chunk 6 (96 voxels, 8.0 px): the
/// chunk is re-listed as a refinement, its new records carry the near tier's bits (0), and its
/// block fades once: one run of steps holding a fade, of `FADE - 1` steps (the landing step's
/// advance is the fade's first frame; the step whose advance reaches `FADE` ends it), and no
/// second.
#[test]
fn a_far_record_is_promoted_as_one_fade_when_the_camera_comes_near() {
    let c = [8, 0, 0];
    let Some(mut q) = tiered(far_row(), &Setup::at([32.5, 32.5, 32.5])) else {
        return;
    };
    q.settle();
    let far = q.target(c).expect("chunk 8's block");
    assert_eq!(tiers_of(&far), vec![2], "the far tier's bits");
    q.camera = camera_at([416.5, 32.5, 32.5]);
    let mut runs: Vec<usize> = Vec::new();
    let mut fading = false;
    for _ in 0..MAX_STEPS {
        q.step();
        let now = q.q().tracks(ChunkCoord(c)).contains(&"fades");
        if now {
            if fading {
                *runs.last_mut().expect("a run") += 1;
            } else {
                runs.push(1);
            }
        }
        fading = now;
        if q.q().empty() {
            break;
        }
    }
    assert!(q.q().empty(), "{:?}", q.q().stats());
    let near = q.target(c).expect("chunk 8's block");
    assert_eq!(tiers_of(&near), vec![0], "the near tier's bits");
    assert_ne!(near, far);
    assert_eq!(runs, vec![FADE as usize - 1], "one fade over FADE frames");
}

/// Bakes [`far_row`] seen from chunk 0 to an empty queue, its steps sliced by the frame's slice
/// (`samples` `None`) or a pre-roll's count a step: chunk 8's block.
fn far_block(samples: Option<u64>) -> Option<Vec<u32>> {
    let mut q = tiered(far_row(), &Setup::at([32.5, 32.5, 32.5]))?;
    q.samples = samples;
    q.settle();
    q.target([8, 0, 0])
}

/// The partial rig: [`wall_row`] shifted so the camera sits 8 chunks before the receiver, the
/// row at x 8 to 12; with `lod_wall`, the wall's chunk (10) is LOD-only and not resident full,
/// as a streamer leaves a chunk past its full radius. The view's full radius is 0 chunks, so
/// every reach crosses it, and a reach past the row's one-chunk cross-section is never resident
/// full: the receiver, far band, bakes from the chunks resident by rule 4; the near-band chunks
/// by the queue's `ReachMode::Partial`, set here so they bake at all.
fn partial_rig(wall: bool, lod_wall: bool) -> Option<Q> {
    fn but_wall(c: ChunkCoord) -> bool {
        c != ChunkCoord([10, 0, 0])
    }
    fn every(_: ChunkCoord) -> bool {
        true
    }
    let world = world_of([13, 1, 1], AIR);
    fill(&world, [582, 8, 8], [632, 56, 56], ROCK);
    if wall {
        fill(&world, [662, 0, 0], [682, 64, 32], ROCK);
    }
    fill(&world, [712, 24, 16], [716, 40, 48], ORE);
    if lod_wall {
        support::make_lod(&world, |c| c == ChunkCoord([10, 0, 0]));
    }
    let mut q = tiered(
        world,
        &Setup {
            light_radius: 2,
            ..Setup::at([32.5, 32.5, 32.5])
        },
    )?;
    q.resident = Some(if lod_wall { but_wall } else { every });
    q.full_radius = Some(0);
    q.frame.set_bake_reach(vxe_render::ReachMode::Partial);
    Some(q)
}

/// Plan 0096 Phase 3 bar 3 (ADR-0046's identity per tier, ADR-0080): chunk 8's far-tier record
/// of [`far_row`] hashes the same over two runs and two slicings (the frame's slice and the
/// pre-roll's 16 384 samples a step); and the receiver's partial far record of the partial rig (the
/// wall LOD-only), baked from a still camera with the rig's chunks resident, the same over two
/// runs.
#[test]
fn a_tiers_record_bytes_are_the_same_on_every_run() {
    let Some(a) = far_block(None) else {
        return;
    };
    let b = far_block(None).expect("an adapter");
    let c = far_block(Some(16_384)).expect("an adapter");
    assert_eq!(tiers_of(&a), vec![2]);
    assert_eq!(a, b, "two runs");
    assert_eq!(a, c, "two slicings");
    let partial = || {
        let mut q = partial_rig(true, true).expect("an adapter");
        q.settle();
        assert!(
            q.q().tracks(ChunkCoord([9, 0, 0])).contains(&"partial"),
            "{:?}",
            q.q().tracks(ChunkCoord([9, 0, 0]))
        );
        q.target([9, 0, 0]).expect("the receiver's block")
    };
    let (p, r) = (partial(), partial());
    assert_eq!(tiers_of(&p), vec![2]);
    assert_eq!(p, r, "the partial far record over two runs");
}

/// Plan 0096 Phase 3 bar 5 (ADR-0080 rule 4): the receiver (chunk 9) of the partial rig, far
/// band, bakes from the chunks resident with its wall chunk (10) LOD-only: its record is darker
/// than the same rig with no wall (the walk casts from the wall's mips), and its light is within
/// the far tier's noise (`1 / sqrt(16)` of the open rig's light) of the record baked with the
/// wall resident full.
#[test]
fn a_lod_only_chunk_in_a_partial_reach_casts_from_its_mip() {
    let read = |wall: bool, lod_wall: bool| {
        let mut q = partial_rig(wall, lod_wall)?;
        if lod_wall {
            assert!(q.gpu.is_lod(ChunkCoord([10, 0, 0])), "the wall is LOD-only");
        }
        q.settle();
        let block = q.target([9, 0, 0]).expect("the receiver's block");
        assert_eq!(tiers_of(&block), vec![2]);
        Some(light_of(&block))
    };
    let Some(open) = read(false, false) else {
        return;
    };
    let mip = read(true, true).expect("an adapter");
    let full = read(true, false).expect("an adapter");
    println!("receiver light: open {open:.4}, wall from its mips {mip:.4}, wall full {full:.4}");
    assert!(open > 0.0, "the ore lights the receiver");
    assert!(mip < open, "the LOD-only wall darkens the receiver");
    let noise = open / f64::from(TIERS[2]).sqrt();
    assert!(
        (mip - full).abs() <= noise,
        "mips {mip} against full {full}: over the far tier's noise {noise}"
    );
}

/// Chunk 8 of [`far_row`] baked from chunk 0 (far band) at tiers `[near, 32, 16]` and a queue
/// count of 128 (neither tier), then a pit carved into the rock's -x face, the reach ring off:
/// the samples the dig's ring re-baked, and chunk 8's records' tier bits after it.
fn dig_samples(near: u32) -> Option<(u64, Vec<u8>)> {
    let setup = Setup {
        counts: vxe_render::BakeSettings {
            brick_samples: 128,
            ..counts()
        },
        ..Setup::at([32.5, 32.5, 32.5])
    };
    let mut q = tiered(far_row(), &setup)?;
    q.frame.set_bake_tiers([near, TIERS[1], TIERS[2]]);
    q.frame
        .set_bake_digs(vxe_render::DEFAULT_BAKE_EDIT_MS, false);
    q.settle();
    assert_eq!(tiers_of(&q.target([8, 0, 0])?), vec![2], "baked far first");
    let before = q.q().stats().total_samples;
    q.edit(&box_op(
        [520, 28, 28],
        [526, 36, 36],
        vxe_world::Mode::Carve,
    ));
    q.settle();
    let st = q.q().stats();
    assert!(st.digs_baked > 0 && st.dig_bricks > 0, "{st:?}");
    Some((st.total_samples - before, tiers_of(&q.target([8, 0, 0])?)))
}

/// Plan 0096 Phase 9 (ADR-0080 rule 2): a dig at 48 m, in the far band, re-bakes its ring at the
/// near tier's count, not its band's (16) nor the queue's own (128): the same dig at near tiers
/// 64 and 32 draws samples in that ratio, each a whole number of the near count a direction,
/// and the dug records carry the near tier's bits beside the far ones the ring kept.
#[test]
fn a_digs_ring_bakes_at_the_near_tier_in_the_far_band() {
    let Some((at64, tiers)) = dig_samples(64) else {
        return;
    };
    let (at32, _) = dig_samples(32).expect("an adapter");
    println!("the dig's samples: near 64 {at64}, near 32 {at32}");
    assert!(at32 > 0, "the dig re-baked");
    assert_eq!(at32 % 32, 0, "a whole number of the near count a direction");
    assert_eq!(at64, 2 * at32, "the ring's count is the near tier's");
    assert_eq!(tiers, vec![0, 2], "dug records near, kept records far");
}

/// Readings, run by hand (`--run-ignored only -E 'test(=reading::<name>)'`).
mod reading {
    use super::*;

    /// The far tier's record against the near count's: chunk 8 of [`far_row`] seen from chunk
    /// 0 at the fly-through's tiers (4096, 1024, 256) and at 4096 throughout; the light sums and
    /// the mean relative difference of the direction words that hold light in either.
    #[test]
    #[ignore = "reading"]
    fn far_tier_against_the_full_count() {
        let bake = |tiers: [u32; 3]| {
            let mut q = tiered(far_row(), &Setup::at([32.5, 32.5, 32.5]))?;
            q.frame.set_bake_tiers(tiers);
            q.settle();
            q.target([8, 0, 0])
        };
        let Some(far) = bake([4096, 1024, 256]) else {
            return;
        };
        let full = bake([4096; 3]).expect("an adapter");
        let w = vxe_render::internals::BRICK_WORDS;
        let lum = |b: &[u32], i: usize| {
            let [r, g, bl] = vxe_render::internals::rgb9e5_unpack(b[i]);
            f64::from(0.2126 * r + 0.7152 * g + 0.0722 * bl)
        };
        let (mut sum, mut n) = (0.0, 0);
        for (s, &d) in full[..512].iter().enumerate() {
            if d == 0 || far[s] == 0 {
                continue;
            }
            let (ef, en) = ((far[s] >> 11) as usize, (d >> 11) as usize);
            for k in 0..6 {
                let (a, b) = (lum(&far, ef - w + k), lum(&full, en - w + k));
                if a.max(b) > 0.0 {
                    sum += (a - b).abs() / a.max(b);
                    n += 1;
                }
            }
        }
        println!(
            "far tier light {:.4}, full count {:.4}, tiers {:?} / {:?}, {n} lit words, mean relative difference {:.4}",
            light_of(&far),
            light_of(&full),
            tiers_of(&far),
            tiers_of(&full),
            sum / f64::from(n.max(1))
        );
    }
}
