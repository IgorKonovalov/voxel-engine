//! The set of placed entities and the ray query over it.

use std::collections::BTreeMap;
use std::sync::Arc;

use vxe_world::{Face, Fixed3, MaterialId};

use crate::model::Model;
use crate::pose::{PartFrame, Pose, PoseError};
use crate::raycast::{Frac, box_span, part_raycast};

/// An entity's handle in its [`EntitySet`]. Ids are handed out in ascending order and never
/// reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntityId(pub u32);

/// The first entity voxel along a ray.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EntityHit {
    pub entity: EntityId,
    pub part: u8,
    /// The voxel in the part's volume.
    pub voxel: [u8; 3],
    /// The face of that voxel the ray entered through, in the part's space (`NegX` is the face
    /// at the voxel's minimum x). `None` when the ray starts inside the voxel.
    pub face: Option<Face>,
    pub material: MaterialId,
    /// Ray parameter where the ray enters the voxel, rounded down.
    pub t: i64,
}

#[derive(Clone, Debug)]
struct Entry {
    model: Arc<Model>,
    pose: Pose,
    frames: Vec<PartFrame>,
    /// A box holding every part under the pose, in 1/256 world voxel, half-open.
    lo: [i64; 3],
    hi: [i64; 3],
}

impl Entry {
    fn new(model: Arc<Model>, pose: Pose) -> Self {
        let frames = pose.compose(&model);
        let mut lo = [i64::MAX; 3];
        let mut hi = [i64::MIN; 3];
        for (i, f) in frames.iter().enumerate() {
            let r = model.reach(i);
            for k in 0..3 {
                lo[k] = lo[k].min(f.pivot[k] - r);
                hi[k] = hi[k].max(f.pivot[k] + r);
            }
        }
        Self {
            model,
            pose,
            frames,
            lo,
            hi,
        }
    }
}

/// Placed entities: a model each and its pose.
#[derive(Clone, Debug, Default)]
pub struct EntitySet {
    entries: BTreeMap<EntityId, Entry>,
    next: u32,
}

impl EntitySet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Places `model` at `pose`, refused when the pose does not fit the model.
    ///
    /// # Panics
    ///
    /// When `u32::MAX` entities have been inserted over the set's life.
    pub fn insert(&mut self, model: Arc<Model>, pose: Pose) -> Result<EntityId, PoseError> {
        pose.check(&model)?;
        let id = EntityId(self.next);
        self.next = self
            .next
            .checked_add(1)
            .expect("entity ids are never reused and u32::MAX of them are spent");
        self.entries.insert(id, Entry::new(model, pose));
        Ok(id)
    }

    /// Replaces an entity's pose.
    pub fn set_pose(&mut self, entity: EntityId, pose: Pose) -> Result<(), PoseError> {
        let e = self
            .entries
            .get_mut(&entity)
            .ok_or(PoseError::UnknownEntity { entity })?;
        pose.check(&e.model)?;
        *e = Entry::new(e.model.clone(), pose);
        Ok(())
    }

    /// Takes an entity out, returning its model and pose.
    pub fn remove(&mut self, entity: EntityId) -> Option<(Arc<Model>, Pose)> {
        self.entries.remove(&entity).map(|e| (e.model, e.pose))
    }

    pub fn get(&self, entity: EntityId) -> Option<(&Arc<Model>, &Pose)> {
        self.entries.get(&entity).map(|e| (&e.model, &e.pose))
    }

    /// The entities in ascending id order.
    pub fn iter(&self) -> impl Iterator<Item = (EntityId, &Arc<Model>, &Pose)> {
        self.entries.iter().map(|(&id, e)| (id, &e.model, &e.pose))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The first entity voxel the ray `origin + dir * t / 256`, `0 <= t <= max_t`, enters, with
    /// the argument conventions of `World::raycast` (with `|dir| == 256`, `t` is a distance in
    /// 1/256 voxel). Among hits at the same exact parameter the lower entity id wins, then the
    /// lower part. A voxel that is unwritten or air is empty.
    ///
    /// Integer arithmetic throughout: the same set and ray give the same hit on every platform,
    /// for origins in the `i32` voxel range and `dir` components below 2^40 in magnitude.
    pub fn raycast(&self, origin: Fixed3, dir: [i64; 3], max_t: i64) -> Option<EntityHit> {
        if max_t < 0 {
            return None;
        }
        let o = origin.0.map(i128::from);
        let d = dir.map(i128::from);
        let mut limit = Frac::int(max_t);
        let mut best: Option<(Frac, EntityHit)> = None;
        for (&entity, e) in &self.entries {
            let Some((enter, _, _)) = box_span(o, d, e.lo.map(i128::from), e.hi.map(i128::from))
            else {
                continue;
            };
            if limit.lt(enter) {
                continue;
            }
            for (i, (part, frame)) in e.model.parts().iter().zip(&e.frames).enumerate() {
                let Some(h) = part_raycast(part, frame, origin.0, dir, limit) else {
                    continue;
                };
                if best.as_ref().is_none_or(|(bt, _)| h.t.lt(*bt)) {
                    limit = h.t;
                    let hit = EntityHit {
                        entity,
                        part: i as u8,
                        voxel: h.voxel,
                        face: h.face,
                        material: h.material,
                        t: h.t.floor(),
                    };
                    best = Some((h.t, hit));
                }
            }
        }
        best.map(|(_, h)| h)
    }
}
