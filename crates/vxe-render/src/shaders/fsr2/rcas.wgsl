// FSR 2.2.1's RCAS pass, ported to WGSL (ADR-0034) from tag v2.2.1 of
// GPUOpen-Effects/FidelityFX-FSR2: `ffx_fsr2_rcas_pass.glsl`, `ffx_fsr2_rcas.h` and the
// 32-bit `FsrRcasF` of `ffx_fsr1.h` it includes, with FSR_RCAS_DENOISE. One workgroup of 64
// per 16 x 16 display pixels, four pixels a thread. Compiled after common.wgsl, which carries
// AMD's notice.
//
// RCAS sharpens each pixel against its cross (up, left, right, down) by one negative lobe,
// the largest that keeps every channel inside [0, 1] for a centre within the cross's minimum
// and maximum, capped at RCAS_LIMIT, scaled by `cb.rcas_con` (`2^-stops`) and by the noise
// detection's `nz`. FSR 2 runs it on the exposed colour (`PrepareRgb`) and unprepares the
// result.
//
// Bindings, group 1:
//   0  texture_2d<f32>   rcas_in       the accumulate pass's internal upscaled colour
//   1  texture_2d<f32>   exposure_in   rg32float 1x1
//   2  storage texture   rcas_out      rgba16float: the sharpened colour, unprepared (the
//                                      frame writes it into last frame's copy of the internal
//                                      upscaled colour, which nothing reads after the
//                                      accumulate pass), which the tone pass reads

@group(1) @binding(0) var rcas_in: texture_2d<f32>;
@group(1) @binding(1) var exposure_in: texture_2d<f32>;
@group(1) @binding(2) var rcas_out: texture_storage_2d<rgba16float, write>;

// FSR_RCAS_LIMIT.
const RCAS_LIMIT: f32 = 0.25 - (1.0 / 16.0);

// FsrRcasLoadF: the exposed colour of display pixel `p`.
fn rcas_load(p: vec2<i32>) -> vec3<f32> {
    let c = load_clamped(rcas_in, p).rgb;
    return prepare_rgb(c, exposure_of(exposure_in), cb.pre_exposure);
}

// ffxApproximateReciprocalMedium.
fn approximate_reciprocal_medium(v: f32) -> f32 {
    let b = bitcast<f32>(0x7ef19fffu - bitcast<u32>(v));
    return b * (-b * v + 2.0);
}

fn max3(a: f32, b: f32, c: f32) -> f32 {
    return max(a, max(b, c));
}

fn min3(a: f32, b: f32, c: f32) -> f32 {
    return min(a, min(b, c));
}

// FsrRcasF with the denoise, for display pixel `ip`.
fn fsr_rcas(ip: vec2<i32>) -> vec3<f32> {
    let b = rcas_load(ip + vec2<i32>(0, -1));
    let d = rcas_load(ip + vec2<i32>(-1, 0));
    let e = rcas_load(ip);
    let f = rcas_load(ip + vec2<i32>(1, 0));
    let h = rcas_load(ip + vec2<i32>(0, 1));
    // Luma times 2.
    let b_l = b.b * 0.5 + (b.r * 0.5 + b.g);
    let d_l = d.b * 0.5 + (d.r * 0.5 + d.g);
    let e_l = e.b * 0.5 + (e.r * 0.5 + e.g);
    let f_l = f.b * 0.5 + (f.r * 0.5 + f.g);
    let h_l = h.b * 0.5 + (h.r * 0.5 + h.g);
    // Noise detection.
    var nz = 0.25 * b_l + 0.25 * d_l + 0.25 * f_l + 0.25 * h_l - e_l;
    nz = saturate(abs(nz) * approximate_reciprocal_medium(max(max3(b_l, d_l, e_l), max(f_l, h_l)) - min(min3(b_l, d_l, e_l), min(f_l, h_l))));
    nz = -0.5 * nz + 1.0;
    // The ring's minimum and maximum.
    let mn4 = min(min(b, d), min(f, h));
    let mx4 = max(max(b, d), max(f, h));
    let peak = vec2<f32>(1.0, -4.0);
    // The limiters: high-precision reciprocals.
    let hit_min = mn4 * (1.0 / (4.0 * mx4));
    let hit_max = (vec3<f32>(peak.x) - mx4) * (1.0 / (4.0 * mn4 + vec3<f32>(peak.y)));
    let lobe3 = max(-hit_min, hit_max);
    var lobe = max(-RCAS_LIMIT, min(max3(lobe3.x, lobe3.y, lobe3.z), 0.0)) * cb.rcas_con;
    lobe *= nz;
    // The resolve takes the medium-precision reciprocal, against visible tonality changes.
    let rcp_l = approximate_reciprocal_medium(4.0 * lobe + 1.0);
    return (lobe * b + lobe * d + lobe * h + lobe * f + e) * rcp_l;
}

// ffxRemapForQuad: a 64-lane index to 8 x 8 in quads.
fn remap_for_quad(a: u32) -> vec2<u32> {
    return vec2<u32>(extractBits(a, 1u, 3u), insertBits(extractBits(a, 3u, 3u), a, 0u, 1u));
}

fn curr_filter(p: vec2<u32>) {
    let pos = vec2<i32>(p);
    if (any(pos >= display_size())) {
        return;
    }
    let c = unprepare_rgb(fsr_rcas(pos), exposure_of(exposure_in));
    textureStore(rcas_out, pos, vec4<f32>(c, 1.0));
}

@compute @workgroup_size(64, 1, 1)
fn rcas(@builtin(local_invocation_id) lid: vec3<u32>, @builtin(workgroup_id) wg: vec3<u32>) {
    var gxy = remap_for_quad(lid.x) + vec2<u32>(wg.x << 4u, wg.y << 4u);
    curr_filter(gxy);
    gxy.x += 8u;
    curr_filter(gxy);
    gxy.y += 8u;
    curr_filter(gxy);
    gxy.x -= 8u;
    curr_filter(gxy);
}
