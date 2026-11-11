//! A model: up to [`MAX_PARTS`] rigid parts, each a voxel volume with a parent, a pivot and a
//! detail.

use std::sync::Arc;

use vxe_world::Prefab;
use vxe_world::hash::Fnv1a;

use crate::quat::isqrt;

/// Parts a model holds at most.
pub const MAX_PARTS: usize = 16;
/// A part volume's largest edge, in model voxels.
pub const MAX_EDGE: u32 = 64;

/// One rigid part of a [`Model`].
///
/// The part's own space is its volume's voxel grid: voxel `(i, j, k)` spans `[i, i + 1)` model
/// voxels on each axis, and a model voxel is `1 / detail` of a world voxel. The pivot is the
/// point the part turns about and the point its pose places.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    /// The voxels; at most [`MAX_EDGE`] a side. A voxel that is unwritten or [`vxe_world::AIR`]
    /// is empty.
    pub volume: Arc<Prefab>,
    /// The part this one hangs from: an earlier part. `None` hangs it from the entity's root.
    pub parent: Option<u8>,
    /// The pivot in the part's space, in 1/256 model voxel.
    pub pivot: [i32; 3],
    /// Model voxels per world voxel along an axis: 1, 2 or 4.
    pub detail: u8,
}

/// Why [`Model::new`] refused its parts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelError {
    /// No parts at all.
    NoParts,
    /// More than [`MAX_PARTS`] parts.
    TooManyParts { count: usize },
    /// A parent that is not an earlier part: itself, a later part or one past the end. Only such
    /// a parent can close a cycle, so every parent chain ends at the root.
    ParentCycle { part: u8, parent: u8 },
    /// A volume with an edge longer than [`MAX_EDGE`].
    OversizedVolume { part: u8, size: [u32; 3] },
    /// A detail other than 1, 2 or 4.
    BadDetail { part: u8, detail: u8 },
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::NoParts => write!(f, "a model needs at least one part"),
            Self::TooManyParts { count } => {
                write!(f, "{count} parts, a model holds at most {MAX_PARTS}")
            }
            Self::ParentCycle { part, parent } => {
                write!(
                    f,
                    "part {part} names part {parent} as its parent, not an earlier part"
                )
            }
            Self::OversizedVolume { part, size } => {
                write!(
                    f,
                    "part {part} is {size:?} voxels, at most {MAX_EDGE} a side"
                )
            }
            Self::BadDetail { part, detail } => {
                write!(f, "part {part} has detail {detail}, not 1, 2 or 4")
            }
        }
    }
}

impl std::error::Error for ModelError {}

/// A validated set of parts, shared between entities through `Arc`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Model {
    parts: Vec<Part>,
    /// Per part, a radius about the pivot that holds the whole volume under any rotation, in
    /// 1/256 world voxel, with a margin of one world voxel for the rounding of a pose.
    reach: Vec<i64>,
    id: u64,
}

impl Model {
    /// A model of `parts`, refused with a typed error when it is empty, holds more than
    /// [`MAX_PARTS`], names a parent that is not an earlier part, carries a volume over
    /// [`MAX_EDGE`] a side or a detail other than 1, 2 or 4.
    pub fn new(parts: Vec<Part>) -> Result<Self, ModelError> {
        if parts.is_empty() {
            return Err(ModelError::NoParts);
        }
        if parts.len() > MAX_PARTS {
            return Err(ModelError::TooManyParts { count: parts.len() });
        }
        for (i, p) in parts.iter().enumerate() {
            let part = i as u8;
            if let Some(parent) = p.parent
                && usize::from(parent) >= i
            {
                return Err(ModelError::ParentCycle { part, parent });
            }
            let size = p.volume.size();
            if size.iter().any(|&s| s > MAX_EDGE) {
                return Err(ModelError::OversizedVolume { part, size });
            }
            if !matches!(p.detail, 1 | 2 | 4) {
                return Err(ModelError::BadDetail {
                    part,
                    detail: p.detail,
                });
            }
        }
        let reach = parts.iter().map(part_reach).collect();
        let id = model_id(&parts);
        Ok(Self { parts, reach, id })
    }

    pub fn parts(&self) -> &[Part] {
        &self.parts
    }

    /// FNV-1a 64 over the part count (`u32`), then per part its volume's
    /// [`Prefab::id`] (`u64`), its parent (`u8`, 255 for none), its pivot (three `i32`) and its
    /// detail (`u8`), all little-endian.
    pub fn id(&self) -> u64 {
        self.id
    }

    pub(crate) fn reach(&self, part: usize) -> i64 {
        self.reach[part]
    }
}

fn model_id(parts: &[Part]) -> u64 {
    let mut h = Fnv1a::default();
    h.write(&(parts.len() as u32).to_le_bytes());
    for p in parts {
        h.write(&p.volume.id().to_le_bytes());
        h.write(&[p.parent.unwrap_or(u8::MAX)]);
        for c in p.pivot {
            h.write(&c.to_le_bytes());
        }
        h.write(&[p.detail]);
    }
    h.finish()
}

/// The distance from the pivot to the volume's farthest corner, rounded up, in 1/256 world
/// voxel, plus one world voxel.
fn part_reach(p: &Part) -> i64 {
    let size = p.volume.size();
    let d2: u128 = (0..3)
        .map(|k| {
            let lo = -i128::from(p.pivot[k]);
            let hi = i128::from(size[k]) * 256 - i128::from(p.pivot[k]);
            let m = lo.abs().max(hi.abs()).cast_unsigned();
            m * m
        })
        .sum();
    let r = isqrt(d2) + 1;
    let detail = u128::from(p.detail);
    (r.div_ceil(detail) + 256) as i64
}
