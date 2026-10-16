//! Integer coordinates: voxels, chunks, and the lattice constants that relate them.
//!
//! A voxel `v` lies in chunk `v.div_euclid(64)`, in brick `v.rem_euclid(64).div_euclid(8)` of
//! that chunk. Slot and in-brick indices are x-fastest: `x + 8 y + 64 z` (ADR-0002).

/// Voxels along one edge of a brick.
pub const BRICK_EDGE: i32 = 8;
/// Bricks along one edge of a chunk.
pub const CHUNK_EDGE_BRICKS: i32 = 8;
/// Voxels along one edge of a chunk.
pub const CHUNK_EDGE: i32 = BRICK_EDGE * CHUNK_EDGE_BRICKS;
/// Voxels in one brick.
pub const BRICK_VOXELS: usize = 512;
/// Brick slots in one chunk.
pub const CHUNK_BRICKS: usize = 512;

/// A voxel's integer lattice position. Voxel `i` spans `[i, i + 1)` along each axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct VoxelCoord(pub [i32; 3]);

/// A chunk's integer position; chunk `c` holds voxels `[64 c, 64 c + 64)` along each axis.
///
/// The derived order (x, then y, then z) is the one every multi-chunk walk, result list and
/// hash uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct ChunkCoord(pub [i32; 3]);

impl From<[i32; 3]> for VoxelCoord {
    fn from(v: [i32; 3]) -> Self {
        Self(v)
    }
}

impl From<[i32; 3]> for ChunkCoord {
    fn from(c: [i32; 3]) -> Self {
        Self(c)
    }
}

impl PartialEq<[i32; 3]> for VoxelCoord {
    fn eq(&self, other: &[i32; 3]) -> bool {
        self.0 == *other
    }
}

impl PartialEq<[i32; 3]> for ChunkCoord {
    fn eq(&self, other: &[i32; 3]) -> bool {
        self.0 == *other
    }
}

impl VoxelCoord {
    /// The chunk holding this voxel.
    pub fn chunk(self) -> ChunkCoord {
        ChunkCoord(self.0.map(|v| v.div_euclid(CHUNK_EDGE)))
    }

    /// Position inside the chunk, each component in `0..64`.
    pub fn local(self) -> [i32; 3] {
        self.0.map(|v| v.rem_euclid(CHUNK_EDGE))
    }

    /// Index of the brick slot inside the chunk that holds this voxel.
    pub fn brick_slot(self) -> usize {
        brick_slot_of_local(self.local())
    }

    /// Index of this voxel inside its brick.
    pub fn index_in_brick(self) -> usize {
        let l = self.0.map(|v| v.rem_euclid(BRICK_EDGE));
        index_in_brick(l)
    }
}

impl ChunkCoord {
    /// The voxel at the chunk's minimum corner.
    pub fn min_voxel(self) -> VoxelCoord {
        VoxelCoord(self.0.map(|c| c * CHUNK_EDGE))
    }
}

/// Slot index of the brick holding chunk-local voxel `l` (components in `0..64`).
pub fn brick_slot_of_local(l: [i32; 3]) -> usize {
    let b = l.map(|v| v / BRICK_EDGE);
    brick_slot(b)
}

/// Slot index of brick `b` (components in `0..8`).
pub fn brick_slot(b: [i32; 3]) -> usize {
    (b[0] + CHUNK_EDGE_BRICKS * (b[1] + CHUNK_EDGE_BRICKS * b[2])) as usize
}

/// The brick position (components in `0..8`) of slot `slot`.
pub fn brick_of_slot(slot: usize) -> [i32; 3] {
    let s = slot as i32;
    [s % 8, (s / 8) % 8, s / 64]
}

/// Index of brick-local voxel `l` (components in `0..8`) inside its brick.
pub fn index_in_brick(l: [i32; 3]) -> usize {
    (l[0] + BRICK_EDGE * (l[1] + BRICK_EDGE * l[2])) as usize
}

/// A half-open box of chunks, `min` inclusive and `max` exclusive on every axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChunkExtent {
    pub min: ChunkCoord,
    pub max: ChunkCoord,
}

impl ChunkExtent {
    pub fn new(min: impl Into<ChunkCoord>, max: impl Into<ChunkCoord>) -> Self {
        Self {
            min: min.into(),
            max: max.into(),
        }
    }

    /// Chunks along each axis.
    pub fn size(&self) -> [i32; 3] {
        [0, 1, 2].map(|k| (self.max.0[k] - self.min.0[k]).max(0))
    }

    /// Number of chunks in the extent.
    pub fn len(&self) -> usize {
        self.size().iter().map(|&n| n as usize).product()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn contains(&self, c: ChunkCoord) -> bool {
        (0..3).all(|k| c.0[k] >= self.min.0[k] && c.0[k] < self.max.0[k])
    }

    /// Every chunk in the extent, in ascending coordinate order.
    pub fn coords(&self) -> Vec<ChunkCoord> {
        let mut out = Vec::with_capacity(self.len());
        for x in self.min.0[0]..self.max.0[0] {
            for y in self.min.0[1]..self.max.0[1] {
                for z in self.min.0[2]..self.max.0[2] {
                    out.push(ChunkCoord([x, y, z]));
                }
            }
        }
        out
    }
}
