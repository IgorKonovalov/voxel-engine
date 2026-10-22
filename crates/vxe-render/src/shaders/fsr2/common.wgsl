// FidelityFX Super Resolution 2 (FSR 2.2.1), the parts its passes share: the constant block,
// the colour and depth helpers and the sampling helpers, ported to WGSL (ADR-0034) from
// tag v2.2.1 of GPUOpen-Effects/FidelityFX-FSR2: `ffx_fsr2_common.h`, `ffx_fsr2_sample.h`,
// the accessors of `ffx_fsr2_callbacks_glsl.h` and the helpers of `ffx_core_gpu_common.h`
// they call. The FP32 path only (`FFX_HALF` 0). Every pass file is compiled after this one,
// after a line defining `const HDR: bool` (FFX_FSR2_OPTION_HDR_COLOR_INPUT).
//
// The port carries AMD's notice:
//
//   Copyright (c) 2022-2023 Advanced Micro Devices, Inc. All rights reserved.
//
//   Permission is hereby granted, free of charge, to any person obtaining a copy of this
//   software and associated documentation files (the "Software"), to deal in the Software
//   without restriction, including without limitation the rights to use, copy, modify, merge,
//   publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons
//   to whom the Software is furnished to do so, subject to the following conditions:
//
//   The above copyright notice and this permission notice shall be included in all copies or
//   substantial portions of the Software.
//
//   THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED,
//   INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR
//   PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE
//   FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
//   OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
//   DEALINGS IN THE SOFTWARE.
//
// The engine's configuration of FSR 2, fixed here: inverted infinite depth
// (FFX_FSR2_OPTION_INVERTED_DEPTH), motion vectors at the render size
// (FFX_FSR2_OPTION_LOW_RESOLUTION_MOTION_VECTORS) and without jitter, the reference Lanczos
// for the history (FFX_FSR2_OPTION_REPROJECT_USE_LANCZOS_TYPE 0) and the approximation for the
// upsample (type 2), so no Lanczos look-up table is read.
//
// Portable forms (WGSL core, no feature): every bilinear sample of AMD's `s_LinearClamp` is a
// four-tap `textureLoad` here (`bilinear`), clamped to the edge, since the r32float and
// rg32float targets are not filterable in core; a texel load outside the texture is clamped
// to its edge.
//
// Bindings, group 0 of every FSR pass:
//   0  uniform  cb   `Fsr2Constants` in fsr2.rs, field for field (AMD's `cbFSR2`, plus the
//                    luminance pyramid's `cbSPD` mip and workgroup counts and RCAS's
//                    sharpening constant)

struct Fsr2Constants {
    render_size: vec2<i32>,
    max_render_size: vec2<i32>,
    display_size: vec2<i32>,
    input_color_size: vec2<i32>,
    luma_mip_size: vec2<i32>,
    luma_mip_level: i32,
    frame_index: i32,
    device_to_view: vec4<f32>,
    jitter: vec2<f32>,
    motion_vector_scale: vec2<f32>,
    downscale: vec2<f32>,
    motion_vector_jitter_cancellation: vec2<f32>,
    pre_exposure: f32,
    previous_pre_exposure: f32,
    tan_half_fov: f32,
    jitter_sequence_length: f32,
    delta_time: f32,
    dynamic_res_change: f32,
    view_space_to_meters: f32,
    spd_mips: u32,
    spd_work_groups: u32,
    rcas_con: f32,
    _pad0: u32,
    _pad1: u32,
};

@group(0) @binding(0) var<uniform> cb: Fsr2Constants;

// ffx_fsr2_common.h's constants.
const FSR2_FP16_MAX: f32 = 65504.0;
const FSR2_EPSILON: f32 = 1e-3;
const FSR2_TONEMAP_EPSILON: f32 = 1.0 / 65504.0;
const FSR2_FLT_MAX: f32 = 3.402823466e+38;
const RECONSTRUCTED_DEPTH_BILINEAR_WEIGHT_THRESHOLD: f32 = 0.01;
const UPSAMPLE_LANCZOS_WEIGHT_SCALE: f32 = 1.0 / 12.0;
const MAX_ACCUMULATION_LANCZOS_WEIGHT: f32 = 1.0;
const AVERAGE_LANCZOS_WEIGHT_PER_FRAME: f32 = 0.74 * (1.0 / 12.0);
const RESET_AUTO_EXPOSURE_AVERAGE_SMOOTHING: f32 = 1e8;
// LOCK_LIFETIME_REMAINING is component x of a lock status, LOCK_TEMPORAL_LUMA component y.

fn render_size() -> vec2<i32> {
    return cb.render_size;
}

fn display_size() -> vec2<i32> {
    return cb.display_size;
}

fn render_size_f() -> vec2<f32> {
    return vec2<f32>(cb.render_size);
}

fn display_size_f() -> vec2<f32> {
    return vec2<f32>(cb.display_size);
}

// A texel of `t` at `p`, `p` clamped to the texture.
fn load_clamped(t: texture_2d<f32>, p: vec2<i32>) -> vec4<f32> {
    let last = vec2<i32>(textureDimensions(t)) - vec2<i32>(1);
    return textureLoad(t, clamp(p, vec2<i32>(0), last), 0);
}

// AMD's bilinear `s_LinearClamp` sample of `t` at `uv`: the four texels around
// `uv * size - 0.5`, each clamped to the edge, weighted by the fraction.
fn bilinear(t: texture_2d<f32>, uv: vec2<f32>) -> vec4<f32> {
    let size = vec2<f32>(textureDimensions(t));
    let s = uv * size - vec2<f32>(0.5);
    let base = vec2<i32>(floor(s));
    let f = s - floor(s);
    let a = load_clamped(t, base);
    let b = load_clamped(t, base + vec2<i32>(1, 0));
    let c = load_clamped(t, base + vec2<i32>(0, 1));
    let d = load_clamped(t, base + vec2<i32>(1, 1));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// The input exposure, 1 where it reads 0 (`Exposure` in ffx_fsr2_callbacks_glsl.h).
fn exposure_of(t: texture_2d<f32>) -> f32 {
    let e = textureLoad(t, vec2<i32>(0), 0).x;
    return select(e, 1.0, e == 0.0);
}

// ---- ffx_fsr2_common.h ----

struct RectificationBox {
    box_center: vec3<f32>,
    box_vec: vec3<f32>,
    aabb_min: vec3<f32>,
    aabb_max: vec3<f32>,
    box_center_weight: f32,
};

fn rectification_box_reset() -> RectificationBox {
    return RectificationBox(vec3<f32>(0.0), vec3<f32>(0.0), vec3<f32>(FSR2_FLT_MAX), vec3<f32>(-FSR2_FLT_MAX), 0.0);
}

// RectificationBoxAddInitialSample and RectificationBoxAddSample.
fn rectification_box_add_sample(initial: bool, b: ptr<function, RectificationBox>, color_sample: vec3<f32>, sample_weight: f32) {
    let weighted = color_sample * sample_weight;
    if (initial) {
        (*b).aabb_min = color_sample;
        (*b).aabb_max = color_sample;
        (*b).box_center = weighted;
        (*b).box_vec = color_sample * weighted;
        (*b).box_center_weight = sample_weight;
    } else {
        (*b).aabb_min = min((*b).aabb_min, color_sample);
        (*b).aabb_max = max((*b).aabb_max, color_sample);
        (*b).box_center += weighted;
        (*b).box_vec += color_sample * weighted;
        (*b).box_center_weight += sample_weight;
    }
}

// RectificationBoxComputeVarianceBoxData.
fn rectification_box_compute_variance(b: ptr<function, RectificationBox>) {
    (*b).box_center_weight = select(1.0, (*b).box_center_weight, abs((*b).box_center_weight) > FSR2_EPSILON);
    (*b).box_center /= (*b).box_center_weight;
    (*b).box_vec /= (*b).box_center_weight;
    let std_dev = sqrt(abs((*b).box_vec - (*b).box_center * (*b).box_center));
    (*b).box_vec = std_dev;
}

fn min_divided_by_max(v0: f32, v1: f32) -> f32 {
    let m = max(v0, v1);
    return select(0.0, min(v0, v1) / m, m != 0.0);
}

fn ycocg_to_rgb(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(c.x + c.y - c.z, c.x + c.z, c.x - c.y - c.z);
}

fn rgb_to_ycocg(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        0.25 * c.r + 0.5 * c.g + 0.25 * c.b,
        0.5 * c.r - 0.5 * c.b,
        -0.25 * c.r + 0.5 * c.g - 0.25 * c.b,
    );
}

fn rgb_to_luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn rgb_to_perceived_luma(c: vec3<f32>) -> f32 {
    let l = rgb_to_luma(c);
    var p = 0.0;
    if (l <= 216.0 / 24389.0) {
        p = l * (24389.0 / 27.0);
    } else {
        p = pow(l, 1.0 / 3.0) * 116.0 - 16.0;
    }
    return p * 0.01;
}

fn tonemap(c: vec3<f32>) -> vec3<f32> {
    return c / vec3<f32>(max(max(0.0, c.r), max(c.g, c.b)) + 1.0);
}

fn inverse_tonemap(c: vec3<f32>) -> vec3<f32> {
    return c / vec3<f32>(max(FSR2_TONEMAP_EPSILON, 1.0 - max(c.r, max(c.g, c.b))));
}

// ClampLoad: `p + o`, clamped to the texture on the side `o` points to.
fn clamp_load(p: vec2<i32>, o: vec2<i32>, size: vec2<i32>) -> vec2<i32> {
    var r = p + o;
    r.x = select(r.x, max(r.x, 0), o.x < 0);
    r.x = select(r.x, min(r.x, size.x - 1), o.x > 0);
    r.y = select(r.y, max(r.y, 0), o.y < 0);
    r.y = select(r.y, min(r.y, size.y - 1), o.y > 0);
    return r;
}

fn clamp_uv(uv: vec2<f32>, texture_size: vec2<i32>, resource_size: vec2<i32>) -> vec2<f32> {
    let s = uv * vec2<f32>(texture_size);
    let c = max(vec2<f32>(0.5), min(s, vec2<f32>(texture_size) - vec2<f32>(0.5)));
    return c / vec2<f32>(resource_size);
}

fn is_on_screen(p: vec2<i32>, size: vec2<i32>) -> bool {
    return all(vec2<u32>(p) < vec2<u32>(size));
}

fn compute_auto_exposure_from_lavg(lavg_log: f32) -> f32 {
    let lavg = exp(lavg_log);
    let s = 100.0;
    let k = 12.5;
    let exposure_iso100 = log2((lavg * s) / k);
    let q = 0.65;
    let lmax = (78.0 / (q * s)) * pow(2.0, exposure_iso100);
    return 1.0 / lmax;
}

fn compute_hr_pos_from_lr_pos(lr: vec2<i32>) -> vec2<i32> {
    let jittered = vec2<f32>(lr) + 0.5 - cb.jitter;
    let in_hr = (jittered / render_size_f()) * display_size_f();
    return vec2<i32>(floor(in_hr));
}

fn compute_ndc(px: vec2<f32>, size: vec2<i32>) -> vec2<f32> {
    return px / vec2<f32>(size) * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);
}

fn get_view_space_depth(device_depth: f32) -> f32 {
    return cb.device_to_view.y / (device_depth - cb.device_to_view.x);
}

fn get_view_space_depth_in_meters(device_depth: f32) -> f32 {
    return get_view_space_depth(device_depth) * cb.view_space_to_meters;
}

fn get_view_space_position(viewport_pos: vec2<i32>, viewport_size: vec2<i32>, device_depth: f32) -> vec3<f32> {
    let z = get_view_space_depth(device_depth);
    let ndc = compute_ndc(vec2<f32>(viewport_pos), viewport_size);
    let x = cb.device_to_view.z * ndc.x * z;
    let y = cb.device_to_view.w * ndc.y * z;
    return vec3<f32>(x, y, z);
}

// GetMaxDistanceInMeters with inverted depth.
fn get_max_distance_in_meters() -> f32 {
    return get_view_space_depth(0.0) * cb.view_space_to_meters;
}

fn prepare_rgb(rgb: vec3<f32>, exposure: f32, pre_exposure: f32) -> vec3<f32> {
    var c = rgb / pre_exposure;
    c *= exposure;
    return clamp(c, vec3<f32>(0.0), vec3<f32>(FSR2_FP16_MAX));
}

fn unprepare_rgb(rgb: vec3<f32>, exposure: f32) -> vec3<f32> {
    var c = rgb / exposure;
    c *= cb.pre_exposure;
    return c;
}

// GetBilinearSamplingData: the base texel and the fraction; the four offsets are (0, 0),
// (1, 0), (0, 1) and (1, 1), weighted (1 - fx)(1 - fy), fx (1 - fy), (1 - fx) fy and fx fy.
struct BilinearSamplingData {
    base: vec2<i32>,
    frac: vec2<f32>,
};

fn get_bilinear_sampling_data(uv: vec2<f32>, size: vec2<i32>) -> BilinearSamplingData {
    let s = uv * vec2<f32>(size) - vec2<f32>(0.5);
    return BilinearSamplingData(vec2<i32>(floor(s)), fract(s));
}

fn bilinear_offset(i: i32) -> vec2<i32> {
    return vec2<i32>(i & 1, i >> 1);
}

fn bilinear_weight(d: BilinearSamplingData, i: i32) -> f32 {
    let wx = select(1.0 - d.frac.x, d.frac.x, (i & 1) == 1);
    let wy = select(1.0 - d.frac.y, d.frac.y, (i >> 1) == 1);
    return wx * wy;
}

// ---- ffx_fsr2_sample.h ----

fn lanczos2_no_clamp(x: f32) -> f32 {
    let pi = 3.141592653589793;
    if (abs(x) < FSR2_EPSILON) {
        return 1.0;
    }
    return (sin(pi * x) / (pi * x)) * (sin(0.5 * pi * x) / (0.5 * pi * x));
}

fn lanczos2(x: f32) -> f32 {
    return lanczos2_no_clamp(min(abs(x), 2.0));
}

// FSR 1's Lanczos approximation of x * x, which must be at most 4.
fn lanczos2_approx_sq_no_clamp(x2: f32) -> f32 {
    let a = (2.0 / 5.0) * x2 - 1.0;
    let b = (1.0 / 4.0) * x2 - 1.0;
    return ((25.0 / 16.0) * a * a - (25.0 / 16.0 - 1.0)) * (b * b);
}

fn lanczos2_approx_sq(x2: f32) -> f32 {
    return lanczos2_approx_sq_no_clamp(min(x2, 4.0));
}

// Lanczos2 over four colours at offset `t` (the 1-D form of `Lanczos2` in the sample header).
fn lanczos2_4(c0: vec4<f32>, c1: vec4<f32>, c2: vec4<f32>, c3: vec4<f32>, t: f32) -> vec4<f32> {
    let w0 = lanczos2(-1.0 - t);
    let w1 = lanczos2(-0.0 - t);
    let w2 = lanczos2(1.0 - t);
    let w3 = lanczos2(2.0 - t);
    return (w0 * c0 + w1 * c1 + w2 * c2 + w3 * c3) / (w0 + w1 + w2 + w3);
}
