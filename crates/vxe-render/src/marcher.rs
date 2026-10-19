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
/// `Params::flags` bits 2..4: the [`NormalMode`].
pub const NORMAL_SHIFT: u32 = 2;
/// `Params::flags` bit: perturb the shading with per-material procedural detail.
pub const FLAG_DETAIL: u32 = 1 << 4;
/// `Params::flags` bits 5..8: the [`ViewMode`].
pub const VIEW_SHIFT: u32 = 5;
/// `Params::flags` bit: the mirror holds attribute pages (`MirrorConfig::surface_attrs`).
pub const FLAG_ATTR_PAGES: u32 = 1 << 8;
/// `Params::flags` bit: skip empty cells larger than a voxel (`LightParams::skips`).
pub const FLAG_SKIPS: u32 = 1 << 9;

/// How the marcher finds the normal at a hit, and in `Contour` the hit itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NormalMode {
    /// The normal of the voxel face the ray entered through.
    #[default]
    Face,
    /// The gradient of the trilinearly interpolated occupancy of the 2 x 2 x 2 voxels around
    /// the hit point pushed half a voxel into the surface.
    Trilinear8,
    /// Central differences of occupancy over the 3 x 3 x 3 voxels around the hit voxel.
    Gradient27,
    /// The per-voxel contour plane read from the mirror's attribute pages; it moves the hit
    /// too. Needs a mirror with `surface_attrs`; without one it shades as `Face`.
    Contour,
}

impl NormalMode {
    pub const ALL: [Self; 4] = [
        Self::Face,
        Self::Trilinear8,
        Self::Gradient27,
        Self::Contour,
    ];

    /// The lower-case name the fly-through prints and parses.
    pub fn name(self) -> &'static str {
        match self {
            Self::Face => "face",
            Self::Trilinear8 => "trilinear8",
            Self::Gradient27 => "gradient27",
            Self::Contour => "contour",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }

    /// The next mode in `ALL` order, wrapping, skipping `Contour` unless `contour` is set.
    pub fn next(self, contour: bool) -> Self {
        let i = Self::ALL.iter().position(|&m| m == self).unwrap_or(0);
        let n = Self::ALL[(i + 1) % Self::ALL.len()];
        if n == Self::Contour && !contour {
            Self::Face
        } else {
            n
        }
    }
}

/// What the marcher writes to its target.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewMode {
    /// The lit image, display-encoded.
    #[default]
    Shaded,
    /// The shading normal `n` as `n * 0.5 + 0.5` per channel, linear; black on a miss.
    Normals,
    /// White where the primary ray hits, black elsewhere.
    HitMask,
    /// The primary ray's walk cost, linear: red `min(steps, 255) / 255` for the iterations
    /// of the walk's loop (the hitting one included), green `min(loads, 255) / 255` for its
    /// 32-bit storage loads from the mirror (chunk table, record bits and entries, masks,
    /// pool, palette), blue `min(pool loads, 255) / 255` for the pool loads among them.
    /// [`crate::readback::StepHistogram`] reads it back.
    Steps,
}

impl ViewMode {
    pub const ALL: [Self; 4] = [Self::Shaded, Self::Normals, Self::HitMask, Self::Steps];

    pub fn name(self) -> &'static str {
        match self {
            Self::Shaded => "shaded",
            Self::Normals => "normals",
            Self::HitMask => "hitmask",
            Self::Steps => "steps",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }
}

/// The surface look: normal mode, procedural detail and view. The default is face normals,
/// no detail, shaded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LookParams {
    pub normal_mode: NormalMode,
    pub detail: bool,
    pub view: ViewMode,
}

impl LookParams {
    /// The look's bits of `Params::flags`.
    pub fn flags(&self) -> u32 {
        (self.normal_mode as u32) << NORMAL_SHIFT
            | if self.detail { FLAG_DETAIL } else { 0 }
            | (self.view as u32) << VIEW_SHIFT
    }
}

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
    look: LookParams,
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
            look: LookParams::default(),
        }
    }

    /// Sets how LOD-only chunks pick their mip level.
    pub fn set_lod(&mut self, lod: LodParams) {
        self.lod = lod;
    }

    pub fn lod(&self) -> LodParams {
        self.lod
    }

    /// Sets the surface look: normal mode, detail and view.
    pub fn set_look(&mut self, look: LookParams) {
        self.look = look;
    }

    pub fn look(&self) -> LookParams {
        self.look
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
                | if light.masks { FLAG_MASKS } else { 0 }
                | if light.skips { FLAG_SKIPS } else { 0 }
                | self.look.flags()
                | if world.surface_attrs() {
                    FLAG_ATTR_PAGES
                } else {
                    0
                },
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

/// Bytes one resolved timestamp occupies: a `u64` tick count.
const QUERY_BYTES: u64 = 8;

/// GPU duration of one pass through a two-timestamp query set: the pass writes its start and
/// end, [`PassTimer::resolve`] copies them out after it, and [`PassTimer::read_ms`] maps them.
///
/// The resolve buffer (`QUERY_RESOLVE | COPY_SRC`) and the readback buffer (`COPY_DST |
/// MAP_READ`) are separate because wgpu forbids one buffer carrying both usages. The
/// timestamp period is read off the queue once, at construction.
pub struct PassTimer {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
    /// Nanoseconds per timestamp tick.
    period_ns: f32,
}

/// The march pass's timer; the march and the HUD are timed alike.
pub type MarchTimer = PassTimer;

impl PassTimer {
    /// A timer, or `None` when the device lacks `Features::TIMESTAMP_QUERY` (the software
    /// adapter among them), so a caller there gets no timer rather than a validation error.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let bytes = 2 * QUERY_BYTES;
        Some(Self {
            queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("pass timer"),
                ty: wgpu::QueryType::Timestamp,
                count: 2,
            }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pass timer resolve"),
                size: bytes,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            readback: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pass timer readback"),
                size: bytes,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
            period_ns: queue.get_timestamp_period(),
        })
    }

    /// The timestamp writes for a compute pass, such as [`Marcher::encode`]'s.
    pub fn writes(&self) -> wgpu::ComputePassTimestampWrites<'_> {
        wgpu::ComputePassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: Some(0),
            end_of_pass_write_index: Some(1),
        }
    }

    /// The timestamp writes for a render pass, such as the HUD overlay's.
    pub fn render_writes(&self) -> wgpu::RenderPassTimestampWrites<'_> {
        wgpu::RenderPassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: Some(0),
            end_of_pass_write_index: Some(1),
        }
    }

    /// Records the resolve of both timestamps and their copy to the readback buffer; call
    /// after the timed pass, in the same or a later encoder.
    pub fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.resolve_query_set(&self.queries, 0..2, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.readback, 0, 2 * QUERY_BYTES);
    }

    /// Milliseconds between the two timestamps of the last resolved pass; zero when they are
    /// out of order. Blocks until the GPU has finished the submission that resolved them.
    pub fn read_ms(&self, device: &wgpu::Device) -> f64 {
        self.readback.map_async(wgpu::MapMode::Read, .., |r| {
            r.expect("timer readback maps");
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("device poll for the timer readback");
        let ticks = {
            let bytes = self
                .readback
                .get_mapped_range(..)
                .expect("timer readback mapped");
            let t: &[u64] = bytemuck::cast_slice(&bytes);
            t[1].saturating_sub(t[0])
        };
        self.readback.unmap();
        ticks as f64 * f64::from(self.period_ns) / 1.0e6
    }
}
