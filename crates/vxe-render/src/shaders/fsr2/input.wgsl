// The FSR 2 input pass (ours, ADR-0034): FSR 2's depth and motion-vector inputs from the hit
// buffer, at the internal size, one invocation per internal pixel. Compiled after march.wgsl,
// whose bindings (group 0), views and reprojection it uses.
//
// Bindings, group 2:
//   0  texture_2d<f32>   in_pos          this frame's hit positions (rgba32float, t in w)
//   1  texture_2d<u32>   in_info         this frame's hit records
//   2  texture_2d<f32>   in_colour       the composite's colour (internal size)
//   3  uniform           in_tuning       `Fsr2InputUniform` in fsr2.rs
//   4  storage texture   depth_out       r32float: inverted infinite device depth,
//                                        `near / z_view`, 0 on a miss
//   5  storage texture   motion_out      rg32float: motion to the previous frame, in UV
//   6  storage texture   prepared_out    rgba16float: the prepared input colour, YCoCg, with
//                                        a depth clip of 0 in a
//
// Motion: the hit `p` of pixel `px` reprojected through the previous view as if both frames'
// rays went through their pixels' centres (`reproject_centred`), less the pixel's centre, over
// the internal size; a miss reprojects its ray direction (rotation only); a point behind the
// previous view moves off screen. When the previous view is this frame's view (jitter aside)
// the motion is 0 exactly.

@group(2) @binding(0) var in_pos: texture_2d<f32>;
@group(2) @binding(1) var in_info: texture_2d<u32>;
@group(2) @binding(2) var in_colour: texture_2d<f32>;

struct Fsr2InputTuning {
    near: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(2) @binding(3) var<uniform> in_tuning: Fsr2InputTuning;
@group(2) @binding(4) var depth_out: texture_storage_2d<r32float, write>;
@group(2) @binding(5) var motion_out: texture_storage_2d<rg32float, write>;
@group(2) @binding(6) var prepared_out: texture_storage_2d<rgba16float, write>;

// A motion that leaves the screen: FSR finds no history there.
const OFF_SCREEN: vec2<f32> = vec2<f32>(-4.0, -4.0);

// Whether `a` is `b` but for the jitter: the same eye, basis, frustum and rectangle.
fn same_view(a: View, b: View) -> bool {
    return all(a.origin == b.origin) && all(a.right == b.right) && all(a.up == b.up)
        && all(a.fwd == b.fwd) && a.tan_left == b.tan_left && a.tan_right == b.tan_right
        && a.tan_up == b.tan_up && a.tan_down == b.tan_down && all(a.rect == b.rect);
}

@compute @workgroup_size(8, 8, 1)
fn fsr2_input(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.size.x || gid.y >= params.size.y) {
        return;
    }
    let px = vec2<i32>(gid.xy);
    let vi = view_of(gid.xy);
    let v = params.views[vi];
    let prev = params.prev_views[vi];
    let rec = unpack_hit(textureLoad(in_pos, px, 0), textureLoad(in_info, px, 0));
    var depth = 0.0;
    var motion = vec2<f32>(0.0);
    let still = same_view(v, prev);
    if (rec.found) {
        let z = dot(rec.p - v.origin, v.fwd);
        depth = in_tuning.near / max(z, 1e-6);
        if (!still) {
            let q = reproject_centred(prev, v.jitter, rec.p);
            motion = select((q - (vec2<f32>(px) + 0.5)) / vec2<f32>(params.size), OFF_SCREEN, q.x < 0.0);
        }
    } else if (!still) {
        let d = pixel_ray(gid.xy).d;
        let q = reproject_centred(prev, v.jitter, prev.origin + d);
        motion = select((q - (vec2<f32>(px) + 0.5)) / vec2<f32>(params.size), OFF_SCREEN, q.x < 0.0);
    }
    textureStore(depth_out, px, vec4<f32>(depth, 0.0, 0.0, 0.0));
    textureStore(motion_out, px, vec4<f32>(motion, 0.0, 0.0));
    // ComputePreparedInputColor with exposure 1 and no depth clip.
    let c = max(vec3<f32>(0.0), textureLoad(in_colour, px, 0).xyz);
    let prepared = clamp(c, vec3<f32>(0.0), vec3<f32>(65504.0));
    let ycocg = vec3<f32>(
        0.25 * prepared.r + 0.5 * prepared.g + 0.25 * prepared.b,
        0.5 * prepared.r - 0.5 * prepared.b,
        -0.25 * prepared.r + 0.5 * prepared.g - 0.25 * prepared.b,
    );
    textureStore(prepared_out, px, vec4<f32>(ycocg, 0.0));
}
