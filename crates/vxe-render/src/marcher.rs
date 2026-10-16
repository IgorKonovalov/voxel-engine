//! The primary-ray compute pass: one invocation per pixel of an `rgba8unorm` target.

use crate::camera::Camera;
use crate::gpu_world::GpuWorld;

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
    pub extent_min: [i32; 3],
    pub _pad2: i32,
    pub extent_size: [i32; 3],
    pub _pad3: i32,
    pub light_pos: [f32; 3],
    pub light_intensity: f32,
    pub light_dir: [f32; 3],
    pub cos_outer: f32,
    pub size: [u32; 2],
    pub cos_inner: f32,
    pub flags: u32,
}

/// Spot-lamp intensity in the shader's units: radiance at one voxel of distance, on axis.
const LAMP_INTENSITY: f32 = 1200.0;
/// Cone half-angles of the lamp, inner (full) and outer (zero), in radians.
const LAMP_INNER: f32 = 0.35;
const LAMP_OUTER: f32 = 0.70;

pub struct Marcher {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    target: wgpu::Texture,
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
        let target = device.create_texture(&wgpu::TextureDescriptor {
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
        });
        Self {
            pipeline,
            layout,
            params,
            target,
        }
    }

    pub fn target(&self) -> &wgpu::Texture {
        &self.target
    }

    /// The uniform block for `camera` over `world`, with the lamp at the eye.
    pub fn params(&self, world: &GpuWorld, camera: &Camera) -> Params {
        let (fwd, right, up) = camera.basis();
        let (w, h) = (self.target.width(), self.target.height());
        let e = world.extent();
        Params {
            cam_pos: camera.position.to_array(),
            tan_half_fov: (camera.vfov * 0.5).tan(),
            cam_right: right.to_array(),
            aspect: w as f32 / h as f32,
            cam_up: up.to_array(),
            cam_fwd: fwd.to_array(),
            extent_min: e.min.0,
            extent_size: e.size(),
            light_pos: camera.position.to_array(),
            light_intensity: LAMP_INTENSITY,
            light_dir: fwd.to_array(),
            cos_outer: LAMP_OUTER.cos(),
            size: [w, h],
            cos_inner: LAMP_INNER.cos(),
            ..Params::default()
        }
    }

    /// Records the march of `camera` over `world` into `encoder`.
    pub fn encode(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        world: &GpuWorld,
        params: &Params,
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
                    resource: world.pool().as_entire_binding(),
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
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("march"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(
            self.target.width().div_ceil(WORKGROUP),
            self.target.height().div_ceil(WORKGROUP),
            1,
        );
    }

    /// Marches `camera` over `world` and submits the work.
    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        world: &GpuWorld,
        camera: &Camera,
    ) {
        let params = self.params(world, camera);
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        self.encode(device, queue, &mut enc, world, &params);
        queue.submit([enc.finish()]);
    }
}
