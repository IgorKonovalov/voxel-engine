// FSR 2.2.1's accumulate pass, ported to WGSL (ADR-0034) from tag v2.2.1 of
// GPUOpen-Effects/FidelityFX-FSR2: `ffx_fsr2_accumulate_pass.glsl`, `ffx_fsr2_accumulate.h`,
// `ffx_fsr2_upsample.h`, `ffx_fsr2_reproject.h` and `ffx_fsr2_postprocess_lock_status.h`. One
// invocation per output pixel. Compiled after common.wgsl, which carries AMD's notice.
//
// Bindings, group 1:
//   0  texture_2d<f32>   exposure_in              rg32float 1x1: the exposure in x
//   1  texture_2d<f32>   dilated_reactive_in      rgba8unorm: the dilated reactive mask in x,
//                                                 the accumulation mask in y (render size)
//   2  texture_2d<f32>   dilated_motion_in        rg32float: dilated motion, UV (render size)
//   3  texture_2d<f32>   upscaled_prev_in         rgba16float: last frame's internal upscaled
//                                                 colour (rgb) and temporal reactivity (a)
//   4  texture_2d<f32>   lock_status_prev_in      rg32float: last frame's lock lifetime (x)
//                                                 and temporal luma (y) (display size)
//   5  texture_2d<f32>   prepared_in              rgba16float: the prepared input colour in
//                                                 YCoCg (rgb) and the depth clip (a)
//   6  texture_2d<f32>   luma_mip_in              r32float: the scene luminance pyramid's
//                                                 shading-change mip (log luma)
//   7  texture_2d<f32>   luma_history_prev_in     rgba8unorm: last frame's luma history
//   8  storage buffer    new_locks                f32 per display pixel, row-major, read and
//                                                 cleared here (AMD's read-write r8 image)
//   9  storage texture   lock_status_out          rg32float
//   10 storage texture   upscaled_out             rgba16float
//   11 storage texture   luma_history_out         rgba8unorm
//
// `HDR` (FFX_FSR2_OPTION_HDR_COLOR_INPUT) is defined before common.wgsl. The upscaled image is
// `upscaled_out`'s rgb, the accumulated colour unprepared: AMD's pass also writes it to its
// output when sharpening is off (FFX_FSR2_OPTION_APPLY_SHARPENING 0), and the tone pass reads
// it here instead.

@group(1) @binding(0) var exposure_in: texture_2d<f32>;
@group(1) @binding(1) var dilated_reactive_in: texture_2d<f32>;
@group(1) @binding(2) var dilated_motion_in: texture_2d<f32>;
@group(1) @binding(3) var upscaled_prev_in: texture_2d<f32>;
@group(1) @binding(4) var lock_status_prev_in: texture_2d<f32>;
@group(1) @binding(5) var prepared_in: texture_2d<f32>;
@group(1) @binding(6) var luma_mip_in: texture_2d<f32>;
@group(1) @binding(7) var luma_history_prev_in: texture_2d<f32>;
@group(1) @binding(8) var<storage, read_write> new_locks: array<f32>;
@group(1) @binding(9) var lock_status_out: texture_storage_2d<rg32float, write>;
@group(1) @binding(10) var upscaled_out: texture_storage_2d<rgba16float, write>;
@group(1) @binding(11) var luma_history_out: texture_storage_2d<rgba8unorm, write>;

// ffx_fsr2_common.h's AccumulationPassCommonParams.
struct AccumulationParams {
    hr_pos: vec2<i32>,
    hr_uv: vec2<f32>,
    lr_uv_hw_sampler: vec2<f32>,
    motion_vector: vec2<f32>,
    reprojected_hr_uv: vec2<f32>,
    hr_velocity: f32,
    depth_clip_factor: f32,
    dilated_reactive_factor: f32,
    accumulation_mask: f32,
    is_reset_frame: bool,
    is_existing_sample: bool,
    is_new_sample: bool,
};

struct LockState {
    new_lock: bool,
    was_locked_prev_frame: bool,
};

fn exposure() -> f32 {
    return exposure_of(exposure_in);
}

fn new_lock_index(p: vec2<i32>) -> u32 {
    return u32(p.y) * u32(cb.display_size.x) + u32(p.x);
}

// LoadPreparedInputColor.
fn load_prepared_input_color(p: vec2<i32>) -> vec3<f32> {
    return load_clamped(prepared_in, p).xyz;
}

// SampleDepthClip.
fn sample_depth_clip(uv: vec2<f32>) -> f32 {
    return bilinear(prepared_in, uv).w;
}

// ---- ffx_fsr2_upsample.h ----

fn get_upsample_lanczos_weight(src_sample_offset: vec2<f32>, kernel_weight: f32) -> f32 {
    let biased = src_sample_offset * kernel_weight;
    return lanczos2_approx_sq(dot(biased, biased));
}

fn compute_max_kernel_weight() -> f32 {
    let kernel_size_bias = 1.0;
    let w = 1.0 + (1.0 / cb.downscale - vec2<f32>(1.0)).x * kernel_size_bias;
    return min(1.99, w);
}

// ComputeUpsampledColorAndWeight: the 3 x 3 internal samples nearest the output pixel,
// weighted by the biased Lanczos approximation, with the rectification box over them.
fn compute_upsampled_color_and_weight(params: AccumulationParams, clipping_box: ptr<function, RectificationBox>, reactive_factor: f32) -> vec4<f32> {
    let dst_output_pos = vec2<f32>(params.hr_pos) + vec2<f32>(0.5);
    let src_output_pos = dst_output_pos * cb.downscale;
    let src_input_pos = vec2<i32>(floor(src_output_pos));
    let src_unjittered_pos = (vec2<f32>(src_input_pos) + vec2<f32>(0.5)) - cb.jitter;
    var offset_tl = vec2<i32>(-1);
    offset_tl.x = select(-1, -2, src_unjittered_pos.x > src_output_pos.x);
    offset_tl.y = select(-1, -2, src_unjittered_pos.y > src_output_pos.y);
    let flip_row = src_unjittered_pos.y > src_output_pos.y;
    let flip_col = src_unjittered_pos.x > src_output_pos.x;
    let f_offset_tl = vec2<f32>(offset_tl);

    var color_and_weight = vec4<f32>(0.0);
    let base_sample_offset = src_unjittered_pos - src_output_pos;
    let kernel_reactive_factor = max(reactive_factor, f32(params.is_new_sample));
    let kernel_bias_max = compute_max_kernel_weight() * (1.0 - kernel_reactive_factor);
    let kernel_bias_min = max(1.0, (1.0 + kernel_bias_max) * 0.3);
    let kernel_bias_factor = max(0.0, max(0.25 * params.depth_clip_factor, kernel_reactive_factor));
    let kernel_bias = mix(kernel_bias_max, kernel_bias_min, kernel_bias_factor);
    let rectification_curve_bias = mix(-2.0, -3.0, saturate(params.hr_velocity / 50.0));

    for (var row = 0; row < 3; row++) {
        for (var col = 0; col < 3; col++) {
            let sample_col_row = vec2<i32>(select(col, 3 - col, flip_col), select(row, 3 - row, flip_row));
            let src_sample_pos = src_input_pos + offset_tl + sample_col_row;
            let sample_coord = clamp_load(src_sample_pos, vec2<i32>(0), render_size());
            let s = load_prepared_input_color(sample_coord);

            let offset = f_offset_tl + vec2<f32>(sample_col_row);
            let src_sample_offset = base_sample_offset + offset;
            let on_screen = f32(is_on_screen(src_sample_pos, render_size()));
            let sample_weight = on_screen * get_upsample_lanczos_weight(src_sample_offset, kernel_bias);
            color_and_weight += vec4<f32>(s * sample_weight, sample_weight);

            let offset_sq = dot(src_sample_offset, src_sample_offset);
            let box_sample_weight = exp(rectification_curve_bias * offset_sq);
            rectification_box_add_sample(row == 0 && col == 0, clipping_box, s, box_sample_weight);
        }
    }
    rectification_box_compute_variance(clipping_box);

    color_and_weight.w *= f32(color_and_weight.w > FSR2_EPSILON);
    if (color_and_weight.w > FSR2_EPSILON) {
        // Normalised for deringing (colours are compared).
        let c = color_and_weight.xyz / color_and_weight.w;
        color_and_weight = vec4<f32>(clamp(c, (*clipping_box).aabb_min, (*clipping_box).aabb_max), color_and_weight.w * UPSAMPLE_LANCZOS_WEIGHT_SCALE);
    }
    return color_and_weight;
}

// ---- ffx_fsr2_reproject.h ----

// HistorySample: the reference Lanczos-2 over the 4 x 4 history texels around `uv`, clamped
// to the four middle texels (deringing).
fn history_sample(uv: vec2<f32>) -> vec4<f32> {
    let size = display_size();
    var s = uv * vec2<f32>(size) - vec2<f32>(0.5);
    s.x = max(0.0, min(f32(size.x), s.x));
    s.y = max(0.0, min(f32(size.y), s.y));
    let base = vec2<i32>(floor(s));
    let f = fract(s);
    var rows: array<vec4<f32>, 4>;
    var mn = vec4<f32>(0.0);
    var mx = vec4<f32>(0.0);
    for (var y = 0; y < 4; y++) {
        var c: array<vec4<f32>, 4>;
        for (var x = 0; x < 4; x++) {
            let p = clamp_load(base, vec2<i32>(x - 1, y - 1), size);
            c[x] = load_clamped(upscaled_prev_in, p);
            if ((x == 1 || x == 2) && (y == 1 || y == 2)) {
                if (x == 1 && y == 1) {
                    mn = c[x];
                    mx = c[x];
                } else {
                    mn = min(mn, c[x]);
                    mx = max(mx, c[x]);
                }
            }
        }
        rows[y] = lanczos2_4(c[0], c[1], c[2], c[3], f.x);
    }
    let xy = lanczos2_4(rows[0], rows[1], rows[2], rows[3], f.y);
    return clamp(xy, mn, mx);
}

fn is_uv_inside(uv: vec2<f32>) -> bool {
    return uv.x >= 0.0 && uv.x <= 1.0 && uv.y >= 0.0 && uv.y <= 1.0;
}

// ---- ffx_fsr2_postprocess_lock_status.h ----

fn get_shading_change_luma(uv_in: vec2<f32>) -> f32 {
    let div = f32(2 << u32(cb.luma_mip_level));
    let mip_render_size = vec2<i32>(render_size_f() / div);
    let uv = clamp_uv(uv_in, mip_render_size, cb.luma_mip_size);
    let l = exposure() * exp(bilinear(luma_mip_in, uv).x);
    return pow(l, 1.0 / 6.0);
}

// UpdateLockStatus; returns (reactive factor, lock contribution, luminance difference) and
// updates `lock_status`.
fn update_lock_status(params: AccumulationParams, reactive_in: f32, state: LockState, lock_status: ptr<function, vec2<f32>>) -> vec3<f32> {
    var reactive = reactive_in;
    let shading_change_luma = get_shading_change_luma(params.hr_uv);
    if ((*lock_status).y == 0.0) {
        (*lock_status).y = shading_change_luma;
    }
    let previous_shading_change_luma = (*lock_status).y;
    let luminance_diff = 1.0 - min_divided_by_max(previous_shading_change_luma, shading_change_luma);
    if (state.new_lock) {
        (*lock_status).y = shading_change_luma;
        (*lock_status).x = select(1.0, 2.0, (*lock_status).x != 0.0);
    } else if ((*lock_status).x <= 1.0) {
        (*lock_status).y = mix((*lock_status).y, shading_change_luma, 0.5);
    } else {
        if (luminance_diff > 0.1) {
            (*lock_status).x = 0.0;
        }
    }
    reactive = max(reactive, saturate((luminance_diff - 0.1) * 10.0));
    (*lock_status).x *= 1.0 - reactive;
    (*lock_status).x *= saturate(1.0 - params.accumulation_mask);
    (*lock_status).x *= f32(params.depth_clip_factor < 0.1);
    let lifetime_contribution = saturate((*lock_status).x - 1.0);
    let shading_change_contribution = saturate(min_divided_by_max((*lock_status).y, shading_change_luma));
    let contribution = saturate(saturate(lifetime_contribution * 4.0) * shading_change_contribution);
    return vec3<f32>(reactive, contribution, luminance_diff);
}

// ---- ffx_fsr2_accumulate.h ----

fn get_px_hr_velocity(motion_vector: vec2<f32>) -> f32 {
    return length(motion_vector * display_size_f());
}

// Accumulate (the blend of history and the upsampled colour), both in YCoCg; returns RGB.
fn accumulate_colors(history_in: vec3<f32>, accumulation_in: vec3<f32>, upsampled_in: vec4<f32>) -> vec3<f32> {
    var history = history_in;
    var upsampled = upsampled_in;
    let accumulation = max(vec3<f32>(FSR2_EPSILON), accumulation_in + upsampled.www);
    if (HDR) {
        // YCoCg -> RGB -> tone map -> YCoCg: the RGB tone map avoids desaturation.
        upsampled = vec4<f32>(rgb_to_ycocg(tonemap(ycocg_to_rgb(upsampled.xyz))), upsampled.w);
        history = rgb_to_ycocg(tonemap(ycocg_to_rgb(history)));
    }
    let alpha = upsampled.www / accumulation;
    history = mix(history, upsampled.xyz, alpha);
    history = ycocg_to_rgb(history);
    if (HDR) {
        history = inverse_tonemap(history);
    }
    return history;
}

// RectifyHistory: clamps history outside the scaled variance box toward it, by how little the
// lock and the luma instability protect it; returns (history, accumulation).
fn rectify_history(params: AccumulationParams, clipping_box: RectificationBox, history: ptr<function, vec3<f32>>, accumulation: ptr<function, vec3<f32>>, lock_contribution: f32, luma_instability: f32) {
    let scale_factor_influence = min(20.0, pow(1.0 / length(cb.downscale.x * cb.downscale.y), 3.0));
    let velocity_factor = saturate(params.hr_velocity / 20.0);
    let box_scale_t = max(params.depth_clip_factor, max(params.accumulation_mask, velocity_factor));
    let box_scale = mix(scale_factor_influence, 1.0, box_scale_t);
    let scaled_box_vec = clipping_box.box_vec * box_scale;
    var box_min = clipping_box.box_center - scaled_box_vec;
    var box_max = clipping_box.box_center + scaled_box_vec;
    box_min = max(clipping_box.aabb_min, box_min);
    box_max = min(clipping_box.aabb_max, box_max);
    let h = *history;
    if (any(box_min > h) || any(h > box_max)) {
        let clamped = clamp(h, box_min, box_max);
        var contribution = vec3<f32>(max(luma_instability, lock_contribution));
        let reactive_contribution = 1.0 - pow(params.dilated_reactive_factor, 1.0 / 2.0);
        contribution *= reactive_contribution;
        *history = mix(clamped, h, saturate(contribution));
        let accumulation_min = min(*accumulation, vec3<f32>(0.1));
        *accumulation = mix(accumulation_min, *accumulation, saturate(contribution));
    }
}

fn finalize_lock_status(params: AccumulationParams, lock_status_in: vec2<f32>, upsampled_weight: f32) {
    var lock_status = lock_status_in;
    // Similar motion expected next frame: a lock about to leave the screen dies rather than
    // clamping to its border.
    let estimated_uv_next_frame = params.hr_uv - params.motion_vector;
    if (!is_uv_inside(estimated_uv_next_frame)) {
        lock_status.x = 0.0;
    } else {
        let lifetime_decrease_lanczos_max = cb.jitter_sequence_length * AVERAGE_LANCZOS_WEIGHT_PER_FRAME;
        let lifetime_decrease = upsampled_weight / lifetime_decrease_lanczos_max;
        lock_status.x = max(0.0, lock_status.x - lifetime_decrease);
    }
    textureStore(lock_status_out, params.hr_pos, vec4<f32>(lock_status, 0.0, 0.0));
}

fn compute_base_accumulation_weight(params: AccumulationParams, this_frame_reactive: f32, in_motion_last_frame: bool, upsampled_weight: f32) -> vec3<f32> {
    var base = MAX_ACCUMULATION_LANCZOS_WEIGHT * f32(params.is_existing_sample) * (1.0 - this_frame_reactive) * (1.0 - params.depth_clip_factor);
    base = min(base, mix(base, upsampled_weight * 10.0, max(f32(in_motion_last_frame), saturate(params.hr_velocity * 10.0))));
    base = min(base, mix(base, upsampled_weight, saturate(params.hr_velocity / 20.0)));
    return vec3<f32>(base);
}

fn compute_luma_instability_factor(params: AccumulationParams, clipping_box: RectificationBox, this_frame_reactive: f32, luminance_diff: f32) -> f32 {
    let unorm_threshold = 1.0 / 255.0;
    var current_luma = clipping_box.box_center.x;
    if (HDR) {
        current_luma = current_luma / (1.0 + max(0.0, current_luma));
    }
    current_luma = round(current_luma * 255.0) / 255.0;
    let sample_luma_history = (max(max(params.depth_clip_factor, params.accumulation_mask), luminance_diff) < 0.1) && !params.is_new_sample;
    var hist = vec4<f32>(0.0);
    if (sample_luma_history) {
        hist = bilinear(luma_history_prev_in, params.reprojected_hr_uv);
    }
    var instability = 0.0;
    let diffs0 = current_luma - hist.x;
    var m = abs(diffs0);
    if (m >= unorm_threshold) {
        for (var i = 1; i <= 3; i++) {
            let diffs1 = current_luma - hist[i];
            if (sign(diffs0) == sign(diffs1)) {
                // Scaled difference protects historically similar values.
                let min_bias = 1.0;
                m = min(m, abs(diffs1) * min_bias);
            }
        }
        let box_size = clipping_box.box_vec.x;
        let box_size_factor = pow(saturate(box_size / 0.1), 6.0);
        instability = f32(m != abs(diffs0)) * box_size_factor;
        instability = f32(instability > unorm_threshold);
        instability *= 1.0 - max(params.accumulation_mask, pow(this_frame_reactive, 1.0 / 6.0));
    }
    // Shift the history: N-1 into x.
    let shifted = vec4<f32>(current_luma, hist.x, hist.y, hist.z);
    textureStore(luma_history_out, params.hr_pos, shifted);
    return instability * f32(shifted.w != 0.0);
}

fn compute_temporal_reactive_factor(params: AccumulationParams, temporal_reactive: f32) -> f32 {
    var f = min(0.99, temporal_reactive);
    f = max(f, mix(f, 0.4, saturate(params.hr_velocity)));
    f = max(f * f, max(params.depth_clip_factor * 0.1, params.dilated_reactive_factor));
    // Forced for new samples.
    f = select(f, 1.0, params.is_new_sample);
    if (saturate(params.hr_velocity * 10.0) >= 1.0) {
        f = max(FSR2_EPSILON, f) * -1.0;
    }
    return f;
}

fn init_params(hr_pos: vec2<i32>) -> AccumulationParams {
    var p: AccumulationParams;
    p.hr_pos = hr_pos;
    let hr_uv = (vec2<f32>(hr_pos) + 0.5) / display_size_f();
    p.hr_uv = hr_uv;
    let lr_uv_jittered = hr_uv + cb.jitter / render_size_f();
    p.lr_uv_hw_sampler = clamp_uv(lr_uv_jittered, render_size(), cb.max_render_size);
    // GetMotionVector with motion vectors at the render size.
    p.motion_vector = load_clamped(dilated_motion_in, vec2<i32>(hr_uv * render_size_f())).xy;
    p.hr_velocity = get_px_hr_velocity(p.motion_vector);
    p.reprojected_hr_uv = p.hr_uv + p.motion_vector;
    p.is_existing_sample = is_uv_inside(p.reprojected_hr_uv);
    p.depth_clip_factor = saturate(sample_depth_clip(p.lr_uv_hw_sampler));
    let masks = bilinear(dilated_reactive_in, p.lr_uv_hw_sampler).xy;
    p.dilated_reactive_factor = masks.x;
    p.accumulation_mask = masks.y;
    p.is_reset_frame = cb.frame_index == 0;
    p.is_new_sample = !p.is_existing_sample || p.is_reset_frame;
    return p;
}

@compute @workgroup_size(8, 8, 1)
fn accumulate(@builtin(global_invocation_id) gid: vec3<u32>) {
    let hr_pos = vec2<i32>(gid.xy);
    if (any(hr_pos >= display_size())) {
        return;
    }
    let params = init_params(hr_pos);
    var history = vec3<f32>(0.0);
    var lock_status = vec2<f32>(0.0);
    var temporal_reactive = 0.0;
    var in_motion_last_frame = false;
    var lock_state = LockState(false, false);
    if (params.is_existing_sample && !params.is_reset_frame) {
        // ReprojectHistoryColor.
        let h = history_sample(params.reprojected_hr_uv);
        history = rgb_to_ycocg(prepare_rgb(h.rgb, exposure(), cb.previous_pre_exposure));
        temporal_reactive = saturate(abs(h.w));
        in_motion_last_frame = h.w < 0.0;
        // ReprojectHistoryLockStatus.
        let intensity = new_locks[new_lock_index(hr_pos)];
        lock_state.new_lock = intensity > (127.0 / 255.0);
        lock_status = bilinear(lock_status_prev_in, params.reprojected_hr_uv).xy;
        lock_state.was_locked_prev_frame = lock_status.x != 0.0;
    }
    let lock = update_lock_status(params, max(params.dilated_reactive_factor, temporal_reactive), lock_state, &lock_status);
    let this_frame_reactive = lock.x;
    let lock_contribution = lock.y;
    let luminance_diff = lock.z;

    var clipping_box = rectification_box_reset();
    let upsampled = compute_upsampled_color_and_weight(params, &clipping_box, this_frame_reactive);
    let luma_instability = compute_luma_instability_factor(params, clipping_box, this_frame_reactive, luminance_diff);
    var accumulation = compute_base_accumulation_weight(params, this_frame_reactive, in_motion_last_frame, upsampled.w);
    if (params.is_new_sample) {
        history = ycocg_to_rgb(upsampled.xyz);
    } else {
        rectify_history(params, clipping_box, &history, &accumulation, lock_contribution, luma_instability);
        history = accumulate_colors(history, accumulation, upsampled);
    }
    history = unprepare_rgb(history, exposure());
    finalize_lock_status(params, lock_status, upsampled.w);
    let reactive_out = compute_temporal_reactive_factor(params, this_frame_reactive);
    textureStore(upscaled_out, hr_pos, vec4<f32>(history, reactive_out));
    new_locks[new_lock_index(hr_pos)] = 0.0;
}
