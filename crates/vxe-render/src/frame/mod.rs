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

/// Workgroup edge in pixels; must match `@workgroup_size` in every frame shader.
pub(crate) const WORKGROUP: u32 = 8;

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
    pub _pad4: u32,
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
    pub _pad8: u32,
    /// The size of the frame's image: the output size with the upscaler, else `size`.
    pub output_size: [u32; 2],
    pub _pad10: u32,
    /// [`LookParams::far_normal`].
    pub far_normal: [f32; 2],
    pub _pad9: u32,
    /// 1: [`Fsr2Settings::dither`] while FSR 2 runs unbypassed.
    pub dither: u32,
    /// Pads the block to a multiple of 16 bytes, as the shaders' struct is.
    pub _tail: [u32; 3],
}

const _: () = assert!(size_of::<Params>().is_multiple_of(16));

/// `Params::lighting` bit: the headlamp lights the frame.
pub(crate) const LIGHTING_LAMP: u32 = 1;
/// `Params::lighting` bit: shadow rays take the coarse walk ([`ShadowWalk::Coarse`]).
pub(crate) const LIGHTING_COARSE_SHADOW: u32 = 1 << 1;
/// `Params::lighting` bit: the temporal pass also shortens history by A-SVGF's gradient
/// ([`History::Gradient`]).
pub(crate) const LIGHTING_GRADIENT: u32 = 1 << 3;
/// [`Lighting::probe_freeze`] by default: where a running mean's noise, `1 / n`, roughly
/// equals hysteresis 0.97's steady noise, `0.03 / 1.97`.
pub const DEFAULT_PROBE_FREEZE: u32 = 64;
/// Frames after an emission-scale change (the frame of the change too) during which the
/// emissive part blends at no less
/// than the lamp's 0.2 (`0.8^11 = 0.086`: a step settles within 10 % in them).
pub(crate) const EMISSION_CHANGE_FRAMES: u32 = 11;
/// `Params::lighting` bit: an emission scale changed within the last
/// [`EMISSION_CHANGE_FRAMES`] frames.
pub(crate) const LIGHTING_EMISSION_CHANGED: u32 = 1 << 8;
/// `Params::lighting` bit: the lamp's light changed since the last encode with history (its
/// switch, a parameter, or the walk its shadow rays take): the temporal pass takes this
/// frame's lamp part as it is, the way an edit box resets a pixel (ADR-0018), and keeps the
/// emissive part's history. The light pass's lamp estimate carries no sampling noise (one
/// point light, one shadow ray), so the reset adds none.
pub(crate) const LIGHTING_LAMP_CHANGED: u32 = 1 << 9;
/// `Params::lighting` bit: the a-trous filter is off ([`Lighting::filter`]).
pub(crate) const LIGHTING_FILTER_OFF: u32 = 1 << 5;
/// `Params::lighting` bit: the composite's tone map is the identity clamp.
pub(crate) const LIGHTING_IDENTITY_TONE_MAP: u32 = 1 << 6;
/// `Params::lighting` bit: the probe passes ran and the composite adds the gathered indirect
/// irradiance ([`Lighting::lamp_bounce`]).
pub(crate) const LIGHTING_INDIRECT: u32 = 1 << 12;
/// `Params::lighting` bit: the exposure is automatic ([`Exposure::Auto`]): the frame's is the
/// exposure state's `current`.
pub(crate) const LIGHTING_AUTO_EXPOSURE: u32 = 1 << 13;
/// `Params::lighting` bit: a cube face's stored part is read by its shading normal
/// ([`StoreRead::Normal`], ADR-0049).
pub(crate) const LIGHTING_STORE_BY_NORMAL: u32 = 1 << 14;
/// `Params::lighting` bit: the half-resolution upsample and the a-trous edge weights compare
/// the taps' face normals ([`LightWeights::Face`], ADR-0049).
pub(crate) const LIGHTING_FACE_WEIGHTS: u32 = 1 << 15;

/// `Params::flags` bit: trace the shadow ray.
pub(crate) const FLAG_SHADOWS: u32 = 1;
/// `Params::flags` bit: skip empty blocks through the brick occupancy masks.
pub(crate) const FLAG_MASKS: u32 = 2;
/// `Params::flags` bits 2..4: the [`NormalMode`].
pub(crate) const NORMAL_SHIFT: u32 = 2;
/// `Params::flags` bits 5..8: the [`ViewMode`].
pub(crate) const VIEW_SHIFT: u32 = 5;
/// `Params::flags` bit: skip empty cells larger than a voxel (`LightParams::skips`).
pub(crate) const FLAG_SKIPS: u32 = 1 << 9;
/// `Params::flags` bit: the primary ray passes through solid until it has crossed real air
/// (`LookParams::cutaway`).
pub(crate) const FLAG_CUTAWAY: u32 = 1 << 10;
/// `Params::flags` bit, set by the frame and not by a look: the field look's split walk. The
/// hit pipeline walks each ray as the cube walk does until it enters a brick the field can hit
/// in (a band brick of a full chunk with a smooth solid voxel in it or its shell), and a second
/// dispatch walks the field from there ([`Frame::set_field_split`]).
pub(crate) const FLAG_FIELD_SPLIT: u32 = 1 << 19;

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

/// How a cube face's hit reads its emissive part from the store (ADR-0049).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StoreRead {
    /// The face's own direction value, the same at every pixel of the face. The default.
    #[default]
    Face,
    /// Where the shading normal leaves the face normal (the far normal's band, or a whole-frame
    /// normal look), the mix of the cell's direction values by the normal's squared components,
    /// over the directions that hold a value, renormalised; the face's own read where none
    /// does. Where the shading normal is the face normal it is [`StoreRead::Face`] to the bit.
    Normal,
}

impl StoreRead {
    pub const ALL: [Self; 2] = [Self::Face, Self::Normal];

    pub fn name(self) -> &'static str {
        match self {
            Self::Face => "face",
            Self::Normal => "normal",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }
}

/// The normal the half-resolution light's filters compare between two hits: the composite's
/// 2 x 2 upsample (`max(n.n', 0)^32`) and the a-trous edge weight (`max(n.n', 0)^128`).
/// The plane term keeps the shading normal under both.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LightWeights {
    /// The hit record's shading normal. The default.
    #[default]
    Normal,
    /// A cube-face hit's face normal, facing the eye; a hit with no face axis (a field hit)
    /// keeps its shading normal. Where the shading normal is the face normal it is
    /// [`LightWeights::Normal`] to the bit.
    Face,
}

impl LightWeights {
    pub const ALL: [Self; 2] = [Self::Normal, Self::Face];

    pub fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Face => "face",
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
pub(crate) const LAMP_FRAMES: usize = 32;

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
    /// the composite shows the temporal pass's accumulation unfiltered (a cube face's emissive
    /// part the store's value, which the light pass's irradiance does not hold).
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
    /// The store's pool (ADR-0046), clamped to the device's storage binding (default
    /// [`crate::bake::DEFAULT_BAKE_POOL_BYTES`]): allocated by the first bake or live queue
    /// ([`Frame::bake_until`], [`Frame::start_bake_queue`]); a minimal one until then, which
    /// holds no light.
    pub bake_pool_bytes: u64,
    /// How a cube face reads the store ([`StoreRead`]; default `Face`).
    pub store_read: StoreRead,
    /// The normal the half-resolution light's filters compare ([`LightWeights`]; default
    /// `Normal`).
    pub light_weights: LightWeights,
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
            bake_pool_bytes: crate::bake::DEFAULT_BAKE_POOL_BYTES,
            store_read: StoreRead::Face,
            light_weights: LightWeights::Normal,
        }
    }
}

impl Lighting {
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
            | if self.store_read == StoreRead::Normal {
                LIGHTING_STORE_BY_NORMAL
            } else {
                0
            }
            | if self.light_weights == LightWeights::Face {
                LIGHTING_FACE_WEIGHTS
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
    temporal_layout: wgpu::BindGroupLayout,
    /// The store (minimal until the first bake or live queue, [`Lighting::bake_pool_bytes`]) and
    /// the bake's pipeline, built on the first bake.
    store: crate::bake::Store,
    bake: Option<crate::bake::BakePipeline>,
    /// The edit boxes since the last dig re-bake ([`Frame::rebake_digs`]), every one, while the
    /// store holds a piece.
    dig_boxes: Vec<[[i32; 3]; 2]>,
    /// The live queue ([`Frame::start_bake_queue`]), when the store fills a frame at a time.
    bake_queue: Option<crate::bake::BakeQueue>,
    /// The live queue's world group and the buffers it binds ([`Frame::queue_world_bind`]).
    queue_world_bind: Option<(Vec<wgpu::Buffer>, wgpu::BindGroup)>,
    /// The temporal pass, its `COUNT_STORE` off: it writes no counter.
    temporal: wgpu::ComputePipeline,
    /// The temporal pass with `COUNT_STORE` on, built on the first encode that counts
    /// ([`Frame::set_count_store`]).
    temporal_counting: std::sync::OnceLock<wgpu::ComputePipeline>,
    /// Whether the temporal pass counts the store's reads ([`Frame::set_count_store`]).
    count_store: bool,
    /// The store's five read counters (`counters` in `temporal.wgsl`): missing, brick reads,
    /// coarse reads, fallback, still black.
    store_counters: wgpu::Buffer,
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
    /// Blocking reads of the store's counters ([`Frame::read_store_stats`]) so far.
    store_blocking_reads: std::sync::atomic::AtomicU64,
    /// The store's counters, copied out by [`Frame::copy_store_stats`].
    store_ring: ReadbackRing,
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

/// A-trous iterations per frame, at strides 1, 2, 4, 8.
pub(crate) const ATROUS_ITERATIONS: usize = 4;

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
pub(crate) const LIGHT_TABLE_ENTRIES: usize = 1024;

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
            let d2 = c.distance2(ChunkCoord(cam));
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
        // The light pass's pipeline (`shade`) is built on the first encode that runs it.
        let light_layout = io_layout(device, "light", &[Io::Float, Io::Uint, f16]);
        let targets = Targets::new(
            device,
            width,
            height,
            LightingResolution::default().scale(),
            (internal != output).then_some(output),
            true,
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
                Io::ReadStorage,
                Io::Storage,
            ],
        );
        let store_counters = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("store counters"),
            size: STORE_STATS_BYTES,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let gradient_layout = io_layout(device, "gradient", &[Io::Float, Io::Uint, Io::Float, f16]);
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
            light: std::sync::OnceLock::new(),
            temporal_layout,
            store: crate::bake::Store::new(device, crate::bake::MINIMAL_POOL_BYTES),
            bake: None,
            dig_boxes: Vec::new(),
            bake_queue: None,
            queue_world_bind: None,
            temporal,
            temporal_counting: std::sync::OnceLock::new(),
            count_store: false,
            store_counters,
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
            store_blocking_reads: std::sync::atomic::AtomicU64::new(0),
            store_ring: ReadbackRing::new(device, "store stats readback", STORE_STATS_BYTES),
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
    /// A store holding a piece also keeps the box for [`Frame::rebake_digs`]; under the live
    /// queue the box goes to the queue instead, which re-bakes its rings over its slices.
    pub fn invalidate(&mut self, min: [i32; 3], max: [i32; 3]) {
        if self.store.piece().is_some() && self.bake_queue.is_none() {
            self.dig_boxes.push([min, max]);
        }
        if let Some(q) = self.bake_queue.as_mut() {
            q.invalidate(min, max);
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
    }

    /// Group 0 of the frame's shaders over `world`.
    /// [`Frame::world_bind`] for the live queue's step, made again only when one of the buffers
    /// it binds was replaced (a mirror shard or the light-sample buffer grown, say).
    fn queue_world_bind(&mut self, device: &wgpu::Device, world: &GpuWorld) -> wgpu::BindGroup {
        let key = vec![
            self.params.clone(),
            world.pool_shard(0).clone(),
            world.pool_shard(1).clone(),
            world.pool_shard(2).clone(),
            world.pool_shard(3).clone(),
            world.records().clone(),
            world.table().clone(),
            world.materials().clone(),
            world.light_buffer().clone(),
            self.emission_scale.clone(),
            self.exposure.state_uniform().clone(),
        ];
        match &self.queue_world_bind {
            Some((k, bind)) if *k == key => bind.clone(),
            _ => {
                let bind = self.world_bind(device, world);
                self.queue_world_bind = Some((key, bind.clone()));
                bind
            }
        }
    }

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
    /// the bake's reach. `world` is the voxels `gpu` mirrors, which `gpu` must hold already. The
    /// first bake allocates the pool ([`Lighting::bake_pool_bytes`]). `Err` when the pool or the
    /// light samples cannot hold the extent.
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
        self.fit_store(true);
        hold_reach(device, queue, gpu, world, extent)?;
        self.dig_boxes.clear();
        let params = self.bake_params(gpu, settings);
        let world_bind = self.world_bind(device, gpu);
        if self.bake.is_none() {
            self.bake = Some(crate::bake::BakePipeline::new(device, &self.world_layout));
        }
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

    /// Starts the live queue (ADR-0046's scheduler): the store emptied for `gpu`'s window, every
    /// block with brick records of `settings`' cell, filled by [`Frame::bake_queue_step`] a slice
    /// of `slice_ms` a frame; the first allocates the pool ([`Lighting::bake_pool_bytes`]).
    pub fn start_bake_queue(
        &mut self,
        queue: &wgpu::Queue,
        gpu: &GpuWorld,
        settings: crate::bake::BakeSettings,
        slice_ms: f64,
    ) {
        self.fit_store(true);
        self.store.start_queue(queue, gpu.window(), settings.cell);
        self.dig_boxes.clear();
        self.bake_queue = Some(crate::bake::BakeQueue::new(settings, slice_ms));
    }

    /// The lighting grid's pixels: what the store's missing count is a share of.
    pub fn light_pixels(&self) -> u32 {
        let [w, h] = self.targets.light_size();
        w * h
    }

    /// The live queue, if started.
    pub fn bake_queue(&self) -> Option<&crate::bake::BakeQueue> {
        self.bake_queue.as_ref()
    }

    /// The live queue, if started, to drain its readings
    /// ([`crate::bake::BakeQueue::take_angle_needs`]).
    pub fn bake_queue_mut(&mut self) -> Option<&mut crate::bake::BakeQueue> {
        self.bake_queue.as_mut()
    }

    /// The live queue's slice, in milliseconds a frame ([`crate::bake::Budget::PerFrame`]).
    pub fn set_bake_slice(&mut self, ms: f64) {
        self.set_bake_budget(crate::bake::Budget::PerFrame { ms });
    }

    /// The live queue's bake budget (ADR-0052).
    pub fn set_bake_budget(&mut self, budget: crate::bake::Budget) {
        if let Some(q) = self.bake_queue.as_mut() {
            q.budget = budget;
        }
    }

    /// The live queue's coarse pass: `samples` a brick direction (0 none) for a chunk's first
    /// block beyond `near` chunks (Chebyshev) of the camera's.
    pub fn set_bake_coarse(&mut self, samples: u32, near: i32) {
        if let Some(q) = self.bake_queue.as_mut() {
            q.coarse_samples = samples;
            q.near = near;
            q.full_first = Some(near);
        }
    }

    /// The live queue's full-first radius ([`crate::bake::BakeQueue::full_first`]): the chunks
    /// (Chebyshev) around the camera's within which a chunk with no block takes the full count
    /// directly; `None` gives every such chunk the coarse pass first.
    pub fn set_bake_full_first(&mut self, radius: Option<i32>) {
        if let Some(q) = self.bake_queue.as_mut() {
            q.full_first = radius;
        }
    }

    /// The live queue's reach mode ([`crate::bake::BakeQueue::reach_mode`]): whether a chunk
    /// whose reach crosses the streamer's full radius bakes from the chunks resident full.
    pub fn set_bake_reach(&mut self, mode: crate::bake::ReachMode) {
        if let Some(q) = self.bake_queue.as_mut() {
            q.set_reach_mode(mode);
        }
    }

    /// The live queue's digs: the slice's growth while a dig's bricks wait, and whether a dig
    /// that touched lit bricks re-bakes the chunks within the light radius (ring 3).
    pub fn set_bake_digs(&mut self, edit_ms: f64, reach_ring: bool) {
        if let Some(q) = self.bake_queue.as_mut() {
            q.edit_ms = edit_ms;
            q.reach_ring = reach_ring;
        }
    }

    /// Whether each of `chunks` has had the live queue's first pass
    /// ([`crate::bake::BakeQueue::passed`]); true without the queue.
    pub fn bake_passed(&self, chunks: &[ChunkCoord]) -> bool {
        self.bake_queue
            .as_ref()
            .is_none_or(|q| chunks.iter().all(|&c| q.passed(&self.store, c)))
    }

    /// Hands the live queue the stream workers' prepared masks ([`crate::bake::FacePrep`]).
    pub fn take_prepared(
        &mut self,
        prepared: &[(ChunkCoord, vxe_world::Prepared)],
        world: &vxe_world::World,
    ) {
        if let Some(q) = self.bake_queue.as_mut()
            && !prepared.is_empty()
        {
            q.take_prepared(prepared, world.materials());
        }
    }

    /// Whether `c` holds its block in the live queue's store, or is known to need none (`absent`:
    /// the world holds no chunk there). False without the queue.
    pub fn bake_settled(&self, c: ChunkCoord, absent: bool) -> bool {
        self.bake_queue
            .as_ref()
            .is_some_and(|q| q.settled(&self.store, c, absent))
    }

    /// One frame of the live queue, ahead of the encode: the light-sample ranges held out to
    /// `view.hold` (by default the window's far corner plus the light radius), then [`crate::bake::BakeQueue::step`], which
    /// waits on the GPU only without a slice (`view.samples`). `gpu` must already hold this frame's sync, submitted. `None` without
    /// the queue.
    pub fn bake_queue_step(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu: &mut GpuWorld,
        world: &vxe_world::World,
        view: &crate::bake::QueueView<'_>,
    ) -> Option<Result<crate::bake::QueueStats, crate::bake::BakeError>> {
        let t0 = crate::bake::stopwatch();
        let settings = self.bake_queue.as_ref()?.settings.clone();
        let w = gpu.window();
        let f = gpu.focus().0;
        let size = w.size();
        let far: i64 = (0..3)
            .map(|k| {
                let lo = i64::from(w.min.0[k] - f[k]).abs();
                let hi = i64::from(w.min.0[k] + size[k] - 1 - f[k]).abs();
                lo.max(hi).pow(2)
            })
            .sum();
        gpu.hold_lights(
            view.hold
                .unwrap_or((far as f64).sqrt().ceil() as i32 + gpu.light_radius() + 1),
        );
        let params = self.bake_params(gpu, &settings);
        let world_bind = self.queue_world_bind(device, gpu);
        if self.bake.is_none() {
            self.bake = Some(crate::bake::BakePipeline::new(device, &self.world_layout));
        }
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
        let q = self.bake_queue.as_mut()?;
        Some(q.step(&run, &mut self.store, view, t0))
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

    /// The edit boxes waiting for [`Frame::rebake_digs`], `min ..= max` in voxels.
    pub fn pending_digs(&self) -> &[[[i32; 3]; 2]] {
        &self.dig_boxes
    }

    /// Whether the temporal pass counts the store's reads (off by default): on, it runs the
    /// pipeline whose `COUNT_STORE` is true and [`Frame::read_store_stats`] and
    /// [`Frame::try_store_stats`] read its counts; off, the counters stay zero and the pass
    /// does no atomic write.
    pub fn set_count_store(&mut self, on: bool) {
        self.count_store = on;
    }

    /// Whether the temporal pass counts the store's reads ([`Frame::set_count_store`]).
    pub fn counts_store(&self) -> bool {
        self.count_store
    }

    /// The store's counters; `missing` is a blocking read of the last submitted encode's
    /// count (0 with [`Frame::set_count_store`] off), counted in
    /// [`Frame::store_blocking_reads`].
    pub fn read_store_stats(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> crate::bake::StoreStats {
        self.store_blocking_reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let bytes =
            crate::readback::read_buffer(device, queue, &self.store_counters, 0, STORE_STATS_BYTES);
        self.store_stats_of(&bytes)
    }

    /// Records a copy of the store's GPU counters into a read-back ring, for
    /// [`Frame::try_store_stats`]; call after [`Frame::encode`] in the same encoder, and submit
    /// it before the next read.
    pub fn copy_store_stats(&self, encoder: &mut wgpu::CommandEncoder) {
        self.store_ring.copy(encoder, &self.store_counters, 0);
    }

    /// The store's counters as of the newest [`Frame::copy_store_stats`] that has finished on
    /// the GPU, without waiting, and how many copies older than the last one recorded it is (1
    /// or 2 with the GPU keeping up and a copy a frame); `None` until the first has finished.
    pub fn try_store_stats(&self, device: &wgpu::Device) -> Option<(crate::bake::StoreStats, u64)> {
        self.store_ring.try_read(device, |b| self.store_stats_of(b))
    }

    /// The store's counts with its GPU counters' five words.
    fn store_stats_of(&self, bytes: &[u8]) -> crate::bake::StoreStats {
        let word =
            |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
        crate::bake::StoreStats {
            fallback: word(12),
            still_black: word(16),
            ..self.store.stats(word(0), word(4), word(8))
        }
    }

    /// What a cube face with no brick value reads (ADR-0053); [`crate::bake::MissingLight::Chain`]
    /// by default.
    pub fn set_missing_light(&mut self, queue: &wgpu::Queue, mode: crate::bake::MissingLight) {
        self.store.set_missing_light(queue, mode);
    }

    /// What a cube face with no brick value reads.
    pub fn missing_light(&self) -> crate::bake::MissingLight {
        self.store.missing_light()
    }

    /// How many times [`Frame::read_store_stats`] waited on the GPU so far.
    pub fn store_blocking_reads(&self) -> u64 {
        self.store_blocking_reads
            .load(std::sync::atomic::Ordering::Relaxed)
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

    /// The temporal pass's pipeline: with [`Frame::set_count_store`] on, the one whose
    /// `COUNT_STORE` is true, built on its first use.
    fn temporal_pipeline(&self, device: &wgpu::Device) -> &wgpu::ComputePipeline {
        if !self.count_store {
            return &self.temporal;
        }
        self.temporal_counting.get_or_init(|| {
            compute_pipeline_with(
                device,
                "temporal counting",
                format!("{MARCH_WGSL}\n{TEMPORAL_WGSL}"),
                &[Some(&self.world_layout), None, Some(&self.temporal_layout)],
                "temporal",
                &[("COUNT_STORE", 1.0)],
            )
        })
    }

    /// The light pass's pipeline (`shade`), built on its first use.
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

    /// Sets the lighting knobs; a change of [`Lighting::bake_pool_bytes`] rebuilds an allocated
    /// store empty.
    pub fn set_lighting(&mut self, lighting: Lighting) {
        self.lighting = lighting;
        self.fit_store(false);
        self.rebuild_probes();
    }

    /// Rebuilds the store empty where its pool's size changes: [`Lighting::bake_pool_bytes`]
    /// within the device's storage binding, once `allocate` or a bake has allocated it; a minimal
    /// one before.
    fn fit_store(&mut self, allocate: bool) {
        if !allocate && self.store.pool().size() <= crate::bake::MINIMAL_POOL_BYTES {
            return;
        }
        let limits = self.device.limits();
        let bytes = self
            .lighting
            .bake_pool_bytes
            .min(limits.max_storage_buffer_binding_size)
            .min(limits.max_buffer_size)
            & !3;
        if bytes != self.store.pool().size() {
            let missing = self.store.missing_light();
            self.store = crate::bake::Store::new(&self.device, bytes);
            // The new pool's header, the flags with it, is written at its first reset.
            self.store.keep_missing_light(missing);
        }
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

    /// Bytes the frame's targets, its probe volume (once built), FSR 2's resources and the store's
    /// pool (once allocated) occupy.
    pub fn frame_bytes(&self) -> u64 {
        self.targets.bytes()
            + self.probes.get().map_or(0, ProbeVolume::bytes)
            + self.exposure.bytes()
            + self.fsr.as_ref().map_or(0, Fsr2::bytes)
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
            h.overflow = false;
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
        // The store's counters: missing, brick reads, coarse reads.
        encoder.clear_buffer(&self.store_counters, 0, None);
        let light_bind = bind_io(
            device,
            &self.light_layout,
            &[
                Res::Tex(t.lit_pos(c)),
                Res::Tex(t.lit_info(c)),
                Res::Tex(&t.irradiance),
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
                Res::Buf(self.store.pool()),
                Res::Buf(&self.store_counters),
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
            &t.accum[c]
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
        // The gradient pass runs in a shaded view for the gradient mode's lambda.
        let gradient_on = self.look.view.shades() && p.lighting & LIGHTING_GRADIENT != 0;
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
                        blue_noise: &self.blue_noise,
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
        if gradient_on {
            gradient.push(frame_pass(&self.gradient, &gradient_bind, 3, strata));
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
            vec![frame_pass(self.light_pipeline(device), &light_bind, 2, lw)],
            vec![frame_pass(
                self.temporal_pipeline(device),
                &temporal_bind,
                2,
                lw,
            )],
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
                d.record(&mut pass);
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

impl Dispatch<'_> {
    /// Sets the pipeline and the bind groups on `pass` and dispatches.
    fn record(&self, pass: &mut wgpu::ComputePass<'_>) {
        pass.set_pipeline(self.pipeline);
        for (g, bind) in self.groups.iter().enumerate() {
            if let Some(b) = bind {
                pass.set_bind_group(g as u32, *b, &[]);
            }
        }
        match self.indirect {
            Some((args, offset)) => pass.dispatch_workgroups_indirect(args, offset),
            None => pass.dispatch_workgroups(self.workgroups[0], self.workgroups[1], 1),
        }
    }
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
/// timestamps `2 i` and `2 i + 1` of the timer it is given.
/// The nine before the last are FSR 2's ([`crate::fsr2::FSR2_PASSES`]); in FSR mode `upscale`
/// and `sharpen` are empty, and otherwise FSR's are. `exposure` runs last, after either.
pub const FRAME_PASSES: [&str; 20] = [
    "hit",
    "gradient",
    "light",
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

/// Bytes of the store's GPU counters that [`Frame::read_store_stats`] reads: five `u32` words
/// (missing, brick reads, coarse reads, fallback, still black).
const STORE_STATS_BYTES: u64 = 20;

/// Read-back buffers a [`ReadbackRing`] cycles through: with the GPU keeping up, a frame's
/// copy is mapped while the next two are recorded.
pub(crate) const READBACK_RING: usize = 3;

/// A ring slot's progress.
enum RingPhase {
    Idle,
    /// Holds the copy of this index; not yet asked to map.
    Copied(u64),
    /// Asked to map the copy of this index: the flag reads 0 pending, 1 mapped, 2 failed.
    Mapping(u64, std::sync::Arc<std::sync::atomic::AtomicU8>),
}

struct RingSlot {
    buffer: wgpu::Buffer,
    phase: RingPhase,
}

struct RingState {
    slots: Vec<RingSlot>,
    /// Copies recorded so far; the next copy's index.
    copies: u64,
    /// The newest copy collected and its bytes (the vector is reused).
    newest: Option<u64>,
    bytes: Vec<u8>,
}

/// A ring of [`READBACK_RING`] mappable buffers that reads a GPU buffer's bytes without waiting:
/// [`ReadbackRing::copy`] records a copy into a free slot, [`ReadbackRing::try_read`] asks the
/// copied slots to map, polls without blocking and hands over the newest copy that has finished
/// mapping, a frame or two late. [`ReadbackRing::read`] is the blocking form, for tests and
/// tools.
///
/// Every encoder holding a [`ReadbackRing::copy`] must be submitted before the next
/// [`ReadbackRing::try_read`] or [`ReadbackRing::read`]: those ask the copied slots to map, and
/// wgpu refuses a submit that uses a buffer with a map pending. When every slot is mapping (the
/// GPU three copies behind) a copy is skipped; when none is free but one holds a copy not yet
/// asked to map, the oldest such is overwritten.
pub(crate) struct ReadbackRing {
    state: std::sync::Mutex<RingState>,
    size: u64,
}

impl ReadbackRing {
    /// A ring of `size`-byte slots (`size` a multiple of 4, as a buffer copy needs).
    pub(crate) fn new(device: &wgpu::Device, label: &str, size: u64) -> Self {
        let slots = (0..READBACK_RING)
            .map(|_| RingSlot {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                phase: RingPhase::Idle,
            })
            .collect();
        Self {
            state: std::sync::Mutex::new(RingState {
                slots,
                copies: 0,
                newest: None,
                bytes: Vec::with_capacity(size as usize),
            }),
            size,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, RingState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Records a copy of the ring's size from `src` at `offset` into a slot; first collects the
    /// slots that finished mapping. Returns the copy's index, or `None` when it was skipped.
    pub(crate) fn copy(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        src: &wgpu::Buffer,
        offset: u64,
    ) -> Option<u64> {
        let mut st = self.lock();
        st.collect();
        let at = st.copies;
        let oldest_copied = st
            .slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| match s.phase {
                RingPhase::Copied(c) => Some((c, i)),
                _ => None,
            })
            .min()
            .map(|(_, i)| i);
        let slot = st
            .slots
            .iter()
            .position(|s| matches!(s.phase, RingPhase::Idle))
            .or(oldest_copied)?;
        encoder.copy_buffer_to_buffer(src, offset, &st.slots[slot].buffer, 0, self.size);
        st.slots[slot].phase = RingPhase::Copied(at);
        st.copies += 1;
        Some(at)
    }

    /// Asks the copied slots to map, polls `device` without waiting, collects the finished
    /// ones, and returns `f` of the newest copy collected so far with its age in copies (0 for
    /// the last one recorded); `None` until the first has finished mapping.
    pub(crate) fn try_read<T>(
        &self,
        device: &wgpu::Device,
        f: impl FnOnce(&[u8]) -> T,
    ) -> Option<(T, u64)> {
        let mut st = self.lock();
        st.request_maps();
        let _ = device.poll(wgpu::PollType::Poll);
        st.collect();
        st.newest_with(f)
    }

    /// As [`ReadbackRing::try_read`], but waits for the GPU first, so the newest copy is the
    /// last one recorded (age 0) unless it was skipped.
    pub(crate) fn read<T>(
        &self,
        device: &wgpu::Device,
        f: impl FnOnce(&[u8]) -> T,
    ) -> Option<(T, u64)> {
        let mut st = self.lock();
        st.request_maps();
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("device poll for the read-back ring");
        st.collect();
        st.newest_with(f)
    }
}

impl RingState {
    /// Asks every slot holding a copy to map.
    fn request_maps(&mut self) {
        for slot in &mut self.slots {
            if let RingPhase::Copied(at) = slot.phase {
                let done = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0));
                let flag = done.clone();
                slot.buffer.map_async(wgpu::MapMode::Read, .., move |r| {
                    flag.store(
                        if r.is_ok() { 1 } else { 2 },
                        std::sync::atomic::Ordering::Release,
                    );
                });
                slot.phase = RingPhase::Mapping(at, done);
            }
        }
    }

    /// Takes the bytes of every slot that finished mapping, keeping the newest copy's, and
    /// frees those slots.
    fn collect(&mut self) {
        for slot in &mut self.slots {
            let RingPhase::Mapping(at, done) = &slot.phase else {
                continue;
            };
            let at = *at;
            match done.load(std::sync::atomic::Ordering::Acquire) {
                0 => continue,
                1 => {
                    if self.newest.is_none_or(|n| at > n) {
                        let bytes = slot
                            .buffer
                            .get_mapped_range(..)
                            .expect("read-back slot mapped");
                        self.bytes.clear();
                        self.bytes.extend_from_slice(&bytes);
                        self.newest = Some(at);
                    }
                    slot.buffer.unmap();
                }
                _ => {}
            }
            slot.phase = RingPhase::Idle;
        }
    }

    fn newest_with<T>(&self, f: impl FnOnce(&[u8]) -> T) -> Option<(T, u64)> {
        let at = self.newest?;
        Some((f(&self.bytes), self.copies - 1 - at))
    }
}

/// GPU durations of one or more passes through a query set of two timestamps per pass: pass
/// `i` writes its start and end into timestamps `2 i` and `2 i + 1`, [`PassTimer::resolve`]
/// copies them out after the passes into a [`ReadbackRing`], [`PassTimer::try_read_all_ms`]
/// hands over the newest set that has finished without waiting, and
/// [`PassTimer::read_all_ms`] waits for the last.
///
/// The resolve buffer (`QUERY_RESOLVE | COPY_SRC`) and the ring's buffers (`COPY_DST |
/// MAP_READ`) are separate because wgpu forbids one buffer carrying both usages. One query set
/// and one resolve buffer serve every frame: the queue runs a frame's resolve and copy before
/// the next frame's passes write the queries again. The timestamp period is read off the queue
/// once, at construction.
pub struct PassTimer {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    ring: ReadbackRing,
    /// The newest copy [`PassTimer::try_read_all_ms`] handed over, plus one; 0 before any.
    handed: std::sync::atomic::AtomicU64,
    /// Nanoseconds per timestamp tick.
    period_ns: f32,
    passes: u32,
    /// Reads that waited on the GPU so far ([`PassTimer::read_all_ms`]).
    blocking_reads: std::sync::atomic::AtomicU64,
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
            ring: ReadbackRing::new(device, "pass timer readback", bytes),
            handed: std::sync::atomic::AtomicU64::new(0),
            period_ns: queue.get_timestamp_period(),
            passes,
            blocking_reads: std::sync::atomic::AtomicU64::new(0),
        })
    }

    pub fn passes(&self) -> u32 {
        self.passes
    }

    /// How many reads waited on the GPU so far: [`PassTimer::read_all_ms`] and
    /// [`PassTimer::read_ms`] each count one.
    pub fn blocking_reads(&self) -> u64 {
        self.blocking_reads
            .load(std::sync::atomic::Ordering::Relaxed)
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

    /// Records the resolve of every timestamp and their copy into the ring; call after the
    /// timed passes, in the same or a later encoder, and submit it before the next read.
    pub fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        let n = 2 * self.passes;
        encoder.resolve_query_set(&self.queries, 0..n, &self.resolve, 0);
        self.ring.copy(encoder, &self.resolve, 0);
    }

    /// Milliseconds of pass 0 of the last resolve, waiting; see [`PassTimer::read_all_ms`].
    pub fn read_ms(&self, device: &wgpu::Device) -> f64 {
        self.read_all_ms(device)[0]
    }

    /// Milliseconds of pass 0 of the newest finished resolve, without waiting; see
    /// [`PassTimer::try_read_all_ms`].
    pub fn try_read_ms(&self, device: &wgpu::Device) -> Option<f64> {
        self.try_read_all_ms(device).map(|ms| ms[0])
    }

    /// Milliseconds between each pass's two timestamps of the last resolve, by pass; zero
    /// when they are out of order, and every pass zero before any resolve. Blocks until the
    /// GPU has finished the submission that resolved them, and counts in
    /// [`PassTimer::blocking_reads`].
    pub fn read_all_ms(&self, device: &wgpu::Device) -> Vec<f64> {
        self.blocking_reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.ring
            .read(device, |b| self.to_ms(b))
            .map_or_else(|| vec![0.0; self.passes as usize], |(ms, _)| ms)
    }

    /// As [`PassTimer::read_all_ms`] for the newest resolve that has finished on the GPU,
    /// without waiting: a non-blocking poll, then the set if it is newer than the last this
    /// call handed over, else `None` (none finished yet, or none new since). With the GPU
    /// keeping up the set is one or two frames old.
    pub fn try_read_all_ms(&self, device: &wgpu::Device) -> Option<Vec<f64>> {
        let (ms, age) = self.ring.try_read(device, |b| self.to_ms(b))?;
        let next = self.ring.lock().copies - age;
        let handed = self
            .handed
            .fetch_max(next, std::sync::atomic::Ordering::Relaxed);
        (next > handed).then_some(ms)
    }

    /// Milliseconds by pass from a resolve's timestamp bytes.
    fn to_ms(&self, bytes: &[u8]) -> Vec<f64> {
        let t: &[u64] = bytemuck::cast_slice(bytes);
        t.chunks(2)
            .map(|p| p[1].saturating_sub(p[0]) as f64 * f64::from(self.period_ns) / 1.0e6)
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
