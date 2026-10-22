// The light pass: direct irradiance at each pixel's hit, read from the hit buffer, from the
// headlamp and from one emissive voxel face. Compiled after march.wgsl, whose bindings
// (group 0), walk, hit-record helpers and random numbers it uses; every shadow ray is
// `march`.
//
// Bindings, group 0 beyond march.wgsl's:
//  10  storage  light_samples  the mirror's light samples (see march.wgsl)
//
// Bindings, group 2:
//   0  texture_2d<f32>   hit_pos_in      the hit pass's rgba32float position + t
//   1  texture_2d<u32>   hit_info_in     the hit pass's rgba16uint normal, material, flags
//   3  texture_2d<f32>   prev_pos_in     last frame's hit positions
//   4  texture_2d<u32>   prev_info_in    last frame's hit records
//   5  texture_2d<u32>   prev_reservoir_in  last frame's reservoirs
//   6  storage texture   reservoir_out   rgba32uint: this frame's reservoir: the kept point x
//                                        (three f32), then W as bfloat16 (the top 16 bits
//                                        of its f32) in bits 0..16, the
//                                        effective sample count M bits 16..24, the face bits
//                                        24..27 and a valid bit 27
//   2  storage texture   irradiance_out  rgba16float: the irradiance E at the hit (light
//                                        per unit area arriving over the hemisphere of the
//                                        shading normal), its emissive part in rgb and the
//                                        lamp's (white) in a; 0 on a miss. In the steps view the lamp shadow
//                                        ray's counters instead (see `shade`)
//
// The headlamp (Params.lighting bit 0) is a point light: E = PI * intensity * cone *
// max(n.l, 0) / dist^2, so that the composite's Lambertian `albedo * E / PI` is the former
// march's `albedo * lit`; one shadow ray per lit pixel, from SHADOW_OFFSET off the surface
// along the shadow normal toward the lamp.
//
// The emissive estimate is resampled importance sampling over `params.candidates`
// candidates. Each draws a chunk from the candidate table with probability w / W (binary
// search over the cumulative weights) and a sample uniformly in its range, or with
// Params.lighting bit 7 (tests) a sample uniformly from the `params.light_span` of the buffer
// (a hole, material 0, weighs nothing); then one of the sample's exposed faces that face the
// hit uniformly, and a uniform point x on that unit face: source density
// `(w / W) / len / faces` (or `1 / (span * faces)`) per unit area. Its target is the unshadowed contribution `lum(Le * scale) cos_hit cos_face /
// d^2`, and one candidate is kept with probability proportional to target over source
// density. One shadow ray to the kept point; unoccluded, the estimate of E is
// `Le / lum(Le) * (sum of weights) / candidates`, unbiased for the irradiance from every
// exposed emissive face but for a clamp of its luminance at `2 PI lum(Le)` (FIREFLY_CLAMP). Random numbers come from `rand` with dimensions 2 + 5 j .. for
// candidate j.
//
// In the cutaway view a hit takes the hemisphere term `PI (0.3 + 0.7 (0.5 + 0.5 n.z))`
// instead, with no lamp, no emission and no shadow ray. Every shadow ray takes the frame's
// shadow walk (`shadowed`): the full walk, or the coarse walk that treats any 2^3 block with a
// set opaque bit as solid. Only the shaded view reads this pass's irradiance; the steps view
// reads its shadow-ray counters; every other view skips the pass.

@group(0) @binding(10) var<storage, read> light_samples: array<vec4<i32>>;
@group(2) @binding(0) var hit_pos_in: texture_2d<f32>;
@group(2) @binding(1) var hit_info_in: texture_2d<u32>;
@group(2) @binding(2) var irradiance_out: texture_storage_2d<rgba16float, write>;
@group(2) @binding(3) var prev_pos_in: texture_2d<f32>;
@group(2) @binding(4) var prev_info_in: texture_2d<u32>;
@group(2) @binding(5) var prev_reservoir_in: texture_2d<u32>;
@group(2) @binding(6) var reservoir_out: texture_storage_2d<rgba32uint, write>;

// The frame's candidate table (ADR-0025), in light_samples from `params.light_table_base`:
// a header slot (count, total weight as f32 bits), then per entry the start and length of an
// emitter chunk's range and the cumulative weight up to and including it (f32 bits), in
// chunk-coordinate order, for the chunks within the light radius of the camera.
fn table_slot(i: u32) -> vec4<u32> {
    return bitcast<vec4<u32>>(light_samples[params.light_table_base + i]);
}

// The largest finite f16.
const HALF_MAX: f32 = 65504.0;
// Random dimensions per emissive candidate: chunk or sample, face, two for the point, the keep
// test, the sample within the chunk.
const CANDIDATE_DIMS: u32 = 6u;

@compute @workgroup_size(8, 8, 1)
fn shade(@builtin(global_invocation_id) gid: vec3<u32>) {
    let view = view_mode();
    if (gid.x >= params.light_size.x || gid.y >= params.light_size.y || (view != VIEW_SHADED && view != VIEW_STEPS)) {
        return;
    }
    let px = vec2<i32>(gid.xy);
    let rec = unpack_hit(textureLoad(hit_pos_in, px, 0), textureLoad(hit_info_in, px, 0));
    if (view == VIEW_STEPS) {
        // The cost of each hit's shadow ray toward the lamp, lit or not, in the frame's
        // shadow walk: its loop iterations, storage loads and pool loads (unclamped), a = 1;
        // a = 0 where no ray is cast.
        var counts = vec4<f32>(0.0);
        if (rec.found && (params.flags & FLAG_CUTAWAY) == 0u) {
            let ray = pixel_ray(full_px(gid.xy));
            let to_light = params.light_pos - rec.p;
            let dist = max(length(to_light), 1e-2);
            walk_steps = 0u;
            loads = 0u;
            pool_reads = 0u;
            shadowed(rec.p + shadow_normal_of(rec, ray.d) * SHADOW_OFFSET, to_light / dist, dist);
            counts = vec4<f32>(f32(walk_steps), f32(loads), f32(pool_reads), 1.0);
        }
        textureStore(irradiance_out, px, counts);
        return;
    }
    var e = vec3<f32>(0.0);
    var emissive = vec3<f32>(0.0);
    // No reservoir unless `emissive_irradiance` writes one below.
    textureStore(reservoir_out, px, vec4<u32>(0u));
    if (rec.found) {
        let ray = pixel_ray(full_px(gid.xy));
        let n = lighting_normal(rec);
        if ((params.flags & FLAG_CUTAWAY) != 0u) {
            e = vec3<f32>(PI * (0.3 + 0.7 * (0.5 + 0.5 * n.z)));
        } else {
            let ns = shadow_normal_of(rec, ray.d);
            if ((params.lighting & LIGHTING_LAMP) != 0u) {
                e = vec3<f32>(PI * lamp_lit(rec.p, n, ns));
            }
            emissive = emissive_irradiance(gid.xy, rec, n, ns);
        }
    }
    // The emissive part in rgb, the lamp's (white, or the cutaway's hemisphere) in a: the
    // temporal pass keeps them on histories of different lengths.
    textureStore(
        irradiance_out,
        px,
        vec4<f32>(min(emissive, vec3<f32>(HALF_MAX)), min(e.x, HALF_MAX)),
    );
}

// The headlamp's `intensity * cone * max(n.l, 0) / dist^2` at `p` with shading normal `n`,
// zero when the shadow ray (offset along `ns`) finds rock before the lamp.
fn lamp_lit(p: vec3<f32>, n: vec3<f32>, ns: vec3<f32>) -> f32 {
    let to_light = params.light_pos - p;
    let dist2 = max(dot(to_light, to_light), 1e-4);
    let l = to_light * inverseSqrt(dist2);
    let ndl = max(dot(n, l), 0.0);
    let cone = smoothstep(params.cos_outer, params.cos_inner, dot(-l, params.light_dir));
    var lit = params.light_intensity * cone * ndl / dist2;
    if ((params.flags & FLAG_SHADOWS) != 0u && lit > 0.0) {
        if (shadowed(p + ns * SHADOW_OFFSET, l, sqrt(dist2))) {
            lit = 0.0;
        }
    }
    return lit;
}

// The walk a shadow ray leaves to the full walk at each end under the coarse mode, in voxels:
// more than a 2^3 block's diagonal (2 sqrt 3), so the blocks of the surface it leaves and of
// the face it aims at do not stop it.
const COARSE_MARGIN: f32 = 3.5;

// Whether rock lies on `o + d t` for `0 <= t <= len`: the full walk, or with
// `Params.lighting` bit 1 the coarse walk between COARSE_MARGIN of either end.
fn shadowed(o: vec3<f32>, d: vec3<f32>, len: f32) -> bool {
    if ((params.lighting & LIGHTING_COARSE_SHADOW) != 0u) {
        coarse_span = vec2<f32>(COARSE_MARGIN, len - COARSE_MARGIN);
    }
    let s = march(o, d, len, false);
    coarse_span = vec2<f32>(BIG, -BIG);
    return s.found;
}

// The unit normal of face `f` (2 k toward -k, 2 k + 1 toward +k).
fn face_dir(f: u32) -> vec3<f32> {
    let s = select(-1.0, 1.0, (f & 1u) == 1u);
    return select(vec3<f32>(0.0), vec3<f32>(s), axis_mask(i32(f / 2u)));
}

// One emissive candidate: its point, emitted radiance and resampling weight.
struct Candidate {
    x: vec3<f32>,
    le: vec3<f32>,
    weight: f32,
    face: u32,
    // The unshadowed contribution's luminance at the hit (the resampling target).
    target_pdf: f32,
};

// Candidate `j` for the hit at `p` with shading normal `n`; weight 0 when it contributes
// nothing.
fn candidate(px: vec2<u32>, j: u32, p: vec3<f32>, n: vec3<f32>) -> Candidate {
    var none = Candidate(vec3<f32>(0.0), vec3<f32>(0.0), 0.0, 0u, 0.0);
    let dim = 2u + CANDIDATE_DIMS * j;
    // The sample and the inverse of its source probability.
    var idx = 0u;
    var inv_pdf = 0.0;
    if ((params.lighting & LIGHTING_UNIFORM_DRAW) != 0u) {
        let span = params.light_span;
        if (span == 0u) {
            return none;
        }
        idx = min(u32(rand(px, dim) * f32(span)), span - 1u);
        inv_pdf = f32(span);
    } else {
        let head = table_slot(0u);
        let count = head.x;
        let total = bitcast<f32>(head.y);
        if (count == 0u || total <= 0.0) {
            return none;
        }
        // The first chunk whose cumulative weight exceeds u.
        let u = rand(px, dim) * total;
        var lo = 0u;
        var hi = count - 1u;
        while (lo < hi) {
            let mid = (lo + hi) / 2u;
            if (bitcast<f32>(table_slot(1u + mid).z) > u) {
                hi = mid;
            } else {
                lo = mid + 1u;
            }
        }
        let e = table_slot(1u + lo);
        var prev = 0.0;
        if (lo > 0u) {
            prev = bitcast<f32>(table_slot(lo).z);
        }
        let w = bitcast<f32>(e.z) - prev;
        if (w <= 0.0 || e.y == 0u) {
            return none;
        }
        idx = e.x + min(u32(rand(px, dim + 5u) * f32(e.y)), e.y - 1u);
        inv_pdf = total / w * f32(e.y);
    }
    let s = light_samples[idx];
    let m = u32(s.w) & 0xFFFFu;
    if (m == 0u) {
        return none;
    }
    let faces = (u32(s.w) >> 16u) & 63u;
    let v = vec3<f32>(s.xyz);
    // The exposed faces whose outward side holds `p`: at most one per axis.
    var cand = array<u32, 3>(0u, 0u, 0u);
    var count = 0u;
    for (var k = 0u; k < 3u; k++) {
        var f = 6u;
        if (p[k] >= v[k] + 1.0) {
            f = 2u * k + 1u;
        } else if (p[k] <= v[k]) {
            f = 2u * k;
        }
        if (f < 6u && ((faces >> f) & 1u) != 0u) {
            cand[count] = f;
            count += 1u;
        }
    }
    if (count == 0u) {
        return none;
    }
    let f = cand[min(u32(rand(px, dim + 1u) * f32(count)), count - 1u)];
    let k = i32(f / 2u);
    let a = rand(px, dim + 2u);
    let b = rand(px, dim + 3u);
    // The face's plane on axis k, a and b across the other two axes in order.
    let plane = v[k] + f32(f & 1u);
    var x = v + select(vec3<f32>(a, b, b), vec3<f32>(a, a, b), vec3<bool>(false, k == 0, k != 2));
    x = select(x, vec3<f32>(plane), axis_mask(k));
    let to = x - p;
    let d2 = max(dot(to, to), 1e-4);
    let dir = to * inverseSqrt(d2);
    let cos_hit = max(dot(n, dir), 0.0);
    let cos_face = max(dot(face_dir(f), -dir), 0.0);
    let le = emission_of(m);
    let target_pdf = luminance(le) * cos_hit * cos_face / d2;
    return Candidate(x, le, target_pdf * inv_pdf * f32(count), f, target_pdf);
}

// The resampling target of emitter point `x` on face `face` of radiance `le` at the hit `p`
// with shading normal `n`: the unshadowed contribution's luminance.
fn target_at(p: vec3<f32>, n: vec3<f32>, x: vec3<f32>, face: u32, le: vec3<f32>) -> f32 {
    let to = x - p;
    let d2 = max(dot(to, to), 1e-4);
    let dir = to * inverseSqrt(d2);
    let cos_hit = max(dot(n, dir), 0.0);
    let cos_face = max(dot(face_dir(face), -dir), 0.0);
    return luminance(le) * cos_hit * cos_face / d2;
}

// A weighted reservoir over emitter points: the kept point, its face and radiance, its
// target at this pixel, the weights' sum and the effective sample count M.
struct Reservoir {
    x: vec3<f32>,
    face: u32,
    le: vec3<f32>,
    target_pdf: f32,
    w_sum: f32,
    m: f32,
};

// Streams a point of weight `w` and `m` samples into `r`, keeping it with probability
// `w / w_sum` by the random number `u`.
fn merge(r: ptr<function, Reservoir>, x: vec3<f32>, face: u32, le: vec3<f32>, target_pdf: f32, w: f32, m: f32, u: f32) {
    (*r).w_sum += w;
    (*r).m += m;
    if (w > 0.0 && u * (*r).w_sum < w) {
        (*r).x = x;
        (*r).face = face;
        (*r).le = le;
        (*r).target_pdf = target_pdf;
    }
}

// Reservoirs reuse at most this many times the frame's candidates from the past.
const REUSE_M_CAP: f32 = 20.0;
// Spatial neighbours, their radius in pixels, and the random dimensions reuse starts at.
const SPATIAL_TAPS: u32 = 5u;
const SPATIAL_RADIUS: f32 = 30.0;
const REUSE_DIMS: u32 = 1000u;

// A stored reservoir of last frame's pixel `q`, re-targeted at this frame's hit `p` (normal
// `n`) and streamed into `r` with M capped; `false` when `q`'s reservoir is empty.
fn reuse(r: ptr<function, Reservoir>, q: vec2<i32>, p: vec3<f32>, n: vec3<f32>, u: f32) -> bool {
    let stored = textureLoad(prev_reservoir_in, q, 0);
    if ((stored.w & 0x8000000u) == 0u) {
        return false;
    }
    let x = bitcast<vec3<f32>>(stored.xyz);
    let w_big = bitcast<f32>((stored.w & 0xFFFFu) << 16u);
    let m = min(f32((stored.w >> 16u) & 0xFFu), REUSE_M_CAP * f32(params.candidates));
    let face = (stored.w >> 24u) & 7u;
    // The emitter voxel lies behind the point's face: its material gives the radiance now.
    let c = lookup(vec3<i32>(floor(x - 0.5 * face_dir(face))));
    var le = vec3<f32>(0.0);
    if (c.size == 0) {
        le = emission_of(c.material);
    }
    let t = target_at(p, n, x, face, le);
    merge(r, x, face, le, t, t * w_big * m, m, u);
    return true;
}

// Whether last frame's hit at `q` is this hit's surface: hit, the same material, a normal
// within 0.9 in cosine and within a voxel of its tangent plane.
fn same_surface(q: vec2<i32>, rec: HitRecord) -> bool {
    if (!on_light_grid(q)) {
        return false;
    }
    let prev = unpack_hit(textureLoad(prev_pos_in, q, 0), textureLoad(prev_info_in, q, 0));
    return prev.found && prev.material == rec.material && dot(prev.n, rec.n) > 0.9
        && abs(dot(rec.n, prev.p - rec.p)) < 1.0;
}

// The irradiance at hit `rec` (shading normal `n`, shadow normal `ns`) from one emissive face
// point chosen by resampling `params.candidates` candidates, and with Params.lighting bit 2
// (reservoir reuse, ReSTIR, Bitterli 2020) also last frame's reservoir at the reprojected
// pixel and SPATIAL_TAPS of its neighbours within SPATIAL_RADIUS, each on the same surface,
// re-targeted at this hit and capped at REUSE_M_CAP times the candidates; one shadow ray to
// the kept point. Writes this pixel's reservoir.
fn emissive_irradiance(px: vec2<u32>, rec: HitRecord, n: vec3<f32>, ns: vec3<f32>) -> vec3<f32> {
    let p = rec.p;
    var r = Reservoir(vec3<f32>(0.0), 0u, vec3<f32>(0.0), 0.0, 0.0, 0.0);
    for (var j = 0u; j < params.candidates; j++) {
        let c = candidate(px, j, p, n);
        merge(&r, c.x, c.face, c.le, c.target_pdf, c.weight, 1.0, rand(px, 2u + CANDIDATE_DIMS * j + 4u));
    }
    if ((params.lighting & LIGHTING_REUSE) != 0u && (params.history & 1u) != 0u) {
        let vi = view_of(full_px(px));
        let full = reproject_centred(params.prev_views[vi], params.views[vi].jitter, p);
        let q = select(prev_light_grid(full), full, full.x < 0.0);
        let centre = vec2<i32>(floor(q));
        if (q.x >= 0.0 && same_surface(centre, rec)) {
            reuse(&r, centre, p, n, rand(px, REUSE_DIMS));
        }
        for (var k = 0u; k < SPATIAL_TAPS; k++) {
            let a = rand(px, REUSE_DIMS + 1u + 3u * k) * 2.0 * PI;
            let d = SPATIAL_RADIUS * sqrt(rand(px, REUSE_DIMS + 2u + 3u * k));
            let tap = vec2<i32>(floor(q + d * vec2<f32>(cos(a), sin(a))));
            if (q.x >= 0.0 && same_surface(tap, rec)) {
                reuse(&r, tap, p, n, rand(px, REUSE_DIMS + 3u + 3u * k));
            }
        }
    }
    // The stored count is capped at REUSE_M_CAP times the candidates, the weights scaled
    // with it so W stays.
    let cap = REUSE_M_CAP * f32(params.candidates);
    if (r.m > cap) {
        r.w_sum *= cap / r.m;
        r.m = cap;
    }
    var stored = vec4<u32>(0u);
    var e = vec3<f32>(0.0);
    if (r.w_sum > 0.0 && r.target_pdf > 0.0 && r.m > 0.0) {
        // The estimate's luminance is `target * W = w_sum / M`.
        let w_big = r.w_sum / (r.m * r.target_pdf);
        // W as bfloat16, the top half of its f32 bits rounded to nearest: a truncation would
        // bias every reused W low, and temporal reuse compounds it frame after frame.
        let packed = ((bitcast<u32>(w_big) + 0x8000u) >> 16u)
            | (u32(min(r.m, 255.0)) << 16u) | (r.face << 24u) | 0x8000000u;
        stored = vec4<u32>(bitcast<vec3<u32>>(r.x), packed);
        let o = p + ns * SHADOW_OFFSET;
        let to = r.x - o;
        let dist = length(to);
        // Stop short of the face itself, which the ray reaches from its air side.
        if (!shadowed(o, to / dist, dist - 1e-3)) {
            // A candidate on a face next to the hit has a tiny d^2 and a huge weight, a
            // firefly the history would carry for many frames. No surface receives more than
            // `PI L` from emitters of radiance L filling its hemisphere, so the estimate is
            // clamped at FIREFLY_CLAMP times that bound for the kept emitter: biased dark only
            // where one sample overshoots the physical maximum.
            let l = min(r.w_sum / r.m, FIREFLY_CLAMP * PI * luminance(r.le));
            e = r.le / luminance(r.le) * l;
        }
    }
    textureStore(reservoir_out, vec2<i32>(px), stored);
    return e;
}

// The emissive estimate's ceiling, in units of `PI lum(Le)` of the kept emitter.
const FIREFLY_CLAMP: f32 = 2.0;
