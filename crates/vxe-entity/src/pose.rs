//! Poses in fixed point and their composition down a model's parts.

use vxe_world::Fixed3;

use crate::model::Model;
use crate::quat::{Mat3, Q14_ONE, Quat, rotate};
use crate::set::EntityId;

/// One part's pose relative to its parent.
///
/// The part's pivot sits at `t` from the parent's pivot (from the entity's root for a part with
/// no parent), measured along the parent's turned axes, and the part turns by `q` after its
/// parent's rotation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PartPose {
    /// The translation, in 1/256 world voxel.
    pub t: [i32; 3],
    pub q: Quat,
}

/// An entity's pose: where its root is and how it is turned, and one [`PartPose`] a part.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Pose {
    /// The root in world fixed point, 1/256 voxel.
    pub root: Fixed3,
    pub root_q: Quat,
    /// One per part of the model, in the model's part order.
    pub parts: Vec<PartPose>,
}

/// Why a pose does not fit a model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoseError {
    /// The pose holds `got` part poses for a model of `expected` parts.
    PartCount { expected: usize, got: usize },
    /// No entity of this id is in the set.
    UnknownEntity { entity: EntityId },
}

impl std::fmt::Display for PoseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::PartCount { expected, got } => {
                write!(
                    f,
                    "the pose has {got} part poses, the model {expected} parts"
                )
            }
            Self::UnknownEntity { entity } => write!(f, "no entity {}", entity.0),
        }
    }
}

impl std::error::Error for PoseError {}

/// A part's composed placement in the world: its pivot, its rotation and that rotation's
/// matrix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PartFrame {
    /// The pivot in world fixed point, 1/256 voxel.
    pub pivot: [i64; 3],
    pub q: Quat,
    pub m: Mat3,
}

impl Pose {
    /// The pose with every rotation and translation the identity, and the root at the origin.
    pub fn rest(model: &Model) -> Self {
        Self {
            root: Fixed3::default(),
            root_q: Quat::IDENTITY,
            parts: vec![PartPose::default(); model.parts().len()],
        }
    }

    /// `Ok` when the pose holds one part pose per part of `model`.
    pub fn check(&self, model: &Model) -> Result<(), PoseError> {
        let expected = model.parts().len();
        if self.parts.len() == expected {
            Ok(())
        } else {
            Err(PoseError::PartCount {
                expected,
                got: self.parts.len(),
            })
        }
    }

    /// The world point of `local`, a point in part `part`'s space in 1/256 model voxel (the
    /// pivot is in the same units), rounded half away from zero to 1/256 world voxel. `None`
    /// when the part is past the model's end or the pose does not fit the model.
    pub fn part_point(&self, model: &Model, part: usize, local: [i32; 3]) -> Option<Fixed3> {
        self.check(model).ok()?;
        let p = model.parts().get(part)?;
        let frames = self.compose(model);
        let f = &frames[part];
        let detail = i64::from(p.detail);
        let rel = [0, 1, 2].map(|k| i64::from(local[k]) - i64::from(p.pivot[k]));
        // m * rel / (16384 * detail), the division done once.
        let one = i128::from(Q14_ONE) * i128::from(detail);
        let w = [0, 1, 2].map(|r| {
            let s: i128 = (0..3)
                .map(|c| i128::from(f.m[r][c]) * i128::from(rel[c]))
                .sum();
            f.pivot[r] + crate::quat::div_round(s, one) as i64
        });
        Some(Fixed3(w))
    }

    /// Every part's placement, parents before children. The pose must fit the model.
    pub(crate) fn compose(&self, model: &Model) -> Vec<PartFrame> {
        let root = PartFrame {
            pivot: self.root.0,
            q: self.root_q.normalised(),
            m: self.root_q.matrix(),
        };
        let mut frames: Vec<PartFrame> = Vec::with_capacity(model.parts().len());
        for (part, pp) in model.parts().iter().zip(&self.parts) {
            let parent = match part.parent {
                Some(i) => frames[usize::from(i)],
                None => root,
            };
            let t = rotate(&parent.m, pp.t.map(i64::from));
            let q = parent.q * pp.q;
            frames.push(PartFrame {
                pivot: [0, 1, 2].map(|k| parent.pivot[k] + t[k]),
                q,
                m: q.matrix(),
            });
        }
        frames
    }
}
