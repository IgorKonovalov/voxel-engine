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
//   7  storage texture   accum_out       rgba16float: the accumulated irradiance, the
//                                        emissive part in rgb, the lamp's (white) in a
//   8  storage texture   moments_out     rgba16float: first and second moments of the
//                                        filtered irradiance's luminance (the lamp's part and a
//                                        fallback hit's emissive part), then the age: frames of
//                                        history reused, 0 after a reset; then 1 where the
//                                        emissive part is the table's (the a-trous passes it
//                                        through), else 0
//   9  uniform           edits           the frame's edit boxes (see `Edits`)
//  10  texture_2d<f32>   strata_in       the gradient pass's strata (gradient.wgsl), read in
//                                        the gradient mode (Params.lighting bit 3): the
//                                        lamp's blend weight rises to twice the lambda averaged
//                                        over the 3 x 3 strata around the reprojected pixel's
//                                        (clamped to 1), and the age falls by that fraction
//  11  storage           cache_cells     the emissive table (march.wgsl), only read
//
// The emissive part (ADR-0043): a hit on a cube face whose cell holds a sample takes the
// cell's mean times this frame's exposure, with no screen history, the same value at every
// pixel of the face. Any other hit (no face: the field look, a translucent hit, a ray from
// inside rock; or a key with no cell, the table full) is a fallback: its own emissive samples'
// running mean over the pixel's age, `1 / (age + 1)`, until the age reaches FALLBACK_CAP, then
// held, so a still fallback pixel holds still; it restarts where the pixel resets (no
// accepted history, no history, an edit box), and follows an emission-scale change at ALPHA.
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
// sample in at `max(ALPHA, 1 / (age + 1))`, a fallback's emissive part as above. On the frame
// Params.lighting bit 9 says the lamp's light changed, the lamp's part takes this frame's
// sample as it is: the light pass's lamp estimate is exact up to its shadow ray, so a lamp
// switched off leaves nothing behind.
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
@group(2) @binding(7) var accum_out: texture_storage_2d<rgba16float, write>;
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
@group(2) @binding(11) var<storage, read_write> cache_cells: array<u32>;

// The mean of the key's cell in rgb, with a 1 in a; 0 for a key with no cell or no sample.
fn cache_mean(v: vec3<i32>, face: u32) -> vec4<f32> {
    let base = cache_slot(v, face);
    let key = cache_key(v, face);
    for (var i = 0u; i < CACHE_PROBES; i++) {
        let word = CACHE_WORDS * ((base + i) & params.cache_mask);
        if (cache_cells[word + 1u] == key.y && cache_cells[word] == key.x) {
            let count = cache_cells[word + CACHE_COUNT];
            if (count == 0u) {
                break;
            }
            if (cache_blend_on()) {
                return vec4<f32>(blend_unpack(cache_cells[word + CACHE_BLEND]), 1.0);
            }
            let n = f32(min(count, cache_c_cap())) * CACHE_Q;
            let sums = vec3<f32>(
                f32(cache_cells[word + CACHE_COUNT + 1u]),
                f32(cache_cells[word + CACHE_COUNT + 2u]),
                f32(cache_cells[word + CACHE_COUNT + 3u]),
            );
            return vec4<f32>(sums / n, 1.0);
        }
    }
    return vec4<f32>(0.0);
}

// The gradient's gain: lambda = |new - old| / max(new, old) reads 0.9 where light fell to a
// tenth, and a blend at 0.9 would keep a tenth of the old light; twice lambda, clamped, takes
// such a pixel to its new light at once.
const GRADIENT_GAIN: f32 = 2.0;
// Strata edge in pixels; gradient.wgsl's STRATUM.
const GRADIENT_STRATUM: f32 = 3.0;

// The gradient pass's lambdas at fractional pixel `q` of last frame, averaged over the 3 x 3
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
// The age at which a fallback's emissive mean stops taking samples: 64 samples, its noise an
// eighth of one sample's, then held until the pixel resets.
const FALLBACK_CAP: f32 = 64.0;
// A held fallback's history is its heaviest tap's alone where that tap's bilinear weight is at
// least this: a still camera reprojects each pixel onto its own centre up to rounding, and the
// blend of four taps would re-round the stored half every frame.
const HELD_TAP_W: f32 = 0.999;
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
    var sample = textureLoad(irradiance_in, px, 0);
    var cached = false;
    if (cache_read_on() && cacheable(rec)) {
        let ray = pixel_ray(full_px(gid.xy));
        let nf = shadow_normal_of(rec, ray.d);
        let c = cache_mean(cache_voxel(rec.p, nf), cache_face(rec.axis, nf));
        if (c.w > 0.0) {
            cached = true;
            sample = vec4<f32>(c.xyz * exposure_now(), sample.w);
        }
    }
    // The filtered part's luminance: the lamp's, and a fallback's emissive part.
    let l = min(select(luminance(sample.xyz), 0.0, cached) + sample.w, MOMENT_LUM_MAX);
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
        // The accepted tap of the largest weight and its history: a held fallback copies it
        // where it carries nearly all the weight, so a still pixel's value is not re-rounded.
        var best_w = 0.0;
        var best_h = vec3<f32>(0.0);
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
            let ht = textureLoad(prev_accum_in, tap, 0);
            h += w * ht;
            if (w > best_w) {
                best_w = w;
                best_h = ht.xyz;
            }
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
                best_w = 1.0;
                best_h = h.xyz;
                m = textureLoad(prev_moments_in, tap, 0).xyz;
            }
        }
        if (w_sum > 1e-3) {
            let ratio = exposure_ratio();
            h = h / w_sum * ratio;
            m = vec3<f32>(m.xy / w_sum * vec2<f32>(ratio, ratio * ratio), m.z / w_sum);
            age = min(floor(m.z + 0.5) + 1.0, AGE_MAX);
            // A fallback's emissive part: the running mean, held from FALLBACK_CAP samples.
            // Within an emission-scale change's frames (Params.lighting bit 8) it blends at no
            // less than ALPHA, so a held mean follows the new emission.
            var a_emissive = select(1.0 / (age + 1.0), 0.0, age >= FALLBACK_CAP);
            if ((params.lighting & LIGHTING_EMISSION_CHANGED) != 0u) {
                a_emissive = max(a_emissive, ALPHA);
            }
            var a = max(ALPHA, 1.0 / (age + 1.0));
            if ((params.lighting & LIGHTING_GRADIENT) != 0u) {
                // Where the lamp's light changed, the new sample weighs as much as the change.
                let lambda = clamp(GRADIENT_GAIN * gradient_at(q + 0.5), 0.0, 1.0);
                a = max(a, lambda);
                age = floor(age * (1.0 - lambda) + 0.5);
            }
            if ((params.lighting & LIGHTING_LAMP_CHANGED) != 0u) {
                a = 1.0;
            }
            var e_fallback = mix(h.xyz, sample.xyz, a_emissive);
            if (a_emissive == 0.0 && best_w >= HELD_TAP_W) {
                e_fallback = best_h * ratio;
            }
            let e_new = select(e_fallback, sample.xyz, cached);
            accum = vec4<f32>(e_new, mix(h.w, sample.w, a));
            moments = mix(m.xy, vec2<f32>(l, l * l), a);
        }
    }
    textureStore(accum_out, px, min(accum, vec4<f32>(HDR_MAX)));
    // The moments of a luminance at most MOMENT_LUM_MAX: the rescaled history can carry them
    // past it, and the second moment's bound, 65 025, is still a finite half.
    let m_max = vec2<f32>(MOMENT_LUM_MAX, MOMENT_LUM_MAX * MOMENT_LUM_MAX);
    textureStore(moments_out, px, vec4<f32>(min(moments, m_max), age, select(0.0, 1.0, cached)));
}
