//! The frame: the hit pass ([`Marcher`], `march.wgsl`), the light pass (`light.wgsl`) and the
//! composite (`composite.wgsl`), each a compute dispatch over the internal target (ADR-0018).

use crate::camera::{Camera, View};
use crate::gpu_world::GpuWorld;
use crate::light::LightParams;
use crate::{COMPOSITE_WGSL, LIGHT_WGSL, MARCH_WGSL};

/// Workgroup edge in pixels; must match `@workgroup_size` in every frame shader.
pub const WORKGROUP: u32 = 8;

/// Views a frame holds at most; must match `MAX_VIEWS` in `march.wgsl`.
pub const MAX_VIEWS: usize = 2;

/// The uniform block every frame shader reads, field for field.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Params {
    /// The views; `view_count` of them are in use.
    pub views: [View; MAX_VIEWS],
    pub view_count: u32,
    /// Translucent voxels composited in front of the opaque hit, nearest first; 0 ignores
    /// them.
    pub translucent_layers: u32,
    /// The frame's index, for per-frame random numbers.
    pub frame: u32,
    /// [`Lighting::flags`].
    pub lighting: u32,
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
    /// A LOD-only chunk within this squared chunk distance of the first view's chunk is
    /// walked at level 1, within `lod2_r2` at level 2, and farther at level 3.
    pub lod1_r2: i32,
    pub lod2_r2: i32,
    /// Radiance is multiplied by this before the tone map.
    pub exposure: f32,
    /// Light samples from 0 to the buffer's high-water mark, which the light pass draws
    /// candidates from ([`GpuWorld::light_span`]).
    pub light_span: u32,
    /// Emissive candidates per pixel ([`Lighting::candidates`]).
    pub candidates: u32,
    pub _pad4: u32,
}

/// `Params::lighting` bit: the headlamp lights the frame.
pub const LIGHTING_LAMP: u32 = 1;
/// `Params::lighting` bit: the composite's tone map is the identity clamp.
pub const LIGHTING_IDENTITY_TONE_MAP: u32 = 1 << 6;

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
/// `Params::flags` bit: the primary ray passes through solid until it has crossed real air
/// (`LookParams::cutaway`).
pub const FLAG_CUTAWAY: u32 = 1 << 10;

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
    /// The opaque walk alone: the translucent walk does not run in this view.
    /// [`crate::readback::StepHistogram`] reads it back.
    Steps,
    /// The composited albedo, unlit and linear: each translucent layer's `albedo × opacity`
    /// front to back over the opaque hit's albedo; black on a miss.
    Albedo,
}

impl ViewMode {
    pub const ALL: [Self; 5] = [
        Self::Shaded,
        Self::Normals,
        Self::HitMask,
        Self::Steps,
        Self::Albedo,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Shaded => "shaded",
            Self::Normals => "normals",
            Self::HitMask => "hitmask",
            Self::Steps => "steps",
            Self::Albedo => "albedo",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }
}

/// The surface look: normal mode, procedural detail, view, translucent layers and cutaway. The
/// default is face normals, no detail, shaded, 4 layers, cutaway off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LookParams {
    pub normal_mode: NormalMode,
    pub detail: bool,
    pub view: ViewMode,
    /// Translucent voxels composited in front of the opaque hit, nearest first; 0 ignores
    /// them. The translucent walk runs only when the opaque walk read a brick entry flagged
    /// translucent, so a scene without the class pays nothing.
    pub translucent_layers: u32,
    /// The cutaway view: the primary ray passes through solid voxels until it has entered an
    /// empty cell of a resident chunk, then hits the next surface as usual. A chunk outside
    /// the window or absent from the mirror is not air, so from outside the world each
    /// cavern's far wall shows through the rock and rock with no cavern behind it is a miss.
    /// A ray that starts in air walks as without it; shadow rays never pass. In the shaded view a hit is lit by a fixed
    /// hemisphere light (`albedo × (0.3 + 0.7 × (0.5 + 0.5 n.z))`, world z up) with no lamp and
    /// no shadow ray, and a miss is a flat grey-blue sky.
    pub cutaway: bool,
}

impl Default for LookParams {
    fn default() -> Self {
        Self {
            normal_mode: NormalMode::default(),
            detail: false,
            view: ViewMode::default(),
            translucent_layers: 4,
            cutaway: false,
        }
    }
}

impl LookParams {
    /// The look's bits of `Params::flags`.
    pub fn flags(&self) -> u32 {
        (self.normal_mode as u32) << NORMAL_SHIFT
            | if self.detail { FLAG_DETAIL } else { 0 }
            | (self.view as u32) << VIEW_SHIFT
            | if self.cutaway { FLAG_CUTAWAY } else { 0 }
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

/// How the composite maps HDR radiance to the display.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToneMap {
    /// The ACES filmic fit (Narkowicz 2015), then `pow(c, 1 / 2.2)`.
    #[default]
    Aces,
    /// Clamp to `[0, 1]`, then `pow(c, 1 / 2.2)`: the march's display encoding before the
    /// frame was split into passes, kept for tests.
    Identity,
}

/// The frame's lighting knobs beside the lamp's own ([`LightParams`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lighting {
    /// Radiance is multiplied by this before the tone map.
    pub exposure: f32,
    pub tone_map: ToneMap,
    /// The headlamp lights the frame; off leaves emissive light alone.
    pub lamp: bool,
    /// Emissive candidates the light pass resamples per pixel; 0 turns emissive light off.
    pub candidates: u32,
}

impl Default for Lighting {
    fn default() -> Self {
        Self {
            exposure: 1.0,
            tone_map: ToneMap::default(),
            lamp: true,
            candidates: 8,
        }
    }
}

impl Lighting {
    /// The bits of `Params::lighting`.
    pub fn flags(&self) -> u32 {
        (if self.tone_map == ToneMap::Identity {
            LIGHTING_IDENTITY_TONE_MAP
        } else {
            0
        }) | if self.lamp { LIGHTING_LAMP } else { 0 }
    }
}

/// The hit pass: `march.wgsl`'s `main`, one primary ray per pixel into the hit buffer.
pub struct Marcher {
    pipeline: wgpu::ComputePipeline,
    out_layout: wgpu::BindGroupLayout,
}

impl Marcher {
    fn new(device: &wgpu::Device, world_layout: &wgpu::BindGroupLayout) -> Self {
        let out_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hit out"),
            entries: &[
                storage_texture(0, wgpu::TextureFormat::Rgba32Float),
                storage_texture(1, wgpu::TextureFormat::Rgba16Uint),
                storage_texture(2, wgpu::TextureFormat::Rgba16Float),
            ],
        });
        let pipeline = compute_pipeline(
            device,
            "hit",
            MARCH_WGSL.into(),
            &[Some(world_layout), Some(&out_layout)],
            "main",
        );
        Self {
            pipeline,
            out_layout,
        }
    }
}

/// One frame of the renderer: the hit pass ([`Marcher`]), the light pass and the composite,
/// with their targets at one internal resolution. [`Frame::target`] is the `rgba8unorm`
/// image the presenter draws.
pub struct Frame {
    world_layout: wgpu::BindGroupLayout,
    hit: Marcher,
    light_layout: wgpu::BindGroupLayout,
    light: wgpu::ComputePipeline,
    composite_layout: wgpu::BindGroupLayout,
    composite: wgpu::ComputePipeline,
    params: wgpu::Buffer,
    targets: Targets,
    lod: LodParams,
    look: LookParams,
    lighting: Lighting,
    /// The per-material emission scale, an `f32` per material id, 1.0 unless set.
    emission_scale: wgpu::Buffer,
    emission_cpu: Vec<f32>,
    /// The blue-noise texture, uploaded by the first `encode`.
    blue_noise: wgpu::Texture,
    blue_noise_written: std::sync::atomic::AtomicBool,
    /// The frame index the random numbers are seeded with.
    frame: u32,
}

/// The 128 x 128 blue-noise ranks (`assets/blue_noise_128.r8`, made by
/// `assets/blue_noise_128.py`: void and cluster, Ulichney 1993), one byte per texel, row-major.
const BLUE_NOISE: &[u8; 128 * 128] = include_bytes!("../assets/blue_noise_128.r8");

/// Material ids the emission-scale buffer covers: every `u16`.
const EMISSION_ROWS: usize = 1 << 16;

impl Frame {
    /// A frame drawing into a `width` × `height` target.
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
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
        let world_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("world"),
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
                storage(10),
                storage(11),
                sampled_texture(12, FLOAT),
            ],
        });
        let hit = Marcher::new(device, &world_layout);
        let light_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("light io"),
            entries: &[
                sampled_texture(0, FLOAT),
                sampled_texture(1, UINT),
                storage_texture(2, wgpu::TextureFormat::Rgba16Float),
            ],
        });
        let light = compute_pipeline(
            device,
            "light",
            format!("{MARCH_WGSL}\n{LIGHT_WGSL}"),
            &[Some(&world_layout), None, Some(&light_layout)],
            "shade",
        );
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("composite io"),
            entries: &[
                sampled_texture(0, FLOAT),
                sampled_texture(1, UINT),
                sampled_texture(2, FLOAT),
                sampled_texture(3, FLOAT),
                storage_texture(4, wgpu::TextureFormat::Rgba8Unorm),
            ],
        });
        let composite = compute_pipeline(
            device,
            "composite",
            format!("{MARCH_WGSL}\n{COMPOSITE_WGSL}"),
            &[Some(&world_layout), None, Some(&composite_layout)],
            "composite",
        );
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame params"),
            size: size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            world_layout,
            hit,
            light_layout,
            light,
            composite_layout,
            composite,
            params,
            targets: Targets::new(device, width, height),
            lod: LodParams::default(),
            look: LookParams::default(),
            lighting: Lighting::default(),
            emission_scale: wgpu::util::DeviceExt::create_buffer_init(
                device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("emission scale"),
                    contents: bytemuck::cast_slice(&vec![1.0f32; EMISSION_ROWS]),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                },
            ),
            emission_cpu: vec![1.0; EMISSION_ROWS],
            blue_noise: device.create_texture(&wgpu::TextureDescriptor {
                label: Some("blue noise"),
                size: wgpu::Extent3d {
                    width: 128,
                    height: 128,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            }),
            blue_noise_written: std::sync::atomic::AtomicBool::new(false),
            frame: 0,
        }
    }

    /// The frame index the next `params` seeds its random numbers with.
    pub fn frame_index(&self) -> u32 {
        self.frame
    }

    pub fn set_frame_index(&mut self, frame: u32) {
        self.frame = frame;
    }

    /// Moves to the next frame index; call once per rendered frame.
    pub fn advance(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    /// Scales material `material`'s emission by `scale` from the next frame on: a visual
    /// knob that leaves the world untouched (an authoritative change is a `Replace` op).
    pub fn set_emission_scale(&mut self, queue: &wgpu::Queue, material: u16, scale: f32) {
        let i = usize::from(material);
        if self.emission_cpu[i] != scale {
            self.emission_cpu[i] = scale;
            queue.write_buffer(
                &self.emission_scale,
                i as u64 * 4,
                bytemuck::bytes_of(&scale),
            );
        }
    }

    pub fn emission_scale(&self, material: u16) -> f32 {
        self.emission_cpu[usize::from(material)]
    }

    /// The emission-scale buffer, an `f32` per material id.
    pub fn emission_scales(&self) -> &wgpu::Buffer {
        &self.emission_scale
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

    /// Sets exposure and tone map.
    pub fn set_lighting(&mut self, lighting: Lighting) {
        self.lighting = lighting;
    }

    pub fn lighting(&self) -> Lighting {
        self.lighting
    }

    /// The composite's `rgba8unorm` output.
    pub fn target(&self) -> &wgpu::Texture {
        &self.targets.output
    }

    /// The hit buffer's position texture (rgba32float: world position in voxels, then `t`,
    /// `3e38` on a miss).
    pub fn hit_positions(&self) -> &wgpu::Texture {
        &self.targets.hit_pos
    }

    /// The hit buffer's record texture (rgba16uint: octahedral normal u and v as snorm16
    /// bits, material id, flags: bit 0 hit, bit 1 contour-plane hit, bit 2 translucent layers
    /// in front, bits 3..5 the entry face's axis plus one, 0 for none).
    pub fn hit_records(&self) -> &wgpu::Texture {
        &self.targets.hit_info
    }

    /// The light pass's irradiance (rgba16float, rgb).
    pub fn irradiance(&self) -> &wgpu::Texture {
        &self.targets.irradiance
    }

    /// Bytes the frame's targets occupy.
    pub fn frame_bytes(&self) -> u64 {
        self.targets.bytes()
    }

    /// Replaces the targets with ones of `width` × `height`; the pipelines are kept.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        self.targets = Targets::new(device, width, height);
    }

    /// The uniform block for `camera` over `world`, lit by `light`: one view filling the
    /// target.
    pub fn params(&self, world: &GpuWorld, camera: &Camera, light: &LightParams) -> Params {
        let rect = [0, 0, self.targets.width, self.targets.height];
        self.params_for(world, &[View::of_camera(camera, rect)], camera, light)
    }

    /// The uniform block for `views` (at most [`MAX_VIEWS`]; a pixel outside every rectangle
    /// takes the first) over `world`, lit by `light` placed relative to `camera`.
    pub fn params_for(
        &self,
        world: &GpuWorld,
        views: &[View],
        camera: &Camera,
        light: &LightParams,
    ) -> Params {
        assert!(
            (1..=MAX_VIEWS).contains(&views.len()),
            "{} views",
            views.len()
        );
        let (fwd, right, up) = camera.basis();
        let mut vs = [View::default(); MAX_VIEWS];
        vs[..views.len()].copy_from_slice(views);
        let e = world.window();
        Params {
            views: vs,
            view_count: views.len() as u32,
            translucent_layers: self.look.translucent_layers,
            lighting: self.lighting.flags(),
            window_min: e.min.0,
            window_size: e.size(),
            light_pos: (camera.position + right * light.offset_right - up * light.offset_down)
                .to_array(),
            light_intensity: light.intensity,
            light_dir: fwd.to_array(),
            cos_outer: light.outer.cos(),
            size: [self.targets.width, self.targets.height],
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
            exposure: self.lighting.exposure,
            light_span: world.light_span(),
            candidates: self.lighting.candidates,
            frame: self.frame,
            ..Params::default()
        }
    }

    /// Records the frame's passes over `world` into `encoder`. With `timer` (of at least
    /// `FRAME_PASSES.len()` passes) pass `i` writes its start and end into timestamps `2 i`
    /// and `2 i + 1`; the caller resolves them.
    pub fn encode(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        world: &GpuWorld,
        params: &Params,
        timer: Option<&PassTimer>,
    ) {
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(params));
        if !self
            .blue_noise_written
            .swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            queue.write_texture(
                self.blue_noise.as_image_copy(),
                BLUE_NOISE,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(128),
                    rows_per_image: Some(128),
                },
                self.blue_noise.size(),
            );
        }
        let noise = view(&self.blue_noise);
        let t = &self.targets;
        let world_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("world"),
            layout: &self.world_layout,
            entries: &[
                buffer_entry(0, &self.params),
                buffer_entry(1, world.pool_shard(0)),
                buffer_entry(7, world.pool_shard(1)),
                buffer_entry(8, world.pool_shard(2)),
                buffer_entry(9, world.pool_shard(3)),
                buffer_entry(2, world.records()),
                buffer_entry(3, world.table()),
                buffer_entry(4, world.materials()),
                buffer_entry(6, world.masks()),
                buffer_entry(10, world.light_buffer()),
                buffer_entry(11, &self.emission_scale),
                wgpu::BindGroupEntry {
                    binding: 12,
                    resource: wgpu::BindingResource::TextureView(&noise),
                },
            ],
        });
        let (pos, info, layer, irr, out) = (
            view(&t.hit_pos),
            view(&t.hit_info),
            view(&t.layer),
            view(&t.irradiance),
            view(&t.output),
        );
        let hit_bind = bind_views(device, &self.hit.out_layout, &[&pos, &info, &layer]);
        let light_bind = bind_views(device, &self.light_layout, &[&pos, &info, &irr]);
        let composite_bind = bind_views(
            device,
            &self.composite_layout,
            &[&pos, &info, &layer, &irr, &out],
        );
        let groups = [t.width.div_ceil(WORKGROUP), t.height.div_ceil(WORKGROUP)];
        let passes: [(&wgpu::ComputePipeline, &wgpu::BindGroup, u32); 3] = [
            (&self.hit.pipeline, &hit_bind, 1),
            (&self.light, &light_bind, 2),
            (&self.composite, &composite_bind, 2),
        ];
        for (i, (pipeline, bind, group)) in passes.into_iter().enumerate() {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(FRAME_PASSES[i]),
                timestamp_writes: timer.map(|t| t.writes_at(i as u32)),
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &world_bind, &[]);
            pass.set_bind_group(group, bind, &[]);
            pass.dispatch_workgroups(groups[0], groups[1], 1);
        }
    }

    /// Renders `camera` over `world` lit by `light`, and submits the work.
    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        world: &GpuWorld,
        camera: &Camera,
        light: &LightParams,
    ) {
        let params = self.params(world, camera, light);
        self.submit(device, queue, world, &params);
    }

    /// Renders the frame `params` describes over `world`, and submits the work.
    pub fn submit(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        world: &GpuWorld,
        params: &Params,
    ) {
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        self.encode(device, queue, &mut enc, world, params, None);
        queue.submit([enc.finish()]);
    }
}

fn buffer_entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

/// A bind group of `views` at bindings `0..views.len()` of `layout`.
fn bind_views(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    views: &[&wgpu::TextureView],
) -> wgpu::BindGroup {
    let entries: Vec<wgpu::BindGroupEntry<'_>> = views
        .iter()
        .enumerate()
        .map(|(i, v)| wgpu::BindGroupEntry {
            binding: i as u32,
            resource: wgpu::BindingResource::TextureView(v),
        })
        .collect();
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout,
        entries: &entries,
    })
}

/// Passes of a frame in encode order, as [`Frame::encode`] times them: pass `i` writes
/// timestamps `2 i` and `2 i + 1` of the timer it is given.
pub const FRAME_PASSES: [&str; 3] = ["hit", "light", "composite"];

/// The frame's targets at one internal resolution.
struct Targets {
    width: u32,
    height: u32,
    /// The hit buffer: position + t (rgba32float), and octahedral normal, material and flags
    /// (rgba16uint).
    hit_pos: wgpu::Texture,
    hit_info: wgpu::Texture,
    /// The translucent layers in front of each hit (rgba16float), or the steps view's
    /// counters.
    layer: wgpu::Texture,
    /// The light pass's irradiance (rgba16float).
    irradiance: wgpu::Texture,
    /// The composite's display-encoded output (rgba8unorm).
    output: wgpu::Texture,
}

impl Targets {
    fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let tex = |label, format, extra| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC
                    | extra,
                view_formats: &[],
            })
        };
        let none = wgpu::TextureUsages::empty();
        Self {
            width,
            height,
            hit_pos: tex("hit pos", wgpu::TextureFormat::Rgba32Float, none),
            hit_info: tex("hit info", wgpu::TextureFormat::Rgba16Uint, none),
            layer: tex("layer", wgpu::TextureFormat::Rgba16Float, none),
            irradiance: tex("irradiance", wgpu::TextureFormat::Rgba16Float, none),
            output: tex("frame target", wgpu::TextureFormat::Rgba8Unorm, none),
        }
    }

    /// Bytes the targets occupy.
    fn bytes(&self) -> u64 {
        let px = u64::from(self.width) * u64::from(self.height);
        px * (16 + 8 + 8 + 8 + 4)
    }
}

fn view(t: &wgpu::Texture) -> wgpu::TextureView {
    t.create_view(&wgpu::TextureViewDescriptor::default())
}

fn storage_texture(binding: u32, format: wgpu::TextureFormat) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

fn sampled_texture(
    binding: u32,
    sample_type: wgpu::TextureSampleType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

const FLOAT: wgpu::TextureSampleType = wgpu::TextureSampleType::Float { filterable: false };
const UINT: wgpu::TextureSampleType = wgpu::TextureSampleType::Uint;

fn compute_pipeline(
    device: &wgpu::Device,
    label: &str,
    source: String,
    layouts: &[Option<&wgpu::BindGroupLayout>],
    entry: &str,
) -> wgpu::ComputePipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: layouts,
        immediate_size: 0,
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        module: &module,
        entry_point: Some(entry),
        compilation_options: Default::default(),
        cache: None,
    })
}

/// Bytes one resolved timestamp occupies: a `u64` tick count.
const QUERY_BYTES: u64 = 8;

/// GPU durations of one or more passes through a query set of two timestamps per pass: pass
/// `i` writes its start and end into timestamps `2 i` and `2 i + 1`, [`PassTimer::resolve`]
/// copies them out after the passes, and [`PassTimer::read_all_ms`] maps them.
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
    passes: u32,
}

/// The march pass's timer; the march and the HUD are timed alike.
pub type MarchTimer = PassTimer;

impl PassTimer {
    /// A timer of one pass, or `None` when the device lacks `Features::TIMESTAMP_QUERY` (the
    /// software adapter among them), so a caller there gets no timer rather than a validation
    /// error.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        Self::with_passes(device, queue, 1)
    }

    /// A timer of `passes` passes, such as [`Frame::encode`]'s [`FRAME_PASSES`]; `None` as
    /// [`PassTimer::new`].
    pub fn with_passes(device: &wgpu::Device, queue: &wgpu::Queue, passes: u32) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let bytes = 2 * QUERY_BYTES * u64::from(passes);
        Some(Self {
            queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("pass timer"),
                ty: wgpu::QueryType::Timestamp,
                count: 2 * passes,
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
            passes,
        })
    }

    pub fn passes(&self) -> u32 {
        self.passes
    }

    /// The timestamp writes for a compute pass timed as pass 0.
    pub fn writes(&self) -> wgpu::ComputePassTimestampWrites<'_> {
        self.writes_at(0)
    }

    /// The timestamp writes for a compute pass timed as pass `i`.
    pub fn writes_at(&self, i: u32) -> wgpu::ComputePassTimestampWrites<'_> {
        assert!(i < self.passes, "pass {i} of {}", self.passes);
        wgpu::ComputePassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: Some(2 * i),
            end_of_pass_write_index: Some(2 * i + 1),
        }
    }

    /// The timestamp writes for a render pass timed as pass 0, such as the HUD overlay's.
    pub fn render_writes(&self) -> wgpu::RenderPassTimestampWrites<'_> {
        wgpu::RenderPassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: Some(0),
            end_of_pass_write_index: Some(1),
        }
    }

    /// Records the resolve of every timestamp and their copy to the readback buffer; call
    /// after the timed passes, in the same or a later encoder.
    pub fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        let n = 2 * self.passes;
        encoder.resolve_query_set(&self.queries, 0..n, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(
            &self.resolve,
            0,
            &self.readback,
            0,
            u64::from(n) * QUERY_BYTES,
        );
    }

    /// Milliseconds of pass 0 of the last resolve; see [`PassTimer::read_all_ms`].
    pub fn read_ms(&self, device: &wgpu::Device) -> f64 {
        self.read_all_ms(device)[0]
    }

    /// Milliseconds between each pass's two timestamps of the last resolve, by pass; zero
    /// when they are out of order. Blocks until the GPU has finished the submission that
    /// resolved them.
    pub fn read_all_ms(&self, device: &wgpu::Device) -> Vec<f64> {
        self.readback.map_async(wgpu::MapMode::Read, .., |r| {
            r.expect("timer readback maps");
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("device poll for the timer readback");
        let ticks: Vec<u64> = {
            let bytes = self
                .readback
                .get_mapped_range(..)
                .expect("timer readback mapped");
            let t: &[u64] = bytemuck::cast_slice(&bytes);
            t.chunks(2).map(|p| p[1].saturating_sub(p[0])).collect()
        };
        self.readback.unmap();
        ticks
            .into_iter()
            .map(|t| t as f64 * f64::from(self.period_ns) / 1.0e6)
            .collect()
    }
}
