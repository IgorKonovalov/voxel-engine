//! The entity mirror (ADR-0050): voxel-model parts in a pool of their own, one instance a posed
//! part, and a tree over the entities' boxes, all in one storage buffer that `entity.wgsl`
//! walks after the world march.
//!
//! The buffer's words, from 0:
//!
//! - the tree: [`ENTITY_NODES`] nodes of [`NODE_WORDS`] words, the root at node 0. A node is
//!   its box's minimum and maximum in world voxels (six `f32`), then for an inner node its two
//!   children's indices, for a leaf its first instance with [`LEAF_BIT`] set and its instance
//!   count, with [`LEAF_MOVED`] set when any of the entity's parts moved (a ghost leaf, over
//!   the box of an entity that left at this write, has no instance and the bit set). The last
//!   node's slot is the header ([`TREE_HEADER_WORD`]): the leaves (the entities drawn and the
//!   ghosts), the nodes, the instances drawn, and whether any instance moved.
//! - the instances from [`INSTANCE_WORD`]: [`MAX_ENTITY_PARTS`] of [`INSTANCE_WORDS`] words. An
//!   instance is its world-to-part transform (3 x 4 `f32`, row-major, model voxels from world
//!   voxels), the part-to-world transform of the frame before (3 x 4 `f32`), the word its
//!   volume starts at, its slot and its slot's generation (`slot | generation << 9`, the
//!   instances module's docs), its flags ([`INSTANCE_MOVED`]),
//!   `1 / detail` as `f32`, and the volume's size in model voxels.
//! - the pool from [`POOL_WORD`]: each volume once, keyed by its prefab's id. A volume is a
//!   header of [`VOLUME_HEADER_WORDS`] words (its size, then the word offsets of its
//!   occupancy bits, block bits, palette indices and palette from the volume's start), one
//!   occupancy bit a voxel (x fastest), one bit per 4 x 4 x 4 block that holds a voxel, one
//!   palette byte a voxel, and a palette of 256 material ids. A 16-voxel cube is
//!   `32 + 512 + 8 + 4096 + 1024` = 5672 bytes.
//!
//! The pool frees (ADR-0060): the consumer that uploaded a model frees it by its handle
//! ([`EntityMirror::free_model`], or [`EntityMirror::clear`] for every model), and a volume no
//! model claims returns its words to a coalescing free list ([`VolumePool`]). A volume never
//! moves.
//!
//! **Queue order is the rule that makes a freed range reusable at once.** Every write to this
//! buffer is a `queue.write_buffer`, which the queue orders after every command buffer
//! submitted before it and before the next submission. A range freed and refilled between two
//! frames is read with its old words by the frame already submitted and with its new words by
//! the next; nothing in flight sees a mix, so no free waits for a frame. A path that maps this
//! buffer for writing, or writes it from a frame's own commands, breaks the rule.

mod alloc;
mod display;
mod instances;
mod pool;

pub use alloc::RangeAlloc;
pub use display::DisplayPose;
pub use instances::EntityInstances;
pub use pool::{FreedBytes, ModelHandle, PoolUpload, VolumePool};

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
pub(crate) const PALETTE: usize = 256;

/// Why [`EntityMirror::upload_model`] refused a model, or [`EntityMirror::free_model`] a
/// handle. Nothing is written on a refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityUploadError {
    /// The model's new volumes need `needed` bytes in one range; the pool holds `free` bytes
    /// over all its holes, `largest` in its largest. `largest < needed <= free` is a
    /// fragmented pool, which a [`EntityMirror::clear`] and a re-upload answers.
    PoolFull {
        needed: u64,
        free: u64,
        largest: u64,
    },
    /// Part `part` holds more than 255 materials.
    Palette { part: usize },
    /// Part `part` holds a material the table has no row for.
    UnknownMaterial { part: usize, material: u16 },
    /// The handle names no model of this mirror: freed, or another mirror's.
    UnknownModel,
    /// The model table holds 65 536 live models.
    TooManyModels,
}

impl std::fmt::Display for EntityUploadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::PoolFull {
                needed,
                free,
                largest,
            } => {
                write!(
                    f,
                    "the entity pool is full: {needed} bytes needed, {free} free, \
                     {largest} in the largest hole"
                )
            }
            Self::Palette { part } => write!(f, "part {part} holds more than 255 materials"),
            Self::UnknownMaterial { part, material } => {
                write!(f, "part {part} holds material {material}, which has no row")
            }
            Self::UnknownModel => write!(f, "the handle names no model of this mirror"),
            Self::TooManyModels => write!(f, "the mirror holds 65536 models"),
        }
    }
}

impl std::error::Error for EntityUploadError {}

/// The entity mirror: one storage buffer holding the tree, the instances and the pool
/// (module docs), the pool's bookkeeping, and the instances of the last write.
pub struct EntityMirror {
    buffer: wgpu::Buffer,
    pool: VolumePool,
    instances: EntityInstances,
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
            pool: VolumePool::new(pool_bytes),
            instances: EntityInstances::new(),
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

    /// The pool's bookkeeping: its free list, and each model's words.
    pub fn pool(&self) -> &VolumePool {
        &self.pool
    }

    /// Bytes of the pool in use: the live models' distinct volumes.
    pub fn pool_used_bytes(&self) -> u64 {
        self.pool.used_bytes()
    }

    /// The pool's bytes.
    pub fn pool_total_bytes(&self) -> u64 {
        self.pool.total_bytes()
    }

    /// Bytes of the pool free, over every hole.
    pub fn pool_free_bytes(&self) -> u64 {
        self.pool.free_bytes()
    }

    /// Bytes of the pool's largest hole: the most one upload's new volumes can take.
    pub fn pool_largest_free_bytes(&self) -> u64 {
        self.pool.largest_free_bytes()
    }

    /// The bytes [`EntityMirror::upload_model`] would write for `prefab`: its volume's words.
    pub fn volume_bytes(prefab: &Prefab) -> u64 {
        let table = MaterialTable::new();
        // Size alone sets the layout; the materials only fill it.
        let empty = Prefab::new(prefab.size(), vec![AIR; prefab.len()], None);
        (pool::volume_words(&empty, &table, 0).map_or(0, |w| w.len()) * 4) as u64
    }

    /// Writes each part's volume of `model` not yet in the pool, keyed by its prefab's id (two
    /// models sharing a prefab share its words), into one range, first fit from the pool's
    /// lowest word. Every material must have a row in `table`. Free the model with
    /// [`EntityMirror::free_model`] once nothing draws it.
    pub fn upload_model(
        &mut self,
        queue: &wgpu::Queue,
        model: &Model,
        table: &MaterialTable,
    ) -> Result<ModelHandle, EntityUploadError> {
        let up = self.pool.upload(model, table)?;
        for (word, words) in &up.writes {
            queue.write_buffer(
                &self.buffer,
                u64::from(*word) * 4,
                bytemuck::cast_slice(words),
            );
        }
        Ok(up.handle)
    }

    /// Frees the model `handle` names: its volumes no other model claims return to the pool,
    /// reusable by the next upload at once (module docs, the queue-order rule). An entity
    /// listed under the handle afterwards is counted and not drawn.
    pub fn free_model(&mut self, handle: ModelHandle) -> Result<FreedBytes, EntityUploadError> {
        self.pool.free_model(handle)
    }

    /// Frees every model: the pool is empty and every handle names nothing.
    pub fn clear(&mut self) {
        self.pool.clear();
    }

    /// Writes one instance a part of the entities in `list` (in its order) as `set` poses
    /// them, and the tree over their boxes ([`EntityInstances::write`] names what is counted
    /// and not drawn).
    pub fn set_entities(
        &mut self,
        queue: &wgpu::Queue,
        set: &EntitySet,
        list: &[(EntityId, ModelHandle)],
    ) {
        self.instances.set_entities(&self.pool, set, list);
        self.upload_instances(queue);
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
        self.instances.write(
            &self.pool,
            list.into_iter().map(|(id, h, d)| (id, h, Some(d))),
        );
        self.upload_instances(queue);
    }

    fn upload_instances(&self, queue: &wgpu::Queue) {
        queue.write_buffer(
            &self.buffer,
            0,
            bytemuck::cast_slice(self.instances.tree_words()),
        );
        let inst = self.instances.instance_words();
        if !inst.is_empty() {
            queue.write_buffer(
                &self.buffer,
                (INSTANCE_WORD * 4) as u64,
                bytemuck::cast_slice(inst),
            );
        }
    }

    /// The instance slots and the tree of the last [`EntityMirror::set_entities`]: each
    /// entity's run of slots and their generations.
    pub fn instances(&self) -> &EntityInstances {
        &self.instances
    }

    /// Whether the last [`EntityMirror::set_entities`] left a tree to walk: an entity drawn, or
    /// the ghost of one that left (a shadow ray crossing it marks its pixel fresh), so the
    /// light pass walks entities.
    pub fn walks_entities(&self) -> bool {
        self.instances.walks()
    }

    /// Entities the last [`EntityMirror::set_entities`] drew.
    pub fn entities_drawn(&self) -> usize {
        self.instances.entities_drawn()
    }

    /// Parts the last [`EntityMirror::set_entities`] drew.
    pub fn parts_drawn(&self) -> usize {
        self.instances.parts_drawn()
    }

    /// Entities the last [`EntityMirror::set_entities`] was given and did not draw.
    pub fn entities_over_cap(&self) -> usize {
        self.instances.entities_over_cap()
    }

    /// Whether any part drawn by the last [`EntityMirror::set_entities`] moved since the call
    /// before.
    pub fn entities_moved(&self) -> bool {
        self.instances.entities_moved()
    }
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

    /// `entity.wgsl` masks the slot and the hit record's instance field to the bits the
    /// instances module packs.
    #[test]
    fn the_shader_reads_the_slot_bits_the_mirror_packs() {
        let wgsl = include_str!("../shaders/entity.wgsl");
        let slot_mask = (1u32 << instances::INSTANCE_SLOT_BITS) - 1;
        assert!(wgsl.contains(&format!("const ENTITY_SLOT_MASK: u32 = {slot_mask}u;")));
        assert!(wgsl.contains(&format!(
            "const ENTITY_RECORD_BITS: u32 = {}u;",
            instances::INSTANCE_RECORD_BITS
        )));
        // The record's instance field is the hit record flags' bits 6..15.
        assert_eq!(6 + instances::INSTANCE_RECORD_BITS, 16);
        assert_eq!(1 << instances::INSTANCE_SLOT_BITS, MAX_ENTITY_PARTS);
    }

    #[test]
    fn a_sixteen_voxel_cube_is_5672_bytes() {
        let cube = Prefab::new([16; 3], vec![AIR; 4096], None);
        assert_eq!(EntityMirror::volume_bytes(&cube), 5672);
    }
}
