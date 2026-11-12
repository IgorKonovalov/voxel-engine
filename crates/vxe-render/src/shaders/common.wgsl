// What every pass of the frame shares: the views and `Params`, group 0's bindings, the
// flag bits, the pixel and view helpers, the lamp, rand, the exposure and luminance, and the
// emission scale. A pass's source joins this and the modules it uses (`frame/compose.rs`).
//
// Bindings, group 0 (every pass of the frame):
//   0  uniform  params         views, mirror window, light, target size, flags (bit 0: shadow ray,
//                              bit 1: occupancy masks, bits 2..4: normal mode,
//                              bits 5..8: view, bit 9: skip empty cells larger than a voxel,
//                              bit 10: cutaway, bit 19: the split field walk), the lighting
//                              flags, exposure and the face mix
//   1  storage  brick_pool0    shard 0 of the pool: [unit][16 x u32], 64-byte units; unit u is
//                              unit u % units_per_shard of shard u / units_per_shard; 7, 8 and
//                              9 are shards 1 to 3. A Full brick's slot is a run of units
//                              (gpu_world.rs): unit 0 its opaque mask, bit i % 32 of word
//                              i / 32 set when voxel i (x + 8 y + 64 z) is neither air nor
//                              TRANSLUCENT; unit 1 its header (words 0..8 the palette, 16 u16
//                              ascending; words 8, 9 the translucent 2^3 block bits, bit
//                              bx + 4 by + 16 bz); then 0, 1, 2 or 4 bit planes in the mask's
//                              layout (plane k is bit k of a voxel's palette index), or for
//                              class 5 (Wide) 16 units of u16 ids, voxel i in half i % 2 of
//                              word i / 2; then, for a planar brick holding a TRANSLUCENT
//                              voxel, one unit of its translucent voxels. A band brick's
//                              header names its band slot (word 15) and holds its band
//                              prefix (words 12 and 13, see the field walk)
//   2  storage  chunk_records  [record][528 x u32]: 16 words of brick bits (bit b of word
//                              b / 32 set when entry b is not uniform air), then 512 brick
//                              entries. Entry bit 31 set: the slot's first unit in bits 0..26
//                              and its class in bits 26..29 (1 to 4: 0, 1, 2 or 4 planes; 5:
//                              Wide); clear: the uniform material in bits 0..16; bit 29 set
//                              either way when the brick holds a TRANSLUCENT voxel
//   3  storage  chunk_table    [wrapped chunk]u32 record index in bits 0..29, bit 29 set when
//                              the record's brick bits are all clear, bit 30 set for a LOD
//                              record (entries 0..64 mip level 1, 64..72 level 2, 72 level 3);
//                              0xFFFFFFFF when absent
//   4  storage  materials      [id][4 x vec4]: albedo rgb + roughness, emission rgb + flag bits,
//                              2 spare + opacity + 1 spare, 4 spare
//   7  storage  brick_pool1    pool shard 1
//   8  storage  brick_pool2    pool shard 2
//   9  storage  brick_pool3    pool shard 3
//  10  storage  light_samples  [sample]vec4<i32>: an exposed emissive voxel's world coordinate,
//                              then its material (bits 0..16) and the faces it shows (bit
//                              16 + 2 k toward -k, the next toward +k); material 0 is a hole.
//                              Declared by light.wgsl
//  11  storage  emission_scale [material]f32, the frame's emission scale in the light the
//                              pixels sample, then EMISSION_ROWS more in the light the probe
//                              rays sample (`emission_base`)
//  12  texture  blue_noise     128 x 128 r8unorm blue noise (void and cluster)
//  13  uniform  exposure_state the exposure state the last frame's exposure pass wrote
//                              (exposure.wgsl, ADR-0033): `exposure_now` and `exposure_ratio`

struct View {
    origin: vec3<f32>,
    tan_left: f32,
    right: vec3<f32>,
    tan_right: f32,
    up: vec3<f32>,
    tan_up: f32,
    fwd: vec3<f32>,
    tan_down: f32,
    rect: vec4<u32>,
    aspect: f32,
    // (L + R) / unit and L / unit, rounded once on the CPU.
    unit_scale: f32,
    unit_offset: f32,
    _pad0: f32,
    // The rays' sub-pixel offset in internal pixels (`View::jitter`).
    jitter: vec2<f32>,
    _pad1: vec2<f32>,
};

struct Params {
    views: array<View, 2>,
    view_count: u32,
    translucent_layers: u32,
    frame: u32,
    lighting: u32,
    window_min: vec3<i32>,
    // A field-surface hit's shading normal is mixed this far toward its entry face's normal
    // (`SurfaceTuning::face_mix`); 0 keeps the field's.
    face_mix: f32,
    window_size: vec3<i32>,
    _pad3: i32,
    light_pos: vec3<f32>,
    light_intensity: f32,
    light_dir: vec3<f32>,
    cos_outer: f32,
    size: vec2<u32>,
    cos_inner: f32,
    flags: u32,
    units_per_shard: u32,
    forced_level: u32,
    lod1_r2: i32,
    lod2_r2: i32,
    exposure: f32,
    _pad4: u32,
    candidates: u32,
    // The slot of light_samples where the frame's candidate table starts (light.wgsl).
    light_table_base: u32,
    // Bit 0: the previous frame's hit buffer and history are valid.
    history: u32,
    _pad5: u32,
    _pad6: u32,
    _pad7: u32,
    // The views of the previous frame, for reprojection.
    prev_views: array<View, 2>,
    // The lighting grid: its size, the full-resolution pixels per lighting pixel on an edge
    // (1 or 2), and this and last frame's offsets of the sampled pixel in each 2 x 2 (x in
    // bits 0..8, y in bits 8..16).
    light_size: vec2<u32>,
    light_scale: u32,
    light_offset: u32,
    prev_light_offset: u32,
    _pad8: u32,
    // The size of the frame's image (the upscale pass's output); `size` without the upscaler.
    output_size: vec2<u32>,
    _pad10: u32,
    // The face look's far shading normal (`LookParams::far_normal`), in pixels per voxel:
    // `far_hi <= far_lo` is off.
    far_lo: f32,
    far_hi: f32,
    _pad9: u32,
    // FSR 2's tone pass's static dither (`Fsr2Settings::dither`, 1 on), 0 without FSR 2.
    dither: u32,
    _tail0: u32,
    _tail1: u32,
    _tail2: u32,
};

@group(0) @binding(0) var<uniform> params: Params;

@group(0) @binding(1) var<storage, read> brick_pool0: array<u32>;

@group(0) @binding(2) var<storage, read> chunk_records: array<u32>;

@group(0) @binding(3) var<storage, read> chunk_table: array<u32>;

@group(0) @binding(4) var<storage, read> materials: array<vec4<f32>>;

@group(0) @binding(7) var<storage, read> brick_pool1: array<u32>;

@group(0) @binding(8) var<storage, read> brick_pool2: array<u32>;

@group(0) @binding(9) var<storage, read> brick_pool3: array<u32>;

@group(0) @binding(11) var<storage, read> emission_scale: array<f32>;

@group(0) @binding(12) var blue_noise: texture_2d<f32>;

// `ExposureState` in exposure.rs: the exposure this frame uses in auto (`current`) and the one
// the last frame used (`previous`), then the adapted and metered levels, a counter and flags.
struct ExposureState {
    current: f32,
    previous: f32,
    adapted_log2: f32,
    metered_log2: f32,
    frame: u32,
    flags: u32,
    _pad0: u32,
    _pad1: u32,
};

@group(0) @binding(13) var<uniform> exposure_state: ExposureState;

const BIG: f32 = 3.0e38;

// Params.flags bits: the prelude's FLAG_SHADOWS, FLAG_MASKS, FLAG_SKIPS, FLAG_CUTAWAY and
// FLAG_FIELD_SPLIT (the field look's split walk: the hit pipeline's `main` walks to the first
// field brick and hands the ray to `field_resume`, `split_pixel`), the normal mode at
// NORMAL_SHIFT and the view at VIEW_SHIFT.
const VIEW_BITS: u32 = 7u;

// Normal modes (flags bits 2..4) and views (flags bits 5..8).
const NORMAL_FACE: u32 = 0u;
const NORMAL_TRILINEAR8: u32 = 1u;
const NORMAL_GRADIENT27: u32 = 2u;
const NORMAL_CONTOUR: u32 = 3u;
const VIEW_SHADED: u32 = 0u;
const VIEW_NORMALS: u32 = 1u;
const VIEW_HITMASK: u32 = 2u;
const VIEW_STEPS: u32 = 3u;
const VIEW_ALBEDO: u32 = 4u;
const VIEW_CRACKS: u32 = 5u;

// Params.lighting bits are the prelude's LIGHTING_* (frame/settings.rs). Among them:
// LIGHTING_AUTO_EXPOSURE, the exposure is automatic (this frame's is the state's `current`, not
// `params.exposure`); LIGHTING_STORE_BY_NORMAL, a cube face reads the store by its shading
// normal (temporal.wgsl, ADR-0049); LIGHTING_FACE_WEIGHTS, the half-resolution light's filters
// compare face normals (`filter_normal`, ADR-0049).

const PI: f32 = 3.14159265358979;

// True in the lane of axis `k`. A component store through a runtime index (`v[k] = x`) is
// not addressable under FXC, the D3D12 HLSL compiler, so writes go through
// `select(v, vec3(x), axis_mask(k))` instead; reads through a runtime index are fine.
fn axis_mask(k: i32) -> vec3<bool> {
    return vec3<i32>(0, 1, 2) == vec3<i32>(k);
}

// `c` (each channel in 0..65408) as an rgb9e5 word: 9-bit mantissas over a shared exponent,
// red in the low bits, the exponent (bias 15) in the top five; `bake::rgb9e5_pack`'s rounding.
fn t9e5_pack(c: vec3<f32>) -> u32 {
    let v = clamp(c, vec3<f32>(0.0), vec3<f32>(65408.0));
    let top = max(v.x, max(v.y, v.z));
    if (top <= 0.0) {
        return 0u;
    }
    let e2 = i32(bitcast<u32>(top) >> 23u) - 127;
    var e = max(e2, -16) + 1 + 15;
    var scale = exp2(f32(e - 24));
    if (floor(top / scale + 0.5) >= 512.0) {
        e += 1;
        scale *= 2.0;
    }
    let m = vec3<u32>(floor(v / scale + 0.5));
    return m.x | (m.y << 9u) | (m.z << 18u) | (u32(e) << 27u);
}

// The value of an rgb9e5 word (`t9e5_pack`).
fn t9e5_unpack(w: u32) -> vec3<f32> {
    let m = vec3<u32>(w & 511u, (w >> 9u) & 511u, (w >> 18u) & 511u);
    return vec3<f32>(m) * exp2(f32(i32(w >> 27u) - 24));
}

// The lamp's irradiance at `p` before any surface term: intensity over squared distance inside
// the cone, the terms `shade` applies to an opaque hit.
fn lamp(p: vec3<f32>) -> f32 {
    let to_light = params.light_pos - p;
    let dist2 = max(dot(to_light, to_light), 1e-4);
    let l = to_light * inverseSqrt(dist2);
    let cone = smoothstep(params.cos_outer, params.cos_inner, dot(-l, params.light_dir));
    return params.light_intensity * cone / dist2;
}

// The headlamp's `intensity * cone * max(n.l, 0) / dist^2` at `p`, normal `n`, without the
// shadow ray (light.wgsl's `lamp_lit` adds it); 0 when the lamp is off.
fn lamp_term(on: bool, pos: vec3<f32>, dir: vec3<f32>, intensity: f32, cos_o: f32, cos_i: f32, p: vec3<f32>, n: vec3<f32>) -> f32 {
    if (!on) {
        return 0.0;
    }
    let to_light = pos - p;
    let dist2 = max(dot(to_light, to_light), 1e-4);
    let l = to_light * inverseSqrt(dist2);
    let cone = smoothstep(cos_o, cos_i, dot(-l, dir));
    return intensity * cone * max(dot(n, l), 0.0) / dist2;
}

fn normal_mode() -> u32 {
    return (params.flags >> NORMAL_SHIFT) & 3u;
}

fn view_mode() -> u32 {
    return (params.flags >> VIEW_SHIFT) & VIEW_BITS;
}

// The cutaway view's miss colour, linear: a flat grey-blue the rooms read against.
const CUTAWAY_SKY: vec3<f32> = vec3<f32>(0.30, 0.38, 0.50);

// A packed lighting-grid offset (x in bits 0..8, y in 8..16).
fn unpack_offset(o: u32) -> vec2<u32> {
    return vec2<u32>(o & 0xFFu, (o >> 8u) & 0xFFu);
}

// The full-resolution pixel lighting pixel `lp` samples this frame, inside the target.
fn full_px(lp: vec2<u32>) -> vec2<u32> {
    return min(lp * params.light_scale + unpack_offset(params.light_offset), params.size - vec2<u32>(1u));
}

// A fractional full-resolution pixel position of last frame (centres at + 0.5) on last
// frame's lighting grid, centres at + 0.5.
fn prev_light_grid(q: vec2<f32>) -> vec2<f32> {
    let o = vec2<f32>(unpack_offset(params.prev_light_offset));
    return (q - 0.5 - o) / f32(params.light_scale) + 0.5;
}

// Whether lighting pixel `q` lies on the lighting grid.
fn on_light_grid(q: vec2<i32>) -> bool {
    return all(q >= vec2<i32>(0)) && all(q < vec2<i32>(params.light_size));
}

// The ray of one pixel.
struct Ray {
    o: vec3<f32>,
    d: vec3<f32>,
};

// The index of the first view whose rectangle holds pixel `px`, 0 when none does.
fn view_of(px: vec2<u32>) -> u32 {
    for (var i = 0u; i < min(params.view_count, MAX_VIEWS); i++) {
        let r = params.views[i].rect;
        if (all(px >= r.xy) && all(px < r.xy + r.zw)) {
            return i;
        }
    }
    return 0u;
}

// The primary ray of pixel `px` through the centre of the pixel moved by the view's jitter
// (zero: the centre itself, bit for bit), in its view. Horizontal
// tangents are taken in units of `unit = hv aspect` (hv the mean vertical tangent): with
// `u = (px - x + 0.5) / w`, `x = ((u (L + R) / unit - L / unit) hv) aspect`, the two ratios
// rounded on the CPU (`View::unit_scale`, `unit_offset`). A symmetric
// view of `View::of_camera` has the factors 2 and 1 exactly, so it rounds as
// `((2 u - 1) tan) aspect`; two views of one aspect whose rectangles cover the same pixels of
// the same frustum (a view and its left half over half the pixels) take bit-identical rays.
fn pixel_ray(px: vec2<u32>) -> Ray {
    let v = params.views[view_of(px)];
    let u = (vec2<f32>(px - v.rect.xy) + 0.5 + v.jitter) / vec2<f32>(v.rect.zw);
    let hv = (v.tan_up + v.tan_down) * 0.5;
    let cv = (v.tan_up - v.tan_down) * 0.5;
    let nx = u.x * v.unit_scale - v.unit_offset;
    let ny = u.y * 2.0 - 1.0;
    let d = normalize(v.fwd + v.right * (nx * hv * v.aspect) - v.up * (ny * hv - cv));
    return Ray(v.origin, d);
}

// The bake's random numbers (bake.wgsl): `rand` reads `px` as a stream and a sample's index and
// hashes them with the dimension, with no blue noise and no frame.
override RAND_HASHED: bool = false;

// A 32-bit integer hash (Wellons' lowbias32).
fn hash_u32(x: u32) -> u32 {
    var h = x;
    h ^= h >> 16u;
    h *= 0x7feb352du;
    h ^= h >> 15u;
    h *= 0x846ca68bu;
    h ^= h >> 16u;
    return h;
}

// Random number `dim` of pixel `px` in this frame, in [0, 1). The blue-noise texel at `px`
// shifted by an offset hashed from `dim` (blue in space), plus a Weyl rotation by the frame
// times a per-dimension multiple of the golden ratio in 32-bit fixed point (low-discrepancy
// in time). A function of (pixel, frame, dim) alone, so a frame is deterministic on one
// adapter.
fn rand(px: vec2<u32>, dim: u32) -> f32 {
    if (RAND_HASHED) {
        return f32(hash_u32(px.x ^ hash_u32(px.y ^ (dim * 0x9E3779B9u))) >> 8u) / 16777216.0;
    }
    let h = hash_u32(dim * 0x9E3779B9u + 0x632BE5ABu);
    let at = (px + vec2<u32>(h & 127u, (h >> 7u) & 127u)) & vec2<u32>(127u);
    let texel = u32(textureLoad(blue_noise, vec2<i32>(at), 0).x * 255.0 + 0.5);
    let v = (texel << 24u) + params.frame * (0x9E3779B9u * (dim + 1u)) + ((h >> 14u) & 0xFFFFFFu);
    return f32(v >> 8u) / 16777216.0;
}

// The exposure this frame's passes use: the state's `current` in auto, else the manual
// `params.exposure`.
fn exposure_now() -> f32 {
    if ((params.lighting & LIGHTING_AUTO_EXPOSURE) != 0u) {
        return exposure_state.current;
    }
    return params.exposure;
}

// The ratio of this frame's exposure to the last frame's: a pass that reads a value an earlier
// frame stored pre-exposed multiplies it by this (its square for a second moment).
fn exposure_ratio() -> f32 {
    return exposure_now() / exposure_state.previous;
}

// Every store into an rgba16float HDR target is at most this (ADR-0033), and a second moment is
// of a luminance at most MOMENT_LUM_MAX (its square, 65 025, stays finite in f16).
const HDR_MAX: f32 = 60000.0;

const MOMENT_LUM_MAX: f32 = 255.0;

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// Material ids the emission-scale buffer holds per half (`marcher::EMISSION_ROWS`).
const EMISSION_ROWS: u32 = 65536u;

// The half of the emission-scale buffer `emission_of` reads: 0 for the pixels' light,
// EMISSION_ROWS once an entry point (the probe trace) sets it for the probe rays' light.
var<private> emission_base: u32 = 0u;

// The bake's light (bake.wgsl) is at unit emission: `emission_of` leaves the scale out.
override EMISSION_UNIT: bool = false;

// A material's emitted radiance under the frame's emission scale.
fn emission_of(m: u32) -> vec3<f32> {
    if (EMISSION_UNIT) {
        return materials[m * 4u + 1u].xyz;
    }
    return materials[m * 4u + 1u].xyz * emission_scale[emission_base + m];
}

// ---- The field surface (the Contour normal mode, the hit pipeline with FIELD_SURFACE) ----
//
// ADR-0041: the surface is the zero set of `phi = count / 27 - 1/2` trilinearly interpolated
// over the voxel centres, `count` a voxel's solid voxels among the 27 around it, itself
// included (solid is not air, a translucent voxel included). A band brick's header names its
// band slot (HEADER_BAND_WORD: HEADER_BAND_BIT and the slot's first unit; a uniform band brick
// has a class 6 slot for it): the band mask (unit 0, the opaque mask's layout), then the band
// voxels' u8 counts in band-index order, the k-th in byte k % 4 of word k / 4 of unit 1 on;
// the brick's header holds its band prefix (HEADER_BAND_PREFIX_WORD, `layer_prefix`), so a
// voxel's band index is its layer's prefix and the band bits below it in its layer. A
// band voxel's count is read there, any other voxel's is 27 when it is solid and 0 when it is
// air. The walk is `march`'s DDA over the cells that hold a band voxel (0 < count < 27) or a
// solid one: a band brick's band mask ORed with its opaque mask (all set for a uniform solid
// brick, a translucent one included), and a chunk or a group of bricks is skipped only when
// its brick bits are clear. In a band
// voxel the ray's segment through the voxel is split at the voxel's three mid-planes into its
// pieces in the dual cells it crosses (the cubes between eight voxel centres, at most four),
// and on a piece whose cell's eight corners mix signs `phi` along the ray is a cubic in `t`:
// the cubic's extrema split the piece into monotone runs, and the first run that rises to
// zero holds the hit, found by eight steps of the Illinois method. A piece that starts at or
// above zero is a hit at its start. A leak is a ray that reaches a solid voxel outside the
// band, a piece whose corners all lie above zero, or a piece that starts more than
// FIELD_LEAK_EPS above zero: each is hit there as a cube and flagged, in the cracks view's
// place. A LOD-only chunk is walked as `march` walks it, its cells cubes.
//
// A crisp voxel (a voxel of a `MaterialFlags::CRISP` row, its band count carrying
// CRISP_COUNT_BIT) counts as solid in the field and is hit as its cube where the ray enters
// it, with its entry face's normal; no field piece inside it is tested. A dual cell with a
// crisp corner gives a field hit only when one of its corners carries SMOOTH_COUNT_BIT (a
// smooth solid band voxel), so two crisp faces meeting in a concave corner stay a sharp
// corner, and a field hit takes the material of its largest-weight smooth solid corner. A band
// brick whose header carries BAND_CUBES_BIT (no smooth solid voxel in it or its shell, set by
// the field pass) can give no field hit and is walked as `march` walks a brick, its solid
// voxels cubes.
//
// The shading normal is the normalised gradient of the trilinear `phi` at the hit, mixed
// toward the entry face by the face mix (`face_mixed`). The hit record flags a field hit
// (HIT_FIELD), so the light pass starts its shadow rays along the field normal
// (`shadow_normal_of`); they walk the voxels (`march`), not the field. Translucent voxels are
// solid to the field, and the field walk composites no translucent layer.
