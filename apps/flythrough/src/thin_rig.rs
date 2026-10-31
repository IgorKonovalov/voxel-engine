//! `--thin-rig`: a bounded scene of free-standing thin features (the field surface's hard case,
//! ADR-0041): a vertical trunk 5 voxels across standing on a floor, carrying on its +x face a
//! one-voxel branch along x, a free 2 x 2 rod, a free 3 x 3 rod and a one-voxel diagonal branch
//! (one voxel per step in x and z), and on its -x face a one-voxel and a two-voxel free sheet
//! (vertical, their thickness along y). Each feature is 14 voxels long and clear of every other
//! by at least 4 voxels; the branches, rods and sheets are of the rig's second material (its
//! "wood"), the trunk and floor of rock.

use vxe_world::{
    AIR, BRICK_EDGE, Brick, CHUNK_BRICKS, Chunk, ChunkCoord, ChunkExtent, MaterialId, VoxelCoord,
    WorldSource, brick_of_slot,
};

use crate::cave::ROCK;

/// The rig's extent: 2 x 2 x 2 chunks, 128 voxels to an edge.
pub const EXTENT: [i32; 3] = [2, 2, 2];
/// Rock below this height.
pub const FLOOR_Z: i32 = 64;
/// The trunk: `x` and `y` in `TRUNK_LO..TRUNK_LO + 5`, from the floor up to `TRUNK_TOP`.
pub const TRUNK_LO: [i32; 2] = [62, 62];
pub const TRUNK_TOP: i32 = 118;
/// Each feature's length along x.
pub const LENGTH: i32 = 14;

/// One feature of the rig: its name and its voxels as half-open boxes `[lo, hi)`, or for the
/// diagonal branch its first voxel and its step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Feature {
    pub name: &'static str,
    pub shape: Shape,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// The voxels of the box `[lo, hi)`.
    Box { lo: [i32; 3], hi: [i32; 3] },
    /// `LENGTH` voxels from `start`, each one voxel on in x and in z from the last.
    Diagonal { start: [i32; 3] },
}

const PX: i32 = TRUNK_LO[0] + 5;
const MX: i32 = TRUNK_LO[0];

/// The six features, +x side first.
pub const FEATURES: [Feature; 6] = [
    Feature {
        name: "one-voxel branch",
        shape: Shape::Box {
            lo: [PX, 64, 72],
            hi: [PX + LENGTH, 65, 73],
        },
    },
    Feature {
        name: "2x2 rod",
        shape: Shape::Box {
            lo: [PX, 64, 78],
            hi: [PX + LENGTH, 66, 80],
        },
    },
    Feature {
        name: "3x3 rod",
        shape: Shape::Box {
            lo: [PX, 63, 86],
            hi: [PX + LENGTH, 66, 89],
        },
    },
    Feature {
        name: "diagonal branch",
        shape: Shape::Diagonal {
            start: [PX, 64, 96],
        },
    },
    Feature {
        name: "one-voxel sheet",
        shape: Shape::Box {
            lo: [MX - LENGTH, 64, 72],
            hi: [MX, 65, 82],
        },
    },
    Feature {
        name: "two-voxel sheet",
        shape: Shape::Box {
            lo: [MX - LENGTH, 64, 88],
            hi: [MX, 66, 98],
        },
    },
];

impl Feature {
    /// Whether voxel `p` is one of the feature's.
    pub fn holds(&self, p: [i32; 3]) -> bool {
        match self.shape {
            Shape::Box { lo, hi } => (0..3).all(|k| p[k] >= lo[k] && p[k] < hi[k]),
            Shape::Diagonal { start } => {
                let k = p[0] - start[0];
                (0..LENGTH).contains(&k) && p[1] == start[1] && p[2] == start[2] + k
            }
        }
    }

    /// The feature's bounding box `[lo, hi)` less the `gap` voxels nearest the trunk: the part
    /// a reading measures, clear of the trunk's surface.
    pub fn measured(&self, gap: i32) -> ([i32; 3], [i32; 3]) {
        let (mut lo, mut hi) = match self.shape {
            Shape::Box { lo, hi } => (lo, hi),
            Shape::Diagonal { start } => {
                (start, [start[0] + LENGTH, start[1] + 1, start[2] + LENGTH])
            }
        };
        if lo[0] >= PX {
            lo[0] += gap;
            if let Shape::Diagonal { .. } = self.shape {
                lo[2] += gap;
            }
        } else {
            hi[0] -= gap;
        }
        (lo, hi)
    }
}

/// The rig's world source: rock below [`FLOOR_Z`] and in the trunk, `wood` in the features,
/// air elsewhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThinRig {
    pub wood: MaterialId,
}

impl ThinRig {
    pub fn extent(&self) -> ChunkExtent {
        ChunkExtent::new([0, 0, 0], EXTENT)
    }

    /// The spawn: in front of the trunk on -y, at the height of the features' middle.
    pub fn spawn(&self) -> VoxelCoord {
        VoxelCoord([64, 20, 94])
    }

    pub fn material(&self, p: [i32; 3]) -> MaterialId {
        let trunk =
            (0..2).all(|k| p[k] >= TRUNK_LO[k] && p[k] < TRUNK_LO[k] + 5) && p[2] < TRUNK_TOP;
        if p[2] < FLOOR_Z || trunk {
            ROCK
        } else if FEATURES.iter().any(|f| f.holds(p)) {
            self.wood
        } else {
            AIR
        }
    }
}

impl WorldSource for ThinRig {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
        let base = coord.min_voxel().0;
        for slot in 0..CHUNK_BRICKS {
            let b = brick_of_slot(slot);
            let lo = [0, 1, 2].map(|k| base[k] + b[k] * BRICK_EDGE);
            let brick = if lo[2] + BRICK_EDGE <= FLOOR_Z {
                Brick::Uniform(ROCK)
            } else {
                Brick::from_fn(|l| self.material([0, 1, 2].map(|k| lo[k] + l[k])))
            };
            out.set_brick(slot, brick);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every feature is at least 12 voxels long, and every two features are at least 4 voxels
    /// apart (Chebyshev distance between their voxels, so 4 empty voxels lie between them).
    #[test]
    fn the_features_are_long_and_clear_of_each_other() {
        let voxels = |f: &Feature| -> Vec<[i32; 3]> {
            let mut v = Vec::new();
            for z in FLOOR_Z..TRUNK_TOP {
                for y in 56..72 {
                    for x in 40..90 {
                        if f.holds([x, y, z]) {
                            v.push([x, y, z]);
                        }
                    }
                }
            }
            v
        };
        let all: Vec<Vec<[i32; 3]>> = FEATURES.iter().map(voxels).collect();
        for (f, v) in FEATURES.iter().zip(&all) {
            let xs = v.iter().map(|p| p[0]);
            let len = xs.clone().max().unwrap() - xs.min().unwrap() + 1;
            assert!(len >= 12, "{}: {len}", f.name);
        }
        for i in 0..all.len() {
            for j in i + 1..all.len() {
                let d = all[i]
                    .iter()
                    .flat_map(|a| {
                        all[j]
                            .iter()
                            .map(move |b| (0..3).map(|k| (a[k] - b[k]).abs()).max().unwrap())
                    })
                    .min()
                    .unwrap();
                assert!(d >= 5, "{} and {}: {d}", FEATURES[i].name, FEATURES[j].name);
            }
        }
    }
}
