//! The entity mirror (ADR-0050): voxel-model parts in a pool of their own, one instance a posed
//! part, and a tree over the entities' boxes, all in one storage buffer that `entity.wgsl`
//! walks after the world march.
//!
//! The buffer's words, from 0:
//!
//! - the tree: [`ENTITY_NODES`] nodes of [`NODE_WORDS`] words, the root at node 0. A node is
//!   its box's minimum and maximum in world voxels (six `f32`), then for an inner node its two
//!   children's indices, for a leaf its first instance with [`LEAF_BIT`] set and its instance
//!   count, with [`LEAF_MOVED`] set when any of the entity's parts moved. The last node's
//!   slot is the header ([`TREE_HEADER_WORD`]): the entities drawn, the nodes, the instances, and
//!   whether any instance moved.
//! - the instances from [`INSTANCE_WORD`]: [`MAX_ENTITY_PARTS`] of [`INSTANCE_WORDS`] words. An
//!   instance is its world-to-part transform (3 x 4 `f32`, row-major, model voxels from world
//!   voxels), the part-to-world transform of the frame before (3 x 4 `f32`), the word its
//!   volume starts at, the entity's index in the drawn order, its flags ([`INSTANCE_MOVED`]),
//!   `1 / detail` as `f32`, and the volume's size in model voxels.
//! - the pool from [`POOL_WORD`]: each volume once, keyed by its prefab's id. A volume is a
//!   header of [`VOLUME_HEADER_WORDS`] words (its size, then the word offsets of its
//!   occupancy bits, block bits, palette indices and palette from the volume's start), one
//!   occupancy bit a voxel (x fastest), one bit per 4 x 4 x 4 block that holds a voxel, one
//!   palette byte a voxel, and a palette of 256 material ids. A 16-voxel cube is
//!   `32 + 512 + 8 + 4096 + 1024` = 5672 bytes.

use std::collections::HashMap;

use vxe_entity::{EntityId, EntitySet, Model};
use vxe_world::{AIR, MaterialTable, Prefab};

/// Entities drawn at most; the rest are counted ([`EntityMirror::entities_over_cap`]).
pub const MAX_ENTITIES: usize = 64;
/// Parts (instances) drawn at most, over every entity.
pub const MAX_ENTITY_PARTS: usize = 512;
/// The pool's bytes by default.
pub const DEFAULT_ENTITY_POOL_BYTES: u64 = 8 << 20;

/// Tree nodes the buffer holds room for: `2 * MAX_ENTITIES - 1` nodes and the header.
pub(crate) const ENTITY_NODES: usize = 2 * MAX_ENTITIES;
/// Words of a tree node.
pub(crate) const NODE_WORDS: usize = 8;
/// The header's first word: the last node's slot.
pub(crate) const TREE_HEADER_WORD: usize = (ENTITY_NODES - 1) * NODE_WORDS;
/// A leaf's first-instance word carries this bit.
pub(crate) const LEAF_BIT: u32 = 1 << 31;
/// A leaf's count word carries this bit when any of its entity's parts moved since the call
/// before (a shadow ray that enters its box marks its pixel fresh, `entity.wgsl`).
pub(crate) const LEAF_MOVED: u32 = 1 << 31;
/// The first instance's word.
pub(crate) const INSTANCE_WORD: usize = ENTITY_NODES * NODE_WORDS;
/// Words of an instance (128 bytes).
pub(crate) const INSTANCE_WORDS: usize = 32;
/// The pool's first word.
pub(crate) const POOL_WORD: usize = INSTANCE_WORD + MAX_ENTITY_PARTS * INSTANCE_WORDS;
/// Words of a volume's header.
pub(crate) const VOLUME_HEADER_WORDS: usize = 8;
/// An instance's flag: its transform differs from the frame before's.
pub(crate) const INSTANCE_MOVED: u32 = 1;
/// A volume's palette entries.
const PALETTE: usize = 256;

/// A model uploaded to an [`EntityMirror`]: an index into its models.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ModelHandle(pub u32);

/// Why [`EntityMirror::upload_model`] refused a model. Nothing is written on a refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityUploadError {
    /// The pool holds `free` bytes and the model's new volumes need `needed`.
    PoolFull { needed: u64, free: u64 },
    /// Part `part` holds more than 255 materials.
    Palette { part: usize },
    /// Part `part` holds a material the table has no row for.
    UnknownMaterial { part: usize, material: u16 },
}

impl std::fmt::Display for EntityUploadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::PoolFull { needed, free } => {
                write!(
                    f,
                    "the entity pool is full: {needed} bytes needed, {free} free"
                )
            }
            Self::Palette { part } => write!(f, "part {part} holds more than 255 materials"),
            Self::UnknownMaterial { part, material } => {
                write!(f, "part {part} holds material {material}, which has no row")
            }
        }
    }
}

impl std::error::Error for EntityUploadError {}

/// A part's placement as `f32` affine maps: `part_to_world` takes a point in model voxels to
/// world voxels; `world_to_part` is its inverse. Rows of `[m | t]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PartXform {
    pub(crate) part_to_world: [[f32; 4]; 3],
    pub(crate) world_to_part: [[f32; 4]; 3],
}

impl PartXform {
    /// The part's placement from a composed pose, through `ComposedPose::part_point` (exact in
    /// fixed point): its pivot's world point and the world points 256 model voxels along each
    /// axis from it.
    fn of(pose: &vxe_entity::ComposedPose<'_>, part: usize) -> Option<Self> {
        let p = &pose.model().parts()[part];
        let fx = |l: [i32; 3]| pose.part_point(part, l).map(|f| f.0);
        let base = fx(p.pivot)?;
        const REACH: i32 = 1 << 16;
        let mut m = [[0f32; 3]; 3];
        for (k, col) in (0..3).zip([0usize, 1, 2]) {
            let mut l = p.pivot;
            l[k] += REACH;
            let q = fx(l)?;
            for r in 0..3 {
                // World 1/256 voxels per 256 model voxels: world voxels per model voxel.
                m[r][col] = (q[r] - base[r]) as f32 / (256.0 * 256.0);
            }
        }
        let pivot_w = base.map(|v| v as f32 / 256.0);
        Some(Self::from_parts(
            m,
            p.pivot.map(|v| v as f32 / 256.0),
            pivot_w,
            p.detail,
        ))
    }

    /// The placement whose linear part is `m` (a rotation over `detail`) and which takes the
    /// model point `pivot_m` to the world point `pivot_w`.
    fn from_parts(m: [[f32; 3]; 3], pivot_m: [f32; 3], pivot_w: [f32; 3], detail: u8) -> Self {
        let mut p2w = [[0f32; 4]; 3];
        for r in 0..3 {
            p2w[r][..3].copy_from_slice(&m[r]);
            p2w[r][3] = pivot_w[r] - (0..3).map(|c| m[r][c] * pivot_m[c]).sum::<f32>();
        }
        // The linear part is a rotation over `detail`, so its inverse is its transpose times
        // `detail^2`.
        let d2 = f32::from(detail) * f32::from(detail);
        let mut w2p = [[0f32; 4]; 3];
        for r in 0..3 {
            for c in 0..3 {
                w2p[r][c] = m[c][r] * d2;
            }
            w2p[r][3] = pivot_m[r] - (0..3).map(|c| w2p[r][c] * pivot_w[c]).sum::<f32>();
        }
        Self {
            part_to_world: p2w,
            world_to_part: w2p,
        }
    }

    /// The world box of `[0, size]` in model voxels through `xf`.
    fn world_box(xf: &[[f32; 4]; 3], size: [u32; 3]) -> [[f32; 3]; 2] {
        let mut b = [[f32::MAX; 3], [f32::MIN; 3]];
        for i in 0..8 {
            let c = [0, 1, 2].map(|k| if i >> k & 1 == 1 { size[k] as f32 } else { 0.0 });
            for r in 0..3 {
                let w = xf[r][3] + (0..3).map(|k| xf[r][k] * c[k]).sum::<f32>();
                b[0][r] = b[0][r].min(w);
                b[1][r] = b[1][r].max(w);
            }
        }
        b
    }
}

/// One entity's parts placed for drawing, as `f32` affine maps: from one pose
/// ([`DisplayPose::of`]) or between two ([`DisplayPose::between`]), so a viewer draws between
/// two ticks of a fixed-point clip. Drawing only: the entity queries read the pose in the set.
///
/// A display pose kept across frames and refreshed with [`DisplayPose::of_into`] or
/// [`DisplayPose::between_into`] reuses its storage: once grown to its model's part count it
/// allocates nothing.
#[derive(Clone, Debug, Default)]
pub struct DisplayPose {
    /// Each part's placement and its detail.
    parts: Vec<(PartXform, u8)>,
    /// The two compositions' storage, kept for the next refresh.
    scratch: [vxe_entity::ComposeScratch; 2],
}

impl PartialEq for DisplayPose {
    /// Two display poses are equal when they place the same parts alike; the kept storage
    /// does not count.
    fn eq(&self, o: &Self) -> bool {
        self.parts == o.parts
    }
}

impl DisplayPose {
    /// `model`'s parts as `pose` places them; `None` when the pose does not fit the model.
    pub fn of(model: &Model, pose: &vxe_entity::Pose) -> Option<Self> {
        let mut d = Self::default();
        d.of_into(model, pose).then_some(d)
    }

    /// [`DisplayPose::of`] into `self`, reusing its storage; `false`, and `self` emptied, when
    /// the pose does not fit the model.
    pub fn of_into(&mut self, model: &Model, pose: &vxe_entity::Pose) -> bool {
        self.parts.clear();
        let scratch = std::mem::take(&mut self.scratch[0]);
        let c = match pose.compose_with(model, scratch) {
            Ok(c) => c,
            Err(s) => {
                self.scratch[0] = s;
                return false;
            }
        };
        let mut fits = true;
        for k in 0..model.parts().len() {
            let Some(x) = PartXform::of(&c, k) else {
                fits = false;
                break;
            };
            self.parts.push((x, model.parts()[k].detail));
        }
        self.scratch[0] = c.into_scratch();
        if !fits {
            self.parts.clear();
        }
        fits
    }

    /// `model`'s parts a fraction `alpha` (0 to 1) of the way from pose `a` to pose `b`: per
    /// part, its pivot's world point linear between the two, and its turn a normalised linear
    /// blend of the two turns along the shorter arc. Each part blends its own composed
    /// placement, so a child stays on its parent's pivot only to the blend's accuracy. `None`
    /// when a pose does not fit the model.
    pub fn between(
        model: &Model,
        a: &vxe_entity::Pose,
        b: &vxe_entity::Pose,
        alpha: f32,
    ) -> Option<Self> {
        let mut d = Self::default();
        d.between_into(model, a, b, alpha).then_some(d)
    }

    /// [`DisplayPose::between`] into `self`, reusing its storage; `false`, and `self` emptied,
    /// when a pose does not fit the model.
    pub fn between_into(
        &mut self,
        model: &Model,
        a: &vxe_entity::Pose,
        b: &vxe_entity::Pose,
        alpha: f32,
    ) -> bool {
        self.parts.clear();
        let [sa, sb] = std::mem::take(&mut self.scratch);
        let (ca, cb) = match (a.compose_with(model, sa), b.compose_with(model, sb)) {
            (Ok(ca), Ok(cb)) => (ca, cb),
            (ra, rb) => {
                self.scratch = [
                    ra.map_or_else(|s| s, |c| c.into_scratch()),
                    rb.map_or_else(|s| s, |c| c.into_scratch()),
                ];
                return false;
            }
        };
        let fits = self.blend_parts(model, &ca, &cb, f64::from(alpha));
        self.scratch = [ca.into_scratch(), cb.into_scratch()];
        if !fits {
            self.parts.clear();
        }
        fits
    }

    /// Pushes each part of `model` blended `alpha` of the way from `ca` to `cb`; `false` at
    /// the first part either composition cannot place.
    fn blend_parts(
        &mut self,
        model: &Model,
        ca: &vxe_entity::ComposedPose<'_>,
        cb: &vxe_entity::ComposedPose<'_>,
        alpha: f64,
    ) -> bool {
        for (k, p) in model.parts().iter().enumerate() {
            let (Some(xa), Some(xb)) = (PartXform::of(ca, k), PartXform::of(cb, k)) else {
                return false;
            };
            let d = f64::from(p.detail);
            let pivot_m = p.pivot.map(|v| v as f32 / 256.0);
            let qa = quat_of(&xa.part_to_world, d);
            let mut qb = quat_of(&xb.part_to_world, d);
            if (0..4).map(|i| qa[i] * qb[i]).sum::<f64>() < 0.0 {
                qb = qb.map(|v| -v);
            }
            let q: [f64; 4] = std::array::from_fn(|i| qa[i] + (qb[i] - qa[i]) * alpha);
            let r = matrix_of(q);
            let m = r.map(|row| row.map(|v| (v / d) as f32));
            let (pa, pb) = (
                apply(&xa.part_to_world, pivot_m),
                apply(&xb.part_to_world, pivot_m),
            );
            let pivot_w: [f32; 3] = std::array::from_fn(|i| {
                (f64::from(pa[i]) + (f64::from(pb[i]) - f64::from(pa[i])) * alpha) as f32
            });
            self.parts.push((
                PartXform::from_parts(m, pivot_m, pivot_w, p.detail),
                p.detail,
            ));
        }
        true
    }

    /// The parts placed.
    pub fn len(&self) -> usize {
        self.parts.len()
    }

    /// Whether no part is placed.
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    /// Part `part`'s map from model voxels to world voxels, rows of `[m | t]`.
    pub fn part_to_world(&self, part: usize) -> Option<[[f32; 4]; 3]> {
        self.parts.get(part).map(|(x, _)| x.part_to_world)
    }
}

/// `xf` applied to the point `p`.
fn apply(xf: &[[f32; 4]; 3], p: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|r| xf[r][3] + (0..3).map(|c| xf[r][c] * p[c]).sum::<f32>())
}

/// The unit quaternion `(w, x, y, z)` of the rotation `detail` times `xf`'s linear part
/// (Shepperd's method: the largest of the four diagonal sums is the one divided by).
fn quat_of(xf: &[[f32; 4]; 3], detail: f64) -> [f64; 4] {
    let m = |r: usize, c: usize| f64::from(xf[r][c]) * detail;
    let tr = m(0, 0) + m(1, 1) + m(2, 2);
    let q = if tr > 0.0 {
        let s = (tr + 1.0).sqrt() * 2.0;
        [
            0.25 * s,
            (m(2, 1) - m(1, 2)) / s,
            (m(0, 2) - m(2, 0)) / s,
            (m(1, 0) - m(0, 1)) / s,
        ]
    } else if m(0, 0) > m(1, 1) && m(0, 0) > m(2, 2) {
        let s = (1.0 + m(0, 0) - m(1, 1) - m(2, 2)).sqrt() * 2.0;
        [
            (m(2, 1) - m(1, 2)) / s,
            0.25 * s,
            (m(0, 1) + m(1, 0)) / s,
            (m(0, 2) + m(2, 0)) / s,
        ]
    } else if m(1, 1) > m(2, 2) {
        let s = (1.0 + m(1, 1) - m(0, 0) - m(2, 2)).sqrt() * 2.0;
        [
            (m(0, 2) - m(2, 0)) / s,
            (m(0, 1) + m(1, 0)) / s,
            0.25 * s,
            (m(1, 2) + m(2, 1)) / s,
        ]
    } else {
        let s = (1.0 + m(2, 2) - m(0, 0) - m(1, 1)).sqrt() * 2.0;
        [
            (m(1, 0) - m(0, 1)) / s,
            (m(0, 2) + m(2, 0)) / s,
            (m(1, 2) + m(2, 1)) / s,
            0.25 * s,
        ]
    };
    let n = q.iter().map(|v| v * v).sum::<f64>().sqrt();
    q.map(|v| v / n)
}

/// The rotation matrix of the quaternion `(w, x, y, z)`, normalised first.
fn matrix_of(q: [f64; 4]) -> [[f64; 3]; 3] {
    let n = q.iter().map(|v| v * v).sum::<f64>().sqrt();
    let [w, x, y, z] = q.map(|v| v / n);
    [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - w * z),
            2.0 * (x * z + w * y),
        ],
        [
            2.0 * (x * y + w * z),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - w * x),
        ],
        [
            2.0 * (x * z - w * y),
            2.0 * (y * z + w * x),
            1.0 - 2.0 * (x * x + y * y),
        ],
    ]
}

/// One uploaded volume's place in the pool.
#[derive(Clone, Copy, Debug)]
struct Volume {
    word: u32,
    size: [u32; 3],
}

/// The words of `prefab` as a pool volume, or the part's refusal.
fn volume_words(
    prefab: &Prefab,
    table: &MaterialTable,
    part: usize,
) -> Result<Vec<u32>, EntityUploadError> {
    let size = prefab.size();
    let n = (size[0] * size[1] * size[2]) as usize;
    let blocks = size.map(|s| s.div_ceil(4));
    let nb = (blocks[0] * blocks[1] * blocks[2]) as usize;
    let occ = VOLUME_HEADER_WORDS;
    let blk = occ + n.div_ceil(32);
    let idx = blk + nb.div_ceil(32);
    let pal = idx + n.div_ceil(4);
    let mut w = vec![0u32; pal + PALETTE];
    w[..VOLUME_HEADER_WORDS].copy_from_slice(&[
        size[0], size[1], size[2], occ as u32, blk as u32, idx as u32, pal as u32, 0,
    ]);
    let mut palette: Vec<u16> = Vec::new();
    let mut i = 0usize;
    for z in 0..size[2] {
        for y in 0..size[1] {
            for x in 0..size[0] {
                let m = prefab.get([x as i32, y as i32, z as i32]).unwrap_or(AIR);
                if m != AIR {
                    if table.get(m).is_none() {
                        return Err(EntityUploadError::UnknownMaterial { part, material: m });
                    }
                    let k = match palette.iter().position(|&p| p == m) {
                        Some(k) => k,
                        None => {
                            if palette.len() == PALETTE - 1 {
                                return Err(EntityUploadError::Palette { part });
                            }
                            palette.push(m);
                            palette.len() - 1
                        }
                    };
                    w[occ + i / 32] |= 1 << (i % 32);
                    let b = (x / 4 + blocks[0] * (y / 4 + blocks[1] * (z / 4))) as usize;
                    w[blk + b / 32] |= 1 << (b % 32);
                    w[idx + i / 4] |= (k as u32) << (8 * (i % 4));
                }
                i += 1;
            }
        }
    }
    for (k, m) in palette.iter().enumerate() {
        w[pal + k] = u32::from(*m);
    }
    Ok(w)
}

/// The entity mirror: one storage buffer holding the tree, the instances and the pool
/// (module docs), the volumes uploaded, and the instances' transforms of the frame before.
pub struct EntityMirror {
    buffer: wgpu::Buffer,
    pool_words: usize,
    used_words: usize,
    volumes: HashMap<u64, Volume>,
    /// Per model, each part's volume.
    models: Vec<Vec<Volume>>,
    /// Each drawn part's part-to-world of the last [`EntityMirror::set_entities`].
    prev: HashMap<(EntityId, usize), [[f32; 4]; 3]>,
    /// [`EntityMirror::set_entities`]'s buffers, cleared and refilled each call so the frame
    /// path allocates only when a list outgrows every list before it.
    scratch: Scratch,
    drawn: usize,
    parts: usize,
    over: usize,
    moved: bool,
}

/// The working buffers of one [`EntityMirror::set_entities`] or
/// [`EntityMirror::set_entity_displays`] call. `prev` fills with this
/// call's transforms and is swapped with the mirror's map at the end, so the two maps trade
/// places each frame instead of one being built fresh.
#[derive(Default)]
struct Scratch {
    inst: Vec<u32>,
    boxes: Vec<([[f32; 3]; 2], u32, u32)>,
    prev: HashMap<(EntityId, usize), [[f32; 4]; 3]>,
    nodes: Vec<[u32; NODE_WORDS]>,
    order: Vec<usize>,
    tree: Vec<u32>,
}

impl EntityMirror {
    /// An empty mirror whose pool holds `pool_bytes` (rounded down to whole words).
    pub fn new(device: &wgpu::Device, pool_bytes: u64) -> Self {
        let pool_words = (pool_bytes / 4) as usize;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("entities"),
            size: ((POOL_WORD + pool_words) * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            buffer,
            pool_words,
            used_words: 0,
            volumes: HashMap::new(),
            models: Vec::new(),
            prev: HashMap::new(),
            scratch: Scratch::default(),
            drawn: 0,
            parts: 0,
            over: 0,
            moved: false,
        }
    }

    /// The buffer `entity.wgsl` binds.
    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    /// Bytes the mirror's buffer occupies: the tree, the instances and the pool.
    pub fn bytes(&self) -> u64 {
        self.buffer.size()
    }

    /// Bytes of the pool in use.
    pub fn pool_used_bytes(&self) -> u64 {
        (self.used_words * 4) as u64
    }

    /// The bytes [`EntityMirror::upload_model`] would write for `prefab`: its volume's words.
    pub fn volume_bytes(prefab: &Prefab) -> u64 {
        let table = MaterialTable::new();
        // Size alone sets the layout; the materials only fill it.
        let empty = Prefab::new(prefab.size(), vec![AIR; prefab.len()], None);
        (volume_words(&empty, &table, 0).map_or(0, |w| w.len()) * 4) as u64
    }

    /// Writes each part's volume of `model` into the pool once, keyed by its prefab's id (two
    /// models sharing a prefab share its words). Every material must have a row in `table`.
    pub fn upload_model(
        &mut self,
        queue: &wgpu::Queue,
        model: &Model,
        table: &MaterialTable,
    ) -> Result<ModelHandle, EntityUploadError> {
        let mut fresh: Vec<(u64, Vec<u32>)> = Vec::new();
        for (part, p) in model.parts().iter().enumerate() {
            let id = p.volume.id();
            if self.volumes.contains_key(&id) || fresh.iter().any(|(f, _)| *f == id) {
                continue;
            }
            fresh.push((id, volume_words(&p.volume, table, part)?));
        }
        let needed: usize = fresh.iter().map(|(_, w)| w.len()).sum();
        let free = self.pool_words - self.used_words;
        if needed > free {
            return Err(EntityUploadError::PoolFull {
                needed: (needed * 4) as u64,
                free: (free * 4) as u64,
            });
        }
        for (id, words) in fresh {
            let word = POOL_WORD + self.used_words;
            queue.write_buffer(
                &self.buffer,
                (word * 4) as u64,
                bytemuck::cast_slice(&words),
            );
            self.used_words += words.len();
            self.volumes.insert(
                id,
                Volume {
                    word: word as u32,
                    size: [words[0], words[1], words[2]],
                },
            );
        }
        let parts = model
            .parts()
            .iter()
            .map(|p| self.volumes[&p.volume.id()])
            .collect();
        self.models.push(parts);
        Ok(ModelHandle(self.models.len() as u32 - 1))
    }

    /// Writes one instance a part of the entities in `list` (in its order) as `set` poses
    /// them, and the tree over their boxes. An entity past [`MAX_ENTITIES`], or whose parts
    /// would pass [`MAX_ENTITY_PARTS`], is counted and not drawn; so is an entry whose entity
    /// is not in `set` or whose handle is not this mirror's. An entity's box holds its parts
    /// now and in the call before.
    pub fn set_entities(
        &mut self,
        queue: &wgpu::Queue,
        set: &EntitySet,
        list: &[(EntityId, ModelHandle)],
    ) {
        let shown: Vec<_> = list
            .iter()
            .map(|&(id, h)| {
                let d = set.get(id).and_then(|(m, p)| DisplayPose::of(m, p));
                (id, h, d)
            })
            .collect();
        self.write(queue, shown.iter().map(|(id, h, d)| (*id, *h, d.as_ref())));
    }

    /// [`EntityMirror::set_entities`] with each entity's parts placed by a [`DisplayPose`]
    /// (between two ticks, say) in place of a pose in a set. An entry whose display pose has
    /// not its model's part count is counted and not drawn. `list` is any iterator, so a
    /// caller keeping its display poses across frames passes them with no list to build.
    pub fn set_entity_displays<'a>(
        &mut self,
        queue: &wgpu::Queue,
        list: impl IntoIterator<Item = (EntityId, ModelHandle, &'a DisplayPose)>,
    ) {
        self.write(queue, list.into_iter().map(|(id, h, d)| (id, h, Some(d))));
    }

    fn write<'a>(
        &mut self,
        queue: &wgpu::Queue,
        list: impl Iterator<Item = (EntityId, ModelHandle, Option<&'a DisplayPose>)>,
    ) {
        // Taken out of `self` for the call so `self.models` and `self.prev` stay readable;
        // `mem::take` of the empty default allocates nothing, and the buffers go back below.
        let mut s = std::mem::take(&mut self.scratch);
        let Scratch {
            inst,
            boxes,
            prev,
            nodes,
            order,
            tree,
        } = &mut s;
        inst.clear();
        boxes.clear();
        prev.clear();
        nodes.clear();
        let mut over = 0;
        let mut moved = false;
        for (id, handle, shown) in list {
            let (Some(shown), Some(vols)) = (shown, self.models.get(handle.0 as usize)) else {
                over += 1;
                continue;
            };
            let n = shown.parts.len();
            if boxes.len() == MAX_ENTITIES
                || inst.len() / INSTANCE_WORDS + n > MAX_ENTITY_PARTS
                || vols.len() != n
            {
                over += 1;
                continue;
            }
            let first = (inst.len() / INSTANCE_WORDS) as u32;
            let mut entity_moved = false;
            let mut bx = [[f32::MAX; 3], [f32::MIN; 3]];
            for (k, ((xf, detail), vol)) in shown.parts.iter().zip(vols).enumerate() {
                let before = self.prev.get(&(id, k)).copied().unwrap_or(xf.part_to_world);
                let part_moved = before != xf.part_to_world;
                entity_moved |= part_moved;
                for b in [
                    PartXform::world_box(&xf.part_to_world, vol.size),
                    PartXform::world_box(&before, vol.size),
                ] {
                    for r in 0..3 {
                        bx[0][r] = bx[0][r].min(b[0][r]);
                        bx[1][r] = bx[1][r].max(b[1][r]);
                    }
                }
                prev.insert((id, k), xf.part_to_world);
                let detail = f32::from(*detail);
                for row in xf.world_to_part.iter().chain(&before) {
                    inst.extend(row.map(f32::to_bits));
                }
                inst.extend([
                    vol.word,
                    boxes.len() as u32,
                    u32::from(part_moved) * INSTANCE_MOVED,
                    (1.0 / detail).to_bits(),
                    vol.size[0],
                    vol.size[1],
                    vol.size[2],
                    0,
                ]);
            }
            moved |= entity_moved;
            let count = n as u32 | if entity_moved { LEAF_MOVED } else { 0 };
            boxes.push((bx, first, count));
        }
        if !boxes.is_empty() {
            order.clear();
            order.extend(0..boxes.len());
            build(boxes, order, nodes);
        }
        tree.clear();
        tree.resize(INSTANCE_WORD, 0);
        for (i, n) in nodes.iter().enumerate() {
            tree[i * NODE_WORDS..(i + 1) * NODE_WORDS].copy_from_slice(n);
        }
        let parts = inst.len() / INSTANCE_WORDS;
        tree[TREE_HEADER_WORD..TREE_HEADER_WORD + 4].copy_from_slice(&[
            boxes.len() as u32,
            nodes.len() as u32,
            parts as u32,
            u32::from(moved),
        ]);
        queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(tree));
        if !inst.is_empty() {
            queue.write_buffer(
                &self.buffer,
                (INSTANCE_WORD * 4) as u64,
                bytemuck::cast_slice(inst),
            );
        }
        std::mem::swap(&mut self.prev, prev);
        self.drawn = boxes.len();
        self.parts = parts;
        self.over = over;
        self.moved = moved;
        self.scratch = s;
    }

    /// Entities the last [`EntityMirror::set_entities`] drew.
    pub fn entities_drawn(&self) -> usize {
        self.drawn
    }

    /// Parts the last [`EntityMirror::set_entities`] drew.
    pub fn parts_drawn(&self) -> usize {
        self.parts
    }

    /// Entities the last [`EntityMirror::set_entities`] was given and did not draw.
    pub fn entities_over_cap(&self) -> usize {
        self.over
    }

    /// Whether any part drawn by the last [`EntityMirror::set_entities`] moved since the call
    /// before.
    pub fn entities_moved(&self) -> bool {
        self.moved
    }
}

/// Appends the subtree over `order`'s boxes to `nodes`, by median split on the longest axis
/// of their centres' extent, one entity a leaf; returns its root's index.
fn build(
    boxes: &[([[f32; 3]; 2], u32, u32)],
    order: &mut [usize],
    nodes: &mut Vec<[u32; NODE_WORDS]>,
) -> u32 {
    let mut b = [[f32::MAX; 3], [f32::MIN; 3]];
    let mut c = [[f32::MAX; 3], [f32::MIN; 3]];
    for &i in order.iter() {
        for r in 0..3 {
            b[0][r] = b[0][r].min(boxes[i].0[0][r]);
            b[1][r] = b[1][r].max(boxes[i].0[1][r]);
            let m = 0.5 * (boxes[i].0[0][r] + boxes[i].0[1][r]);
            c[0][r] = c[0][r].min(m);
            c[1][r] = c[1][r].max(m);
        }
    }
    let at = nodes.len();
    let mut node = [0u32; NODE_WORDS];
    for r in 0..3 {
        node[r] = b[0][r].to_bits();
        node[3 + r] = b[1][r].to_bits();
    }
    nodes.push(node);
    if let [one] = order {
        nodes[at][6] = boxes[*one].1 | LEAF_BIT;
        nodes[at][7] = boxes[*one].2;
        return at as u32;
    }
    let axis = (0..3)
        .max_by(|&a, &k| (c[1][a] - c[0][a]).total_cmp(&(c[1][k] - c[0][k])))
        .unwrap_or(0);
    let centre = |i: usize| boxes[i].0[0][axis] + boxes[i].0[1][axis];
    order.sort_by(|&a, &k| centre(a).total_cmp(&centre(k)).then(a.cmp(&k)));
    let mid = order.len() / 2;
    let (lo, hi) = order.split_at_mut(mid);
    let l = build(boxes, lo, nodes);
    let r = build(boxes, hi, nodes);
    nodes[at][6] = l;
    nodes[at][7] = r;
    at as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_layout_constants_hold_the_plans_arithmetic() {
        assert_eq!(INSTANCE_WORD * 4, 4096);
        assert_eq!(MAX_ENTITY_PARTS * INSTANCE_WORDS * 4, 65_536);
        assert_eq!(
            POOL_WORD * 4 + DEFAULT_ENTITY_POOL_BYTES as usize,
            8_458_240
        );
    }

    /// A one-part model of a 2-voxel cube at detail 1, its pivot at its corner, and the pose
    /// that translates the part by `t` (1/256 world voxel) and turns it `q`.
    fn one_part(t: [i32; 3], q: vxe_entity::Quat) -> (Model, vxe_entity::Pose) {
        let part = vxe_entity::Part {
            volume: std::sync::Arc::new(Prefab::new([2; 3], vec![AIR; 8], None)),
            parent: None,
            pivot: [0; 3],
            detail: 1,
        };
        let model = Model::new(vec![part]).expect("one part");
        let mut pose = vxe_entity::Pose::rest(&model);
        pose.parts[0].t = t;
        pose.parts[0].q = q;
        (model, pose)
    }

    fn close(a: &[[f32; 4]; 3], b: &[[f32; 4]; 3], tol: f32) -> bool {
        a.iter()
            .flatten()
            .zip(b.iter().flatten())
            .all(|(x, y)| (x - y).abs() <= tol)
    }

    /// Plan 0063 Phase 3 done-when 1: a part translated 0 at one tick and 256 (one voxel) at the
    /// next sits a quarter voxel along at `alpha` 0.25, and `alpha` 0 and 1 give the two ticks'
    /// transforms; a quarter turn about z between the ticks is an eighth turn at 0.5.
    #[test]
    fn display_between_two_ticks_is_between_them() {
        let id = vxe_entity::Quat::IDENTITY;
        let (model, a) = one_part([0; 3], id);
        let (_, b) = one_part([256, 0, 0], id);
        let xa = DisplayPose::of(&model, &a)
            .expect("fits")
            .part_to_world(0)
            .expect("part");
        let xb = DisplayPose::of(&model, &b)
            .expect("fits")
            .part_to_world(0)
            .expect("part");
        let at = |alpha: f32| {
            DisplayPose::between(&model, &a, &b, alpha)
                .expect("fits")
                .part_to_world(0)
                .expect("part")
        };
        let q = at(0.25);
        assert!((q[0][3] - 0.25).abs() <= 1e-4, "{q:?}");
        assert!(q[1][3].abs() <= 1e-4 && q[2][3].abs() <= 1e-4, "{q:?}");
        assert!(close(&at(0.0), &xa, 1e-6), "{:?} against {xa:?}", at(0.0));
        assert!(close(&at(1.0), &xb, 1e-6), "{:?} against {xb:?}", at(1.0));

        // (cos 45, 0, 0, sin 45) in Q14: a quarter turn about z.
        let (_, c) = one_part([0; 3], vxe_entity::Quat([11585, 0, 0, 11585]));
        let h = DisplayPose::between(&model, &a, &c, 0.5)
            .expect("fits")
            .part_to_world(0)
            .expect("part");
        let (s, k) = (
            std::f32::consts::FRAC_PI_8.sin(),
            std::f32::consts::FRAC_PI_8.cos(),
        );
        // An eighth turn: x maps to (cos 45, sin 45), so m00 = cos^2 - sin^2 of 22.5 degrees.
        let c45 = k * k - s * s;
        assert!(
            (h[0][0] - c45).abs() <= 1e-4 && (h[1][0] - c45).abs() <= 1e-4,
            "{h:?}"
        );
    }

    /// A display pose refreshed in place equals the one built new, keeps its parts' storage
    /// (the same buffer, so no allocation) from the second refresh on, and empties itself on
    /// a pose that does not fit the model.
    #[test]
    fn a_display_refreshed_in_place_reuses_its_storage() {
        let id = vxe_entity::Quat::IDENTITY;
        let (model, a) = one_part([0; 3], id);
        let (_, b) = one_part([256, 0, 0], vxe_entity::Quat([11585, 0, 0, 11585]));
        let mut d = DisplayPose::default();
        assert!(d.between_into(&model, &a, &b, 0.25));
        assert_eq!(
            Some(&d),
            DisplayPose::between(&model, &a, &b, 0.25).as_ref()
        );
        let held = (d.parts.as_ptr(), d.parts.capacity());
        for k in 0..4 {
            let alpha = k as f32 / 4.0;
            assert!(d.between_into(&model, &a, &b, alpha));
            assert_eq!(
                Some(&d),
                DisplayPose::between(&model, &a, &b, alpha).as_ref()
            );
            assert!(d.of_into(&model, &b));
            assert_eq!(Some(&d), DisplayPose::of(&model, &b).as_ref());
            assert_eq!((d.parts.as_ptr(), d.parts.capacity()), held, "refresh {k}");
        }
        let unfit = vxe_entity::Pose {
            parts: Vec::new(),
            ..a.clone()
        };
        assert!(!d.of_into(&model, &unfit));
        assert!(d.is_empty());
        assert!(!d.between_into(&model, &a, &unfit, 0.5));
        assert!(d.is_empty());
        assert!(d.of_into(&model, &a), "the storage survives a refusal");
    }

    #[test]
    fn a_sixteen_voxel_cube_is_5672_bytes() {
        let cube = Prefab::new([16; 3], vec![AIR; 4096], None);
        assert_eq!(EntityMirror::volume_bytes(&cube), 5672);
    }
}
