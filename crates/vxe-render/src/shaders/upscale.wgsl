// The upscale pass (ADR-0024): a temporal upscaler from the composite's internal pixels to the
// output size, one invocation per output pixel. Compiled after march.wgsl, whose bindings
// (group 0), views, hit-record helpers and reprojection it uses.
//
// Bindings, group 2 of `upscale`:
//   0  texture_2d<f32>   up_pos_in       this frame's hit positions (internal size)
//   1  texture_2d<u32>   up_info_in      this frame's hit records
//   2  texture_2d<f32>   up_colour_in    the composite's display-encoded colour, the internal
//                                        image in its top-left `params.size` pixels
//   3  texture_2d<f32>   up_prev_pos_in  last frame's hit positions
//   4  texture_2d<u32>   up_prev_info_in last frame's hit records
//   5  texture_2d<f32>   up_prev_hist_in last frame's history (output size)
//   6  storage texture   up_hist_out     rgba16float: this frame's history, the upscaled
//                                        image, display-encoded rgb, and in a the weight
//                                        of the current colour in the blend (1 without
//                                        history)
//   7  uniform           up_edits        the frame's edit boxes (temporal.wgsl's `Edits`)
//   8  uniform           up_tuning       the kernel width, blend weights, clip and light
//                                        change tuning (`Upscaler` in marcher.rs)
//
// Coordinates: internal pixel `i` spans `[i, i + 1)` and this frame's sample of it lies at
// `i + 0.5 + jitter`; output pixel `o`'s centre is at internal `(o + 0.5) size / output_size`.
//
// Per output pixel, in the shaded view: the nearest internal sample's hit `rec` (the internal
// pixel whose jittered sample lies nearest the centre). The current colour is the kernel
// weighted mean of the 3 x 3 samples around the nearest (a narrow Gaussian at each sample's
// jittered position, distances in output pixels), clamped to their range. History is taken when `Params.history` bit 1 says it is valid, `rec`
// hit outside every edit box dilated by one voxel, and last frame's hit buffer agrees with
// `rec` at one of the 3 x 3 pixels around where `rec.p` reprojects through the previous view:
// a hit, the same material, a normal within UP_HISTORY_COS, a position within
// `UP_HISTORY_DIST + UP_HISTORY_DIST_PER_T t` voxels (temporal.wgsl's thresholds). The history
// is read by Catmull-Rom where the output centre moved with the sample's motion, clipped
// toward the centre of the YCoCg box of the 3 x 3 samples (their mean plus and minus
// `clip_gamma` standard deviations, inside their minimum and maximum) unless it held still
// and the light at its hit did not change, and blended with the current colour at `alpha`
// (still) or `alpha_moving`, times the nearest sample's kernel weight over `w_mean`, at most
// 1; a pixel without history takes the current colour (alpha 1). Every other view takes the nearest sample's colour unblended.


@group(2) @binding(0) var up_pos_in: texture_2d<f32>;
@group(2) @binding(1) var up_info_in: texture_2d<u32>;
@group(2) @binding(2) var up_colour_in: texture_2d<f32>;
@group(2) @binding(3) var up_prev_pos_in: texture_2d<f32>;
@group(2) @binding(4) var up_prev_info_in: texture_2d<u32>;
@group(2) @binding(5) var up_prev_hist_in: texture_2d<f32>;
@group(2) @binding(6) var up_hist_out: texture_storage_2d<rgba16float, write>;

// temporal.wgsl's `Edits`: up to MAX_EDITS boxes, box i from `boxes[2 i]` to `boxes[2 i + 1]`.
struct UpEdits {
    count: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    boxes: array<vec4<i32>, 128>,
};

@group(2) @binding(7) var<uniform> up_edits: UpEdits;

// `UpscaleUniform` in marcher.rs.
struct UpTuning {
    // The kernel `exp(-2.29 (d / width)^2)`, `d` the sample's distance from the output
    // pixel's centre in output pixels.
    width: f32,
    alpha: f32,
    // `alpha` where the history moved by CLIP_STILL output pixels or more.
    alpha_moving: f32,
    // The clip box: the 3 x 3's YCoCg mean plus and minus `clip_gamma` deviations.
    clip_gamma: f32,
    // The nearest sample's mean kernel weight at this scale (the blend divides by it).
    w_mean: f32,
    // A light change under a still pixel: the lamp's unshadowed term at the
    // hit moved by more than `change_rel` of the larger of the two and `change_abs` since the
    // lamp of `old_*` (an older frame's), or an emission scale changed.
    change_rel: f32,
    change_abs: f32,
    old_on: u32,
    old_pos: vec3<f32>,
    old_intensity: f32,
    old_dir: vec3<f32>,
    old_cos_outer: f32,
    old_cos_inner: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(2) @binding(8) var<uniform> up_tuning: UpTuning;

const CLIP_STILL: f32 = 0.01;
// temporal.wgsl's HISTORY_COS, HISTORY_DIST and HISTORY_DIST_PER_T.
const UP_HISTORY_COS: f32 = 0.8;
const UP_HISTORY_DIST: f32 = 0.5;
const UP_HISTORY_DIST_PER_T: f32 = 0.03;

// Whether `p` lies inside an edit box dilated by one voxel, faces included.
fn up_in_edit(p: vec3<f32>) -> bool {
    for (var i = 0u; i < up_edits.count; i++) {
        let lo = vec3<f32>(up_edits.boxes[2u * i].xyz - 1);
        let hi = vec3<f32>(up_edits.boxes[2u * i + 1u].xyz + 2);
        if (all(p >= lo) && all(p <= hi)) {
            return true;
        }
    }
    return false;
}

// The reconstruction kernel's weight of a sample `d` output pixels (a vector) from the output
// pixel's centre, 1 at the centre.
fn up_kernel(d: vec2<f32>) -> f32 {
    let s = up_tuning.width;
    return exp(-2.29 * dot(d, d) / (s * s));
}

// The headlamp's `intensity * cone * max(n.l, 0) / dist^2` at `p`, normal `n`, without the
// shadow ray (light.wgsl's `lamp_lit`); 0 when the lamp is off.
fn up_lamp(on: bool, pos: vec3<f32>, dir: vec3<f32>, intensity: f32, cos_o: f32, cos_i: f32, p: vec3<f32>, n: vec3<f32>) -> f32 {
    if (!on) {
        return 0.0;
    }
    let to_light = pos - p;
    let dist2 = max(dot(to_light, to_light), 1e-4);
    let l = to_light * inverseSqrt(dist2);
    let cone = smoothstep(cos_o, cos_i, dot(-l, dir));
    return intensity * cone * max(dot(n, l), 0.0) / dist2;
}

// The exposure's move between frames, in EV, past which every still pixel is clipped as a
// moving one.
const EXPOSURE_MOVE: f32 = 1.0 / 64.0;

// Whether the light at `rec` changed since the older frame `up_tuning` holds: an emission
// scale changed within EMISSION_CHANGE_FRAMES, the exposure moved by more than EXPOSURE_MOVE EV
// since the last frame (the history is display-encoded, so it cannot be rescaled: ADR-0033),
// or the lamp's term moved past the thresholds.
fn up_light_changed(rec: HitRecord) -> bool {
    if ((params.lighting & LIGHTING_EMISSION_CHANGED) != 0u) {
        return true;
    }
    if (abs(log2(exposure_ratio())) > EXPOSURE_MOVE) {
        return true;
    }
    let now = up_lamp((params.lighting & LIGHTING_LAMP) != 0u, params.light_pos, params.light_dir,
        params.light_intensity, params.cos_outer, params.cos_inner, rec.p, rec.n);
    let then = up_lamp(up_tuning.old_on != 0u, up_tuning.old_pos, up_tuning.old_dir,
        up_tuning.old_intensity, up_tuning.old_cos_outer, up_tuning.old_cos_inner, rec.p, rec.n);
    let d = abs(now - then);
    return d > up_tuning.change_abs && d > up_tuning.change_rel * max(now, then);
}

fn to_ycocg(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        0.25 * c.x + 0.5 * c.y + 0.25 * c.z,
        0.5 * c.x - 0.5 * c.z,
        -0.25 * c.x + 0.5 * c.y - 0.25 * c.z,
    );
}

fn from_ycocg(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(c.x + c.y - c.z, c.x + c.z, c.x - c.y - c.z);
}

fn hist_texel(t: vec2<i32>) -> vec3<f32> {
    let n = vec2<i32>(params.output_size) - vec2<i32>(1);
    return textureLoad(up_prev_hist_in, clamp(t, vec2<i32>(0), n), 0).xyz;
}

// Last frame's history at continuous output position `h` (pixel centres at + 0.5), clamped
// to the edge: Catmull-Rom over the 4 x 4 texels around `h` (its negative lobes held off
// below zero; the clip bounds the overshoot above).
fn history_at(h: vec2<f32>) -> vec3<f32> {
    let q = h - 0.5;
    let b = vec2<i32>(floor(q));
    let f = q - floor(q);
    // Catmull-Rom weights of the texels at offsets -1, 0, 1, 2 for fraction f.
    let w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    let w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    let w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    let w3 = f * f * (-0.5 + 0.5 * f);
    var wx = array<f32, 4>(w0.x, w1.x, w2.x, w3.x);
    var wy = array<f32, 4>(w0.y, w1.y, w2.y, w3.y);
    var sum = vec3<f32>(0.0);
    for (var y = 0; y < 4; y++) {
        var row = vec3<f32>(0.0);
        for (var x = 0; x < 4; x++) {
            row += wx[x] * hist_texel(b + vec2<i32>(x - 1, y - 1));
        }
        sum += wy[y] * row;
    }
    return max(sum, vec3<f32>(0.0));
}

// Whether last frame saw `rec`'s surface around internal pixel `k`: any of the 3 x 3 pixels
// around it hit with `rec`'s material, a normal within UP_HISTORY_COS and a position within
// the distance tolerance. A jittered sample beside a face's edge reprojects onto the
// neighbouring face as often as onto its own, so one pixel alone would reject every edge.
fn prev_surface_seen(k: vec2<i32>, rec: HitRecord) -> bool {
    let last = vec2<i32>(params.size) - vec2<i32>(1);
    let tol = UP_HISTORY_DIST + UP_HISTORY_DIST_PER_T * rec.t;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let tap = k + vec2<i32>(dx, dy);
            if (any(tap < vec2<i32>(0)) || any(tap > last)) {
                continue;
            }
            let prev = unpack_hit(textureLoad(up_prev_pos_in, tap, 0), textureLoad(up_prev_info_in, tap, 0));
            if (prev.found && prev.material == rec.material && dot(prev.n, rec.n) >= UP_HISTORY_COS
                && distance(prev.p, rec.p) <= tol) {
                return true;
            }
        }
    }
    return false;
}

@compute @workgroup_size(8, 8, 1)
fn upscale(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.output_size.x || gid.y >= params.output_size.y) {
        return;
    }
    let o = vec2<i32>(gid.xy);
    let size = vec2<i32>(params.size);
    let last = size - vec2<i32>(1);
    // Internal pixels per output pixel, on each edge.
    let ratio = vec2<f32>(params.size) / vec2<f32>(params.output_size);
    let centre = (vec2<f32>(gid.xy) + 0.5) * ratio;
    let vi = view_of(vec2<u32>(clamp(vec2<i32>(floor(centre)), vec2<i32>(0), last)));
    let v = params.views[vi];
    let near = clamp(vec2<i32>(floor(centre - v.jitter)), vec2<i32>(0), last);
    if (view_mode() != VIEW_SHADED) {
        textureStore(up_hist_out, o, vec4<f32>(textureLoad(up_colour_in, near, 0).xyz, 1.0));
        return;
    }
    let rec = unpack_hit(textureLoad(up_pos_in, near, 0), textureLoad(up_info_in, near, 0));
    var sum = vec3<f32>(0.0);
    var w_sum = 0.0;
    var w_near = 0.0;
    var lo_rgb = vec3<f32>(1e9);
    var hi_rgb = vec3<f32>(-1e9);
    var lo = vec3<f32>(1e9);
    var hi = vec3<f32>(-1e9);
    var m1 = vec3<f32>(0.0);
    var m2 = vec3<f32>(0.0);
    var n = 0.0;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let tap = near + vec2<i32>(dx, dy);
            if (any(tap < vec2<i32>(0)) || any(tap > last)) {
                continue;
            }
            let c = textureLoad(up_colour_in, tap, 0).xyz;
            let d = (vec2<f32>(tap) + 0.5 + v.jitter - centre) / ratio;
            let w = up_kernel(d);
            if (dx == 0 && dy == 0) {
                w_near = w;
            }
            sum += w * c;
            w_sum += w;
            let y = to_ycocg(c);
            lo_rgb = min(lo_rgb, c);
            hi_rgb = max(hi_rgb, c);
            lo = min(lo, y);
            hi = max(hi, y);
            m1 += y;
            m2 += y * y;
            n += 1.0;
        }
    }
    var current = textureLoad(up_colour_in, near, 0).xyz;
    if (w_sum > 1e-3) {
        current = clamp(sum / w_sum, lo_rgb, hi_rgb);
    }
    var result = current;
    var alpha = 1.0;
    if ((params.history & 2u) != 0u && rec.found && !up_in_edit(rec.p)) {
        let pv = params.prev_views[vi];
        let y = project_prev(pv, rec.p);
        // As the temporal pass: the previous pixel as if both rays went through centres.
        let k = vec2<i32>(floor(y - v.jitter));
        if (y.x >= 0.0 && all(k >= vec2<i32>(0)) && all(k <= last)) {
            if (prev_surface_seen(k, rec)) {
                // The sample's motion in unjittered internal pixels moves the output centre.
                let moved = centre + y - (vec2<f32>(near) + 0.5 + v.jitter);
                let h = moved / ratio;
                if (all(h >= vec2<f32>(0.0)) && all(h <= vec2<f32>(params.output_size))) {
                    let mean = m1 / n;
                    let sigma = sqrt(max(m2 / n - mean * mean, vec3<f32>(0.0)));
                    // A still pixel (its history moved less than CLIP_STILL output pixels)
                    // keeps its history unclipped: it is the average of the jittered samples,
                    // which one frame's 3 x 3 does not span. What changes under a still
                    // camera without moving the history is an edit (rejected above) or light,
                    // which is clipped and blended as motion.
                    var still = length(h - (vec2<f32>(gid.xy) + 0.5)) < CLIP_STILL;
                    if (still && up_light_changed(rec)) {
                        still = false;
                    }
                    let b_lo = select(max(lo, mean - up_tuning.clip_gamma * sigma), vec3<f32>(-1e9), still);
                    let b_hi = select(min(hi, mean + up_tuning.clip_gamma * sigma), vec3<f32>(1e9), still);
                    let mid = 0.5 * (b_lo + b_hi);
                    let ext = 0.5 * (b_hi - b_lo) + vec3<f32>(1e-5);
                    var hy = to_ycocg(history_at(h)) - mid;
                    let u = abs(hy / ext);
                    let m = max(u.x, max(u.y, u.z));
                    if (m > 1.0) {
                        hy /= m;
                    }
                    alpha = select(up_tuning.alpha_moving, up_tuning.alpha, still);
                    alpha = min(1.0, alpha * w_near / max(up_tuning.w_mean, 1e-3));
                    result = mix(from_ycocg(mid + hy), current, alpha);
                }
            }
        }
    }
    textureStore(up_hist_out, o, vec4<f32>(result, alpha));
}
