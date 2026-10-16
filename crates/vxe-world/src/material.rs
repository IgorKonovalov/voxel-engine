//! The material table: one row per `MaterialId`, read by edits (flags), the renderer (albedo,
//! roughness, emission) and the consumer (the opaque payload).

/// A voxel's material. 0 is air.
pub type MaterialId = u16;

/// The air material: empty space.
pub const AIR: MaterialId = 0;

/// Per-material behaviour bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct MaterialFlags(pub u32);

impl MaterialFlags {
    /// `Carve` leaves voxels of this material in place.
    pub const INDESTRUCTIBLE: Self = Self(1);

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for MaterialFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// One row of the material table. Only `flags` affects edits; the rest is carried for the
/// renderer and the consumer. `payload` is opaque to the engine.
#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub name: String,
    pub albedo: [f32; 3],
    pub roughness: f32,
    pub emission: [f32; 3],
    pub hardness: f32,
    pub flags: MaterialFlags,
    /// Absorption, transmission, resonance.
    pub acoustic: [f32; 3],
    pub payload: u64,
}

impl Material {
    /// A grey, rough, non-emissive row with no flags.
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            albedo: [0.5; 3],
            roughness: 1.0,
            emission: [0.0; 3],
            hardness: 1.0,
            flags: MaterialFlags::empty(),
            acoustic: [0.0; 3],
            payload: 0,
        }
    }
}

/// Rows indexed by `MaterialId`. Row 0 is always air.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialTable {
    rows: Vec<Material>,
}

impl Default for MaterialTable {
    fn default() -> Self {
        Self::new()
    }
}

impl MaterialTable {
    /// A table holding only the air row.
    pub fn new() -> Self {
        Self {
            rows: vec![Material::named("air")],
        }
    }

    /// Appends a row and returns its id.
    ///
    /// # Panics
    /// If the table already holds `u16::MAX + 1` rows.
    pub fn push(&mut self, m: Material) -> MaterialId {
        let id = MaterialId::try_from(self.rows.len()).expect("material table is full");
        self.rows.push(m);
        id
    }

    pub fn get(&self, id: MaterialId) -> Option<&Material> {
        self.rows.get(usize::from(id))
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn rows(&self) -> &[Material] {
        &self.rows
    }

    /// Flags of `id`; an id with no row has none.
    pub fn flags(&self, id: MaterialId) -> MaterialFlags {
        self.get(id).map_or(MaterialFlags::empty(), |m| m.flags)
    }

    /// Whether `Carve` leaves voxels of `id` in place.
    pub fn is_indestructible(&self, id: MaterialId) -> bool {
        self.flags(id).contains(MaterialFlags::INDESTRUCTIBLE)
    }
}
