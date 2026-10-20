//! Brick occupancy masks: one bit per 2 × 2 × 2 block holding a non-air voxel.

mod support;

use support::SplitMix64;
use vxe_world::{AIR, BRICK_VOXELS, Brick, MaterialId};

fn brick_with(voxels: &[([usize; 3], MaterialId)]) -> Brick {
    let mut v = Box::new([AIR; BRICK_VOXELS]);
    for &([x, y, z], m) in voxels {
        v[x + 8 * y + 64 * z] = m;
    }
    Brick::from_voxels(v)
}

/// The mask by its definition: for each block, look at its eight voxels.
fn brute_force_mask(b: &Brick) -> u64 {
    let mut mask = 0;
    for bz in 0..4 {
        for by in 0..4 {
            for bx in 0..4 {
                let occupied = (0..8).any(|k| {
                    let (x, y, z) = (2 * bx + (k & 1), 2 * by + (k >> 1 & 1), 2 * bz + (k >> 2));
                    b.get(x + 8 * y + 64 * z) != AIR
                });
                if occupied {
                    mask |= 1u64 << (bx + 4 * by + 16 * bz);
                }
            }
        }
    }
    mask
}

#[test]
fn a_single_voxel_sets_its_blocks_bit() {
    let b = brick_with(&[([3, 3, 3], 2)]);
    assert!(b.is_full());
    assert_eq!(b.occupancy_mask(), 1 << 21);
}

#[test]
fn uniform_bricks_are_all_clear_or_all_set() {
    assert_eq!(Brick::Uniform(AIR).occupancy_mask(), 0);
    assert_eq!(Brick::Uniform(3).occupancy_mask(), u64::MAX);
}

#[test]
fn the_mask_matches_brute_force_over_seeded_random_bricks() {
    let mut rng = SplitMix64(0xb10c_3a5c);
    for n in 0..64 {
        // Solid fractions from about 1/256 to 1/2, so both sparse and dense bricks occur.
        let rarity = 1 + n % 8;
        let mut v = Box::new([AIR; BRICK_VOXELS]);
        for m in v.iter_mut() {
            if rng.next_u64().is_multiple_of(1 << rarity) {
                *m = 1 + (rng.next_u64() % 5) as MaterialId;
            }
        }
        let b = Brick::from_voxels(v);
        assert_eq!(b.occupancy_mask(), brute_force_mask(&b), "brick {n}");
    }
}

#[test]
fn a_brick_slot_is_sixteen_bytes() {
    assert_eq!(size_of::<Brick>(), 16);
}

/// `Brick` equality agrees with decoded equality over 256 seeded pairs: half equal voxels
/// built in two ways (directly and by a round trip through `modify`), half differing in one
/// voxel, over bricks of 1 to 20 materials.
#[test]
fn brick_equality_is_voxel_equality() {
    let mut rng = SplitMix64(0x0021_0003);
    for i in 0..256 {
        let materials = rng.range(1, 20) as u64;
        let solid = rng.range(0, 8);
        let mut v = Box::new([AIR; BRICK_VOXELS]);
        for m in v.iter_mut() {
            if rng.range(0, 8) < solid {
                *m = 1 + rng.range(0, materials as i64 - 1) as MaterialId;
            }
        }
        let a = Brick::from_voxels(v.clone());
        let mut w = v.clone();
        if i % 2 == 1 {
            let at = rng.range(0, BRICK_VOXELS as i64 - 1) as usize;
            w[at] = if w[at] == AIR { 1 } else { AIR };
        }
        let mut b = Brick::Uniform(AIR);
        b.modify(|out| *out = *w);
        let mut da = [AIR; BRICK_VOXELS];
        let mut db = [AIR; BRICK_VOXELS];
        a.decode(&mut da);
        b.decode(&mut db);
        assert_eq!(da, *v, "pair {i}");
        assert_eq!(db, *w, "pair {i}");
        assert_eq!(a == b, da == db, "pair {i}");
    }
}
