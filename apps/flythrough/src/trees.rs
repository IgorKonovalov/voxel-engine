//! The outdoor testbed's trees: [`TREE_VARIANTS`] trees built once per seed as masked
//! prefabs, stood at generation time on the scatter cells of a fixed forest box around the
//! mouth and the forest spawn. Every value is an integer function of the seed and the cell or
//! voxel coordinate, so chunks generate in any order on any thread with identical bytes.
//!
//! **A tree** ([`build_tree`]) of a [`TreeShape`] is a trunk of [`BARK`], the voxels with
//! `dx^2 + dy^2 <= rt^2` about its axis column from [`TRUNK_SINK`] voxels under its base to
//! `base + height - 1`, and a canopy of one of [`LEAF_ROWS`] (the variant's, [`leaf_row`]), the
//! voxels with `dx^2 + dy^2 + dz^2 <= rc^2` about `(axis, base + height)` that an integer hash
//! keeps with probability `leaf_fill / 256`.
//! Bark wins over leaf. Every other voxel of its box is left unwritten, so the world around a
//! tree stays as it was. The prefab's frame puts the axis column at `(rc, rc)` and the base at
//! z = [`TRUNK_SINK`], so a quarter turn about z keeps the axis where it was.
//!
//! **The scatter** ([`ForestSource`]): the xy plane of [`tree_box`] is cut into cells of
//! [`TREE_CELL`] voxels. Per cell a hash of the seed and the cell says whether a tree stands
//! (`hash & 255 < keep`, [`TreeDensity::keep`]), which variant it is, which of four z
//! rotations, and its axis's offset in the cell (`8 + bits mod 32` on each axis). No axis lies
//! within [`MOUTH_CLEAR`] voxels of [`MOUTH`], within [`PATH_CLEAR`] of the xy segment from the
//! `outside` spawn's column to the mouth, or within [`FOREST_CLEAR`] of [`FOREST`]. A tree's
//! base is the surface height of its axis column. A chunk takes every placement whose box
//! meets it, in cell order, a later placement winning where two overlap, as
//! `vxe_world::StampedSource` writes its placements; unlike it the placements share the
//! rotated variants (64 prefabs, not one copy a placement).

use std::collections::HashMap;
use std::sync::Arc;

use vxe_world::hash::hash3;
use vxe_world::{
    BRICK_EDGE, CHUNK_BRICKS, CHUNK_EDGE, Chunk, ChunkCoord, MaterialId, Prefab, Rot, WorldSource,
    brick_of_slot, index_in_brick,
};

use crate::outdoor::{BARK, FOREST, LEAF_ROWS, MOUTH, OUTSIDE_RUN, OutdoorSource};

/// The scatter cell's edge in voxels.
pub const TREE_CELL: i32 = 48;
/// The trees built per seed.
pub const TREE_VARIANTS: usize = 16;
/// `--leaf-fill` by default: a canopy voxel is kept when its hash's low byte is under this.
pub const LEAF_FILL: u32 = 64;
/// The trunk reaches this many voxels under the base, so no tree floats on a slope.
pub const TRUNK_SINK: i32 = 12;
/// The forest box's edge in voxels, square in xy ([`tree_box`]).
pub const TREE_BOX_EDGE: i32 = 3072;
/// No tree's axis within this many voxels of [`MOUTH`]: the largest canopy radius is 29, so
/// no bark or leaf voxel lies within 121.
pub const MOUTH_CLEAR: i64 = 150;
/// No tree's axis within this many voxels of the xy segment from the `outside` spawn's
/// column to [`MOUTH`].
pub const PATH_CLEAR: i64 = 60;
/// No tree's axis within this many voxels of [`FOREST`].
pub const FOREST_CLEAR: i64 = 10;
/// The largest canopy radius a [`TreeShape`] draws.
pub const MAX_CANOPY: i32 = 29;

const CELL_SALT: u64 = 0x7EE5_0000_0000_0001;
const VARIANT_SALT: u64 = 0x7EE5_0000_0000_0002;
const LEAF_SALT: u64 = 0x7EE5_0000_0000_0003;

/// `--trees`: how many scatter cells hold a tree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TreeDensity {
    /// Bare hills.
    None,
    /// About 9 % of the cells; the outdoor source's default.
    #[default]
    Sparse,
    /// About 94 % of the cells.
    Forest,
}

impl TreeDensity {
    pub const ALL: [Self; 3] = [Self::None, Self::Sparse, Self::Forest];

    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Sparse => "sparse",
            Self::Forest => "forest",
        }
    }

    /// The banner's source text for the outdoor hills with these trees: `outdoor, trees forest`.
    pub fn source_words(self) -> &'static str {
        match self {
            Self::None => "outdoor, trees none",
            Self::Sparse => "outdoor, trees sparse",
            Self::Forest => "outdoor, trees forest",
        }
    }

    /// A cell holds a tree when its hash's low byte is under this.
    pub fn keep(self) -> u64 {
        match self {
            Self::None => 0,
            Self::Sparse => 24,
            Self::Forest => 240,
        }
    }

    /// The next density in [`TreeDensity::ALL`], back to the first after the last.
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|&d| d == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

/// A tree's proportions in voxels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeShape {
    /// 2 or 3.
    pub trunk_r: i32,
    /// 50 to 89: the trunk's voxels from the base up.
    pub height: i32,
    /// 18 to 29.
    pub canopy_r: i32,
}

impl TreeShape {
    /// The shape drawn from the bits of `h`.
    pub fn from_hash(h: u64) -> Self {
        Self {
            trunk_r: 2 + (h & 1) as i32,
            height: 50 + ((h >> 1) % 40) as i32,
            canopy_r: 18 + ((h >> 8) % 12) as i32,
        }
    }

    /// The prefab's box: `2 rc + 1` square, from [`TRUNK_SINK`] under the base to the
    /// canopy's top.
    pub fn size(self) -> [u32; 3] {
        let e = (2 * self.canopy_r + 1) as u32;
        [e, e, (TRUNK_SINK + self.height + self.canopy_r + 1) as u32]
    }

    /// The axis column's local xy, `(rc, rc)`.
    pub fn axis(self) -> [i32; 2] {
        [self.canopy_r; 2]
    }

    /// The canopy's centre in the prefab's frame.
    pub fn canopy_centre(self) -> [i32; 3] {
        [self.canopy_r, self.canopy_r, TRUNK_SINK + self.height]
    }

    /// Whether local voxel `l` is trunk.
    pub fn is_trunk(self, l: [i32; 3]) -> bool {
        let [dx, dy] = [l[0] - self.canopy_r, l[1] - self.canopy_r].map(i64::from);
        let rt = i64::from(self.trunk_r);
        dx * dx + dy * dy <= rt * rt && (0..TRUNK_SINK + self.height).contains(&l[2])
    }

    /// Whether local voxel `l` lies in the canopy's sphere.
    pub fn in_canopy(self, l: [i32; 3]) -> bool {
        let c = self.canopy_centre();
        let d: i64 = (0..3).map(|k| i64::from(l[k] - c[k]).pow(2)).sum();
        d <= i64::from(self.canopy_r).pow(2)
    }
}

/// The tree of `shape` as a masked prefab: [`BARK`] on the trunk, `leaf` on the canopy
/// voxels whose hash of `(leaf_seed, local voxel)` has its low byte under `leaf_fill` (256
/// keeps every one), nothing written elsewhere.
pub fn build_tree(shape: TreeShape, leaf_seed: u64, leaf_fill: u32, leaf: MaterialId) -> Prefab {
    let size = shape.size();
    let n = size.iter().map(|&s| s as usize).product::<usize>();
    let mut voxels = vec![vxe_world::AIR; n];
    let mut mask = vec![0u64; n.div_ceil(64)];
    let mut i = 0usize;
    for z in 0..size[2] as i32 {
        for y in 0..size[1] as i32 {
            for x in 0..size[0] as i32 {
                let l = [x, y, z];
                let m = if shape.is_trunk(l) {
                    Some(BARK)
                } else if shape.in_canopy(l)
                    && (hash3(leaf_seed ^ LEAF_SALT, l) & 255) < u64::from(leaf_fill)
                {
                    Some(leaf)
                } else {
                    None
                };
                if let Some(m) = m {
                    voxels[i] = m;
                    mask[i / 64] |= 1 << (i % 64);
                }
                i += 1;
            }
        }
    }
    Prefab::new(size, voxels, Some(mask))
}

/// Variant `v`'s leaf row for `seed`: one of [`LEAF_ROWS`] by a hash of the seed and `v`.
pub fn leaf_row(seed: u64, v: usize) -> MaterialId {
    let h = hash3(seed ^ VARIANT_SALT, [v as i32, 1, 0]);
    LEAF_ROWS[(h % LEAF_ROWS.len() as u64) as usize]
}

/// Variant `v`'s shape for `seed`.
pub fn variant_shape(seed: u64, v: usize) -> TreeShape {
    TreeShape::from_hash(hash3(seed ^ VARIANT_SALT, [v as i32, 0, 0]))
}

/// The forest box's xy corners, inclusive minimum and exclusive maximum: [`TREE_BOX_EDGE`]
/// square, centred between [`MOUTH`] and [`FOREST`].
pub fn tree_box() -> [[i32; 2]; 2] {
    let c = [0, 1].map(|k| (MOUTH[k] + FOREST[k]) / 2);
    let h = TREE_BOX_EDGE / 2;
    [[c[0] - h, c[1] - h], [c[0] + h, c[1] + h]]
}

/// A quarter turn about z, `k` times.
fn z_turn(k: u64) -> Rot {
    (0..k).fold(Rot::IDENTITY, |r, _| r.then(Rot::Z90))
}

/// Whether a tree's axis at column `a` is kept out: near [`MOUTH`], the path from the
/// `outside` spawn to it, or [`FOREST`].
pub fn axis_excluded(a: [i32; 2]) -> bool {
    let d2 = |p: [i32; 2]| -> i64 { (0..2).map(|k| i64::from(a[k] - p[k]).pow(2)).sum() };
    if d2(MOUTH) <= MOUTH_CLEAR * MOUTH_CLEAR || d2(FOREST) <= FOREST_CLEAR * FOREST_CLEAR {
        return true;
    }
    // The segment from the outside spawn's column to the mouth, compared exactly.
    let s = [MOUTH[0] - OUTSIDE_RUN, MOUTH[1]].map(i64::from);
    let e = MOUTH.map(i64::from);
    let p = a.map(i64::from);
    let d = [e[0] - s[0], e[1] - s[1]];
    let w = [p[0] - s[0], p[1] - s[1]];
    let dd = d[0] * d[0] + d[1] * d[1];
    let wd = w[0] * d[0] + w[1] * d[1];
    let ww = w[0] * w[0] + w[1] * w[1];
    let r2 = PATH_CLEAR * PATH_CLEAR;
    if wd <= 0 {
        ww <= r2
    } else if wd >= dd {
        d2(MOUTH) <= r2
    } else {
        i128::from(ww) * i128::from(dd) - i128::from(wd).pow(2) <= i128::from(r2 * dd)
    }
}

/// One tree stood in the world.
#[derive(Clone, Debug)]
pub struct Placement {
    /// The variant's index and shape.
    pub variant: usize,
    pub shape: TreeShape,
    /// The axis column and the base: the surface height there.
    pub axis: [i32; 2],
    pub base: i32,
    /// The rotated variant and its inclusive voxel box.
    prefab: Arc<Prefab>,
    lo: [i32; 3],
    hi: [i32; 3],
}

/// [`OutdoorSource`] with the trees of a density stood on it.
#[derive(Clone, Debug)]
pub struct ForestSource {
    base: OutdoorSource,
    placements: Vec<Placement>,
    /// Per chunk a placement's box meets, the indices of every such placement, ascending.
    by_chunk: HashMap<ChunkCoord, Vec<u32>>,
}

impl ForestSource {
    /// `base`'s hills with the trees of `density` at `leaf_fill`, the variants and the
    /// scatter drawn from `base`'s seed.
    pub fn new(base: OutdoorSource, density: TreeDensity, leaf_fill: u32) -> Self {
        let seed = base.seed;
        let turned: Vec<[Arc<Prefab>; 4]> = (0..TREE_VARIANTS)
            .map(|v| {
                let p = build_tree(
                    variant_shape(seed, v),
                    seed ^ v as u64,
                    leaf_fill,
                    leaf_row(seed, v),
                );
                [0, 1, 2, 3].map(|k| Arc::new(p.rotated(z_turn(k))))
            })
            .collect();
        let [lo, hi] = tree_box();
        let mut placements = Vec::new();
        for cy in (lo[1]..hi[1]).step_by(TREE_CELL as usize) {
            for cx in (lo[0]..hi[0]).step_by(TREE_CELL as usize) {
                let h = hash3(seed ^ CELL_SALT, [cx, cy, 0]);
                if h & 255 >= density.keep() {
                    continue;
                }
                let variant = ((h >> 8) % TREE_VARIANTS as u64) as usize;
                let axis = [
                    cx + 8 + ((h >> 14) & 31) as i32,
                    cy + 8 + ((h >> 19) & 31) as i32,
                ];
                if axis_excluded(axis) {
                    continue;
                }
                let shape = variant_shape(seed, variant);
                let prefab = Arc::clone(&turned[variant][((h >> 12) & 3) as usize]);
                let ground = base.height(axis[0], axis[1]);
                let r = shape.canopy_r;
                let lo = [axis[0] - r, axis[1] - r, ground - TRUNK_SINK];
                let s = prefab.size();
                let hi = [0, 1, 2].map(|k| lo[k] + s[k] as i32 - 1);
                placements.push(Placement {
                    variant,
                    shape,
                    axis,
                    base: ground,
                    prefab,
                    lo,
                    hi,
                });
            }
        }
        let mut by_chunk: HashMap<ChunkCoord, Vec<u32>> = HashMap::new();
        for (i, p) in placements.iter().enumerate() {
            let [c0, c1] = [p.lo, p.hi].map(|v| v.map(|c| c.div_euclid(CHUNK_EDGE)));
            for z in c0[2]..=c1[2] {
                for y in c0[1]..=c1[1] {
                    for x in c0[0]..=c1[0] {
                        by_chunk
                            .entry(ChunkCoord([x, y, z]))
                            .or_default()
                            .push(i as u32);
                    }
                }
            }
        }
        Self {
            base,
            placements,
            by_chunk,
        }
    }

    pub fn base(&self) -> &OutdoorSource {
        &self.base
    }

    /// The trees, in cell order.
    pub fn placements(&self) -> &[Placement] {
        &self.placements
    }

    /// The material of voxel `p`: the last placement that writes it, else the hills'.
    pub fn material(&self, p: [i32; 3]) -> MaterialId {
        let c = ChunkCoord(p.map(|v| v.div_euclid(CHUNK_EDGE)));
        for &i in self.by_chunk.get(&c).into_iter().flatten().rev() {
            let t = &self.placements[i as usize];
            if let Some(m) = t.prefab.get([0, 1, 2].map(|k| p[k] - t.lo[k])) {
                return m;
            }
        }
        self.base.material(p)
    }
}

impl WorldSource for ForestSource {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
        self.base.chunk(coord, out);
        let Some(list) = self.by_chunk.get(&coord) else {
            return;
        };
        let base = coord.min_voxel().0;
        for slot in 0..CHUNK_BRICKS {
            let s = brick_of_slot(slot);
            let blo = [0, 1, 2].map(|k| base[k] + s[k] * BRICK_EDGE);
            let bhi = blo.map(|v| v + BRICK_EDGE - 1);
            let mut meets = list
                .iter()
                .map(|&i| &self.placements[i as usize])
                .filter(|t| (0..3).all(|k| t.lo[k] <= bhi[k] && t.hi[k] >= blo[k]));
            let Some(first) = meets.next() else {
                continue;
            };
            let mut brick = out.brick(slot).clone();
            brick.modify(|v| {
                for t in std::iter::once(first).chain(meets) {
                    let a = [0, 1, 2].map(|k| t.lo[k].max(blo[k]));
                    let b = [0, 1, 2].map(|k| t.hi[k].min(bhi[k]));
                    for z in a[2]..=b[2] {
                        for y in a[1]..=b[1] {
                            for x in a[0]..=b[0] {
                                let l = [x - t.lo[0], y - t.lo[1], z - t.lo[2]];
                                if let Some(m) = t.prefab.get(l) {
                                    v[index_in_brick([x - blo[0], y - blo[1], z - blo[2]])] = m;
                                }
                            }
                        }
                    }
                }
            });
            out.set_brick(slot, brick);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The density cycles none, sparse, forest and back; a cell holds a tree under 24 or
    /// 240 of 256.
    #[test]
    fn the_density_cycles_and_keeps() {
        let mut d = TreeDensity::None;
        let mut seen = Vec::new();
        for _ in 0..3 {
            d = d.next();
            seen.push(d.name());
        }
        assert_eq!(seen, ["sparse", "forest", "none"]);
        assert_eq!(TreeDensity::ALL.map(TreeDensity::keep), [0, 24, 240]);
    }

    /// Seed 1's variants take every leaf row, and a variant's tree is leaved in its own row
    /// alone.
    #[test]
    fn the_variants_take_every_leaf_row() {
        let rows: Vec<MaterialId> = (0..TREE_VARIANTS).map(|v| leaf_row(1, v)).collect();
        for m in LEAF_ROWS {
            assert!(rows.contains(&m), "{rows:?}");
        }
        for v in [0, 1, 2] {
            let p = build_tree(variant_shape(1, v), 1 ^ v as u64, LEAF_FILL, leaf_row(1, v));
            let [sx, sy, sz] = p.size().map(|s| s as i32);
            let mut leaves = 0;
            for z in 0..sz {
                for y in 0..sy {
                    for x in 0..sx {
                        match p.get([x, y, z]) {
                            Some(m) if m == BARK => {}
                            Some(m) => {
                                assert_eq!(m, leaf_row(1, v));
                                leaves += 1;
                            }
                            None => {}
                        }
                    }
                }
            }
            assert!(leaves > 0);
        }
    }

    /// A quarter turn about z keeps the axis column at `(rc, rc)`: the trunk's bark lies on
    /// the same local column in every rotation.
    #[test]
    fn a_turn_keeps_the_axis() {
        let shape = TreeShape {
            trunk_r: 2,
            height: 50,
            canopy_r: 18,
        };
        let p = build_tree(shape, 1, LEAF_FILL, crate::outdoor::LEAF);
        for k in 0..4 {
            let r = p.rotated(z_turn(k));
            assert_eq!(r.size(), p.size());
            for z in 0..TRUNK_SINK + shape.height {
                assert_eq!(r.get([18, 18, z]), Some(BARK), "turn {k}, z {z}");
            }
        }
    }
}
