//! The attribute pass (`attrib.wgsl`): fills the contour attributes of a work list of bricks
//! in the mirror's attribute pages. `GpuWorld::update_attrs` builds the list and records the
//! pass.

/// Invocations per workgroup, one per pool word of a brick's 1024-byte attribute; must match
/// `@workgroup_size` in `attrib.wgsl`. One workgroup fills one brick.
pub const WORKGROUP: u32 = 256;
/// Most workgroups along one dispatch dimension (the WebGPU default limit).
const MAX_GROUPS_X: u32 = 65_535;

/// One work-list entry: a brick slot in an attribute page and the brick's origin in voxels.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct AttribItem {
    pub slot: u32,
    pub origin: [i32; 3],
}

/// The uniform block of `attrib.wgsl`, field for field.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    window_min: [i32; 3],
    slots_per_shard: u32,
    window_size: [i32; 3],
    items: u32,
    groups_x: u32,
    _pad: [u32; 3],
}

/// The buffers of the mirror the pass binds.
pub struct AttribBindings<'a> {
    /// The four pool shards, placeholders included.
    pub shards: [&'a wgpu::Buffer; 4],
    pub records: &'a wgpu::Buffer,
    pub table: &'a wgpu::Buffer,
    /// The mirror window's minimum corner and size, in chunks.
    pub window: ([i32; 3], [i32; 3]),
    pub slots_per_shard: u32,
}

pub struct AttribPass {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

impl AttribPass {
    pub fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("attrib.wgsl"),
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
            label: Some("attrib"),
            entries: &[
                buffer(0, wgpu::BufferBindingType::Uniform),
                buffer(1, rw),
                buffer(2, rw),
                buffer(3, rw),
                buffer(4, rw),
                buffer(5, ro),
                buffer(6, ro),
                buffer(7, ro),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("attrib"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("attrib"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self { pipeline, layout }
    }

    /// Records the pass over `items` into `encoder`, one workgroup per item. Records nothing
    /// for an empty list.
    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        b: &AttribBindings<'_>,
        items: &[AttribItem],
        timestamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        if items.is_empty() {
            return;
        }
        let groups = items.len() as u32;
        let groups_x = groups.min(MAX_GROUPS_X);
        let groups_y = groups.div_ceil(groups_x);
        let params = Params {
            window_min: b.window.0,
            slots_per_shard: b.slots_per_shard,
            window_size: b.window.1,
            items: groups,
            groups_x,
            _pad: [0; 3],
        };
        let uniform = init_buffer(
            device,
            "attrib params",
            bytemuck::bytes_of(&params),
            wgpu::BufferUsages::UNIFORM,
        );
        let work = init_buffer(
            device,
            "attrib_work",
            bytemuck::cast_slice(items),
            wgpu::BufferUsages::STORAGE,
        );
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        }];
        for (k, s) in b.shards.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry {
                binding: 1 + k as u32,
                resource: s.as_entire_binding(),
            });
        }
        entries.extend([
            wgpu::BindGroupEntry {
                binding: 5,
                resource: b.records.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: b.table.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: work.as_entire_binding(),
            },
        ]);
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("attrib"),
            layout: &self.layout,
            entries: &entries,
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("attrib"),
            timestamp_writes: timestamps,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(groups_x, groups_y, 1);
    }
}

/// A buffer of `usage` holding `bytes`, zero-padded to at least 16 bytes.
fn init_buffer(
    device: &wgpu::Device,
    label: &str,
    bytes: &[u8],
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    use wgpu::util::DeviceExt;
    let mut contents = bytes.to_vec();
    contents.resize(bytes.len().max(16).next_multiple_of(4), 0);
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: &contents,
        usage,
    })
}
