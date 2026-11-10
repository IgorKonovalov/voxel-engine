//! Entities: voxel models on rigid parts, posed in fixed point, and a ray query that names the
//! entity, part and voxel a ray hits (ADR-0050).
//!
//! This crate is headless and deterministic, as `vxe-world` is: no GPU API, no wall clock, no
//! unseeded randomness and no float in a result. A [`Model`] is up to 16 [`Part`]s, each a
//! [`vxe_world::Prefab`] volume with a parent, a pivot and a detail. A [`Pose`] places the root
//! with a [`Fixed3`](vxe_world::Fixed3) and a [`Quat`] in Q14, and each part relative to its
//! parent. An [`EntitySet`] holds placed models and answers [`EntitySet::raycast`].
//!
//! The world and the entities are queried separately, and the consumer orders the two: the
//! world first, then the entities with the world's distance as their limit.
//!
//! ```
//! use std::sync::Arc;
//! use vxe_entity::{EntitySet, Model, Part, Pose};
//! use vxe_world::{Face, Fixed3, Material, MaterialTable, Prefab, World};
//!
//! let mut table = MaterialTable::new();
//! let bone = table.push(Material::named("bone"));
//! let world = World::new(&table);
//!
//! // One 8-voxel cube turning about its centre, its minimum corner at voxel (100, 50, 100).
//! let cube = Arc::new(Prefab::new([8; 3], vec![bone; 512], None));
//! let part = Part { volume: cube, parent: None, pivot: [4 * 256; 3], detail: 1 };
//! let model = Arc::new(Model::new(vec![part]).unwrap());
//! let mut pose = Pose::rest(&model);
//! pose.root = Fixed3([104 * 256, 54 * 256, 104 * 256]);
//! let mut set = EntitySet::new();
//! let id = set.insert(model, pose).unwrap();
//!
//! let origin = Fixed3([90 * 256 + 128, 54 * 256 + 64, 104 * 256 + 64]);
//! let (dir, max_t) = ([256, 0, 0], 64 * 256);
//! let world_hit = world.raycast(origin, dir, max_t).unwrap();
//! let limit = world_hit.map_or(max_t, |h| h.t);
//! let hit = set.raycast(origin, dir, limit).unwrap();
//! assert_eq!((hit.entity, hit.voxel, hit.face), (id, [0, 4, 4], Some(Face::NegX)));
//! assert_eq!(hit.t, 9 * 256 + 128);
//! ```

mod model;
mod pose;
mod quat;
mod raycast;
mod set;

pub use model::{MAX_EDGE, MAX_PARTS, Model, ModelError, Part};
pub use pose::{PartPose, Pose, PoseError};
pub use quat::{Q14_ONE, Quat};
pub use set::{EntityHit, EntityId, EntitySet};
