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
    /// The renderer's opaque walk passes through voxels of this material and composites them
    /// at `Material::opacity`; every other reader (collision, the coarse grid, collapse, the
    /// raycast) treats them as solid. The class is a table property, fixed for the world's
    /// life: `World::materials` hands out a shared reference and nothing mutates the table
    /// after `World::generate`.
    pub const TRANSLUCENT: Self = Self(2);

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
    /// The acoustic triple `[A, T, R]`. `A` is absorption per metre of path: a sound crossing
    /// the material keeps `(1 - A)` of its amplitude per metre, and a reflection off its face
    /// keeps `(1 - A)`. `T` is transmission, the amplitude factor of entering the material,
    /// applied once per run of it along a path. `R`, resonance, is unused. A row of zeros
    /// (transmission 0) blocks sound completely. The `vxe-audio` crate's `acoustics` module
    /// applies it; nothing in this crate reads it.
    pub acoustic: [f32; 3],
    pub payload: u64,
    /// Procedural surface detail for the renderer: feature scale in voxels, strength in
    /// `[0, 1]`. Strength 0 is no detail.
    pub detail: [f32; 2],
    /// The alpha the renderer composites a `TRANSLUCENT` voxel at, in `[0, 1]`; ignored
    /// without the flag.
    pub opacity: f32,
}

impl Material {
    /// A grey, rough, non-emissive row with no flags, no detail and opacity 1.
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
            detail: [0.0; 2],
            opacity: 1.0,
        }
    }

    /// Whether the row is `TRANSLUCENT`.
    pub fn is_translucent(&self) -> bool {
        self.flags.contains(MaterialFlags::TRANSLUCENT)
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

    /// Whether the renderer composites voxels of `id` instead of stopping at them.
    pub fn is_translucent(&self, id: MaterialId) -> bool {
        self.flags(id).contains(MaterialFlags::TRANSLUCENT)
    }
}
