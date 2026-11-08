//! The frame: the hit pass ([`Marcher`], `march.wgsl`), the light pass (`light.wgsl`), the
//! probe volume's passes (`probe_gather.wgsl`, ADR-0023) and the composite (`composite.wgsl`),
//! each a compute dispatch over the internal target (ADR-0018).

use crate::camera::{Camera, View};
use crate::exposure::{
    EX_AUTO, EX_METER, EX_RESET, Exposure, ExposurePass, ExposureReadout, ExposureUniform,
    METER_TILE, Metering,
};
use crate::fsr2::{Fsr2, UpscalerKind};
use crate::gpu_world::GpuWorld;
use crate::light::LightParams;
use crate::probes::{DEFAULT_PROBE_SPACING, ProbeUniform, ProbeVolume};
use crate::{
    ATROUS_WGSL, COMPOSITE_WGSL, GRADIENT_WGSL, LIGHT_WGSL, MARCH_WGSL, PROBE_GATHER_WGSL,
    PROBE_UPDATE_WGSL, TEMPORAL_WGSL,
};
use vxe_world::ChunkCoord;

/// The upscale pass, compiled after [`MARCH_WGSL`].
pub const UPSCALE_WGSL: &str = include_str!("shaders/upscale.wgsl");
/// The sharpening pass (RCAS), compiled after [`MARCH_WGSL`].
pub const SHARPEN_WGSL: &str = include_str!("shaders/sharpen.wgsl");
const CACHE_CLEAR_WGSL: &str = include_str!("shaders/cache_clear.wgsl");
const CACHE_CELL_WGSL: &str = include_str!("shaders/cache_cell.wgsl");
const CACHE_SWEEP_WGSL: &str = include_str!("shaders/cache_sweep.wgsl");
const CACHE_BLEND_WGSL: &str = include_str!("shaders/cache_blend.wgsl");
const CACHE_FILL_WGSL: &str = include_str!("shaders/cache_fill.wgsl");

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
    /// [`SurfaceTuning::face_mix`].
    pub face_mix: f32,
    pub window_size: [i32; 3],
    pub _pad3: i32,
    pub light_pos: [f32; 3],
    pub light_intensity: f32,
    pub light_dir: [f32; 3],
    pub cos_outer: f32,
    pub size: [u32; 2],
    pub cos_inner: f32,
    pub flags: u32,
    /// Pool units per shard: unit `u` is unit `u % n` of shard `u / n`.
    pub units_per_shard: u32,
    /// The mip level every LOD-only chunk is walked at, 1 to 3; 0 picks it by distance.
    pub forced_level: u32,
    /// A LOD-only chunk within this squared chunk distance of the first view's chunk is
    /// walked at level 1, within `lod2_r2` at level 2, and farther at level 3.
    pub lod1_r2: i32,
    pub lod2_r2: i32,
    /// The manual exposure ([`Exposure::Manual`]); 1.0 and unread in auto, where the exposure
    /// state's `current` is the frame's.
    pub exposure: f32,
    /// The emissive table as the shaders read it: bit 0 the temporal pass reads it
    /// ([`Lighting::table_read`]), bits 14..28 [`Lighting::table_cap`].
    pub cache: u32,
    /// Emissive candidates per pixel ([`Lighting::candidates`]).
    pub candidates: u32,
    /// The slot of the light-sample buffer where the frame's candidate table starts.
    pub light_table_base: u32,
    /// Bit 0: the previous frame's hit buffer and history are valid (set by
    /// [`Frame::encode`]).
    pub history: u32,
    /// The lamp's `offset_right`, `offset_down` and `turn` ([`LightParams`]): what
    /// [`Frame::encode`] compares between encodes, with the cone and intensity, to decide
    /// [`LIGHTING_LAMP_CHANGED`]. The shaders read these words as padding.
    pub lamp_pose: [f32; 3],
    /// The previous frame's views, for reprojection (set by [`Frame::encode`]).
    pub prev_views: [View; MAX_VIEWS],
    /// The lighting grid's size, full-resolution pixels per lighting pixel on an edge, and
    /// this and last frame's sampled pixel of each 2 x 2 (x in bits 0..8, y in 8..16; the
    /// last set by [`Frame::encode`]).
    pub light_size: [u32; 2],
    pub light_scale: u32,
    pub light_offset: u32,
    pub prev_light_offset: u32,
    /// [`Lighting::table_cells`] less one: a slot index's mask.
    pub cache_mask: u32,
    /// The size of the frame's image: the output size with the upscaler, else `size`.
    pub output_size: [u32; 2],
    /// The reopened fill ([`Lighting::fill_samples`] in bits 0..8, [`Lighting::fill_until`] in
    /// bits 8..21).
    pub fill: u32,
    /// [`LookParams::far_normal`].
    pub far_normal: [f32; 2],
    pub _pad9: u32,
    /// [`Fsr2Settings::emitter_cap`] while FSR 2 runs unbypassed; 0 is no cap.
    pub emitter_cap: f32,
    /// 1: [`Fsr2Settings::dither`] while FSR 2 runs unbypassed.
    pub dither: u32,
    /// Pads the block to a multiple of 16 bytes, as the shaders' struct is.
    pub _tail: [u32; 2],
}

const _: () = assert!(size_of::<Params>().is_multiple_of(16));

/// `Params::lighting` bit: the headlamp lights the frame.
pub const LIGHTING_LAMP: u32 = 1;
/// `Params::lighting` bit: shadow rays take the coarse walk ([`ShadowWalk::Coarse`]).
pub const LIGHTING_COARSE_SHADOW: u32 = 1 << 1;
/// `Params::lighting` bit: the temporal pass also shortens history by A-SVGF's gradient
/// ([`History::Gradient`]).
pub const LIGHTING_GRADIENT: u32 = 1 << 3;
/// [`Lighting::probe_freeze`] by default: where a running mean's noise, `1 / n`, roughly
/// equals hysteresis 0.97's steady noise, `0.03 / 1.97`.
pub const DEFAULT_PROBE_FREEZE: u32 = 64;
/// Frames after an emission-scale change (the frame of the change too) during which the
/// emissive part blends at no less
/// than the lamp's 0.2 (`0.8^11 = 0.086`: a step settles within 10 % in them).
pub const EMISSION_CHANGE_FRAMES: u32 = 11;
/// `Params::lighting` bit: an emission scale changed within the last
/// [`EMISSION_CHANGE_FRAMES`] frames.
pub const LIGHTING_EMISSION_CHANGED: u32 = 1 << 8;
/// `Params::lighting` bit: the lamp's light changed since the last encode with history (its
/// switch, a parameter, or the walk its shadow rays take): the temporal pass takes this
/// frame's lamp part as it is, the way an edit box resets a pixel (ADR-0018), and keeps the
/// emissive part's history. The light pass's lamp estimate carries no sampling noise (one
/// point light, one shadow ray), so the reset adds none.
pub const LIGHTING_LAMP_CHANGED: u32 = 1 << 9;
/// `Params::lighting` bit: the table's guard runs (`guard_cell` in `gradient.wgsl`): the
/// frame lies within [`GUARD_WINDOW`] frames of a change to the world's emissive light.
pub const LIGHTING_GUARD: u32 = 1 << 10;
/// Frames after a change to the world's emissive light during which the table's guard
/// re-shades kept emitters and re-verifies dark capped cells, and may reopen them; outside it
/// no cell's light can have changed, so the guard does nothing. The changes that open it: an
/// edit box ([`Frame::invalidate`], or more than [`MAX_EDITS`] of them), any write to the
/// mirror (`GpuWorld::bytes_written` moved since the last encode: a chunk streamed in or out,
/// an edit's upload), and a change of a material's emission scale in the pixels' light
/// ([`Frame::set_emission_scales`]). The first encode opens it. A later source of changing
/// emissive light (a moving emitter) must open it too. A guess at ten times the far face's
/// re-verify cadence (about 26 frames at 0.34 lighting pixels a face).
pub const GUARD_WINDOW: u32 = 256;
/// `Params::lighting` bit: the a-trous filter is off ([`Lighting::filter`]).
pub const LIGHTING_FILTER_OFF: u32 = 1 << 5;
/// `Params::lighting` bit: the composite's tone map is the identity clamp.
pub const LIGHTING_IDENTITY_TONE_MAP: u32 = 1 << 6;
/// `Params::lighting` bit: the probe passes ran and the composite adds the gathered indirect
/// irradiance ([`Lighting::lamp_bounce`]).
pub const LIGHTING_INDIRECT: u32 = 1 << 12;
/// `Params::lighting` bit: the exposure is automatic ([`Exposure::Auto`]): the frame's is the
/// exposure state's `current`.
pub const LIGHTING_AUTO_EXPOSURE: u32 = 1 << 13;

/// `Params::flags` bit: trace the shadow ray.
pub const FLAG_SHADOWS: u32 = 1;
/// `Params::flags` bit: skip empty blocks through the brick occupancy masks.
pub const FLAG_MASKS: u32 = 2;
/// `Params::flags` bits 2..4: the [`NormalMode`].
pub const NORMAL_SHIFT: u32 = 2;
/// `Params::flags` bits 5..8: the [`ViewMode`].
pub const VIEW_SHIFT: u32 = 5;
/// `Params::flags` bit: skip empty cells larger than a voxel (`LightParams::skips`).
pub const FLAG_SKIPS: u32 = 1 << 9;
/// `Params::flags` bit: the primary ray passes through solid until it has crossed real air
/// (`LookParams::cutaway`).
pub const FLAG_CUTAWAY: u32 = 1 << 10;
/// `Params::flags` bit, set by the frame and not by a look: the field look's split walk. The
/// hit pipeline walks each ray as the cube walk does until it enters a brick the field can hit
/// in (a band brick of a full chunk with a smooth solid voxel in it or its shell), and a second
/// dispatch walks the field from there ([`Frame::set_field_split`]).
pub const FLAG_FIELD_SPLIT: u32 = 1 << 19;

/// `field_resume`'s workgroup size in `march.wgsl`: the split walk hands at most
/// `SPLIT_GROUP` times the dispatch limit of rays on, and a frame of more pixels walks the
/// field unsplit.
const SPLIT_GROUP: u32 = 128;

/// Workgroups a dispatch may count on one dimension (WebGPU's default limit).
const MAX_WORKGROUPS: u32 = 65535;

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
    /// The field surface (ADR-0041): the primary ray hits the zero set of the trilinear
    /// interpolation of `count / 27 - 1/2` over the voxel centres, `count` a voxel's solid
    /// voxels among the 27 around it, and shades with the field's gradient; it moves the hit
    /// too. Needs a mirror with the field's band slots ([`GpuWorld::with_field_surface`]);
    /// without one, and in the cutaway view, the ray walks the voxels and shades as `Face`.
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
    /// The field walk's leaks in the contour look, over the shaded image without its
    /// translucent layers: the hit pass writes 1 into [`Frame::layers`]'s r for a ray that
    /// reached rock outside the field's band or missed its crossing (hit there as a cube), and
    /// the composite paints it red (see [`CrackShares`]). The voxel walk has no leak. Every
    /// pass of the shaded view runs.
    Cracks = 5,
}

impl ViewMode {
    /// The cycle's order.
    pub const ALL: [Self; 6] = [
        Self::Shaded,
        Self::Normals,
        Self::HitMask,
        Self::Steps,
        Self::Cracks,
        Self::Albedo,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Shaded => "shaded",
            Self::Normals => "normals",
            Self::HitMask => "hitmask",
            Self::Steps => "steps",
            Self::Albedo => "albedo",
            Self::Cracks => "cracks",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }

    /// The next view in [`ViewMode::ALL`] order, wrapping.
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|&m| m == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// Whether the frame lights this view as the shaded one: the light, probe, filter,
    /// gradient and meter passes run.
    pub fn shades(self) -> bool {
        matches!(self, Self::Shaded | Self::Cracks)
    }
}

/// The share of a cracks-view frame's hit pixels that are leaks ([`ViewMode::Cracks`]), read
/// from the hit pass's outputs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CrackShares {
    /// Hit pixels.
    pub hits: u64,
    /// Hit pixels that are leaks.
    pub leaks: u64,
}

impl CrackShares {
    /// From [`Frame::layers`] (`rgba16float`, the leak in r) and [`Frame::hit_records`]
    /// (`rgba16uint`, bit 0 of a's flags set on a hit) as [`crate::readback::read_rgba16f`]
    /// and [`crate::readback::read_rgba16u`] return them, one texel per pixel.
    pub fn from_layers(layers: &[[f32; 4]], records: &[[u16; 4]]) -> Self {
        let mut s = Self::default();
        for (l, r) in layers.iter().zip(records) {
            if r[3] & 1 == 0 {
                continue;
            }
            s.hits += 1;
            s.leaks += u64::from(l[0] > 0.5);
        }
        s
    }

    /// Whether a pixel is a leak (`None` on a miss), decoded from a cracks-view image without
    /// an upscaler (`rgba8unorm`, the composite's colours: a miss black, a leak red, any other
    /// hit every channel in 4..=124).
    pub fn decode_rgba8(p: &[u8]) -> Option<bool> {
        match (p[0], p[1], p[2]) {
            (0, 0, 0) => None,
            (255, 0, 0) => Some(true),
            _ => Some(false),
        }
    }

    /// [`CrackShares`] of a cracks-view image (see [`CrackShares::decode_rgba8`]).
    pub fn from_rgba8(pixels: &[u8]) -> Self {
        let mut s = Self::default();
        for leak in pixels.chunks_exact(4).filter_map(Self::decode_rgba8) {
            s.hits += 1;
            s.leaks += u64::from(leak);
        }
        s
    }

    /// The leaks' share of the hit pixels, 0 with none.
    pub fn leak_share(&self) -> f64 {
        self.leaks as f64 / self.hits.max(1) as f64
    }
}

/// The surface look's tuning beyond its normal mode: a mix of a field-surface hit's shading
/// normal toward its entry face's normal. The default is no mix.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SurfaceTuning {
    /// A field-surface hit's shading normal is `normalize(mix(field, face, face_mix))`, `face`
    /// the normal of the face the ray entered its voxel through; the hit point and the shadow
    /// ray's offset keep the field's. In `[0, 0.5]`; 0 is the field normal.
    pub face_mix: f32,
}

/// The surface look: normal mode, view, translucent layers, cutaway, the surface tuning and
/// the far normal.
/// The default is face normals, shaded, 4 layers, cutaway off, the tuning off, the far normal
/// off.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LookParams {
    pub normal_mode: NormalMode,
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
    pub tuning: SurfaceTuning,
    /// Face mode's far shading normal, `[lo, hi]` in pixels per voxel across the view's width:
    /// at or below `lo` a hit shades with the 27-count gradient, at or above `hi` with its face
    /// normal, a smoothstep between. `hi <= lo` (`[0.0, 0.0]`) is off: the face normal, with no
    /// neighbour read. The shadow ray keeps the face normal. No other normal mode reads it.
    pub far_normal: [f32; 2],
}

impl Default for LookParams {
    fn default() -> Self {
        Self {
            normal_mode: NormalMode::default(),
            view: ViewMode::default(),
            translucent_layers: 4,
            cutaway: false,
            tuning: SurfaceTuning::default(),
            far_normal: [0.0; 2],
        }
    }
}

impl LookParams {
    /// The look's bits of `Params::flags`.
    pub fn flags(&self) -> u32 {
        (self.normal_mode as u32) << NORMAL_SHIFT
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

/// The sub-pixel jitter of frame `frame` in a sequence of `phases`, in internal pixels:
/// elements `1 ..= phases` of the Halton (2, 3) sequence by `frame % phases`, centred on zero,
/// each component in `(-0.5, 0.5)`. The upscaler of ADR-0024 takes [`JITTER_PHASES`], FSR 2
/// its own count (`fsr2::jitter_phases`).
pub fn jitter_at(frame: u32, phases: u32) -> [f32; 2] {
    let i = frame % phases.max(1) + 1;
    [halton(i, 2) - 0.5, halton(i, 3) - 0.5]
}

/// The upscale pass's tuning ([`Frame::set_upscaler`]). This frame's colour at an output
/// pixel is reconstructed from the 3 x 3 internal samples around the nearest with the kernel
/// `exp(-2.29 (d / width)^2)`, `d` a sample's distance from the output pixel's centre in
/// output pixels: at width 1, Karis's Gaussian fit of a Blackman-Harris window of width 3.3
/// ("High Quality Temporal Supersampling", 2014). Last frame's history is read at the
/// reprojected position by Catmull-Rom over the 4 x 4 texels around it, held non-negative.
/// The current colour's share of the blend where history is kept is the alpha times the
/// nearest sample's kernel weight over the kernel's peak, divided by the nearest sample's
/// mean kernel weight at the frame's scale (its sample uniform over one internal pixel
/// around the centre), at most 1: a sample far from the output pixel's centre counts for
/// less, and the current colour's mean share stays the alpha. A still pixel (one whose
/// history moved less than 0.01 output pixels) keeps its history unclipped unless the light
/// at its hit changed: the lamp's unshadowed term moved past [`Upscaler::change_rel`] and
/// [`Upscaler::change_abs`] since the lamp of [`Upscaler::change_frames`] frames before, or
/// an emission scale changed within [`EMISSION_CHANGE_FRAMES`]; such a pixel is clipped and
/// blended as a moving one (ADR-0024).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Upscaler {
    /// The reconstruction kernel's width.
    pub width: f32,
    /// The current colour's weight in the blend where the history held still.
    pub alpha: f32,
    /// [`Upscaler::alpha`] where the history moved by 0.01 output pixels or more.
    pub alpha_moving: f32,
    /// The clip box around the 3 x 3 samples' YCoCg mean, in standard deviations (inside
    /// their minimum and maximum).
    pub clip_gamma: f32,
    /// A light change: the lamp's term moved by more than this share of the larger of the
    /// two.
    pub change_rel: f32,
    /// ... and by more than this, in the lamp's irradiance units.
    pub change_abs: f32,
    /// The frames between the lamps compared, 1 to [`LAMP_FRAMES`].
    pub change_frames: u32,
}

/// Frames of lamp parameters the frame keeps for [`Upscaler::change_frames`].
pub const LAMP_FRAMES: usize = 32;

impl Default for Upscaler {
    /// The Gaussian of width 0.6 over the 3 x 3, the Catmull-Rom fetch and the
    /// nearness-weighted blend, normalised to a mean share of 0.1 still and 0.2 in motion
    /// (ADR-0024).
    fn default() -> Self {
        Self {
            width: 0.6,
            alpha: 0.1,
            alpha_moving: 0.2,
            clip_gamma: 1.0,
            change_rel: 0.1,
            change_abs: 0.01,
            change_frames: 24,
        }
    }
}

/// `UpTuning` in `upscale.wgsl`, field for field.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct UpscaleUniform {
    width: f32,
    alpha: f32,
    alpha_moving: f32,
    clip_gamma: f32,
    w_mean: f32,
    change_rel: f32,
    change_abs: f32,
    old_on: u32,
    old_pos: [f32; 3],
    old_intensity: f32,
    old_dir: [f32; 3],
    old_cos_outer: f32,
    old_cos_inner: f32,
    _pad: [f32; 3],
}

/// One frame's lamp, as `Params` carries it.
#[derive(Clone, Copy, Debug, Default)]
struct Lamp {
    on: bool,
    pos: [f32; 3],
    intensity: f32,
    dir: [f32; 3],
    cos_outer: f32,
    cos_inner: f32,
}

impl Lamp {
    fn of(p: &Params) -> Self {
        Self {
            on: p.lighting & LIGHTING_LAMP != 0,
            pos: p.light_pos,
            intensity: p.light_intensity,
            dir: p.light_dir,
            cos_outer: p.cos_outer,
            cos_inner: p.cos_inner,
        }
    }
}

impl Upscaler {
    /// The reconstruction kernel at offset `(x, y)` output pixels, 1 at the centre
    /// (`up_kernel` in `upscale.wgsl`).
    fn kernel(&self, x: f32, y: f32) -> f32 {
        (-2.29 * (x * x + y * y) / (self.width * self.width)).exp()
    }

    /// The nearest sample's mean kernel weight at `scale` output pixels per internal pixel:
    /// its offset uniform over the internal pixel around the output centre, a 32 x 32 grid.
    fn mean_weight(&self, scale: f32) -> f32 {
        const N: usize = 32;
        let at = |i: usize| ((i as f32 + 0.5) / N as f32 - 0.5) * scale;
        let mut sum = 0.0;
        for j in 0..N {
            for i in 0..N {
                sum += self.kernel(at(i), at(j));
            }
        }
        sum / (N * N) as f32
    }

    fn uniform(&self, scale: f32, old: Lamp) -> UpscaleUniform {
        UpscaleUniform {
            width: self.width,
            alpha: self.alpha,
            alpha_moving: self.alpha_moving,
            clip_gamma: self.clip_gamma,
            w_mean: self.mean_weight(scale),
            change_rel: self.change_rel,
            change_abs: self.change_abs,
            old_on: u32::from(old.on),
            old_pos: old.pos,
            old_intensity: old.intensity,
            old_dir: old.dir,
            old_cos_outer: old.cos_outer,
            old_cos_inner: old.cos_inner,
            _pad: [0.0; 3],
        }
    }
}

/// Cells of the emissive table by default ([`Lighting::table_cells`]): 48 B a cell, so
/// 201 326 592 B.
pub const DEFAULT_TABLE_CELLS: u32 = 1 << 22;
/// `u32`s of a cell: the key (two words), the count, the three sums (Q17.15), the kept
/// emitter's voxel and face (two words), its luminance, a frame stamp, the frame it was last
/// seen ([`CACHE_SEEN_WORD`]) and its blended mean ([`CACHE_BLEND_WORD`]).
pub const CACHE_CELL_WORDS: u64 = 12;
/// The cell's word of the count; the three sums follow it.
pub const CACHE_COUNT_WORD: usize = 2;
/// The cell's seen word: `frame + 1` of the last frame a lighting pixel took the cell or a seen
/// cell's blend read it; 0 never.
pub const CACHE_SEEN_WORD: usize = 10;
/// The cell's blended mean, each channel's square root over a shared exponent (`blend_unpack`,
/// `cache_cell.wgsl`; written by `cache_blend.wgsl`).
pub const CACHE_BLEND_WORD: usize = 11;
/// Slots a key probes linearly: the smallest of 4, 8 and 16 under which the hall's, the
/// vein's and the patch's key sets at the window's size, inserted at a load of 0.25 of
/// [`DEFAULT_TABLE_CELLS`], leave no key without a slot (Plan 0057 Phase 7's census: 1356 at
/// 4, 12 at 8).
pub const CACHE_PROBES: u32 = 16;
/// [`Lighting::table_age`] by default, in frames: ten seconds at 60 fps.
pub const DEFAULT_TABLE_AGE: u32 = 600;
/// [`Lighting::table_age_min`] by default, in frames.
pub const DEFAULT_TABLE_AGE_MIN: u32 = 60;
/// The occupied share over which the age-out runs (EVICT_LOAD), as sixteenths: 3/16.
pub const EVICT_LOAD_SIXTEENTHS: u32 = 3;
/// The occupied cells' growth over the mark that runs the age-out again, as a divisor of the
/// cells: 1/32.
pub const EVICT_STEP_DIVISOR: u32 = 32;
/// Cells the table's walk (the chunk sweep and the age-out) visits a frame at most.
pub const TABLE_WALK_SLICE: u32 = 1 << 20;
/// The walk's state buffer (`TableWalk` in `cache_sweep.wgsl`): 20 words.
const WALK_STATE_BYTES: u64 = 80;
/// The fixed point of the sums.
pub const CACHE_Q: f32 = 32768.0;
/// Samples at which a cell stops adding, and its pixels stop drawing, by default
/// ([`Lighting::table_cap`]). At most 4096: one sample's channel reaches 18.85 at the testbed's
/// brightest emitter, which at Q17.15 and 4096 samples is `2.53e9`, under `2^32`.
pub const DEFAULT_TABLE_CAP: u32 = 4096;
/// Cells the reopened fill lists a frame at most (`FILL_LIST_MAX` in `light.wgsl`); a frame's
/// offers past it are dropped and counted.
pub const FILL_LIST_MAX: u32 = 16_384;
/// The table's counters' bytes (`CacheStats` in `march.wgsl` up to its fill list).
const CACHE_STATS_BYTES: u64 = 32;
/// The fill list's bytes, in the counters' buffer past [`CACHE_STATS_BYTES`]: the offer
/// counter, three words of padding and the slots.
pub const FILL_LIST_BYTES: u64 = 16 + 4 * FILL_LIST_MAX as u64;
/// The reopened fill's indirect dispatch: three words.
pub const FILL_DISPATCH_BYTES: u64 = 12;
/// The most samples a frame the fill draws per listed cell (`FILL_LANES` in
/// `cache_fill.wgsl`).
pub const FILL_SAMPLES_MAX: u32 = 16;
/// [`Lighting::fill_samples`] by default.
pub const DEFAULT_FILL_SAMPLES: u32 = 8;
/// [`Lighting::fill_until`] by default.
pub const DEFAULT_FILL_UNTIL: u32 = 256;
/// Workgroups a dispatch dimension takes at most (wgpu's default limit).
pub const MAX_DISPATCH_GROUPS: u32 = 65_535;
/// [`Lighting::preload_radius`] by default, in voxels: 8 bricks.
pub const DEFAULT_PRELOAD_RADIUS: u32 = 64;
/// The largest [`Lighting::preload_radius`], in voxels (16 bricks).
pub const PRELOAD_RADIUS_MAX: u32 = 128;
/// [`Lighting::preload_bricks`] by default.
pub const DEFAULT_PRELOAD_BRICKS: u32 = 128;
/// [`Lighting::preload_claim_radius`] by default (r0), in voxels.
pub const DEFAULT_PRELOAD_CLAIM_RADIUS: u32 = 32;
/// [`Lighting::preload_until`] by default (C_PRE).
pub const DEFAULT_PRELOAD_UNTIL: u32 = 256;
/// [`Lighting::preload_samples`] by default (S_PRE).
pub const DEFAULT_PRELOAD_SAMPLES: u32 = 32_768;
/// Cells each of the preload's two lists holds (`PRELOAD_LIST_MAX` in `march.wgsl`); a frame's
/// entries past it are dropped and counted, and the sweep lists them again.
pub const PRELOAD_LIST_MAX: u32 = 32_768;
/// Samples the sweep draws for a face with no cell before it claims one (k0;
/// `PRELOAD_PROBE_MAX` in `cache_fill.wgsl`).
pub const PRELOAD_PROBE_SAMPLES: u32 = 8;
/// The share of the table's cells past which the sweep claims no cell (PRE_LOAD_MAX), as a
/// divisor: a quarter.
pub const PRELOAD_LOAD_DIVISOR: u32 = 4;
/// The edit boxes' bricks the sweep visits first in a frame, at most (`brick_list` in
/// `cache_fill.wgsl`); the rest of a large edit wait for the sweep's cursor.
pub const PRELOAD_EDIT_BRICKS_MAX: usize = 256;
/// Offsets the sweep's table holds at [`PRELOAD_RADIUS_MAX`]: `preload_offsets(16).len()`.
pub const PRELOAD_OFFSETS_MAX: u64 = 17_077;
/// The counters' bytes before the preload's lists: `CacheStats` from `pre_cursor` to
/// `pre_idle_epoch` (38 words, [`PRELOAD_STATE_BYTES`]; the frame's counts and the args pass's
/// snapshots are words 16 to 31, [`PRELOAD_COUNTS_BYTES`] from byte 64),
/// then its two fill lists' 64 distance bands.
const PRELOAD_HEADER_BYTES: u64 = PRELOAD_STATE_BYTES + 2 * 64 * 4;
const PRELOAD_COUNTS_BYTES: u64 = 64;
const PRELOAD_STATE_BYTES: u64 = 152;
/// Faces each of the preload's two probe lists holds (`PRELOAD_PROBE_LIST_MAX` in
/// `cache_fill.wgsl`), two words a face; a frame's faces past it are dropped and counted, and
/// their slab is visited again.
pub const PRELOAD_PROBE_LIST_MAX: u32 = 16_384;
/// The preload's slab stamps (`cache_fill.wgsl`): one word for each z layer of 8 x 8 voxels of
/// each brick of a cube of 33 bricks an edge, the bricks within [`PRELOAD_RADIUS_MAX`] of the
/// eye's brick at their coordinates modulo 33.
pub const PRELOAD_STAMPS: u64 = 33 * 33 * 33 * 8;
/// The preload's store in the counters' buffer past the fill list: its header, its two fill
/// lists (two words an entry), the sweep's offsets, its two probe lists, the left and claimed
/// lists and its slab stamps.
pub const PRELOAD_BYTES: u64 = PRELOAD_HEADER_BYTES
    + 4 * (6 * PRELOAD_LIST_MAX as u64
        + PRELOAD_OFFSETS_MAX
        + 4 * PRELOAD_PROBE_LIST_MAX as u64
        + PRELOAD_STAMPS);
/// The preload's indirect dispatches: the sweep's three words, then the light pass's.
pub const PRELOAD_DISPATCH_BYTES: u64 = 24;

fn cache_mix(v: [i32; 3], face: u32, a: u32, b: u32) -> u32 {
    let mut h = (v[0] as u32).wrapping_mul(a)
        ^ (v[1] as u32).wrapping_mul(0xD816_3841)
        ^ (v[2] as u32).wrapping_mul(b)
        ^ (face + 1).wrapping_mul(0x1656_67B1);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    h
}

/// A key's first slot in a table of `cells` cells (`cache_slot` in `march.wgsl`): the voxel and
/// its entry face, `2 axis + 1` toward `+axis`.
pub fn cache_slot(voxel: [i32; 3], face: u32, cells: u32) -> u32 {
    cache_mix(voxel, face, 0x8DA6_B343, 0xCB1A_B31F) & (cells - 1)
}

/// A key's two words (`cache_key` in `march.wgsl`): each coordinate's low 20 bits, x and y's
/// low 12 in the first; y's high 8, z, the face in bits 28..31 and a valid bit 31 in the
/// second, which is never 0.
pub fn cache_key(voxel: [i32; 3], face: u32) -> [u32; 2] {
    let [x, y, z] = voxel.map(|c| c as u32 & 0xF_FFFF);
    [x | y << 20, y >> 12 | z << 8 | face << 28 | 1 << 31]
}

/// The brick offsets the preload's sweep visits around the eye's brick, nearest first: every
/// `o` with `|o|^2 <= radius^2` in bricks, sorted by `|o|^2`, then z, y and x. At a radius of
/// 8 bricks (64 voxels) 2109 offsets.
pub fn preload_offsets(radius: u32) -> Vec<[i32; 3]> {
    let r = radius as i32;
    let mut out = Vec::new();
    for z in -r..=r {
        for y in -r..=r {
            for x in -r..=r {
                if x * x + y * y + z * z <= r * r {
                    out.push([x, y, z]);
                }
            }
        }
    }
    out.sort_by_key(|&[x, y, z]| (x * x + y * y + z * z, z, y, x));
    out
}

/// The preload's counts of the last encode, read back by [`Frame::read_preload_stats`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PreloadStats {
    /// Bricks the sweep visited: the edit boxes' within the radius, then the table's.
    pub edit_bricks: u32,
    pub visited: u32,
    /// Slabs whose voxels were read (past their stamp and the brick's early-out).
    pub scanned: u32,
    /// Faces put on the list (a cell under C_PRE not yet on it, or a cell just claimed).
    pub listed: u32,
    /// Cells the sweep claimed for faces with none, in every band.
    pub claimed: u32,
    /// Of `claimed`, the cells within the claim radius whose probes drew light, and those whose
    /// probes were all dark; the rest (`claimed - near_lit - near_unlit`) lay from r0 to R.
    pub claimed_near_lit: u32,
    pub claimed_near_unlit: u32,
    /// Faces with no cell refused one: the table's load, a miss (the last frame's light pass
    /// or the claim), no lit probe sample (beyond the claim radius only). A face within r0
    /// that the load or a miss holds back is counted on each pass of the sweep that finds it.
    pub refused_load: u32,
    pub refused_miss: u32,
    pub refused_unlit: u32,
    /// Emissive samples the preload drew (the fill's and the sweep's probes).
    pub samples: u32,
    /// List entries dropped by a full list.
    pub drops: u32,
    /// Faces with no cell the probes sampled (the last frame's sweep found them), and the
    /// probe list's entries dropped by a full list this frame.
    pub probed: u32,
    pub probe_drops: u32,
    /// Slabs (8 x 8 x 1 voxels) the sweep skipped as done at this epoch.
    pub done: u32,
    /// The list's entries the frame filled from (its input) and the cells it drew for.
    pub list: u32,
    pub drawn: u32,
    /// The probe list's entries the frame probed (its input).
    pub probes: u32,
}

/// The preload's idle skip: while the args pass of some encode read the sweep idle (both lists
/// empty and no brick visited), and every encode since was quiet (the camera still to the
/// preload's test and in the same brick, the guard's window closed, no edit box, no world or
/// table change, no chunk sweep, the same C_PRE, radius, claim radius and lighting grid), the
/// preload's passes would change nothing and draw nothing, so the frame runs without them (as
/// with the preload off) until an encode is not quiet. The args pass's words are copied into a
/// small ring of mappable buffers and read back without blocking, a frame or more late.
struct PreloadIdle {
    slots: Vec<(wgpu::Buffer, IdleSlot)>,
    /// Encodes with the preload wanted, and the first of the run of quiet ones.
    encodes: u64,
    quiet_since: Option<u64>,
    /// A readback of an encode in the quiet run read idle.
    confirmed: bool,
    /// The last encode's eye brick and lighting grid.
    eye_brick: Option<[i32; 3]>,
    grid: Option<[u32; 2]>,
}

enum IdleSlot {
    Free,
    Copied(u64),
    Mapping(u64, std::sync::Arc<std::sync::atomic::AtomicU8>),
}

/// C_PRE, the preload's radius in bricks and its claim radius: a change raises the epoch.
type PreloadKey = (u32, u32, u32);

/// The preload's state words the idle skip reads: `pre_n_in` on (`CacheStats` words 5 to 12 of
/// the preload's header, of which `pre_n_in`, `pre_visit` and `pre_probe_n`), and the ring's
/// slots.
const PRELOAD_IDLE_WORDS_AT: u64 = 20;
const PRELOAD_IDLE_BYTES: u64 = 32;
const PRELOAD_IDLE_SLOTS: usize = 4;

impl PreloadIdle {
    fn new(device: &wgpu::Device) -> Self {
        Self {
            slots: (0..PRELOAD_IDLE_SLOTS)
                .map(|_| {
                    (
                        device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("preload idle"),
                            size: PRELOAD_IDLE_BYTES,
                            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                            mapped_at_creation: false,
                        }),
                        IdleSlot::Free,
                    )
                })
                .collect(),
            encodes: 0,
            quiet_since: None,
            confirmed: false,
            eye_brick: None,
            grid: None,
        }
    }

    /// The encode's decision: whether it skips the preload's passes, given whether it is
    /// quiet, after a non-blocking poll of the slots copied before it.
    fn decide(&mut self, device: &wgpu::Device, quiet: bool) -> bool {
        let now = self.encodes;
        self.encodes += 1;
        if !quiet {
            self.quiet_since = None;
            self.confirmed = false;
        } else if self.quiet_since.is_none() {
            self.quiet_since = Some(now);
        }
        for (buf, slot) in &mut self.slots {
            // Copied two encodes back or more: the encoder that copied it has been submitted.
            if let IdleSlot::Copied(at) = *slot
                && at + 1 < now
            {
                let done = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0));
                let flag = done.clone();
                buf.map_async(wgpu::MapMode::Read, .., move |r| {
                    flag.store(
                        if r.is_ok() { 1 } else { 2 },
                        std::sync::atomic::Ordering::Release,
                    );
                });
                *slot = IdleSlot::Mapping(at, done);
            }
        }
        let _ = device.poll(wgpu::PollType::Poll);
        for (buf, slot) in &mut self.slots {
            let IdleSlot::Mapping(at, done) = slot else {
                continue;
            };
            match done.load(std::sync::atomic::Ordering::Acquire) {
                0 => continue,
                1 => {
                    let idle = {
                        let bytes = buf
                            .get_mapped_range(..PRELOAD_IDLE_BYTES)
                            .expect("the idle words are mapped");
                        let w: &[u32] = bytemuck::cast_slice(&bytes);
                        // n_in, visit and probe_n (words 5, 8 and 11 of the header).
                        w[0] == 0 && w[3] == 0 && w[6] == 0
                    };
                    buf.unmap();
                    if idle && self.quiet_since.is_some_and(|q| *at >= q) {
                        self.confirmed = true;
                    }
                }
                _ => {}
            }
            *slot = IdleSlot::Free;
        }
        quiet && self.confirmed
    }

    /// A free slot to copy this encode's words into, marked copied.
    fn take_slot(&mut self) -> Option<&wgpu::Buffer> {
        let at = self.encodes.saturating_sub(1);
        self.slots
            .iter_mut()
            .find(|(_, s)| matches!(s, IdleSlot::Free))
            .map(|(b, s)| {
                *s = IdleSlot::Copied(at);
                &*b
            })
    }
}

/// The blend's uniform (`Blend` in `cache_blend.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlendUniform {
    cache_mask: u32,
    c_cap: u32,
    pool_min: u32,
    wide: u32,
    frame: u32,
    force: u32,
    _pad: [u32; 2],
}

/// What the temporal pass reads of a table cell (`cache_blend.wgsl`): a pool of its face's samples
/// and its coplanar neighbours', or its own mean.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TableBlend {
    /// The cell's own mean.
    Off,
    /// The 3 x 3 pool.
    Narrow,
    /// The 3 x 3 pool, and the 5 x 5's where it holds under [`Lighting::blend_pool_min`] samples.
    #[default]
    Wide,
}

impl TableBlend {
    /// The mode's word in the banner and on the command line.
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Narrow => "3x3",
            Self::Wide => "5x5",
        }
    }
}

/// [`Lighting::blend_pool_min`] by default.
pub const DEFAULT_BLEND_POOL_MIN: u32 = 4_096;

/// A blend word's colour (`blend_unpack` in `cache_cell.wgsl`).
pub fn blend_unpack(word: u32) -> [f32; 3] {
    let scale = 2f32.powi((word >> 27) as i32 - 26);
    [word & 511, (word >> 9) & 511, (word >> 18) & 511].map(|m| {
        let q = m as f32 / 511.0;
        q * q * scale
    })
}

/// The table walk's uniform (`Sweep` in `cache_sweep.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SweepUniform {
    chunk: [i32; 3],
    r2: i32,
    eye: [f32; 3],
    reach2: f32,
    request: u32,
    cells: u32,
    evict_load: u32,
    evict_step: u32,
    age_max: u32,
    age_min: u32,
    slice: u32,
    frame: u32,
    age_out: u32,
    _pad: [u32; 3],
}

/// The table's age-out after the last submitted encode ([`Frame::read_age_stats`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AgeStats {
    /// Age-out passes started since the frame was made.
    pub passes: u32,
    /// Cells the age-out has freed since the frame was made, and in the last encode.
    pub evicted: u32,
    pub evicted_frame: u32,
    /// Cells the chunk sweep has freed since the frame was made.
    pub swept: u32,
    /// The age the next pass takes, in frames.
    pub age: u32,
    /// Cells the running walk has left (0 idle).
    pub walk_left: u32,
}

/// The preload's uniform (`Preload` in `cache_fill.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PreloadUniform {
    eye_brick: [i32; 3],
    edit_bricks: u32,
    offsets: u32,
    bricks: u32,
    still: u32,
    _pad0: u32,
    until: u32,
    draw_cells: u32,
    load_max: u32,
    probe: u32,
    epoch: u32,
    light_cols: u32,
    light_rows: u32,
    _pad1: u32,
    eye: [f32; 3],
    claim: f32,
    brick_list: [[i32; 4]; PRELOAD_EDIT_BRICKS_MAX],
}

/// The emissive table's counters, read back by [`Frame::read_table_stats`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TableStats {
    /// The table's cells ([`Lighting::table_cells`]).
    pub cells: u32,
    /// Cells holding a key.
    pub occupied: u32,
    /// Lighting pixels of the last encode whose key found neither its cell nor a free slot
    /// in [`CACHE_PROBES`] probes; their emissive part was their own sample.
    pub misses: u32,
}

/// Where a cube face's emissive light comes from (ADR-0046).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Light {
    /// The view-driven per-face table (ADR-0043).
    #[default]
    Table,
    /// The loaded store, baked ahead of the frames that read it ([`Frame::bake_until`]).
    Baked,
}

impl Light {
    /// The name `--light` takes and the modes' line shows.
    pub fn name(self) -> &'static str {
        match self {
            Self::Table => "table",
            Self::Baked => "baked",
        }
    }

    /// The light named `name`.
    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Table, Self::Baked]
            .into_iter()
            .find(|l| l.name() == name)
    }
}

/// The frame's lighting knobs beside the lamp's own ([`LightParams`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lighting {
    /// Radiance is multiplied by the exposure before the tone map (ADR-0033).
    pub exposure: Exposure,
    pub tone_map: ToneMap,
    /// The headlamp lights the frame; off leaves emissive light alone.
    pub lamp: bool,
    /// Emissive candidates the light pass resamples per pixel; 0 turns emissive light off.
    pub candidates: u32,
    pub shadow_walk: ShadowWalk,
    /// The a-trous filter runs over the lamp's part (and a fallback hit's emissive part); off,
    /// the composite shows the light pass's raw irradiance.
    pub filter: bool,
    pub history: HistoryKill,
    /// The lamp's bounce from the probe volume (ADR-0043's lamp channel over ADR-0023's
    /// volume): with the lamp on, the probes within [`Lighting::lamp_reach`] of it trace the
    /// headlamp's light and the gather adds their irradiance, in the shaded view outside the
    /// cutaway. With the lamp off no probe pass is encoded and the gather is skipped (a volume
    /// whose updates are off, a test's preset, is still gathered). Off by default here; the
    /// fly-through turns it on.
    pub lamp_bounce: bool,
    /// Rays each updated probe traces, clamped to 1 ..= 64 (default 64): fewer rays cost less
    /// and blend a noisier estimate.
    pub probe_rays: u32,
    /// The probe volume refreshes one probe in this many each frame outside a light-change
    /// window ([`Frame::probe_light_window`]), rounded up to a power of two in 8 ..= 32
    /// (default 8): more slices cost less and lag longer.
    pub probe_slices: u32,
    /// A probe blends as a running mean over this many visits since its restart, 1 ..=
    /// `probes::MAX_AGE` (default [`DEFAULT_PROBE_FREEZE`]), then stops tracing and blending
    /// until a restart: the channel's first fill (the lamp switched on, its parameters changed,
    /// or the channel turned on), an edit touching it, a scroll bringing a new probe into its
    /// slot, the probe entering the lamp's reach, or the lamp moving. A probe inside rock
    /// returns at once from the probe passes until a restart.
    pub probe_freeze: u32,
    /// The lamp's reach in voxels (default [`crate::probes::LAMP_REACH`]): only the probes
    /// within it of the lamp are updated, and the gather reads the others as zero. The volume's
    /// side is the smallest power of two whose half covers it
    /// ([`crate::probes::probes_per_axis`]); 0 or less is no cut.
    pub lamp_reach: f32,
    /// A probe's blend in reach while the lamp moves, `h old + (1 - h) new` (default
    /// [`crate::probes::DEFAULT_H_MOVE`]); its running mean restarts once the lamp is still.
    pub probe_move_blend: f32,
    /// Cells of the emissive table (ADR-0043), a power of two (default
    /// [`DEFAULT_TABLE_CELLS`]): the emissive direct irradiance per cube face, its only store.
    /// The light pass adds each sample to the hit face's cell and draws nothing on a capped
    /// one; the temporal pass shows the cell's mean from its first sample. A hit with no face
    /// (the field look, a translucent hit, a ray from inside rock) or whose key finds no slot
    /// blends its own sample as the lamp's part does. An edit box clears its cells, an
    /// emission-scale change ([`Frame::set_emission_scales`]) the whole table, the camera
    /// entering another chunk frees the cells beyond the light radius, and the age-out
    /// ([`Lighting::table_age_out`]) frees the cells far and long unseen.
    pub table_cells: u32,
    /// Samples at which a cell stops adding, at most 4096 (default [`DEFAULT_TABLE_CAP`]).
    pub table_cap: u32,
    /// The temporal pass reads the table (default on). Off, every hit takes the fallback: for
    /// tests that compare against the table.
    pub table_read: bool,
    /// What the temporal pass reads of a cell (default [`TableBlend::Wide`]).
    pub blend: TableBlend,
    /// The samples under which a 3 x 3 pool widens to the 5 x 5 under [`TableBlend::Wide`].
    pub blend_pool_min: u32,
    /// The table's age-out (default on): when the occupied cells cross 3/16 of the table
    /// upward, again once they have grown by 1/32 of it since the last pass while still over,
    /// and again [`Lighting::table_age`] frames after a pass that left them over having freed a
    /// cell or started on their growth, a walk frees every cell whose voxel lies beyond the preload's radius of the eye
    /// and that no lighting pixel nor a seen cell's blend has read in its age. Off is for
    /// tests that count what the table holds without it.
    pub table_age_out: bool,
    /// The age-out's age in frames (default [`DEFAULT_TABLE_AGE`]): halved by each pass that
    /// leaves the occupied cells over the mark, down to [`Lighting::table_age_min`] (default
    /// [`DEFAULT_TABLE_AGE_MIN`]), and back once they are under it.
    pub table_age: u32,
    pub table_age_min: u32,
    /// The reopened fill's samples a frame per listed cell, at most [`FILL_SAMPLES_MAX`]
    /// (default [`DEFAULT_FILL_SAMPLES`]); 0 is off. A cell an edit box or the guard reopened
    /// is listed by the first of its pixels each frame, and the fill draws this many emissive
    /// samples from random points on its face, so a dug hole converges at this rate however
    /// few pixels its faces cover. A face seen for the first time with no edit is not listed.
    pub fill_samples: u32,
    /// The count at which a reopened cell leaves the fill's list (C_FILL; default
    /// [`DEFAULT_FILL_UNTIL`], at most [`Lighting::table_cap`]).
    pub fill_until: u32,
    /// The camera-radius preload's radius in voxels, rounded down to whole bricks and at most
    /// [`PRELOAD_RADIUS_MAX`] (default [`DEFAULT_PRELOAD_RADIUS`]); 0 is off. A cursor sweeps the
    /// mirror's bricks within it around the eye's brick, nearest first, and lists every lit
    /// face it finds, claiming a cell for a face with none when one of
    /// [`PRELOAD_PROBE_SAMPLES`] samples is lit, the table held under a quarter of its cells
    /// and the last frame counted no miss; the listed cells are filled to
    /// [`Lighting::preload_until`] whatever the view shows. Each slab of a brick the sweep has
    /// read is stamped with the world's epoch, which every edit, upload, table clear and chunk
    /// sweep raises: a face no emitter reaches is probed once an epoch, and a slab with nothing
    /// left to fill is skipped. The fill and the probes run in the light pass's dispatch.
    pub preload_radius: u32,
    /// Bricks the sweep visits a frame (B; default [`DEFAULT_PRELOAD_BRICKS`]), after the
    /// frame's edit boxes' bricks; none while idle: the lists empty, the camera moved under a
    /// voxel and turned under a degree since the last encode, the guard's window closed, and a
    /// whole pass of the sweep at this epoch found every slab done.
    pub preload_bricks: u32,
    /// The count to which the preload fills a listed cell (C_PRE; default
    /// [`DEFAULT_PRELOAD_UNTIL`], at most [`Lighting::table_cap`]).
    pub preload_until: u32,
    /// The preload's claim radius in voxels (r0; default [`DEFAULT_PRELOAD_CLAIM_RADIUS`], at
    /// most the radius; 0 none): a face whose voxel centre lies within it of the eye claims a
    /// cell whatever its probes read (the load and miss guards still apply) and is filled to
    /// [`Lighting::preload_until`] lit or unlit, so a face no emitter reaches is filled once with
    /// zeros and then skipped for the life of its cell. From r0 to the radius a face claims a
    /// cell only when a probe sample is lit.
    pub preload_claim_radius: u32,
    /// The preload fill's samples a frame at most (S_PRE; default
    /// [`DEFAULT_PRELOAD_SAMPLES`]), [`FILL_SAMPLES_MAX`] a listed cell, so the list's first
    /// `S_PRE / 16` cells draw each frame.
    pub preload_samples: u32,
    /// Where a cube face's emissive light comes from (default [`Light::Table`]). Under
    /// [`Light::Baked`] the temporal pass reads the store, the light pass draws no emissive sample
    /// for a cube face, and [`Frame::set_lighting`] turns off the table's blend, fill, preload and
    /// age-out; a hit with no face keeps the fallback.
    pub light: Light,
    /// The store's pool under [`Light::Baked`], clamped to the device's storage binding (default
    /// [`crate::bake::DEFAULT_BAKE_POOL_BYTES`]). [`Light::Table`] allocates a minimal one.
    pub bake_pool_bytes: u64,
    /// Under [`Light::Table`], the store's pool is kept as it is rather than shrunk, so a
    /// comparison can swap the read between the table and a baked store in place (default off).
    pub keep_store: bool,
}

impl Default for Lighting {
    fn default() -> Self {
        Self {
            exposure: Exposure::default(),
            tone_map: ToneMap::default(),
            lamp: true,
            candidates: 8,
            shadow_walk: ShadowWalk::default(),
            filter: true,
            history: HistoryKill::default(),
            lamp_bounce: false,
            probe_rays: crate::probes::RAYS_PER_PROBE,
            probe_slices: crate::probes::SLICES,
            probe_freeze: DEFAULT_PROBE_FREEZE,
            lamp_reach: crate::probes::LAMP_REACH,
            probe_move_blend: crate::probes::DEFAULT_H_MOVE,
            table_cells: DEFAULT_TABLE_CELLS,
            table_cap: DEFAULT_TABLE_CAP,
            table_read: true,
            blend: TableBlend::default(),
            blend_pool_min: DEFAULT_BLEND_POOL_MIN,
            table_age_out: true,
            table_age: DEFAULT_TABLE_AGE,
            table_age_min: DEFAULT_TABLE_AGE_MIN,
            fill_samples: DEFAULT_FILL_SAMPLES,
            fill_until: DEFAULT_FILL_UNTIL,
            preload_radius: DEFAULT_PRELOAD_RADIUS,
            preload_bricks: DEFAULT_PRELOAD_BRICKS,
            preload_until: DEFAULT_PRELOAD_UNTIL,
            preload_claim_radius: DEFAULT_PRELOAD_CLAIM_RADIUS,
            preload_samples: DEFAULT_PRELOAD_SAMPLES,
            light: Light::Table,
            bake_pool_bytes: crate::bake::DEFAULT_BAKE_POOL_BYTES,
            keep_store: false,
        }
    }
}

impl Lighting {
    /// [`Lighting::table_cells`] as the frame allocates it: a power of two, at least 64.
    pub fn cells(&self) -> u32 {
        self.table_cells.max(64).next_power_of_two()
    }

    /// `Params::fill`.
    fn fill_word(&self) -> u32 {
        let until = self
            .fill_until
            .clamp(1, self.table_cap.clamp(1, DEFAULT_TABLE_CAP));
        self.fill_samples.min(FILL_SAMPLES_MAX) | until << 8
    }

    /// [`Lighting::preload_radius`] in whole bricks, at most 16; 0 is off.
    pub fn preload_brick_radius(&self) -> u32 {
        self.preload_radius.min(PRELOAD_RADIUS_MAX) / 8
    }

    /// [`Lighting::preload_until`] as the preload takes it: 1 to the cap.
    pub fn preload_cap(&self) -> u32 {
        self.preload_until
            .clamp(1, self.table_cap.clamp(1, DEFAULT_TABLE_CAP))
    }

    /// `Params::cache`.
    fn cache_word(&self) -> u32 {
        if self.light == Light::Baked {
            return 4 | (self.table_cap.clamp(1, DEFAULT_TABLE_CAP) & 0x3FFF) << 14;
        }
        u32::from(self.table_read)
            | u32::from(self.blend != TableBlend::Off) << 1
            | (self.table_cap.clamp(1, DEFAULT_TABLE_CAP) & 0x3FFF) << 14
    }

    /// The word of `ProbeUniform::steady`: the freeze count.
    fn probe_steady(&self) -> u32 {
        self.probe_freeze.clamp(1, crate::probes::MAX_AGE)
    }

    /// [`Lighting::probe_rays`] as the probe passes take it.
    pub fn rays(&self) -> u32 {
        self.probe_rays.clamp(1, crate::probes::RAYS_PER_PROBE)
    }

    /// [`Lighting::probe_slices`] as the probe passes take it.
    pub fn slices(&self) -> u32 {
        self.probe_slices
            .clamp(crate::probes::SLICES, crate::probes::MAX_SLICES)
            .next_power_of_two()
    }

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
            | if self.lamp_bounce {
                LIGHTING_INDIRECT
            } else {
                0
            }
            | if matches!(self.exposure, Exposure::Auto(_)) {
                LIGHTING_AUTO_EXPOSURE
            } else {
                0
            }
    }

    /// The manual exposure, or 1.0 in auto.
    pub fn manual_exposure(&self) -> f32 {
        match self.exposure {
            Exposure::Manual(e) => e,
            Exposure::Auto(_) => 1.0,
        }
    }
}

/// The hit pass: `march.wgsl`'s `main`, one primary ray per pixel into the hit buffer, and in
/// the contour look over a mirror with the field ([`NormalMode::Contour`]) its pipeline with
/// `FIELD_SURFACE`, which walks the field surface, or with the split walk
/// ([`FLAG_FIELD_SPLIT`]) the plain pipeline and then `field_resume` over the rays it handed
/// on.
pub struct Marcher {
    pipeline: wgpu::ComputePipeline,
    field: wgpu::ComputePipeline,
    resume: wgpu::ComputePipeline,
    out_layout: wgpu::BindGroupLayout,
    resume_layout: wgpu::BindGroupLayout,
}

impl Marcher {
    fn new(device: &wgpu::Device, world_layout: &wgpu::BindGroupLayout) -> Self {
        use wgpu::TextureFormat::{R32Uint, Rgba16Float, Rgba16Uint, Rgba32Float};
        let out_layout = io_layout(
            device,
            "hit out",
            &[
                Io::Store(Rgba32Float),
                Io::Store(Rgba16Uint),
                Io::Store(Rgba16Float),
                Io::Storage,
                Io::Storage,
                Io::Store(R32Uint),
                Io::ReadStorage,
            ],
        );
        // The resume dispatch reads its workgroup count from the split arguments, so they are
        // not bound to it.
        let resume_layout = io_layout(
            device,
            "hit resume",
            &[
                Io::Store(Rgba32Float),
                Io::Store(Rgba16Uint),
                Io::Store(Rgba16Float),
                Io::Storage,
            ],
        );
        let layouts = [Some(world_layout), Some(&out_layout)];
        let pipeline = compute_pipeline(device, "hit", MARCH_WGSL.into(), &layouts, "main");
        let field = compute_pipeline_with(
            device,
            "hit field",
            MARCH_WGSL.into(),
            &layouts,
            "main",
            &[("FIELD_SURFACE", 1.0)],
        );
        let resume = compute_pipeline(
            device,
            "hit field resume",
            MARCH_WGSL.into(),
            &[Some(world_layout), Some(&resume_layout)],
            "field_resume",
        );
        Self {
            pipeline,
            field,
            resume,
            out_layout,
            resume_layout,
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
    /// The light pass alone, built on first use (`light_layout` in group 2).
    light: std::sync::OnceLock<wgpu::ComputePipeline>,
    /// The reopened fill (`cache_fill.wgsl`): its two pipelines, their group 2 and 3 layouts,
    /// and the indirect dispatch the args pass writes (the list the light pass writes lies in
    /// `cache_stats` past its 32 bytes of counters).
    fill: wgpu::ComputePipeline,
    fill_args: wgpu::ComputePipeline,
    fill_layout: wgpu::BindGroupLayout,
    fill_stats_layout: wgpu::BindGroupLayout,
    fill_args_layout: wgpu::BindGroupLayout,
    fill_dispatch: wgpu::Buffer,
    /// The camera-radius preload (`cache_fill.wgsl`): its args pass, fill and sweep, their
    /// group 3 layouts (the uniform; the uniform and the dispatches), its uniform and its
    /// indirect dispatches. Its lists and the sweep's offsets lie in `cache_stats` past the
    /// fill list.
    preload_args: wgpu::ComputePipeline,
    /// The light pass and the preload's fill, probes and sweep in one indirect dispatch
    /// (`shade_preload`), written by the preload's args pass.
    light_preload: wgpu::ComputePipeline,
    /// The preload's sweep (`preload_sweep`), its own dispatch after the args pass.
    preload_sweep: wgpu::ComputePipeline,
    /// The preload's epoch (the slab stamps', `cache_fill.wgsl`), raised on every encode whose
    /// world or table changed as a whole, and C_PRE, the radius in bricks and the claim radius
    /// at the last encode.
    preload_epoch: std::sync::Mutex<(u32, Option<PreloadKey>)>,
    preload_layout: wgpu::BindGroupLayout,
    preload_args_layout: wgpu::BindGroupLayout,
    preload_uniform: wgpu::Buffer,
    preload_dispatch: wgpu::Buffer,
    /// The brick radius whose offsets `cache_stats` holds, `None` before the first write.
    preload_offsets_radius: std::sync::Mutex<Option<u32>>,
    /// The edit and table bricks the last encode's sweep visited at most (the edit bricks,
    /// then the table's B), for [`Frame::read_preload_stats`].
    preload_edit_bricks: std::sync::atomic::AtomicU32,
    /// The last encode ran the preload.
    preload_ran: std::sync::atomic::AtomicBool,
    /// The preload's idle skip ([`PreloadIdle`]), and the encodes it skipped.
    preload_idle: std::sync::Mutex<PreloadIdle>,
    preload_skips: std::sync::atomic::AtomicU32,
    temporal_layout: wgpu::BindGroupLayout,
    /// The loaded store (minimal under [`Light::Table`]) and the bake's pipeline, built on the
    /// first bake.
    store: crate::bake::Store,
    bake: Option<crate::bake::BakePipeline>,
    /// The edit boxes since the last dig re-bake ([`Frame::rebake_digs`]), every one, while the
    /// store holds a piece.
    dig_boxes: Vec<[[i32; 3]; 2]>,
    temporal: wgpu::ComputePipeline,
    gradient_layout: wgpu::BindGroupLayout,
    gradient_cache_layout: wgpu::BindGroupLayout,
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
    /// Whether the field look takes the split walk ([`FLAG_FIELD_SPLIT`]); on by default.
    field_split: bool,
    lighting: Lighting,
    /// The per-material emission scales, 1.0 unless set: an `f32` per material id for the
    /// light the frame's pixels sample, then [`EMISSION_ROWS`] more for the light the probe
    /// rays sample.
    emission_scale: wgpu::Buffer,
    emission_cpu: Vec<f32>,
    /// The blue-noise texture, uploaded by the first `encode`.
    blue_noise: wgpu::Texture,
    blue_noise_written: std::sync::atomic::AtomicBool,
    /// The frame index the random numbers are seeded with.
    frame: u32,
    /// The frame index of the last emission-scale change.
    emission_changed_at: Option<u32>,
    /// Whether the last encode ran the probe update (the lamp channel).
    probe_channel_ran: std::sync::atomic::AtomicBool,
    /// The probe trace, blend and gather dispatches the last encode recorded.
    probe_dispatches: std::sync::atomic::AtomicU32,
    /// The copy of the double-buffered targets the next `encode` writes.
    cur: usize,
    resolution: LightingResolution,
    downsample_layout: wgpu::BindGroupLayout,
    downsample: wgpu::ComputePipeline,
    upscale_layout: wgpu::BindGroupLayout,
    upscale: wgpu::ComputePipeline,
    sharpen_layout: wgpu::BindGroupLayout,
    sharpen: wgpu::ComputePipeline,
    /// The sharpening pass's `con` (`Sharpen` in `sharpen.wgsl`).
    sharpen_params: wgpu::Buffer,
    sharpness: f32,
    /// The upscale pass's tuning (`UpTuning` in `upscale.wgsl`).
    upscale_params: wgpu::Buffer,
    upscaler: Upscaler,
    history: std::sync::Mutex<History>,
    /// The probe volume, built on first use (an encode with the lamp channel running,
    /// [`Frame::probes`] or [`Frame::probes_mut`]), at `probe_spacing`; its passes' group-3
    /// layout; the gather's group-2 layout. The device builds the volume.
    probes: std::sync::OnceLock<ProbeVolume>,
    /// The emissive table (ADR-0043): [`Lighting::cells`] cells of [`CACHE_CELL_WORDS`]
    /// `u32`s, rebuilt when the cell count changes; and its counters (`CacheStats` in
    /// `march.wgsl`: misses, occupied, two pads).
    cache: wgpu::Buffer,
    cache_stats: wgpu::Buffer,
    /// The table clears at the next encode: [`Frame::reset_history`] asked, or the table was
    /// rebuilt.
    cache_clear_requested: std::sync::atomic::AtomicBool,
    /// Whole-table clears and sweeps encoded since the frame was made, for readings.
    cache_clears: std::sync::atomic::AtomicU32,
    cache_sweeps: std::sync::atomic::AtomicU32,
    cache_clear: wgpu::ComputePipeline,
    cache_clear_layout: wgpu::BindGroupLayout,
    cache_sweep: wgpu::ComputePipeline,
    cache_sweep_layout: wgpu::BindGroupLayout,
    /// The walk's args pass (`walk_args`, on `cache_sweep_layout`) and the slice's layout.
    cache_walk_args: wgpu::ComputePipeline,
    cache_walk_layout: wgpu::BindGroupLayout,
    /// The walk's uniform (`Sweep` in `cache_sweep.wgsl`), its state across frames (`TableWalk`,
    /// read back by [`Frame::read_age_stats`]) and its slice's indirect dispatch.
    sweep_params: wgpu::Buffer,
    walk_state: wgpu::Buffer,
    walk_dispatch: wgpu::Buffer,
    /// The table's cells as allocated ([`Frame::table_cells`]).
    table_cells: u32,
    /// The blend pass (`cache_blend.wgsl`): its args pass and layout, the pass and layout, its
    /// uniform and indirect dispatch, and whether the next encode runs it whatever the table
    /// did (the mode changed).
    cache_blend_args: wgpu::ComputePipeline,
    cache_blend_args_layout: wgpu::BindGroupLayout,
    cache_blend: wgpu::ComputePipeline,
    cache_blend_layout: wgpu::BindGroupLayout,
    blend_params: wgpu::Buffer,
    blend_dispatch: wgpu::Buffer,
    blend_force: std::sync::atomic::AtomicBool,
    /// The chunk sweeps requested: the walk takes a new one when it moves.
    sweep_requests: std::sync::atomic::AtomicU32,
    /// The table's edit-box reset by cells (`clear_box_cells`), for a frame whose boxes' voxel
    /// faces outnumber the cells; built on its first use.
    cache_clear_cells: std::sync::OnceLock<wgpu::ComputePipeline>,
    /// The camera's chunk at the last encode, `None` before the first: the sweep runs when it
    /// changes.
    sweep_chunk: std::sync::Mutex<Option<[i32; 3]>>,
    /// Workgroups a dispatch row of the table's passes takes at most ([`MAX_DISPATCH_GROUPS`];
    /// a test sets fewer, [`Frame::set_table_dispatch_cap`]).
    table_dispatch_cap: u32,
    /// The guard runs in every frame, its window ignored ([`Frame::force_guard_window`]).
    guard_forced: bool,
    probe_spacing: u32,
    /// The probe volume's side, if fixed ([`Frame::set_probe_axis`]).
    probe_axis_fixed: Option<u32>,
    device: wgpu::Device,
    probe_layout: wgpu::BindGroupLayout,
    gather_layout: wgpu::BindGroupLayout,
    gather: wgpu::ComputePipeline,
    /// The probe update's two dispatches (`probe_update.wgsl`).
    probe_trace: wgpu::ComputePipeline,
    probe_blend: wgpu::ComputePipeline,
    /// Probe rays the last encode dispatched: update-list entries times rays per probe.
    traced_probe_rays: std::sync::atomic::AtomicU64,
    /// Whether the last encode's probe update blended inside a light-change window.
    probe_light_window: std::sync::atomic::AtomicBool,
    /// Whether the last encode with history set [`LIGHTING_LAMP_CHANGED`].
    lamp_changed: std::sync::atomic::AtomicBool,
    /// The output size the frame's image is presented at; the targets are at the internal
    /// size.
    output: [u32; 2],
    /// The exposure pass, its state and its read-back ring (ADR-0033).
    exposure: ExposurePass,
    metering: Metering,
    /// The exposure pass's step in seconds ([`Frame::set_dt`]).
    dt: f32,
    /// The next encode's exposure pass jumps the adapted level to the meter.
    reset_exposure: std::sync::atomic::AtomicBool,
    /// The views' rays are jittered by [`jitter_at`] of the frame index.
    jitter: bool,
    /// The upscaler the frame runs; with FSR 2, its context and the composite's variant
    /// that stores linear radiance (`HDR_TARGET` in composite.wgsl), built with it.
    kind: UpscalerKind,
    fsr: Option<Fsr2>,
    composite_hdr: Option<(wgpu::BindGroupLayout, wgpu::ComputePipeline)>,
    /// A copy of the composite's image at the internal size, taken right after the composite
    /// pass ([`Frame::set_composite_capture`]); `None` unless a test or tool asked for it.
    composite_capture: Option<wgpu::Texture>,
}

/// The emissive table's buffer: `cells` cells, then the blend list's header
/// ([`BLEND_HEADER_BYTES`]) and `list` slots of 4 B (`cache_blend.wgsl`).
fn table_buffer(device: &wgpu::Device, cells: u32, list: u32) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("emissive table"),
        size: table_bytes(cells, list),
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// The cells a device binds: `want`, halved until the table with its `list` slots fits the
/// device's storage binding size (lavapipe's is 128 MiB, under the default table).
pub fn table_cells_within(limit: u64, want: u32, list: u32) -> u32 {
    let mut cells = want;
    while cells > 64 && table_bytes(cells, list) > limit {
        cells /= 2;
    }
    cells
}

/// [`table_buffer`]'s bytes.
pub fn table_bytes(cells: u32, list: u32) -> u64 {
    u64::from(cells) * CACHE_CELL_WORDS * 4 + BLEND_HEADER_BYTES + 4 * u64::from(list)
}

/// The blend list's header past the table's cells: the entries listed this frame, the dirty
/// word, the last frame's and a word of padding.
pub const BLEND_HEADER_BYTES: u64 = 16;

/// A dispatch of `groups` workgroups in rows of at most `cap`: `[x, y]` with `x * y >= groups`.
pub fn dispatch_rows(groups: u32, cap: u32) -> [u32; 2] {
    let cap = cap.max(1);
    if groups <= cap {
        [groups, 1]
    } else {
        [cap, groups.div_ceil(cap)]
    }
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
    /// The lamps of the last encodes with history, newest last, at most [`LAMP_FRAMES`].
    lamps: std::collections::VecDeque<Lamp>,
    /// The static light of the last encode with history (`None` before the first).
    light_key: Option<LightKey>,
    /// Encoded frames with a probe update left in the open light-change window.
    light_window: u32,
    /// The frame of the last change that opens the guard's window ([`GUARD_WINDOW`]), and the
    /// mirror's `bytes_written` at the last encode with history.
    guard_event: Option<u32>,
    guard_bytes: Option<u64>,
}

/// What a frame's static light depends on, as the probe volume's light-change window compares
/// it between encodes: the lamp's switch and its parameters (not `masks` or `skips`, which
/// leave the walk byte-identical, and not its pose, which follows the camera), the shadow walk
/// the lamp's shadow rays take. Exposure and the tone map come after the light.
#[derive(Clone, Copy, Debug, PartialEq)]
struct LightKey {
    lamp: bool,
    params: LampKey,
    shadow_walk: ShadowWalk,
}

/// The lamp's parameters as a frame's [`Params`] carry them: its camera-relative pose
/// ([`Params::lamp_pose`]), its intensity, its cone's cosines and its shadow switch. `masks`
/// and `skips` leave the walk byte-identical and are not part of it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct LampKey {
    pose: [f32; 3],
    intensity: f32,
    cos_inner: f32,
    cos_outer: f32,
    shadows: bool,
}

impl LampKey {
    fn of(p: &Params) -> Self {
        Self {
            pose: p.lamp_pose,
            intensity: p.light_intensity,
            cos_inner: p.cos_inner,
            cos_outer: p.cos_outer,
            shadows: p.flags & FLAG_SHADOWS != 0,
        }
    }
}

impl LightKey {
    /// Whether the lamp's light differs between `self` and `other`: its switch, its
    /// parameters or its shadow rays' walk.
    fn lamp_differs(&self, other: &Self) -> bool {
        self.lamp != other.lamp
            || self.params != other.params
            || self.shadow_walk != other.shadow_walk
    }
}

/// Entries the candidate table holds at most.
pub const LIGHT_TABLE_ENTRIES: usize = 1024;

/// The light pass's candidate table as written past the light-sample reservation
/// (`GpuWorld::light_table_base`): a header slot `count, total (f32 bits), 0, 0`, then per
/// entry `start, len`, the cumulative weight (f32 bits) and a pad.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct LightTableUniform {
    pub(crate) count: u32,
    pub(crate) total: f32,
    pub(crate) _pad: [u32; 2],
    pub(crate) chunks: [[u32; 4]; LIGHT_TABLE_ENTRIES],
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
    /// A read-write storage buffer.
    Storage,
    /// A read-only storage buffer.
    ReadStorage,
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
            Io::Uniform | Io::Storage | Io::ReadStorage => wgpu::BindGroupLayoutEntry {
                binding: b,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: if matches!(k, Io::Uniform) {
                        wgpu::BufferBindingType::Uniform
                    } else {
                        wgpu::BufferBindingType::Storage {
                            read_only: matches!(k, Io::ReadStorage),
                        }
                    },
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
                storage(7),
                storage(8),
                storage(9),
                storage(10),
                storage(11),
                sampled_texture(12, FLOAT),
                wgpu::BindGroupLayoutEntry {
                    binding: 13,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
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
        // The light pass's pipeline alone (`shade`) is built on the first encode that runs it
        // (the preload off): with the preload on, `light_preload` runs the pass.
        let light_layout = io_layout(
            device,
            "light",
            &[
                Io::Float,
                Io::Uint,
                f16,
                Io::Storage,
                Io::Storage,
                Io::Uniform,
            ],
        );
        // The reopened fill: light.wgsl's table and counters (which hold the list) at their
        // bindings in group 2; the args pass writes the indirect dispatch through group 3.
        let fill_layout = io_layout_at(device, "fill", &[(3, Io::Storage), (4, Io::Storage)]);
        // The args pass binds the counters alone in group 2 (the world's storage buffers and
        // the table's would pass `max_storage_buffers_per_shader_stage`'s 11).
        let fill_stats_layout = io_layout_at(device, "fill stats", &[(4, Io::Storage)]);
        let fill_args_layout = io_layout(device, "fill args", &[Io::Storage]);
        let fill_source = format!("{MARCH_WGSL}\n{LIGHT_WGSL}\n{CACHE_FILL_WGSL}");
        let fill = compute_pipeline(
            device,
            "fill",
            fill_source.clone(),
            &[Some(&world_layout), None, Some(&fill_layout)],
            "fill",
        );
        let fill_args = compute_pipeline(
            device,
            "fill args",
            fill_source,
            &[
                Some(&world_layout),
                None,
                Some(&fill_stats_layout),
                Some(&fill_args_layout),
            ],
            "fill_args",
        );
        let fill_dispatch = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fill dispatch"),
            size: FILL_DISPATCH_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT,
            mapped_at_creation: false,
        });
        // The preload: the fill's group 2 with its uniform at group 3 binding 1; the args pass
        // binds the counters alone in group 2, as the fill's does, and the dispatches at 2.
        let preload_layout = io_layout_at(device, "preload", &[(1, Io::Uniform)]);
        let preload_args_layout = io_layout_at(
            device,
            "preload args",
            &[(1, Io::Uniform), (2, Io::Storage)],
        );
        let preload_source = format!("{MARCH_WGSL}\n{LIGHT_WGSL}\n{CACHE_FILL_WGSL}");
        let preload_args = compute_pipeline(
            device,
            "preload args",
            preload_source.clone(),
            // No world: the table, the counters and the dispatches' buffer are three of the
            // stage's storage buffers.
            &[None, None, Some(&fill_layout), Some(&preload_args_layout)],
            "preload_args",
        );
        let preload_sweep = compute_pipeline(
            device,
            "preload sweep",
            preload_source.clone(),
            &[
                Some(&world_layout),
                None,
                Some(&fill_layout),
                Some(&preload_layout),
            ],
            "preload_sweep",
        );
        let light_preload = compute_pipeline(
            device,
            "light and preload",
            preload_source,
            &[
                Some(&world_layout),
                None,
                Some(&light_layout),
                Some(&preload_layout),
            ],
            "shade_preload",
        );
        let preload_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("preload"),
            size: size_of::<PreloadUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let preload_dispatch = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("preload dispatch"),
            size: PRELOAD_DISPATCH_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT,
            mapped_at_creation: false,
        });
        let (cache_clear_layout, cache_clear) = pass(
            "cache clear",
            CACHE_CLEAR_WGSL,
            "clear_cells",
            &[Io::Uniform, Io::Storage],
        );
        // The walk binds no world: its table, counters, state and dispatch are four of the
        // stage's storage buffers.
        let cache_sweep_layout = io_layout(
            device,
            "cache walk",
            &[
                Io::Uniform,
                Io::Storage,
                Io::Storage,
                Io::Storage,
                Io::Storage,
            ],
        );
        // The slice reads no dispatch words: the buffer is its indirect argument there.
        let cache_walk_layout = io_layout(
            device,
            "cache walk slice",
            &[Io::Uniform, Io::Storage, Io::Storage, Io::Storage],
        );
        let walk_source = format!("{CACHE_CELL_WGSL}\n{CACHE_SWEEP_WGSL}");
        let cache_walk_args = compute_pipeline(
            device,
            "cache walk args",
            walk_source.clone(),
            &[None, None, Some(&cache_sweep_layout)],
            "walk_args",
        );
        let cache_sweep = compute_pipeline(
            device,
            "cache walk",
            walk_source,
            &[None, None, Some(&cache_walk_layout)],
            "walk_cells",
        );
        let cache_blend_layout = io_layout(device, "cache blend", &[Io::Uniform, Io::Storage]);
        let cache_blend_args_layout = io_layout(
            device,
            "cache blend args",
            &[Io::Uniform, Io::Storage, Io::Storage],
        );
        let cache_blend_args = compute_pipeline(
            device,
            "cache blend args",
            format!("{CACHE_CELL_WGSL}\n{CACHE_BLEND_WGSL}"),
            &[None, None, Some(&cache_blend_args_layout)],
            "blend_args",
        );
        let blend_dispatch = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("emissive table blend dispatch"),
            size: 12,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT,
            mapped_at_creation: false,
        });
        let cache_blend = compute_pipeline(
            device,
            "cache blend",
            format!("{CACHE_CELL_WGSL}\n{CACHE_BLEND_WGSL}"),
            &[None, None, Some(&cache_blend_layout)],
            "blend_cells",
        );
        let blend_params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("emissive table blend"),
            size: size_of::<BlendUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let walk_state = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("emissive table walk"),
            size: WALK_STATE_BYTES,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let walk_dispatch = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("emissive table walk dispatch"),
            size: 12,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT,
            mapped_at_creation: false,
        });
        let targets = Targets::new(
            device,
            width,
            height,
            LightingResolution::default().scale(),
            (internal != output).then_some(output),
            true,
        );
        let [lw, lh] = targets.light_size();
        let table_cells = table_cells_within(
            device.limits().max_storage_buffer_binding_size,
            DEFAULT_TABLE_CELLS,
            lw * lh,
        );
        let cache = table_buffer(device, table_cells, lw * lh);
        let cache_stats = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("emissive table counters"),
            size: CACHE_STATS_BYTES + FILL_LIST_BYTES + PRELOAD_BYTES,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sweep_params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("emissive table sweep"),
            size: size_of::<SweepUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
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
                Io::Storage,
                Io::Storage,
            ],
        );
        let gradient_layout = io_layout(device, "gradient", &[Io::Float, Io::Uint, Io::Float, f16]);
        // The guard's group 2: light.wgsl's table and counters at their bindings there.
        let gradient_cache_layout = io_layout_at(
            device,
            "gradient cache",
            &[(3, Io::Storage), (4, Io::Storage)],
        );
        let gradient = compute_pipeline(
            device,
            "gradient",
            format!("{MARCH_WGSL}\n{LIGHT_WGSL}\n{GRADIENT_WGSL}"),
            &[
                Some(&world_layout),
                None,
                Some(&gradient_cache_layout),
                Some(&gradient_layout),
            ],
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
                (7, Io::Float),
                (8, Io::Float),
            ],
        );
        let atrous = compute_pipeline(
            device,
            "atrous",
            format!("{MARCH_WGSL}\n{ATROUS_WGSL}"),
            &[Some(&world_layout), None, Some(&atrous_layout)],
            "atrous",
        );
        let steps = [0usize, 1, 2, 3].map(|i| {
            wgpu::util::DeviceExt::create_buffer_init(
                device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("atrous step"),
                    contents: bytemuck::cast_slice(&[
                        1u32 << i,
                        u32::from(i + 1 == ATROUS_ITERATIONS),
                        0,
                        0,
                    ]),
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
                Io::Uniform,
            ],
        );
        let sharpen_layout = io_layout(
            device,
            "sharpen",
            &[
                Io::Float,
                Io::Store(wgpu::TextureFormat::Rgba8Unorm),
                Io::Uniform,
            ],
        );
        let sharpen = compute_pipeline(
            device,
            "sharpen",
            format!("{MARCH_WGSL}\n{SHARPEN_WGSL}"),
            &[Some(&world_layout), None, None, Some(&sharpen_layout)],
            "sharpen_main",
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
                Io::Float,
                Io::Storage,
                Io::Uint,
            ],
        );
        let probe_storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let probe_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("probes"),
            entries: &[
                probe_storage(0),
                probe_storage(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let gather_layout = io_layout(device, "gather", &[Io::Float, Io::Uint, f16]);
        let gather = compute_pipeline(
            device,
            "gather",
            format!("{MARCH_WGSL}\n{PROBE_GATHER_WGSL}"),
            &[
                Some(&world_layout),
                None,
                Some(&gather_layout),
                Some(&probe_layout),
            ],
            "gather",
        );
        let update_source =
            format!("{MARCH_WGSL}\n{LIGHT_WGSL}\n{PROBE_GATHER_WGSL}\n{PROBE_UPDATE_WGSL}");
        let probe_pass = |entry: &str| {
            compute_pipeline(
                device,
                entry,
                update_source.clone(),
                &[Some(&world_layout), None, None, Some(&probe_layout)],
                entry,
            )
        };
        let probe_trace = probe_pass("probe_trace");
        let probe_blend = probe_pass("probe_blend");
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
            fill,
            fill_args,
            fill_layout,
            fill_stats_layout,
            fill_args_layout,
            fill_dispatch,
            preload_args,
            light_preload,
            preload_sweep,
            preload_epoch: std::sync::Mutex::new((0, None)),
            preload_layout,
            preload_args_layout,
            preload_uniform,
            preload_dispatch,
            preload_offsets_radius: std::sync::Mutex::new(None),
            preload_edit_bricks: std::sync::atomic::AtomicU32::new(0),
            preload_ran: std::sync::atomic::AtomicBool::new(false),
            preload_idle: std::sync::Mutex::new(PreloadIdle::new(device)),
            preload_skips: std::sync::atomic::AtomicU32::new(0),
            light: std::sync::OnceLock::new(),
            temporal_layout,
            store: crate::bake::Store::new(device, crate::bake::MINIMAL_POOL_BYTES),
            bake: None,
            dig_boxes: Vec::new(),
            temporal,
            gradient_layout,
            gradient_cache_layout,
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
            targets,
            resolution: LightingResolution::default(),
            lod: LodParams::default(),
            look: LookParams::default(),
            field_split: true,
            lighting: Lighting::default(),
            emission_scale: wgpu::util::DeviceExt::create_buffer_init(
                device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("emission scale"),
                    contents: bytemuck::cast_slice(&vec![1.0f32; 2 * EMISSION_ROWS]),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                },
            ),
            emission_cpu: vec![1.0; 2 * EMISSION_ROWS],
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
            probe_channel_ran: std::sync::atomic::AtomicBool::new(false),
            probe_dispatches: std::sync::atomic::AtomicU32::new(0),
            cur: 0,
            downsample_layout,
            downsample,
            upscale_layout,
            upscale,
            sharpen_layout,
            sharpen,
            sharpen_params: wgpu::util::DeviceExt::create_buffer_init(
                device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("sharpen"),
                    contents: bytemuck::cast_slice(&[0.0f32; 4]),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                },
            ),
            sharpness: 0.0,
            upscale_params: wgpu::util::DeviceExt::create_buffer_init(
                device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("upscale tuning"),
                    contents: bytemuck::bytes_of(
                        &Upscaler::default().uniform(1.0, Lamp::default()),
                    ),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                },
            ),
            upscaler: Upscaler::default(),
            history: std::sync::Mutex::new(History::default()),
            output,
            jitter: false,
            kind: UpscalerKind::Off,
            fsr: None,
            composite_hdr: None,
            composite_capture: None,
            probes: std::sync::OnceLock::new(),
            cache,
            table_cells,
            cache_blend_args,
            cache_blend_args_layout,
            cache_blend,
            cache_blend_layout,
            blend_params,
            blend_dispatch,
            blend_force: std::sync::atomic::AtomicBool::new(false),
            cache_stats,
            // The new table's words are zero on creation; clear them once anyway with the
            // counters.
            cache_clear_requested: std::sync::atomic::AtomicBool::new(true),
            cache_clears: std::sync::atomic::AtomicU32::new(0),
            cache_sweeps: std::sync::atomic::AtomicU32::new(0),
            cache_clear,
            cache_clear_layout,
            cache_sweep,
            cache_sweep_layout,
            cache_walk_args,
            cache_walk_layout,
            sweep_params,
            walk_state,
            walk_dispatch,
            sweep_requests: std::sync::atomic::AtomicU32::new(0),
            cache_clear_cells: std::sync::OnceLock::new(),
            sweep_chunk: std::sync::Mutex::new(None),
            table_dispatch_cap: MAX_DISPATCH_GROUPS,
            guard_forced: false,
            probe_spacing: DEFAULT_PROBE_SPACING,
            probe_axis_fixed: None,
            device: device.clone(),
            probe_layout,
            gather_layout,
            gather,
            probe_trace,
            probe_blend,
            traced_probe_rays: std::sync::atomic::AtomicU64::new(0),
            probe_light_window: std::sync::atomic::AtomicBool::new(false),
            lamp_changed: std::sync::atomic::AtomicBool::new(false),
            exposure: ExposurePass::new(device),
            metering: Metering::default(),
            dt: 1.0 / 60.0,
            reset_exposure: std::sync::atomic::AtomicBool::new(true),
        }
    }

    /// Sets the frame's step in seconds (1/60 by default): the time the exposure pass's
    /// adapted level moves over in each encoded frame, and FSR 2's `delta_time`. A negative or
    /// non-finite `dt` is 0, which holds the exposure.
    pub fn set_dt(&mut self, seconds: f32) {
        self.dt = if seconds.is_finite() {
            seconds.max(0.0)
        } else {
            0.0
        };
    }

    pub fn dt(&self) -> f32 {
        self.dt
    }

    /// Makes the next frame's exposure pass that meters jump the adapted level to the meter
    /// (a new frame starts so).
    pub fn reset_exposure(&mut self) {
        self.reset_exposure
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Sets how the exposure pass weights the meter's tiles under a manual exposure
    /// (centre-weighted by default); in auto, [`crate::AutoExposure::metering`] does.
    pub fn set_metering(&mut self, metering: Metering) {
        self.metering = metering;
    }

    pub fn metering(&self) -> Metering {
        self.metering
    }

    /// The exposure state as last read back, without waiting: a non-blocking poll of
    /// `device`, then the newest of the read-back ring's slots that has finished mapping
    /// (two or three frames late), else the last one read; `None` until the first. Each
    /// encode's commands must be submitted before the next encode or this call.
    pub fn exposure_readout(&self, device: &wgpu::Device) -> Option<ExposureReadout> {
        self.exposure.readout(device)
    }

    /// The exposure state buffer the exposure pass writes (32 bytes, `COPY_SRC`:
    /// [`crate::exposure::ExposureState::from_bytes`] reads a copy), for tests and tools.
    pub fn exposure_state(&self) -> &wgpu::Buffer {
        self.exposure.state()
    }

    /// The meter's tiles the composite writes (`COPY_SRC`): per 8 x 8 workgroup of the
    /// internal target, row-major over `ceil(w / 8) x ceil(h / 8)`, the sum of its pixels'
    /// clamped log2 luminance and their count, as two `f32`.
    pub fn exposure_tiles(&self) -> &wgpu::Buffer {
        &self.targets.tiles
    }

    /// Probe rays the last encode dispatched (update-list entries times
    /// [`Lighting::probe_rays`]); 0 with the channel or the lamp off, or the update off.
    pub fn traced_probe_rays(&self) -> u64 {
        self.traced_probe_rays
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The probe trace, blend and gather dispatches the last encode recorded: 0 with the lamp
    /// off or the channel off.
    pub fn probe_dispatches(&self) -> u32 {
        self.probe_dispatches
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Whether the last encode's probe update ran inside a light-change window: the channel
    /// starting (the lamp switched on, the channel turned on, the volume rebuilt) or a change of
    /// the lamp's parameters or shadow walk clears every probe's state and opens one for
    /// [`crate::probes::WINDOW_FRAMES`] encoded frames with a probe update; a change inside it
    /// restarts it. In the window the update refreshes one probe in
    /// [`crate::probes::WINDOW_SLICES`] a frame, so the channel fills from zero faster.
    pub fn probe_light_window(&self) -> bool {
        self.probe_light_window
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Whether the last encode with history reset the lamp's history
    /// ([`LIGHTING_LAMP_CHANGED`]): the lamp's switch, its parameters or its shadow walk
    /// differ from the encode before.
    pub fn lamp_changed(&self) -> bool {
        self.lamp_changed.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The probe volume (ADR-0023), built here if no encode with the lamp channel running, no
    /// [`Frame::probes_mut`] and no spacing change has built it yet, at
    /// [`Frame::probe_axis`] probes a side. Until then the frame holds none of its buffers.
    pub fn probes(&self) -> &ProbeVolume {
        self.probes
            .get_or_init(|| ProbeVolume::new(&self.device, self.probe_spacing, self.probe_axis()))
    }

    /// The probe volume's side: the one [`Frame::set_probe_axis`] fixed, or the smallest power
    /// of two whose half covers [`Lighting::lamp_reach`] at the spacing
    /// ([`crate::probes::probes_per_axis`]).
    pub fn probe_axis(&self) -> u32 {
        self.probe_axis_fixed.unwrap_or_else(|| {
            crate::probes::probes_per_axis(self.lighting.lamp_reach, self.probe_spacing)
        })
    }

    /// Fixes the probe volume's side at `axis` probes (a power of two in 4 ..= 32), or with
    /// `None` lets it follow the lamp's reach; a built volume of another side is rebuilt as
    /// [`Frame::set_probe_spacing`] does.
    pub fn set_probe_axis(&mut self, axis: Option<u32>) {
        self.probe_axis_fixed = axis;
        self.rebuild_probes();
    }

    /// Replaces a built probe volume whose spacing or side differs from the frame's, every
    /// probe never traced; the old volume's pinned origin and switches carry over, and it is
    /// dropped before the new one is built.
    fn rebuild_probes(&mut self) {
        let (spacing, axis) = (self.probe_spacing, self.probe_axis());
        if self
            .probes
            .get()
            .is_none_or(|v| v.spacing() == spacing && v.axis() == axis)
        {
            return;
        }
        let old = self.probes.take();
        let carried = old.as_ref().map(|o| (o.pinned_origin(), o.updates()));
        drop(old);
        let mut v = ProbeVolume::new(&self.device, spacing, axis);
        if let Some((origin, updates)) = carried {
            v.set_origin(origin);
            v.set_updates(updates);
        }
        self.probe_channel_ran
            .store(false, std::sync::atomic::Ordering::Relaxed);
        let _ = self.probes.set(v);
    }

    /// The probe volume, built here as [`Frame::probes`] does.
    pub fn probes_mut(&mut self) -> &mut ProbeVolume {
        self.probes();
        self.probes.get_mut().expect("built by probes()")
    }

    /// Replaces the probe volume with one of probes `spacing` voxels apart, every probe never
    /// traced, when the spacing changes (its side follows, see [`Frame::probe_axis`]); the old
    /// volume's pinned origin and switches carry over, and it is dropped before the new one is
    /// built. A volume not yet built is built at the new spacing on first use.
    pub fn set_probe_spacing(&mut self, _device: &wgpu::Device, spacing: u32) {
        self.probe_spacing = spacing;
        self.rebuild_probes();
    }

    /// The gather's indirect irradiance on the lighting grid (rgba16float, rgb).
    pub fn indirect_irradiance(&self) -> &wgpu::Texture {
        &self.targets.indirect
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

    /// Sets the sharpening pass's strength in `0..=1` (clamped): 0 (the default) writes the
    /// upscale pass's output unchanged, 1 is RCAS at full strength (0 stops), and between them
    /// `2 (1 - s)` stops (`con = 2^-stops`). It runs with the upscaler.
    pub fn set_sharpness(&mut self, queue: &wgpu::Queue, sharpness: f32) {
        self.sharpness = sharpness.clamp(0.0, 1.0);
        let con = if self.sharpness > 0.0 {
            (-2.0 * (1.0 - self.sharpness)).exp2()
        } else {
            0.0
        };
        queue.write_buffer(
            &self.sharpen_params,
            0,
            bytemuck::cast_slice(&[con, 0.0, 0.0, 0.0]),
        );
    }

    pub fn sharpness(&self) -> f32 {
        self.sharpness
    }

    /// Sets the upscale pass's reconstruction, history fetch and blend from the next frame on.
    pub fn set_upscaler(&mut self, upscaler: Upscaler) {
        self.upscaler = upscaler;
    }

    pub fn upscaler(&self) -> Upscaler {
        self.upscaler
    }

    /// Whether an upscaler runs: FSR 2, or the frame jitters or renders below its output
    /// size. Without one the composite's image is the frame's, byte for byte.
    pub fn upscaling(&self) -> bool {
        self.fsr.is_some() || self.jitter || self.internal_size() != self.output
    }

    /// Selects the upscaler: `Off` turns the jitter off (at equal sizes no upscaler runs),
    /// `Own` sets the tuning of ADR-0024's upscaler (the jitter as [`Frame::set_jitter`] left
    /// it), and `Fsr2` runs FSR 2 with its own jitter sequence. A change of upscaler replaces
    /// the targets and drops the history; a change of FSR's settings alone keeps them.
    pub fn set_upscaler_kind(&mut self, device: &wgpu::Device, kind: UpscalerKind) {
        let was_fsr = self.fsr.is_some();
        match kind {
            UpscalerKind::Off => self.jitter = false,
            UpscalerKind::Own(u) => self.upscaler = u,
            UpscalerKind::Fsr2(s) => {
                if let Some(f) = self.fsr.as_mut() {
                    f.set_settings(s);
                } else {
                    self.fsr = Some(Fsr2::new(
                        device,
                        &self.world_layout,
                        self.internal_size(),
                        self.output,
                        s,
                    ));
                }
                if self.composite_hdr.is_none() {
                    self.composite_hdr = Some(composite_hdr_pass(device, &self.world_layout));
                }
            }
        }
        if !kind.is_fsr2() {
            self.fsr = None;
        }
        let changed = std::mem::discriminant(&kind) != std::mem::discriminant(&self.kind)
            || was_fsr != self.fsr.is_some();
        self.kind = kind;
        if changed {
            self.set_sizes(device, self.internal_size(), self.output);
        }
    }

    pub fn upscaler_kind(&self) -> UpscalerKind {
        self.kind
    }

    /// FSR 2's context while the frame runs it.
    pub fn fsr2(&self) -> Option<&Fsr2> {
        self.fsr.as_ref()
    }

    pub fn fsr2_mut(&mut self) -> Option<&mut Fsr2> {
        self.fsr.as_mut()
    }

    /// The copy of the double-buffered targets the last (or next) encode writes.
    pub fn current_copy(&self) -> usize {
        self.cur
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
    /// [`MAX_EDITS`] boxes in one frame flush all history. With the lamp channel running, the
    /// next encoded frame also restarts every probe within the lamp's reach whose cell the box
    /// touches ([`ProbeVolume::touched`]): it blends at hysteresis 0, its age at 1; boxes past
    /// [`MAX_EDITS`] touch no probe, which then refresh in their slices.
    /// A store holding a piece also keeps the box for [`Frame::rebake_digs`].
    pub fn invalidate(&mut self, min: [i32; 3], max: [i32; 3]) {
        if self.store.piece().is_some() {
            self.dig_boxes.push([min, max]);
        }
        let h = self.history();
        if h.edits.len() == MAX_EDITS {
            h.overflow = true;
        } else {
            h.edits.push([min, max]);
        }
    }

    /// Drops the upscaler's history alone in the next encoded frame (the lighting's is kept),
    /// as a change of view does.
    pub fn reset_upscaler_history(&mut self) {
        self.history().view_changed = true;
    }

    /// Drops all history: the next encoded frame starts every pixel afresh.
    pub fn reset_history(&mut self) {
        self.history().valid = false;
        self.cache_clear_requested
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// The emissive table (`COPY_SRC`; [`CACHE_CELL_WORDS`] `u32`s a cell, laid out as
    /// `march.wgsl` says).
    pub fn emissive_table(&self) -> &wgpu::Buffer {
        &self.cache
    }

    /// The emissive table's counters after the last submitted encode: a blocking read.
    pub fn read_table_stats(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> TableStats {
        let bytes = crate::readback::read_buffer(device, queue, &self.cache_stats, 0, 16);
        let w: &[u32] = bytemuck::cast_slice(&bytes);
        TableStats {
            cells: self.table_cells(),
            occupied: w[1],
            misses: w[0],
        }
    }

    /// The table's age-out and sweep after the last submitted encode: a blocking read.
    pub fn read_age_stats(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> AgeStats {
        let bytes = crate::readback::read_buffer(device, queue, &self.walk_state, 0, 64);
        let w: &[u32] = bytemuck::cast_slice(&bytes);
        AgeStats {
            passes: w[11],
            evicted: w[13],
            evicted_frame: w[14],
            swept: w[15],
            age: w[6],
            walk_left: w[1],
        }
    }

    /// The whole-table clears (the first frame's, an emission-scale change, a lighting change
    /// that drops it, an overflowed edit frame) and the sweeps encoded since the
    /// frame was made.
    pub fn table_clears_and_sweeps(&self) -> (u32, u32) {
        use std::sync::atomic::Ordering::Relaxed;
        (
            self.cache_clears.load(Relaxed),
            self.cache_sweeps.load(Relaxed),
        )
    }

    /// The guard's reopens since the table was last cleared by branch (a kept emitter's
    /// re-shade out of the dead zone, a torn tag whose key lights nothing, a dark capped cell
    /// whose fresh sample drew light), and the last re-shade reopen's kept and new
    /// luminances. A blocking read.
    pub fn read_table_reopens_by(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> ([u32; 3], [f32; 2]) {
        let bytes = crate::readback::read_buffer(device, queue, &self.cache_stats, 0, 32);
        let w: &[u32] = bytemuck::cast_slice(&bytes);
        (
            [w[4], w[5], w[6]],
            [f32::from_bits(w[3]), f32::from_bits(w[7])],
        )
    }

    /// The edit-box reset by cells (`clear_box_cells`), built on its first use.
    fn cache_clear_cells_pipeline(&self, device: &wgpu::Device) -> &wgpu::ComputePipeline {
        self.cache_clear_cells.get_or_init(|| {
            compute_pipeline(
                device,
                "cache clear by cells",
                format!("{MARCH_WGSL}\n{CACHE_CLEAR_WGSL}"),
                &[
                    Some(&self.world_layout),
                    None,
                    Some(&self.cache_clear_layout),
                ],
                "clear_box_cells",
            )
        })
    }

    /// Group 0 of the frame's shaders over `world`.
    fn world_bind(&self, device: &wgpu::Device, world: &GpuWorld) -> wgpu::BindGroup {
        let noise = view(&self.blue_noise);
        device.create_bind_group(&wgpu::BindGroupDescriptor {
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
                buffer_entry(10, world.light_buffer()),
                buffer_entry(11, &self.emission_scale),
                wgpu::BindGroupEntry {
                    binding: 12,
                    resource: wgpu::BindingResource::TextureView(&noise),
                },
                buffer_entry(13, self.exposure.state_uniform()),
            ],
        })
    }

    /// Bakes every chunk of `extent` into the store, emptied first, before any frame reads it,
    /// blocking: submits of at most [`crate::bake::SUBMIT_SAMPLES`] samples, each waited on, with
    /// `progress` after each. The light-sample buffer's ranges are first held and written out to
    /// the bake's reach. `world` is the voxels `gpu` mirrors, which `gpu` must hold already.
    /// `Err` without [`Light::Baked`], or when the pool or the light samples cannot hold the
    /// extent.
    #[allow(
        clippy::too_many_arguments,
        reason = "the device and queue, the two forms of the world, the extent, the settings and the progress are all independent"
    )]
    pub fn bake_until(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu: &mut GpuWorld,
        world: &vxe_world::World,
        extent: vxe_world::ChunkExtent,
        settings: &crate::bake::BakeSettings,
        progress: &mut dyn FnMut(&crate::bake::BakeProgress),
    ) -> Result<crate::bake::BakeReport, crate::bake::BakeError> {
        if self.lighting.light != Light::Baked {
            return Err(crate::bake::BakeError::NotBaked);
        }
        hold_reach(device, queue, gpu, world, extent)?;
        self.dig_boxes.clear();
        let params = self.bake_params(gpu, settings);
        let world_bind = self.world_bind(device, gpu);
        if self.bake.is_none() {
            self.bake = Some(crate::bake::BakePipeline::new(device, &self.world_layout));
        }
        self.bake
            .as_mut()
            .expect("built above")
            .ensure(device, settings.form);
        let run = crate::bake::BakeRun {
            device,
            queue,
            gpu,
            world,
            pipeline: self.bake.as_ref().expect("built above"),
            world_bind: &world_bind,
            params_buffer: &self.params,
            params,
        };
        crate::bake::run(&run, &mut self.store, extent, settings, progress)
    }

    /// The frame parameters a bake dispatches with: no view but the LOD walk's camera chunk,
    /// which each face takes from its job.
    fn bake_params(&self, gpu: &GpuWorld, settings: &crate::bake::BakeSettings) -> Params {
        let camera = Camera {
            position: [0.0, 0.0, 0.0].into(),
            yaw: 0.0,
            pitch: 0.0,
            vfov: 1.0,
        };
        let mut params = self.params(gpu, &camera, &LightParams::default());
        params.flags = FLAG_SHADOWS | FLAG_MASKS | FLAG_SKIPS;
        params.lighting = 0;
        params.cache = 0;
        params.fill = 0;
        params.history = 0;
        params.forced_level = 0;
        params.candidates = settings.candidates;
        params
    }

    /// Whether edit boxes wait for [`Frame::rebake_digs`].
    pub fn digs_pending(&self) -> bool {
        !self.dig_boxes.is_empty()
    }

    /// Re-bakes the ring of every edit box since the last call (ADR-0046's rings 1 and 2,
    /// [`crate::bake::dig_ring`]) to `settings`' stop, blocking, ahead of the next encode: the
    /// light-sample ranges are first held out to the piece's reach from the focus and synced.
    /// `gpu` must already mirror the edits. `None` when no box waits or the store holds no piece.
    pub fn rebake_digs(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu: &mut GpuWorld,
        world: &vxe_world::World,
        settings: &crate::bake::BakeSettings,
        progress: &mut dyn FnMut(&crate::bake::BakeProgress),
    ) -> Option<Result<crate::bake::DigReport, crate::bake::BakeError>> {
        if self.dig_boxes.is_empty() {
            return None;
        }
        let piece = self.store.piece()?;
        let boxes = std::mem::take(&mut self.dig_boxes);
        if let Err(e) = hold_reach(device, queue, gpu, world, piece) {
            return Some(Err(e));
        }
        let params = self.bake_params(gpu, settings);
        let world_bind = self.world_bind(device, gpu);
        if self.bake.is_none() {
            self.bake = Some(crate::bake::BakePipeline::new(device, &self.world_layout));
        }
        self.bake
            .as_mut()
            .expect("built above")
            .ensure(device, settings.form);
        let run = crate::bake::BakeRun {
            device,
            queue,
            gpu,
            world,
            pipeline: self.bake.as_ref().expect("built above"),
            world_bind: &world_bind,
            params_buffer: &self.params,
            params,
        };
        Some(crate::bake::rebake(
            &run,
            &mut self.store,
            &boxes,
            settings,
            progress,
        ))
    }

    /// The store file's key for the store a bake of `piece` at `settings` gives on `gpu`, with the
    /// world's seed and extent and the piece's centre and radius as the caller names them.
    pub fn store_key(
        gpu: &GpuWorld,
        world: vxe_world::ChunkExtent,
        world_seed: u64,
        piece: vxe_world::ChunkExtent,
        centre: ChunkCoord,
        radius: i32,
        settings: &crate::bake::BakeSettings,
    ) -> crate::bake::StoreKey {
        let span = |e: vxe_world::ChunkExtent| {
            let size = e.size();
            [e.min.0, [0, 1, 2].map(|k| e.min.0[k] + size[k])]
        };
        crate::bake::StoreKey {
            world_seed,
            world: span(world),
            piece: span(piece),
            centre: centre.0,
            radius,
            light_radius: gpu.light_radius(),
            candidates: settings.candidates,
            clamp: settings.clamp,
            se_bits: settings.se.to_bits(),
            n_min: settings.n_min,
            n_max: settings.n_max,
            round: settings.round,
            seed: settings.seed,
            source_hash: crate::bake::source_hash_for(settings.form),
            form: settings.form,
            tent: settings.tent,
        }
    }

    /// Writes the store's blocks under `key` to `path`.
    pub fn write_store_file(
        &self,
        path: &std::path::Path,
        key: &crate::bake::StoreKey,
    ) -> Result<u64, crate::bake::StoreFileError> {
        let bytes = self.store.to_file_bytes(key);
        std::fs::write(path, &bytes).map_err(|e| crate::bake::StoreFileError::Io(e.to_string()))?;
        Ok(bytes.len() as u64)
    }

    /// Loads `path`'s blocks into the store, emptied first, in place of a bake, where its key
    /// matches `key`; the light-sample ranges are held and synced as a bake's are, so a dig can
    /// re-bake. `Err` names the field that differs.
    pub fn load_store_file(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu: &mut GpuWorld,
        world: &vxe_world::World,
        path: &std::path::Path,
        key: &crate::bake::StoreKey,
    ) -> Result<(), crate::bake::StoreFileError> {
        if self.lighting.light != Light::Baked {
            return Err(crate::bake::StoreFileError::Mismatch("light"));
        }
        let bytes =
            std::fs::read(path).map_err(|e| crate::bake::StoreFileError::Io(e.to_string()))?;
        let image = crate::bake::Store::parse_file(key, &bytes)?;
        let piece = vxe_world::ChunkExtent::new(key.piece[0], key.piece[1]);
        hold_reach(device, queue, gpu, world, piece)
            .map_err(|e| crate::bake::StoreFileError::Io(e.to_string()))?;
        self.dig_boxes.clear();
        self.store.load_blocks(queue, gpu.window(), image)
    }

    /// Writes `image` into the store in place of what it holds, and returns what it held: one
    /// pool read by the temporal pass, several stores swapped by a key (Plan 0059 Phase 4's
    /// cut against the reference). Edit boxes waiting for [`Frame::rebake_digs`] stay. `Err`
    /// where the pool cannot hold `image`, the store unchanged.
    pub fn swap_store_image(
        &mut self,
        queue: &wgpu::Queue,
        gpu: &GpuWorld,
        image: crate::bake::StoreImage,
    ) -> Result<crate::bake::StoreImage, crate::bake::StoreFileError> {
        let held = self.store.image();
        self.store.load_blocks(queue, gpu.window(), image)?;
        Ok(held)
    }

    /// The edit boxes waiting for [`Frame::rebake_digs`], `min ..= max` in voxels.
    pub fn pending_digs(&self) -> &[[[i32; 3]; 2]] {
        &self.dig_boxes
    }

    /// Adds boxes for [`Frame::rebake_digs`], as [`Frame::invalidate`] keeps them while the store
    /// holds a piece: a swapped-in store's re-bake of the digs it missed.
    pub fn queue_digs(&mut self, boxes: &[[[i32; 3]; 2]]) {
        if self.store.piece().is_some() {
            self.dig_boxes.extend_from_slice(boxes);
        }
    }
    /// The store's counters; `missing` is a blocking read of the last submitted encode's count.
    pub fn read_store_stats(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> crate::bake::StoreStats {
        let missing = if self.lighting.light == Light::Baked {
            let bytes = crate::readback::read_buffer(device, queue, self.store.pool(), 0, 4);
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
        } else {
            0
        };
        self.store.stats(missing)
    }

    /// The store.
    pub fn store(&self) -> &crate::bake::Store {
        &self.store
    }

    /// `coord`'s block in the pool as its words, the directory first: a blocking read.
    pub fn read_store_block(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        coord: ChunkCoord,
    ) -> Option<Vec<u32>> {
        let (first, words) = self.store.block(coord)?;
        let bytes =
            crate::readback::read_buffer(device, queue, self.store.pool(), first * 4, words * 4);
        Some(bytemuck::pod_collect_to_vec(&bytes))
    }

    /// The light pass's pipeline without the preload (`shade`), built on its first use.
    fn light_pipeline(&self, device: &wgpu::Device) -> &wgpu::ComputePipeline {
        self.light.get_or_init(|| {
            compute_pipeline(
                device,
                "light",
                format!("{MARCH_WGSL}\n{LIGHT_WGSL}"),
                &[Some(&self.world_layout), None, Some(&self.light_layout)],
                "shade",
            )
        })
    }

    /// The preload's counts of the last submitted encode (zero where it did not run): a
    /// blocking read.
    pub fn read_preload_stats(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> PreloadStats {
        if !self.preload_ran.load(std::sync::atomic::Ordering::Relaxed) {
            return PreloadStats::default();
        }
        let bytes = crate::readback::read_buffer(
            device,
            queue,
            &self.cache_stats,
            CACHE_STATS_BYTES + FILL_LIST_BYTES,
            PRELOAD_STATE_BYTES,
        );
        let w: &[u32] = bytemuck::cast_slice(&bytes);
        PreloadStats {
            edit_bricks: self
                .preload_edit_bricks
                .load(std::sync::atomic::Ordering::Relaxed),
            visited: w[8],
            scanned: w[16],
            listed: w[17],
            claimed: w[18],
            claimed_near_lit: w[30],
            claimed_near_unlit: w[31],
            refused_load: w[19],
            refused_miss: w[20],
            refused_unlit: w[21],
            samples: w[22],
            drops: w[23],
            probed: w[24],
            done: w[25],
            probe_drops: w[26],
            list: w[5],
            drawn: w[6],
            probes: w[11],
        }
    }

    /// The encodes the preload's idle skip has run without the preload's passes.
    pub fn preload_skips(&self) -> u32 {
        self.preload_skips
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The cells the table's guard reopened since the table was last cleared, after the last
    /// submitted encode: a blocking read.
    pub fn read_table_reopens(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> u32 {
        let bytes = crate::readback::read_buffer(device, queue, &self.cache_stats, 8, 4);
        bytemuck::cast_slice::<u8, u32>(&bytes)[0]
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

    /// Scales material `material`'s emission by `scale` from the next frame on, in the
    /// pixels' light (and the probe rows, see [`Frame::set_emission_scales`]): a visual knob that leaves the world untouched
    /// (an authoritative change is a `Replace` op).
    pub fn set_emission_scale(&mut self, queue: &wgpu::Queue, material: u16, scale: f32) {
        self.set_emission_scales(queue, material, scale, scale);
    }

    /// Scales material `material`'s emission by `pixels` in the light the frame's pixels
    /// sample (the light pass, the composite) and by `probes` in the light the probe rays
    /// sample. An emitter that varies over time passes its current scale and its time
    /// average. The probe rows are kept for a per-source bounce channel; the lamp channel
    /// (ADR-0043) reads none of them, so `probes` changes nothing a frame shows.
    pub fn set_emission_scales(
        &mut self,
        queue: &wgpu::Queue,
        material: u16,
        pixels: f32,
        probes: f32,
    ) {
        let i = usize::from(material);
        for (row, scale) in [(i, pixels), (EMISSION_ROWS + i, probes)] {
            if self.emission_cpu[row] != scale {
                self.emission_cpu[row] = scale;
                if row == i {
                    self.emission_changed_at = Some(self.frame);
                    // The table's cells hold the old emission's light: they restart.
                    self.cache_clear_requested
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                }
                queue.write_buffer(
                    &self.emission_scale,
                    row as u64 * 4,
                    bytemuck::bytes_of(&scale),
                );
            }
        }
    }

    /// Material `material`'s emission scale in the pixels' light.
    pub fn emission_scale(&self, material: u16) -> f32 {
        self.emission_cpu[usize::from(material)]
    }

    /// Material `material`'s emission scale in the probe rays' light.
    pub fn probe_emission_scale(&self, material: u16) -> f32 {
        self.emission_cpu[EMISSION_ROWS + usize::from(material)]
    }

    /// The emission-scale buffer: an `f32` per material id for the pixels' light, then one
    /// per material id for the probe rays' (see [`Frame::set_emission_scales`]).
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

    /// Sets the surface look: normal mode and view. A change of view drops the
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

    /// Whether the field look walks split ([`FLAG_FIELD_SPLIT`], the default): the cube
    /// walk's pipeline carries each ray to the first brick the field can hit in, and the field
    /// walk goes on from there. Off, the field walk carries every ray from its origin. Either
    /// gives the same hit records; the split walks only when the world's table holds a crisp
    /// row ([`GpuWorld::crisp_rows`]), outside the cracks and steps views.
    pub fn set_field_split(&mut self, on: bool) {
        self.field_split = on;
    }

    /// Sets exposure, tone map, lamp, candidates and shadow walk.
    /// Sets the lighting knobs; a change of [`Lighting::cells`] rebuilds the emissive table
    /// empty.
    pub fn set_lighting(&mut self, lighting: Lighting) {
        let blend_changed = (lighting.blend, lighting.blend_pool_min, lighting.table_cap)
            != (
                self.lighting.blend,
                self.lighting.blend_pool_min,
                self.lighting.table_cap,
            );
        if blend_changed {
            self.blend_force
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.lighting = lighting;
        if self.lighting.light == Light::Baked {
            self.lighting.blend = TableBlend::Off;
            self.lighting.fill_samples = 0;
            self.lighting.preload_radius = 0;
            self.lighting.table_age_out = false;
        }
        self.fit_store();
        self.fit_table();
        self.rebuild_probes();
    }

    /// Rebuilds the store empty where its pool's size changes: a minimal one under
    /// [`Light::Table`], else [`Lighting::bake_pool_bytes`] within the device's storage binding.
    fn fit_store(&mut self) {
        let limits = self.device.limits();
        let bytes = match self.lighting.light {
            Light::Table if self.lighting.keep_store => self.store.pool().size(),
            Light::Table => crate::bake::MINIMAL_POOL_BYTES,
            Light::Baked => {
                self.lighting
                    .bake_pool_bytes
                    .min(limits.max_storage_buffer_binding_size)
                    .min(limits.max_buffer_size)
                    & !3
            }
        };
        if bytes != self.store.pool().size() {
            self.store = crate::bake::Store::new(&self.device, bytes);
        }
    }

    /// The table's cells as allocated: [`Lighting::cells`], or fewer where the device's storage
    /// binding cannot hold them ([`table_cells_within`]).
    pub fn table_cells(&self) -> u32 {
        self.table_cells
    }

    /// Rebuilds the emissive table empty where its cells or the lighting grid's pixels (the
    /// blend list's slots, one a lighting pixel) changed.
    fn fit_table(&mut self) {
        let [w, h] = self.targets.light_size();
        let cells = table_cells_within(
            self.device.limits().max_storage_buffer_binding_size,
            self.lighting.cells(),
            w * h,
        );
        if cells != self.table_cells || table_bytes(cells, w * h) != self.cache.size() {
            self.table_cells = cells;
            self.cache = table_buffer(&self.device, cells, w * h);
            self.cache_clear_requested
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Caps a dispatch row of the table's edit-box reset and sweep at `groups` workgroups (at
    /// least 1, at most [`MAX_DISPATCH_GROUPS`], the default): a test's way to read the rows
    /// a dispatch past the cap takes.
    pub fn set_table_dispatch_cap(&mut self, groups: u32) {
        self.table_dispatch_cap = groups.clamp(1, MAX_DISPATCH_GROUPS);
    }

    /// Runs the table's guard in every frame, outside its window ([`GUARD_WINDOW`]) too: a
    /// test's way to read what the window keeps out of a still world.
    pub fn force_guard_window(&mut self, on: bool) {
        self.guard_forced = on;
    }

    pub fn lighting(&self) -> Lighting {
        self.lighting
    }

    /// The frame's `rgba8unorm` image: the composite's output, or with the upscaler (see
    /// [`Frame::upscaling`]) the upscaled image at the output size. With the upscaler the
    /// composite writes the internal image into its top-left pixels and the upscale pass's
    /// sharpening pass overwrites the whole of it.
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
    /// bits, material id, flags: bit 0 hit, bit 1 field-surface hit, bit 2 translucent layers
    /// in front, bits 3..5 the entry face's axis plus one, 0 for none).
    pub fn hit_records(&self) -> &wgpu::Texture {
        &self.targets.hit_info[self.cur]
    }

    /// The translucent solid in front of each hit (rgba16float: its reflected light before
    /// exposure in rgb, the luminance of its transmittance in a; ADR-0047).
    pub fn layers(&self) -> &wgpu::Texture {
        &self.targets.layer
    }

    /// The translucent solid's transmittance per channel in front of each hit (r32uint, rgb9e5;
    /// `bake::rgb9e5_unpack` reads a texel), 1 where none is in front.
    pub fn transmittance(&self) -> &wgpu::Texture {
        &self.targets.transmit
    }

    /// The light pass's irradiance (rgba16float: rgb, then the emissive part's luminance).
    pub fn irradiance(&self) -> &wgpu::Texture {
        &self.targets.irradiance
    }

    /// Turns the composite capture on or off: while on, every encode copies the composite's
    /// image (rgba8unorm, display-encoded, at the internal size) into
    /// [`Frame::composite_capture`] right after the composite pass, before the upscaler
    /// overwrites the target. For tests and tools that read the composite apart from the
    /// upscaled display; it costs one copy a frame and is not counted in
    /// [`Frame::frame_bytes`]. Under FSR 2 the composite writes FSR's linear input instead
    /// ([`Fsr2::colour_in`]), and nothing is captured.
    pub fn set_composite_capture(&mut self, device: &wgpu::Device, on: bool) {
        self.composite_capture =
            on.then(|| composite_capture_texture(device, self.internal_size()));
    }

    /// The composite capture the last encode wrote, or `None` while it is off.
    pub fn composite_capture(&self) -> Option<&wgpu::Texture> {
        self.composite_capture.as_ref()
    }

    /// The temporal pass's accumulated irradiance (rgba16float: the emissive part in rgb, the
    /// lamp's in a).
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

    /// Bytes the frame's targets, the emissive table, its probe volume (once built) and FSR 2's
    /// resources occupy.
    pub fn frame_bytes(&self) -> u64 {
        self.targets.bytes()
            + self.probes.get().map_or(0, ProbeVolume::bytes)
            + self.exposure.bytes()
            + self.fsr.as_ref().map_or(0, Fsr2::bytes)
            + self.cache.size()
            + WALK_STATE_BYTES
            + self.blend_params.size()
            + self.blend_dispatch.size()
            + self.walk_dispatch.size()
            + FILL_LIST_BYTES
            + self.fill_dispatch.size()
            + PRELOAD_BYTES
            + self.preload_uniform.size()
            + self.preload_dispatch.size()
            + PRELOAD_IDLE_SLOTS as u64 * PRELOAD_IDLE_BYTES
            + if self.store.pool().size() > crate::bake::MINIMAL_POOL_BYTES {
                self.store.pool().size()
            } else {
                0
            }
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
        let fsr = self.fsr.is_some();
        let upscaled = (fsr || self.jitter || internal != output).then_some(output);
        self.targets = Targets::new(
            device,
            internal[0],
            internal[1],
            self.resolution.scale(),
            upscaled,
            !fsr,
        );
        if let Some(f) = self.fsr.as_mut() {
            f.resize(device, internal, output);
        }
        if self.composite_capture.is_some() {
            self.composite_capture = Some(composite_capture_texture(device, internal));
        }
        self.fit_table();
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
        let fsr = self
            .fsr
            .as_ref()
            .filter(|f| !f.bypassed())
            .map(Fsr2::settings);
        let jitter = if let Some(f) = self.fsr.as_ref().filter(|f| !f.bypassed()) {
            // FSR's sample of pixel p lies at p + 0.5 - offset (`fsr2::jitter_offset`).
            let [w, _] = self.internal_size();
            let [x, y] = jitter_at(self.frame, f.settings().phases(w, self.output[0]));
            [-x, -y]
        } else if self.jitter {
            jitter_at(self.frame, JITTER_PHASES)
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
            face_mix: self.look.tuning.face_mix,
            far_normal: self.look.far_normal,
            window_size: e.size(),
            light_pos: (camera.position + right * light.offset_right - up * light.offset_down)
                .to_array(),
            lamp_pose: [light.offset_right, light.offset_down, light.turn],
            light_intensity: light.intensity,
            light_dir: (fwd * light.turn.cos() + right * light.turn.sin()).to_array(),
            cos_outer: light.outer.cos(),
            size: [self.targets.width, self.targets.height],
            cos_inner: light.inner.cos(),
            flags: if light.shadows { FLAG_SHADOWS } else { 0 }
                | if light.masks { FLAG_MASKS } else { 0 }
                | if light.skips { FLAG_SKIPS } else { 0 }
                | self.look.flags(),
            units_per_shard: world.shard_layout().units_per_shard,
            forced_level: self.lod.forced_level.unwrap_or(0),
            lod1_r2: (2 * self.lod.r_full).pow(2),
            lod2_r2: (4 * self.lod.r_full).pow(2),
            exposure: self.lighting.manual_exposure(),
            cache_mask: self.table_cells() - 1,
            cache: self.lighting.cache_word(),
            fill: self.lighting.fill_word(),
            emitter_cap: fsr.and_then(|f| f.emitter_cap).unwrap_or(0.0),
            dither: u32::from(fsr.is_some_and(|f| f.dither)),
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
        // The probe passes run in the shaded and cracks views outside the cutaway, with the
        // lamp's bounce on and the lamp on (or a preset volume, whose updates are off).
        let presets = self.probes.get().is_some_and(|v| !v.updates());
        let indirect = self.lighting.lamp_bounce
            && (self.lighting.lamp || presets)
            && self.look.view.shades()
            && !self.look.cutaway;
        // The volume exists from the first encode with the channel running.
        let volume = indirect.then(|| self.probes());
        let probe_update = volume.is_some_and(ProbeVolume::updates);
        // The channel starts from zero when it was not running at the last encode or the
        // lamp's light changed: every probe's state is cleared and a window opens.
        let channel_was = self
            .probe_channel_ran
            .swap(probe_update, std::sync::atomic::Ordering::Relaxed);
        let mut probe_restart = probe_update && !channel_was;
        let mut light_window = false;
        let mut lamp_moved = false;
        let mut boxes: Vec<[[i32; 3]; 2]> = Vec::new();
        let mut edits = EditsUniform {
            count: 0,
            _pad: [0; 3],
            boxes: [[0; 4]; 2 * MAX_EDITS],
        };
        let mut old_lamp = Lamp::of(params);
        // More than MAX_EDITS boxes arrived: the table clears whole, as the screen flushes.
        let mut edits_overflowed = false;
        // The camera moved under a voxel and turned under a degree since the last encode.
        let mut camera_still = false;
        // The world changed (an edit, an upload, a history reset) since the last encode: the
        // preload's stamps are stale.
        let mut world_changed = !keep_history;
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
            boxes = std::mem::take(&mut h.edits);
            edits_overflowed = h.overflow;
            h.overflow = false;
            let bytes = world.bytes_written();
            if !boxes.is_empty() || edits_overflowed || h.guard_bytes != Some(bytes) {
                h.guard_event = Some(self.frame);
                world_changed = true;
            }
            h.guard_bytes = Some(bytes);
            let within =
                |f: Option<u32>| f.is_some_and(|f| self.frame.wrapping_sub(f) < GUARD_WINDOW);
            if self.lighting.light == Light::Table
                && (self.guard_forced || within(h.guard_event) || within(self.emission_changed_at))
            {
                p.lighting |= LIGHTING_GUARD;
            }
            let (was, now) = (h.last_views[0], params.views[0]);
            let moved: f32 = (0..3)
                .map(|k| (now.origin[k] - was.origin[k]).powi(2))
                .sum();
            let turn: f32 = (0..3).map(|k| now.fwd[k] * was.fwd[k]).sum();
            // `valid`: the last frame was encoded (`encoded` is this frame's, set below).
            camera_still = h.valid && moved < 1.0 && turn > 1f32.to_radians().cos();
            h.last_views = params.views;
            h.encoded = true;
            let key = LightKey {
                lamp: self.lighting.lamp,
                params: LampKey::of(params),
                shadow_walk: self.lighting.shadow_walk,
            };
            let lamp_changed = h.light_key.is_some_and(|k| k.lamp_differs(&key));
            if lamp_changed {
                p.lighting |= LIGHTING_LAMP_CHANGED;
            }
            self.lamp_changed
                .store(lamp_changed, std::sync::atomic::Ordering::Relaxed);
            // The probes see the lamp alone: its switch, parameters or shadow walk restart them.
            probe_restart |= probe_update && lamp_changed;
            if probe_restart {
                h.light_window = crate::probes::WINDOW_FRAMES;
            }
            h.light_key = Some(key);
            if probe_update && h.light_window > 0 {
                light_window = true;
                h.light_window -= 1;
            }
            let back = (self.upscaler.change_frames.max(1) as usize).min(LAMP_FRAMES);
            old_lamp = h
                .lamps
                .len()
                .checked_sub(back)
                .and_then(|i| h.lamps.get(i))
                .or(h.lamps.front())
                .copied()
                .unwrap_or_else(|| Lamp::of(params));
            let now = Lamp::of(params);
            lamp_moved = h
                .lamps
                .back()
                .is_some_and(|l| l.pos != now.pos || l.dir != now.dir);
            if h.lamps.len() == LAMP_FRAMES {
                h.lamps.pop_front();
            }
            h.lamps.push_back(Lamp::of(params));
        } else {
            p.history = 0;
        }
        queue.write_buffer(&self.edits, 0, bytemuck::bytes_of(&edits));
        let scale = self.output[0] as f32 / self.targets.width as f32;
        queue.write_buffer(
            &self.upscale_params,
            0,
            bytemuck::bytes_of(&self.upscaler.uniform(scale, old_lamp)),
        );
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
        let t = &self.targets;
        let world_bind = self.world_bind(device, world);
        // The cutaway and a mirror without band slots walk the voxels.
        let field = self.look.normal_mode == NormalMode::Contour
            && !self.look.cutaway
            && world.field_surface();
        // The split walk pays only where a brick walks as cubes: with no crisp row every band
        // brick is a field brick. The cracks and steps views read the field walk's own leaks
        // and counters, so they walk it unsplit.
        let split = field
            && self.field_split
            && world.crisp_rows()
            && !matches!(self.look.view, ViewMode::Cracks | ViewMode::Steps)
            && u64::from(t.width) * u64::from(t.height)
                <= u64::from(SPLIT_GROUP) * u64::from(MAX_WORKGROUPS);
        // The hit pass binds the full list only in a frame that splits; the unsplit walk
        // leaves the stub untouched.
        let split_list = if split {
            t.split_list(device)
        } else {
            &t.split_stub
        };
        let (c, q) = (self.cur, self.cur ^ 1);
        let hit_bind = bind_io(
            device,
            &self.hit.out_layout,
            &[
                Res::Tex(&t.hit_pos[c]),
                Res::Tex(&t.hit_info[c]),
                Res::Tex(&t.layer),
                Res::Buf(split_list),
                Res::Buf(&t.split_args),
                Res::Tex(&t.transmit),
                Res::Buf(self.store.pool()),
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
        let cache_table = &self.cache;
        // The edit boxes' dilated voxels, in u64: boxes whose voxels' faces outnumber the
        // table's cells are cleared by a walk over the cells instead (`clear_box_cells`), the
        // same cells; only a frame of more than MAX_EDITS boxes clears the whole table.
        let cache_clear_voxels: u64 = boxes
            .iter()
            .map(|[min, max]| {
                (0..3)
                    .map(|k| (i64::from(max[k]) - i64::from(min[k]) + 3).max(0) as u64)
                    .product::<u64>()
            })
            .sum();
        let cells = self.table_cells();
        let clear_whole = edits_overflowed;
        let clear_by_cells = cache_clear_voxels * 6 > u64::from(cells);
        // The preload runs where the fill could (a shaded view outside the cutaway) with its
        // radius and emissive candidates above 0, unless the idle skip holds (`PreloadIdle`).
        let preload_r = self.lighting.preload_brick_radius();
        let preload_want = self.look.view.shades()
            && !self.look.cutaway
            && preload_r > 0
            && self.lighting.candidates > 0;
        let table_cleared = self
            .cache_clear_requested
            .swap(false, std::sync::atomic::Ordering::Relaxed)
            || clear_whole;
        // The sweep runs on the frame the camera's chunk changes, before the light pass.
        let eye = p.views[0].origin;
        let chunk = eye.map(|v| (v / 64.0).floor() as i32);
        let sweep = {
            let mut last = self
                .sweep_chunk
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let moved = last.is_some_and(|c| c != chunk);
            *last = Some(chunk);
            moved && self.lighting.light == Light::Table
        };
        if sweep {
            self.cache_sweeps
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.sweep_requests
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        {
            let r = world.light_radius();
            let reach = (8 * preload_r) as f32;
            let age_max = self.lighting.table_age.max(1);
            let u = SweepUniform {
                chunk,
                r2: r.saturating_mul(r),
                eye,
                reach2: reach * reach,
                request: self
                    .sweep_requests
                    .load(std::sync::atomic::Ordering::Relaxed),
                cells,
                evict_load: cells / 16 * EVICT_LOAD_SIXTEENTHS,
                evict_step: cells / EVICT_STEP_DIVISOR,
                age_max,
                age_min: self.lighting.table_age_min.clamp(1, age_max),
                slice: TABLE_WALK_SLICE,
                frame: p.frame.wrapping_add(1),
                age_out: u32::from(self.lighting.table_age_out),
                _pad: [0; 3],
            };
            queue.write_buffer(&self.sweep_params, 0, bytemuck::bytes_of(&u));
        }
        let eye_brick = eye.map(|c| (c / 8.0).floor() as i32);
        let preload_key = (
            self.lighting.preload_cap(),
            preload_r,
            self.lighting.preload_claim_radius,
        );
        let grid = t.light_size();
        // The camera is still to the preload: under a voxel and a degree since the last encode,
        // in the same brick, with the guard's window closed.
        let (preload_still, preload_skip) = {
            let mut idle = self
                .preload_idle
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let still = camera_still
                && p.lighting & LIGHTING_GUARD == 0
                && idle.eye_brick == Some(eye_brick);
            let quiet = preload_want
                && still
                && boxes.is_empty()
                && !edits_overflowed
                && !world_changed
                && !table_cleared
                && !sweep
                && idle.grid == Some(grid)
                && self
                    .preload_epoch
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .1
                    == Some(preload_key);
            idle.eye_brick = Some(eye_brick);
            idle.grid = Some(grid);
            let skip = preload_want && idle.decide(device, quiet);
            (still, skip)
        };
        let preload_on = preload_want && !preload_skip;
        self.preload_skips.fetch_add(
            u32::from(preload_skip),
            std::sync::atomic::Ordering::Relaxed,
        );
        if self.lighting.light == Light::Baked {
            encoder.clear_buffer(self.store.pool(), 0, Some(4));
        }
        if table_cleared {
            encoder.clear_buffer(cache_table, 0, None);
            // The counters, the fill list and the preload's header: its lists' entries name
            // cleared cells, which their check drops, and the sweep's offsets stay.
            encoder.clear_buffer(
                &self.cache_stats,
                0,
                Some(CACHE_STATS_BYTES + FILL_LIST_BYTES + PRELOAD_HEADER_BYTES),
            );
            self.cache_clears
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        } else if !preload_on {
            // The misses count this frame's (with the preload on, its args pass takes the last
            // frame's and clears them, before the light pass).
            encoder.clear_buffer(&self.cache_stats, 0, Some(4));
        }
        let light_bind = bind_io(
            device,
            &self.light_layout,
            &[
                Res::Tex(t.lit_pos(c)),
                Res::Tex(t.lit_info(c)),
                Res::Tex(&t.irradiance),
                Res::Buf(cache_table),
                Res::Buf(&self.cache_stats),
                Res::Buf(&self.edits),
            ],
        );
        // The reopened fill runs in a shaded view outside the cutaway with F above 0: the light
        // pass lists the frame's reopened cells into a list cleared here.
        let fill_on = self.look.view.shades()
            && !self.look.cutaway
            && self.lighting.fill_samples.min(FILL_SAMPLES_MAX) > 0;
        if fill_on {
            encoder.clear_buffer(&self.cache_stats, CACHE_STATS_BYTES, Some(16));
        }
        let fill_bind = fill_on.then(|| {
            (
                bind_io_at(
                    device,
                    &self.fill_layout,
                    &[3, 4],
                    &[Res::Buf(cache_table), Res::Buf(&self.cache_stats)],
                ),
                bind_io_at(
                    device,
                    &self.fill_stats_layout,
                    &[4],
                    &[Res::Buf(&self.cache_stats)],
                ),
                bind_io(
                    device,
                    &self.fill_args_layout,
                    &[Res::Buf(&self.fill_dispatch)],
                ),
            )
        });
        let preload_bind = preload_on.then(|| {
            let offsets = {
                let mut written = self
                    .preload_offsets_radius
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let table = preload_offsets(preload_r);
                if *written != Some(preload_r) {
                    *written = Some(preload_r);
                    let packed: Vec<u32> = table
                        .iter()
                        .map(|o| {
                            let [x, y, z] = o.map(|c| (c + 512) as u32);
                            x | y << 10 | z << 20
                        })
                        .collect();
                    queue.write_buffer(
                        &self.cache_stats,
                        CACHE_STATS_BYTES
                            + FILL_LIST_BYTES
                            + PRELOAD_HEADER_BYTES
                            + 16 * u64::from(PRELOAD_LIST_MAX),
                        bytemuck::cast_slice(&packed),
                    );
                }
                table.len() as u32
            };
            // A new epoch where the world, the table or C_PRE changed as a whole: every slab's
            // stamp is stale, so the sweep probes its faces again.
            let epoch = {
                let mut e = self
                    .preload_epoch
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if world_changed || table_cleared || sweep || e.1 != Some(preload_key) {
                    e.0 = e.0.wrapping_add(1);
                }
                e.1 = Some(preload_key);
                e.0
            };
            let r2 = (preload_r * preload_r) as i32;
            let mut u = PreloadUniform {
                eye_brick,
                edit_bricks: 0,
                offsets,
                bricks: self.lighting.preload_bricks.clamp(1, 4096),
                _pad0: 0,
                still: u32::from(preload_still),
                until: self.lighting.preload_cap(),
                draw_cells: self.lighting.preload_samples / FILL_SAMPLES_MAX,
                load_max: cells / PRELOAD_LOAD_DIVISOR,
                probe: PRELOAD_PROBE_SAMPLES,
                epoch,
                light_cols: grid[0].div_ceil(WORKGROUP),
                light_rows: grid[1].div_ceil(WORKGROUP),
                _pad1: 0,
                eye: p.views[0].origin,
                claim: self.lighting.preload_claim_radius.min(8 * preload_r) as f32,
                brick_list: [[0; 4]; PRELOAD_EDIT_BRICKS_MAX],
            };
            // The edit boxes' bricks, dilated by a voxel as the reset's are, within the radius.
            let mut n = 0;
            'boxes: for [min, max] in &boxes {
                let lo = min.map(|c| (c - 1).div_euclid(8));
                let hi = max.map(|c| (c + 1).div_euclid(8));
                for z in lo[2]..=hi[2] {
                    for y in lo[1]..=hi[1] {
                        for x in lo[0]..=hi[0] {
                            let d = [x - eye_brick[0], y - eye_brick[1], z - eye_brick[2]];
                            if d.iter().map(|c| c * c).sum::<i32>() > r2 {
                                continue;
                            }
                            if n == PRELOAD_EDIT_BRICKS_MAX {
                                break 'boxes;
                            }
                            u.brick_list[n] = [x, y, z, 0];
                            n += 1;
                        }
                    }
                }
            }
            u.edit_bricks = n as u32;
            self.preload_edit_bricks
                .store(u.edit_bricks, std::sync::atomic::Ordering::Relaxed);
            queue.write_buffer(&self.preload_uniform, 0, bytemuck::bytes_of(&u));
            // The frame's counts.
            encoder.clear_buffer(
                &self.cache_stats,
                CACHE_STATS_BYTES + FILL_LIST_BYTES + 64,
                Some(PRELOAD_COUNTS_BYTES),
            );
            (
                bind_io_at(
                    device,
                    &self.fill_layout,
                    &[3, 4],
                    &[Res::Buf(cache_table), Res::Buf(&self.cache_stats)],
                ),
                bind_io_at(
                    device,
                    &self.preload_layout,
                    &[1],
                    &[Res::Buf(&self.preload_uniform)],
                ),
                bind_io_at(
                    device,
                    &self.preload_args_layout,
                    &[1, 2],
                    &[
                        Res::Buf(&self.preload_uniform),
                        Res::Buf(&self.preload_dispatch),
                    ],
                ),
            )
        });
        self.preload_ran
            .store(preload_on, std::sync::atomic::Ordering::Relaxed);
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
                Res::Buf(cache_table),
                Res::Buf(self.store.pool()),
            ],
        );
        // The filter runs in the shaded and cracks views unless switched off; the variance pass
        // writes `filter[0]` and the iterations ping-pong, ending in `filter[0]`.
        let filtering = self.lighting.filter && self.look.view.shades();
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
                    &[0, 1, 2, 5, 6, 7, 8],
                    &[
                        Res::Tex(t.lit_pos(c)),
                        Res::Tex(t.lit_info(c)),
                        Res::Tex(&t.filter[i % 2]),
                        Res::Tex(&t.filter[(i + 1) % 2]),
                        Res::Buf(&self.steps[i]),
                        Res::Tex(&t.accum[c]),
                        Res::Tex(&t.moments[c]),
                    ],
                )
            })
            .collect();
        let lit = if filtering {
            &t.filter[ATROUS_ITERATIONS % 2]
        } else {
            &t.irradiance
        };
        // In FSR mode the composite writes FSR's colour input at the internal size.
        let composite_out = self.fsr.as_ref().map_or(&t.output, Fsr2::colour);
        // FSR 2's composite stores linear radiance (`composite_hdr`).
        let (composite_layout, composite) = self
            .composite_hdr
            .as_ref()
            .filter(|_| self.fsr.is_some())
            .map_or((&self.composite_layout, &self.composite), |(l, p)| (l, p));
        let composite_bind = bind_io(
            device,
            composite_layout,
            &[
                Res::Tex(&t.hit_pos[c]),
                Res::Tex(&t.hit_info[c]),
                Res::Tex(&t.layer),
                Res::Tex(lit),
                Res::Tex(composite_out),
                Res::Tex(t.lit_pos(c)),
                Res::Tex(t.lit_info(c)),
                Res::Tex(&t.indirect),
                Res::Buf(&t.tiles),
                Res::Tex(&t.transmit),
            ],
        );
        let gradient_bind = bind_io(
            device,
            &self.gradient_layout,
            &[
                Res::Tex(t.lit_pos(q)),
                Res::Tex(t.lit_info(q)),
                Res::Tex(&t.irradiance),
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
                    Res::Buf(&self.upscale_params),
                ],
            )
        });
        let sharpen_bind = t.history.as_ref().map(|hist| {
            bind_io(
                device,
                &self.sharpen_layout,
                &[
                    Res::Tex(&hist[c]),
                    Res::Tex(&t.output),
                    Res::Buf(&self.sharpen_params),
                ],
            )
        });
        // The gradient pass runs in a shaded view for the gradient mode's lambda, and for the
        // table's guard within its window.
        let gradient_on =
            self.look.view.shades() && p.lighting & (LIGHTING_GRADIENT | LIGHTING_GUARD) != 0;
        let gradient_cache_bind = bind_io_at(
            device,
            &self.gradient_cache_layout,
            &[3, 4],
            &[Res::Buf(&self.cache), Res::Buf(&self.cache_stats)],
        );
        let full = [t.width.div_ceil(WORKGROUP), t.height.div_ceil(WORKGROUP)];
        let lw = t.light_size().map(|n| n.div_ceil(WORKGROUP));
        let strata = [t.strata.width(), t.strata.height()].map(|n| n.div_ceil(WORKGROUP));
        let fsr_binds = self.fsr.as_ref().map(|f| {
            (
                f,
                f.prepare(
                    device,
                    queue,
                    &p,
                    &crate::fsr2::FrameInputs {
                        hit_pos: &t.hit_pos[c],
                        hit_info: &t.hit_info[c],
                        layer: &t.layer,
                        edits: &self.edits,
                        lamp: &self.upscale_params,
                        output: &t.output,
                        cur: c,
                        dt: self.dt,
                    },
                ),
            )
        });
        if let Some(f) = &self.fsr {
            f.copy_exposure(
                encoder,
                self.exposure.state(),
                matches!(self.lighting.exposure, Exposure::Auto(_)),
                p.history & 2 == 0,
            );
        }
        // A frame pass's dispatch with the world's group 0 and its own group `group`.
        let frame_pass = |pipeline, bind, group: usize, workgroups| {
            let mut groups = [Some(&world_bind), None, None, None];
            groups[group] = Some(bind);
            Dispatch {
                pipeline,
                groups,
                workgroups,
                indirect: None,
            }
        };
        let hit_pipeline = if split {
            p.flags |= FLAG_FIELD_SPLIT;
            &self.hit.pipeline
        } else if field {
            &self.hit.field
        } else {
            &self.hit.pipeline
        };
        let resume_bind = split.then(|| {
            encoder.clear_buffer(&t.split_args, 0, None);
            encoder.clear_buffer(split_list, 0, Some(16));
            bind_io(
                device,
                &self.hit.resume_layout,
                &[
                    Res::Tex(&t.hit_pos[c]),
                    Res::Tex(&t.hit_info[c]),
                    Res::Tex(&t.layer),
                    Res::Buf(split_list),
                ],
            )
        });
        let mut hit: Vec<Dispatch<'_>> = vec![frame_pass(hit_pipeline, &hit_bind, 1, full)];
        if let Some(b) = &resume_bind {
            hit.push(Dispatch {
                indirect: Some((&t.split_args, 0)),
                ..frame_pass(&self.hit.resume, b, 1, [0, 0])
            });
        }
        if half {
            hit.push(frame_pass(&self.downsample, &downsample_bind, 3, lw));
        }
        let mut gradient: Vec<Dispatch<'_>> = Vec::new();
        // The table's edit-box reset and sweep run in the gradient's slot, before the light
        // pass.
        let cache_clear_bind = (!clear_whole && cache_clear_voxels > 0).then(|| {
            bind_io(
                device,
                &self.cache_clear_layout,
                &[Res::Buf(&self.edits), Res::Buf(cache_table)],
            )
        });
        if let Some(b) = &cache_clear_bind {
            gradient.push(if clear_by_cells {
                frame_pass(
                    self.cache_clear_cells_pipeline(device),
                    b,
                    2,
                    dispatch_rows(cells.div_ceil(64), self.table_dispatch_cap),
                )
            } else {
                frame_pass(
                    &self.cache_clear,
                    b,
                    2,
                    dispatch_rows(
                        cache_clear_voxels.div_ceil(64) as u32,
                        self.table_dispatch_cap,
                    ),
                )
            });
        }
        // The walk (the chunk sweep and the age-out): its args pass every frame, then the
        // slice it sets, none while idle.
        let walk_bind = bind_io(
            device,
            &self.cache_sweep_layout,
            &[
                Res::Buf(&self.sweep_params),
                Res::Buf(cache_table),
                Res::Buf(&self.cache_stats),
                Res::Buf(&self.walk_state),
                Res::Buf(&self.walk_dispatch),
            ],
        );
        let walk_slice_bind = bind_io(
            device,
            &self.cache_walk_layout,
            &[
                Res::Buf(&self.sweep_params),
                Res::Buf(cache_table),
                Res::Buf(&self.cache_stats),
                Res::Buf(&self.walk_state),
            ],
        );
        {
            let mut a = frame_pass(&self.cache_walk_args, &walk_bind, 2, [1, 1]);
            a.groups[0] = None;
            gradient.push(a);
            let mut w = frame_pass(&self.cache_sweep, &walk_slice_bind, 2, [0, 0]);
            w.groups[0] = None;
            w.indirect = Some((&self.walk_dispatch, 0));
            gradient.push(w);
        }
        if gradient_on {
            let mut pass = frame_pass(&self.gradient, &gradient_bind, 3, strata);
            pass.groups[2] = Some(&gradient_cache_bind);
            gradient.push(pass);
        }
        let blend_bind = (self.lighting.blend != TableBlend::Off).then(|| {
            queue.write_buffer(
                &self.blend_params,
                0,
                bytemuck::bytes_of(&BlendUniform {
                    cache_mask: cells - 1,
                    c_cap: self.lighting.table_cap.clamp(1, DEFAULT_TABLE_CAP),
                    pool_min: self.lighting.blend_pool_min,
                    wide: u32::from(self.lighting.blend == TableBlend::Wide),
                    frame: p.frame.wrapping_add(1),
                    force: u32::from(
                        self.blend_force
                            .swap(false, std::sync::atomic::Ordering::Relaxed),
                    ),
                    _pad: [0; 2],
                }),
            );
            (
                bind_io(
                    device,
                    &self.cache_blend_args_layout,
                    &[
                        Res::Buf(&self.blend_params),
                        Res::Buf(cache_table),
                        Res::Buf(&self.blend_dispatch),
                    ],
                ),
                bind_io(
                    device,
                    &self.cache_blend_layout,
                    &[Res::Buf(&self.blend_params), Res::Buf(cache_table)],
                ),
            )
        });
        let mut blend: Vec<Dispatch<'_>> = Vec::new();
        if let Some((args, cells)) = &blend_bind {
            let mut a = frame_pass(&self.cache_blend_args, args, 2, [1, 1]);
            a.groups[0] = None;
            blend.push(a);
            let mut d = frame_pass(&self.cache_blend, cells, 2, [0, 0]);
            d.groups[0] = None;
            d.indirect = Some((&self.blend_dispatch, 0));
            blend.push(d);
        }
        let mut fill: Vec<Dispatch<'_>> = Vec::new();
        if let Some((list, stats, args)) = &fill_bind {
            let mut a = frame_pass(&self.fill_args, stats, 2, [1, 1]);
            a.groups[3] = Some(args);
            fill.push(a);
            fill.push(Dispatch {
                indirect: Some((&self.fill_dispatch, 0)),
                ..frame_pass(&self.fill, list, 2, [0, 0])
            });
        }
        let mut preload: Vec<Dispatch<'_>> = Vec::new();
        if let Some((cells, uniform, args)) = &preload_bind {
            let mut a = frame_pass(&self.preload_args, cells, 2, [1, 1]);
            a.groups[0] = None;
            a.groups[3] = Some(args);
            preload.push(a);
            let mut w = frame_pass(&self.preload_sweep, cells, 2, [0, 0]);
            w.groups[3] = Some(uniform);
            w.indirect = Some((&self.preload_dispatch, 0));
            preload.push(w);
        }
        let mut atrous: Vec<Dispatch<'_>> = Vec::new();
        if filtering {
            atrous.push(frame_pass(&self.variance, &variance_bind, 2, lw));
            atrous.extend(
                atrous_binds
                    .iter()
                    .map(|b| frame_pass(&self.atrous, b, 2, lw)),
            );
        }
        p.lighting &= !LIGHTING_INDIRECT;
        let mut gather: Vec<Dispatch<'_>> = Vec::new();
        let mut probes: Vec<Dispatch<'_>> = Vec::new();
        let mut updates = 0;
        self.traced_probe_rays
            .store(0, std::sync::atomic::Ordering::Relaxed);
        self.probe_light_window
            .store(light_window, std::sync::atomic::Ordering::Relaxed);
        let probe_bind = volume.map(|volume| {
            p.lighting |= LIGHTING_INDIRECT;
            if probe_update {
                let slices = if light_window {
                    crate::probes::WINDOW_SLICES
                } else {
                    self.lighting.slices()
                };
                let origin = volume.origin_for(p.views[0].origin);
                let touched: Vec<[i32; 3]> = boxes
                    .iter()
                    .flat_map(|&[min, max]| volume.touched(origin, min, max))
                    .collect();
                if probe_restart {
                    let base = u64::from(volume.state_base()) * 4;
                    encoder.clear_buffer(volume.data(), base, Some(volume.state_bytes()));
                    volume.forget_reach();
                }
                let reach = self.lighting.lamp_reach;
                let lamp = p.light_pos;
                let moved = lamp_moved && self.lighting.lamp;
                let list = volume.update_list(origin, p.frame, slices, &touched, moved, |w| {
                    let q = volume.probe_position(w);
                    let d2: f32 = (0..3).map(|k| (q[k] - lamp[k]).powi(2)).sum();
                    reach <= 0.0 || d2 <= reach * reach
                });
                queue.write_buffer(volume.scratch(), 0, bytemuck::cast_slice(&list));
                updates = list.len() as u32;
            }
            self.traced_probe_rays.store(
                u64::from(updates) * u64::from(self.lighting.rays()),
                std::sync::atomic::Ordering::Relaxed,
            );
            self.write_probe_uniform(queue, volume, &p, updates);
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("probes"),
                layout: &self.probe_layout,
                entries: &[
                    buffer_entry(0, volume.data()),
                    buffer_entry(1, volume.scratch()),
                    buffer_entry(2, volume.uniform()),
                ],
            })
        });
        let gather_bind = probe_bind.is_some().then(|| {
            bind_io(
                device,
                &self.gather_layout,
                &[
                    Res::Tex(t.lit_pos(c)),
                    Res::Tex(t.lit_info(c)),
                    Res::Tex(&t.indirect),
                ],
            )
        });
        if let (Some(b), Some(g)) = (&probe_bind, &gather_bind) {
            gather.push(Dispatch {
                pipeline: &self.gather,
                groups: [Some(&world_bind), None, Some(g), Some(b)],
                workgroups: lw,
                indirect: None,
            });
            if updates > 0 {
                probes.push(frame_pass(&self.probe_trace, b, 3, [updates, 1]));
                probes.push(frame_pass(&self.probe_blend, b, 3, [updates, 1]));
            }
        }
        self.probe_dispatches.store(
            (gather.len() + probes.len()) as u32,
            std::sync::atomic::Ordering::Relaxed,
        );
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&p));
        let mut upscale: Vec<Dispatch<'_>> = Vec::new();
        let mut sharpen: Vec<Dispatch<'_>> = Vec::new();
        if let (Some(u), Some(r)) = (&upscale_bind, &sharpen_bind) {
            let out = self.output.map(|n| n.div_ceil(WORKGROUP));
            upscale.push(frame_pass(&self.upscale, u, 2, out));
            sharpen.push(frame_pass(&self.sharpen, r, 3, out));
        }
        let fsr_passes = fsr_binds
            .as_ref()
            .map(|(f, b)| b.passes(f, &world_bind))
            .unwrap_or_default();
        let passes = [
            hit,
            gradient,
            preload,
            match &preload_bind {
                Some((_, uniform, _)) => {
                    let mut m = frame_pass(&self.light_preload, &light_bind, 2, [0, 0]);
                    m.groups[3] = Some(uniform);
                    m.indirect = Some((&self.preload_dispatch, 12));
                    vec![m]
                }
                None => vec![frame_pass(self.light_pipeline(device), &light_bind, 2, lw)],
            },
            fill,
            blend,
            vec![frame_pass(&self.temporal, &temporal_bind, 2, lw)],
            atrous,
            probes,
            gather,
            vec![frame_pass(composite, &composite_bind, 2, full)],
            upscale,
            sharpen,
        ]
        .into_iter()
        .chain(fsr_passes);
        for (i, dispatches) in passes.enumerate() {
            // An empty slot is still recorded under a timer, so every timestamp pair is written.
            if dispatches.is_empty() && timer.is_none() {
                continue;
            }
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(FRAME_PASSES[i]),
                timestamp_writes: timer.map(|t| t.writes_at(i as u32)),
            });
            for d in dispatches {
                pass.set_pipeline(d.pipeline);
                for (g, bind) in d.groups.iter().enumerate() {
                    if let Some(b) = bind {
                        pass.set_bind_group(g as u32, *b, &[]);
                    }
                }
                match d.indirect {
                    Some((args, offset)) => pass.dispatch_workgroups_indirect(args, offset),
                    None => pass.dispatch_workgroups(d.workgroups[0], d.workgroups[1], 1),
                }
            }
            drop(pass);
            if FRAME_PASSES[i] == "composite"
                && self.fsr.is_none()
                && let Some(capture) = &self.composite_capture
            {
                encoder.copy_texture_to_texture(
                    t.output.as_image_copy(),
                    capture.as_image_copy(),
                    capture.size(),
                );
            }
        }
        // The args pass's words, for the idle skip.
        if preload_bind.is_some() {
            let mut idle = self
                .preload_idle
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(slot) = idle.take_slot() {
                encoder.copy_buffer_to_buffer(
                    &self.cache_stats,
                    CACHE_STATS_BYTES + FILL_LIST_BYTES + PRELOAD_IDLE_WORDS_AT,
                    slot,
                    0,
                    PRELOAD_IDLE_BYTES,
                );
            }
        }
        let exposure_pass = FRAME_PASSES.len() as u32 - 1;
        self.exposure.encode(
            device,
            queue,
            encoder,
            &t.tiles,
            &self.exposure_uniform(&p),
            timer
                .filter(|t| t.passes() > exposure_pass)
                .map(|t| t.writes_at(exposure_pass)),
        );
    }

    /// The exposure pass's parameters for the frame `p`.
    fn exposure_uniform(&self, p: &Params) -> ExposureUniform {
        let metering = match self.lighting.exposure {
            Exposure::Auto(a) => a.metering,
            Exposure::Manual(_) => self.metering,
        };
        let (strength, sigma) = metering.params();
        let meter = self.look.view.shades();
        let mut mode = if meter { EX_METER } else { 0 };
        if let Exposure::Auto(_) = self.lighting.exposure {
            mode |= EX_AUTO;
            if meter
                && self
                    .reset_exposure
                    .swap(false, std::sync::atomic::Ordering::Relaxed)
            {
                mode |= EX_RESET;
            }
        }
        let mut rects = [[0.0; 4]; 2];
        for (r, v) in rects.iter_mut().zip(&p.views) {
            *r = v.rect.map(|x| x as f32);
        }
        let mut u = ExposureUniform {
            tiles: self.targets.tile_counts(),
            view_count: p.view_count,
            mode,
            rects,
            manual: self.lighting.manual_exposure(),
            strength,
            sigma,
            dt: self.dt,
            ..ExposureUniform::default()
        };
        if let Exposure::Auto(a) = &self.lighting.exposure {
            u.set_auto(a);
        }
        u
    }

    /// Writes the probe passes' uniform for the frame `p`: the volume's origin for its first
    /// view's eye, `updates` entries in the update list, the freeze count, the side, the lamp's
    /// reach (0 for a preset volume) and the moving lamp's blend.
    fn write_probe_uniform(
        &self,
        queue: &wgpu::Queue,
        volume: &ProbeVolume,
        p: &Params,
        updates: u32,
    ) {
        let u = ProbeUniform {
            origin: volume.origin_for(p.views[0].origin),
            spacing: volume.spacing(),
            updates,
            rays: self.lighting.rays(),
            steady: self.lighting.probe_steady(),
            axis: volume.axis(),
            reach: if volume.updates() {
                self.lighting.lamp_reach.max(0.0)
            } else {
                0.0
            },
            h_move: self.lighting.probe_move_blend.clamp(0.0, 1.0),
            _pad: [0; 2],
        };
        queue.write_buffer(volume.uniform(), 0, bytemuck::bytes_of(&u));
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

/// The source of the composite's variant for FSR 2: `HDR_TARGET` set and the target
/// rgba32float, linear radiance before exposure in the shaded view.
pub fn composite_hdr_source() -> String {
    let flag = "const HDR_TARGET: bool = false;";
    let target = "var target_tex: texture_storage_2d<rgba8unorm, write>;";
    assert!(
        COMPOSITE_WGSL.matches(flag).count() == 1 && COMPOSITE_WGSL.matches(target).count() == 1,
        "composite.wgsl declares its HDR flag and its target once"
    );
    COMPOSITE_WGSL
        .replacen(flag, "const HDR_TARGET: bool = true;", 1)
        .replacen(
            target,
            "var target_tex: texture_storage_2d<rgba32float, write>;",
            1,
        )
}

/// The composite's FSR 2 variant: its layout (the composite's, the target rgba32float) and
/// pipeline.
fn composite_hdr_pass(
    device: &wgpu::Device,
    world_layout: &wgpu::BindGroupLayout,
) -> (wgpu::BindGroupLayout, wgpu::ComputePipeline) {
    let layout = io_layout(
        device,
        "composite hdr",
        &[
            Io::Float,
            Io::Uint,
            Io::Float,
            Io::Float,
            Io::Store(wgpu::TextureFormat::Rgba32Float),
            Io::Float,
            Io::Uint,
            Io::Float,
            Io::Storage,
            Io::Uint,
        ],
    );
    let pipeline = compute_pipeline(
        device,
        "composite hdr",
        format!("{MARCH_WGSL}\n{}", composite_hdr_source()),
        &[Some(world_layout), None, Some(&layout)],
        "composite",
    );
    (layout, pipeline)
}

/// One dispatch of a frame pass: its pipeline, the bind groups at indices 0 to 3, and the
/// workgroups in x and y.
pub(crate) struct Dispatch<'a> {
    pub pipeline: &'a wgpu::ComputePipeline,
    pub groups: [Option<&'a wgpu::BindGroup>; 4],
    pub workgroups: [u32; 2],
    /// Dispatched with the workgroup counts this buffer holds at this byte offset instead.
    pub indirect: Option<(&'a wgpu::Buffer, u64)>,
}

fn buffer_entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

/// The texture [`Frame::set_composite_capture`] copies the composite into.
fn composite_capture_texture(device: &wgpu::Device, size: [u32; 2]) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("composite capture"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// Passes of a frame in encode order, as [`Frame::encode`] times them: pass `i` writes
/// timestamps `2 i` and `2 i + 1` of the timer it is given. With the preload on, `preload`
/// holds its args pass and sweep, and `light` the light pass with the preload's fill and
/// probes in one dispatch.
/// The nine before the last are FSR 2's ([`crate::fsr2::FSR2_PASSES`]); in FSR mode `upscale`
/// and `sharpen` are empty, and otherwise FSR's are. `exposure` runs last, after either.
pub const FRAME_PASSES: [&str; 23] = [
    "hit",
    "gradient",
    "preload",
    "light",
    "fill",
    "blend",
    "temporal",
    "atrous",
    "probes",
    "gather",
    "composite",
    "upscale",
    "sharpen",
    "fsr2 input",
    "fsr2 spd",
    "fsr2 spd tail",
    "fsr2 reconstruct",
    "fsr2 depth clip",
    "fsr2 lock",
    "fsr2 accumulate",
    "fsr2 rcas",
    "fsr2 tone",
    "exposure",
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
    /// The translucent solid in front of each hit: its reflected light (rgba16float), or the
    /// steps view's counters.
    layer: wgpu::Texture,
    /// The translucent solid's transmittance per channel, rgb9e5 in r32uint (ADR-0047).
    transmit: wgpu::Texture,
    /// The light pass's irradiance (rgba16float), on the lighting grid.
    irradiance: wgpu::Texture,
    /// The temporal pass's accumulated irradiance and moments (rgba16float).
    accum: [wgpu::Texture; 2],
    moments: [wgpu::Texture; 2],
    /// The filter's ping-pong pair (rgba16float: irradiance rgb, variance a).
    filter: [wgpu::Texture; 2],
    /// The gradient pass's strata, one texel per 3 x 3 lighting pixels (rgba16float: the
    /// lamp's lambda, the new and old luminances, 0).
    strata: wgpu::Texture,
    /// The frame's display-encoded image (rgba8unorm): the composite's output at the
    /// internal size, or with the upscaler at the output size, the composite's image in its
    /// top-left pixels until the sharpening pass overwrites it.
    output: wgpu::Texture,
    /// The probe gather's indirect irradiance (rgba16float), on the lighting grid.
    indirect: wgpu::Texture,
    /// The upscaler's history at the output size (rgba16float), this frame's and the last.
    history: Option<[wgpu::Texture; 2]>,
    /// The exposure meter's tiles: two `f32` per 8 x 8 workgroup of the internal target.
    tiles: wgpu::Buffer,
    /// The split walk's hand-off list (a count word and three of padding, then a `vec4<u32>`
    /// per pixel; `march.wgsl`'s `SplitList`), built by the first encode that splits
    /// ([`Targets::split_list`]); until then the hit pass binds `split_stub`, the header and
    /// one ray (the binding's minimum size), which the unsplit walk never writes. The resume
    /// dispatch's workgroup counts (`SplitArgs`).
    split_list: std::sync::OnceLock<wgpu::Buffer>,
    split_stub: wgpu::Buffer,
    split_args: wgpu::Buffer,
}

impl Targets {
    /// The targets at the internal size `width` x `height`, with the output at `upscaled` when
    /// an upscaler runs, and the history of ADR-0024's upscaler with `own_history`.
    fn new(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        scale: u32,
        upscaled: Option<[u32; 2]>,
        own_history: bool,
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
        use wgpu::TextureFormat::{R32Uint, Rgba8Unorm, Rgba16Float, Rgba16Uint, Rgba32Float};
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
            transmit: tex("transmittance", R32Uint),
            irradiance: lit("irradiance", Rgba16Float),
            accum: [0, 1].map(|_| lit("accumulated irradiance", Rgba16Float)),
            moments: [0, 1].map(|_| lit("moments", Rgba16Float)),
            filter: [0, 1].map(|_| lit("filter", Rgba16Float)),
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
                .filter(|_| own_history)
                .map(|[w, h]| [0, 1].map(|_| sized("upscale history", Rgba16Float, w, h))),
            indirect: lit("indirect irradiance", Rgba16Float),
            tiles: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("exposure meter tiles"),
                size: 8 * u64::from(width.div_ceil(METER_TILE) * height.div_ceil(METER_TILE)),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            split_list: std::sync::OnceLock::new(),
            split_stub: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("split walk rays (unsplit)"),
                size: 32,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            split_args: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("split walk dispatch"),
                size: 16,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::INDIRECT
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
        }
    }

    /// The split walk's hand-off list at one ray per pixel, built here on the first call.
    fn split_list(&self, device: &wgpu::Device) -> &wgpu::Buffer {
        self.split_list.get_or_init(|| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("split walk rays"),
                size: 16 + 16 * u64::from(self.width) * u64::from(self.height),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        })
    }

    /// The meter's tiles on each edge.
    fn tile_counts(&self) -> [u32; 2] {
        [self.width, self.height].map(|n| n.div_ceil(METER_TILE))
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
            + lpx * (half + 8 + 2 * 8 + 2 * 8 + 2 * 8 + 8)
            + strata * 8
            + self.tiles.size()
            + self.split_list.get().map_or(0, wgpu::Buffer::size)
            + self.split_stub.size()
            + self.split_args.size()
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
    compute_pipeline_with(device, label, source, layouts, entry, &[])
}

/// [`compute_pipeline`] with the pipeline-overridable constants `constants`.
fn compute_pipeline_with(
    device: &wgpu::Device,
    label: &str,
    source: String,
    layouts: &[Option<&wgpu::BindGroupLayout>],
    entry: &str,
    constants: &[(&str, f64)],
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
        compilation_options: wgpu::PipelineCompilationOptions {
            constants,
            ..Default::default()
        },
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

/// Holds the light-sample ranges out to every chunk of `extent`'s reach (its farthest corner from
/// the focus chunk, plus the light radius and one, for a job's box) and syncs until they settle.
fn hold_reach(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    gpu: &mut GpuWorld,
    world: &vxe_world::World,
    extent: vxe_world::ChunkExtent,
) -> Result<(), crate::bake::BakeError> {
    let f = gpu.focus().0;
    let size = extent.size();
    let far: i64 = (0..3)
        .map(|k| {
            let lo = i64::from(extent.min.0[k] - f[k]).abs();
            let hi = i64::from(extent.min.0[k] + size[k] - 1 - f[k]).abs();
            lo.max(hi).pow(2)
        })
        .sum();
    let reach = (far as f64).sqrt().ceil() as i32 + gpu.light_radius() + 1;
    gpu.hold_lights(reach);
    let mut refused = 0;
    loop {
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let synced = gpu.sync(world, queue, &mut enc);
        queue.submit([enc.finish()]);
        if let Ok(s) = synced {
            refused += s.light_refused;
        }
        if gpu.lights_settled() || refused > 0 {
            break;
        }
    }
    if refused > 0 {
        return Err(crate::bake::BakeError::LightsRefused(refused));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{JITTER_PHASES, RenderScale, jitter_at};

    /// The preload's offsets hold every brick within the radius once, nearest first, from the
    /// eye's own brick: 2109 at 8 bricks, and at 16 exactly the store's `PRELOAD_OFFSETS_MAX`;
    /// each packs into the shader's ten bits a component.
    #[test]
    fn the_preload_offsets_run_nearest_first() {
        use super::{PRELOAD_OFFSETS_MAX, PRELOAD_RADIUS_MAX, preload_offsets};
        let near = preload_offsets(8);
        assert_eq!(near.len(), 2109);
        assert_eq!(near[0], [0, 0, 0]);
        let d2 = |o: &[i32; 3]| o.iter().map(|c| c * c).sum::<i32>();
        assert!(near.windows(2).all(|w| d2(&w[0]) <= d2(&w[1])));
        let mut set = near.clone();
        set.sort_unstable();
        set.dedup();
        assert_eq!(set.len(), near.len());
        let far = preload_offsets(PRELOAD_RADIUS_MAX / 8);
        assert_eq!(far.len() as u64, PRELOAD_OFFSETS_MAX);
        assert!(far.iter().flatten().all(|c| (c + 512) as u32 <= 1023));
        assert!(preload_offsets(0) == vec![[0, 0, 0]]);
    }

    /// The jitter's sixteen phases are distinct points of `(-0.5, 0.5)^2` that repeat with
    /// period 16 and average to within 1/16 of zero on each axis (-0.029 and -0.037).
    #[test]
    fn the_jitter_cycles_sixteen_distinct_centred_offsets() {
        let phases: Vec<[f32; 2]> = (0..JITTER_PHASES)
            .map(|f| jitter_at(f, JITTER_PHASES))
            .collect();
        for (i, p) in phases.iter().enumerate() {
            assert!(p.iter().all(|c| c.abs() < 0.5), "phase {i}: {p:?}");
            assert_eq!(jitter_at(i as u32 + JITTER_PHASES, JITTER_PHASES), *p);
            assert!(phases[..i].iter().all(|q| q != p), "phase {i} repeats");
        }
        assert_eq!(phases[0], [0.0, 1.0 / 3.0 - 0.5]);
        for k in 0..2 {
            let mean = phases.iter().map(|p| p[k]).sum::<f32>() / JITTER_PHASES as f32;
            assert!(mean.abs() <= 1.0 / 16.0, "axis {k}: mean {mean}");
        }
    }

    /// The upscale pass parses and validates headless, compiled after the hit
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

    /// The sharpening pass parses and validates headless, compiled after the hit pass.
    #[test]
    fn sharpen_wgsl_validates_through_naga() {
        let source = format!("{}\n{}", crate::MARCH_WGSL, super::SHARPEN_WGSL);
        let module = wgpu::naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("sharpen.wgsl parses: {}", e.emit_to_string(&source)));
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::default(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("sharpen.wgsl validates: {e:?}"));
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
