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
//   2  storage texture   irradiance_out  rgba16float: rgb the irradiance E at the hit (light
//                                        per unit area arriving over the hemisphere of the
//                                        shading normal), a the luminance of its emissive
//                                        part; 0 on a miss. In the steps view the lamp shadow
//                                        ray's counters instead (see `shade`)
//
// The headlamp (Params.lighting bit 0) is a point light: E = PI * intensity * cone *
// max(n.l, 0) / dist^2, so that the composite's Lambertian `albedo * E / PI` is the former
// march's `albedo * lit`; one shadow ray per lit pixel, from SHADOW_OFFSET off the surface
// along the shadow normal toward the lamp.
//
// The emissive estimate is resampled importance sampling over `params.candidates`
// candidates. Each draws a sample uniformly from the `params.light_span` of the light-sample
// buffer (a hole, material 0, weighs nothing), one of its exposed faces that face the hit
// uniformly, and a uniform point x on that unit face: source density 1 / (span * faces) per
// unit area. Its target is the unshadowed contribution `lum(Le * scale) cos_hit cos_face /
// d^2`, and one candidate is kept with probability proportional to target over source
// density. One shadow ray to the kept point; unoccluded, the estimate of E is
// `Le / lum(Le) * (sum of weights) / candidates`, unbiased for the irradiance from every
// exposed emissive face. Random numbers come from `rand` with dimensions 2 + 5 j .. for
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

// The largest finite f16.
const HALF_MAX: f32 = 65504.0;
// Random dimensions per emissive candidate: sample, face, two for the point, the keep test.
const CANDIDATE_DIMS: u32 = 5u;

@compute @workgroup_size(8, 8, 1)
fn shade(@builtin(global_invocation_id) gid: vec3<u32>) {
    let view = view_mode();
    if (gid.x >= params.size.x || gid.y >= params.size.y || (view != VIEW_SHADED && view != VIEW_STEPS)) {
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
            let ray = pixel_ray(gid.xy);
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
    if (rec.found) {
        let ray = pixel_ray(gid.xy);
        let n = surface_detail(rec).xyz;
        if ((params.flags & FLAG_CUTAWAY) != 0u) {
            e = vec3<f32>(PI * (0.3 + 0.7 * (0.5 + 0.5 * n.z)));
        } else {
            let ns = shadow_normal_of(rec, ray.d);
            if ((params.lighting & LIGHTING_LAMP) != 0u) {
                e = vec3<f32>(PI * lamp_lit(rec.p, n, ns));
            }
            emissive = emissive_irradiance(gid.xy, rec.p, n, ns);
            e += emissive;
        }
    }
    textureStore(
        irradiance_out,
        px,
        vec4<f32>(min(e, vec3<f32>(HALF_MAX)), min(luminance(emissive), HALF_MAX)),
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
};

// Candidate `j` for the hit at `p` with shading normal `n`; weight 0 when it contributes
// nothing.
fn candidate(px: vec2<u32>, j: u32, p: vec3<f32>, n: vec3<f32>) -> Candidate {
    var none = Candidate(vec3<f32>(0.0), vec3<f32>(0.0), 0.0);
    let span = params.light_span;
    let dim = 2u + CANDIDATE_DIMS * j;
    let idx = min(u32(rand(px, dim) * f32(span)), span - 1u);
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
    return Candidate(x, le, target_pdf * f32(span) * f32(count));
}

// The irradiance at `p` (shading normal `n`, shadow normal `ns`) from one emissive face
// point chosen by resampling `params.candidates` candidates, with one shadow ray.
fn emissive_irradiance(px: vec2<u32>, p: vec3<f32>, n: vec3<f32>, ns: vec3<f32>) -> vec3<f32> {
    if (params.light_span == 0u || params.candidates == 0u) {
        return vec3<f32>(0.0);
    }
    var w_sum = 0.0;
    var kept = Candidate(vec3<f32>(0.0), vec3<f32>(0.0), 0.0);
    for (var j = 0u; j < params.candidates; j++) {
        let c = candidate(px, j, p, n);
        if (c.weight <= 0.0) {
            continue;
        }
        w_sum += c.weight;
        if (rand(px, 2u + CANDIDATE_DIMS * j + 4u) * w_sum < c.weight) {
            kept = c;
        }
    }
    if (kept.weight <= 0.0) {
        return vec3<f32>(0.0);
    }
    let o = p + ns * SHADOW_OFFSET;
    let to = kept.x - o;
    let dist = length(to);
    // Stop short of the face itself, which the ray reaches from its air side.
    if (shadowed(o, to / dist, dist - 1e-3)) {
        return vec3<f32>(0.0);
    }
    return kept.le / luminance(kept.le) * w_sum / f32(params.candidates);
}
