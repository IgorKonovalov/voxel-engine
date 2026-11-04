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
//   2  storage texture   irradiance_out  rgba16float: the irradiance E at the hit (light
//                                        per unit area arriving over the hemisphere of the
//                                        shading normal), this frame's emissive sample in rgb
//                                        (0 where the hit's cell is capped) and the lamp's
//                                        (white) in a, pre-exposed (`exposure_now`, clamped at
//                                        HDR_MAX); 0 on a miss. In the steps view the lamp
//                                        shadow ray's counters instead (see `shade`)
//   3  storage           cache_cells     the emissive table (march.wgsl): a hit on a cube face
//                                        finds or claims its cell, adds its emissive sample to
//                                        it and offers the sample's emitter as the cell's kept
//                                        emitter (`cache_offer`); a capped cell's pixel draws
//                                        nothing
//   4  storage           cache_stats     the table's counters (`CacheStats`)
//                                        and, past them, the frame's reopened-fill list: a
//                                        hit on a reopened cell under C_FILL lists it once a
//                                        frame (`fill_offer`), with the fill on
//   5  uniform           light_edits     the frame's edit boxes (temporal.wgsl's `Edits`): a
//                                        cell whose hit lies in one, dilated by one voxel, is
//                                        marked reopened, so the faces a dig exposes (cells
//                                        claimed new this frame) are filled as the cells the
//                                        edit-box reset cleared are
//
// The headlamp (Params.lighting bit 0) is a point light: E = PI * intensity * cone *
// max(n.l, 0) / dist^2, so that the composite's Lambertian `albedo * E / PI` is the former
// march's `albedo * lit`; one shadow ray per lit pixel, from SHADOW_OFFSET off the surface
// along the shadow normal toward the lamp.
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
//
// In the cutaway view a hit takes the hemisphere term `PI (0.3 + 0.7 (0.5 + 0.5 n.z))`
// instead, with no lamp, no emission and no shadow ray. Every shadow ray takes the frame's
// shadow walk (`shadowed`): the full walk, or the coarse walk that treats any 2^3 block with a
// set opaque bit as solid. Only the shaded and cracks views read this pass's irradiance; the
// steps view reads its shadow-ray counters; every other view skips the pass.

@group(0) @binding(10) var<storage, read> light_samples: array<vec4<i32>>;
@group(2) @binding(0) var hit_pos_in: texture_2d<f32>;
@group(2) @binding(1) var hit_info_in: texture_2d<u32>;
@group(2) @binding(2) var irradiance_out: texture_storage_2d<rgba16float, write>;
@group(2) @binding(3) var<storage, read_write> cache_cells: array<atomic<u32>>;
@group(2) @binding(4) var<storage, read_write> cache_stats: CacheStats;

// Up to MAX_EDITS boxes, inclusive voxel bounds (temporal.wgsl's `Edits`).
struct LightEdits {
    count: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    boxes: array<vec4<i32>, 128>,
};

@group(2) @binding(5) var<uniform> light_edits: LightEdits;

// Whether `p` lies in one of the frame's edit boxes dilated by one voxel (temporal.wgsl's
// `in_edit`).
fn in_light_edit(p: vec3<f32>) -> bool {
    for (var i = 0u; i < light_edits.count; i++) {
        let lo = vec3<f32>(light_edits.boxes[2u * i].xyz - 1);
        let hi = vec3<f32>(light_edits.boxes[2u * i + 1u].xyz + 2);
        if (all(p >= lo) && all(p <= hi)) {
            return true;
        }
    }
    return false;
}

// Lists `slot` for this frame's fill where it is reopened, under C_FILL and not yet listed this
// frame (the stamp's compare-exchange: one pixel of the face lists it).
fn fill_offer(slot: u32) {
    let word = CACHE_WORDS * slot;
    let stamp = atomicLoad(&cache_cells[word + CACHE_STAMP]);
    let now = CACHE_REOPENED | (params.frame & 0x7FFFFFFFu);
    if ((stamp & CACHE_REOPENED) == 0u || stamp == now) {
        return;
    }
    if (atomicLoad(&cache_cells[word + CACHE_COUNT]) >= fill_until()) {
        return;
    }
    if (atomicCompareExchangeWeak(&cache_cells[word + CACHE_STAMP], stamp, now).exchanged) {
        let i = atomicAdd(&cache_stats.fill_offered, 1u);
        if (i < FILL_LIST_MAX) {
            cache_stats.fill_cells[i] = slot;
        }
    }
}

// The frame's candidate table (ADR-0025), in light_samples from `params.light_table_base`:
// a header slot (count, total weight as f32 bits), then per entry the start and length of an
// emitter chunk's range and the cumulative weight up to and including it (f32 bits), in
// chunk-coordinate order, for the chunks within the light radius of the camera.
fn table_slot(i: u32) -> vec4<u32> {
    return bitcast<vec4<u32>>(light_samples[params.light_table_base + i]);
}

// Random dimensions per emissive candidate: chunk or sample, face, two for the point, the keep
// test, the sample within the chunk.
const CANDIDATE_DIMS: u32 = 6u;

@compute @workgroup_size(8, 8, 1)
fn shade(@builtin(global_invocation_id) gid: vec3<u32>) {
    let view = view_mode();
    if (gid.x >= params.light_size.x || gid.y >= params.light_size.y || (view != VIEW_SHADED && view != VIEW_STEPS && view != VIEW_CRACKS)) {
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
            shadowed(shadow_origin(rec.p, shadow_normal_of(rec, ray.d)), to_light / dist, dist);
            counts = vec4<f32>(f32(walk_steps), f32(counted_loads()), f32(counted_pool_reads()), 1.0);
        }
        textureStore(irradiance_out, px, counts);
        return;
    }
    var e = vec3<f32>(0.0);
    var emissive = vec3<f32>(0.0);
    if (rec.found) {
        let ray = pixel_ray(full_px(gid.xy));
        let n = rec.n;
        if ((params.flags & FLAG_CUTAWAY) != 0u) {
            e = vec3<f32>(PI * (0.3 + 0.7 * (0.5 + 0.5 * n.z)));
        } else {
            let ns = shadow_normal_of(rec, ray.d);
            if ((params.lighting & LIGHTING_LAMP) != 0u) {
                e = vec3<f32>(PI * lamp_lit(rec.p, n, ns));
            }
            var cell = -1;
            var capped = false;
            if (cacheable(rec)) {
                cell = cache_claim(cache_voxel(rec.p, ns), cache_face(rec.axis, ns));
                if (cell < 0) {
                    atomicAdd(&cache_stats.misses, 1u);
                } else {
                    capped = atomicLoad(&cache_cells[CACHE_WORDS * u32(cell) + CACHE_COUNT]) >= cache_c_cap();
                    if (fill_samples() > 0u) {
                        if (light_edits.count > 0u && in_light_edit(rec.p)) {
                            atomicOr(&cache_cells[CACHE_WORDS * u32(cell) + CACHE_STAMP], CACHE_REOPENED);
                        }
                        fill_offer(u32(cell));
                    }
                }
            }
            // A capped cell's pixel casts no emissive shadow ray: the temporal pass reads the
            // cell's mean.
            if (!capped) {
                // A cell's sample is drawn at a uniform point of its face, with the face's
                // normal, as the fill's are (`face_point`): the cell holds the face's mean
                // whatever points of it the pixels hit.
                var sp = rec.p;
                var sn = n;
                var sns = ns;
                if (cell >= 0) {
                    let f = cache_face(rec.axis, ns);
                    sp = face_point(cache_voxel(rec.p, ns), f, gid.xy);
                    sn = face_dir(f);
                    sns = sn;
                }
                let r = draw_candidates(gid.xy, sp, sn);
                emissive = resolve_emissive(r, sp, sns);
                if (cell >= 0) {
                    cache_add(u32(cell), emissive);
                    if (luminance(emissive) > 0.0) {
                        cache_offer(u32(cell), cache_voxel(rec.p, ns), cache_face(rec.axis, ns), r);
                    }
                }
            }
        }
    }
    // The emissive sample in rgb, the lamp's (white, or the cutaway's hemisphere) in a. Stored
    // pre-exposed: times this frame's exposure, at most HDR_MAX (ADR-0033).
    let ex = exposure_now();
    textureStore(
        irradiance_out,
        px,
        min(vec4<f32>(emissive * ex, e.x * ex), vec4<f32>(HDR_MAX)),
    );
}

// The slot holding the key `(v, face)`, claiming a free one of its CACHE_PROBES slots; -1 when
// they all hold other keys (a miss). Two scans keep one cell a key: the first looks for the key
// in every probe (a sweep may have freed a slot before it), the second claims the first free
// slot. A claim exchanges word 1 from 0 to the key's busy value (`cache_busy`, never a valid
// word 1), stores word 0, then publishes word 1; a pixel that meets its own key's busy value
// takes that slot (its count and sums are the key's already) instead of probing on, and every
// reader compares word 1 with a valid key, so no pixel sees a half-written key. Nothing frees
// a slot during the light pass, so every pixel of a key meets the same first free slot.
fn cache_claim(v: vec3<i32>, face: u32) -> i32 {
    let base = cache_slot(v, face);
    let key = cache_key(v, face);
    let busy = cache_busy(key);
    for (var i = 0u; i < CACHE_PROBES; i++) {
        let word = CACHE_WORDS * ((base + i) & params.cache_mask);
        let w1 = atomicLoad(&cache_cells[word + 1u]);
        if (w1 == busy) {
            return i32((base + i) & params.cache_mask);
        }
        if (w1 == key.y && cache_word0_is(word, key.x)) {
            return i32((base + i) & params.cache_mask);
        }
    }
    for (var i = 0u; i < CACHE_PROBES; i++) {
        let slot = (base + i) & params.cache_mask;
        let word = CACHE_WORDS * slot;
        // The weak exchange may fail spuriously on a free slot: try again.
        loop {
            let r = atomicCompareExchangeWeak(&cache_cells[word + 1u], 0u, busy);
            if (r.exchanged) {
                atomicStore(&cache_cells[word], key.x);
                atomicStore(&cache_cells[word + 1u], key.y);
                atomicAdd(&cache_stats.occupied, 1u);
                return i32(slot);
            }
            if (r.old_value == busy) {
                return i32(slot);
            }
            if (r.old_value == key.y && cache_word0_is(word, key.x)) {
                return i32(slot);
            }
            if (r.old_value != 0u) {
                break;
            }
        }
    }
    return -1;
}

// A claim's busy word 1 for `key`: a hash of both words with the valid bit clear and bit 0 set,
// so never a valid word 1 nor a free slot's 0; two keys share one with a chance of 2^-30.
fn cache_busy(key: vec2<u32>) -> u32 {
    return (hash_u32(key.x ^ hash_u32(key.y)) & 0x7FFFFFFEu) | 1u;
}

// Whether word 0 of the cell at `word` reads `x`, re-read a few times while it reads 0 and `x`
// is not: a claim's store of word 0 that this invocation does not see yet beside its word 1
// (storage atomics are relaxed in WGSL).
fn cache_word0_is(word: u32, x: u32) -> bool {
    for (var t = 0u; t < 8u; t++) {
        let w0 = atomicLoad(&cache_cells[word]);
        if (w0 == x) {
            return true;
        }
        if (w0 != 0u) {
            return false;
        }
    }
    return false;
}

// Adds an emissive sample to `slot`'s sums while its count is under C_CAP.
fn cache_add(slot: u32, emissive: vec3<f32>) {
    let word = CACHE_WORDS * slot + CACHE_COUNT;
    let before = atomicAdd(&cache_cells[word], 1u);
    if (before < cache_c_cap()) {
        let q = vec3<u32>(emissive * CACHE_Q + 0.5);
        atomicAdd(&cache_cells[word + 1u], q.x);
        atomicAdd(&cache_cells[word + 2u], q.y);
        atomicAdd(&cache_cells[word + 3u], q.z);
    }
}

// The cell's words of its kept emitter (ADR-0043's guard): the emitter face's key
// (`cache_key`, two words; word 1 is 0 while none is kept), then the packed luminance
// (`kept_pack`).
const CACHE_KEPT: u32 = 6u;
const CACHE_KEPT_LUM: u32 = 8u;

// The low 16 bits of a kept emitter's packed word: a hash of its key's two words.
fn kept_tag(k: vec2<u32>) -> u32 {
    return hash_u32(k.x ^ hash_u32(k.y)) & 0xFFFFu;
}

// A kept emitter's packed word: the luminance's f32 bits truncated to their high 16 (sign,
// exponent, seven mantissa bits, so a u32 `atomicMax` orders non-negative luminances), the
// key's tag in the low 16. A store of the key's two words that raced another winner's leaves
// a key whose tag differs from the packed word's, which the guard detects (`guard_cell`).
fn kept_pack(lum: f32, k: vec2<u32>) -> u32 {
    return (bitcast<u32>(max(lum, 0.0)) & 0xFFFF0000u) | kept_tag(k);
}

fn kept_lum(w: u32) -> f32 {
    return bitcast<f32>(w & 0xFFFF0000u);
}

// A uniform point of face `face` of voxel `v`, from `rand` at pixel `px` (dimensions 0 and 1;
// the candidates take 2 on).
fn face_point(v: vec3<i32>, face: u32, px: vec2<u32>) -> vec3<f32> {
    let ax = face / 2u;
    let t0 = select(vec3<f32>(0.0), vec3<f32>(1.0), axis_mask(i32((ax + 1u) % 3u)));
    let t1 = select(vec3<f32>(0.0), vec3<f32>(1.0), axis_mask(i32((ax + 2u) % 3u)));
    return face_centre(v, face) + t0 * (rand(px, 0u) - 0.5) + t1 * (rand(px, 1u) - 0.5);
}

// The unit face `face` of voxel `v`'s centre.
fn face_centre(v: vec3<i32>, face: u32) -> vec3<f32> {
    return vec3<f32>(v) + vec3<f32>(0.5) + 0.5 * face_dir(face);
}

// The luminance of the irradiance at the centre of face `face` of voxel `v` from the centre of
// the emitter face `ef` of voxel `ev`, with the emitter voxel's material's emission now:
// `lum(Le) cos cos / d^2`, 0 when the emitter voxel emits nothing, either face looks away
// from the other, or the shadow ray between the centres finds rock; also 0, with no shadow
// ray cast, when the unshadowed value is at most `above`. The light pass keeps it and the
// gradient pass re-shades it with the same arithmetic, so nothing changed reads the same
// value.
fn kept_irradiance(v: vec3<i32>, face: u32, ev: vec3<i32>, ef: u32, above: f32) -> f32 {
    let le = emission_of(lookup(ev).material);
    let l = luminance(le);
    if (l <= 0.0) {
        return 0.0;
    }
    let n = face_dir(face);
    let c = face_centre(v, face);
    let x = face_centre(ev, ef);
    let to = x - c;
    let d2 = max(dot(to, to), 1e-4);
    let dir = to * inverseSqrt(d2);
    let g = max(dot(n, dir), 0.0) * max(dot(face_dir(ef), -dir), 0.0);
    let e = l * g / d2;
    if (g <= 0.0 || e <= above) {
        return 0.0;
    }
    let o = shadow_origin(c, n);
    let ray = x - o;
    let dist = length(ray);
    if (shadowed(o, ray / dist, dist - 1e-3)) {
        return 0.0;
    }
    return e;
}

// Offers the emitter of a lit sample `r` at the cell `slot` of face `face` of voxel `v` as its
// kept emitter: kept when its centre-to-centre irradiance (`kept_irradiance`) is above the
// cell's kept luminance (`atomicMax` on the packed word, then the key's two words stored; a
// lost race keeps the other winner).
fn cache_offer(slot: u32, v: vec3<i32>, face: u32, r: Reservoir) {
    let word = CACHE_WORDS * slot;
    let ef = r.face;
    let ev = vec3<i32>(floor(r.x - 0.5 * face_dir(ef)));
    let cur = kept_lum(atomicLoad(&cache_cells[word + CACHE_KEPT_LUM]));
    let e = kept_irradiance(v, face, ev, ef, cur);
    let k = cache_key(ev, ef);
    let packed = kept_pack(e, k);
    if (kept_lum(packed) <= cur) {
        return;
    }
    let old = atomicMax(&cache_cells[word + CACHE_KEPT_LUM], packed);
    if (old < packed) {
        atomicStore(&cache_cells[word + CACHE_KEPT], k.x);
        atomicStore(&cache_cells[word + CACHE_KEPT + 1u], k.y);
    }
}

// The slot holding the key `(v, face)`, without claiming one; -1 when none does.
fn cache_find(v: vec3<i32>, face: u32) -> i32 {
    let base = cache_slot(v, face);
    let key = cache_key(v, face);
    for (var i = 0u; i < CACHE_PROBES; i++) {
        let slot = (base + i) & params.cache_mask;
        let word = CACHE_WORDS * slot;
        if (atomicLoad(&cache_cells[word + 1u]) == key.y && atomicLoad(&cache_cells[word]) == key.x) {
            return i32(slot);
        }
    }
    return -1;
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
        if (shadowed(shadow_origin(p, ns), l, sqrt(dist2))) {
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
