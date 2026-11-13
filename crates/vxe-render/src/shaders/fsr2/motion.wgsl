// The input motion vector at pixel `p`, scaled to FSR 2's convention: read from `motion_in`,
// which each pass that joins this module declares at its own binding (depth_clip.wgsl,
// reconstruct_previous_depth.wgsl).
fn load_input_motion_vector(p: vec2<i32>) -> vec2<f32> {
    return load_clamped(motion_in, p).xy * cb.motion_vector_scale;
}
