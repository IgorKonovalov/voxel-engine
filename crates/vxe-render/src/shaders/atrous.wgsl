// The spatial filter: an a-trous wavelet filter (Dammertz 2010) over the accumulated
// irradiance, weighted by the hits' positions and normals and by the luminance variance
// estimated from the temporal moments (SVGF, Schied 2017). Irradiance only: the composite
// multiplies the albedo back after it, so a material edge never blurs. Compiled after
// march.wgsl, whose bindings (group 0) and hit-record helpers it uses.
//
// Two entry points:
//   `variance` writes each hit's irradiance and its luminance variance: from the moments
//   (`m2 - m1^2`) once the pixel has VARIANCE_AGE frames of history, else from the moments of
//   its 7 x 7 neighbourhood under the same edge weights as the filter.
//   `atrous` is one iteration with the 5 x 5 B3-spline kernel at stride `step.size` (1, 2, 4,
//   8 over four dispatches): each tap weighs `exp(-plane distance / SIGMA_PLANE)`,
//   `max(n.n', 0)^128` and `exp(-|l - l'| / (SIGMA_LUM sqrt(var) + eps))`, `var` the
//   pixel's variance blurred by a 3 x 3 Gaussian; the variance is carried with squared
//   weights. A miss keeps its input.
//
// Bindings, group 2 of `variance`:
//   0  texture_2d<f32>   hit_pos_in      this frame's hit positions
//   1  texture_2d<u32>   hit_info_in     this frame's hit records
//   2  texture_2d<f32>   accum_in        the temporal pass's accumulated irradiance
//   3  texture_2d<f32>   moments_in      the temporal pass's moments and age
//   4  storage texture   filter_out      rgba16float: irradiance rgb, variance a
// Bindings, group 2 of `atrous`:
//   0  texture_2d<f32>   hit_pos_in
//   1  texture_2d<u32>   hit_info_in
//   2  texture_2d<f32>   filter_in       the previous iteration's irradiance and variance
//   5  storage texture   filter_out      rgba16float, the same
//   6  uniform           step            the iteration's stride in pixels

@group(2) @binding(0) var hit_pos_in: texture_2d<f32>;
@group(2) @binding(1) var hit_info_in: texture_2d<u32>;
@group(2) @binding(2) var src_in: texture_2d<f32>;

struct Step {
    size: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

const VARIANCE_AGE: f32 = 4.0;
const SIGMA_PLANE: f32 = 1.0;
const SIGMA_LUM: f32 = 4.0;

fn load_hit(q: vec2<i32>) -> HitRecord {
    return unpack_hit(textureLoad(hit_pos_in, q, 0), textureLoad(hit_info_in, q, 0));
}

fn inside(q: vec2<i32>) -> bool {
    return on_light_grid(q);
}

// The geometric edge weight between hit `a` and hit `b`: the distance of `b` from `a`'s
// tangent plane and the angle between their normals.
fn edge_weight(a: HitRecord, b: HitRecord) -> f32 {
    let plane = abs(dot(a.n, b.p - a.p));
    // max(n.n', 0)^128 by seven squarings.
    var c = max(dot(a.n, b.n), 0.0);
    c *= c;
    c *= c;
    c *= c;
    c *= c;
    c *= c;
    c *= c;
    c *= c;
    return exp(-plane / SIGMA_PLANE) * c;
}

@group(2) @binding(3) var moments_in: texture_2d<f32>;
@group(2) @binding(4) var filter_out: texture_storage_2d<rgba16float, write>;

@compute @workgroup_size(8, 8, 1)
fn variance(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.light_size.x || gid.y >= params.light_size.y) {
        return;
    }
    let px = vec2<i32>(gid.xy);
    let c = textureLoad(src_in, px, 0).xyz;
    let m = textureLoad(moments_in, px, 0).xyz;
    let rec = load_hit(px);
    var v = max(m.y - m.x * m.x, 0.0);
    if (rec.found && m.z < VARIANCE_AGE) {
        var w_sum = 0.0;
        var mm = vec2<f32>(0.0);
        for (var y = -3; y <= 3; y++) {
            for (var x = -3; x <= 3; x++) {
                let q = px + vec2<i32>(x, y);
                if (!inside(q)) {
                    continue;
                }
                let other = load_hit(q);
                if (!other.found) {
                    continue;
                }
                let w = edge_weight(rec, other);
                w_sum += w;
                mm += w * textureLoad(moments_in, q, 0).xy;
            }
        }
        if (w_sum > 0.0) {
            mm /= w_sum;
            v = max(mm.y - mm.x * mm.x, 0.0);
        }
    }
    textureStore(filter_out, px, vec4<f32>(c, v));
}

@group(2) @binding(6) var<uniform> step: Step;

// The B3-spline kernel's 1D weights at offsets -2..2.
const KERNEL = array<f32, 5>(0.0625, 0.25, 0.375, 0.25, 0.0625);

@compute @workgroup_size(8, 8, 1)
fn atrous(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.light_size.x || gid.y >= params.light_size.y) {
        return;
    }
    let px = vec2<i32>(gid.xy);
    let centre = textureLoad(src_in, px, 0);
    let rec = load_hit(px);
    if (!rec.found) {
        textureStore(filter_out_atrous, px, centre);
        return;
    }
    // The pixel's variance under a 3 x 3 Gaussian.
    var var_sum = 0.0;
    var var_w = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let q = px + vec2<i32>(x, y);
            if (inside(q)) {
                let w = select(0.5, 1.0, x == 0) * select(0.5, 1.0, y == 0);
                var_sum += w * textureLoad(src_in, q, 0).w;
                var_w += w;
            }
        }
    }
    let sigma = SIGMA_LUM * sqrt(max(var_sum / var_w, 0.0)) + 1e-4;
    let l = luminance(centre.xyz);
    var w_sum = 0.0;
    var c_sum = vec3<f32>(0.0);
    var v_sum = 0.0;
    let s = i32(step.size);
    for (var y = -2; y <= 2; y++) {
        for (var x = -2; x <= 2; x++) {
            let q = px + vec2<i32>(x, y) * s;
            if (!inside(q)) {
                continue;
            }
            let other = load_hit(q);
            if (!other.found) {
                continue;
            }
            let tap = textureLoad(src_in, q, 0);
            let k = KERNEL[x + 2] * KERNEL[y + 2];
            let w = k * edge_weight(rec, other) * exp(-abs(luminance(tap.xyz) - l) / sigma);
            w_sum += w;
            c_sum += w * tap.xyz;
            v_sum += w * w * tap.w;
        }
    }
    // The centre tap always weighs its kernel weight, so w_sum > 0.
    textureStore(filter_out_atrous, px, vec4<f32>(c_sum / w_sum, v_sum / (w_sum * w_sum)));
}

@group(2) @binding(5) var filter_out_atrous: texture_storage_2d<rgba16float, write>;
