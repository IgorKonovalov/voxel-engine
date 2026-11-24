// The FSR 2 input pass (ours, ADR-0034): FSR 2's depth, motion-vector and reactive inputs from
// the hit buffer, at the internal size, one invocation per internal pixel. Compiled after
// common.wgsl, hitrec.wgsl, entity.wgsl, edits.wgsl and the sky's modules, whose bindings (group 0), views, entity
// transforms and reprojection it uses.
//
// Bindings, group 2:
//   0  texture_2d<f32>   in_pos          this frame's hit positions (rgba32float, t in w)
//   1  texture_2d<u32>   in_info         this frame's hit records
//   2  texture_2d<f32>   in_layer        the hit pass's translucent layers (transmittance w)
//   3  uniform           in_tuning       `InputUniform` in fsr2/prepare.rs
//   4  uniform           edits           the frame's edit boxes (edits.wgsl's `Edits`)
//   5  uniform           in_lamp         the light-change thresholds (`LightChangeUniform`
//                                        in frame/upscale.rs; the older list is
//                                        `lights.old`, group 0)
//   6  storage texture   depth_out       r32float: inverted infinite device depth,
//                                        `near / z_view`, 0 on a miss
//   7  storage texture   motion_out      rg32float: motion to the previous frame, in UV
//   8  storage texture   reactive_out    rgba8unorm: FSR's reactive mask in x, its
//                                        transparency-and-composition mask in y (zero), and
//                                        in z 1 where the hit lies in an edit box dilated by
//                                        one voxel (the depth-clip pass's edit flag)
//   9  texture_2d<f32>   in_emissive     the temporal pass's emissive part (rgb, lighting
//                                        grid): where an emission change lands; a: where a
//                                        fresh pixel lies
//
// Motion: where the hit of pixel `px` was in the frame before (`prev_world`: a moved entity
// part's voxel through the part's transform of the frame before, any other hit where it is)
// reprojected through the previous view as if both frames' rays went through their pixels'
// centres (`reproject_centred`), less the pixel's centre, over the internal size; a miss
// reprojects its ray direction (rotation only); a point behind the previous view moves off
// screen. When the previous view is this frame's view (jitter aside) the motion is 0 exactly
// at every pixel but a moved part's.
//
// Reactive: the largest of a translucent layer's `1 - transmittance` times
// `in_tuning.reactive_translucent` (capped at 1), 1 where the hit lies in an edit box dilated
// by one voxel, and `in_tuning.reactive_light` where the light at a still hit changed (still:
// the hit moved less than STILL_PX output pixels; changed: an emission scale changed where an
// emitter's light lands, or the list of lights' summed unshadowed term moved past the
// thresholds since the list of `CHANGE_FRAMES` frames before, or the sky's key light or scale moved past them since the frame before:
// `sky_light_changed`, which a jump trips and a drift does not). A moving
// pixel's lamp term changes with the camera that carries the lamp; FSR's rectification takes
// that, where the mask would take its history. A fresh pixel (light.wgsl: a shadow ray crossed
// a moving entity's box, or the hit is a moved part's; `fresh_lands`) takes
// `in_tuning.reactive_light` moving or still.

@group(2) @binding(0) var in_pos: texture_2d<f32>;
@group(2) @binding(1) var in_info: texture_2d<u32>;
@group(2) @binding(2) var in_layer: texture_2d<f32>;

struct Fsr2InputTuning {
    near: f32,
    reactive_translucent: f32,
    reactive_light: f32,
    _pad0: f32,
};

@group(2) @binding(3) var<uniform> in_tuning: Fsr2InputTuning;

@group(2) @binding(4) var<uniform> edits: Edits;

// upscale.wgsl's `UpTuning`, field for field (`UpscaleUniform` in frame/upscale.rs).
struct LightChange {
    change_rel: f32,
    change_abs: f32,
    _pad0: f32,
    _pad1: f32,
};

@group(2) @binding(5) var<uniform> in_lamp: LightChange;
@group(2) @binding(6) var depth_out: texture_storage_2d<r32float, write>;
@group(2) @binding(7) var motion_out: texture_storage_2d<rg32float, write>;
@group(2) @binding(8) var reactive_out: texture_storage_2d<rgba8unorm, write>;
@group(2) @binding(9) var in_emissive: texture_2d<f32>;

// A motion that leaves the screen: FSR finds no history there.
const OFF_SCREEN: vec2<f32> = vec2<f32>(-4.0, -4.0);
// upscale.wgsl's CLIP_STILL: a pixel moving less than this many output pixels is still.
const STILL_PX: f32 = 0.01;

// Whether `a` is `b` but for the jitter: the same eye, basis, frustum and rectangle.
fn same_view(a: View, b: View) -> bool {
    return all(a.origin == b.origin) && all(a.right == b.right) && all(a.up == b.up)
        && all(a.fwd == b.fwd) && a.tan_left == b.tan_left && a.tan_right == b.tan_right
        && a.tan_up == b.tan_up && a.tan_down == b.tan_down && all(a.rect == b.rect);
}

// upscale.wgsl's `up_light_changed` at internal pixel `px`.
fn light_changed(px: vec2<i32>, rec: HitRecord) -> bool {
    if ((params.lighting & LIGHTING_EMISSION_CHANGED) != 0u && emission_lands(in_emissive, px, rec)) {
        return true;
    }
    if (sky_light_changed(rec.p, rec.n, in_lamp.change_abs, in_lamp.change_rel)) {
        return true;
    }
    if (lights.changed == 0u) {
        return false;
    }
    let now = lights_term(false, rec.p, rec.n);
    let then = lights_term(true, rec.p, rec.n);
    let d = abs(now - then);
    return d > in_lamp.change_abs && d > in_lamp.change_rel * max(now, then);
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
    var reactive = 0.0;
    var edit = 0.0;
    let still = same_view(v, prev);
    if (rec.found) {
        let z = dot(rec.p - v.origin, v.fwd);
        depth = in_tuning.near / max(z, 1e-6);
        if (!still || entity_moved(rec)) {
            let q = reproject_centred(prev, v.jitter, prev_world(rec));
            motion = select((q - (vec2<f32>(px) + 0.5)) / vec2<f32>(params.size), OFF_SCREEN, q.x < 0.0);
        }
        if (in_edit(rec.p)) {
            reactive = 1.0;
            edit = 1.0;
        }
        let moved = length(motion * vec2<f32>(params.output_size));
        if ((moved < STILL_PX && light_changed(px, rec)) || fresh_lands(in_emissive, px)) {
            reactive = max(reactive, in_tuning.reactive_light);
        }
    } else if (!still) {
        let d = pixel_ray(gid.xy).d;
        let q = reproject_centred(prev, v.jitter, prev.origin + d);
        motion = select((q - (vec2<f32>(px) + 0.5)) / vec2<f32>(params.size), OFF_SCREEN, q.x < 0.0);
    }
    if (rec.translucent) {
        let layer = textureLoad(in_layer, px, 0);
        reactive = max(reactive, min(1.0, (1.0 - layer.w) * in_tuning.reactive_translucent));
    }
    textureStore(depth_out, px, vec4<f32>(depth, 0.0, 0.0, 0.0));
    textureStore(motion_out, px, vec4<f32>(motion, 0.0, 0.0));
    textureStore(reactive_out, px, vec4<f32>(reactive, 0.0, edit, 0.0));
}
