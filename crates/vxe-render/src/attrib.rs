//! The attribute pass (`attrib.wgsl`): fills the contour attributes of a work list of bricks
//! inside their mirror slots. `GpuWorld::update_attrs` builds the list and records the
//! pass.

/// Invocations per workgroup; must match `@workgroup_size` in `attrib.wgsl`.
pub const WORKGROUP: u32 = 256;
/// Bricks one workgroup fills, 64 invocations each: item `4 g + b` is brick `b` of workgroup
/// `g`, the last workgroup partly empty when the list is not a multiple of four.
pub const BRICKS_PER_GROUP: u32 = 4;
/// Most workgroups along one dispatch dimension (the WebGPU default limit).
const MAX_GROUPS_X: u32 = 65_535;

/// How the attribute pass fits a surface voxel's plane normal: the gradient of occupancy over
/// its 3 x 3 x 3 neighbourhood with every neighbour weighing 1, or each weighing `1 / |o|^2`
/// for its offset `o` (face 1, edge 1/2, corner 1/3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlaneFit {
    #[default]
    Uniform27,
    Weighted27,
}

impl PlaneFit {
    pub fn name(self) -> &'static str {
        match self {
            Self::Uniform27 => "uniform27",
            Self::Weighted27 => "weighted27",
        }
    }
}

/// How the attribute pass places a surface voxel's plane along its normal: `(f - 0.5) * 3`
/// voxels for the solid fraction `f` of its 27 voxels, or the offset at which the plane's
/// solid half-space holds the block's solid volume exactly (ADR-0011's encoding either way:
/// clamped to 7/8 and rounded to 1/8).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlaneOffset {
    #[default]
    Fraction,
    VolumeMatched,
}

impl PlaneOffset {
    pub fn name(self) -> &'static str {
        match self {
            Self::Fraction => "fraction",
            Self::VolumeMatched => "volume",
        }
    }
}

/// The attribute pass's plane fit: its normal and its offset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SurfaceFit {
    pub fit: PlaneFit,
    pub offset: PlaneOffset,
}

impl SurfaceFit {
    /// The bits of `attrib.wgsl`'s `params.fit`.
    fn bits(self) -> u32 {
        u32::from(self.fit == PlaneFit::Weighted27)
            | u32::from(self.offset == PlaneOffset::VolumeMatched) << 1
    }

    /// `uniform27` or `weighted27`, `+ volume` for the volume-matched offset.
    pub fn name(self) -> String {
        match self.offset {
            PlaneOffset::Fraction => self.fit.name().to_owned(),
            PlaneOffset::VolumeMatched => format!("{} + volume", self.fit.name()),
        }
    }
}

/// One work-list entry: the brick's record entry less its `FULL_BIT` (its slot's first unit,
/// class and translucent flag, which locate its attributes) and the brick's origin in
/// voxels.
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
    units_per_shard: u32,
    window_size: [i32; 3],
    items: u32,
    groups_x: u32,
    fit: u32,
    _pad: [u32; 2],
}

/// The buffers of the mirror the pass binds.
pub struct AttribBindings<'a> {
    /// The four pool shards, placeholders included.
    pub shards: [&'a wgpu::Buffer; 4],
    pub records: &'a wgpu::Buffer,
    pub table: &'a wgpu::Buffer,
    /// The mirror window's minimum corner and size, in chunks.
    pub window: ([i32; 3], [i32; 3]),
    pub units_per_shard: u32,
}

/// The pass, with the buffers it keeps between calls: its uniform, its work list (sized by
/// [`AttribPass::reserve`] or grown on demand, never shrunk) and the bind group over them and the mirror's buffers.
pub struct AttribPass {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    /// The work-list buffer and its capacity in items.
    work: Option<(wgpu::Buffer, usize)>,
    /// The bind group and the mirror buffers it binds, in binding order (shards, records,
    /// table); rebuilt when the work buffer or one of those is replaced.
    bind: Option<(wgpu::BindGroup, [wgpu::Buffer; 6])>,
    builds: u32,
    /// Items the first work buffer holds at least; 0 sizes it by the first list.
    reserve: usize,
    fit: SurfaceFit,
}

/// Fewest items a work buffer holds.
const MIN_WORK_ITEMS: usize = 256;

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
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("attrib params"),
            size: size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            layout,
            uniform,
            work: None,
            bind: None,
            builds: 0,
            reserve: 0,
            fit: SurfaceFit::default(),
        }
    }

    /// The plane fit the next encoded passes compute; bricks filled before keep theirs.
    pub fn set_fit(&mut self, fit: SurfaceFit) {
        self.fit = fit;
    }

    pub fn fit(&self) -> SurfaceFit {
        self.fit
    }

    /// Sizes the work buffer, when it is first built, for at least `items` items, so a caller
    /// whose lists are bounded builds it once. A list longer than the buffer still grows it.
    pub fn reserve(&mut self, items: usize) {
        self.reserve = items;
    }

    /// Work buffers built so far: the first at first use, then one each time a list outgrew
    /// the last.
    pub fn buffer_builds(&self) -> u32 {
        self.builds
    }

    /// The work buffer's capacity in items, 0 before first use.
    pub fn work_capacity(&self) -> usize {
        self.work.as_ref().map_or(0, |w| w.1)
    }

    /// Records the pass over `items` into `encoder`, four items a workgroup. Records nothing
    /// for an empty list.
    ///
    /// The uniform and the work list are written through `queue`, so they take effect at the
    /// next `queue.submit`, ahead of every command buffer it submits: at most one encoded
    /// pass may be submitted per `submit` call, or the later list overwrites the earlier.
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        b: &AttribBindings<'_>,
        items: &[AttribItem],
        timestamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        if items.is_empty() {
            return;
        }
        let groups = (items.len() as u32).div_ceil(BRICKS_PER_GROUP);
        let groups_x = groups.min(MAX_GROUPS_X);
        let groups_y = groups.div_ceil(groups_x);
        let params = Params {
            window_min: b.window.0,
            units_per_shard: b.units_per_shard,
            window_size: b.window.1,
            items: items.len() as u32,
            groups_x,
            fit: self.fit.bits(),
            _pad: [0; 2],
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&params));
        if self.work_capacity() < items.len() {
            let cap = if items.len() <= self.reserve {
                self.reserve
            } else {
                items.len().next_power_of_two().max(MIN_WORK_ITEMS)
            };
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("attrib_work"),
                size: (cap * size_of::<AttribItem>()) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.work = Some((buffer, cap));
            self.bind = None;
            self.builds += 1;
        }
        let work = &self.work.as_ref().expect("built above").0;
        queue.write_buffer(work, 0, bytemuck::cast_slice(items));
        let bound = [
            b.shards[0].clone(),
            b.shards[1].clone(),
            b.shards[2].clone(),
            b.shards[3].clone(),
            b.records.clone(),
            b.table.clone(),
        ];
        if self.bind.as_ref().is_none_or(|(_, was)| *was != bound) {
            let mut entries = vec![wgpu::BindGroupEntry {
                binding: 0,
                resource: self.uniform.as_entire_binding(),
            }];
            for (k, buffer) in bound.iter().enumerate() {
                entries.push(wgpu::BindGroupEntry {
                    binding: 1 + k as u32,
                    resource: buffer.as_entire_binding(),
                });
            }
            entries.push(wgpu::BindGroupEntry {
                binding: 7,
                resource: work.as_entire_binding(),
            });
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("attrib"),
                layout: &self.layout,
                entries: &entries,
            });
            self.bind = Some((group, bound));
        }
        let group = &self.bind.as_ref().expect("built above").0;
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("attrib"),
            timestamp_writes: timestamps,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.dispatch_workgroups(groups_x, groups_y, 1);
    }
}
