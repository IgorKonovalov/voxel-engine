//! Asset import: MagicaVoxel `.vox` files and the map from their palette to material rows.
//!
//! [`VoxFile::parse`] reads models, the palette and the scene graph's placements;
//! [`VoxMap::to_prefab`] turns a model into a `vxe_world::Prefab` by mapping each palette
//! index to a material, to air, or to "leave the world alone". The mapping is explicit: an
//! index the map does not decide is an error, never a guess.
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
mod vox;

pub use map::{Slot, Unmapped, VoxMap};
pub use palette::DEFAULT_PALETTE;
pub use vox::{
    MAX_MODEL_EDGE, MAX_PLACEMENTS, MAX_TRANSLATION, Node, Placement, VoxError, VoxFile, VoxModel,
    decode_rotation,
};
