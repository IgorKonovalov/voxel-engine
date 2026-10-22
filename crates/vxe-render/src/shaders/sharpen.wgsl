// The sharpening pass (ADR-0024): FSR 1's robust contrast-adaptive sharpening (RCAS), ported
// to WGSL from `FsrRcasF` in AMD's `ffx_fsr1.h` (FidelityFX Super Resolution 1.0,
// https://github.com/GPUOpen-Effects/FidelityFX-FSR), without its optional denoise. It reads
// the upscale pass's history and writes the frame's rgba8 image. Compiled after march.wgsl,
// whose parameters (`params.output_size`) it uses.
//
// The port carries AMD's notice:
//
//   Copyright (c) 2021 Advanced Micro Devices, Inc. All rights reserved.
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
// RCAS sharpens each pixel against its cross (up, left, right, down) by one negative lobe,
// the largest that keeps every channel inside [0, 1] for any centre within the cross's minimum
// and maximum, capped at RCAS_LIMIT and scaled by `sharpen.con` (`2^-stops`; 0 writes the input
// unchanged, bit for bit); the result is held inside the five taps' minimum and maximum.
//
// Bindings, group 3:
//   0  texture_2d<f32>   sharpen_in      the upscale pass's history (rgba16float, rgb)
//   1  storage texture   sharpen_out     rgba8unorm: the frame's image (output size)
//   2  uniform           sharpen         `con` in x; y, z, w unused

@group(3) @binding(0) var sharpen_in: texture_2d<f32>;
@group(3) @binding(1) var sharpen_out: texture_storage_2d<rgba8unorm, write>;

struct Sharpen {
    con: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(3) @binding(2) var<uniform> sharpen: Sharpen;

// FSR_RCAS_LIMIT: the strongest lobe, 0.25 - 1/16.
const RCAS_LIMIT: f32 = 0.1875;

fn rcas_tap(q: vec2<i32>) -> vec3<f32> {
    let last = vec2<i32>(params.output_size) - vec2<i32>(1);
    return textureLoad(sharpen_in, clamp(q, vec2<i32>(0), last), 0).xyz;
}

@compute @workgroup_size(8, 8, 1)
fn sharpen_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.output_size.x || gid.y >= params.output_size.y) {
        return;
    }
    let o = vec2<i32>(gid.xy);
    let e = rcas_tap(o);
    if (sharpen.con <= 0.0) {
        textureStore(sharpen_out, o, vec4<f32>(e, 1.0));
        return;
    }
    let b = rcas_tap(o + vec2<i32>(0, -1));
    let d = rcas_tap(o + vec2<i32>(-1, 0));
    let f = rcas_tap(o + vec2<i32>(1, 0));
    let h = rcas_tap(o + vec2<i32>(0, 1));
    let mn4 = min(min(b, d), min(f, h));
    let mx4 = max(max(b, d), max(f, h));
    // The lobe that would take the result to 0 or to 1 (peak 1 - 4 lobe).
    let hit_min = mn4 / (4.0 * mx4 + vec3<f32>(1e-6));
    let hit_max = (vec3<f32>(1.0) - mx4) / (4.0 * mn4 - vec3<f32>(4.0) - vec3<f32>(1e-6));
    let lobe3 = max(-hit_min, hit_max);
    let lobe = max(-RCAS_LIMIT, min(max(lobe3.x, max(lobe3.y, lobe3.z)), 0.0)) * sharpen.con;
    let pix = (lobe * (b + d + f + h) + e) / (4.0 * lobe + 1.0);
    // FsrRcasF keeps the result inside [0, 1]; a pixel brighter or darker than its whole cross
    // would still overshoot itself, so the result is also held inside the five taps' range.
    let lo = min(mn4, e);
    let hi = max(mx4, e);
    textureStore(sharpen_out, o, vec4<f32>(clamp(pix, lo, hi), 1.0));
}
