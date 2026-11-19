//! Palette rows (ADR-0065): every palette entry of a `.vox` becomes a material row, named by
//! its colour and its MagicaVoxel material and shared by every file whose entry reads the same.
//!
//! An entry's row is [`palette_row`]: the colour decoded from sRGB to linear albedo, and the
//! entry's `MATL` mapped onto the row's fields. `_glass` and `_media` are `TRANSLUCENT` at
//! opacity `1 - trans`; `_emit` emits `albedo * emit * 10^flux * k`, `k` the loader's emission
//! factor; `_metal` sets the roughness. The row's name carries the colour and the material, so
//! two entries that read the same share one row, and an application that resolved a file
//! before `World::generate` finds the rows again by name ([`resolve_existing`]) once the table
//! is fixed.

use std::collections::BTreeMap;

use vxe_world::{Material, MaterialFlags, MaterialId, MaterialTable};

use crate::map::{Slot, VoxMap};
use crate::vox::{VoxFile, VoxMaterialKind};

/// The emission factor `k` of [`PaletteRows`] unless the application sets one: an `_emit 1`
/// entry at `_flux 1` emits three times its albedo, about the fly-through's torch.
pub const DEFAULT_EMISSION_FACTOR: f32 = 0.3;

/// An sRGB channel byte as linear light, decoded exactly (IEC 61966-2-1) in `f64` and rounded
/// once to `f32`.
pub fn srgb_to_linear(c: u8) -> f32 {
    let v = f64::from(c) / 255.0;
    let l = if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    };
    l as f32
}

/// The row palette entry `index` of `file` becomes, with emission factor `k`. Its name is
/// `vox #rrggbb`, followed for a non-diffuse material by its kind and values: `metal 0.2`,
/// `glass 0.5 0.1` (trans, rough), `emit 1.0 2` (emit, flux), `media 0.5`.
pub fn palette_row(file: &VoxFile, index: u8, k: f32) -> Material {
    let [r, g, b, _] = file.palette[usize::from(index)];
    let albedo = [r, g, b].map(srgb_to_linear);
    let mut name = format!("vox #{r:02x}{g:02x}{b:02x}");
    let mut m = Material {
        albedo,
        ..Material::named("")
    };
    match file.materials[usize::from(index)].kind {
        VoxMaterialKind::Diffuse => {}
        VoxMaterialKind::Metal { rough } => {
            name += &format!(" metal {rough:?}");
            m.roughness = rough;
        }
        VoxMaterialKind::Glass { trans, rough } => {
            name += &format!(" glass {trans:?} {rough:?}");
            m.roughness = rough;
            m.flags = MaterialFlags::TRANSLUCENT;
            m.opacity = (1.0 - trans).clamp(0.0, 1.0);
        }
        VoxMaterialKind::Emit { emit, flux } => {
            name += &format!(" emit {emit:?} {flux}");
            let s = emit * 10f32.powi(i32::from(flux)) * k;
            m.emission = albedo.map(|a| a * s);
        }
        VoxMaterialKind::Media { trans } => {
            name += &format!(" media {trans:?}");
            m.flags = MaterialFlags::TRANSLUCENT;
            m.opacity = (1.0 - trans).clamp(0.0, 1.0);
        }
    }
    m.name = name;
    m
}

/// The row of each name in `table`, the first where two rows share one.
fn names(table: &MaterialTable) -> BTreeMap<String, MaterialId> {
    let mut by = BTreeMap::new();
    for (i, m) in table.rows().iter().enumerate() {
        if let Ok(id) = MaterialId::try_from(i) {
            by.entry(m.name.clone()).or_insert(id);
        }
    }
    by
}

/// A material table being built: palette entries resolved to rows, a row added the first time
/// its name is seen. Rows already in the table when it is wrapped count as seen, so a second
/// file with the same palette adds nothing.
#[derive(Debug)]
pub struct PaletteRows<'t> {
    table: &'t mut MaterialTable,
    by_name: BTreeMap<String, MaterialId>,
    /// `k` of [`palette_row`].
    pub emission_factor: f32,
    added: usize,
}

impl<'t> PaletteRows<'t> {
    /// Wraps `table` with emission factor `k`.
    pub fn new(table: &'t mut MaterialTable, k: f32) -> Self {
        let by_name = names(table);
        Self {
            table,
            by_name,
            emission_factor: k,
            added: 0,
        }
    }

    /// Every palette entry of `file` (indices 1 to 255) as a row, adding the rows not yet in
    /// the table, and the map: `explicit`'s entries where it decides one (a `material` line),
    /// the entry's row elsewhere. Index 0 is `explicit`'s.
    ///
    /// # Panics
    /// When the table would pass `u16::MAX + 1` rows.
    pub fn resolve(&mut self, file: &VoxFile, explicit: &VoxMap) -> VoxMap {
        let mut map = explicit.clone();
        for index in 1..=255u8 {
            if explicit.get(index) != Slot::Unset {
                continue;
            }
            let row = palette_row(file, index, self.emission_factor);
            let id = match self.by_name.get(&row.name) {
                Some(&id) => id,
                None => {
                    let name = row.name.clone();
                    let id = self.table.push(row);
                    self.by_name.insert(name, id);
                    self.added += 1;
                    id
                }
            };
            map.set(index, Slot::Material(id));
        }
        map
    }

    /// Rows added since the table was wrapped.
    pub fn added(&self) -> usize {
        self.added
    }

    /// The table as it stands.
    pub fn table(&self) -> &MaterialTable {
        self.table
    }
}

/// [`PaletteRows::resolve`] against a fixed table: each entry `explicit` leaves undecided maps
/// to the table's row of its name, and stays [`Slot::Unset`] when the table has none (an
/// entry recoloured since the table was built). The emission factor does not enter a name, so
/// none is needed.
pub fn resolve_existing(table: &MaterialTable, file: &VoxFile, explicit: &VoxMap) -> VoxMap {
    let by = names(table);
    let mut map = explicit.clone();
    for index in 1..=255u8 {
        if explicit.get(index) != Slot::Unset {
            continue;
        }
        if let Some(&id) = by.get(&palette_row(file, index, 0.0).name) {
            map.set(index, Slot::Material(id));
        }
    }
    map
}
