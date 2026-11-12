//! The frame's settings: the uniform block ([`Params`]) and its flag bits, the views and
//! normal modes, the look, the LOD, the lighting and their enums.

use super::*;

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
pub(super) const SPLIT_GROUP: u32 = 128;

/// Workgroups a dispatch may count on one dimension (WebGPU's default limit).
pub(super) const MAX_WORKGROUPS: u32 = 65535;

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
pub(super) const HALF_OFFSET: [u32; 2] = [0, 1];

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
    pub(super) fn probe_steady(&self) -> u32 {
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

/// A-trous iterations per frame, at strides 1, 2, 4, 8.
pub(crate) const ATROUS_ITERATIONS: usize = 4;

/// Edit boxes one frame takes; one more flushes all history.
pub const MAX_EDITS: usize = 64;

/// The gradient pass's stratum edge in pixels; `STRATUM` in `gradient.wgsl`.
pub const GRADIENT_STRATUM: u32 = 3;

#[cfg(test)]
mod tests {
    use super::RenderScale;

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
