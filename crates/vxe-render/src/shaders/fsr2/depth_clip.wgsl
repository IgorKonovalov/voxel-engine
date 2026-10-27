// FSR 2.2.1's depth-clip pass, ported to WGSL (ADR-0034) from tag v2.2.1 of
// GPUOpen-Effects/FidelityFX-FSR2: `ffx_fsr2_depth_clip_pass.glsl` and `ffx_fsr2_depth_clip.h`.
// One invocation per render pixel. Compiled after common.wgsl, which carries AMD's notice.
//
// One engine rule is added to AMD's pass, in `depth_clip` below: a pixel whose hit lies in
// one of the frame's edit boxes dilated by one voxel (our input pass's flag) takes a depth
// clip of 1, full disocclusion, so no history survives where the world changed.
//
// Bindings, group 1:
//   0  texture_2d<f32>   depth_in               our input pass's device depth
//   1  texture_2d<f32>   motion_in              our input pass's motion vectors (UV)
//   2  texture_2d<f32>   colour_in              the composite's linear colour
//   3  texture_2d<f32>   reactive_in            our input pass's masks: reactive (x),
//                                               transparency and composition (y), edit (z)
//   4  texture_2d<f32>   exposure_in            rg32float 1x1
//   5  texture_2d<f32>   dilated_depth_in       the reconstruct pass's dilated depth
//   6  texture_2d<f32>   dilated_motion_in      its dilated motion, this frame's
//   7  texture_2d<f32>   prev_dilated_motion_in last frame's dilated motion
//   8  storage buffer    reconstructed          u32 per render pixel: the reconstructed
//                                               previous depth (f32 bits)
//   9  storage texture   prepared_out           rgba16float: the prepared input colour,
//                                               YCoCg, and the depth clip in a
//   10 storage texture   dilated_reactive_out   rgba8unorm: the dilated reactive mask (x) and
//                                               the accumulation mask (y)

@group(1) @binding(0) var depth_in: texture_2d<f32>;
@group(1) @binding(1) var motion_in: texture_2d<f32>;
@group(1) @binding(2) var colour_in: texture_2d<f32>;
@group(1) @binding(3) var reactive_in: texture_2d<f32>;
@group(1) @binding(4) var exposure_in: texture_2d<f32>;
@group(1) @binding(5) var dilated_depth_in: texture_2d<f32>;
@group(1) @binding(6) var dilated_motion_in: texture_2d<f32>;
@group(1) @binding(7) var prev_dilated_motion_in: texture_2d<f32>;
@group(1) @binding(8) var<storage, read> reconstructed: array<u32>;
@group(1) @binding(9) var prepared_out: texture_storage_2d<rgba16float, write>;
@group(1) @binding(10) var dilated_reactive_out: texture_storage_2d<rgba8unorm, write>;

const DEPTH_CLIP_BASE_SCALE: f32 = 4.0;

fn load_reconstructed_prev_depth(p: vec2<i32>) -> f32 {
    let q = clamp(p, vec2<i32>(0), render_size() - vec2<i32>(1));
    return bitcast<f32>(reconstructed[u32(q.y) * u32(cb.render_size.x) + u32(q.x)]);
}

fn load_input_motion_vector(p: vec2<i32>) -> vec2<f32> {
    return load_clamped(motion_in, p).xy * cb.motion_vector_scale;
}

fn compute_depth_clip(uv_sample: vec2<f32>, current_depth_sample: f32) -> f32 {
    let current_depth_view_space = get_view_space_depth(current_depth_sample);
    let b = get_bilinear_sampling_data(uv_sample, render_size());
    var depth = 0.0;
    var weight_sum = 0.0;
    for (var i = 0; i < 4; i++) {
        let sample_pos = b.base + bilinear_offset(i);
        if (is_on_screen(sample_pos, render_size())) {
            let weight = bilinear_weight(b, i);
            if (weight > RECONSTRUCTED_DEPTH_BILINEAR_WEIGHT_THRESHOLD) {
                let prev_depth_sample = load_reconstructed_prev_depth(sample_pos);
                let prev_nearest_depth_view_space = get_view_space_depth(prev_depth_sample);
                let depth_diff = current_depth_view_space - prev_nearest_depth_view_space;
                if (depth_diff > 0.0) {
                    // Inverted depth: the plane is the farther of the two.
                    let plane_depth = min(prev_depth_sample, current_depth_sample);
                    let centre = get_view_space_position(vec2<i32>(render_size_f() * 0.5), render_size(), plane_depth);
                    let corner = get_view_space_position(vec2<i32>(0), render_size(), plane_depth);
                    let half_viewport_width = length(render_size_f());
                    let depth_threshold = max(current_depth_view_space, prev_nearest_depth_view_space);
                    let k_sep = 1.37e-05;
                    let k_fov = length(corner) / length(centre);
                    let required_depth_separation = k_sep * k_fov * half_viewport_width * depth_threshold;
                    let resolution_factor = saturate(length(render_size_f()) / length(vec2<f32>(1920.0, 1080.0)));
                    let power = mix(1.0, 3.0, resolution_factor);
                    depth += pow(saturate(required_depth_separation / depth_diff), power) * weight;
                    weight_sum += weight;
                }
            }
        }
    }
    return select(0.0, saturate(1.0 - depth / weight_sum), weight_sum > 0.0);
}

fn compute_motion_divergence(px: vec2<i32>, mv_size: vec2<i32>) -> f32 {
    var min_convergence = 1.0;
    let nucleus = load_input_motion_vector(px);
    let nucleus_velocity_lr = length(nucleus * render_size_f());
    var max_velocity_uv = length(nucleus);
    let motion_vector_velocity_epsilon = 1e-02;
    if (nucleus_velocity_lr > motion_vector_velocity_epsilon) {
        for (var y = -1; y <= 1; y++) {
            for (var x = -1; x <= 1; x++) {
                let sp = clamp_load(px, vec2<i32>(x, y), mv_size);
                let mv = load_input_motion_vector(sp);
                var velocity_uv = length(mv);
                max_velocity_uv = max(velocity_uv, max_velocity_uv);
                velocity_uv = max(velocity_uv, max_velocity_uv);
                min_convergence = min(min_convergence, dot(mv / velocity_uv, nucleus / velocity_uv));
            }
        }
    }
    return saturate(1.0 - min_convergence) * saturate(max_velocity_uv / 0.01);
}

fn compute_depth_divergence(px: vec2<i32>) -> f32 {
    let max_dist_in_meters = get_max_distance_in_meters();
    var depth_max = 0.0;
    var depth_min = max_dist_in_meters;
    var max_dist_found = false;
    for (var y = -1; y < 2; y++) {
        for (var x = -1; x < 2; x++) {
            let sample_pos = px + vec2<i32>(x, y);
            let on_screen = select(0.0, 1.0, is_on_screen(sample_pos, render_size()));
            let d = get_view_space_depth_in_meters(load_clamped(dilated_depth_in, sample_pos).x) * on_screen;
            max_dist_found = max_dist_found || max_dist_in_meters == d;
            depth_min = min(depth_min, d);
            depth_max = max(depth_max, d);
        }
    }
    return (1.0 - depth_min / depth_max) * select(1.0, 0.0, max_dist_found);
}

fn compute_temporal_motion_divergence(px: vec2<i32>) -> f32 {
    let uv = (vec2<f32>(px) + 0.5) / render_size_f();
    let mv = load_clamped(dilated_motion_in, px).xy;
    var reprojected_uv = uv + mv;
    reprojected_uv = clamp_uv(reprojected_uv, render_size(), cb.max_render_size);
    let prev_mv = bilinear(prev_dilated_motion_in, reprojected_uv).xy;
    let px_distance = length(mv * display_size_f());
    if (px_distance > 1.0) {
        return mix(0.0, 1.0 - saturate(length(prev_mv) / length(mv)), saturate(pow(px_distance / 20.0, 3.0)));
    }
    return 0.0;
}

// PreProcessReactiveMasks: the reactive and composition masks dilated over the 3 x 3,
// weighted by colour similarity (compensating the accumulate pass's bilinear sampling).
fn pre_process_reactive_masks(px: vec2<i32>, motion_divergence: f32) -> vec2<f32> {
    let reference = load_clamped(colour_in, px).xyz;
    var reactive = vec2<f32>(0.0, motion_divergence);
    var masks_sum = 0.0;
    var colours: array<vec3<f32>, 9>;
    var reactives: array<f32, 9>;
    var compositions: array<f32, 9>;
    for (var y = -1; y < 2; y++) {
        for (var x = -1; x < 2; x++) {
            let sc = clamp_load(px, vec2<i32>(x, y), render_size());
            let i = (y + 1) * 3 + x + 1;
            let masks = load_clamped(reactive_in, sc);
            colours[i] = load_clamped(colour_in, sc).xyz;
            reactives[i] = masks.x;
            compositions[i] = masks.y;
            masks_sum += masks.x + masks.y;
        }
    }
    if (masks_sum > 0.0) {
        for (var i = 0; i < 9; i++) {
            let c = colours[i];
            let max_len_sq = max(dot(reference, reference), dot(c, c));
            let similarity = dot(reference, c) / max_len_sq;
            // A higher power for dissimilar samples.
            let power_bias_max = 6.0;
            let similarity_power = 1.0 + (power_bias_max - similarity * power_bias_max);
            let weighted_reactive = pow(reactives[i], similarity_power);
            let weighted_composition = pow(compositions[i], similarity_power);
            reactive = max(reactive, vec2<f32>(weighted_reactive, weighted_composition));
        }
    }
    return reactive;
}

fn compute_prepared_input_color(px: vec2<i32>) -> vec3<f32> {
    // Linear input assumed.
    var rgb = max(vec3<f32>(0.0), load_clamped(colour_in, px).rgb);
    rgb = prepare_rgb(rgb, exposure_of(exposure_in), cb.pre_exposure);
    return rgb_to_ycocg(rgb);
}

fn evaluate_surface(px: vec2<i32>) -> f32 {
    let d0 = get_view_space_depth(load_reconstructed_prev_depth(px + vec2<i32>(0, -1)));
    let d1 = get_view_space_depth(load_reconstructed_prev_depth(px));
    let d2 = get_view_space_depth(load_reconstructed_prev_depth(px + vec2<i32>(0, 1)));
    return 1.0 - f32(((d0 - d1) > (d1 * 0.01)) && ((d1 - d2) > (d2 * 0.01)));
}

@compute @workgroup_size(8, 8, 1)
fn depth_clip(@builtin(global_invocation_id) gid: vec3<u32>) {
    let px = vec2<i32>(gid.xy);
    if (any(px >= render_size())) {
        return;
    }
    let depth_uv = (vec2<f32>(px) + 0.5) / render_size_f();
    var mv = load_clamped(dilated_motion_in, px).xy;
    // Tiny motion is discarded.
    mv *= f32(length(mv * display_size_f()) > 0.01);
    let dilated_uv = depth_uv + mv;
    let dilated_depth = load_clamped(dilated_depth_in, px).x;
    var clip = compute_depth_clip(dilated_uv, dilated_depth) * evaluate_surface(px);
    // The engine's rule (not AMD's): a hit inside an edit box is fully disoccluded.
    if (load_clamped(reactive_in, px).z > 0.5) {
        clip = 1.0;
    }
    textureStore(prepared_out, px, vec4<f32>(compute_prepared_input_color(px), clip));
    // Motion vectors at the render size: the divergence is read at the pixel itself.
    let motion_divergence = compute_motion_divergence(px, render_size());
    let temporal_motion_difference = saturate(compute_temporal_motion_divergence(px) - compute_depth_divergence(px));
    let masks = pre_process_reactive_masks(px, max(temporal_motion_difference, motion_divergence));
    textureStore(dilated_reactive_out, px, vec4<f32>(masks, 0.0, 0.0));
}
