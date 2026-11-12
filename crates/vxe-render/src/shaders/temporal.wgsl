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
//  11  storage, read      store           the store (ADR-0046, `bake::Store`)
//  12  storage           counters        five words, written only where the pipeline's
//                                        COUNT_STORE is true: word 0 counts this frame's
//                                        cube-face hits with no brick record value
//                                        (`store_missing`), word 1 those that read one, word 2
//                                        those whose voxel's chunk holds a coarse block
//                                        (TABLE_COARSE); of word 0's, word 3 those the
//                                        missing-light chain gave a value, word 4 those it left
//                                        black (every one of them under `black`)
//
// A cube face's emissive part (`reads_store`) is its brick record's value for its direction
// (`store_brick_value`), times the dominant material's
// emission scale now and this frame's exposure, with no screen history, the same value at every
// pixel of the face. With no value, the flags word's missing-light mode (ADR-0053,
// `store_fallback`) reads a chain of summaries, nearest first: the chunk's own block's, the
// mean of its six face neighbours', the level-1 then level-2 parent record; 0 where none holds
// the direction, and 0 at once under `black`.
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
// A cube face's pixel with the lamp off (Params.lighting bit 0 clear, no cutaway, not the steps
// view) skips the reprojection: its emissive part is the store's and its lamp part, moments and
// their history are zero (the frame the lamp switched off took its sample, zero, at once), so it
// writes the store's value and zero moments. Its age is carried on the pixel: last frame's age at
// the same pixel plus one, 0 where the pixel would reset; a still camera maps each pixel onto
// itself, so there it is the reprojected age.
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
@group(2) @binding(11) var<storage, read> store: array<u32>;
@group(2) @binding(12) var<storage, read_write> counters: array<atomic<u32>, 5>;

// Whether this pipeline counts the store's reads into `counters` (the HUD's and
// `--read-missing`'s pipeline); off, `counters` is never written.
override COUNT_STORE: bool = false;

// The store's words and a record's layout: march.wgsl's STORE_ORIGIN to TABLE_COARSE.

// The value of an rgb9e5 word: three 9-bit mantissas over a shared exponent, biased by 15 + 9.
fn rgb9e5_decode(w: u32) -> vec3<f32> {
    let m = vec3<u32>(w & 511u, (w >> 9u) & 511u, (w >> 18u) & 511u);
    return vec3<f32>(m) * ldexp(1.0, i32(w >> 27u) - 24);
}

// The store's window in chunks and its brick level's cell as a shift, read once a pixel.
struct StoreHead {
    origin: vec3<i32>,
    size: vec3<i32>,
    // 2 (4 voxels, flags bit 1) or 3 (the brick).
    shift: u32,
};

fn store_head() -> StoreHead {
    let origin = vec3<i32>(
        i32(store[STORE_ORIGIN]),
        i32(store[STORE_ORIGIN + 1u]),
        i32(store[STORE_ORIGIN + 2u]),
    );
    let size = vec3<i32>(
        i32(store[STORE_SIZE]),
        i32(store[STORE_SIZE + 1u]),
        i32(store[STORE_SIZE + 2u]),
    );
    return StoreHead(origin, size, select(3u, 2u, (store[STORE_FLAGS] & 2u) != 0u));
}

// Chunk `c`'s table word (world chunks): its block's first word with the TABLE_COARSE flag, 0 for
// none or outside the window.
fn store_table(head: StoreHead, c: vec3<i32>) -> u32 {
    let w = c - head.origin;
    if (any(w < vec3<i32>(0)) || any(w >= head.size)) {
        return 0u;
    }
    return store[STORE_TABLE + u32(w.x + head.size.x * (w.y + head.size.y * w.z))];
}

// Cell `c`'s (world cells of 1 << `head.shift` voxels) value for direction `face` times its
// material's emission scale now in rgb, with a 1 in a; 0 where its brick has no record or the cell
// no face of that direction. A brick's record is its cells' BRICK_WORDS records, `x + n (y + n z)`
// with n cells a side, just before its run.
fn store_cell(head: StoreHead, c: vec3<i32>, face: u32) -> vec4<f32> {
    let per = 3u - head.shift;
    let b = c >> vec3<u32>(per);
    let block = store_table(head, b >> vec3<u32>(3u)) & ~TABLE_COARSE;
    if (block == 0u) {
        return vec4<f32>(0.0);
    }
    let s = b & vec3<i32>(7);
    let dir = store[block + u32(s.x + 8 * (s.y + 8 * s.z))];
    if (dir == 0u) {
        return vec4<f32>(0.0);
    }
    let n = 1 << per;
    let l = c & vec3<i32>(n - 1);
    let sub = u32(l.x + n * (l.y + n * l.z));
    let cells = u32(n * n * n);
    let rec = block + (dir >> 11u) - BRICK_WORDS * cells + BRICK_WORDS * sub;
    let tail = store[rec + 6u];
    if (((tail >> (16u + face)) & 1u) == 0u) {
        return vec4<f32>(0.0);
    }
    let x = rgb9e5_decode(store[rec + face]);
    return vec4<f32>(x * emission_scale[tail & 0xFFFFu], 1.0);
}

// Face `face` of voxel `v` at the brick level: the direction values of the 2 x 2 cells around the
// voxel's centre in the face's plane (march.wgsl's `store_tap`), bilinear over those that have
// the direction; its own cell's alone where none has it with weight. 0 in a where that has none
// either. march.wgsl's `glass_store_value` is the same read over the hit pass's binding.
fn store_brick_value(head: StoreHead, v: vec3<i32>, face: u32) -> vec4<f32> {
    var sum = vec3<f32>(0.0);
    var weight = 0.0;
    for (var i = 0; i < 4; i++) {
        let tap = store_tap(v, face, head.shift, i);
        let r = store_cell(head, tap.cell, face);
        sum += r.xyz * (tap.w * r.w);
        weight += tap.w * r.w;
    }
    if (weight > 0.0) {
        return vec4<f32>(sum / weight, 1.0);
    }
    return store_cell(head, v >> vec3<u32>(head.shift), face);
}

// A block's summary record (`bake::SUMMARY_AT`): just past its 512-word directory.
const STORE_SUMMARY_AT: u32 = 512u;
// The flags word's missing-light mode, bits 2 and 3 (`bake::MissingLight::bits`): 0 black.
const MISSING_NEIGHBOUR: u32 = 1u;
const MISSING_PARENT: u32 = 2u;

fn store_mode() -> u32 {
    return (store[STORE_FLAGS] >> 2u) & 3u;
}

// The record at word `rec`'s value for direction `face` times its material's emission scale now
// in rgb, with a 1 in a; 0 where its mask lacks the direction.
fn store_record(rec: u32, face: u32) -> vec4<f32> {
    let tail = store[rec + 6u];
    if (((tail >> (16u + face)) & 1u) == 0u) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(rgb9e5_decode(store[rec + face]) * emission_scale[tail & 0xFFFFu], 1.0);
}

// Chunk `c`'s summary's value for `face`; 0 where the chunk holds no block.
fn store_summary(head: StoreHead, c: vec3<i32>, face: u32) -> vec4<f32> {
    let block = store_table(head, c) & ~TABLE_COARSE;
    if (block == 0u) {
        return vec4<f32>(0.0);
    }
    return store_record(block + STORE_SUMMARY_AT, face);
}

// The level-`level` (1 or 2) parent record over chunk `c` for `face` (`bake::ParentTable`): after
// the chunk table, level 1 over cells of 2 chunks a side, `(size + 1) / 2 + 1` cells a side from
// the window origin's cell, then level 2 over 4, `(size + 3) / 4 + 1`; cells world-aligned.
fn store_parent(head: StoreHead, c: vec3<i32>, level: u32, face: u32) -> vec4<f32> {
    let first = STORE_TABLE + u32(head.size.x * head.size.y * head.size.z);
    let s1 = ((head.size + vec3<i32>(1)) >> vec3<u32>(1u)) + vec3<i32>(1);
    var base = first;
    var side = s1;
    if (level == 2u) {
        base = first + u32(s1.x * s1.y * s1.z) * BRICK_WORDS;
        side = ((head.size + vec3<i32>(3)) >> vec3<u32>(2u)) + vec3<i32>(1);
    }
    let w = (c >> vec3<u32>(level)) - (head.origin >> vec3<u32>(level));
    if (any(w < vec3<i32>(0)) || any(w >= side)) {
        return vec4<f32>(0.0);
    }
    return store_record(base + u32(w.x + side.x * (w.y + side.y * w.z)) * BRICK_WORDS, face);
}

// The missing-light chain (ADR-0053) for face `face` of voxel `v`, which has no brick value:
// the first source holding the direction, nearest first, under the flags word's mode: the
// chunk's own summary; the mean of its six face neighbours' summaries that hold it (not under
// `parent`); the level-1 then level-2 parent record (not under `neighbour`). 0 in a for none,
// and at once under `black`. One value a chunk and direction, never per voxel.
fn store_fallback(head: StoreHead, v: vec3<i32>, face: u32) -> vec4<f32> {
    let mode = store_mode();
    if (mode == 0u) {
        return vec4<f32>(0.0);
    }
    let c = v >> vec3<u32>(6u);
    let own = store_summary(head, c, face);
    if (own.w > 0.0) {
        return own;
    }
    if (mode != MISSING_PARENT) {
        var sum = vec3<f32>(0.0);
        var n = 0.0;
        for (var f = 0u; f < 6u; f++) {
            var o = vec3<i32>(0);
            o[f / 2u] = select(1, -1, (f & 1u) == 0u);
            let r = store_summary(head, c + o, face);
            sum += r.xyz * r.w;
            n += r.w;
        }
        if (n > 0.0) {
            return vec4<f32>(sum / n, 1.0);
        }
    }
    if (mode != MISSING_NEIGHBOUR) {
        let p1 = store_parent(head, c, 1u, face);
        if (p1.w > 0.0) {
            return p1;
        }
        return store_parent(head, c, 2u, face);
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
    var stored = false;
    if (reads_store(rec)) {
        let ray = pixel_ray(full_px(gid.xy));
        let nf = shadow_normal_of(rec, ray.d);
        let voxel = store_voxel(rec.p, nf);
        let face = store_face(rec.axis, nf);
        let head = store_head();
        let c = store_brick_value(head, voxel, face);
        var value = c;
        if (c.w == 0.0) {
            value = store_fallback(head, voxel, face);
        }
        if (COUNT_STORE) {
            if (c.w > 0.0) {
                atomicAdd(&counters[1], 1u);
            }
            if (c.w == 0.0) {
                atomicAdd(&counters[0], 1u);
                if (value.w > 0.0) {
                    atomicAdd(&counters[3], 1u);
                } else {
                    atomicAdd(&counters[4], 1u);
                }
            } else if ((store_table(head, voxel >> vec3<u32>(6u)) & TABLE_COARSE) != 0u) {
                atomicAdd(&counters[2], 1u);
            }
        }
        stored = true;
        sample = vec4<f32>(value.xyz * exposure_now(), sample.w);
        if ((params.lighting & LIGHTING_LAMP) == 0u && (params.flags & FLAG_CUTAWAY) == 0u
            && view_mode() != VIEW_STEPS) {
            var held = 0.0;
            if ((params.history & 1u) != 0u && !in_edit(rec.p)) {
                held = min(floor(textureLoad(prev_moments_in, px, 0).z + 0.5) + 1.0, AGE_MAX);
            }
            textureStore(accum_out, px, min(vec4<f32>(sample.xyz, 0.0), vec4<f32>(HDR_MAX)));
            textureStore(moments_out, px, vec4<f32>(0.0, 0.0, held, 1.0));
            return;
        }
    }
    // The filtered part's luminance: the lamp's, and a fallback's emissive part.
    let l = min(select(luminance(sample.xyz), 0.0, stored) + sample.w, MOMENT_LUM_MAX);
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
            let e_new = select(e_fallback, sample.xyz, stored);
            accum = vec4<f32>(e_new, mix(h.w, sample.w, a));
            moments = mix(m.xy, vec2<f32>(l, l * l), a);
        }
    }
    textureStore(accum_out, px, min(accum, vec4<f32>(HDR_MAX)));
    // The moments of a luminance at most MOMENT_LUM_MAX: the rescaled history can carry them
    // past it, and the second moment's bound, 65 025, is still a finite half.
    let m_max = vec2<f32>(MOMENT_LUM_MAX, MOMENT_LUM_MAX * MOMENT_LUM_MAX);
    textureStore(moments_out, px, vec4<f32>(min(moments, m_max), age, select(0.0, 1.0, stored)));
}
