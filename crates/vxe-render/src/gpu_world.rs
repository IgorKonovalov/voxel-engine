//! The GPU mirror of a world's bricks (ADR-0002): brick pool, chunk records, chunk table and
//! material rows, uploaded per dirty brick.
//!
//! The CPU keeps a copy of every chunk record entry, so `sync` knows which pool slot a brick
//! held before the edit. A brick that becomes `Full` takes a slot from the free list, or the
//! next never-used slot; a brick that collapses to `Uniform` returns its slot to the free list.
//! The pool grows by whole pages, copying the old contents on the GPU.

use std::collections::HashMap;
use std::sync::PoisonError;

use wgpu::util::DeviceExt;

use vxe_world::{BRICK_VOXELS, Brick, CHUNK_BRICKS, ChunkCoord, ChunkExtent, MaterialTable, World};

/// Pool slots per growth step: 4 MiB of bricks.
pub const POOL_PAGE_SLOTS: u32 = 4096;
/// Bytes of one brick's voxels in the pool.
pub const BRICK_BYTES: u64 = (BRICK_VOXELS * 2) as u64;
/// Chunk table entry of a chunk with no record.
pub const ABSENT: u32 = u32::MAX;
/// Chunk record entry bit: set, the low bits are a pool slot; clear, they are a material.
pub const FULL_BIT: u32 = 1 << 31;
/// `f32`s per material row: albedo rgb, roughness, emission rgb, flags as bits.
pub const MATERIAL_FLOATS: usize = 8;

/// Why a sync could not complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MirrorError {
    /// The pool would need `needed` bytes; the device allows `limit` in one binding.
    PoolTooLarge { needed: u64, limit: u64 },
}

impl std::fmt::Display for MirrorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PoolTooLarge { needed, limit } => write!(
                f,
                "brick pool needs {needed} bytes; the device binds at most {limit}"
            ),
        }
    }
}

impl std::error::Error for MirrorError {}

/// What one `sync` uploaded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyncStats {
    pub bytes: u64,
    pub bricks: u64,
    pub chunks: u64,
}

pub struct GpuWorld {
    device: wgpu::Device,
    extent: ChunkExtent,
    pool: wgpu::Buffer,
    pool_slots: u32,
    records: wgpu::Buffer,
    table: wgpu::Buffer,
    materials: wgpu::Buffer,
    materials_written: bool,
    /// Record index of every chunk in the extent, in ascending coordinate order.
    record_of: HashMap<ChunkCoord, u32>,
    /// Whether the chunk table already points at each record.
    present: Vec<bool>,
    /// CPU copy of every chunk record entry.
    entries: Vec<u32>,
    free: Vec<u32>,
    next_slot: u32,
    bytes_written: u64,
}

impl GpuWorld {
    /// A mirror sized for `extent`. The pool starts empty and is sized on the first `sync`.
    pub fn new(device: &wgpu::Device, extent: ChunkExtent) -> Self {
        let coords = extent.coords();
        let record_of: HashMap<_, _> = coords
            .iter()
            .enumerate()
            .map(|(i, &c)| (c, i as u32))
            .collect();
        let n = coords.len().max(1);
        let storage = wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC;
        let records = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chunk_records"),
            size: (n * CHUNK_BRICKS * 4) as u64,
            usage: storage,
            mapped_at_creation: false,
        });
        // Every entry starts absent; `sync` points an entry at its record on first upload.
        let table = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("chunk_table"),
            contents: bytemuck::cast_slice(&vec![ABSENT; n]),
            usage: storage,
        });
        let pool = Self::create_pool(device, 0);
        Self {
            device: device.clone(),
            extent,
            pool,
            pool_slots: 0,
            records,
            table,
            materials: Self::create_materials(device, 1),
            materials_written: false,
            record_of,
            present: vec![false; coords.len()],
            entries: vec![0; coords.len() * CHUNK_BRICKS],
            free: Vec::new(),
            next_slot: 0,
            bytes_written: 0,
        }
    }

    fn create_pool(device: &wgpu::Device, slots: u32) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("brick_pool"),
            // A zero-sized binding is invalid; an empty pool still holds one slot's bytes.
            size: u64::from(slots.max(1)) * BRICK_BYTES,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    }

    fn create_materials(device: &wgpu::Device, rows: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("materials"),
            size: (rows.max(1) * MATERIAL_FLOATS * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn extent(&self) -> ChunkExtent {
        self.extent
    }

    pub fn pool(&self) -> &wgpu::Buffer {
        &self.pool
    }

    pub fn records(&self) -> &wgpu::Buffer {
        &self.records
    }

    pub fn table(&self) -> &wgpu::Buffer {
        &self.table
    }

    pub fn materials(&self) -> &wgpu::Buffer {
        &self.materials
    }

    /// Pool capacity in slots.
    pub fn pool_slots(&self) -> u32 {
        self.pool_slots
    }

    /// Slots currently holding a `Full` brick.
    pub fn live_slots(&self) -> u32 {
        self.next_slot - self.free.len() as u32
    }

    /// Slots returned by collapsed bricks and not yet reused, in the order they will be reused
    /// last-first.
    pub fn free_slots(&self) -> &[u32] {
        &self.free
    }

    /// Bytes written to the GPU over the mirror's life.
    pub fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    /// The record entry the mirror holds for brick `slot` of chunk `coord`.
    pub fn entry(&self, coord: ChunkCoord, slot: usize) -> Option<u32> {
        let r = *self.record_of.get(&coord)?;
        Some(self.entries[r as usize * CHUNK_BRICKS + slot])
    }

    /// Record index of `coord`, if it lies in the extent.
    pub fn record_index(&self, coord: ChunkCoord) -> Option<u32> {
        self.record_of.get(&coord).copied()
    }

    /// Bytes the mirror occupies on the GPU by the uploader's accounting: pool slots in use,
    /// chunk records, chunk table and material rows.
    pub fn mirror_bytes(&self) -> u64 {
        u64::from(self.live_slots()) * BRICK_BYTES
            + self.entries.len() as u64 * 4
            + self.present.len() as u64 * 4
            + self.materials.size()
    }

    /// Uploads every dirty brick of the chunks of `world` inside the extent and clears their
    /// dirty bits. Takes one chunk lock at a time, in ascending coordinate order.
    pub fn sync(&mut self, world: &World, queue: &wgpu::Queue) -> Result<SyncStats, MirrorError> {
        let mut stats = SyncStats::default();
        if !self.materials_written {
            stats.bytes += self.write_materials(world.materials(), queue);
            self.materials_written = true;
        }
        if self.pool_slots == 0 {
            let full = world.stats().full_bricks as u64;
            self.grow_to(full, queue)?;
        }
        for coord in world.store().coords() {
            let Some(&rec) = self.record_of.get(&coord) else {
                continue;
            };
            let Some(chunk) = world.store().get(coord) else {
                continue;
            };
            let mut chunk = chunk.write().unwrap_or_else(PoisonError::into_inner);
            if !chunk.any_dirty() {
                continue;
            }
            let rec = rec as usize;
            if !self.present[rec] {
                let wrapped = self.wrapped_index(coord);
                queue.write_buffer(&self.table, wrapped * 4, bytemuck::bytes_of(&(rec as u32)));
                stats.bytes += 4;
                self.present[rec] = true;
            }
            let dirty: Vec<usize> = chunk.dirty_slots().collect();
            for slot in dirty {
                let i = rec * CHUNK_BRICKS + slot;
                let old = self.entries[i];
                self.entries[i] = match chunk.brick(slot) {
                    Brick::Full(v) => {
                        let s = if old & FULL_BIT != 0 {
                            old & !FULL_BIT
                        } else {
                            self.alloc(queue)?
                        };
                        queue.write_buffer(
                            &self.pool,
                            u64::from(s) * BRICK_BYTES,
                            bytemuck::cast_slice(&v[..]),
                        );
                        stats.bytes += BRICK_BYTES;
                        stats.bricks += 1;
                        FULL_BIT | s
                    }
                    Brick::Uniform(m) => {
                        if old & FULL_BIT != 0 {
                            self.free.push(old & !FULL_BIT);
                        }
                        u32::from(*m)
                    }
                };
            }
            let range = rec * CHUNK_BRICKS..(rec + 1) * CHUNK_BRICKS;
            queue.write_buffer(
                &self.records,
                (range.start * 4) as u64,
                bytemuck::cast_slice(&self.entries[range]),
            );
            stats.bytes += (CHUNK_BRICKS * 4) as u64;
            stats.chunks += 1;
            chunk.clear_dirty();
        }
        self.bytes_written += stats.bytes;
        Ok(stats)
    }

    /// Index into the chunk table: the coordinate wrapped into the window, which in a bounded
    /// world is the extent itself.
    fn wrapped_index(&self, c: ChunkCoord) -> u64 {
        let w = self.extent.size();
        let wr = [0, 1, 2].map(|k| c.0[k].rem_euclid(w[k].max(1)) as u64);
        let w = w.map(|v| v.max(1) as u64);
        wr[0] + w[0] * (wr[1] + w[1] * wr[2])
    }

    fn write_materials(&mut self, table: &MaterialTable, queue: &wgpu::Queue) -> u64 {
        let rows: Vec<[f32; MATERIAL_FLOATS]> = table
            .rows()
            .iter()
            .map(|m| {
                [
                    m.albedo[0],
                    m.albedo[1],
                    m.albedo[2],
                    m.roughness,
                    m.emission[0],
                    m.emission[1],
                    m.emission[2],
                    f32::from_bits(m.flags.0),
                ]
            })
            .collect();
        if self.materials.size() < (rows.len() * MATERIAL_FLOATS * 4) as u64 {
            self.materials = Self::create_materials(&self.device, rows.len());
        }
        let bytes: &[u8] = bytemuck::cast_slice(&rows);
        queue.write_buffer(&self.materials, 0, bytes);
        bytes.len() as u64
    }

    fn alloc(&mut self, queue: &wgpu::Queue) -> Result<u32, MirrorError> {
        if let Some(s) = self.free.pop() {
            return Ok(s);
        }
        if self.next_slot >= self.pool_slots {
            self.grow_to(u64::from(self.next_slot) + 1, queue)?;
        }
        self.next_slot += 1;
        Ok(self.next_slot - 1)
    }

    /// Grows the pool to hold at least `slots`, in whole pages, keeping its contents.
    fn grow_to(&mut self, slots: u64, queue: &wgpu::Queue) -> Result<(), MirrorError> {
        if slots <= u64::from(self.pool_slots) {
            return Ok(());
        }
        let page = u64::from(POOL_PAGE_SLOTS);
        let new_slots = slots.div_ceil(page).max(1) * page;
        let limits = self.device.limits();
        let limit = limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size);
        let needed = new_slots * BRICK_BYTES;
        if needed > limit {
            return Err(MirrorError::PoolTooLarge { needed, limit });
        }
        let pool = Self::create_pool(&self.device, new_slots as u32);
        if self.pool_slots > 0 {
            let mut enc = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("grow pool"),
                });
            enc.copy_buffer_to_buffer(
                &self.pool,
                0,
                &pool,
                0,
                u64::from(self.pool_slots) * BRICK_BYTES,
            );
            queue.submit([enc.finish()]);
        }
        self.pool = pool;
        self.pool_slots = new_slots as u32;
        Ok(())
    }
}
