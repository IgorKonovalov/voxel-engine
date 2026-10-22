// FSR 2.2.1's reconstruct-previous-depth pass, ported to WGSL (ADR-0034) from tag v2.2.1 of
// GPUOpen-Effects/FidelityFX-FSR2: `ffx_fsr2_reconstruct_previous_depth_pass.glsl` and
// `ffx_fsr2_reconstruct_dilated_velocity_and_previous_depth.h`. One invocation per render
// pixel. Compiled after common.wgsl, which carries AMD's notice.
//
// AMD's `InterlockedMax` on an r32uint image becomes `atomicMax` on a storage buffer indexed
// by render pixel, row-major; the depths are non-negative, so their bits order as the values.
//
// Bindings, group 1:
//   0  texture_2d<f32>   depth_in           our input pass's device depth (r32float)
//   1  texture_2d<f32>   motion_in          our input pass's motion vectors (rg32float, UV)
//   2  texture_2d<f32>   colour_in          the composite's linear colour
//   3  texture_2d<f32>   exposure_in        rg32float 1x1
//   4  storage buffer    reconstructed      atomic u32 per render pixel: the previous frame's
//                                           nearest depth, as f32 bits (the lock pass clears
//                                           it for the next frame)
//   5  storage texture   dilated_depth_out  r32float
//   6  storage texture   dilated_motion_out rg32float: this frame's dilated motion
//   7  storage texture   lock_luma_out      r32float: the lock input luma

@group(1) @binding(0) var depth_in: texture_2d<f32>;
@group(1) @binding(1) var motion_in: texture_2d<f32>;
@group(1) @binding(2) var colour_in: texture_2d<f32>;
@group(1) @binding(3) var exposure_in: texture_2d<f32>;
@group(1) @binding(4) var<storage, read_write> reconstructed: array<atomic<u32>>;
@group(1) @binding(5) var dilated_depth_out: texture_storage_2d<r32float, write>;
@group(1) @binding(6) var dilated_motion_out: texture_storage_2d<rg32float, write>;
@group(1) @binding(7) var lock_luma_out: texture_storage_2d<r32float, write>;

fn load_input_motion_vector(p: vec2<i32>) -> vec2<f32> {
    return load_clamped(motion_in, p).xy * cb.motion_vector_scale;
}

// StoreReconstructedDepth with inverted depth: the maximum.
fn store_reconstructed_depth(p: vec2<i32>, depth: f32) {
    atomicMax(&reconstructed[u32(p.y) * u32(cb.render_size.x) + u32(p.x)], bitcast<u32>(depth));
}

fn reconstruct_prev_depth(px: vec2<i32>, depth: f32, motion_in_uv: vec2<f32>, depth_size: vec2<i32>) {
    let motion = motion_in_uv * f32(length(motion_in_uv * display_size_f()) > 0.1);
    let uv = (vec2<f32>(px) + 0.5) / vec2<f32>(depth_size);
    let reprojected = uv + motion;
    let b = get_bilinear_sampling_data(reprojected, render_size());
    // Project current depth into the previous frame: every pixel the bilinear reprojection
    // touches gets it.
    for (var i = 0; i < 4; i++) {
        let w = bilinear_weight(b, i);
        if (w > RECONSTRUCTED_DEPTH_BILINEAR_WEIGHT_THRESHOLD) {
            let store_pos = b.base + bilinear_offset(i);
            if (is_on_screen(store_pos, depth_size)) {
                store_reconstructed_depth(store_pos, depth);
            }
        }
    }
}

// FindNearestDepth with inverted depth: the largest of the 3 x 3, and where it lies.
struct NearestDepth {
    depth: f32,
    coord: vec2<i32>,
};

fn find_nearest_depth(px: vec2<i32>, size: vec2<i32>) -> NearestDepth {
    var offsets = array<vec2<i32>, 9>(
        vec2<i32>(0, 0), vec2<i32>(1, 0), vec2<i32>(0, 1), vec2<i32>(0, -1), vec2<i32>(-1, 0),
        vec2<i32>(-1, 1), vec2<i32>(1, 1), vec2<i32>(-1, -1), vec2<i32>(1, -1),
    );
    var nearest = NearestDepth(load_clamped(depth_in, px).x, px);
    for (var i = 1; i < 9; i++) {
        let p = px + offsets[i];
        if (is_on_screen(p, size)) {
            let d = load_clamped(depth_in, p).x;
            if (d > nearest.depth) {
                nearest = NearestDepth(d, p);
            }
        }
    }
    return nearest;
}

fn compute_lock_input_luma(px: vec2<i32>) -> f32 {
    var rgb = max(vec3<f32>(0.0), load_clamped(colour_in, px).rgb);
    // The internal exposure for the locking logic.
    rgb /= cb.pre_exposure;
    rgb *= exposure_of(exposure_in);
    if (HDR) {
        rgb = tonemap(rgb);
    }
    // The luma locks compare; any other use must move the pow.
    return pow(rgb_to_perceived_luma(rgb), 1.0 / 6.0);
}

@compute @workgroup_size(8, 8, 1)
fn reconstruct_and_dilate(@builtin(global_invocation_id) gid: vec3<u32>) {
    let px = vec2<i32>(gid.xy);
    if (any(px >= render_size())) {
        return;
    }
    let nearest = find_nearest_depth(px, render_size());
    // Motion vectors at the render size: the nearest depth's own.
    let motion = load_input_motion_vector(nearest.coord);
    textureStore(dilated_depth_out, px, vec4<f32>(nearest.depth, 0.0, 0.0, 0.0));
    textureStore(dilated_motion_out, px, vec4<f32>(motion, 0.0, 0.0));
    reconstruct_prev_depth(px, nearest.depth, motion, render_size());
    textureStore(lock_luma_out, px, vec4<f32>(compute_lock_input_luma(px), 0.0, 0.0, 0.0));
}
