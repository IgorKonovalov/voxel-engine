//! Chunk mips (ADR-0003): three coarser copies of a chunk, 32³, 16³ and 8³ cells, for the
//! LOD-only tier.
//!
//! A cell of level `n` covers `2^n` voxels along each axis and is built from its eight
//! children at level `n - 1` (level 0 being the voxels): it is air only when all eight are
//! air, and otherwise takes the material most of its non-air children hold, ties going to
//! the lowest id. The rule is conservative: any solid child keeps the cell solid, so thin
//! gaps close and openings shrink by up to a cell, but no surface disappears.
//!
//! Each level is stored as mip-bricks of 8³ cells, the same `Brick` type as voxel bricks
//! (`Uniform` when every cell agrees), so a mip-brick fills one pool slot in the mirror.
//! Level 1 has 4³ = 64 mip-bricks, level 2 has 2³ = 8 and level 3 has 1; mip-brick `i` of a
//! level with `e` mip-bricks along an edge sits at `(i % e, i / e % e, i / e²)`, and cells in
//! a mip-brick are indexed x-fastest like voxels in a brick.

use crate::brick::Brick;
use crate::chunk::Chunk;
use crate::coords::{BRICK_EDGE, BRICK_VOXELS, CHUNK_BRICKS, brick_of_slot, index_in_brick};
use crate::material::{AIR, MaterialId};

/// Mip levels kept for a LOD-only chunk.
pub const MIP_LEVELS: usize = 3;
/// Mip-bricks in each level, level 1 first.
pub const MIP_BRICKS: [usize; MIP_LEVELS] = [64, 8, 1];

/// A chunk's mips: `levels[n - 1]` holds level `n`'s mip-bricks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkMips {
    levels: [Vec<Brick>; MIP_LEVELS],
}

/// The material a cell takes from its eight children: the most frequent non-air material,
/// ties to the lowest id, or air when every child is air.
pub fn majority(children: [MaterialId; 8]) -> MaterialId {
    let mut v = children;
    v.sort_unstable();
    let mut best = AIR;
    let mut best_n = 0;
    let mut i = 0;
    while i < 8 {
        let m = v[i];
        let mut j = i;
        while j < 8 && v[j] == m {
            j += 1;
        }
        // Ascending order: a later run replaces the best only with a strictly larger count.
        if m != AIR && j - i > best_n {
            best = m;
            best_n = j - i;
        }
        i = j;
    }
    best
}

impl ChunkMips {
    /// The mips of `chunk`, built level by level under the majority rule above.
    pub fn build(chunk: &Chunk) -> Self {
        // Level 1 cells, 32³ x-fastest, from the voxels brick by brick.
        let mut l1 = vec![AIR; 32 * 32 * 32];
        for slot in 0..CHUNK_BRICKS {
            let b = brick_of_slot(slot);
            let base = b.map(|v| v * (BRICK_EDGE / 2));
            match chunk.brick(slot) {
                Brick::Uniform(m) => {
                    for z in 0..4 {
                        for y in 0..4 {
                            for x in 0..4 {
                                l1[cell_index(32, [base[0] + x, base[1] + y, base[2] + z])] = *m;
                            }
                        }
                    }
                }
                Brick::Full(p) => {
                    let mut v = [AIR; BRICK_VOXELS];
                    p.decode(&mut v);
                    for z in 0..4 {
                        for y in 0..4 {
                            for x in 0..4 {
                                let children = std::array::from_fn(|i| {
                                    let o = [i as i32 & 1, (i as i32 >> 1) & 1, i as i32 >> 2];
                                    v[index_in_brick([2 * x + o[0], 2 * y + o[1], 2 * z + o[2]])]
                                });
                                l1[cell_index(32, [base[0] + x, base[1] + y, base[2] + z])] =
                                    majority(children);
                            }
                        }
                    }
                }
            }
        }
        let l2 = downsample(&l1, 32);
        let l3 = downsample(&l2, 16);
        Self {
            levels: [pack(&l1, 32), pack(&l2, 16), pack(&l3, 8)],
        }
    }

    /// The mip-bricks of level `n` (1 to 3).
    pub fn level(&self, n: usize) -> &[Brick] {
        &self.levels[n - 1]
    }

    /// The material of cell `c` of level `n` (each component in `0..64 >> n`).
    pub fn cell(&self, n: usize, c: [i32; 3]) -> MaterialId {
        let e: i32 = 8 >> n;
        let b = c.map(|v| v / BRICK_EDGE);
        let l = c.map(|v| v % BRICK_EDGE);
        self.levels[n - 1][(b[0] + e * (b[1] + e * b[2])) as usize].get(index_in_brick(l))
    }

    /// `Full` mip-bricks over all levels.
    pub fn full_bricks(&self) -> usize {
        self.levels.iter().flatten().filter(|b| b.is_full()).count()
    }
}

fn cell_index(edge: i32, c: [i32; 3]) -> usize {
    (c[0] + edge * (c[1] + edge * c[2])) as usize
}

/// The next level of a cube of `edge`³ cells.
fn downsample(src: &[MaterialId], edge: i32) -> Vec<MaterialId> {
    let half = edge / 2;
    let mut out = vec![AIR; (half * half * half) as usize];
    for z in 0..half {
        for y in 0..half {
            for x in 0..half {
                let children = std::array::from_fn(|i| {
                    let o = [i as i32 & 1, (i as i32 >> 1) & 1, i as i32 >> 2];
                    src[cell_index(edge, [2 * x + o[0], 2 * y + o[1], 2 * z + o[2]])]
                });
                out[cell_index(half, [x, y, z])] = majority(children);
            }
        }
    }
    out
}

/// Cuts a cube of `edge`³ cells into mip-bricks of 8³, collapsing uniform ones.
fn pack(cells: &[MaterialId], edge: i32) -> Vec<Brick> {
    let e = edge / BRICK_EDGE;
    let mut out = Vec::with_capacity((e * e * e) as usize);
    for bz in 0..e {
        for by in 0..e {
            for bx in 0..e {
                let base = [bx, by, bz].map(|v| v * BRICK_EDGE);
                let mut v = Box::new([AIR; BRICK_VOXELS]);
                for z in 0..BRICK_EDGE {
                    for y in 0..BRICK_EDGE {
                        for x in 0..BRICK_EDGE {
                            v[index_in_brick([x, y, z])] =
                                cells[cell_index(edge, [base[0] + x, base[1] + y, base[2] + z])];
                        }
                    }
                }
                out.push(Brick::from_voxels(v));
            }
        }
    }
    out
}
