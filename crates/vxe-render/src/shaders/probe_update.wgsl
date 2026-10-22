// The probe update (ADR-0023): the frame's update list of probes each traces `probe.rays` rays
// (at most RAYS, the workgroup's size) and
// blends what they saw into its irradiance and visibility maps. Compiled after march.wgsl,
// light.wgsl and probe_gather.wgsl, whose walk, direct estimator, probe bindings (group 3)
// and probe reads it uses.
//
// `probe_trace`, one workgroup per list entry and one invocation per ray: ray i leaves the
// probe along the i-th of `probe.rays` spherical Fibonacci directions turned by the frame's random
// rotation, through the full walk. A ray that starts inside rock is a back face: signed
// distance -1 and no radiance. A ray that hits a face returns its distance and the radiance
// the face reflects toward the probe, `albedo / PI (E_direct + E_probes)`: E_direct the light
// pass's estimate at the hit (the headlamp by next-event estimation, one emissive sample, each
// with its shadow ray in the frame's shadow walk, every emitter at its probe emission scale),
// E_probes the probes' irradiance there as
// they stood before this frame's blend (`probe_irradiance`). The hit's own emission is left out,
// since the direct estimate samples every emissive face. A miss returns PROBE_MISS_DISTANCE
// spacings and no radiance. Each ray's radiance and signed distance go to the scratch buffer.
//
// `probe_blend`, one workgroup per entry, after every trace: interior irradiance texel t takes
// `PI sum(L max(0, d_t . w)) / sum(max(0, d_t . w))` over the front-face rays, interior
// visibility texel t the `max(0, d_t . w)^VIS_SHARPNESS`-weighted mean (rays under VIS_CUTOFF
// in cosine left out) of the ray distance
// (clamped to [0, PROBE_MISS_DISTANCE spacings]) and of its square; each blends with the
// previous texel as `h old + (1 - h) new`. h is 0 for a slot holding another probe or one never
// traced (age 0), YOUNG_HYSTERESIS for the YOUNG_UPDATES updates after the first and for a
// probe an edit touched (whose age restarts at 1), HYSTERESIS after. While a light-change
// window is open (`probe.light_window`, one probe in 4 a frame for four visits each), every
// update blends at 0 and the visited probe's age restarts at 1, so a light switched off fades
// by about the room's albedo per visit rather than at HYSTERESIS, and the young updates after
// the window smooth the noise back down. The border texels then
// repeat the interior ones the octahedral fold puts beside them. The probe is inside rock when
// more than a quarter of its rays met a back face; the state word takes the inside flag, the
// age plus one (capped) and the probe's tag, and its texels are left as they were.

const RAYS: u32 = 64u;
// Words of the update list at the front of the scratch buffer (`probes::MAX_UPDATES`).
const UPDATE_LIST_WORDS: u32 = 8192u;
const TOUCHED_BIT: u32 = 0x80000000u;
const YOUNG_UPDATES: u32 = 8u;
const YOUNG_HYSTERESIS: f32 = 0.5;
const HYSTERESIS: f32 = 0.97;
const VIS_SHARPNESS: f32 = 50.0;
const VIS_CUTOFF: f32 = 0.83;
// A missing ray's distance, and the ceiling of every visibility distance, in spacings.
const PROBE_MISS_DISTANCE: f32 = 2.0;
// pi (3 - sqrt 5): the spherical Fibonacci set's turn per ray.
const GOLDEN_ANGLE: f32 = 2.39996322972865;
// The random dimensions of the frame's rotation.
const PROBE_ROTATION_DIMS: u32 = 3000u;

// World probe of slot `slot` in the volume at `probe.origin`.
fn slot_probe(slot: u32) -> vec3<i32> {
    let m = vec3<i32>(vec3<u32>(slot % 32u, (slot / 32u) % 32u, slot / 1024u));
    return probe.origin + (((m - probe.origin) % PROBE_AXIS) + PROBE_AXIS) % PROBE_AXIS;
}

// The frame's rotation as a unit quaternion (Shoemake's uniform random rotation).
fn probe_rotation() -> vec4<f32> {
    let u1 = rand(vec2<u32>(0u), PROBE_ROTATION_DIMS);
    let u2 = rand(vec2<u32>(0u), PROBE_ROTATION_DIMS + 1u);
    let u3 = rand(vec2<u32>(0u), PROBE_ROTATION_DIMS + 2u);
    let a = sqrt(1.0 - u1);
    let b = sqrt(u1);
    return vec4<f32>(a * sin(2.0 * PI * u2), a * cos(2.0 * PI * u2), b * sin(2.0 * PI * u3), b * cos(2.0 * PI * u3));
}

// Ray `i`'s direction this frame.
fn probe_ray_dir(i: u32) -> vec3<f32> {
    let z = 1.0 - (2.0 * f32(i) + 1.0) / f32(probe.rays);
    let r = sqrt(max(1.0 - z * z, 0.0));
    let phi = f32(i) * GOLDEN_ANGLE;
    let v = vec3<f32>(r * cos(phi), r * sin(phi), z);
    let q = probe_rotation();
    return normalize(v + 2.0 * cross(q.xyz, cross(q.xyz, v) + q.w * v));
}

fn ray_word(k: u32, lane: u32) -> u32 {
    return UPDATE_LIST_WORDS + 2u * (k * RAYS + lane);
}

@compute @workgroup_size(64, 1, 1)
fn probe_trace(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    let k = wg.x;
    if (k >= probe.updates || lane >= probe.rays) {
        return;
    }
    // The probe rays see each emitter at its probe scale (a pulsing one at its average).
    emission_base = EMISSION_ROWS;
    let slot = probe_scratch[k] & ~TOUCHED_BIT;
    let w = slot_probe(slot);
    let o = probe_position(w);
    let d = probe_ray_dir(lane);
    let h = march(o, d, BIG, false);
    var radiance = vec3<f32>(0.0);
    var dist = PROBE_MISS_DISTANCE * f32(probe.spacing);
    if (h.found) {
        if (h.axis < 0 && !h.plane) {
            dist = -1.0;
        } else {
            dist = h.t;
            let p = o + d * h.t;
            let n = shadow_normal(d, h);
            var e = vec3<f32>(0.0);
            if ((params.lighting & LIGHTING_LAMP) != 0u) {
                e += vec3<f32>(PI * lamp_lit(p, n, n));
            }
            let px = vec2<u32>(lane + RAYS * (slot & 1u), slot >> 1u);
            e += emissive_estimate(px, p, n, n);
            e += probe_irradiance(p, n);
            radiance = materials[h.material * 4u].xyz * (1.0 / PI) * e;
        }
    }
    let at = ray_word(k, lane);
    probe_scratch[at] = pack_half2(radiance.xy);
    probe_scratch[at + 1u] = pack_half2(vec2<f32>(radiance.z, dist));
}

var<workgroup> ray_radiance: array<vec3<f32>, 64>;
var<workgroup> ray_dirs: array<vec3<f32>, 64>;
var<workgroup> ray_dist: array<f32, 64>;
var<workgroup> new_irradiance: array<vec3<f32>, 36>;
var<workgroup> new_visibility: array<vec2<f32>, 196>;

// The direction of interior texel `t` (row-major) of a map of edge `n`.
fn texel_dir(t: u32, n: u32) -> vec3<f32> {
    let c = (vec2<f32>(f32(t % n), f32(t / n)) + 0.5) / f32(n);
    return probe_unoct(c * 2.0 - 1.0);
}

// The interior texel (row-major, 0-based) tile texel (x, y) of a map of interior edge `n`
// holds: itself inside, the octahedral fold's neighbour on the border.
fn border_source(x: u32, y: u32, n: u32) -> u32 {
    let e = n + 1u;
    var ix = x;
    var iy = y;
    if ((x == 0u || x == e) && (y == 0u || y == e)) {
        ix = select(n, 1u, x == e);
        iy = select(n, 1u, y == e);
    } else if (y == 0u || y == e) {
        ix = e - x;
        iy = select(1u, n, y == e);
    } else if (x == 0u || x == e) {
        ix = select(1u, n, x == e);
        iy = e - y;
    }
    return (ix - 1u) + n * (iy - 1u);
}

@compute @workgroup_size(64, 1, 1)
fn probe_blend(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    let k = wg.x;
    if (k >= probe.updates) {
        return;
    }
    let at = ray_word(k, lane);
    let rg = unpack_half2(probe_scratch[at]);
    let bd = unpack_half2(probe_scratch[at + 1u]);
    // Lanes past the rays in use hold nothing; every loop below stops at `probe.rays`.
    ray_radiance[lane] = vec3<f32>(rg, bd.x);
    ray_dist[lane] = bd.y;
    ray_dirs[lane] = probe_ray_dir(lane);
    workgroupBarrier();
    // Every lane counts the back faces itself: an inside probe skips the texels, which the
    // gather never reads.
    var backs = 0u;
    for (var r = 0u; r < probe.rays; r++) {
        backs += select(0u, 1u, ray_dist[r] < 0.0);
    }
    let inside = backs * 4u > probe.rays;
    let entry = probe_scratch[k];
    let slot = entry & ~TOUCHED_BIT;
    let w = slot_probe(slot);
    let state = probe_data[STATE_BASE + slot];
    let age = select(0u, (state >> 1u) & PROBE_MAX_AGE, (state >> 8u) == probe_tag(w));
    let touched = (entry & TOUCHED_BIT) != 0u;
    let window = probe.light_window != 0u;
    var h = HYSTERESIS;
    if (age == 0u || window) {
        h = 0.0;
    } else if (touched || age <= YOUNG_UPDATES) {
        h = YOUNG_HYSTERESIS;
    }
    let max_dist = PROBE_MISS_DISTANCE * f32(probe.spacing);
    if (!inside && lane < IRR_N * IRR_N) {
        let dir = texel_dir(lane, IRR_N);
        var sum = vec3<f32>(0.0);
        var w_sum = 0.0;
        for (var r = 0u; r < probe.rays; r++) {
            if (ray_dist[r] < 0.0) {
                continue;
            }
            let c = max(dot(dir, ray_dirs[r]), 0.0);
            sum += c * ray_radiance[r];
            w_sum += c;
        }
        let old = irradiance_texel(slot, 1u + lane % IRR_N, 1u + lane / IRR_N);
        var v = old;
        if (w_sum > 0.0) {
            v = h * old + (1.0 - h) * (PI * sum / w_sum);
        }
        new_irradiance[lane] = v;
    }
    for (var t = lane; !inside && t < VIS_N * VIS_N; t += 64u) {
        let dir = texel_dir(t, VIS_N);
        var sum = vec2<f32>(0.0);
        var w_sum = 0.0;
        for (var r = 0u; r < probe.rays; r++) {
            let cos_r = dot(dir, ray_dirs[r]);
            // VIS_CUTOFF^VIS_SHARPNESS is under 1e-4: the rays beyond it weigh nothing.
            if (cos_r <= VIS_CUTOFF) {
                continue;
            }
            let c = pow(cos_r, VIS_SHARPNESS);
            let dr = clamp(ray_dist[r], 0.0, max_dist);
            sum += c * vec2<f32>(dr, dr * dr);
            w_sum += c;
        }
        let old = visibility_texel(slot, 1u + t % VIS_N, 1u + t / VIS_N);
        var v = old;
        if (w_sum > 1e-6) {
            v = h * old + (1.0 - h) * (sum / w_sum);
        }
        new_visibility[t] = v;
    }
    workgroupBarrier();
    // The whole tiles, borders included: 64 irradiance texels, four visibility texels a lane.
    if (!inside) {
        let ie = IRR_N + 2u;
        let iv = new_irradiance[border_source(lane % ie, lane / ie, IRR_N)];
        let iw = 2u * (slot * IRR_EDGE * IRR_EDGE + lane);
        probe_data[iw] = pack_half2(iv.xy);
        probe_data[iw + 1u] = pack_half2(vec2<f32>(iv.z, 0.0));
        let ve = VIS_N + 2u;
        for (var t = lane; t < ve * ve; t += 64u) {
            let vv = new_visibility[border_source(t % ve, t / ve, VIS_N)];
            probe_data[VIS_BASE + slot * VIS_EDGE * VIS_EDGE + t] = pack_half2(vv);
        }
    }
    if (lane == 0u) {
        let next = select(min(age + 1u, PROBE_MAX_AGE), 1u, touched || window);
        probe_data[STATE_BASE + slot] = select(0u, 1u, inside) | (next << 1u) | (probe_tag(w) << 8u);
    }
}
