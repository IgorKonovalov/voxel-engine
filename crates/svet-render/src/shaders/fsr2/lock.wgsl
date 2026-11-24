// FSR 2.2.1's lock pass, ported to WGSL (ADR-0034) from tag v2.2.1 of
// GPUOpen-Effects/FidelityFX-FSR2: `ffx_fsr2_lock_pass.glsl` and `ffx_fsr2_lock.h`. One
// invocation per render pixel. Compiled after common.wgsl, which carries AMD's notice.
//
// Bindings, group 1:
//   0  texture_2d<f32>   lock_luma_in    the reconstruct pass's lock input luma (r32float)
//   1  storage buffer    new_locks       f32 per display pixel, row-major (AMD's read-write
//                                        r8 image, which the accumulate pass reads and clears)
//   2  storage buffer    reconstructed   atomic u32 per render pixel: cleared to the far depth
//                                        (0, inverted) for the next frame

@group(1) @binding(0) var lock_luma_in: texture_2d<f32>;
@group(1) @binding(1) var<storage, read_write> new_locks: array<f32>;
@group(1) @binding(2) var<storage, read_write> reconstructed: array<atomic<u32>>;

fn load_lock_input_luma(p: vec2<i32>) -> f32 {
    return load_clamped(lock_luma_in, p).x;
}

// ComputeThinFeatureConfidence: the pixel is a ridge in luma against its 3 x 3 and no 2 x 2
// quadrant around it is all similar to it.
fn compute_thin_feature_confidence(pos: vec2<i32>) -> bool {
    let nucleus = load_lock_input_luma(pos);
    let similar_threshold = 1.05;
    var dissimilar_luma_min = FSR2_FLT_MAX;
    var dissimilar_luma_max = 0.0;
    // Bits, row-major from the top left: 0 1 2 / 3 4 5 / 6 7 8; the nucleus is similar.
    var mask = 1u << 4u;
    var idx = 0u;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            if (!(x == 0 && y == 0)) {
                let sample_pos = clamp_load(pos, vec2<i32>(x, y), render_size());
                let sample_luma = load_lock_input_luma(sample_pos);
                let difference = max(sample_luma, nucleus) / min(sample_luma, nucleus);
                if (difference > 0.0 && difference < similar_threshold) {
                    mask |= 1u << idx;
                } else {
                    dissimilar_luma_min = min(dissimilar_luma_min, sample_luma);
                    dissimilar_luma_max = max(dissimilar_luma_max, sample_luma);
                }
            }
            idx++;
        }
    }
    let is_ridge = nucleus > dissimilar_luma_max || nucleus < dissimilar_luma_min;
    if (!is_ridge) {
        return false;
    }
    // Upper left, upper right, lower left, lower right.
    let rejection = array<u32, 4>(
        (1u << 0u) | (1u << 1u) | (1u << 3u) | (1u << 4u),
        (1u << 1u) | (1u << 2u) | (1u << 4u) | (1u << 5u),
        (1u << 3u) | (1u << 4u) | (1u << 6u) | (1u << 7u),
        (1u << 4u) | (1u << 5u) | (1u << 7u) | (1u << 8u),
    );
    if ((mask & rejection[0]) == rejection[0] || (mask & rejection[1]) == rejection[1]
        || (mask & rejection[2]) == rejection[2] || (mask & rejection[3]) == rejection[3]) {
        return false;
    }
    return true;
}

@compute @workgroup_size(8, 8, 1)
fn compute_lock(@builtin(global_invocation_id) gid: vec3<u32>) {
    let lr = vec2<i32>(gid.xy);
    if (any(lr >= render_size())) {
        return;
    }
    if (compute_thin_feature_confidence(lr)) {
        let hr = compute_hr_pos_from_lr_pos(lr);
        if (all(hr >= vec2<i32>(0)) && all(hr < display_size())) {
            new_locks[u32(hr.y) * u32(cb.display_size.x) + u32(hr.x)] = 1.0;
        }
    }
    // ClearResourcesForNextFrame: the far depth, inverted.
    atomicStore(&reconstructed[u32(lr.y) * u32(cb.render_size.x) + u32(lr.x)], 0u);
}
