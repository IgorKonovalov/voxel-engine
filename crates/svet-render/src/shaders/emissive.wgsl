// The emissive estimator: resampled importance sampling over the frame's candidate table,
// composed after light.wgsl (its table read, `face_dir` and `shadowed`) by the light pass and
// by the bake.
//
// The emissive estimate is resampled importance sampling over `params.candidates`
// candidates. Each draws a chunk from the candidate table with probability w / W (binary
// search over the cumulative weights) and a sample uniformly in its range (a hole, material
// 0, weighs nothing); then one of the sample's exposed faces that face the hit uniformly,
// and a uniform point x on that unit face: source density `(w / W) / len / faces` per unit
// area. Its target is the unshadowed contribution `lum(Le * scale) cos_hit cos_face /
// d^2`, and one candidate is kept with probability proportional to target over source
// density. One shadow ray to the kept point; unoccluded, the estimate of E is
// `Le / lum(Le) * (sum of weights) / candidates`, unbiased for the irradiance from every
// exposed emissive face but for a clamp of its luminance at `2 PI lum(Le)` (FIREFLY_CLAMP). Random numbers come from `rand` with dimensions 2 + 5 j .. for
// candidate j.

// Random dimensions per emissive candidate: chunk or sample, face, two for the point, the keep
// test, the sample within the chunk.
const CANDIDATE_DIMS: u32 = 6u;

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
    let head = table_slot(0u);
    let chunks = head.x;
    let total = bitcast<f32>(head.y);
    if (chunks == 0u || total <= 0.0) {
        return none;
    }
    // The first chunk whose cumulative weight exceeds u.
    let u = rand(px, dim) * total;
    var lo = 0u;
    var hi = chunks - 1u;
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
    // The sample and the inverse of its source probability.
    let idx = e.x + min(u32(rand(px, dim + 5u) * f32(e.y)), e.y - 1u);
    let inv_pdf = total / w * f32(e.y);
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

// The reservoir over `params.candidates` candidates for the hit at `p` with shading normal
// `n`, drawn with the random numbers of pixel `px`.
fn draw_candidates(px: vec2<u32>, p: vec3<f32>, n: vec3<f32>) -> Reservoir {
    var r = Reservoir(vec3<f32>(0.0), 0u, vec3<f32>(0.0), 0.0, 0.0, 0.0);
    for (var j = 0u; j < params.candidates; j++) {
        let c = candidate(px, j, p, n);
        merge(&r, c.x, c.face, c.le, c.target_pdf, c.weight, 1.0, rand(px, 2u + CANDIDATE_DIMS * j + 4u));
    }
    return r;
}

// The irradiance reservoir `r` estimates at the hit `p` (shadow normal `ns`): its kept
// emitter's, 0 when the shadow ray toward it finds rock or the reservoir is empty.
fn resolve_emissive(r: Reservoir, p: vec3<f32>, ns: vec3<f32>) -> vec3<f32> {
    var e = vec3<f32>(0.0);
    if (r.w_sum > 0.0 && r.target_pdf > 0.0 && r.m > 0.0) {
        let o = shadow_origin(p, ns);
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
    return e;
}

// One emissive estimate at hit `p` (shading normal `n`, shadow normal `ns`): the frame's
// candidates resampled, one shadow ray to the kept point, its random numbers those of `px`.
fn emissive_estimate(px: vec2<u32>, p: vec3<f32>, n: vec3<f32>, ns: vec3<f32>) -> vec3<f32> {
    return resolve_emissive(draw_candidates(px, p, n), p, ns);
}

// The emissive estimate's ceiling, in units of `PI lum(Le)` of the kept emitter.
const FIREFLY_CLAMP: f32 = 2.0;
