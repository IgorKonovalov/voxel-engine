//! The primary-ray compute pass: one invocation per pixel of an `rgba8unorm` target.

use crate::camera::Camera;
use crate::gpu_world::GpuWorld;
use crate::light::LightParams;

/// Workgroup edge in pixels; must match `@workgroup_size` in `march.wgsl`.
pub const WORKGROUP: u32 = 8;

/// The uniform block of `march.wgsl`, field for field.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Params {
    pub cam_pos: [f32; 3],
    pub tan_half_fov: f32,
    pub cam_right: [f32; 3],
    pub aspect: f32,
    pub cam_up: [f32; 3],
    pub _pad0: f32,
    pub cam_fwd: [f32; 3],
    pub _pad1: f32,
    pub window_min: [i32; 3],
    pub _pad2: i32,
    pub window_size: [i32; 3],
    pub _pad3: i32,
    pub light_pos: [f32; 3],
    pub light_intensity: f32,
    pub light_dir: [f32; 3],
    pub cos_outer: f32,
    pub size: [u32; 2],
    pub cos_inner: f32,
    pub flags: u32,
    /// Pool slots per shard: slot `s` is slot `s % n` of shard `s / n`.
    pub slots_per_shard: u32,
    /// The mip level every LOD-only chunk is walked at, 1 to 3; 0 picks it by distance.
    pub forced_level: u32,
    /// A LOD-only chunk within this squared chunk distance of the camera chunk is walked at
    /// level 1, within `lod2_r2` at level 2, and farther at level 3.
    pub lod1_r2: i32,
    pub lod2_r2: i32,
}

/// `Params::flags` bit: trace the shadow ray.
pub const FLAG_SHADOWS: u32 = 1;
/// `Params::flags` bit: skip empty blocks through the brick occupancy masks.
pub const FLAG_MASKS: u32 = 2;

/// How the marcher picks the mip level of a LOD-only chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LodParams {
    /// The streamer's full-detail radius in chunks. A LOD-only chunk within squared chunk
    /// distance `(2 r_full)^2` of the camera chunk is walked at level 1, within
    /// `(4 r_full)^2` at level 2, and farther at level 3.
    pub r_full: i32,
    /// Walks every LOD-only chunk at this level instead (1 to 3), for tests.
    pub forced_level: Option<u32>,
}

impl Default for LodParams {
    fn default() -> Self {
        Self {
            r_full: 16,
            forced_level: None,
        }
    }
}

pub struct Marcher {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    target: wgpu::Texture,
    lod: LodParams,
}

impl Marcher {
    /// A marcher drawing into a `width` × `height` target.
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("march.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/march.wgsl").into()),
        });
        let storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("march"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                storage(1),
                storage(2),
                storage(3),
                storage(4),
                storage(6),
                storage(7),
                storage(8),
                storage(9),
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("march"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("march"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("march params"),
            size: size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            layout,
            params,
            target: create_target(device, width, height),
            lod: LodParams::default(),
        }
    }

    /// Sets how LOD-only chunks pick their mip level.
    pub fn set_lod(&mut self, lod: LodParams) {
        self.lod = lod;
    }

    pub fn lod(&self) -> LodParams {
        self.lod
    }

    pub fn target(&self) -> &wgpu::Texture {
        &self.target
    }

    /// Replaces the target with one of `width` × `height`; the pipeline is kept.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        self.target = create_target(device, width, height);
    }

    /// The uniform block for `camera` over `world`, lit by `light`.
    pub fn params(&self, world: &GpuWorld, camera: &Camera, light: &LightParams) -> Params {
        let (fwd, right, up) = camera.basis();
        let (w, h) = (self.target.width(), self.target.height());
        let e = world.window();
        Params {
            cam_pos: camera.position.to_array(),
            tan_half_fov: (camera.vfov * 0.5).tan(),
            cam_right: right.to_array(),
            aspect: w as f32 / h as f32,
            cam_up: up.to_array(),
            cam_fwd: fwd.to_array(),
            window_min: e.min.0,
            window_size: e.size(),
            light_pos: (camera.position + right * light.offset_right - up * light.offset_down)
                .to_array(),
            light_intensity: light.intensity,
            light_dir: fwd.to_array(),
            cos_outer: light.outer.cos(),
            size: [w, h],
            cos_inner: light.inner.cos(),
            flags: if light.shadows { FLAG_SHADOWS } else { 0 }
                | if light.masks { FLAG_MASKS } else { 0 },
            slots_per_shard: world.shard_layout().slots_per_shard,
            forced_level: self.lod.forced_level.unwrap_or(0),
            lod1_r2: (2 * self.lod.r_full).pow(2),
            lod2_r2: (4 * self.lod.r_full).pow(2),
            ..Params::default()
        }
    }

    /// Records the march of `camera` over `world` into `encoder`, writing the pass's start and
    /// end timestamps when `timestamps` is given.
    pub fn encode(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        world: &GpuWorld,
        params: &Params,
        timestamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(params));
        let view = self
            .target
            .create_view(&wgpu::TextureViewDescriptor::default());
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("march"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: world.pool_shard(0).as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: world.pool_shard(1).as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: world.pool_shard(2).as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: world.pool_shard(3).as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: world.records().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: world.table().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: world.materials().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: world.masks().as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("march"),
            timestamp_writes: timestamps,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(
            self.target.width().div_ceil(WORKGROUP),
            self.target.height().div_ceil(WORKGROUP),
            1,
        );
    }

    /// Marches `camera` over `world` lit by `light`, and submits the work.
    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        world: &GpuWorld,
        camera: &Camera,
        light: &LightParams,
    ) {
        let params = self.params(world, camera, light);
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        self.encode(device, queue, &mut enc, world, &params, None);
        queue.submit([enc.finish()]);
    }
}

fn create_target(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("march target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

/// GPU duration of the march pass through two timestamp queries. Needs
/// `Features::TIMESTAMP_QUERY` on the device.
pub struct MarchTimer {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
}

impl MarchTimer {
    pub fn new(device: &wgpu::Device) -> Self {
        let queries = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("march timer"),
            ty: wgpu::QueryType::Timestamp,
            count: 2,
        });
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("march timer resolve"),
            size: 2 * size_of::<u64>() as u64,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        Self { queries, resolve }
    }

    /// The timestamp writes to pass to [`Marcher::encode`].
    pub fn writes(&self) -> wgpu::ComputePassTimestampWrites<'_> {
        wgpu::ComputePassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: Some(0),
            end_of_pass_write_index: Some(1),
        }
    }

    /// Records the copy of both timestamps into the resolve buffer; call after the pass.
    pub fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.resolve_query_set(&self.queries, 0..2, &self.resolve, 0);
    }

    /// Milliseconds between the two timestamps of the last resolved pass. Blocks until the
    /// GPU has finished it.
    pub fn read_ms(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> f64 {
        let bytes = crate::readback::read_buffer(device, queue, &self.resolve, 0, 16);
        let t: &[u64] = bytemuck::cast_slice(&bytes);
        let ticks = t[1].saturating_sub(t[0]);
        ticks as f64 * f64::from(queue.get_timestamp_period()) / 1.0e6
    }
}
