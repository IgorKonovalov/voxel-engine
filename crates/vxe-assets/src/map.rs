//! The palette-to-material map: what each of a `.vox` file's 256 palette indices becomes in a
//! prefab.

use vxe_world::{AIR, MaterialId, Prefab};

use crate::vox::VoxModel;

/// What one palette index becomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Slot {
    /// Not decided; a model that uses the index cannot be converted.
    #[default]
    Unset,
    /// The voxel is written as air.
    Air,
    /// The voxel is written as this material.
    Material(MaterialId),
    /// The voxel is not written: its mask bit is clear and a stamp leaves the world alone.
    Skip,
}

/// One [`Slot`] per palette index. Index 0 is every cell the model lists no voxel for.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VoxMap([Slot; 256]);

impl Default for VoxMap {
    fn default() -> Self {
        Self::new()
    }
}

/// The palette indices a model uses that the map leaves [`Slot::Unset`], ascending and
/// deduplicated.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Unmapped(pub Vec<u8>);

impl std::fmt::Display for Unmapped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "palette indices with no mapping: {:?}", self.0)
    }
}

impl std::error::Error for Unmapped {}

impl VoxMap {
    /// Index 0 (empty cells) as [`Slot::Air`], every other index [`Slot::Unset`].
    pub fn new() -> Self {
        let mut s = [Slot::Unset; 256];
        s[0] = Slot::Air;
        Self(s)
    }

    pub fn set(&mut self, index: u8, slot: Slot) -> &mut Self {
        self.0[usize::from(index)] = slot;
        self
    }

    pub fn get(&self, index: u8) -> Slot {
        self.0[usize::from(index)]
    }

    /// The model as a prefab of the same size, x fastest. A cell no voxel names is index 0;
    /// where the model lists one cell twice, the later entry wins; a voxel outside the size
    /// (which [`crate::VoxFile::parse`] never yields) is ignored. Fails with every
    /// [`Slot::Unset`] index the model uses (index 0 counts when some cell is empty).
    pub fn to_prefab(&self, model: &VoxModel) -> Result<Prefab, Unmapped> {
        let s = model.size.map(|v| v as usize);
        let n = s[0] * s[1] * s[2];
        let mut index = vec![0u8; n];
        for &[x, y, z, i] in &model.voxels {
            if (0..3).any(|k| u32::from([x, y, z][k]) >= model.size[k]) {
                continue;
            }
            let at = usize::from(x) + s[0] * (usize::from(y) + s[1] * usize::from(z));
            index[at] = i;
        }
        let mut used = [false; 256];
        for &i in &index {
            used[usize::from(i)] = true;
        }
        let unset: Vec<u8> = (0..=255u8)
            .filter(|&i| used[usize::from(i)] && self.get(i) == Slot::Unset)
            .collect();
        if !unset.is_empty() {
            return Err(Unmapped(unset));
        }
        let mut voxels = vec![AIR; n];
        let mut mask = vec![0u64; n.div_ceil(64)];
        for (k, &i) in index.iter().enumerate() {
            let m = match self.get(i) {
                Slot::Air => AIR,
                Slot::Material(m) => m,
                Slot::Skip | Slot::Unset => continue,
            };
            voxels[k] = m;
            mask[k / 64] |= 1 << (k % 64);
        }
        Ok(Prefab::new(model.size, voxels, Some(mask)))
    }
}
