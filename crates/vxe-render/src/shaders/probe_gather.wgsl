// The probe volume's reads (ADR-0023) and the gather pass: the indirect irradiance at each
// lighting pixel's hit from the eight probes around it. Compiled after march.wgsl, whose
// bindings (group 0), hit-record helpers and lighting grid it uses; probe_update.wgsl is
// compiled after this file and shares its bindings and reads.
//
// Bindings, group 3 (every probe pass):
//   0  storage  probe_data     the atlases and state words (`probes.rs` has the layout):
//                              irradiance tile of slot s, texel x + 8 y (6 x 6 plus a border),
//                              words 2 (64 s + x + 8 y) and + 1 as f16 r, g, b, pad;
//                              visibility tile, words VIS_BASE + 256 s + x + 16 y (14 x 14
//                              plus a border) as f16 mean distance, mean squared distance;
//                              state, word STATE_BASE + s: bit 0 inside rock, bits 1..8 age
//                              in updates (0: never traced), bits 8..32 the tag
//                              floor(w / 32) & 0xFF per axis of the world probe w it holds
//   1  storage  probe_scratch  the frame's update list (MAX_UPDATES words: slot, bit 31
//                              touched by an edit), then per entry and ray the radiance and
//                              signed distance as four f16 (probe_update.wgsl)
//   2  uniform  probe          the volume's lowest world probe, the spacing in voxels, the
//                              update list's length
//
// Bindings, group 2 of `gather`:
//   0  texture_2d<f32>  hit_pos_in    the lighting grid's hit positions
//   1  texture_2d<u32>  hit_info_in   the lighting grid's hit records
//   2  storage texture  indirect_out  rgba16float: the gathered irradiance at the hit, rgb;
//                                     0 on a miss and in the cutaway view
//
// World probe w (probe steps) sits at the centre of voxel w * spacing and lives in slot
// w mod 32 per axis, x fastest. A probe is live when its slot's tag is w's, its age is above 0
// and it is not inside rock. The irradiance at a point x with normal n (`probe_irradiance`)
// blends the eight probes of the cell holding x + PROBE_NORMAL_BIAS n, each live one weighted
// by its trilinear weight, by `((1 + cos) / 2)^2 + 0.2` of the angle between n and the
// direction to the probe, and by the Chebyshev bound of its visibility map toward the point
// (the chance the point is not behind the probe's mean occluder in that direction, cubed);
// the weights are renormalized over the live probes, 0 when none is.

struct ProbeParams {
    origin: vec3<i32>,
    spacing: u32,
    updates: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

@group(3) @binding(0) var<storage, read_write> probe_data: array<u32>;
@group(3) @binding(1) var<storage, read_write> probe_scratch: array<u32>;
@group(3) @binding(2) var<uniform> probe: ProbeParams;

const PROBE_AXIS: i32 = 32;
const PROBE_COUNT: u32 = 32768u;
const IRR_N: u32 = 6u;
const IRR_EDGE: u32 = 8u;
const VIS_N: u32 = 14u;
const VIS_EDGE: u32 = 16u;
const VIS_BASE: u32 = 4194304u; // PROBE_COUNT * 64 * 2
const STATE_BASE: u32 = 12582912u; // VIS_BASE + PROBE_COUNT * 256
const PROBE_MAX_AGE: u32 = 127u;
// How far along the normal the gathered point moves off the surface, in voxels: keeps a
// probe's visibility test from reading the surface itself as its occluder.
const PROBE_NORMAL_BIAS: f32 = 2.0;

// The slot of world probe `w`.
fn probe_slot(w: vec3<i32>) -> u32 {
    let m = ((w % PROBE_AXIS) + PROBE_AXIS) % PROBE_AXIS;
    return u32(m.x + PROBE_AXIS * (m.y + PROBE_AXIS * m.z));
}

// The tag of world probe `w`: floor(w / 32) & 0xFF per axis.
fn probe_tag(w: vec3<i32>) -> u32 {
    let t = bitcast<vec3<u32>>(w >> vec3<u32>(5u)) & vec3<u32>(0xFFu);
    return t.x | (t.y << 8u) | (t.z << 16u);
}

// Whether state word `s` is world probe `w`'s, traced and not inside rock.
fn probe_live(s: u32, w: vec3<i32>) -> bool {
    return (s >> 8u) == probe_tag(w) && ((s >> 1u) & PROBE_MAX_AGE) > 0u && (s & 1u) == 0u;
}

// World probe `w`'s position in voxels.
fn probe_position(w: vec3<i32>) -> vec3<f32> {
    return vec3<f32>(w * i32(probe.spacing)) + 0.5;
}

// The octahedral coordinates in [-1, 1]^2 of unit `d`.
fn probe_oct(d: vec3<f32>) -> vec2<f32> {
    var p = d.xy / (abs(d.x) + abs(d.y) + abs(d.z));
    if (d.z < 0.0) {
        let s = select(vec2<f32>(-1.0), vec2<f32>(1.0), p >= vec2<f32>(0.0));
        p = (1.0 - abs(p.yx)) * s;
    }
    return p;
}

// The unit direction of octahedral coordinates `p` in [-1, 1]^2.
fn probe_unoct(p: vec2<f32>) -> vec3<f32> {
    var d = vec3<f32>(p, 1.0 - abs(p.x) - abs(p.y));
    if (d.z < 0.0) {
        let s = select(vec2<f32>(-1.0), vec2<f32>(1.0), d.xy >= vec2<f32>(0.0));
        d = vec3<f32>((1.0 - abs(d.yx)) * s, d.z);
    }
    return normalize(d);
}

// Binary16 bits `h` (low 16 bits) as f32; subnormals included, infinity for exponent 31. The
// built-in pack2x16float family needs a shader capability not every backend validates with.
fn half_to_f32(h: u32) -> f32 {
    let sign = (h & 0x8000u) << 16u;
    let e = (h >> 10u) & 0x1Fu;
    let m = h & 0x3FFu;
    if (e == 0u) {
        return bitcast<f32>(sign | bitcast<u32>(f32(m) * 5.9604645e-8));
    }
    if (e == 31u) {
        return bitcast<f32>(sign | 0x7F800000u);
    }
    return bitcast<f32>(sign | ((e + 112u) << 23u) | (m << 13u));
}

// `v` as binary16 bits, rounded to nearest (ties away from zero); past the largest finite
// half it is infinity, under the smallest normal it flushes to zero.
fn f32_to_half(v: f32) -> u32 {
    let bits = bitcast<u32>(v);
    let sign = (bits >> 16u) & 0x8000u;
    let e = i32((bits >> 23u) & 0xFFu) - 112;
    let m = bits & 0x7FFFFFu;
    if (e >= 31) {
        return sign | 0x7C00u;
    }
    if (e <= 0) {
        return sign;
    }
    return (sign | (u32(e) << 10u) | (m >> 13u)) + ((m >> 12u) & 1u);
}

// Two binary16 values packed low first, as f32.
fn unpack_half2(w: u32) -> vec2<f32> {
    return vec2<f32>(half_to_f32(w & 0xFFFFu), half_to_f32(w >> 16u));
}

fn pack_half2(v: vec2<f32>) -> u32 {
    return f32_to_half(v.x) | (f32_to_half(v.y) << 16u);
}

fn irradiance_texel(slot: u32, x: u32, y: u32) -> vec3<f32> {
    let i = 2u * (slot * IRR_EDGE * IRR_EDGE + x + IRR_EDGE * y);
    return vec3<f32>(unpack_half2(probe_data[i]), unpack_half2(probe_data[i + 1u]).x);
}

fn visibility_texel(slot: u32, x: u32, y: u32) -> vec2<f32> {
    return unpack_half2(probe_data[VIS_BASE + slot * VIS_EDGE * VIS_EDGE + x + VIS_EDGE * y]);
}

// The bilinear tap position in a tile of interior edge `n` for direction `d`: the texel
// below and the fraction toward the next, borders included.
struct Tap {
    base: vec2<u32>,
    f: vec2<f32>,
};

fn probe_tap(d: vec3<f32>, n: u32) -> Tap {
    // Interior texel i (1 ..= n) is centred at i + 0.5; uv 0 and 1 land on the border's centres'
    // inner halves, so the four taps stay inside the tile.
    let c = (probe_oct(d) * 0.5 + 0.5) * f32(n) + 0.5;
    let b = clamp(floor(c), vec2<f32>(0.0), vec2<f32>(f32(n)));
    return Tap(vec2<u32>(b), clamp(c - b, vec2<f32>(0.0), vec2<f32>(1.0)));
}

// Slot `slot`'s irradiance toward normal `d`, bilinear.
fn probe_irradiance_toward(slot: u32, d: vec3<f32>) -> vec3<f32> {
    let t = probe_tap(d, IRR_N);
    let a = mix(irradiance_texel(slot, t.base.x, t.base.y), irradiance_texel(slot, t.base.x + 1u, t.base.y), t.f.x);
    let b = mix(irradiance_texel(slot, t.base.x, t.base.y + 1u), irradiance_texel(slot, t.base.x + 1u, t.base.y + 1u), t.f.x);
    return mix(a, b, t.f.y);
}

// Slot `slot`'s mean distance and mean squared distance toward `d`, bilinear.
fn probe_visibility_toward(slot: u32, d: vec3<f32>) -> vec2<f32> {
    let t = probe_tap(d, VIS_N);
    let a = mix(visibility_texel(slot, t.base.x, t.base.y), visibility_texel(slot, t.base.x + 1u, t.base.y), t.f.x);
    let b = mix(visibility_texel(slot, t.base.x, t.base.y + 1u), visibility_texel(slot, t.base.x + 1u, t.base.y + 1u), t.f.x);
    return mix(a, b, t.f.y);
}

// The irradiance the probes give a surface at `x` with unit normal `n` (see the header).
fn probe_irradiance(x: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let s = f32(probe.spacing);
    let xb = x + n * PROBE_NORMAL_BIAS;
    let q = (xb - 0.5) / s - vec3<f32>(probe.origin);
    let base = vec3<i32>(floor(q));
    let f = q - floor(q);
    var sum = vec3<f32>(0.0);
    var w_sum = 0.0;
    for (var i = 0u; i < 8u; i++) {
        let off = vec3<i32>(i32(i & 1u), i32((i >> 1u) & 1u), i32(i >> 2u));
        let idx = base + off;
        if (any(idx < vec3<i32>(0)) || any(idx >= vec3<i32>(PROBE_AXIS))) {
            continue;
        }
        let w = probe.origin + idx;
        let slot = probe_slot(w);
        if (!probe_live(probe_data[STATE_BASE + slot], w)) {
            continue;
        }
        let tri = select(1.0 - f, f, off == vec3<i32>(1));
        let pp = probe_position(w);
        var weight = 1.0;
        let to_probe = pp - x;
        let len = length(to_probe);
        if (len > 1e-4) {
            let c = (dot(to_probe / len, n) + 1.0) * 0.5;
            weight = c * c + 0.2;
        }
        // The Chebyshev bound: a point farther from the probe than its mean occluder in that
        // direction is likely behind it.
        let v = xb - pp;
        let dist = length(v);
        if (dist > 1e-4) {
            let m = probe_visibility_toward(slot, v / dist);
            if (dist > m.x) {
                let variance = abs(m.y - m.x * m.x);
                let d = dist - m.x;
                let cheb = variance / (variance + d * d);
                weight *= cheb * cheb * cheb;
            }
        }
        weight *= tri.x * tri.y * tri.z;
        sum += weight * probe_irradiance_toward(slot, n);
        w_sum += weight;
    }
    if (w_sum > 0.0) {
        return sum / w_sum;
    }
    return vec3<f32>(0.0);
}

@group(2) @binding(0) var gather_pos_in: texture_2d<f32>;
@group(2) @binding(1) var gather_info_in: texture_2d<u32>;
@group(2) @binding(2) var indirect_out: texture_storage_2d<rgba16float, write>;

@compute @workgroup_size(8, 8, 1)
fn gather(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.light_size.x || gid.y >= params.light_size.y) {
        return;
    }
    let px = vec2<i32>(gid.xy);
    let rec = unpack_hit(textureLoad(gather_pos_in, px, 0), textureLoad(gather_info_in, px, 0));
    var e = vec3<f32>(0.0);
    if (rec.found && (params.flags & FLAG_CUTAWAY) == 0u) {
        e = probe_irradiance(rec.p, surface_detail(rec).xyz);
    }
    textureStore(indirect_out, px, vec4<f32>(e, 0.0));
}
