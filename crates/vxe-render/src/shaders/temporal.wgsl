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
//                                        emissive part is the store's (the a-trous passes it
//                                        through), else 0
//   9  uniform           edits           the frame's edit boxes (see `Edits`)
//  10  texture_2d<f32>   strata_in       the gradient pass's strata (gradient.wgsl), read in
//                                        the gradient mode (Params.lighting bit 3): the
//                                        lamp's blend weight rises to twice the lambda averaged
//                                        over the 3 x 3 strata around the reprojected pixel's
//                                        (clamped to 1), and the age falls by that fraction
//  11  storage           store           the store (ADR-0046, `bake::Store`); word 0 counts
//                                        this frame's cube-face hits with no cell nor brick record
//                                        (`store_missing`), word 1 those that read a brick
//                                        record, word 2
//                                        those whose voxel's chunk holds a coarse block
//                                        (TABLE_COARSE)
//
// A cube face's emissive part (`cacheable`) is its store cell (`store_value`) or, with none, its
// brick record's value for its direction (`store_brick_value`), times the dominant material's
// emission scale now and this frame's exposure, with no screen history, the same value at every
// pixel of the face; 0 with neither.
// Any other hit (no face: the field look, a translucent hit, a ray from inside rock) is a
// fallback: its own emissive samples' running mean over the pixel's age, `1 / (age + 1)`, until
// the age reaches FALLBACK_CAP, then held, so a still fallback pixel holds still; it restarts
// where the pixel resets (no accepted history, no history, an edit box), and follows an
// emission-scale change at ALPHA.
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
@group(2) @binding(11) var<storage, read_write> store: array<atomic<u32>>;

// The store's words (`bake::Store`): the counters, the window's origin and size in chunks, the
// flags, then a word per window chunk, its block's first word or 0. A block is 512 directory words
// (a brick's run offset from the block's start << 11 | its faces; 0 none), then the runs: two
// words a cell, its rgb9e5 irradiance at unit emission, then key (voxel 9 << 3 | face 3) | flags
// 4 << 12 | dominant material << 16, sorted by key.
const STORE_ORIGIN: u32 = 4u;
const STORE_SIZE: u32 = 8u;
// The flags word (`bake::STORE_FLAGS`): bit 0, every block carries a brick record before each
// surface brick's run, BRICK_WORDS a cell (six rgb9e5 direction values in face order, then
// `material | direction mask << 16`); bit 1, the cells are 4 voxels (eight a brick), else 8.
const STORE_FLAGS: u32 = 12u;
const STORE_TABLE: u32 = 16u;
const BRICK_WORDS: u32 = 7u;
// A chunk table word's flag (`bake::TABLE_COARSE`): the block is the live queue's coarse pass. The
// rest of the word is the block's first word.
const TABLE_COARSE: u32 = 0x80000000u;

// The value of an rgb9e5 word: three 9-bit mantissas over a shared exponent, biased by 15 + 9.
fn rgb9e5_decode(w: u32) -> vec3<f32> {
    let m = vec3<u32>(w & 511u, (w >> 9u) & 511u, (w >> 18u) & 511u);
    return vec3<f32>(m) * ldexp(1.0, i32(w >> 27u) - 24);
}

// Face `face` of voxel `v`'s cell in the store: its irradiance times its dominant material's
// emission scale now in rgb, with a 1 in a; 0 for a face with no cell.
fn store_value(v: vec3<i32>, face: u32) -> vec4<f32> {
    let origin = vec3<i32>(
        i32(atomicLoad(&store[STORE_ORIGIN])),
        i32(atomicLoad(&store[STORE_ORIGIN + 1u])),
        i32(atomicLoad(&store[STORE_ORIGIN + 2u])),
    );
    let size = vec3<i32>(
        i32(atomicLoad(&store[STORE_SIZE])),
        i32(atomicLoad(&store[STORE_SIZE + 1u])),
        i32(atomicLoad(&store[STORE_SIZE + 2u])),
    );
    let c = (v >> vec3<u32>(6u)) - origin;
    if (any(c < vec3<i32>(0)) || any(c >= size)) {
        return vec4<f32>(0.0);
    }
    let block = atomicLoad(&store[STORE_TABLE + u32(c.x + size.x * (c.y + size.y * c.z))]) & ~TABLE_COARSE;
    if (block == 0u) {
        return vec4<f32>(0.0);
    }
    let local = v & vec3<i32>(63);
    let brick = local >> vec3<u32>(3u);
    let dir = atomicLoad(&store[block + u32(brick.x + 8 * (brick.y + 8 * brick.z))]);
    let inner = local & vec3<i32>(7);
    let key = (u32(inner.x + 8 * (inner.y + 8 * inner.z)) << 3u) | face;
    let run = block + (dir >> 11u);
    var lo = 0u;
    var hi = dir & 0x7FFu;
    while (lo < hi) {
        let mid = (lo + hi) / 2u;
        let tail = atomicLoad(&store[run + 2u * mid + 1u]);
        let k = tail & 0xFFFu;
        if (k == key) {
            let word = atomicLoad(&store[run + 2u * mid]);
            return vec4<f32>(rgb9e5_decode(word) * emission_scale[tail >> 16u], 1.0);
        }
        if (k < key) {
            lo = mid + 1u;
        } else {
            hi = mid;
        }
    }
    return vec4<f32>(0.0);
}

// The block of chunk `c` (world chunks), 0 for none or outside the window.
fn store_block(c: vec3<i32>) -> u32 {
    return store_table(c) & ~TABLE_COARSE;
}

// Chunk `c`'s table word (world chunks): its block's first word with the TABLE_COARSE flag, 0 for
// none or outside the window.
fn store_table(c: vec3<i32>) -> u32 {
    let origin = vec3<i32>(
        i32(atomicLoad(&store[STORE_ORIGIN])),
        i32(atomicLoad(&store[STORE_ORIGIN + 1u])),
        i32(atomicLoad(&store[STORE_ORIGIN + 2u])),
    );
    let size = vec3<i32>(
        i32(atomicLoad(&store[STORE_SIZE])),
        i32(atomicLoad(&store[STORE_SIZE + 1u])),
        i32(atomicLoad(&store[STORE_SIZE + 2u])),
    );
    let w = c - origin;
    if (any(w < vec3<i32>(0)) || any(w >= size)) {
        return 0u;
    }
    return atomicLoad(&store[STORE_TABLE + u32(w.x + size.x * (w.y + size.y * w.z))]);
}

// The brick level's cell in voxels as a shift: 2 (4 voxels, flags bit 1) or 3 (the brick).
fn store_cell_shift() -> u32 {
    return select(3u, 2u, (atomicLoad(&store[STORE_FLAGS]) & 2u) != 0u);
}

// Cell `c`'s (world cells of 1 << `shift` voxels) value for direction `face` times its material's
// emission scale now in rgb, with a 1 in a; 0 where its brick has no record or the cell no face of
// that direction. A brick's record is its cells' BRICK_WORDS records, `x + n (y + n z)` with n
// cells a side, just before its run.
fn store_cell(c: vec3<i32>, face: u32, shift: u32) -> vec4<f32> {
    let per = 3u - shift;
    let b = c >> vec3<u32>(per);
    let block = store_block(b >> vec3<u32>(3u));
    if (block == 0u) {
        return vec4<f32>(0.0);
    }
    let s = b & vec3<i32>(7);
    let dir = atomicLoad(&store[block + u32(s.x + 8 * (s.y + 8 * s.z))]);
    if (dir == 0u) {
        return vec4<f32>(0.0);
    }
    let n = 1 << per;
    let l = c & vec3<i32>(n - 1);
    let sub = u32(l.x + n * (l.y + n * l.z));
    let cells = u32(n * n * n);
    let rec = block + (dir >> 11u) - BRICK_WORDS * cells + BRICK_WORDS * sub;
    let tail = atomicLoad(&store[rec + 6u]);
    if (((tail >> (16u + face)) & 1u) == 0u) {
        return vec4<f32>(0.0);
    }
    let x = rgb9e5_decode(atomicLoad(&store[rec + face]));
    return vec4<f32>(x * emission_scale[tail & 0xFFFFu], 1.0);
}

// Face `face` of voxel `v` at the brick level: the direction values of the 2 x 2 cells around the
// voxel's centre in the face's plane (its own cell's layer along the normal), bilinear by the
// centre's place between the cells' centres, over those that have the direction; its own cell's
// alone where none has it with weight. 0 in a where that has none either. Cells are addressed in
// world cells, so the blend crosses a brick's border as it crosses a cell's.
fn store_brick_value(v: vec3<i32>, face: u32) -> vec4<f32> {
    let shift = store_cell_shift();
    let size = f32(1u << shift);
    let axis = face / 2u;
    let ta = (axis + 1u) % 3u;
    let tb = (axis + 2u) % 3u;
    let own = v >> vec3<u32>(shift);
    let g = (vec3<f32>(v) + 0.5) / size - 0.5;
    let lo = vec3<i32>(floor(g));
    let f = g - floor(g);
    // The two in-plane axes as masks: a store through a runtime vector index is not portable
    // (`shader_portability`), so the cell's coordinate is chosen per component.
    let axes = vec3<u32>(0u, 1u, 2u);
    let ma = axes == vec3<u32>(ta);
    let mb = axes == vec3<u32>(tb);
    var sum = vec3<f32>(0.0);
    var weight = 0.0;
    for (var i = 0; i < 4; i++) {
        let oa = i & 1;
        let ob = i >> 1u;
        let b = select(select(own, lo + vec3<i32>(oa), ma), lo + vec3<i32>(ob), mb);
        let wa = select(1.0 - f[ta], f[ta], oa == 1);
        let wb = select(1.0 - f[tb], f[tb], ob == 1);
        let r = store_cell(b, face, shift);
        sum += r.xyz * (wa * wb * r.w);
        weight += wa * wb * r.w;
    }
    if (weight > 0.0) {
        return vec4<f32>(sum / weight, 1.0);
    }
    return store_cell(own, face, shift);
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
    if (cacheable(rec)) {
        let ray = pixel_ray(full_px(gid.xy));
        let nf = shadow_normal_of(rec, ray.d);
        let voxel = cache_voxel(rec.p, nf);
        let face = cache_face(rec.axis, nf);
        var c = store_value(voxel, face);
        if (c.w == 0.0 && store_bricks_on()) {
            c = store_brick_value(voxel, face);
            if (c.w > 0.0) {
                atomicAdd(&store[1], 1u);
            }
        }
        if (c.w == 0.0) {
            atomicAdd(&store[0], 1u);
        } else if ((store_table(voxel >> vec3<u32>(6u)) & TABLE_COARSE) != 0u) {
            atomicAdd(&store[2], 1u);
        }
        cached = true;
        sample = vec4<f32>(c.xyz * exposure_now(), sample.w);
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
