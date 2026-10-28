// The temporal pass: accumulates each pixel's irradiance and its moments over frames by
// reprojecting the hit through the previous frame's views (the world does not move; only the
// views and the edits do), and kills history where the world was edited. Compiled after
// march.wgsl, whose bindings (group 0) and hit-record helpers it uses.
//
// Bindings, group 2:
//   0  texture_2d<f32>   hit_pos_in      this frame's hit positions
//   1  texture_2d<u32>   hit_info_in     this frame's hit records
//   2  texture_2d<f32>   prev_pos_in     last frame's hit positions
//   3  texture_2d<u32>   prev_info_in    last frame's hit records
//   4  texture_2d<f32>   irradiance_in   this frame's irradiance from the light pass
//   5  texture_2d<f32>   prev_accum_in   last frame's accumulated irradiance
//   6  texture_2d<f32>   prev_moments_in last frame's moments: luminance, its square, age
//   7  storage texture   accum_out       rgba32float: the accumulated irradiance, the
//                                        emissive part in rgb, the lamp's (white) in a; full
//                                        precision, so a blend of 1 / emissive_history moves
//                                        it however long the history (ADR-0036)
//   8  storage texture   moments_out     rgba16float: first and second moments of the
//                                        irradiance's luminance, then the age: frames of
//                                        history reused, 0 after a reset
//   9  uniform           edits           the frame's edit boxes (see `Edits`)
//  10  texture_2d<f32>   strata_in       the gradient pass's strata (gradient.wgsl), read in
//                                        the gradient mode (Params.lighting bit 3): the
//                                        blend weight rises to twice the lambda averaged over
//                                        the 3 x 3 strata around the reprojected pixel's
//                                        (clamped to 1), and the age falls by that fraction
//
// A hit is reprojected through the previous frame's view of the same index to a fractional
// pixel as if both frames' rays went through their pixels' centres (`reproject_centred`: a
// still camera maps each pixel onto itself whatever the jitter); each of its four bilinear
// taps is accepted when that pixel hit, with the same material, a normal within HISTORY_COS
// of this one, and a position within `HISTORY_DIST + HISTORY_DIST_PER_T t` voxels of this
// hit. The accepted taps' history, moments and age are blended by their bilinear weights;
// with none accepted, the accepted pixel of the 3 x 3 around the nearest whose position lies
// nearest this hit's is taken alone; with none of those either, with no
// history (`Params.history` bit 0 clear), or with the hit inside an edit box dilated by
// one voxel, the pixel resets: age 0, accumulation this frame's sample. Otherwise the age is
// the taps' age plus one, capped at AGE_MAX. The lamp's part and the moments blend the new
// sample in at `max(ALPHA, 1 / (age + 1))`; the emissive part, static light, at
// `max(1 / (age + 1), 1 / Params.emissive_history)`, and at no less than ALPHA while
// Params.lighting bit 8 says an emission scale changed within the last frames. On the frame
// Params.lighting bit 9 says the lamp's light changed, the lamp's part takes this frame's
// sample as it is (and the moments follow it by the lamp's share): the light pass's lamp
// estimate is exact up to its shadow ray, so a lamp switched off leaves nothing behind.
//
// Every value is pre-exposed (ADR-0033): the history read from last frame is rescaled by
// `exposure_ratio()` (the second moment by its square) before it blends, the accumulation's
// store is clamped at HDR_MAX, and the moments are of the luminance clamped at MOMENT_LUM_MAX
// and stored at most MOMENT_LUM_MAX and its square.

@group(2) @binding(0) var hit_pos_in: texture_2d<f32>;
@group(2) @binding(1) var hit_info_in: texture_2d<u32>;
@group(2) @binding(2) var prev_pos_in: texture_2d<f32>;
@group(2) @binding(3) var prev_info_in: texture_2d<u32>;
@group(2) @binding(4) var irradiance_in: texture_2d<f32>;
@group(2) @binding(5) var prev_accum_in: texture_2d<f32>;
@group(2) @binding(6) var prev_moments_in: texture_2d<f32>;
@group(2) @binding(7) var accum_out: texture_storage_2d<rgba32float, write>;
@group(2) @binding(8) var moments_out: texture_storage_2d<rgba16float, write>;

// Up to MAX_EDITS boxes, inclusive voxel bounds: box i is `boxes[2 i]` to `boxes[2 i + 1]`.
struct Edits {
    count: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    boxes: array<vec4<i32>, 128>,
};

@group(2) @binding(9) var<uniform> edits: Edits;
@group(2) @binding(10) var strata_in: texture_2d<f32>;

// The gradient's gain: lambda = |new - old| / max(new, old) reads 0.9 where light fell to a
// tenth, and a blend at 0.9 would keep a tenth of the old light; twice lambda, clamped, takes
// such a pixel to its new light at once.
const GRADIENT_GAIN: f32 = 2.0;
// Strata edge in pixels; gradient.wgsl's STRATUM.
const GRADIENT_STRATUM: f32 = 3.0;

// The gradient pass's lambda at fractional pixel `q` of last frame, averaged over the 3 x 3
// strata around its stratum.
fn gradient_at(q: vec2<f32>) -> f32 {
    let s = vec2<i32>(floor(q / GRADIENT_STRATUM));
    let n = vec2<i32>(textureDimensions(strata_in));
    var sum = 0.0;
    var count = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let t = s + vec2<i32>(x, y);
            if (all(t >= vec2<i32>(0)) && all(t < n)) {
                sum += textureLoad(strata_in, t, 0).x;
                count += 1.0;
            }
        }
    }
    return sum / max(count, 1.0);
}

const ALPHA: f32 = 0.2;
const AGE_MAX: f32 = 2047.0;
const HISTORY_COS: f32 = 0.8;
const HISTORY_DIST: f32 = 0.5;
const HISTORY_DIST_PER_T: f32 = 0.03;

// Whether `p` lies inside an edit box dilated by one voxel: the continuous span of voxels
// `min - 1 ..= max + 1`, faces included.
fn in_edit(p: vec3<f32>) -> bool {
    for (var i = 0u; i < edits.count; i++) {
        let lo = vec3<f32>(edits.boxes[2u * i].xyz - 1);
        let hi = vec3<f32>(edits.boxes[2u * i + 1u].xyz + 2);
        if (all(p >= lo) && all(p <= hi)) {
            return true;
        }
    }
    return false;
}

@compute @workgroup_size(8, 8, 1)
fn temporal(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.light_size.x || gid.y >= params.light_size.y) {
        return;
    }
    let px = vec2<i32>(gid.xy);
    let rec = unpack_hit(textureLoad(hit_pos_in, px, 0), textureLoad(hit_info_in, px, 0));
    // The emissive irradiance in rgb, the lamp's (white) in a.
    let sample = textureLoad(irradiance_in, px, 0);
    let l = min(luminance(sample.xyz) + sample.w, MOMENT_LUM_MAX);
    var accum = sample;
    var moments = vec2<f32>(l, l * l);
    var age = 0.0;
    if (rec.found && (params.history & 1u) != 0u && !in_edit(rec.p)) {
        let vi = view_of(full_px(gid.xy));
        let full = reproject_centred(params.prev_views[vi], params.views[vi].jitter, rec.p);
        let q = select(prev_light_grid(full), full, full.x < 0.0) - 0.5;
        let base = vec2<i32>(floor(q));
        let f = q - floor(q);
        var w_sum = 0.0;
        var h = vec4<f32>(0.0);
        var m = vec3<f32>(0.0);
        let tol = HISTORY_DIST + HISTORY_DIST_PER_T * rec.t;
        for (var i = 0; i < 4; i++) {
            let o = vec2<i32>(i & 1, i >> 1);
            let tap = base + o;
            if (!on_light_grid(tap)) {
                continue;
            }
            let prev = unpack_hit(textureLoad(prev_pos_in, tap, 0), textureLoad(prev_info_in, tap, 0));
            if (!prev.found || prev.material != rec.material || dot(prev.n, rec.n) < HISTORY_COS
                || distance(prev.p, rec.p) > tol) {
                continue;
            }
            let wx = select(1.0 - f.x, f.x, o.x == 1);
            let wy = select(1.0 - f.y, f.y, o.y == 1);
            let w = wx * wy;
            w_sum += w;
            h += w * textureLoad(prev_accum_in, tap, 0);
            m += w * textureLoad(prev_moments_in, tap, 0).xyz;
        }
        if (w_sum <= 1e-3) {
            // No bilinear tap agrees: the best-agreeing pixel of the 3 x 3 around the nearest
            // (the nearest position), so a jittered hit beside a face's edge finds its face.
            let near = vec2<i32>(floor(q + 0.5));
            var best = tol;
            for (var i = 0; i < 9; i++) {
                let tap = near + vec2<i32>(i % 3 - 1, i / 3 - 1);
                if (!on_light_grid(tap)) {
                    continue;
                }
                let prev = unpack_hit(textureLoad(prev_pos_in, tap, 0), textureLoad(prev_info_in, tap, 0));
                let d = distance(prev.p, rec.p);
                if (!prev.found || prev.material != rec.material || dot(prev.n, rec.n) < HISTORY_COS
                    || d > best) {
                    continue;
                }
                best = d;
                w_sum = 1.0;
                h = textureLoad(prev_accum_in, tap, 0);
                m = textureLoad(prev_moments_in, tap, 0).xyz;
            }
        }
        if (w_sum > 1e-3) {
            let ratio = exposure_ratio();
            h = h / w_sum * ratio;
            m = vec3<f32>(m.xy / w_sum * vec2<f32>(ratio, ratio * ratio), m.z / w_sum);
            age = min(floor(m.z + 0.5) + 1.0, AGE_MAX);
            // The lamp's part keeps a short history; static emissive light a long one.
            var a = max(ALPHA, 1.0 / (age + 1.0));
            var a_emissive = max(1.0 / (age + 1.0), 1.0 / max(params.emissive_history, 1.0));
            if ((params.lighting & LIGHTING_EMISSION_CHANGED) != 0u) {
                // An emission scale changed within the last frames: emissive light follows
                // as fast as the lamp's.
                a_emissive = max(a_emissive, ALPHA);
            }
            if ((params.lighting & LIGHTING_GRADIENT) != 0u) {
                // Where the light changed, the new sample weighs as much as the change.
                let lambda = clamp(GRADIENT_GAIN * gradient_at(q + 0.5), 0.0, 1.0);
                a = max(a, lambda);
                a_emissive = max(a_emissive, lambda);
                // Rounded, not floored: lambda reads f16 and bfloat16 rounding (about 0.004)
                // wherever emitted light reaches, and a floor would take a whole frame of age
                // for it every frame, pinning the age, and so the blend, near the first frame.
                age = floor(age * (1.0 - lambda) + 0.5);
            }
            if ((params.lighting & LIGHTING_LAMP_CHANGED) != 0u) {
                a = 1.0;
            }
            accum = vec4<f32>(mix(h.xyz, sample.xyz, a_emissive), mix(h.w, sample.w, a));
            // The moments follow each part at its own pace, by the lamp's share of the
            // history's luminance.
            let lamp_share = h.w / max(luminance(h.xyz) + h.w, 1e-6);
            moments = mix(m.xy, vec2<f32>(l, l * l), mix(a_emissive, a, lamp_share));
        }
    }
    textureStore(accum_out, px, min(accum, vec4<f32>(HDR_MAX)));
    // The moments of a luminance at most MOMENT_LUM_MAX: the rescaled history can carry them
    // past it, and the second moment's bound, 65 025, is still a finite half.
    let m_max = vec2<f32>(MOMENT_LUM_MAX, MOMENT_LUM_MAX * MOMENT_LUM_MAX);
    textureStore(moments_out, px, vec4<f32>(min(moments, m_max), age, 0.0));
}
