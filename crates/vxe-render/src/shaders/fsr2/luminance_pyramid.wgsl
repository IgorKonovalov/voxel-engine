// FSR 2.2.1's compute-luminance-pyramid pass, ported to WGSL (ADR-0034) from tag v2.2.1 of
// GPUOpen-Effects/FidelityFX-FSR2: `ffx_fsr2_compute_luminance_pyramid_pass.glsl`,
// `ffx_fsr2_compute_luminance_pyramid.h` and the LDS path of `ffx_spd.h`
// (SPD_NO_WAVE_OPERATIONS). Compiled after common.wgsl, which carries AMD's notice.
//
// SPD's single pass becomes two dispatches: `spd_mips` (one workgroup of 256 per 64 x 64
// render pixels) reduces the log luminance through mips 0 to 5 of a pyramid whose mip 0 is
// half the render size, storing the shading-change mip (`cb.luma_mip_level`, 4) and mip 5;
// `spd_tail` (one workgroup) reduces mip 5 to the last mip and writes the exposure there,
// where AMD's last workgroup to finish (`globallycoherent` and an atomic counter) does. Only
// the red channel of SPD's intermediate is kept: the pyramid is single-channel.
//
// Bindings, group 1:
//   0  texture_2d<f32>   colour_in        the composite's linear colour (render size)
//   1  storage texture   mip_change_out   r32float: the shading-change mip
//   2  storage texture   mip5_out         r32float: mip 5 (`spd_mips` writes it)
//   3  texture_2d<f32>   mip5_in          mip 5 (`spd_tail` reads it; a separate binding of
//                                         the same texture in its own layout)
//   4  storage texture   exposure_out     rg32float 1x1: FSR's auto exposure and the smoothed
//                                         log luminance (written; the engine's exposure is
//                                         its own, so no pass reads it)
//   5  texture_2d<f32>   exposure_prev_in rg32float 1x1: last frame's of the same

@group(1) @binding(0) var colour_in: texture_2d<f32>;
@group(1) @binding(1) var mip_change_out: texture_storage_2d<r32float, write>;
@group(1) @binding(2) var mip5_out: texture_storage_2d<r32float, write>;

var<workgroup> spd_intermediate: array<array<f32, 16>, 16>;

// ffxRemapForWaveReduction: a 64-lane index to 8 x 8.
fn remap_for_wave_reduction(a: u32) -> vec2<u32> {
    return vec2<u32>(insertBits(extractBits(a, 2u, 3u), a, 0u, 1u), insertBits(extractBits(a, 3u, 3u), extractBits(a, 1u, 2u), 0u, 2u));
}

// SpdLoadSourceImage: the log luminance of the render pixel at `tex`, 0 off screen.
fn spd_load_source_image(tex: vec2<i32>) -> f32 {
    var uv = (vec2<f32>(tex) + 0.5 + cb.jitter) / render_size_f();
    uv = clamp_uv(uv, render_size(), cb.input_color_size);
    var rgb = bilinear(colour_in, uv).rgb;
    rgb /= cb.pre_exposure;
    let log_luma = log(max(FSR2_EPSILON, rgb_to_luma(rgb)));
    return select(0.0, log_luma, all(tex < render_size()));
}

// SpdStore of the first dispatch: mip `index` at `pix`, kept for the shading-change mip and
// mip 5.
fn spd_store(pix: vec2<i32>, v: f32, index: u32) {
    if (i32(index) == cb.luma_mip_level && all(pix < vec2<i32>(textureDimensions(mip_change_out)))) {
        textureStore(mip_change_out, pix, vec4<f32>(v, 0.0, 0.0, 0.0));
    }
    if (index == 5u && all(pix < vec2<i32>(textureDimensions(mip5_out)))) {
        textureStore(mip5_out, pix, vec4<f32>(v, 0.0, 0.0, 0.0));
    }
}

// SpdReduceLoadSourceImage (no linear sampler): the mean of the 2 x 2 at `base`.
fn spd_reduce_load_source_image(base: vec2<i32>) -> f32 {
    let v0 = spd_load_source_image(base);
    let v1 = spd_load_source_image(base + vec2<i32>(0, 1));
    let v2 = spd_load_source_image(base + vec2<i32>(1, 0));
    let v3 = spd_load_source_image(base + vec2<i32>(1, 1));
    return (v0 + v1 + v2 + v3) * 0.25;
}

fn spd_reduce_intermediate(i0: vec2<u32>, i1: vec2<u32>, i2: vec2<u32>, i3: vec2<u32>) -> f32 {
    let v0 = spd_intermediate[i0.x][i0.y];
    let v1 = spd_intermediate[i1.x][i1.y];
    let v2 = spd_intermediate[i2.x][i2.y];
    let v3 = spd_intermediate[i3.x][i3.y];
    return (v0 + v1 + v2 + v3) * 0.25;
}

@compute @workgroup_size(256, 1, 1)
fn spd_mips(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let mips = cb.spd_mips;
    let sub = remap_for_wave_reduction(li % 64u);
    let x = sub.x + 8u * ((li >> 6u) % 2u);
    let y = sub.y + 8u * (li >> 7u);
    let w = wg.xy;

    // SpdDownsampleMips_0_1_LDS.
    var v: array<f32, 4>;
    v[0] = spd_reduce_load_source_image(vec2<i32>(w * 64u) + vec2<i32>(vec2<u32>(x * 2u, y * 2u)));
    spd_store(vec2<i32>(w * 32u) + vec2<i32>(vec2<u32>(x, y)), v[0], 0u);
    v[1] = spd_reduce_load_source_image(vec2<i32>(w * 64u) + vec2<i32>(vec2<u32>(x * 2u + 32u, y * 2u)));
    spd_store(vec2<i32>(w * 32u) + vec2<i32>(vec2<u32>(x + 16u, y)), v[1], 0u);
    v[2] = spd_reduce_load_source_image(vec2<i32>(w * 64u) + vec2<i32>(vec2<u32>(x * 2u, y * 2u + 32u)));
    spd_store(vec2<i32>(w * 32u) + vec2<i32>(vec2<u32>(x, y + 16u)), v[2], 0u);
    v[3] = spd_reduce_load_source_image(vec2<i32>(w * 64u) + vec2<i32>(vec2<u32>(x * 2u + 32u, y * 2u + 32u)));
    spd_store(vec2<i32>(w * 32u) + vec2<i32>(vec2<u32>(x + 16u, y + 16u)), v[3], 0u);
    if (mips <= 1u) {
        return;
    }
    for (var i = 0u; i < 4u; i++) {
        spd_intermediate[x][y] = v[i];
        workgroupBarrier();
        if (li < 64u) {
            v[i] = spd_reduce_intermediate(vec2<u32>(x * 2u, y * 2u), vec2<u32>(x * 2u + 1u, y * 2u), vec2<u32>(x * 2u, y * 2u + 1u), vec2<u32>(x * 2u + 1u, y * 2u + 1u));
            spd_store(vec2<i32>(w * 16u) + vec2<i32>(vec2<u32>(x + (i % 2u) * 8u, y + (i / 2u) * 8u)), v[i], 1u);
        }
        workgroupBarrier();
    }
    if (li < 64u) {
        spd_intermediate[x][y] = v[0];
        spd_intermediate[x + 8u][y] = v[1];
        spd_intermediate[x][y + 8u] = v[2];
        spd_intermediate[x + 8u][y + 8u] = v[3];
    }

    // SpdDownsampleNextFour from mip 2.
    if (mips <= 2u) {
        return;
    }
    workgroupBarrier();
    // SpdDownsampleMip_2.
    if (li < 64u) {
        let m = spd_reduce_intermediate(vec2<u32>(x * 2u, y * 2u), vec2<u32>(x * 2u + 1u, y * 2u), vec2<u32>(x * 2u, y * 2u + 1u), vec2<u32>(x * 2u + 1u, y * 2u + 1u));
        spd_store(vec2<i32>(w * 8u) + vec2<i32>(vec2<u32>(x, y)), m, 2u);
        spd_intermediate[x * 2u + y % 2u][y * 2u] = m;
    }
    if (mips <= 3u) {
        return;
    }
    workgroupBarrier();
    // SpdDownsampleMip_3.
    if (li < 16u) {
        let m = spd_reduce_intermediate(vec2<u32>(x * 4u, y * 4u), vec2<u32>(x * 4u + 2u, y * 4u), vec2<u32>(x * 4u + 1u, y * 4u + 2u), vec2<u32>(x * 4u + 3u, y * 4u + 2u));
        spd_store(vec2<i32>(w * 4u) + vec2<i32>(vec2<u32>(x, y)), m, 3u);
        spd_intermediate[x * 4u + y][y * 4u] = m;
    }
    if (mips <= 4u) {
        return;
    }
    workgroupBarrier();
    // SpdDownsampleMip_4.
    if (li < 4u) {
        let m = spd_reduce_intermediate(vec2<u32>(x * 8u + y * 2u, y * 8u), vec2<u32>(x * 8u + 4u + y * 2u, y * 8u), vec2<u32>(x * 8u + 1u + y * 2u, y * 8u + 4u), vec2<u32>(x * 8u + 5u + y * 2u, y * 8u + 4u));
        spd_store(vec2<i32>(w * 2u) + vec2<i32>(vec2<u32>(x, y)), m, 4u);
        spd_intermediate[x + y * 2u][0] = m;
    }
    if (mips <= 5u) {
        return;
    }
    workgroupBarrier();
    // SpdDownsampleMip_5.
    if (li < 1u) {
        let m = spd_reduce_intermediate(vec2<u32>(0u, 0u), vec2<u32>(1u, 0u), vec2<u32>(2u, 0u), vec2<u32>(3u, 0u));
        spd_store(vec2<i32>(w), m, 5u);
    }
}

@group(1) @binding(3) var mip5_in: texture_2d<f32>;
@group(1) @binding(4) var exposure_out: texture_storage_2d<rg32float, write>;
@group(1) @binding(5) var exposure_prev_in: texture_2d<f32>;

// SpdLoad: mip 5 at `p`, 0 outside it (an out-of-bounds image load).
fn spd_load_mip5(p: vec2<i32>) -> f32 {
    if (any(p < vec2<i32>(0)) || any(p >= vec2<i32>(textureDimensions(mip5_in)))) {
        return 0.0;
    }
    return textureLoad(mip5_in, p, 0).x;
}

fn spd_reduce_load4(base: vec2<i32>) -> f32 {
    let v0 = spd_load_mip5(base);
    let v1 = spd_load_mip5(base + vec2<i32>(0, 1));
    let v2 = spd_load_mip5(base + vec2<i32>(1, 0));
    let v3 = spd_load_mip5(base + vec2<i32>(1, 1));
    return (v0 + v1 + v2 + v3) * 0.25;
}

// SpdStore of the tail: the last mip's texel (0, 0) feeds the exposure.
fn spd_store_tail(pix: vec2<i32>, v: f32, index: u32) {
    if (index != cb.spd_mips - 1u || any(pix != vec2<i32>(0))) {
        return;
    }
    var prev = textureLoad(exposure_prev_in, vec2<i32>(0), 0).y;
    if (cb.frame_index == 0) {
        // The reset clears the exposure to (-1, 1e8): no smoothing from it.
        prev = RESET_AUTO_EXPOSURE_AVERAGE_SMOOTHING;
    }
    var result = v;
    if (prev < RESET_AUTO_EXPOSURE_AVERAGE_SMOOTHING) {
        let rate = 1.0;
        result = prev + (result - prev) * (1.0 - exp(-cb.delta_time * rate));
    }
    textureStore(exposure_out, vec2<i32>(0), vec4<f32>(compute_auto_exposure_from_lavg(result), result, 0.0, 0.0));
}

// The part of SpdDownsample after the last workgroup's exit: SpdDownsampleMips_6_7, then
// SpdDownsampleNextFour from mip 8 at workgroup (0, 0).
@compute @workgroup_size(256, 1, 1)
fn spd_tail(@builtin(local_invocation_index) li: u32) {
    let mips = cb.spd_mips;
    if (mips <= 6u) {
        return;
    }
    let sub = remap_for_wave_reduction(li % 64u);
    let x = sub.x + 8u * ((li >> 6u) % 2u);
    let y = sub.y + 8u * (li >> 7u);
    let tex = vec2<i32>(vec2<u32>(x * 4u, y * 4u));
    let pix = vec2<i32>(vec2<u32>(x * 2u, y * 2u));
    let v0 = spd_reduce_load4(tex);
    spd_store_tail(pix, v0, 6u);
    let v1 = spd_reduce_load4(tex + vec2<i32>(2, 0));
    spd_store_tail(pix + vec2<i32>(1, 0), v1, 6u);
    let v2 = spd_reduce_load4(tex + vec2<i32>(0, 2));
    spd_store_tail(pix + vec2<i32>(0, 1), v2, 6u);
    let v3 = spd_reduce_load4(tex + vec2<i32>(2, 2));
    spd_store_tail(pix + vec2<i32>(1, 1), v3, 6u);
    if (mips <= 7u) {
        return;
    }
    let v = (v0 + v1 + v2 + v3) * 0.25;
    spd_store_tail(vec2<i32>(vec2<u32>(x, y)), v, 7u);
    spd_intermediate[x][y] = v;
    if (mips <= 8u) {
        return;
    }
    workgroupBarrier();
    if (li < 64u) {
        let m = spd_reduce_intermediate(vec2<u32>(x * 2u, y * 2u), vec2<u32>(x * 2u + 1u, y * 2u), vec2<u32>(x * 2u, y * 2u + 1u), vec2<u32>(x * 2u + 1u, y * 2u + 1u));
        spd_store_tail(vec2<i32>(vec2<u32>(x, y)), m, 8u);
        spd_intermediate[x * 2u + y % 2u][y * 2u] = m;
    }
    if (mips <= 9u) {
        return;
    }
    workgroupBarrier();
    if (li < 16u) {
        let m = spd_reduce_intermediate(vec2<u32>(x * 4u, y * 4u), vec2<u32>(x * 4u + 2u, y * 4u), vec2<u32>(x * 4u + 1u, y * 4u + 2u), vec2<u32>(x * 4u + 3u, y * 4u + 2u));
        spd_store_tail(vec2<i32>(vec2<u32>(x, y)), m, 9u);
        spd_intermediate[x * 4u + y][y * 4u] = m;
    }
    if (mips <= 10u) {
        return;
    }
    workgroupBarrier();
    if (li < 4u) {
        let m = spd_reduce_intermediate(vec2<u32>(x * 8u + y * 2u, y * 8u), vec2<u32>(x * 8u + 4u + y * 2u, y * 8u), vec2<u32>(x * 8u + 1u + y * 2u, y * 8u + 4u), vec2<u32>(x * 8u + 5u + y * 2u, y * 8u + 4u));
        spd_store_tail(vec2<i32>(vec2<u32>(x, y)), m, 10u);
        spd_intermediate[x + y * 2u][0] = m;
    }
    if (mips <= 11u) {
        return;
    }
    workgroupBarrier();
    if (li < 1u) {
        let m = spd_reduce_intermediate(vec2<u32>(0u, 0u), vec2<u32>(1u, 0u), vec2<u32>(2u, 0u), vec2<u32>(3u, 0u));
        spd_store_tail(vec2<i32>(0), m, 11u);
    }
}
