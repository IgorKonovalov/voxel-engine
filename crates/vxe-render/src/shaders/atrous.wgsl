// The spatial filter: an a-trous wavelet filter (Dammertz 2010) over the accumulated
// irradiance, weighted by the hits' positions and normals and by the luminance variance
// estimated from the temporal moments (SVGF, Schied 2017). Irradiance only: the composite
// multiplies the albedo back after it, so a material edge never blurs. It filters the lamp's
// part and a fallback hit's emissive part; a pixel whose emissive part is the store's (the
// moments' w, temporal.wgsl) carries a cell's mean, whose variance is the cell's and not the
// pixel's, so that part passes around the filter and the last iteration adds it back
// (ADR-0043). Compiled after common.wgsl, hitrec.wgsl and the sky's modules, whose bindings (group 0), hit-record and sky helpers it
// uses.
//
// Two entry points:
//   `variance` writes each hit's filtered irradiance and its luminance variance: from the moments
//   (`m2 - m1^2`) once the pixel has VARIANCE_AGE frames of history or, with no light shaded
//   (LIGHTING_DIRECT clear: the lamp off and the list empty), is a store's pixel whose lamp part is zero (its moments are
//   zero), else from the moments of its 7 x 7 neighbourhood under the same edge weights as the
//   filter. With the lamp on a young store pixel in the lamp's shadow keeps the 7 x 7 estimate,
//   which lets the filter soften the shadow's edge into it. A young pixel's pair means are
//   pooled first (`pooled_pair`, unless LIGHTING_HOOK_NO_POOL): its sky term is the pooled
//   sky mean's, and its direct part takes the pooled key mean's difference.
//   `atrous` is one iteration with the 5 x 5 B3-spline kernel at stride `step.size` (1, 2, 4,
//   8 over four dispatches): each tap weighs `exp(-plane distance / SIGMA_PLANE)`,
//   `max(n.n', 0)^128` (on `filter_normal`'s normals, hitrec.wgsl) and
//   `exp(-|l - l'| / (SIGMA_LUM sqrt(var) + eps))`, `var` the pixel's variance blurred by a
//   3 x 3 Gaussian; the variance is carried with squared weights. With LIGHTING_YOUNG_EDGES set,
//   a pixel whose age (the temporal moments' z) is under VARIANCE_AGE drops the luminance
//   factor: its taps weigh by plane and normal alone. A miss keeps its input. With
//   LIGHTING_DIRECT clear, a young pixel under LIGHTING_YOUNG_EDGES aside, a hit whose input
//   is zero and whose blurred variance is zero writes its input (zero), plus the store's part
//   on the last
//   iteration, without the 25 taps: its sigma is the epsilon, so only taps of luminance within
//   about 1e-3 of zero weigh, and with no light shaded those are zero (a store's pixel's input
//   is its direct part). With a light shaded, taps lit faintly by it would weigh, so every hit
//   filters.
//
// Bindings, group 2 of `variance`:
//   0  texture_2d<f32>   hit_pos_in      this frame's hit positions
//   1  texture_2d<u32>   hit_info_in     this frame's hit records
//   2  texture_2d<f32>   accum_in        the temporal pass's accumulated irradiance
//                                        (rgba16float)
//   3  texture_2d<f32>   moments_in      the temporal pass's moments and age
//   4  storage texture   filter_out      rgba16float: irradiance rgb, variance a
//   9  texture_2d<f32>   direct_acc_in   the temporal pass's accumulated direct part (rgb, its
//                                        luminance in a)
//  10  texture_2d<f32>   vis_in          this frame's visibility samples (shade.wgsl's
//                                        `vis_out`, rgba8unorm: 1 in a where the sky ray was cast)
//  11  texture_2d<f32>   pair_in         the visibility pair the temporal pass wrote this frame
//                                        (the sky visibility's long mean in r)
//  12  storage, read     sky_e           `E_sky` per direction (`sky_irradiance.wgsl`), as the
//                                        temporal pass binds it
// Bindings, group 2 of `atrous`:
//   0  texture_2d<f32>   hit_pos_in
//   1  texture_2d<u32>   hit_info_in
//   2  texture_2d<f32>   filter_in       the previous iteration's irradiance and variance
//   5  storage texture   filter_out      rgba16float, the same
//   6  uniform           step            the iteration's stride in pixels, and whether it is
//                                        the last
//   7  texture_2d<f32>   accum_pass_in   the temporal pass's accumulation, read by the last
//                                        iteration for the store's emissive part
//   8  texture_2d<f32>   moments_pass_in the temporal pass's moments (z: the age; w: the part is
//                                        the store's, 2 with the pair's sky term in it)
//  10  texture_2d<f32>   vis_in          this frame's visibility samples: a tap whose cast flag
//                                        (a > 0.5, w under 1.5) differs from the centre's
//                                        weighs 0

@group(2) @binding(0) var hit_pos_in: texture_2d<f32>;
@group(2) @binding(1) var hit_info_in: texture_2d<u32>;
@group(2) @binding(2) var src_in: texture_2d<f32>;

struct Step {
    size: u32,
    // 1 on the last iteration, which adds the store's emissive part back.
    last: u32,
    _pad1: u32,
    _pad2: u32,
};

const VARIANCE_AGE: f32 = 4.0;
const SIGMA_PLANE: f32 = 1.0;
const SIGMA_LUM: f32 = 4.0;
// The f16 moments' rounding residue as a share of the second moment: three steps of 2^-11.
const MOMENT_ROUNDING: f32 = 0.0015;

fn load_hit(q: vec2<i32>) -> HitRecord {
    return unpack_hit(textureLoad(hit_pos_in, q, 0), textureLoad(hit_info_in, q, 0));
}

fn inside(q: vec2<i32>) -> bool {
    return on_light_grid(q);
}

// The eye of lighting pixel `q`'s view, for `filter_normal`.
fn eye_of(q: vec2<i32>) -> vec3<f32> {
    return params.views[view_of(full_px(vec2<u32>(q)))].origin;
}

// The geometric edge weight between hit `a` and hit `b` seen from `eye`: the distance of `b`
// from `a`'s tangent plane (along `a`'s shading normal) and the angle between their
// `filter_normal`s (the shading normals, or with Params.lighting bit 15 a cube face's face
// normal).
fn edge_weight(a: HitRecord, b: HitRecord, eye: vec3<f32>) -> f32 {
    let plane = abs(dot(a.n, b.p - a.p));
    // max(n.n', 0)^128 by seven squarings.
    var c = max(dot(filter_normal(a, eye), filter_normal(b, eye)), 0.0);
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
@group(2) @binding(9) var direct_acc_in: texture_2d<f32>;
@group(2) @binding(10) var vis_in: texture_2d<f32>;
@group(2) @binding(11) var pair_in: texture_2d<f32>;
@group(2) @binding(12) var<storage, read> sky_e: array<vec4<f32>, 6>;

// The age-weighted pool (ADR-0076's young pixels): a pixel's pair means are mixed toward the
// mean of its 5 x 5 neighbourhood's at weight `1 - age / POOL_AGE`, full at a reset and none
// from POOL_AGE on, so a young pixel's few samples are averaged with its young neighbours'.
// A tap counts where it is a hit whose voxel lies in the centre's chunk, at the edge weight
// (`edge_weight`: the plane and the normal) times its own samples (its mean's age, at least 1).
// The sky's long mean (the pair's x, its samples the pair's w) pools where the centre cast the
// sky ray and its term does not ride with the stored part (the moments' w under 1.5), over taps
// of the same kind; the key light's mean (the pair's y, its samples the direct part's age, the
// moments' z) pools where the key light falls on the centre (the visibility sample's b), over
// taps where it falls too. Returns the sky's term (`sky_pair_term` of the pooled x, 0 where the
// sky does not pool or ride here) and the key's correction to the direct part: its light times
// the pooled mean less its own, pre-exposed. A footprint at weight 0 reads no tap.
const POOL_AGE: f32 = 32.0;
// The world's chunk edge in voxels (`vxe_world::CHUNK_EDGE`).
const POOL_CHUNK: f32 = 64.0;

struct Pooled {
    sky: vec3<f32>,
    key: vec3<f32>,
};

fn pool_chunk(r: HitRecord) -> vec3<i32> {
    return vec3<i32>(floor((r.p - r.n * 0.5) / POOL_CHUNK));
}

fn pooled_pair(px: vec2<i32>, rec: HitRecord, mw: vec4<f32>) -> Pooled {
    let pair = textureLoad(pair_in, px, 0);
    let vis = textureLoad(vis_in, px, 0);
    let sky_here = mw.w < 1.5 && vis.a > 0.5;
    let key_here = vis.b > 0.5;
    let ws = select(0.0, clamp(1.0 - pair.w / POOL_AGE, 0.0, 1.0), sky_here);
    let wk = select(0.0, clamp(1.0 - mw.z / POOL_AGE, 0.0, 1.0), key_here);
    var x = pair.x;
    var y = pair.y;
    if (ws > 0.0 || wk > 0.0) {
        let eye = eye_of(px);
        let chunk = pool_chunk(rec);
        var sx = 0.0;
        var swx = 0.0;
        var sy = 0.0;
        var swy = 0.0;
        for (var j = -2; j <= 2; j++) {
            for (var i = -2; i <= 2; i++) {
                let q = px + vec2<i32>(i, j);
                if (!inside(q)) {
                    continue;
                }
                let other = load_hit(q);
                if (!other.found || any(pool_chunk(other) != chunk)) {
                    continue;
                }
                let e = edge_weight(rec, other, eye);
                if (e <= 0.0) {
                    continue;
                }
                let tp = textureLoad(pair_in, q, 0);
                let tv = textureLoad(vis_in, q, 0);
                let tm = textureLoad(moments_in, q, 0);
                if (ws > 0.0 && tv.a > 0.5 && tm.w < 1.5) {
                    let w = e * max(tp.w, 1.0);
                    sx += w * tp.x;
                    swx += w;
                }
                if (wk > 0.0 && tv.b > 0.5) {
                    let w = e * max(tm.z, 1.0);
                    sy += w * tp.y;
                    swy += w;
                }
            }
        }
        if (swx > 0.0) {
            x = mix(pair.x, sx / swx, ws);
        }
        if (swy > 0.0) {
            y = mix(pair.y, sy / swy, wk);
        }
    }
    var out: Pooled;
    out.sky = select(vec3<f32>(0.0), sky_pair_term(sky_e, x, vis.a, rec.n), mw.w < 1.5);
    out.key = key_light_at(rec.p, rec.n) * (y - pair.y) * exposure_now();
    return out;
}

@compute @workgroup_size(8, 8, 1)
fn variance(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.light_size.x || gid.y >= params.light_size.y) {
        return;
    }
    let px = vec2<i32>(gid.xy);
    // The accumulated direct rgb, plus the emissive rgb where that is a fallback's: the
    // filter's irradiance.
    let acc = textureLoad(src_in, px, 0);
    let direct = textureLoad(direct_acc_in, px, 0);
    let mw = textureLoad(moments_in, px, 0);
    let rec = load_hit(px);
    // The pair's sky term where the ray was cast (temporal.wgsl keeps it out of the direct
    // part): filtered with the rest.
    // A pixel whose moments' w is 2 carries its pair's term in the stored part instead.
    var sky_t = vec3<f32>(0.0);
    var key_t = vec3<f32>(0.0);
    if (rec.found && (params.lighting & LIGHTING_HOOK_NO_POOL) == 0u) {
        let pooled = pooled_pair(px, rec, mw);
        sky_t = pooled.sky;
        key_t = pooled.key;
    } else if (mw.w < 1.5) {
        sky_t = sky_pair_term(sky_e, textureLoad(pair_in, px, 0).x, textureLoad(vis_in, px, 0).a, rec.n);
    }
    let c = max(select(acc.xyz, vec3<f32>(0.0), mw.w > 0.5) + direct.xyz + key_t, vec3<f32>(0.0)) + sky_t;
    let m = mw.xyz;
    // The pixel's own variance, less the moments' rounding: each moment is an f16 (relative step
    // 2^-11), so `m2 - m1^2` of a still, noise-free light is a residue of up to about three steps
    // of m2, positive or negative. Without the floor that residue sets a sigma that depends on
    // the light's magnitude (a dim blue filters where a white of the same channel does not).
    var v = max(m.y - m.x * m.x - MOMENT_ROUNDING * m.y, 0.0);
    let dark_store = mw.w > 0.5 && (params.lighting & LIGHTING_DIRECT) == 0u && direct.w == 0.0;
    if (rec.found && m.z < VARIANCE_AGE && !dark_store) {
        let eye = eye_of(px);
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
                let w = edge_weight(rec, other, eye);
                w_sum += w;
                mm += w * textureLoad(moments_in, q, 0).xy;
            }
        }
        if (w_sum > 0.0) {
            mm /= w_sum;
            v = max(mm.y - mm.x * mm.x, 0.0);
        }
    }
    textureStore(filter_out, px, min(vec4<f32>(c, v), vec4<f32>(HDR_MAX)));
}

@group(2) @binding(6) var<uniform> step: Step;

// Whether pixel `q`'s filtered quantity holds the pair's sky term: the ray was cast there and
// its term does not ride with the stored part (the moments' w 2, temporal.wgsl).
fn filtered_cast(q: vec2<i32>) -> bool {
    return textureLoad(vis_in, q, 0).a > 0.5 && textureLoad(moments_pass_in, q, 0).w < 1.5;
}

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
    let mp = textureLoad(moments_pass_in, px, 0);
    // The store's emissive part, added back after the last iteration.
    var passed = vec3<f32>(0.0);
    if (step.last != 0u && mp.w > 0.5) {
        passed = textureLoad(accum_pass_in, px, 0).xyz;
    }
    // A young pixel under LIGHTING_YOUNG_EDGES weighs its taps by plane and normal only.
    let edges_only = (params.lighting & LIGHTING_YOUNG_EDGES) != 0u && mp.z < VARIANCE_AGE;
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
    if (!edges_only && (params.lighting & LIGHTING_DIRECT) == 0u && var_sum == 0.0 && all(centre.xyz == vec3<f32>(0.0))) {
        textureStore(filter_out_atrous, px, min(vec4<f32>(centre.xyz + passed, centre.w), vec4<f32>(HDR_MAX)));
        return;
    }
    let sigma = SIGMA_LUM * sqrt(max(var_sum / var_w, 0.0)) + 1e-4;
    let l = luminance(centre.xyz);
    // A cast pixel's filtered quantity holds the pair's sky term and a stored pixel's does not
    // (its sky passes around the filter), so the two kinds estimate different quantities: a tap
    // of the other kind weighs 0.
    let centre_cast = filtered_cast(px);
    var w_sum = 0.0;
    var c_sum = vec3<f32>(0.0);
    var v_sum = 0.0;
    let s = i32(step.size);
    let eye = eye_of(px);
    for (var y = -2; y <= 2; y++) {
        for (var x = -2; x <= 2; x++) {
            let q = px + vec2<i32>(x, y) * s;
            if (!inside(q)) {
                continue;
            }
            let other = load_hit(q);
            if (!other.found || filtered_cast(q) != centre_cast) {
                continue;
            }
            let tap = textureLoad(src_in, q, 0);
            let k = KERNEL[x + 2] * KERNEL[y + 2];
            var w = k * edge_weight(rec, other, eye);
            if (!edges_only) {
                w *= exp(-abs(luminance(tap.xyz) - l) / sigma);
            }
            w_sum += w;
            c_sum += w * tap.xyz;
            v_sum += w * w * tap.w;
        }
    }
    // The centre tap always weighs its kernel weight, so w_sum > 0.
    textureStore(filter_out_atrous, px, min(vec4<f32>(c_sum / w_sum + passed, v_sum / (w_sum * w_sum)), vec4<f32>(HDR_MAX)));
}

@group(2) @binding(5) var filter_out_atrous: texture_storage_2d<rgba16float, write>;
@group(2) @binding(7) var accum_pass_in: texture_2d<f32>;
@group(2) @binding(8) var moments_pass_in: texture_2d<f32>;
