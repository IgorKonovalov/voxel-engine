//! The frame: the hit pass ([`Marcher`], `march.wgsl`), the light pass (`light.wgsl`) and the
//! composite (`composite.wgsl`), each a compute dispatch over the internal target (ADR-0018).

use crate::camera::{Camera, View};
use crate::gpu_world::GpuWorld;
use crate::light::LightParams;
use crate::{ATROUS_WGSL, COMPOSITE_WGSL, GRADIENT_WGSL, LIGHT_WGSL, MARCH_WGSL, TEMPORAL_WGSL};
use vxe_world::ChunkCoord;

/// The upscale pass and its resolve, compiled after [`MARCH_WGSL`].
pub const UPSCALE_WGSL: &str = include_str!("shaders/upscale.wgsl");

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
    /// The slot of the light-sample buffer where the frame's candidate table starts.
    pub light_table_base: u32,
    /// Bit 0: the previous frame's hit buffer and history are valid (set by
    /// [`Frame::encode`]).
    pub history: u32,
    pub _pad5: [u32; 3],
    /// The previous frame's views, for reprojection (set by [`Frame::encode`]).
    pub prev_views: [View; MAX_VIEWS],
    /// The lighting grid's size, full-resolution pixels per lighting pixel on an edge, and
    /// this and last frame's sampled pixel of each 2 x 2 (x in bits 0..8, y in 8..16; the
    /// last set by [`Frame::encode`]).
    pub light_size: [u32; 2],
    pub light_scale: u32,
    pub light_offset: u32,
    pub prev_light_offset: u32,
    /// The emissive part's history length in frames ([`Lighting::emissive_history`]).
    pub emissive_history: f32,
    /// The size of the frame's image: the output size with the upscaler, else `size`.
    pub output_size: [u32; 2],
}

/// `Params::lighting` bit: the headlamp lights the frame.
pub const LIGHTING_LAMP: u32 = 1;
/// `Params::lighting` bit: shadow rays take the coarse walk ([`ShadowWalk::Coarse`]).
pub const LIGHTING_COARSE_SHADOW: u32 = 1 << 1;
/// `Params::lighting` bit: the light pass reuses last frame's reservoirs ([`Sampler::Reuse`]).
pub const LIGHTING_REUSE: u32 = 1 << 2;
/// `Params::lighting` bit: the temporal pass also shortens history by A-SVGF's gradient
/// ([`History::Gradient`]).
pub const LIGHTING_GRADIENT: u32 = 1 << 3;
/// The emissive part's history in frames, by default.
pub const DEFAULT_EMISSIVE_HISTORY: u32 = 32;
/// Frames after an emission-scale change (the frame of the change too) during which the
/// emissive part blends at no less
/// than the lamp's 0.2 (`0.8^11 = 0.086`: a step settles within 10 % in them).
pub const EMISSION_CHANGE_FRAMES: u32 = 11;
/// `Params::lighting` bit: an emission scale changed within the last
/// [`EMISSION_CHANGE_FRAMES`] frames.
pub const LIGHTING_EMISSION_CHANGED: u32 = 1 << 8;
/// `Params::lighting` bit: the a-trous filter is off ([`Lighting::filter`]).
pub const LIGHTING_FILTER_OFF: u32 = 1 << 5;
/// `Params::lighting` bit: candidates are drawn uniformly over the buffer
/// ([`CandidateDraw::Uniform`]).
pub const LIGHTING_UNIFORM_DRAW: u32 = 1 << 7;
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

/// The walk a shadow ray takes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShadowWalk {
    /// Every voxel, as the primary ray.
    Full,
    /// Any 2^3 block of the opaque masks with a set bit is solid: chunk bits, brick bits and
    /// one mask line per brick, never a voxel or the pool. The first and last 3.5 voxels of
    /// the ray take the full walk. The default.
    #[default]
    Coarse,
}

impl ShadowWalk {
    pub const ALL: [Self; 2] = [Self::Full, Self::Coarse];

    pub fn name(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Coarse => "coarse",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }
}

/// How the light pass picks its emissive sample.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sampler {
    /// Resampling over the frame's candidates alone (the flat list).
    List,
    /// The candidates plus last frame's reservoirs at the reprojected pixel and five
    /// neighbours within 30 pixels (ReSTIR); the default.
    #[default]
    Reuse,
}

impl Sampler {
    pub const ALL: [Self; 2] = [Self::List, Self::Reuse];

    pub fn name(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Reuse => "reuse",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }
}

/// What kills the temporal pass's history.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HistoryKill {
    /// The frame's edit boxes alone.
    #[default]
    Edits,
    /// The edit boxes and A-SVGF's gradient: where re-shading last frame's sample with this
    /// frame's lights changes it, accumulation shortens.
    Gradient,
}

impl HistoryKill {
    pub const ALL: [Self; 2] = [Self::Edits, Self::Gradient];

    pub fn name(self) -> &'static str {
        match self {
            Self::Edits => "edits",
            Self::Gradient => "gradient",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }
}

/// The resolution the light, gradient, temporal and a-trous passes run at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LightingResolution {
    /// The internal resolution.
    Full,
    /// Half of it on each edge (the default: the only tier under the lighting line on the dev
    /// box), on a hit buffer sampled from the full one (one pixel of each
    /// 2 x 2, the lower-left one every frame); the composite upsamples by depth and normal.
    #[default]
    Half,
}

impl LightingResolution {
    pub const ALL: [Self; 2] = [Self::Full, Self::Half];

    pub fn name(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Half => "half",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }

    /// Full-resolution pixels per lighting pixel on an edge.
    pub fn scale(self) -> u32 {
        match self {
            Self::Full => 1,
            Self::Half => 2,
        }
    }
}

/// The sampled pixel of each 2 x 2 at half resolution (x, y): the lower-left, the same every
/// frame, so a still camera's upsampled image holds still.
const HALF_OFFSET: [u32; 2] = [0, 1];

/// Where the light pass draws its emissive candidates from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CandidateDraw {
    /// The frame's table of emitter chunks near the camera, by power over squared distance
    /// (ADR-0025).
    #[default]
    Table,
    /// Uniformly over the whole light-sample buffer; for tests.
    Uniform,
}

/// The ratio of the output size to the internal size the frame renders at, on each edge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderScale {
    /// The internal size is the output size.
    #[default]
    One,
    /// Two internal pixels per three output pixels on an edge (720p for 1080p): 44 % of the
    /// output's pixels.
    OneHalf,
    /// One internal pixel per two output pixels on an edge: 25 % of the output's pixels.
    Two,
}

impl RenderScale {
    pub const ALL: [Self; 3] = [Self::One, Self::OneHalf, Self::Two];

    /// The name the fly-through prints and parses.
    pub fn name(self) -> &'static str {
        match self {
            Self::One => "1",
            Self::OneHalf => "1.5",
            Self::Two => "2",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }

    /// The internal size for an `output`-sized image: each edge divided by the scale and
    /// rounded to the nearest pixel, at least 1.
    pub fn internal(self, output: [u32; 2]) -> [u32; 2] {
        output.map(|n| {
            match self {
                Self::One => n,
                Self::OneHalf => (2 * n + 1) / 3,
                Self::Two => n.div_ceil(2),
            }
            .max(1)
        })
    }
}

/// Phases of the jitter sequence: frame `f` takes phase `f % JITTER_PHASES`.
pub const JITTER_PHASES: u32 = 16;

/// The radical inverse of `i` in base `b`: element `i` of the Halton sequence of that base.
fn halton(mut i: u32, b: u32) -> f32 {
    let (mut f, mut r) = (1.0f32, 0.0f32);
    while i > 0 {
        f /= b as f32;
        r += f * (i % b) as f32;
        i /= b;
    }
    r
}

/// The sub-pixel jitter of frame `frame`, in internal pixels: elements `1 ..= 16` of the
/// Halton (2, 3) sequence by `frame % JITTER_PHASES`, centred on zero, each component in
/// `(-0.5, 0.5)`.
pub fn jitter_at(frame: u32) -> [f32; 2] {
    let i = frame % JITTER_PHASES + 1;
    [halton(i, 2) - 0.5, halton(i, 3) - 0.5]
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
    pub shadow_walk: ShadowWalk,
    /// The a-trous filter runs; off, the composite shows the light pass's raw irradiance.
    pub filter: bool,
    pub draw: CandidateDraw,
    pub sampler: Sampler,
    pub history: HistoryKill,
    /// Frames of history the emissive part of the irradiance blends over at least (static
    /// light holds still); the lamp's part keeps alpha 0.2.
    pub emissive_history: u32,
}

impl Default for Lighting {
    fn default() -> Self {
        Self {
            exposure: 1.0,
            tone_map: ToneMap::default(),
            lamp: true,
            candidates: 8,
            shadow_walk: ShadowWalk::default(),
            filter: true,
            draw: CandidateDraw::default(),
            sampler: Sampler::default(),
            history: HistoryKill::default(),
            emissive_history: DEFAULT_EMISSIVE_HISTORY,
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
            | if self.shadow_walk == ShadowWalk::Coarse {
                LIGHTING_COARSE_SHADOW
            } else {
                0
            }
            | if self.filter { 0 } else { LIGHTING_FILTER_OFF }
            | if self.history == HistoryKill::Gradient {
                LIGHTING_GRADIENT
            } else {
                0
            }
            | if self.sampler == Sampler::Reuse {
                LIGHTING_REUSE
            } else {
                0
            }
            | if self.draw == CandidateDraw::Uniform {
                LIGHTING_UNIFORM_DRAW
            } else {
                0
            }
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

/// One frame of the renderer: the hit pass ([`Marcher`]), the light pass, the temporal pass
/// and the composite, with their targets at one internal resolution. [`Frame::target`] is the
/// `rgba8unorm` image the presenter draws.
///
/// The hit buffer and the history are kept twice, this frame's and the last: [`Frame::encode`]
/// writes one copy and reads the other, and [`Frame::advance`] swaps them. History holds while
/// frames are encoded with `encode` and advanced; [`Frame::render`] draws one frame alone,
/// with no history.
pub struct Frame {
    world_layout: wgpu::BindGroupLayout,
    hit: Marcher,
    light_layout: wgpu::BindGroupLayout,
    light: wgpu::ComputePipeline,
    temporal_layout: wgpu::BindGroupLayout,
    temporal: wgpu::ComputePipeline,
    gradient_layout: wgpu::BindGroupLayout,
    gradient: wgpu::ComputePipeline,
    variance_layout: wgpu::BindGroupLayout,
    variance: wgpu::ComputePipeline,
    atrous_layout: wgpu::BindGroupLayout,
    atrous: wgpu::ComputePipeline,
    /// The a-trous iterations' strides, one uniform each.
    steps: [wgpu::Buffer; ATROUS_ITERATIONS],
    composite_layout: wgpu::BindGroupLayout,
    composite: wgpu::ComputePipeline,
    params: wgpu::Buffer,
    edits: wgpu::Buffer,
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
    /// The frame index of the last emission-scale change.
    emission_changed_at: Option<u32>,
    /// The copy of the double-buffered targets the next `encode` writes.
    cur: usize,
    resolution: LightingResolution,
    downsample_layout: wgpu::BindGroupLayout,
    downsample: wgpu::ComputePipeline,
    upscale_layout: wgpu::BindGroupLayout,
    upscale: wgpu::ComputePipeline,
    resolve_layout: wgpu::BindGroupLayout,
    resolve: wgpu::ComputePipeline,
    history: std::sync::Mutex<History>,
    /// The output size the frame's image is presented at; the targets are at the internal
    /// size.
    output: [u32; 2],
    /// The views' rays are jittered by [`jitter_at`] of the frame index.
    jitter: bool,
}

/// A-trous iterations per frame, at strides 1, 2, 4, 8.
pub const ATROUS_ITERATIONS: usize = 4;

/// Edit boxes one frame takes; one more flushes all history.
pub const MAX_EDITS: usize = 64;

/// What the next frame's temporal pass reads of the past.
#[derive(Default)]
struct History {
    /// The previous frame's views, and whether its hit buffer and history are valid.
    prev_views: [View; MAX_VIEWS],
    valid: bool,
    /// The views of the last `encode` with history, and whether one ran since `advance`.
    last_views: [View; MAX_VIEWS],
    encoded: bool,
    /// The lighting grid's offset of the previous frame and of the last encode.
    prev_offset: u32,
    last_offset: u32,
    /// The edit boxes collected for the next `encode`, and whether more than `MAX_EDITS`
    /// arrived.
    edits: Vec<[[i32; 3]; 2]>,
    overflow: bool,
    /// The view mode changed since the last encode: the upscaler's history holds another
    /// view's image.
    view_changed: bool,
}

/// Entries the candidate table holds at most.
pub const LIGHT_TABLE_ENTRIES: usize = 1024;

/// The light pass's candidate table as written past the light-sample reservation
/// (`GpuWorld::light_table_base`): a header slot `count, total (f32 bits), 0, 0`, then per
/// entry `start, len`, the cumulative weight (f32 bits) and a pad.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LightTableUniform {
    count: u32,
    total: f32,
    _pad: [u32; 2],
    chunks: [[u32; 4]; LIGHT_TABLE_ENTRIES],
}

/// The candidate table for a camera at `eye` over `world`: the chunks holding a range within
/// the light radius of the camera's chunk (squared chunk distance), the nearest
/// [`LIGHT_TABLE_ENTRIES`] of them, in chunk-coordinate order, each weighing its power over
/// `max(d^2, 64^2)`, `d` the eye's distance in voxels to the chunk's box.
fn light_table(world: &GpuWorld, eye: [f32; 3]) -> LightTableUniform {
    let r = i64::from(world.light_radius());
    let cam = eye.map(|v| (v / 64.0).floor() as i32);
    let mut near: Vec<(i64, ChunkCoord, u32, u32, f32)> = world
        .light_chunks()
        .map(|(c, start, len, power)| {
            let d2: i64 = (0..3).map(|k| i64::from(c.0[k] - cam[k]).pow(2)).sum();
            (d2, c, start, len, power)
        })
        .filter(|&(d2, ..)| d2 <= r * r)
        .collect();
    near.sort_unstable_by_key(|&(d2, c, ..)| (d2, c.0));
    near.truncate(LIGHT_TABLE_ENTRIES);
    near.sort_unstable_by_key(|&(_, c, ..)| c.0);
    let mut t = LightTableUniform {
        count: near.len() as u32,
        total: 0.0,
        _pad: [0; 2],
        chunks: [[0; 4]; LIGHT_TABLE_ENTRIES],
    };
    let mut cdf = 0.0f32;
    for (i, &(_, c, start, len, power)) in near.iter().enumerate() {
        let d2: f32 = (0..3)
            .map(|k| {
                let lo = c.0[k] as f32 * 64.0;
                let gap = (lo - eye[k]).max(eye[k] - (lo + 64.0)).max(0.0);
                gap * gap
            })
            .sum();
        cdf += power / d2.max(64.0 * 64.0);
        t.chunks[i] = [start, len, cdf.to_bits(), 0];
    }
    t.total = cdf;
    t
}

/// The temporal pass's edit boxes, field for field with `Edits` in `temporal.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct EditsUniform {
    count: u32,
    _pad: [u32; 3],
    boxes: [[i32; 4]; 2 * MAX_EDITS],
}

/// The 128 x 128 blue-noise ranks (`assets/blue_noise_128.r8`, made by
/// `assets/blue_noise_128.py`: void and cluster, Ulichney 1993), one byte per texel, row-major.
const BLUE_NOISE: &[u8; 128 * 128] = include_bytes!("../assets/blue_noise_128.r8");

/// Material ids the emission-scale buffer covers: every `u16`.
const EMISSION_ROWS: usize = 1 << 16;

/// A group-2 binding of a frame pass.
#[derive(Clone, Copy)]
enum Io {
    /// A `texture_2d<f32>` read with `textureLoad`.
    Float,
    /// A `texture_2d<u32>`.
    Uint,
    /// A write-only storage texture of this format.
    Store(wgpu::TextureFormat),
    /// A uniform buffer.
    Uniform,
}

fn io_layout(device: &wgpu::Device, label: &str, io: &[Io]) -> wgpu::BindGroupLayout {
    let at: Vec<(u32, Io)> = io.iter().enumerate().map(|(i, &k)| (i as u32, k)).collect();
    io_layout_at(device, label, &at)
}

/// A group-2 layout of `io` at explicit binding numbers.
fn io_layout_at(device: &wgpu::Device, label: &str, io: &[(u32, Io)]) -> wgpu::BindGroupLayout {
    let entries: Vec<wgpu::BindGroupLayoutEntry> = io
        .iter()
        .map(|&(b, k)| match k {
            Io::Float => sampled_texture(b, FLOAT),
            Io::Uint => sampled_texture(b, UINT),
            Io::Store(f) => storage_texture(b, f),
            Io::Uniform => wgpu::BindGroupLayoutEntry {
                binding: b,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        })
        .collect();
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &entries,
    })
}

/// One binding of a pass's group-2 bind group.
enum Res<'a> {
    Tex(&'a wgpu::Texture),
    Buf(&'a wgpu::Buffer),
}

fn bind_io(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    res: &[Res<'_>],
) -> wgpu::BindGroup {
    let at: Vec<u32> = (0..res.len() as u32).collect();
    bind_io_at(device, layout, &at, res)
}

/// A group-2 bind group of `res` at the binding numbers `at`.
fn bind_io_at(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    at: &[u32],
    res: &[Res<'_>],
) -> wgpu::BindGroup {
    let views: Vec<Option<wgpu::TextureView>> = res
        .iter()
        .map(|r| match r {
            Res::Tex(t) => Some(view(t)),
            Res::Buf(_) => None,
        })
        .collect();
    let entries: Vec<wgpu::BindGroupEntry<'_>> = res
        .iter()
        .zip(&views)
        .zip(at)
        .map(|((r, v), &binding)| wgpu::BindGroupEntry {
            binding,
            resource: match (r, v) {
                (Res::Buf(b), _) => b.as_entire_binding(),
                (_, Some(v)) => wgpu::BindingResource::TextureView(v),
                (Res::Tex(_), None) => unreachable!("a texture binding has its view"),
            },
        })
        .collect();
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout,
        entries: &entries,
    })
}

impl Frame {
    /// A frame drawing into a `width` × `height` target, presented at that size.
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        Self::with_sizes(device, [width, height], [width, height])
    }

    /// A frame rendering at `internal` pixels for an image presented at `output` pixels.
    pub fn with_sizes(device: &wgpu::Device, internal: [u32; 2], output: [u32; 2]) -> Self {
        let [width, height] = internal;
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
        let f16 = Io::Store(wgpu::TextureFormat::Rgba16Float);
        let pass = |label: &str, source: &str, entry: &str, io: &[Io]| {
            let layout = io_layout(device, label, io);
            let pipeline = compute_pipeline(
                device,
                label,
                format!("{MARCH_WGSL}\n{source}"),
                &[Some(&world_layout), None, Some(&layout)],
                entry,
            );
            (layout, pipeline)
        };
        let (light_layout, light) = pass(
            "light",
            LIGHT_WGSL,
            "shade",
            &[
                Io::Float,
                Io::Uint,
                f16,
                Io::Float,
                Io::Uint,
                Io::Uint,
                Io::Store(wgpu::TextureFormat::Rgba32Uint),
            ],
        );
        let (temporal_layout, temporal) = pass(
            "temporal",
            TEMPORAL_WGSL,
            "temporal",
            &[
                Io::Float,
                Io::Uint,
                Io::Float,
                Io::Uint,
                Io::Float,
                Io::Float,
                Io::Float,
                f16,
                f16,
                Io::Uniform,
                Io::Float,
            ],
        );
        let gradient_layout = io_layout(
            device,
            "gradient",
            &[Io::Float, Io::Uint, Io::Float, Io::Uint, f16],
        );
        let gradient = compute_pipeline(
            device,
            "gradient",
            format!("{MARCH_WGSL}\n{LIGHT_WGSL}\n{GRADIENT_WGSL}"),
            &[Some(&world_layout), None, None, Some(&gradient_layout)],
            "gradient",
        );
        let downsample_layout = io_layout(
            device,
            "downsample",
            &[
                Io::Float,
                Io::Uint,
                Io::Store(wgpu::TextureFormat::Rgba32Float),
                Io::Store(wgpu::TextureFormat::Rgba16Uint),
            ],
        );
        let downsample = compute_pipeline(
            device,
            "downsample",
            format!("{MARCH_WGSL}\n{COMPOSITE_WGSL}"),
            &[Some(&world_layout), None, None, Some(&downsample_layout)],
            "downsample",
        );
        let (variance_layout, variance) = pass(
            "variance",
            ATROUS_WGSL,
            "variance",
            &[Io::Float, Io::Uint, Io::Float, Io::Float, f16],
        );
        let atrous_layout = io_layout_at(
            device,
            "atrous",
            &[
                (0, Io::Float),
                (1, Io::Uint),
                (2, Io::Float),
                (5, f16),
                (6, Io::Uniform),
            ],
        );
        let atrous = compute_pipeline(
            device,
            "atrous",
            format!("{MARCH_WGSL}\n{ATROUS_WGSL}"),
            &[Some(&world_layout), None, Some(&atrous_layout)],
            "atrous",
        );
        let steps = [0, 1, 2, 3].map(|i| {
            wgpu::util::DeviceExt::create_buffer_init(
                device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("atrous step"),
                    contents: bytemuck::cast_slice(&[1u32 << i, 0, 0, 0]),
                    usage: wgpu::BufferUsages::UNIFORM,
                },
            )
        });
        let (upscale_layout, upscale) = pass(
            "upscale",
            UPSCALE_WGSL,
            "upscale",
            &[
                Io::Float,
                Io::Uint,
                Io::Float,
                Io::Float,
                Io::Uint,
                Io::Float,
                f16,
                Io::Uniform,
            ],
        );
        let resolve_layout = io_layout(
            device,
            "resolve",
            &[Io::Float, Io::Store(wgpu::TextureFormat::Rgba8Unorm)],
        );
        let resolve = compute_pipeline(
            device,
            "resolve",
            format!("{MARCH_WGSL}\n{UPSCALE_WGSL}"),
            &[Some(&world_layout), None, None, Some(&resolve_layout)],
            "resolve",
        );
        let (composite_layout, composite) = pass(
            "composite",
            COMPOSITE_WGSL,
            "composite",
            &[
                Io::Float,
                Io::Uint,
                Io::Float,
                Io::Float,
                Io::Store(wgpu::TextureFormat::Rgba8Unorm),
                Io::Float,
                Io::Uint,
            ],
        );
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame params"),
            size: size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let edits = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("edit boxes"),
            size: size_of::<EditsUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            world_layout,
            hit,
            light_layout,
            light,
            temporal_layout,
            temporal,
            gradient_layout,
            gradient,
            variance_layout,
            variance,
            atrous_layout,
            atrous,
            steps,
            composite_layout,
            composite,
            params,
            edits,
            targets: Targets::new(
                device,
                width,
                height,
                LightingResolution::default().scale(),
                (internal != output).then_some(output),
            ),
            resolution: LightingResolution::default(),
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
            emission_changed_at: None,
            cur: 0,
            downsample_layout,
            downsample,
            upscale_layout,
            upscale,
            resolve_layout,
            resolve,
            history: std::sync::Mutex::new(History::default()),
            output,
            jitter: false,
        }
    }

    /// The size the hit pass and the lighting passes render at.
    pub fn internal_size(&self) -> [u32; 2] {
        [self.targets.width, self.targets.height]
    }

    /// The size the frame's image is presented at.
    pub fn output_size(&self) -> [u32; 2] {
        self.output
    }

    /// Jitters every view's rays by [`jitter_at`] of the frame index (off by default: each
    /// ray through its pixel's centre). The upscaler runs when the frame jitters or the
    /// internal size differs from the output size, so turning the jitter on or off at equal
    /// sizes replaces the targets and drops the history.
    pub fn set_jitter(&mut self, device: &wgpu::Device, on: bool) {
        if on != self.jitter {
            self.jitter = on;
            self.set_sizes(device, self.internal_size(), self.output);
        }
    }

    /// Whether the upscale pass runs: the frame jitters or renders below its output size.
    /// Without it the composite's image is the frame's, byte for byte.
    pub fn upscaling(&self) -> bool {
        self.jitter || self.internal_size() != self.output
    }

    pub fn jitter(&self) -> bool {
        self.jitter
    }

    /// The frame index the next `params` seeds its random numbers with.
    pub fn frame_index(&self) -> u32 {
        self.frame
    }

    pub fn set_frame_index(&mut self, frame: u32) {
        self.frame = frame;
    }

    fn history(&mut self) -> &mut History {
        self.history
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Ends the frame last encoded: the next frame index, its hit buffer and history become
    /// the previous frame's, and the double-buffered targets swap. Call once per frame, after
    /// [`Frame::encode`].
    pub fn advance(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        let h = self.history();
        h.prev_views = h.last_views;
        h.prev_offset = h.last_offset;
        h.valid = h.encoded;
        h.encoded = false;
        self.cur ^= 1;
    }

    /// Kills the history of every pixel whose hit lies in the voxels `min ..= max` dilated by
    /// one voxel, in the next encoded frame: an edit's `EditResult::aabb`. More than
    /// [`MAX_EDITS`] boxes in one frame flush all history.
    pub fn invalidate(&mut self, min: [i32; 3], max: [i32; 3]) {
        let h = self.history();
        if h.edits.len() == MAX_EDITS {
            h.overflow = true;
        } else {
            h.edits.push([min, max]);
        }
    }

    /// Drops all history: the next encoded frame starts every pixel afresh.
    pub fn reset_history(&mut self) {
        self.history().valid = false;
    }

    /// Encodes and submits one frame of `camera` over `world` with history, after advancing
    /// from the frame before: what a frame loop does, for tests and tools.
    pub fn step(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        world: &GpuWorld,
        camera: &Camera,
        light: &LightParams,
    ) {
        self.advance();
        let params = self.params(world, camera, light);
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        self.encode(device, queue, &mut enc, world, &params, None);
        queue.submit([enc.finish()]);
    }

    /// Scales material `material`'s emission by `scale` from the next frame on: a visual
    /// knob that leaves the world untouched (an authoritative change is a `Replace` op).
    pub fn set_emission_scale(&mut self, queue: &wgpu::Queue, material: u16, scale: f32) {
        let i = usize::from(material);
        if self.emission_cpu[i] != scale {
            self.emission_cpu[i] = scale;
            self.emission_changed_at = Some(self.frame);
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

    /// Sets the surface look: normal mode, detail and view. A change of view drops the
    /// upscaler's history (not the lighting's).
    pub fn set_look(&mut self, look: LookParams) {
        if look.view != self.look.view {
            self.history().view_changed = true;
        }
        self.look = look;
    }

    pub fn look(&self) -> LookParams {
        self.look
    }

    /// Sets exposure, tone map, lamp, candidates and shadow walk.
    pub fn set_lighting(&mut self, lighting: Lighting) {
        self.lighting = lighting;
    }

    pub fn lighting(&self) -> Lighting {
        self.lighting
    }

    /// The frame's `rgba8unorm` image: the composite's output, or with the upscaler (see
    /// [`Frame::upscaling`]) the upscaled image at the output size. With the upscaler the
    /// composite writes the internal image into its top-left pixels and the upscale pass's
    /// resolve overwrites the whole of it.
    pub fn target(&self) -> &wgpu::Texture {
        &self.targets.output
    }

    /// The upscaler's history the next or last encode writes (rgba16float at the output size:
    /// the upscaled image, display-encoded rgb), or `None` without the upscaler.
    pub fn upscale_history(&self) -> Option<&wgpu::Texture> {
        self.targets.history.as_ref().map(|h| &h[self.cur])
    }

    /// The hit buffer's position texture the next or last encode writes (rgba32float: world
    /// position in voxels, then `t`, `3e38` on a miss).
    pub fn hit_positions(&self) -> &wgpu::Texture {
        &self.targets.hit_pos[self.cur]
    }

    /// The hit buffer's record texture (rgba16uint: octahedral normal u and v as snorm16
    /// bits, material id, flags: bit 0 hit, bit 1 contour-plane hit, bit 2 translucent layers
    /// in front, bits 3..5 the entry face's axis plus one, 0 for none).
    pub fn hit_records(&self) -> &wgpu::Texture {
        &self.targets.hit_info[self.cur]
    }

    /// The light pass's irradiance (rgba16float: rgb, then the emissive part's luminance).
    pub fn irradiance(&self) -> &wgpu::Texture {
        &self.targets.irradiance
    }

    /// The light pass's reservoirs the next or last encode writes (rgba32uint: the kept
    /// point, then W, M, the face and a valid bit; see `light.wgsl`).
    pub fn reservoirs(&self) -> &wgpu::Texture {
        &self.targets.reservoir[self.cur]
    }

    /// The temporal pass's accumulated irradiance (rgba16float, rgb).
    pub fn accumulated(&self) -> &wgpu::Texture {
        &self.targets.accum[self.cur]
    }

    /// The filter's output, the irradiance the composite shades with when the filter runs
    /// (rgba16float: irradiance rgb, variance a).
    pub fn filtered(&self) -> &wgpu::Texture {
        &self.targets.filter[ATROUS_ITERATIONS % 2]
    }

    /// The temporal pass's moments (rgba16float: luminance, its square, the age in frames).
    pub fn moments(&self) -> &wgpu::Texture {
        &self.targets.moments[self.cur]
    }

    /// Bytes the frame's targets occupy.
    pub fn frame_bytes(&self) -> u64 {
        self.targets.bytes()
    }

    /// Replaces the targets with ones of `width` × `height`, presented at that size, and
    /// drops the history; the pipelines are kept.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        self.set_sizes(device, [width, height], [width, height]);
    }

    /// Replaces the targets with ones of `internal` pixels for an image presented at
    /// `output` pixels and drops the history; the pipelines are kept.
    pub fn set_sizes(&mut self, device: &wgpu::Device, internal: [u32; 2], output: [u32; 2]) {
        self.output = output;
        let upscaled = (self.jitter || internal != output).then_some(output);
        self.targets = Targets::new(
            device,
            internal[0],
            internal[1],
            self.resolution.scale(),
            upscaled,
        );
        self.reset_history();
    }

    /// Runs the lighting passes at `resolution`; the lighting targets are replaced and the
    /// history dropped when it changes.
    pub fn set_resolution(&mut self, device: &wgpu::Device, resolution: LightingResolution) {
        if resolution != self.resolution {
            self.resolution = resolution;
            self.set_sizes(device, self.internal_size(), self.output);
        }
    }

    pub fn resolution(&self) -> LightingResolution {
        self.resolution
    }

    /// The uniform block for `camera` over `world`, lit by `light`: one view filling the
    /// target.
    pub fn params(&self, world: &GpuWorld, camera: &Camera, light: &LightParams) -> Params {
        let rect = [0, 0, self.targets.width, self.targets.height];
        self.params_for(world, &[View::of_camera(camera, rect)], camera, light)
    }

    /// The uniform block for `views` (at most [`MAX_VIEWS`]; a pixel outside every rectangle
    /// takes the first) over `world`, lit by `light` placed relative to `camera`. Each view's
    /// jitter is the frame's ([`Frame::set_jitter`]). [`Frame::encode`] fills in the history
    /// fields.
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
        let jitter = if self.jitter {
            jitter_at(self.frame)
        } else {
            [0.0; 2]
        };
        for v in &mut vs[..views.len()] {
            v.jitter = jitter;
        }
        let e = world.window();
        Params {
            views: vs,
            view_count: views.len() as u32,
            translucent_layers: self.look.translucent_layers,
            window_min: e.min.0,
            window_size: e.size(),
            light_pos: (camera.position + right * light.offset_right - up * light.offset_down)
                .to_array(),
            light_intensity: light.intensity,
            light_dir: (fwd * light.turn.cos() + right * light.turn.sin()).to_array(),
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
            emissive_history: self.lighting.emissive_history as f32,
            output_size: if self.upscaling() {
                self.output
            } else {
                [self.targets.width, self.targets.height]
            },
            lighting: self.lighting.flags()
                | if self
                    .emission_changed_at
                    .is_some_and(|f| self.frame.wrapping_sub(f) <= EMISSION_CHANGE_FRAMES)
                {
                    LIGHTING_EMISSION_CHANGED
                } else {
                    0
                },
            light_table_base: world.light_table_base(),
            light_span: world.light_span(),
            candidates: self.lighting.candidates,
            frame: self.frame,
            light_size: self.targets.light_size(),
            light_scale: self.targets.scale,
            light_offset: if self.targets.scale > 1 {
                let [x, y] = HALF_OFFSET;
                x | y << 8
            } else {
                0
            },
            ..Params::default()
        }
    }

    /// Records the frame's passes over `world` into `encoder`, with history: the temporal
    /// pass reads the previous frame's hit buffer and history unless they are invalid, and
    /// takes the edit boxes collected since the last encode. With `timer` (of at least
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
        self.encode_with(device, queue, encoder, world, params, timer, true);
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "encode's arguments plus whether the frame keeps history"
    )]
    fn encode_with(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        world: &GpuWorld,
        params: &Params,
        timer: Option<&PassTimer>,
        keep_history: bool,
    ) {
        let mut p = *params;
        let mut edits = EditsUniform {
            count: 0,
            _pad: [0; 3],
            boxes: [[0; 4]; 2 * MAX_EDITS],
        };
        if keep_history {
            let mut h = self
                .history
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            p.prev_views = h.prev_views;
            p.prev_light_offset = h.prev_offset;
            h.last_offset = params.light_offset;
            p.history = u32::from(h.valid && !h.overflow);
            // Bit 1: the upscaler's history holds this view's image.
            p.history |= u32::from(h.valid && !h.overflow && !h.view_changed) << 1;
            h.view_changed = false;
            for (i, [min, max]) in h.edits.iter().enumerate() {
                edits.boxes[2 * i] = [min[0], min[1], min[2], 0];
                edits.boxes[2 * i + 1] = [max[0], max[1], max[2], 0];
            }
            edits.count = h.edits.len() as u32;
            h.edits.clear();
            h.overflow = false;
            h.last_views = params.views;
            h.encoded = true;
        } else {
            p.history = 0;
        }
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&p));
        queue.write_buffer(&self.edits, 0, bytemuck::bytes_of(&edits));
        let table = light_table(world, p.views[0].origin);
        let used = (1 + table.count as usize) * 16;
        queue.write_buffer(
            world.light_buffer(),
            u64::from(world.light_table_base()) * crate::gpu_world::LIGHT_SAMPLE_BYTES,
            &bytemuck::bytes_of(&table)[..used],
        );
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
        let (c, q) = (self.cur, self.cur ^ 1);
        let hit_bind = bind_io(
            device,
            &self.hit.out_layout,
            &[
                Res::Tex(&t.hit_pos[c]),
                Res::Tex(&t.hit_info[c]),
                Res::Tex(&t.layer),
            ],
        );
        let half = t.half_pos.is_some();
        let downsample_bind = bind_io(
            device,
            &self.downsample_layout,
            &[
                Res::Tex(&t.hit_pos[c]),
                Res::Tex(&t.hit_info[c]),
                Res::Tex(t.lit_pos(c)),
                Res::Tex(t.lit_info(c)),
            ],
        );
        let light_bind = bind_io(
            device,
            &self.light_layout,
            &[
                Res::Tex(t.lit_pos(c)),
                Res::Tex(t.lit_info(c)),
                Res::Tex(&t.irradiance),
                Res::Tex(t.lit_pos(q)),
                Res::Tex(t.lit_info(q)),
                Res::Tex(&t.reservoir[q]),
                Res::Tex(&t.reservoir[c]),
            ],
        );
        let temporal_bind = bind_io(
            device,
            &self.temporal_layout,
            &[
                Res::Tex(t.lit_pos(c)),
                Res::Tex(t.lit_info(c)),
                Res::Tex(t.lit_pos(q)),
                Res::Tex(t.lit_info(q)),
                Res::Tex(&t.irradiance),
                Res::Tex(&t.accum[q]),
                Res::Tex(&t.moments[q]),
                Res::Tex(&t.accum[c]),
                Res::Tex(&t.moments[c]),
                Res::Buf(&self.edits),
                Res::Tex(&t.strata),
            ],
        );
        // The filter runs in the shaded view unless switched off; the variance pass writes
        // `filter[0]` and the iterations ping-pong, ending in `filter[0]`.
        let filtering = self.lighting.filter && self.look.view == ViewMode::Shaded;
        let variance_bind = bind_io(
            device,
            &self.variance_layout,
            &[
                Res::Tex(t.lit_pos(c)),
                Res::Tex(t.lit_info(c)),
                Res::Tex(&t.accum[c]),
                Res::Tex(&t.moments[c]),
                Res::Tex(&t.filter[0]),
            ],
        );
        let atrous_binds: Vec<wgpu::BindGroup> = (0..ATROUS_ITERATIONS)
            .map(|i| {
                bind_io_at(
                    device,
                    &self.atrous_layout,
                    &[0, 1, 2, 5, 6],
                    &[
                        Res::Tex(t.lit_pos(c)),
                        Res::Tex(t.lit_info(c)),
                        Res::Tex(&t.filter[i % 2]),
                        Res::Tex(&t.filter[(i + 1) % 2]),
                        Res::Buf(&self.steps[i]),
                    ],
                )
            })
            .collect();
        let lit = if filtering {
            &t.filter[ATROUS_ITERATIONS % 2]
        } else {
            &t.irradiance
        };
        let composite_bind = bind_io(
            device,
            &self.composite_layout,
            &[
                Res::Tex(&t.hit_pos[c]),
                Res::Tex(&t.hit_info[c]),
                Res::Tex(&t.layer),
                Res::Tex(lit),
                Res::Tex(&t.output),
                Res::Tex(t.lit_pos(c)),
                Res::Tex(t.lit_info(c)),
            ],
        );
        let gradient_bind = bind_io(
            device,
            &self.gradient_layout,
            &[
                Res::Tex(t.lit_pos(q)),
                Res::Tex(t.lit_info(q)),
                Res::Tex(&t.irradiance),
                Res::Tex(&t.reservoir[q]),
                Res::Tex(&t.strata),
            ],
        );
        let upscale_bind = t.history.as_ref().map(|hist| {
            bind_io(
                device,
                &self.upscale_layout,
                &[
                    Res::Tex(&t.hit_pos[c]),
                    Res::Tex(&t.hit_info[c]),
                    Res::Tex(&t.output),
                    Res::Tex(&t.hit_pos[q]),
                    Res::Tex(&t.hit_info[q]),
                    Res::Tex(&hist[q]),
                    Res::Tex(&hist[c]),
                    Res::Buf(&self.edits),
                ],
            )
        });
        let resolve_bind = t.history.as_ref().map(|hist| {
            bind_io(
                device,
                &self.resolve_layout,
                &[Res::Tex(&hist[c]), Res::Tex(&t.output)],
            )
        });
        let gradient_on =
            self.lighting.history == HistoryKill::Gradient && self.look.view == ViewMode::Shaded;
        let full = [t.width.div_ceil(WORKGROUP), t.height.div_ceil(WORKGROUP)];
        let lw = t.light_size().map(|n| n.div_ceil(WORKGROUP));
        let strata = [t.strata.width(), t.strata.height()].map(|n| n.div_ceil(WORKGROUP));
        // Each pass's dispatches: pipeline, group-2 or -3 bind group and its index, workgroups.
        type Dispatch<'a> = (
            &'a wgpu::ComputePipeline,
            &'a wgpu::BindGroup,
            u32,
            [u32; 2],
        );
        let mut hit: Vec<Dispatch<'_>> = vec![(&self.hit.pipeline, &hit_bind, 1, full)];
        if half {
            hit.push((&self.downsample, &downsample_bind, 3, lw));
        }
        let mut gradient: Vec<Dispatch<'_>> = Vec::new();
        if gradient_on {
            gradient.push((&self.gradient, &gradient_bind, 3, strata));
        }
        let mut atrous: Vec<Dispatch<'_>> = Vec::new();
        if filtering {
            atrous.push((&self.variance, &variance_bind, 2, lw));
            atrous.extend(atrous_binds.iter().map(|b| (&self.atrous, b, 2, lw)));
        }
        let mut upscale: Vec<Dispatch<'_>> = Vec::new();
        if let (Some(u), Some(r)) = (&upscale_bind, &resolve_bind) {
            let out = self.output.map(|n| n.div_ceil(WORKGROUP));
            upscale.push((&self.upscale, u, 2, out));
            upscale.push((&self.resolve, r, 3, out));
        }
        let passes: [Vec<Dispatch<'_>>; 7] = [
            hit,
            gradient,
            vec![(&self.light, &light_bind, 2, lw)],
            vec![(&self.temporal, &temporal_bind, 2, lw)],
            atrous,
            vec![(&self.composite, &composite_bind, 2, full)],
            upscale,
        ];
        for (i, dispatches) in passes.into_iter().enumerate() {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(FRAME_PASSES[i]),
                timestamp_writes: timer.map(|t| t.writes_at(i as u32)),
            });
            for (pipeline, bind, group, groups) in dispatches {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &world_bind, &[]);
                pass.set_bind_group(group, bind, &[]);
                pass.dispatch_workgroups(groups[0], groups[1], 1);
            }
        }
    }

    /// Renders `camera` over `world` lit by `light` as one frame alone, with no history,
    /// and submits the work; the frame loop's history is left as it was.
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

    /// Renders the frame `params` describes over `world` alone, with no history, and submits
    /// the work.
    pub fn submit(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        world: &GpuWorld,
        params: &Params,
    ) {
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        self.encode_with(device, queue, &mut enc, world, params, None, false);
        queue.submit([enc.finish()]);
    }
}

fn buffer_entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

/// Passes of a frame in encode order, as [`Frame::encode`] times them: pass `i` writes
/// timestamps `2 i` and `2 i + 1` of the timer it is given.
pub const FRAME_PASSES: [&str; 7] = [
    "hit",
    "gradient",
    "light",
    "temporal",
    "atrous",
    "composite",
    "upscale",
];

/// The gradient pass's stratum edge in pixels; `STRATUM` in `gradient.wgsl`.
pub const GRADIENT_STRATUM: u32 = 3;

/// The frame's targets: the hit buffer, the translucent layers and the output at the internal
/// resolution, and the lighting passes' targets on the lighting grid, the same at full
/// resolution and half of it on each edge at half resolution (with its own hit buffer, sampled
/// from the full one). The hit buffers and the history are kept twice, indexed by
/// `Frame::cur`.
struct Targets {
    width: u32,
    height: u32,
    /// Full-resolution pixels per lighting pixel on an edge: 1 or 2.
    scale: u32,
    /// The hit buffer: position + t (rgba32float), and octahedral normal, material and flags
    /// (rgba16uint).
    hit_pos: [wgpu::Texture; 2],
    hit_info: [wgpu::Texture; 2],
    /// The lighting grid's hit buffer at half resolution.
    half_pos: Option<[wgpu::Texture; 2]>,
    half_info: Option<[wgpu::Texture; 2]>,
    /// The translucent layers in front of each hit (rgba16float), or the steps view's
    /// counters.
    layer: wgpu::Texture,
    /// The light pass's irradiance (rgba16float), on the lighting grid.
    irradiance: wgpu::Texture,
    /// The temporal pass's accumulated irradiance and moments (rgba16float).
    accum: [wgpu::Texture; 2],
    moments: [wgpu::Texture; 2],
    /// The filter's ping-pong pair (rgba16float: irradiance rgb, variance a).
    filter: [wgpu::Texture; 2],
    /// The light pass's reservoirs (rgba32uint), this frame's and the last.
    reservoir: [wgpu::Texture; 2],
    /// The gradient pass's strata, one texel per 3 x 3 lighting pixels (rgba16float).
    strata: wgpu::Texture,
    /// The frame's display-encoded image (rgba8unorm): the composite's output at the
    /// internal size, or with the upscaler at the output size, the composite's image in its
    /// top-left pixels until the resolve overwrites it.
    output: wgpu::Texture,
    /// The upscaler's history at the output size (rgba16float), this frame's and the last.
    history: Option<[wgpu::Texture; 2]>,
}

impl Targets {
    /// The targets at the internal size `width` x `height`, with the upscaler's at
    /// `upscaled` when it runs.
    fn new(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        scale: u32,
        upscaled: Option<[u32; 2]>,
    ) -> Self {
        let sized = |label, format, width, height| {
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
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let (lw, lh) = (width.div_ceil(scale), height.div_ceil(scale));
        let tex = |label, format| sized(label, format, width, height);
        let lit = |label, format| sized(label, format, lw, lh);
        use wgpu::TextureFormat::{Rgba8Unorm, Rgba16Float, Rgba16Uint, Rgba32Float, Rgba32Uint};
        let half = scale > 1;
        Self {
            width,
            height,
            scale,
            hit_pos: [0, 1].map(|_| tex("hit pos", Rgba32Float)),
            hit_info: [0, 1].map(|_| tex("hit info", Rgba16Uint)),
            half_pos: half.then(|| [0, 1].map(|_| lit("half hit pos", Rgba32Float))),
            half_info: half.then(|| [0, 1].map(|_| lit("half hit info", Rgba16Uint))),
            layer: tex("layer", Rgba16Float),
            irradiance: lit("irradiance", Rgba16Float),
            accum: [0, 1].map(|_| lit("accumulated irradiance", Rgba16Float)),
            moments: [0, 1].map(|_| lit("moments", Rgba16Float)),
            filter: [0, 1].map(|_| lit("filter", Rgba16Float)),
            reservoir: [0, 1].map(|_| lit("reservoir", Rgba32Uint)),
            strata: sized(
                "gradient strata",
                Rgba16Float,
                lw.div_ceil(GRADIENT_STRATUM),
                lh.div_ceil(GRADIENT_STRATUM),
            ),
            output: match upscaled {
                Some([w, h]) => sized("frame target", Rgba8Unorm, w, h),
                None => tex("frame target", Rgba8Unorm),
            },
            history: upscaled
                .map(|[w, h]| [0, 1].map(|_| sized("upscale history", Rgba16Float, w, h))),
        }
    }

    /// The lighting grid's size.
    fn light_size(&self) -> [u32; 2] {
        [self.irradiance.width(), self.irradiance.height()]
    }

    /// The lighting grid's hit positions and records, copy `i`.
    fn lit_pos(&self, i: usize) -> &wgpu::Texture {
        self.half_pos.as_ref().map_or(&self.hit_pos[i], |h| &h[i])
    }

    fn lit_info(&self, i: usize) -> &wgpu::Texture {
        self.half_info.as_ref().map_or(&self.hit_info[i], |h| &h[i])
    }

    /// Bytes the targets occupy.
    fn bytes(&self) -> u64 {
        let px = u64::from(self.width) * u64::from(self.height);
        let [lw, lh] = self.light_size();
        let lpx = u64::from(lw) * u64::from(lh);
        let half = if self.scale > 1 { 2 * (16 + 8) } else { 0 };
        let strata = u64::from(self.strata.width()) * u64::from(self.strata.height());
        let output = u64::from(self.output.width()) * u64::from(self.output.height());
        let history = self.history.as_ref().map_or(0, |h| {
            2 * 8 * u64::from(h[0].width()) * u64::from(h[0].height())
        });
        px * (2 * (16 + 8) + 8)
            + output * 4
            + history
            + lpx * (half + 8 + 2 * 8 + 2 * 8 + 2 * 8 + 2 * 16)
            + strata * 8
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

#[cfg(test)]
mod tests {
    use super::{JITTER_PHASES, RenderScale, jitter_at};

    /// The jitter's sixteen phases are distinct points of `(-0.5, 0.5)^2` that repeat with
    /// period 16 and average to within 1/16 of zero on each axis (-0.029 and -0.037).
    #[test]
    fn the_jitter_cycles_sixteen_distinct_centred_offsets() {
        let phases: Vec<[f32; 2]> = (0..JITTER_PHASES).map(jitter_at).collect();
        for (i, p) in phases.iter().enumerate() {
            assert!(p.iter().all(|c| c.abs() < 0.5), "phase {i}: {p:?}");
            assert_eq!(jitter_at(i as u32 + JITTER_PHASES), *p);
            assert!(phases[..i].iter().all(|q| q != p), "phase {i} repeats");
        }
        assert_eq!(phases[0], [0.0, 1.0 / 3.0 - 0.5]);
        for k in 0..2 {
            let mean = phases.iter().map(|p| p[k]).sum::<f32>() / JITTER_PHASES as f32;
            assert!(mean.abs() <= 1.0 / 16.0, "axis {k}: mean {mean}");
        }
    }

    /// The upscale pass and its resolve parse and validate headless, compiled after the hit
    /// pass.
    #[test]
    fn upscale_wgsl_validates_through_naga() {
        let source = format!("{}\n{}", crate::MARCH_WGSL, super::UPSCALE_WGSL);
        let module = wgpu::naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("upscale.wgsl parses: {}", e.emit_to_string(&source)));
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::default(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("upscale.wgsl validates: {e:?}"));
    }

    #[test]
    fn the_internal_size_rounds_the_output_over_the_scale() {
        assert_eq!(RenderScale::One.internal([2530, 1378]), [2530, 1378]);
        assert_eq!(RenderScale::OneHalf.internal([2530, 1378]), [1687, 919]);
        assert_eq!(RenderScale::Two.internal([385, 1]), [193, 1]);
        for s in RenderScale::ALL {
            assert_eq!(RenderScale::from_name(s.name()), Some(s));
        }
    }
}
