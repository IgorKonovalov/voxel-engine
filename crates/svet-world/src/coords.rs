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

    /// The squared distance to `other` in chunks, each component widened to `i64` before the
    /// difference, so no pair of `i32` coordinates overflows it.
    pub fn distance2(self, other: ChunkCoord) -> i64 {
        (0..3)
            .map(|k| (i64::from(self.0[k]) - i64::from(other.0[k])).pow(2))
            .sum()
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
    local_of_index(slot).map(|v| v as i32)
}

/// Index of brick-local voxel `l` (components in `0..8`) inside its brick.
pub fn index_in_brick(l: [i32; 3]) -> usize {
    (l[0] + BRICK_EDGE * (l[1] + BRICK_EDGE * l[2])) as usize
}

/// The position `[x, y, z]` (components in `0..8` for `i` in `0..512`) of index `i` in the
/// x-fastest order `x + 8 y + 64 z` that brick indices and slot indices share: the inverse of
/// [`index_in_brick`] and [`brick_slot`].
pub(crate) fn local_of_index(i: usize) -> [usize; 3] {
    [i % 8, i / 8 % 8, i / 64]
}

/// The part of the cube of edge `e` at lattice position `c` (coordinates `c e ..= c e + e - 1`
/// on each axis) inside the inclusive box `lo..=hi`; an axis the box misses comes out with
/// its low end above its high end.
pub(crate) fn clip_box(c: [i64; 3], e: i64, lo: [i64; 3], hi: [i64; 3]) -> ([i64; 3], [i64; 3]) {
    clip_at(c.map(|v| v * e), e, lo, hi)
}

/// [`clip_box`] for the cube of edge `e` whose minimum corner is `min`.
pub(crate) fn clip_at(min: [i64; 3], e: i64, lo: [i64; 3], hi: [i64; 3]) -> ([i64; 3], [i64; 3]) {
    (
        [0, 1, 2].map(|k| min[k].max(lo[k])),
        [0, 1, 2].map(|k| (min[k] + e - 1).min(hi[k])),
    )
}

/// A voxel or chunk coordinate of a checked shape or box as `i32`. Within the limits every
/// such coordinate fits (`±2^30` fixed units plus a radius of as much is `±2^23` voxels), so
/// the clamp never moves one.
pub(crate) fn to_i32(v: [i64; 3]) -> [i32; 3] {
    v.map(|c| {
        debug_assert!(i32::try_from(c).is_ok(), "coordinate {c} beyond i32");
        c.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
    })
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
