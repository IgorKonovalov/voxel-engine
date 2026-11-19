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
    ///
    /// This composes the whole pose for one point; [`Pose::compose`] composes it once for
    /// many.
    pub fn part_point(&self, model: &Model, part: usize, local: [i32; 3]) -> Option<Fixed3> {
        self.compose(model)?.part_point(part, local)
    }

    /// Every part of `model` placed by this pose, composed once, parents before children, so
    /// points on many parts cost one composition. `None` when the pose does not fit the model.
    pub fn compose<'m>(&self, model: &'m Model) -> Option<ComposedPose<'m>> {
        self.compose_with(model, ComposeScratch::default()).ok()
    }

    /// [`Pose::compose`] into `scratch`'s storage, so a caller composing every frame
    /// allocates only while the storage grows: [`ComposedPose::into_scratch`] hands it back.
    /// `Err` gives `scratch` back when the pose does not fit the model.
    pub fn compose_with<'m>(
        &self,
        model: &'m Model,
        scratch: ComposeScratch,
    ) -> Result<ComposedPose<'m>, ComposeScratch> {
        if self.check(model).is_err() {
            return Err(scratch);
        }
        let mut frames = scratch.frames;
        self.frames_into(model, &mut frames);
        Ok(ComposedPose { model, frames })
    }

    /// Every part's placement, parents before children. The pose must fit the model.
    pub(crate) fn frames(&self, model: &Model) -> Vec<PartFrame> {
        let mut frames = Vec::with_capacity(model.parts().len());
        self.frames_into(model, &mut frames);
        frames
    }

    /// [`Pose::frames`] into `frames`, cleared first.
    fn frames_into(&self, model: &Model, frames: &mut Vec<PartFrame>) {
        frames.clear();
        let root = PartFrame {
            pivot: self.root.0,
            q: self.root_q.normalised(),
            m: self.root_q.matrix(),
        };
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
    }
}

/// The storage a [`ComposedPose`] keeps its parts' placements in, reused across calls of
/// [`Pose::compose_with`]. Empty by default, which allocates nothing.
#[derive(Clone, Debug, Default)]
pub struct ComposeScratch {
    frames: Vec<PartFrame>,
}

/// A pose composed down its model's parts: each part's world pivot and rotation, from
/// [`Pose::compose`].
#[derive(Clone, Debug)]
pub struct ComposedPose<'m> {
    model: &'m Model,
    frames: Vec<PartFrame>,
}

impl ComposedPose<'_> {
    /// This composition's storage, for the next [`Pose::compose_with`].
    pub fn into_scratch(self) -> ComposeScratch {
        ComposeScratch {
            frames: self.frames,
        }
    }

    /// The model the pose was composed for.
    pub fn model(&self) -> &Model {
        self.model
    }

    /// The world point of `local` in part `part`'s space, exactly as [`Pose::part_point`]
    /// gives it. `None` when the part is past the model's end.
    pub fn part_point(&self, part: usize, local: [i32; 3]) -> Option<Fixed3> {
        let p = self.model.parts().get(part)?;
        let f = &self.frames[part];
        let detail = i64::from(p.detail);
        let coarse = i128::from(p.coarse);
        let rel = [0, 1, 2].map(|k| i64::from(local[k]) - i64::from(p.pivot[k]));
        // m * rel * coarse / (16384 * detail), the division done once.
        let one = i128::from(Q14_ONE) * i128::from(detail);
        let w = [0, 1, 2].map(|r| {
            let s: i128 = (0..3)
                .map(|c| i128::from(f.m[r][c]) * i128::from(rel[c]))
                .sum::<i128>()
                * coarse;
            f.pivot[r] + crate::quat::div_round(s, one) as i64
        });
        Some(Fixed3(w))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use vxe_world::Prefab;

    use super::*;
    use crate::model::Part;

    fn part(size: [u32; 3], parent: Option<u8>, pivot: [i32; 3], detail: u8) -> Part {
        let n = (size[0] * size[1] * size[2]) as usize;
        Part {
            volume: Arc::new(Prefab::new(size, vec![1; n], None)),
            parent,
            pivot,
            detail,
            coarse: 1,
        }
    }

    /// A torso, a head, two legs and two arms.
    fn six_part() -> Model {
        Model::new(vec![
            part([8, 6, 12], None, [1024, 768, 1536], 2),
            part([6, 6, 6], Some(0), [768, 768, 0], 2),
            part([3, 3, 8], Some(0), [384, 384, 2048], 2),
            part([3, 3, 8], Some(0), [384, 384, 2048], 2),
            part([2, 2, 7], Some(0), [256, 256, 1792], 1),
            part([2, 2, 7], Some(0), [256, 256, 1792], 4),
        ])
        .unwrap()
    }

    /// Sixteen parts: a chain of eight, and eight more hung off it, the details mixed.
    fn sixteen_part() -> Model {
        let parts = (0..16u8)
            .map(|i| {
                let parent = match i {
                    0 => None,
                    1..=7 => Some(i - 1),
                    _ => Some((i * 5) % 8),
                };
                let detail = [1, 2, 4][usize::from(i) % 3];
                part([4, 4, 6], parent, [512 + i32::from(i) * 37, 512, 0], detail)
            })
            .collect();
        Model::new(parts).unwrap()
    }

    /// SplitMix64.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }

        fn within(&mut self, span: i32) -> i32 {
            (self.next() % (2 * span as u64 + 1)) as i32 - span
        }

        fn quat(&mut self) -> Quat {
            Quat([0; 4].map(|_| self.within(16384) as i16)).normalised()
        }
    }

    fn random_pose(model: &Model, rng: &mut Rng) -> Pose {
        let mut p = Pose::rest(model);
        p.root = Fixed3([0; 3].map(|_| i64::from(rng.within(1 << 24))));
        p.root_q = rng.quat();
        for pp in &mut p.parts {
            pp.t = [0; 3].map(|_| rng.within(16 * 256));
            pp.q = rng.quat();
        }
        p
    }

    /// The point composed the slow way: the part's ancestors walked from the root down, this
    /// part alone, then its point.
    fn by_chain(model: &Model, pose: &Pose, part: usize, local: [i32; 3]) -> Fixed3 {
        let mut chain = vec![part];
        while let Some(i) = model.parts()[*chain.last().unwrap()].parent {
            chain.push(usize::from(i));
        }
        let mut f = PartFrame {
            pivot: pose.root.0,
            q: pose.root_q.normalised(),
            m: pose.root_q.matrix(),
        };
        for &k in chain.iter().rev() {
            let pp = &pose.parts[k];
            let t = rotate(&f.m, pp.t.map(i64::from));
            let q = f.q * pp.q;
            f = PartFrame {
                pivot: [0, 1, 2].map(|i| f.pivot[i] + t[i]),
                q,
                m: q.matrix(),
            };
        }
        let p = &model.parts()[part];
        let rel = [0, 1, 2].map(|k| i128::from(local[k]) - i128::from(p.pivot[k]));
        let one = i128::from(Q14_ONE) * i128::from(p.detail);
        Fixed3([0, 1, 2].map(|r| {
            let s: i128 = (0..3).map(|c| i128::from(f.m[r][c]) * rel[c]).sum();
            f.pivot[r] + crate::quat::div_round(s, one) as i64
        }))
    }

    /// Done-when 3b: one composition per pose gives every part's points bit for bit as
    /// `part_point` and as the ancestor chain composed on its own, on a six-part and a
    /// sixteen-part model at the rest pose and 24 seeded poses each.
    #[test]
    fn one_composition_matches_part_point_bit_for_bit() {
        let mut rng = Rng(0x0063_3b00);
        for model in [six_part(), sixteen_part()] {
            let mut poses = vec![Pose::rest(&model)];
            poses.extend((0..24).map(|_| random_pose(&model, &mut rng)));
            for pose in &poses {
                let c = pose.compose(&model).unwrap();
                for (k, p) in model.parts().iter().enumerate() {
                    let mut locals = vec![p.pivot];
                    for axis in 0..3 {
                        let mut l = p.pivot;
                        l[axis] += 1 << 16;
                        locals.push(l);
                    }
                    locals.extend((0..4).map(|_| [0; 3].map(|_| rng.within(64 * 256))));
                    for l in locals {
                        let one = c.part_point(k, l).unwrap();
                        assert_eq!(Some(one), pose.part_point(&model, k, l), "part {k} {l:?}");
                        assert_eq!(one, by_chain(&model, pose, k, l), "part {k} {l:?}");
                    }
                }
                assert_eq!(c.part_point(model.parts().len(), [0; 3]), None);
            }
        }
    }

    /// A pose that does not fit the model composes to nothing.
    #[test]
    fn a_pose_of_the_wrong_length_does_not_compose() {
        let model = six_part();
        let mut pose = Pose::rest(&model);
        pose.parts.pop();
        assert!(pose.compose(&model).is_none());
        assert_eq!(pose.part_point(&model, 0, [0; 3]), None);
    }
}
