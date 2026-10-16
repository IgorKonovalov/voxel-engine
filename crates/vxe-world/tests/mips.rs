//! Chunk mips: the conservative majority rule cell by cell, uniform chunks, the carved-sphere
//! counts, and agreement with a brute-force reference.

mod support;

use support::*;
use vxe_world::*;

/// An air chunk with the given voxels set.
fn chunk_with(voxels: &[([i32; 3], MaterialId)]) -> Chunk {
    let mut c = Chunk::default();
    for &(v, m) in voxels {
        let v = VoxelCoord(v);
        let slot = v.brick_slot();
        let mut cells = Box::new([AIR; BRICK_VOXELS]);
        for (i, cell) in cells.iter_mut().enumerate() {
            *cell = c.brick(slot).get(i);
        }
        cells[v.index_in_brick()] = m;
        c.set_brick(slot, Brick::from_voxels(cells));
    }
    c
}

/// Every cell of level `n` with its coordinate.
fn cells(n: usize) -> impl Iterator<Item = [i32; 3]> {
    let e = 64 >> n;
    (0..e * e * e).map(move |i| [i % e, i / e % e, i / (e * e)])
}

#[test]
fn one_voxel_survives_every_level() {
    let mips = ChunkMips::build(&chunk_with(&[([3, 3, 3], 5)]));
    assert_eq!(mips.cell(1, [1, 1, 1]), 5);
    assert_eq!(mips.cell(2, [0, 0, 0]), 5);
    assert_eq!(mips.cell(3, [0, 0, 0]), 5);
    for (n, solid) in [(1, [1, 1, 1]), (2, [0, 0, 0]), (3, [0, 0, 0])] {
        for c in cells(n) {
            if c != solid {
                assert_eq!(mips.cell(n, c), AIR, "level {n} cell {c:?}");
            }
        }
    }
    let full = |n: usize| mips.level(n).iter().filter(|b| b.is_full()).count();
    assert_eq!(mips.level(1).len(), 64);
    assert_eq!(mips.level(2).len(), 8);
    assert_eq!(mips.level(3).len(), 1);
    assert_eq!((full(1), full(2), full(3)), (1, 1, 1));
}

/// The level-1 cell over the 2³ block at voxel `[2, 4, 6]` holding `block`, offsets
/// x-fastest.
fn block_cell(block: [MaterialId; 8]) -> MaterialId {
    let voxels: Vec<([i32; 3], MaterialId)> = (0..8)
        .map(|i| {
            (
                [2 + (i & 1), 4 + ((i >> 1) & 1), 6 + (i >> 2)],
                block[i as usize],
            )
        })
        .collect();
    ChunkMips::build(&chunk_with(&voxels)).cell(1, [1, 2, 3])
}

#[test]
fn a_block_takes_its_majority_with_ties_low_and_air_last() {
    assert_eq!(
        block_cell([2, 7, 2, 7, 7, 2, 2, 7]),
        2,
        "4 and 4: the lower id"
    );
    assert_eq!(block_cell([7, 0, 7, 2, 0, 0, 7, 0]), 7, "3, 1 and 4 air");
    assert_eq!(block_cell([0; 8]), AIR);
    assert_eq!(
        majority([9, 0, 0, 0, 0, 0, 0, 0]),
        9,
        "one solid child is enough"
    );
}

#[test]
fn a_uniform_chunk_has_only_uniform_mip_bricks() {
    for m in [AIR, 3] {
        let mips = ChunkMips::build(&Chunk::uniform(m));
        for n in 1..=3 {
            assert!(
                mips.level(n).iter().all(|b| *b == Brick::Uniform(m)),
                "{m} level {n}"
            );
        }
        assert_eq!(mips.full_bricks(), 0);
    }
}

#[test]
fn the_carved_sphere_leaves_410_32_and_0_air_cells() {
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &UniformSource(3),
        &test_table(),
    );
    let r = world
        .apply(&EditOp::sphere(
            centre_of_voxel([32, 32, 32]),
            10 * FIXED_ONE,
            Mode::Carve,
        ))
        .unwrap();
    assert_eq!(r.changed_from, [(3, 4169)]);
    let chunk = world.store().get(ChunkCoord([0, 0, 0])).unwrap();
    let mips = ChunkMips::build(&chunk.read().unwrap());
    let air = |n: usize| cells(n).filter(|&c| mips.cell(n, c) == AIR).count();
    assert_eq!((air(1), air(2), air(3)), (410, 32, 0));
    // An air cell is exactly one whose voxels all lie in the sphere.
    for c in cells(1) {
        let inside = (0..8).all(|i| {
            let v = [0, 1, 2].map(|k| 2 * c[k] + (i >> k & 1) - 32);
            v.iter().map(|d| d * d).sum::<i32>() <= 100
        });
        assert_eq!(mips.cell(1, c) == AIR, inside, "cell {c:?}");
    }
}

/// The reference: every level as a flat cube of cells, each cell from its eight children
/// by counting, with no shortcut for uniform bricks.
fn reference(chunk: &Chunk) -> Vec<Vec<MaterialId>> {
    let voxel = |v: [i32; 3]| {
        let v = VoxelCoord(v);
        chunk.brick(v.brick_slot()).get(v.index_in_brick())
    };
    let mut levels: Vec<Vec<MaterialId>> = Vec::new();
    for n in 1..=3 {
        let e = 64 >> n;
        let mut cells = vec![AIR; (e * e * e) as usize];
        for z in 0..e {
            for y in 0..e {
                for x in 0..e {
                    let mut counts = std::collections::BTreeMap::<MaterialId, u32>::new();
                    for i in 0..8 {
                        let c = [2 * x + (i & 1), 2 * y + ((i >> 1) & 1), 2 * z + (i >> 2)];
                        let m = if n == 1 {
                            voxel(c)
                        } else {
                            let pe = 2 * e;
                            levels[n - 2][(c[0] + pe * (c[1] + pe * c[2])) as usize]
                        };
                        if m != AIR {
                            *counts.entry(m).or_default() += 1;
                        }
                    }
                    // Ascending ids: the first with the top count wins a tie.
                    let top = counts.values().copied().max().unwrap_or(0);
                    let m = counts
                        .iter()
                        .find(|&(_, &n)| n == top)
                        .map_or(AIR, |(&m, _)| m);
                    cells[(x + e * (y + e * z)) as usize] = m;
                }
            }
        }
        levels.push(cells);
    }
    levels
}

#[test]
fn mips_equal_a_brute_force_reference_over_seeded_chunks() {
    let mut rng = SplitMix64(0x0313_0006);
    for k in 0..16 {
        // Each brick: uniform of a random material, or voxels drawn from a small set with
        // air weighted up, so blocks mix air, ties and majorities.
        let mut chunk = Chunk::default();
        for slot in 0..CHUNK_BRICKS {
            let brick = if rng.next_u64().is_multiple_of(3) {
                Brick::Uniform(rng.range(0, 4) as MaterialId)
            } else {
                let mut v = Box::new([AIR; BRICK_VOXELS]);
                for m in v.iter_mut() {
                    *m = [0, 0, 0, 1, 2, 3, 7][rng.range(0, 6) as usize];
                }
                Brick::from_voxels(v)
            };
            chunk.set_brick(slot, brick);
        }
        let mips = ChunkMips::build(&chunk);
        let want = reference(&chunk);
        for n in 1..=3 {
            let e = 64 >> n;
            for c in cells(n) {
                assert_eq!(
                    mips.cell(n, c),
                    want[n - 1][(c[0] + e * (c[1] + e * c[2])) as usize],
                    "chunk {k} level {n} cell {c:?}"
                );
            }
        }
    }
}
