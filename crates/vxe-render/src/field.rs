//! The field surface's band (ADR-0041): which voxels of a brick hold a count, as the CPU
//! decides it, and the field pass (`field.wgsl`) that fills the counts into the mirror's band
//! slots.
//!
//! A voxel's count is its solid voxels among the 27 around it, itself included, solid being
//! not air (a translucent voxel and a crisp one are solid). The field
//! pass also marks each solid band voxel smooth or crisp ([`SMOOTH_COUNT_BIT`],
//! [`CRISP_COUNT_BIT`]) from its material's row, and a band brick with no smooth solid voxel
//! near ([`BAND_CUBES_BIT`]). A voxel is in the band when
//! its count is neither 0 nor 27, and a brick is a band brick when it holds a band voxel.
//!
//! [`band_mask`] reads the brick and its 26 neighbours from the world as the GPU will read
//! them from the mirror: a chunk inside the mirror window that the world holds full by its
//! voxels, one it holds only as mips by its level-1 mip bricks (each mip voxel covering 2^3
//! voxels), any other as air. The GPU's counts and the CPU's mask then agree once the mirror
//! holds what the world does. The slot layout and its bookkeeping are `gpu_world.rs`'s.

use vxe_world::{CHUNK_EDGE_BRICKS, ChunkMips, brick_of_slot, brick_slot};

/// Units of a band slot of `n` band voxels: its band mask and `ceil(n / 64)` units of `u8`
/// counts.
pub fn band_slot_units(n: u32) -> u32 {
    1 + n.div_ceil(64)
}

/// Units of a band slot at most: 512 band voxels.
pub const MAX_BAND_UNITS: u32 = 9;

/// How the band reads a chunk around a brick: held full (its voxels), held as mips only (its
/// level-1 mip bricks), or air (outside the window, or not held).
pub enum Around<'a> {
    Air,
    Lod(&'a ChunkMips),
    Full(&'a vxe_world::Chunk),
}

/// The field's solid bits (bit `x + 8 y` of word `z`) of the brick at local brick coordinate
/// `l` (each component in -1..=8) of the chunk at the centre of `around`. A chunk held as mips
/// reads its mips' solid bits.
fn solid_at(around: &[Around<'_>; 27], l: [i32; 3]) -> [u64; 8] {
    let cd = l.map(|v| v.div_euclid(CHUNK_EDGE_BRICKS));
    let ci = ((cd[0] + 1) + 3 * (cd[1] + 1) + 9 * (cd[2] + 1)) as usize;
    let b = l.map(|v| v.rem_euclid(CHUNK_EDGE_BRICKS));
    match &around[ci] {
        Around::Air => [0; 8],
        Around::Full(c) => c.brick(brick_slot(b)).solid_bits(),
        Around::Lod(m) => {
            // Level-0 voxel `v` of brick `b` is level-1 voxel `4 (b % 2) + v / 2` of mip brick
            // `b / 2`.
            let h = b.map(|v| v / 2);
            let o = b.map(|v| (v % 2) * 4);
            let mip = m.level(1)[(h[0] + 4 * (h[1] + 4 * h[2])) as usize].solid_bits();
            let mut out = [0u64; 8];
            for (z, w) in out.iter_mut().enumerate() {
                let src = mip[o[2] as usize + z / 2];
                for y in 0..8 {
                    for x in 0..8 {
                        let bit = (o[0] as usize + x / 2) + 8 * (o[1] as usize + y / 2);
                        *w |= ((src >> bit) & 1) << (x + 8 * y);
                    }
                }
            }
            out
        }
    }
}

/// The band voxels of brick `slot` of the chunk at the centre of `around` (index `(dx + 1) +
/// 3 (dy + 1) + 9 (dz + 1)`), bit `i % 64` of word `i / 64` for voxel `i = x + 8 y + 64 z`: the
/// voxels whose 3 x 3 x 3 neighbourhood holds both a solid and an air voxel. A crisp voxel
/// (`MaterialFlags::CRISP`) is solid here as every other.
pub fn band_mask(around: &[Around<'_>; 27], slot: usize) -> [u64; 8] {
    let l = brick_of_slot(slot);
    let mut bricks = [[0u64; 8]; 27];
    let (mut all_solid, mut all_air) = (true, true);
    for (k, out) in bricks.iter_mut().enumerate() {
        let d = [k as i32 % 3 - 1, (k as i32 / 3) % 3 - 1, k as i32 / 9 - 1];
        *out = solid_at(around, [0, 1, 2].map(|a| l[a] + d[a]));
        all_solid &= out.iter().all(|&w| w == !0);
        all_air &= out.iter().all(|&w| w == 0);
    }
    if all_solid || all_air {
        return [0; 8];
    }
    // The brick and a one-voxel shell, 10^3: row `y + 10 z` holds voxels x - 1 for x in 0..10,
    // each voxel at offset `c - 1` from the brick's origin.
    let byte = |k: usize, ly: usize, lz: usize| ((bricks[k][lz] >> (8 * ly)) & 0xFF) as u16;
    let side = |c: usize| match c {
        0 => (0usize, 7usize),
        9 => (2, 0),
        _ => (1, c - 1),
    };
    let mut rows = [0u16; 100];
    for z in 0..10 {
        let (dz, lz) = side(z);
        for y in 0..10 {
            let (dy, ly) = side(y);
            let k = |dx: usize| dx + 3 * dy + 9 * dz;
            rows[y + 10 * z] = (byte(k(0), ly, lz) >> 7) & 1
                | byte(k(1), ly, lz) << 1
                | (byte(k(2), ly, lz) & 1) << 9;
        }
    }
    let dilate_x = |r: u16| (r | r << 1 | r >> 1) & 0x3FF;
    let mut out = [0u64; 8];
    for z in 1..9 {
        for y in 1..9 {
            let (mut s, mut a) = (0u16, 0u16);
            for dz in 0..3 {
                for dy in 0..3 {
                    let r = rows[(y + dy - 1) + 10 * (z + dz - 1)];
                    s |= dilate_x(r);
                    a |= dilate_x(!r & 0x3FF);
                }
            }
            let band = u64::from((s & a) >> 1 & 0xFF);
            out[z - 1] |= band << (8 * (y - 1));
        }
    }
    out
}

/// The bit of a band voxel's `u8` count that marks it a solid voxel of a crisp material
/// (`march.wgsl` hits it as its cube); the count itself is in the low five bits.
pub const CRISP_COUNT_BIT: u8 = 0x80;
/// The bit of a band voxel's `u8` count that marks it a solid voxel of a smooth material.
pub const SMOOTH_COUNT_BIT: u8 = 0x40;
/// The bit of a band brick's header band word ([`crate::gpu_world::HEADER_BAND_WORD`]) the field
/// pass sets when neither the brick nor its 26 neighbours hold a smooth solid voxel: the walk
/// crosses it as cubes.
pub const BAND_CUBES_BIT: u32 = 0x4000_0000;

/// Set bits of a band mask.
pub fn band_count(m: &[u64; 8]) -> u32 {
    m.iter().map(|w| w.count_ones()).sum()
}

/// The band index of voxel `i` in a brick of band mask `m`: its band voxels below it.
pub fn band_index(m: &[u64; 8], i: usize) -> u32 {
    let z = i / 64;
    let below: u32 = m[..z].iter().map(|w| w.count_ones()).sum();
    below + (m[z] & ((1u64 << (i % 64)) - 1)).count_ones()
}

/// Bricks one workgroup of the field pass fills, 64 invocations each: item `4 g + b` is brick
/// `b` of workgroup `g`, the last workgroup partly empty when the list is not a multiple of
/// four. Must match `field.wgsl`'s `BRICKS`.
pub const BRICKS_PER_GROUP: u32 = 4;

/// The buffers of the mirror the field pass binds.
pub struct FieldBindings<'a> {
    /// The four pool shards, placeholders included.
    pub shards: [&'a wgpu::Buffer; 4],
    pub records: &'a wgpu::Buffer,
    pub table: &'a wgpu::Buffer,
    /// The material rows (`march.wgsl`'s `materials`), whose flags mark the crisp rows.
    pub materials: &'a wgpu::Buffer,
    /// The mirror window's minimum corner and size, in chunks.
    pub window: ([i32; 3], [i32; 3]),
    pub units_per_shard: u32,
}

/// One work-list entry of the field pass: the brick's band slot (its first unit) and its
/// origin in voxels.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FieldItem {
    pub unit: u32,
    pub origin: [i32; 3],
}

/// The field pass's uniform: `field.wgsl`'s `Params`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    window_min: [i32; 3],
    units_per_shard: u32,
    window_size: [i32; 3],
    items: u32,
    groups_x: u32,
    _pad: [u32; 3],
}

/// The field pass: its pipeline and the buffers it keeps between calls.
pub struct FieldPass {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    work: Option<(wgpu::Buffer, usize)>,
}

impl FieldPass {
    pub fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("field.wgsl"),
            source: wgpu::ShaderSource::Wgsl(crate::FIELD_WGSL.into()),
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
                buffer(11, ro),
                buffer(12, ro),
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
        Self {
            pipeline,
            layout,
            uniform,
            work: None,
        }
    }

    /// Records the field pass over `items` into `encoder` (nothing for an empty list).
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        mirror: &FieldBindings<'_>,
        items: &[FieldItem],
        timestamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let n = items.len();
        if n == 0 {
            return;
        }
        let groups = (n as u32).div_ceil(BRICKS_PER_GROUP);
        let groups_x = groups.min(65_535);
        let groups_y = groups.div_ceil(groups_x);
        let params = Params {
            window_min: mirror.window.0,
            units_per_shard: mirror.units_per_shard,
            window_size: mirror.window.1,
            items: n as u32,
            groups_x,
            _pad: [0; 3],
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
        queue.write_buffer(work, 0, bytemuck::cast_slice(items));
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
        entries.push(wgpu::BindGroupEntry {
            binding: 11,
            resource: work.as_entire_binding(),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: 12,
            resource: mirror.materials.as_entire_binding(),
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
    }
}

/// Brick `slot` of chunk `c` in world brick coordinates.
pub fn world_brick(c: vxe_world::ChunkCoord, slot: usize) -> [i32; 3] {
    let l = brick_of_slot(slot);
    [0, 1, 2].map(|k| c.0[k] * CHUNK_EDGE_BRICKS + l[k])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_index_counts_the_band_voxels_below() {
        let mut m = [0u64; 8];
        m[0] = 0b1011;
        m[2] = 1 << 5;
        assert_eq!(band_index(&m, 0), 0);
        assert_eq!(band_index(&m, 3), 2);
        assert_eq!(band_index(&m, 128 + 5), 3);
        assert_eq!(band_count(&m), 4);
        assert_eq!(band_slot_units(0), 1);
        assert_eq!(band_slot_units(64), 2);
        assert_eq!(band_slot_units(65), 3);
        assert_eq!(band_slot_units(512), MAX_BAND_UNITS);
    }
}
