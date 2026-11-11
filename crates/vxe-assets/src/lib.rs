//! Asset import: MagicaVoxel `.vox` files, the map from their palette to material rows, rig
//! files and voxels drawn as text.
//!
//! [`VoxFile::parse`] reads models, the palette and the scene graph's placements, and
//! [`encode`] writes a file back as bytes the parser reads to an equal file;
//! [`VoxMap::to_prefab`] turns a model into a `vxe_world::Prefab` by mapping each palette
//! index to a material, to air, or to "leave the world alone". The mapping is explicit: an
//! index the map does not decide is an error, never a guess.
//!
//! [`Rig::parse`] reads a `.rig` text file (a model's parts, pivots, materials and clips) and
//! [`Rig::build`] turns it and its `.vox` into a `vxe_entity::Model`, a rest pose and clips.
//! [`VoxText`] is voxels drawn as text, converted to and from a [`VoxFile`].
//!
//! ```
//! use vxe_assets::{Slot, VoxMap, VoxModel};
//!
//! let model = VoxModel { size: [2, 1, 1], voxels: vec![[0, 0, 0, 7]] };
//! let mut map = VoxMap::new();
//! map.set(7, Slot::Material(3));
//! let prefab = map.to_prefab(&model).unwrap();
//! assert_eq!(prefab.get([0, 0, 0]), Some(3));
//! assert_eq!(prefab.get([1, 0, 0]), Some(0), "index 0 maps to air");
//! ```

mod map;
mod palette;
mod rig;
mod sines;
mod vox;
mod voxtext;
mod write;

pub use map::{Slot, Unmapped, VoxMap};
pub use palette::DEFAULT_PALETTE;
pub use rig::{
    Axis, DEFAULT_RATE, Line, MAX_DEGREES, Rig, RigClip, RigError, RigKey, RigMaterial, RigModel,
    RigPart, VoxSource, axis_rotation, key_rotation,
};
pub use vox::{
    MAX_MODEL_EDGE, MAX_PLACEMENTS, MAX_TRANSLATION, Node, Placement, VoxError, VoxFile, VoxModel,
    decode_rotation,
};
pub use voxtext::{TextColor, TextModel, VoxText, VoxTextError};
pub use write::{encode, encode_rotation};
