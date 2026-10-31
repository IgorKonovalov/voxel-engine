//! The field surface's scratch pool (ADR-0040 Alternative B): every band brick of the mirror
//! window's full chunks gets a dense slot beside the mirror, outside its accounting, filled by
//! the field pass (`attrib.wgsl`'s `field_main`) and read by the hit pass's field walk
//! (`march.wgsl`, `FIELD_SURFACE`).
//!
//! A voxel's count is its solid voxels among the 27 around it, itself included, solid being
//! not air as in the attribute pass (a translucent voxel is solid). A voxel is in the band when
//! its count is neither 0 nor 27, and a brick is a band brick when it holds a band voxel.
//!
//! The pool is a scratch: dense slots of [`FIELD_SLOT_UNITS`] units (unit 0 the band mask, bit
//! `i % 32` of word `i / 32` for voxel `i = x + 8 y + 64 z`; units 1 to 8 the counts, voxel `i`
//! in byte `i % 4` of word `16 + i / 4`) in [`FIELD_SHARDS`] buffers, slot `s` at slot
//! `s % n` of shard `s / n` for `n` slots a shard, one free list of slots, and an index of
//! [`FIELD_RECORD_WORDS`] words per chunk record at the start of shard 0 (its first slots are
//! the index's): word 0 the record's slot count, words 1..17 its slotted bricks (bit `b % 32`
//! of word `1 + b / 32`, the record's brick-bits layout) and words 17..529 each brick's slot or
//! `ABSENT`. The CPU gives a slot to every band brick of a chunk mirrored full ([`is_band`],
//! from the world's voxels; a chunk the mirror does not hold is air, a LOD-only one may border
//! the band), when it is written or a neighbour is. Slots are allocated from a free list of
//! one size and never compacted. The pool's bytes are outside the mirror's accounting.

use std::collections::{HashMap, HashSet};

use vxe_world::{
    BRICK_EDGE, CHUNK_BRICKS, CHUNK_EDGE_BRICKS, ChunkCoord, brick_of_slot, brick_slot,
};

use crate::gpu_world::{ABSENT, UNIT_BYTES};

/// Units of a field slot: the band mask and 512 `u8` counts.
pub const FIELD_SLOT_UNITS: u32 = 9;
/// Bytes of a field slot.
pub const FIELD_SLOT_BYTES: u64 = FIELD_SLOT_UNITS as u64 * UNIT_BYTES;
/// Field pool buffers the hit pass binds.
pub const FIELD_SHARDS: usize = 2;
/// Words of a record's row of the field index.
pub const FIELD_RECORD_WORDS: usize = 1 + 16 + CHUNK_BRICKS;
/// The row's first brick-bits word.
pub const FIELD_BITS_WORD: usize = 1;
/// The row's first slot word.
pub const FIELD_SLOT_WORD: usize = 17;
/// Largest field shard, in bytes, whatever the device allows.
const MAX_SHARD_BYTES: u64 = 1 << 30;
/// Slots of a shard's first buffer; it doubles as it fills, up to the shard's slots.
const FIRST_SHARD_SLOTS: u32 = 4096;

/// How the field reads a chunk around a brick: mirrored full (its voxels, from the world), a
/// LOD-only record (read through its mips on the GPU, so any of its bricks may border a band
/// brick), or not mirrored (air to the GPU).
pub enum Around<'a> {
    Air,
    Lod,
    Full(&'a vxe_world::Chunk),
}

/// The bits of voxels of a brick's mask selected by a neighbour offset `d` (each component
/// -1, 0 or 1) toward the brick at the centre: per layer word, the voxels at the face (`x = 7`
/// for `d.x = -1`, `x = 0` for `+1`, any for 0, and the same for y), on the layers `z = 7`,
/// `z = 0` or every layer.
fn shell_select(d: [i32; 3]) -> (u64, std::ops::Range<usize>) {
    const X0: u64 = 0x0101_0101_0101_0101;
    let xm = match d[0] {
        -1 => X0 << 7,
        1 => X0,
        _ => !0,
    };
    let ym = match d[1] {
        -1 => 0xFF << 56,
        1 => 0xFF,
        _ => !0,
    };
    let zs = match d[2] {
        -1 => 7..8,
        1 => 0..1,
        _ => 0..8,
    };
    (xm & ym, zs)
}

/// Whether brick `slot` of the chunk at the centre of `around` (index `(dx + 1) + 3 (dy + 1) +
/// 9 (dz + 1)`) holds a voxel whose 27-count is neither 0 nor 27, solid being not air: a brick
/// with both solid and air voxels always does (a solid voxel faces an air one inside it); an
/// all-solid or all-air brick does when a voxel of the other kind lies in the one-voxel shell
/// around it. A LOD-only chunk in the shell counts as holding one.
pub fn is_band(around: &[Around<'_>; 27], slot: usize) -> bool {
    let Around::Full(centre) = around[13] else {
        return false;
    };
    let own = centre.brick(slot).solid_bits();
    let solid = own.iter().all(|&w| w == !0);
    let air = own.iter().all(|&w| w == 0);
    if !solid && !air {
        return true;
    }
    let l = brick_of_slot(slot);
    for k in 0..27 {
        if k == 13 {
            continue;
        }
        let d = [k % 3 - 1, (k / 3) % 3 - 1, k / 9 - 1];
        let b = [0, 1, 2].map(|a| l[a] + d[a]);
        let cd = b.map(|v| v.div_euclid(CHUNK_EDGE_BRICKS));
        let ci = ((cd[0] + 1) + 3 * (cd[1] + 1) + 9 * (cd[2] + 1)) as usize;
        let bits = match &around[ci] {
            Around::Air => [0u64; 8],
            Around::Lod => return true,
            Around::Full(c) => c
                .brick(brick_slot(b.map(|v| v.rem_euclid(CHUNK_EDGE_BRICKS))))
                .solid_bits(),
        };
        let (m, zs) = shell_select(d);
        if bits[zs]
            .iter()
            .any(|&w| if solid { !w & m != 0 } else { w & m != 0 })
        {
            return true;
        }
    }
    false
}

/// One work-list entry of the field pass: the brick's field slot and its origin in voxels.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FieldItem {
    pub slot: u32,
    pub origin: [i32; 3],
}

/// The field pass's uniform: `attrib.wgsl`'s `Params`, its first spare word the slots per
/// field shard.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    window_min: [i32; 3],
    units_per_shard: u32,
    window_size: [i32; 3],
    items: u32,
    groups_x: u32,
    fit: u32,
    slots_per_shard: u32,
    _pad: u32,
}

/// The scratch pool: its buffers, its CPU index and free list, and the pass that fills it.
pub struct FieldScratch {
    device: wgpu::Device,
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    /// The shards' buffers and the slots each holds; a shard not yet needed is a one-slot
    /// placeholder.
    shards: Vec<(wgpu::Buffer, u32)>,
    slots_per_shard: u32,
    /// Slots of shard 0 the index takes.
    index_slots: u32,
    index_cpu: Vec<u32>,
    rows_changed: HashSet<u32>,
    /// Each chunk's slots by brick, and its record.
    chunks: HashMap<ChunkCoord, (u32, Box<[u32; CHUNK_BRICKS]>)>,
    free: Vec<u32>,
    next: u32,
    items: Vec<FieldItem>,
    work: Option<(wgpu::Buffer, usize)>,
    /// Every mirrored full chunk is taken whole at the next `prepare`.
    all: bool,
    /// Bricks the pool had no slot for.
    overflow: u64,
}

impl FieldScratch {
    /// An empty pool for a mirror of `records` reserved records on `device`.
    pub fn new(device: &wgpu::Device, records: u32) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("attrib.wgsl field"),
            source: wgpu::ShaderSource::Wgsl(crate::ATTRIB_WGSL.into()),
        });
        let buffer = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let rw = wgpu::BufferBindingType::Storage { read_only: false };
        let ro = wgpu::BufferBindingType::Storage { read_only: true };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("field"),
            entries: &[
                buffer(0, wgpu::BufferBindingType::Uniform),
                buffer(1, rw),
                buffer(2, rw),
                buffer(3, rw),
                buffer(4, rw),
                buffer(5, ro),
                buffer(6, ro),
                buffer(8, rw),
                buffer(9, rw),
                buffer(11, ro),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("field"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("field"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("field_main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("field params"),
            size: size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let l = device.limits();
        let cap = l
            .max_storage_buffer_binding_size
            .min(l.max_buffer_size)
            .min(MAX_SHARD_BYTES);
        let slots_per_shard = (cap / FIELD_SLOT_BYTES) as u32;
        let words = records as usize * FIELD_RECORD_WORDS;
        let mut row = vec![0u32; FIELD_RECORD_WORDS];
        row[FIELD_SLOT_WORD..].fill(ABSENT);
        let index_cpu: Vec<u32> = row.iter().copied().cycle().take(words.max(1)).collect();
        let index_slots = (index_cpu.len() as u64 * 4).div_ceil(FIELD_SLOT_BYTES) as u32;
        Self {
            device: device.clone(),
            pipeline,
            layout,
            uniform,
            shards: (0..FIELD_SHARDS)
                .map(|_| (Self::shard_buffer(device, 1), 1))
                .collect(),
            slots_per_shard,
            index_slots,
            index_cpu,
            // Every row at the first encode: the shard holds nothing yet.
            rows_changed: (0..records).collect(),
            chunks: HashMap::new(),
            free: Vec::new(),
            next: index_slots,
            items: Vec::new(),
            work: None,
            all: true,
            overflow: 0,
        }
    }

    fn shard_buffer(device: &wgpu::Device, slots: u32) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("field_pool"),
            size: u64::from(slots) * FIELD_SLOT_BYTES,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    }

    pub fn slots_per_shard(&self) -> u32 {
        self.slots_per_shard
    }

    pub fn shard(&self, k: usize) -> &wgpu::Buffer {
        &self.shards[k].0
    }

    /// Slots handed out and not freed.
    pub fn live_slots(&self) -> u64 {
        u64::from(self.next - self.index_slots) - self.free.len() as u64
    }

    /// Bricks that needed a slot when the pool was full, since it was built.
    pub fn overflow(&self) -> u64 {
        self.overflow
    }

    /// The pool's buffers' bytes, its index's included, as built.
    pub fn bytes(&self) -> u64 {
        self.shards.iter().map(|s| s.0.size()).sum::<u64>()
    }

    /// Frees `coord`'s slots and clears its row.
    pub fn drop_chunk(&mut self, coord: ChunkCoord) {
        let Some((rec, slots)) = self.chunks.remove(&coord) else {
            return;
        };
        for &s in slots.iter().filter(|&&s| s != ABSENT) {
            self.free.push(s);
        }
        self.write_row(rec, &[ABSENT; CHUNK_BRICKS]);
    }

    fn write_row(&mut self, rec: u32, slots: &[u32; CHUNK_BRICKS]) {
        let base = rec as usize * FIELD_RECORD_WORDS;
        let row = &mut self.index_cpu[base..base + FIELD_RECORD_WORDS];
        row[..FIELD_SLOT_WORD].fill(0);
        for (b, &s) in slots.iter().enumerate() {
            row[FIELD_SLOT_WORD + b] = s;
            if s != ABSENT {
                row[0] += 1;
                row[FIELD_BITS_WORD + b / 32] |= 1 << (b % 32);
            }
        }
        self.rows_changed.insert(rec);
    }

    fn alloc(&mut self) -> Option<u32> {
        if let Some(s) = self.free.pop() {
            return Some(s);
        }
        if self.next >= self.slots_per_shard * FIELD_SHARDS as u32 {
            return None;
        }
        self.next += 1;
        Some(self.next - 1)
    }

    /// Updates the slots and the work list for what changed: chunks whose record was written
    /// whole (every brick and the bricks of other chunks within one brick of it), re-uploaded
    /// bricks (their 3 x 3 x 3 neighbourhoods) and chunks whose record was written new,
    /// dropped or replaced (the bricks of other chunks within one brick of it). On its first
    /// call it takes every chunk `full_record` names in `full` whole. `full_record(c)` is the
    /// record of a chunk mirrored full, `lod(c)` whether one is mirrored LOD-only; the voxels
    /// are `world`'s. A brick keeps a slot while [`is_band`]; each slotted brick it visits is
    /// listed for the pass, once per call.
    #[allow(
        clippy::too_many_arguments,
        reason = "the mirror's two lookups, its full chunks and the three change sets"
    )]
    pub fn prepare(
        &mut self,
        world: &vxe_world::World,
        full_record: impl Fn(ChunkCoord) -> Option<u32>,
        lod: impl Fn(ChunkCoord) -> bool,
        full: &[ChunkCoord],
        whole: &HashSet<ChunkCoord>,
        bricks: &HashSet<[i32; 3]>,
        shells: &HashSet<ChunkCoord>,
    ) {
        let mut whole: Vec<ChunkCoord> = whole.iter().copied().collect();
        if std::mem::take(&mut self.all) {
            whole.extend_from_slice(full);
        }
        let e = CHUNK_EDGE_BRICKS;
        // The touched bricks, by chunk.
        let mut touched: HashMap<ChunkCoord, HashSet<usize>> = HashMap::new();
        let mut touch = |b: [i32; 3]| {
            let c = ChunkCoord(b.map(|v| v.div_euclid(e)));
            touched
                .entry(c)
                .or_default()
                .insert(brick_slot(b.map(|v| v.rem_euclid(e))));
        };
        let border = |c: ChunkCoord, inner: bool, touch: &mut dyn FnMut([i32; 3])| {
            let lo = c.0.map(|v| v * e - 1);
            for z in lo[2]..lo[2] + e + 2 {
                for y in lo[1]..lo[1] + e + 2 {
                    for x in lo[0]..lo[0] + e + 2 {
                        let b = [x, y, z];
                        let inside = (0..3).all(|k| b[k] > lo[k] && b[k] < lo[k] + e + 1);
                        if inner || !inside {
                            touch(b);
                        }
                    }
                }
            }
        };
        // A chunk that left full residency keeps no slot.
        for &c in whole.iter().chain(shells) {
            if full_record(c).is_none() {
                self.drop_chunk(c);
            }
        }
        for &c in &whole {
            border(c, true, &mut touch);
        }
        for &c in shells {
            border(c, false, &mut touch);
        }
        for b in bricks {
            for k in 0..27 {
                let d = [k % 3 - 1, (k / 3) % 3 - 1, k / 9 - 1];
                touch([0, 1, 2].map(|a| b[a] + d[a]));
            }
        }
        let mut rows: HashSet<ChunkCoord> = whole.iter().copied().collect();
        let mut listed: HashSet<u32> = self.items.iter().map(|it| it.slot).collect();
        let mut order: Vec<ChunkCoord> = touched.keys().copied().collect();
        order.sort_unstable_by_key(|c| [c.0[2], c.0[1], c.0[0]]);
        for c in order {
            let Some(rec) = full_record(c) else {
                continue;
            };
            let arcs: Vec<_> = (0..27)
                .map(|k| {
                    let d = [k % 3 - 1, (k / 3) % 3 - 1, k / 9 - 1];
                    let n = ChunkCoord([0, 1, 2].map(|a| c.0[a] + d[a]));
                    if full_record(n).is_some() {
                        world.store().get(n).map(Some)
                    } else if lod(n) {
                        Some(None)
                    } else {
                        None
                    }
                })
                .collect();
            let guards: Vec<_> = arcs
                .iter()
                .map(|a| match a {
                    Some(Some(a)) => Some(Some(
                        a.read().unwrap_or_else(std::sync::PoisonError::into_inner),
                    )),
                    Some(None) => Some(None),
                    None => None,
                })
                .collect();
            let around: [Around<'_>; 27] = std::array::from_fn(|k| match &guards[k] {
                Some(Some(g)) => Around::Full(g),
                Some(None) => Around::Lod,
                None => Around::Air,
            });
            let entry = self
                .chunks
                .entry(c)
                .or_insert_with(|| (rec, Box::new([ABSENT; CHUNK_BRICKS])));
            if entry.0 != rec {
                // The chunk's record moved: its slots stay, its row is rewritten.
                entry.0 = rec;
                rows.insert(c);
            }
            let mut slots = *entry.1;
            let mut changed = false;
            let mut set: Vec<usize> = touched[&c].iter().copied().collect();
            set.sort_unstable();
            for i in set {
                let had = slots[i];
                if is_band(&around, i) {
                    let slot = if had == ABSENT {
                        match self.alloc() {
                            Some(s) => s,
                            None => {
                                self.overflow += 1;
                                continue;
                            }
                        }
                    } else {
                        had
                    };
                    slots[i] = slot;
                    changed |= had == ABSENT;
                    if listed.insert(slot) {
                        self.items.push(FieldItem {
                            slot,
                            origin: world_brick(c, i).map(|v| v * BRICK_EDGE),
                        });
                    }
                } else if had != ABSENT {
                    slots[i] = ABSENT;
                    self.free.push(had);
                    changed = true;
                }
            }
            *self.chunks.get_mut(&c).expect("inserted above").1 = slots;
            if changed {
                rows.insert(c);
            }
        }
        for c in rows {
            if let Some((rec, slots)) = self.chunks.get(&c) {
                let (rec, slots) = (*rec, **slots);
                self.write_row(rec, &slots);
            }
        }
    }

    /// The work list `prepare` built.
    pub fn items(&self) -> &[FieldItem] {
        &self.items
    }

    /// Grows the shards to hold every slot handed out, writes the changed index rows, and
    /// records the field pass over the work list into `encoder` (nothing for an empty list).
    /// Returns the list's length.
    pub fn encode(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        mirror: &crate::attrib::AttribBindings<'_>,
        timestamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) -> usize {
        let device = self.device.clone();
        for k in 0..FIELD_SHARDS {
            let need = self
                .next
                .saturating_sub(k as u32 * self.slots_per_shard)
                .min(self.slots_per_shard);
            let have = self.shards[k].1;
            if need > have || (k == 0 && have == 1) {
                let slots = need
                    .next_power_of_two()
                    .max(FIRST_SHARD_SLOTS)
                    .min(self.slots_per_shard);
                let buffer = Self::shard_buffer(&device, slots);
                if have > 1 {
                    encoder.copy_buffer_to_buffer(
                        &self.shards[k].0,
                        0,
                        &buffer,
                        0,
                        u64::from(have) * FIELD_SLOT_BYTES,
                    );
                }
                self.shards[k] = (buffer, slots);
            }
        }
        // The rows after the copies above, in the encoder's order: a queue write would land
        // before a grown shard's copy and be overwritten by it.
        let mut rows: Vec<u32> = self.rows_changed.drain().collect();
        rows.sort_unstable();
        if !rows.is_empty() {
            let mut data: Vec<u32> = Vec::with_capacity(rows.len() * FIELD_RECORD_WORDS);
            for &rec in &rows {
                let base = rec as usize * FIELD_RECORD_WORDS;
                data.extend_from_slice(&self.index_cpu[base..base + FIELD_RECORD_WORDS]);
            }
            let staging = wgpu::util::DeviceExt::create_buffer_init(
                &device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("field rows"),
                    contents: bytemuck::cast_slice(&data),
                    usage: wgpu::BufferUsages::COPY_SRC,
                },
            );
            let row_bytes = (FIELD_RECORD_WORDS * 4) as u64;
            // Consecutive records in one copy.
            let mut k = 0;
            while k < rows.len() {
                let mut j = k + 1;
                while j < rows.len() && rows[j] == rows[j - 1] + 1 {
                    j += 1;
                }
                encoder.copy_buffer_to_buffer(
                    &staging,
                    k as u64 * row_bytes,
                    &self.shards[0].0,
                    u64::from(rows[k]) * row_bytes,
                    (j - k) as u64 * row_bytes,
                );
                k = j;
            }
        }
        let n = self.items.len();
        if n == 0 {
            return 0;
        }
        let groups = (n as u32).div_ceil(crate::attrib::BRICKS_PER_GROUP);
        let groups_x = groups.min(65_535);
        let groups_y = groups.div_ceil(groups_x);
        let params = Params {
            window_min: mirror.window.0,
            units_per_shard: mirror.units_per_shard,
            window_size: mirror.window.1,
            items: n as u32,
            groups_x,
            fit: 0,
            slots_per_shard: self.slots_per_shard,
            _pad: 0,
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&params));
        if self.work.as_ref().is_none_or(|w| w.1 < n) {
            let cap = n.next_power_of_two().max(256);
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("field_work"),
                size: (cap * size_of::<FieldItem>()) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.work = Some((buffer, cap));
        }
        let work = &self.work.as_ref().expect("built above").0;
        queue.write_buffer(work, 0, bytemuck::cast_slice(&self.items));
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: self.uniform.as_entire_binding(),
        }];
        for (k, b) in mirror.shards.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry {
                binding: 1 + k as u32,
                resource: b.as_entire_binding(),
            });
        }
        entries.push(wgpu::BindGroupEntry {
            binding: 5,
            resource: mirror.records.as_entire_binding(),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: 6,
            resource: mirror.table.as_entire_binding(),
        });
        for k in 0..FIELD_SHARDS {
            entries.push(wgpu::BindGroupEntry {
                binding: 8 + k as u32,
                resource: self.shards[k].0.as_entire_binding(),
            });
        }
        entries.push(wgpu::BindGroupEntry {
            binding: 11,
            resource: work.as_entire_binding(),
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("field"),
            layout: &self.layout,
            entries: &entries,
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("field"),
            timestamp_writes: timestamps,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(groups_x, groups_y, 1);
        drop(pass);
        self.items.clear();
        n
    }

    /// Slots whose band mask is not clear: the band bricks the pass found, read back.
    pub fn band_bricks(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> u64 {
        let mut live = vec![false; self.next as usize];
        for (_, slots) in self.chunks.values() {
            for &s in slots.iter().filter(|&&s| s != ABSENT) {
                live[s as usize] = true;
            }
        }
        let mut n = 0;
        for (k, (buf, slots)) in self.shards.iter().enumerate() {
            let first = k as u32 * self.slots_per_shard;
            let used = self.next.saturating_sub(first).min(*slots);
            if used == 0 || *slots == 1 {
                continue;
            }
            let bytes = crate::readback::read_buffer(
                device,
                queue,
                buf,
                0,
                u64::from(used) * FIELD_SLOT_BYTES,
            );
            for s in 0..used {
                if !live[(first + s) as usize] {
                    continue;
                }
                let o = (u64::from(s) * FIELD_SLOT_BYTES) as usize;
                if bytes[o..o + UNIT_BYTES as usize].iter().any(|&x| x != 0) {
                    n += 1;
                }
            }
        }
        n
    }

    /// The bricks of every chunk with slots, by kind, for a census: the live slots.
    pub fn chunks(&self) -> impl Iterator<Item = (&ChunkCoord, &[u32; CHUNK_BRICKS])> {
        self.chunks.iter().map(|(c, (_, s))| (c, &**s))
    }
}

/// Brick `slot` of chunk `c` in world brick coordinates.
pub fn world_brick(c: ChunkCoord, slot: usize) -> [i32; 3] {
    let l = brick_of_slot(slot);
    [0, 1, 2].map(|k| c.0[k] * CHUNK_EDGE_BRICKS + l[k])
}
