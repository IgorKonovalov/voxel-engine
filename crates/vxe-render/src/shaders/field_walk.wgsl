// The field walk: the primary ray over the field surface of band bricks (ADR-0018). Only the
// hit pass composes it.

override FIELD_SURFACE: bool = false;

// A piece that starts this far above zero (in phi) missed its crossing: a leak.
const FIELD_LEAK_EPS: f32 = 1e-4;

// The field's view of a brick: its chunk's table entry (ABSENT outside the window or for an
// absent chunk) and, in a full chunk, the brick's record entry, band slot (ABSENT for none)
// and, for a band brick, its header's band prefix (HEADER_BAND_PREFIX_WORD, two words) and
// whether it walks as cubes (BAND_CUBES_BIT).
struct FBrick {
    table: u32,
    entry: u32,
    band: u32,
    prefix: vec2<u32>,
    cubes: bool,
};

// The last brick `field_brick` resolved, the last 4^3 block row `field_cell` read (keyed by
// band slot and word), the last group of bricks it tested (keyed by record and word) and the
// last dual cell's corners `field_piece` loaded.
// Keys are packed into one word each, NOT_HELD when nothing is held. `fb_key` is the brick
// coordinate's low ten bits per axis (`brick_key`): unique while the window, with the one brick
// a corner reads past its edge, spans fewer than 1024 bricks (128 chunks) on every axis.
const NOT_HELD: u32 = 0xFFFFFFFFu;

var<private> fb_key: u32;

var<private> fb: FBrick;

var<private> fm_key: u32;

var<private> fm_opaque: vec4<u32>;

var<private> fm_band: vec4<u32>;

var<private> fg_key: u32;

var<private> fg_bits: u32;

var<private> fc_held: bool;

var<private> fc_base: vec3<i32>;

// The held cell's corner count bytes, corner i in byte i % 4 of word i / 4, CRISP_COUNT_BIT
// kept: two registers held across the walk where eight `phi` values would take eight.
var<private> fc_bytes: vec2<u32>;

fn reset_field_walk() {
    reset_walk();
    fb_key = NOT_HELD;
    fm_key = NOT_HELD;
    fg_key = NOT_HELD;
    fc_held = false;
}

fn brick_key(bw: vec3<i32>) -> u32 {
    let k = vec3<u32>(bw) & vec3<u32>(1023u);
    return k.x | (k.y << 10u) | (k.z << 20u);
}

fn field_brick(bw: vec3<i32>) -> FBrick {
    let key = brick_key(bw);
    if (key == fb_key) {
        return fb;
    }
    fb_key = key;
    var r = FBrick(ABSENT, 0u, ABSENT, vec2<u32>(0u), false);
    let chunk = floor_div(bw, 8);
    let rel = chunk - params.window_min;
    let w = params.window_size;
    if (all(rel >= vec3<i32>(0)) && all(rel < w)) {
        let wr = ((chunk % w) + w) % w;
        loads += 1u;
        r.table = chunk_table[u32(wr.x + w.x * (wr.y + w.y * wr.z))];
        if (r.table != ABSENT && (r.table & LOD_BIT) == 0u) {
            let rec = r.table & RECORD_MASK;
            let b = bw - chunk * 8;
            let bi = u32(b.x + 8 * (b.y + 8 * b.z));
            r.entry = record_word(rec * RECORD_WORDS + ENTRY_WORD + bi);
            if ((r.entry & FULL_BIT) != 0u) {
                let unit = r.entry & UNIT_MASK;
                let h = pool_word(unit, HEADER_WORD + HEADER_BAND_WORD);
                if ((h & HEADER_BAND_BIT) != 0u) {
                    r.band = h & UNIT_MASK;
                    r.cubes = (h & BAND_CUBES_BIT) != 0u;
                    r.prefix = vec2<u32>(
                        pool_word(unit, HEADER_WORD + HEADER_BAND_PREFIX_WORD),
                        pool_word(unit, HEADER_WORD + HEADER_BAND_PREFIX_WORD + 1u),
                    );
                }
            }
        }
    }
    fb = r;
    return r;
}

// Whether voxel `i` of the brick with record entry `entry` (in a full chunk) is solid to the
// field: not air, a translucent voxel included.
fn field_solid(entry: u32, i: u32) -> bool {
    if ((entry & FULL_BIT) == 0u) {
        return (entry & MATERIAL_MASK) != 0u;
    }
    let unit = entry & UNIT_MASK;
    if (((pool_word(unit, i / 32u) >> (i % 32u)) & 1u) != 0u) {
        return true;
    }
    if ((entry & TRANSLUCENT_BIT) == 0u) {
        return false;
    }
    if (((entry >> CLASS_SHIFT) & 7u) == CLASS_WIDE) {
        return voxel_material(entry, i) != 0u;
    }
    let t = pool_word(unit, PAYLOAD_WORD + UNIT_WORDS * payload_units(entry) + i / 32u);
    return ((t >> (i % 32u)) & 1u) != 0u;
}

// A band voxel's count byte, NOT_BAND for a voxel outside the band.
const NOT_BAND: u32 = 0xFFFFFFFFu;

// The count byte of voxel `i` of brick `b` (a full chunk's) from its band slot, or NOT_BAND.
fn field_byte(b: FBrick, i: u32) -> u32 {
    if (b.band == ABSENT) {
        return NOT_BAND;
    }
    let w = i / 32u;
    let m = pool_word(b.band, w);
    let bit = 1u << (i % 32u);
    if ((m & bit) == 0u) {
        return NOT_BAND;
    }
    // The layer's prefix, the low word of the layer when `i` is in its high word, and the bits
    // below `i` in its own word.
    var k = layer_prefix(b.prefix.x, b.prefix.y, i / 64u) + countOneBits(m & (bit - 1u));
    if ((w & 1u) != 0u) {
        k += countOneBits(pool_word(b.band, w - 1u));
    }
    return (pool_word(b.band, UNIT_WORDS + k / 4u) >> (8u * (k % 4u))) & 0xFFu;
}

// The count of voxel `u`: a band voxel's from its band slot, else 27 for a solid voxel and 0
// for air (a LOD-only chunk's through its mip cell).
fn field_count(u: vec3<i32>) -> u32 {
    return field_count_raw(u) & 0x1Fu;
}

// `field_count` with a band voxel's CRISP_COUNT_BIT kept.
fn field_count_raw(u: vec3<i32>) -> u32 {
    let bw = floor_div(u, 8);
    let b = field_brick(bw);
    if (b.table == ABSENT) {
        return 0u;
    }
    if ((b.table & LOD_BIT) != 0u) {
        return select(0u, 27u, lookup(u).size == 0);
    }
    let l = vec3<u32>(u - bw * 8);
    let i = l.x + 8u * (l.y + 8u * l.z);
    let k = field_byte(b, i);
    if (k != NOT_BAND) {
        return k;
    }
    return select(0u, 27u, field_solid(b.entry, i));
}

// Whether voxel `u` is crisp: a band voxel whose count byte carries CRISP_COUNT_BIT.
fn field_crisp(u: vec3<i32>) -> bool {
    let bw = floor_div(u, 8);
    let b = field_brick(bw);
    if (b.table == ABSENT || (b.table & LOD_BIT) != 0u) {
        return false;
    }
    let l = vec3<u32>(u - bw * 8);
    let k = field_byte(b, l.x + 8u * (l.y + 8u * l.z));
    return k != NOT_BAND && (k & CRISP_COUNT_BIT) != 0u;
}

fn field_phi(u: vec3<i32>) -> f32 {
    return f32(field_count(u)) / 27.0 - 0.5;
}

// What the field walk finds at voxel `v`: `size` > 0 an empty cell of that edge; else `band`
// for a band voxel, `solid` for a solid voxel (opaque, or of a uniform solid brick), `lod` for
// a cube hit that is no leak (a LOD-only chunk's solid cell, or a solid voxel of a band brick
// walked as cubes).
struct FCell {
    size: i32,
    band: bool,
    solid: bool,
    lod: bool,
};

fn field_cell(v: vec3<i32>) -> FCell {
    let skips = (params.flags & FLAG_SKIPS) != 0u;
    let chunk = floor_div(v, 64);
    let rel = chunk - params.window_min;
    if (any(rel < vec3<i32>(0)) || any(rel >= params.window_size)) {
        return FCell(64, false, false, false);
    }
    let bw = floor_div(v, 8);
    let b = field_brick(bw);
    if (b.table == ABSENT) {
        return FCell(select(1, 64, skips), false, false, false);
    }
    if ((b.table & LOD_BIT) != 0u) {
        let c = lookup_walk(v, false);
        return FCell(c.size, false, c.size == 0, c.size == 0);
    }
    let rec = b.table & RECORD_MASK;
    if (skips) {
        // A 2 x 2 x 2 group of bricks none of which is uniform air, from the record's brick
        // bits (a band brick's entry is never 0); in an empty chunk they are clear.
        let lb = bw - chunk * 8;
        let g = vec3<u32>(lb / 2);
        let wi = 4u * g.z + g.y / 2u;
        // Record indices stay below 2^28, so the key is never NOT_HELD.
        let key = rec * 16u + wi;
        if (key != fg_key) {
            fg_key = key;
            fg_bits = 0u;
            if ((b.table & EMPTY_BIT) == 0u) {
                let base = rec * RECORD_WORDS;
                fg_bits = record_word(base + wi) | record_word(base + wi + 2u);
            }
        }
        if ((fg_bits & (0x0303u << (2u * g.x + 16u * (g.y & 1u)))) == 0u) {
            return FCell(16, false, false, false);
        }
    }
    let l = vec3<u32>(v - bw * 8);
    let i = l.x + 8u * (l.y + 8u * l.z);
    let full = (b.entry & FULL_BIT) != 0u;
    if (b.band == ABSENT || b.cubes) {
        if (!full) {
            if ((b.entry & MATERIAL_MASK) == 0u) {
                return FCell(select(1, 8, skips), false, false, false);
            }
            // A uniform solid brick outside the band.
            return FCell(0, false, true, false);
        }
        // A Full brick outside the band, or a band brick walked as cubes: its opaque voxels,
        // cubes; in a band brick a cube hit is no leak (`lod`).
        let c = lookup_walk(v, false);
        return FCell(c.size, false, c.size == 0, b.band != ABSENT && c.size == 0);
    }
    // A band brick: its band mask ORed with its opaque mask, read a 4^3 block row at a time as
    // `mask_skip` reads the opaque mask.
    let q = l / 4u;
    let w = 8u * q.z + q.y;
    let key = b.band * 16u + w;
    if (key != fm_key) {
        fm_key = key;
        let unit = b.entry & UNIT_MASK;
        let cls = (b.entry >> CLASS_SHIFT) & 7u;
        if (cls == CLASS_UNIFORM && (b.entry & TRANSLUCENT_BIT) != 0u) {
            // A uniform translucent brick is solid to the field throughout.
            fm_opaque = vec4<u32>(0xFFFFFFFFu);
        } else {
            fm_opaque = vec4<u32>(
                mask_word_at(unit, w),
                mask_word_at(unit, w + 2u),
                mask_word_at(unit, w + 4u),
                mask_word_at(unit, w + 6u),
            );
        }
        fm_band = vec4<u32>(
            pool_word(b.band, w),
            pool_word(b.band, w + 2u),
            pool_word(b.band, w + 4u),
            pool_word(b.band, w + 6u),
        );
    }
    let mk = fm_opaque | fm_band;
    if (skips) {
        if (((mk.x | mk.y | mk.z | mk.w) & (0x0F0F0F0Fu << (4u * q.x))) == 0u) {
            return FCell(4, false, false, false);
        }
        let h = l / 2u;
        let j = 2u * ((l.z / 2u) & 1u);
        if (((mk[j] | mk[j + 1u]) & (0x0303u << (2u * h.x + 16u * (h.y & 1u)))) == 0u) {
            return FCell(2, false, false, false);
        }
    }
    let bit = 1u << (i % 32u);
    if ((mk[l.z & 3u] & bit) == 0u) {
        return FCell(1, false, false, false);
    }
    return FCell(0, (fm_band[l.z & 3u] & bit) != 0u, (fm_opaque[l.z & 3u] & bit) != 0u, false);
}

// A field walk's hit: `t`, the normal (the field's, or a cube hit's face normal), whether it
// is a cube hit (a leak or a LOD cell) and whether it is a leak.
struct FHit {
    found: bool,
    leak: bool,
    cube: bool,
    t: f32,
    n: vec3<f32>,
    // The dual cell's base voxel and the hit's position in it, for the normal and material.
    base: vec3<i32>,
    f: vec3<f32>,
};

// The trilinear gradient of the corners `c` (index x + 2 y + 4 z) at `f`.
fn trilinear_gradient(c: array<f32, 8>, f: vec3<f32>) -> vec3<f32> {
    let gx = mix(mix(c[1] - c[0], c[3] - c[2], f.y), mix(c[5] - c[4], c[7] - c[6], f.y), f.z);
    let gy = mix(mix(c[2] - c[0], c[3] - c[1], f.x), mix(c[6] - c[4], c[7] - c[5], f.x), f.z);
    let gz = mix(mix(c[4] - c[0], c[5] - c[1], f.x), mix(c[6] - c[2], c[7] - c[3], f.x), f.y);
    return vec3<f32>(gx, gy, gz);
}

fn cubic_at(k: vec4<f32>, s: f32) -> f32 {
    return ((k.w * s + k.z) * s + k.y) * s + k.x;
}

// The first crossing of the field along `o + d t` for `t` in `[ta, tb]`, inside one dual
// cell: found, or a leak at `ta` (see the walk's comment).
// The dual cell holding the piece `[ta, tb]` of `o + d t`: its base voxel, its corners
// loaded into `fc_bytes` unless they are held.
fn field_corners(o: vec3<f32>, d: vec3<f32>, ta: f32, tb: f32) -> vec3<i32> {
    let pm = o + d * (0.5 * (ta + tb));
    let base = vec3<i32>(floor(pm - 0.5));
    if (!fc_held || any(base != fc_base)) {
        fc_held = true;
        fc_base = base;
        fc_bytes = vec2<u32>(0u);
        // The corners in brick order, so `field_brick`'s held brick serves each brick's corners
        // in a row: the bits of `j` go to the axes the cell does not cross a brick face on
        // first, then to those it crosses.
        let cross = (base & vec3<i32>(7)) == vec3<i32>(7);
        let nc = u32(!cross.x) + u32(!cross.y) + u32(!cross.z);
        let px = select(0u, nc, cross.x);
        let py = select(u32(!cross.x), nc + u32(cross.x), cross.y);
        let pz = select(u32(!cross.x) + u32(!cross.y), nc + u32(cross.x) + u32(cross.y), cross.z);
        for (var j = 0u; j < 8u; j++) {
            let i = ((j >> px) & 1u) | (((j >> py) & 1u) << 1u) | (((j >> pz) & 1u) << 2u);
            let k = field_count_raw(base + vec3<i32>(vec3<u32>(i & 1u, (i >> 1u) & 1u, i >> 2u)));
            // A select, not a store through `fc_bytes[i / 4u]` (no runtime-indexed vector
            // component store, `shader_portability`).
            let byte = (k & 0xFFu) << (8u * (i % 4u));
            fc_bytes |= select(vec2<u32>(byte, 0u), vec2<u32>(0u, byte), i >= 4u);
        }
    }
    return base;
}

// Whether voxel `u`, a corner of the held cell at `base`, is crisp.
fn corner_crisp(base: vec3<i32>, u: vec3<i32>) -> bool {
    let o = vec3<u32>(u - base);
    let i = o.x + 2u * o.y + 4u * o.z;
    return ((fc_bytes[i / 4u] >> (8u * (i % 4u))) & CRISP_COUNT_BIT) != 0u;
}

// The held cell's corner `phi` values.
fn held_phi() -> array<f32, 8> {
    var c: array<f32, 8>;
    for (var i = 0u; i < 8u; i++) {
        c[i] = f32((fc_bytes[i / 4u] >> (8u * (i % 4u))) & 0x1Fu) / 27.0 - 0.5;
    }
    return c;
}

// With `crisp_test`, the piece is the first of band voxel `v`'s segment, and a crisp `v` (one
// of the piece's corners) is its cube, hit at `ta`. A cell with a crisp corner and no smooth
// solid corner gives no hit.
fn field_piece(o: vec3<f32>, d: vec3<f32>, ta: f32, tb: f32, crisp_test: bool, v: vec3<i32>) -> FHit {
    var none = FHit(false, false, false, ta, vec3<f32>(0.0), vec3<i32>(0), vec3<f32>(0.0));
    let base = field_corners(o, d, ta, tb);
    if (crisp_test && corner_crisp(base, v)) {
        return FHit(true, false, true, ta, vec3<f32>(0.0), v, vec3<f32>(0.5));
    }
    let bits = fc_bytes.x | fc_bytes.y;
    if ((bits & (CRISP_COUNT_BIT * 0x01010101u)) != 0u && (bits & (SMOOTH_COUNT_BIT * 0x01010101u)) == 0u) {
        return none;
    }
    let c = held_phi();
    var pos = false;
    var neg = false;
    for (var i = 0u; i < 8u; i++) {
        pos = pos || c[i] > 0.0;
        neg = neg || c[i] <= 0.0;
    }
    if (!pos) {
        return none;
    }
    let q0 = o + d * ta - (vec3<f32>(base) + 0.5);
    if (!neg) {
        return FHit(true, true, true, ta, vec3<f32>(0.0), base, q0);
    }
    // phi(ta + s) = k.x + k.y s + k.z s^2 + k.w s^3: each corner's weight is a product of
    // three linear factors a + b s.
    var k = vec4<f32>(0.0);
    for (var i = 0u; i < 8u; i++) {
        let bit = vec3<u32>(i & 1u, (i >> 1u) & 1u, i >> 2u) == vec3<u32>(1u);
        let a = select(1.0 - q0, q0, bit);
        let b = select(-d, d, bit);
        k += c[i] * vec4<f32>(
            a.x * a.y * a.z,
            b.x * a.y * a.z + a.x * b.y * a.z + a.x * a.y * b.z,
            b.x * b.y * a.z + b.x * a.y * b.z + a.x * b.y * b.z,
            b.x * b.y * b.z,
        );
    }
    let len = tb - ta;
    if (k.x >= 0.0) {
        return FHit(true, k.x > FIELD_LEAK_EPS, false, ta, vec3<f32>(0.0), base, q0);
    }
    // The extrema: roots of 3 k.w s^2 + 2 k.z s + k.y inside (0, len), ascending.
    var cut = vec2<f32>(len, len);
    let qa = 3.0 * k.w;
    let qb = 2.0 * k.z;
    let qc = k.y;
    if (abs(qa) > 1e-12) {
        let disc = qb * qb - 4.0 * qa * qc;
        if (disc >= 0.0) {
            let qq = -0.5 * (qb + select(-1.0, 1.0, qb >= 0.0) * sqrt(disc));
            var r1 = qq / qa;
            var r2 = select(len, qc / qq, abs(qq) > 1e-20);
            if (r1 > r2) {
                let tmp = r1;
                r1 = r2;
                r2 = tmp;
            }
            cut = vec2<f32>(select(len, r1, r1 > 0.0 && r1 < len), select(len, r2, r2 > 0.0 && r2 < len));
            if (cut.x > cut.y) {
                cut = cut.yx;
            }
        }
    } else if (abs(qb) > 1e-12) {
        let r = -qc / qb;
        cut.x = select(len, r, r > 0.0 && r < len);
    }
    var s0 = 0.0;
    var f0 = k.x;
    for (var run = 0u; run < 3u; run++) {
        let s1 = select(select(len, cut.y, run == 1u), cut.x, run == 0u);
        if (s1 <= s0) {
            continue;
        }
        let f1 = cubic_at(k, s1);
        if (f1 >= 0.0) {
            // Illinois steps on [s0, s1], f(s0) < 0 <= f(s1).
            var a = s0;
            var fa = f0;
            var b = s1;
            var fb = f1;
            var side = 0;
            for (var it = 0u; it < 8u; it++) {
                let s = (a * fb - b * fa) / (fb - fa);
                let fs = cubic_at(k, s);
                if (fs >= 0.0) {
                    b = s;
                    fb = fs;
                    if (side == 1) {
                        fa *= 0.5;
                    }
                    side = 1;
                } else {
                    a = s;
                    fa = fs;
                    if (side == -1) {
                        fb *= 0.5;
                    }
                    side = -1;
                }
            }
            let s = (a * fb - b * fa) / (fb - fa);
            let t = ta + clamp(s, 0.0, len);
            return FHit(true, false, false, t, vec3<f32>(0.0), base, o + d * t - (vec3<f32>(base) + 0.5));
        }
        s0 = s1;
        f0 = f1;
    }
    return none;
}

// The field test of band voxel `v` over the ray's segment `[t0, t1]` through it: its pieces
// in the dual cells, nearest first.
// With `crisp_test` (a solid `v`) a crisp `v` is its cube, hit at `t0`:
// cube and not a leak; its crispness is read with the first piece's corners, of which `v` is one.
fn field_voxel(o: vec3<f32>, d: vec3<f32>, inv: vec3<f32>, v: vec3<i32>, t0: f32, t1: f32, crisp_test: bool) -> FHit {
    var tm = select(vec3<f32>(BIG), (vec3<f32>(v) + 0.5 - o) * inv, d != vec3<f32>(0.0));
    tm = select(vec3<f32>(t1), tm, tm > vec3<f32>(t0) & tm < vec3<f32>(t1));
    var a = tm.x;
    var b = tm.y;
    var c = tm.z;
    if (a > b) {
        let s = a;
        a = b;
        b = s;
    }
    if (b > c) {
        let s = b;
        b = c;
        c = s;
    }
    if (a > b) {
        let s = a;
        a = b;
        b = s;
    }
    let cuts = array<f32, 4>(a, b, c, t1);
    var lo = t0;
    var first = crisp_test;
    for (var i = 0u; i < 4u; i++) {
        let hi = cuts[i];
        if (hi > lo) {
            let h = field_piece(o, d, lo, hi, first, v);
            first = false;
            if (h.found) {
                return h;
            }
            lo = hi;
        }
    }
    return FHit(false, false, false, t1, vec3<f32>(0.0), vec3<i32>(0), vec3<f32>(0.0));
}

// The normal of a field hit at `f` in the cell at `base`: the field's gradient, `fallback`
// where it vanishes.
fn field_normal(base: vec3<i32>, f: vec3<f32>, fallback: vec3<f32>) -> vec3<f32> {
    // The hit's cell is the last one `field_piece` loaded: its corners are held.
    var c = held_phi();
    if (!fc_held || any(base != fc_base)) {
        for (var i = 0u; i < 8u; i++) {
            c[i] = field_phi(base + vec3<i32>(vec3<u32>(i & 1u, (i >> 1u) & 1u, i >> 2u)));
        }
    }
    return normal_from_gradient(trilinear_gradient(c, f), fallback);
}

// Whether voxel `u` is crisp, from the held cell at `base` when `u` is one of its corners.
fn held_crisp(held: bool, base: vec3<i32>, u: vec3<i32>) -> bool {
    if (held && all(u >= base) && all(u <= base + vec3<i32>(1))) {
        return corner_crisp(base, u);
    }
    return field_crisp(u);
}

// Whether voxel `u` is solid to the opaque walk (`lookup`'s `size == 0`: an opaque voxel; a
// translucent one is air to it), read through `field_brick`'s held brick; a LOD-only chunk's
// through `lookup`.
fn field_opaque(u: vec3<i32>) -> bool {
    let bw = floor_div(u, 8);
    let b = field_brick(bw);
    if (b.table == ABSENT) {
        return false;
    }
    if ((b.table & LOD_BIT) != 0u) {
        return lookup(u).size == 0;
    }
    let e = b.entry;
    if (e == 0u) {
        return false;
    }
    if ((e & FULL_BIT) == 0u) {
        return (e & TRANSLUCENT_BIT) == 0u;
    }
    let l = vec3<u32>(u - bw * 8);
    let i = l.x + 8u * (l.y + 8u * l.z);
    return ((pool_word(e & UNIT_MASK, i / 32u) >> (i % 32u)) & 1u) != 0u;
}

// The material of voxel `u`, one `field_opaque` reads as solid: `lookup(u).material`.
fn field_opaque_material(u: vec3<i32>) -> u32 {
    let bw = floor_div(u, 8);
    let b = field_brick(bw);
    if ((b.table & LOD_BIT) != 0u) {
        return lookup(u).material;
    }
    if ((b.entry & FULL_BIT) == 0u) {
        return b.entry & MATERIAL_MASK;
    }
    let l = vec3<u32>(u - bw * 8);
    return voxel_material(b.entry, l.x + 8u * (l.y + 8u * l.z));
}

// The material of a field hit at `p` seen along `d` in the cell at `base`, `f` its position
// there: the voxel holding `p` when it is solid, else the solid corner of the cell with the
// largest trilinear weight, the first in corner order on a tie (0 when none is solid to the
// opaque walk). A crisp voxel is not solid here.
fn field_material(p: vec3<f32>, d: vec3<f32>, base: vec3<i32>, f: vec3<f32>) -> u32 {
    let held = fc_held && all(base == fc_base);
    let at = voxel_at(p, d);
    if (field_opaque(at) && !held_crisp(held, base, at)) {
        return field_opaque_material(at);
    }
    var best = -1.0;
    var chosen = vec3<i32>(0);
    for (var i = 0u; i < 8u; i++) {
        let o = vec3<u32>(i & 1u, (i >> 1u) & 1u, i >> 2u);
        let w3 = select(1.0 - f, f, o == vec3<u32>(1u));
        let w = w3.x * w3.y * w3.z;
        if (w > best) {
            let u = base + vec3<i32>(o);
            if (field_opaque(u) && !held_crisp(held, base, u)) {
                best = w;
                chosen = u;
            }
        }
    }
    if (best < 0.0) {
        return 0u;
    }
    return field_opaque_material(chosen);
}

// The field walk's first hit along `o + d t`, `t >= 0`, with the entry axis of its voxel
// (-1 when the ray started inside it) and its material, FIELD_MATERIAL_PENDING for a field
// hit, whose material `field_material` resolves after the walk.
const FIELD_MATERIAL_PENDING: u32 = 0xFFFFFFFFu;

struct FieldHit {
    h: FHit,
    axis: i32,
    material: u32,
};

fn field_march(o: vec3<f32>, d: vec3<f32>) -> FieldHit {
    var miss = FieldHit(FHit(false, false, false, 0.0, vec3<f32>(0.0), vec3<i32>(0), vec3<f32>(0.0)), -1, 0u);
    let bmin_i = params.window_min * 64;
    let bmax_i = (params.window_min + params.window_size) * 64;
    let bmin = vec3<f32>(bmin_i);
    let bmax = vec3<f32>(bmax_i);
    let inv = select(vec3<f32>(BIG), 1.0 / d, d != vec3<f32>(0.0));
    let ta = (bmin - o) * inv;
    let tb = (bmax - o) * inv;
    let tmin = min(ta, tb);
    let tmax = max(ta, tb);
    let t0 = max(max(tmin.x, tmin.y), tmin.z);
    let t1 = min(min(tmax.x, tmax.y), tmax.z);
    if (t1 < max(t0, 0.0)) {
        return miss;
    }
    var t = max(t0, 0.0);
    var axis = -1;
    var v = voxel_at(o + d * t, d);
    if (t0 > 0.0) {
        axis = select(select(2, 1, tmin.y == t0), 0, tmin.x == t0);
        let face = select(bmax_i[axis], bmin_i[axis], d[axis] > 0.0);
        v = advance(o, d, inv, t, axis, face);
    }
    v = clamp(v, bmin_i, bmax_i - vec3<i32>(1));
    return field_walk(o, d, t, v, axis);
}

// The field walk from voxel `v`, entered at `t` through its face on `axis` (-1 for none):
// `field_march`'s loop, and where `field_resume` takes a split walk's ray on.
fn field_walk(o: vec3<f32>, d: vec3<f32>, t_start: f32, v_start: vec3<i32>, axis_start: i32) -> FieldHit {
    var miss = FieldHit(FHit(false, false, false, 0.0, vec3<f32>(0.0), vec3<i32>(0), vec3<f32>(0.0)), -1, 0u);
    let bmin_i = params.window_min * 64;
    let bmax_i = (params.window_min + params.window_size) * 64;
    let inv = select(vec3<f32>(BIG), 1.0 / d, d != vec3<f32>(0.0));
    reset_field_walk();
    var t = t_start;
    var v = v_start;
    var axis = axis_start;
    for (var step = 0u; step < MAX_STEPS; step++) {
        walk_steps += 1u;
        if (any(v < bmin_i) || any(v >= bmax_i)) {
            return miss;
        }
        let c = field_cell(v);
        var size = c.size;
        if (size == 0) {
            if (!c.band) {
                // A LOD cell, or a solid voxel outside the band: a cube hit, a leak unless LOD.
                var face = FHit(true, !c.lod, true, t, vec3<f32>(0.0), v, vec3<f32>(0.5));
                return FieldHit(face, axis, lookup(v).material);
            }
            let h = field_voxel(o, d, inv, v, t, cube_exit(o, d, inv, v), c.solid);
            if (h.found && h.cube && !h.leak) {
                // A crisp voxel: its cube, hit where the ray enters it.
                return FieldHit(h, axis, field_opaque_material(v));
            }
            if (h.found) {
                // The material is resolved after the walk (`field_pixel`).
                return FieldHit(h, axis, FIELD_MATERIAL_PENDING);
            }
            size = 1;
        }
        let cell_min = floor_div(v, size) * size;
        let edge = cell_min + select(vec3<i32>(0), vec3<i32>(size), d > vec3<f32>(0.0));
        let tk = select(vec3<f32>(BIG), (vec3<f32>(edge) - o) * inv, d != vec3<f32>(0.0));
        var k = 0;
        if (tk.y < tk[k]) { k = 1; }
        if (tk.z < tk[k]) { k = 2; }
        t = tk[k];
        v = advance(o, d, inv, t, k, edge[k]);
        axis = k;
    }
    return miss;
}

// The hit pass's pixel in the field look: the field walk's hit record; the layer holds the
// steps view's counters, the cracks view's leak (1 in r) or nothing (no translucent walk).
// With `resume` the walk starts at voxel `v`, entered at `t` through its face on `axis` (a
// split walk's hand-off), else at the ray's origin.
fn field_pixel(gid: vec2<u32>, resume: bool, t: f32, v: vec3<i32>, axis: i32) {
    let ray = pixel_ray(gid);
    let o = ray.o;
    let d = ray.d;
    walk_steps = 0u;
    loads = 0u;
    var fh: FieldHit;
    if (resume) {
        fh = field_walk(o, d, t, v, axis);
    } else {
        fh = field_march(o, d);
    }
    let h = fh.h;
    if (h.found && fh.material == FIELD_MATERIAL_PENDING) {
        fh.material = field_material(o + d * h.t, d, h.base, h.f);
    }
    let counts = vec3<u32>(min(walk_steps, 255u), min(counted_loads(), 255u), min(counted_pool_reads(), 255u));
    var pos = vec4<f32>(0.0, 0.0, 0.0, BIG);
    var info = vec4<u32>(0u);
    if (h.found) {
        var nf = -d;
        if (fh.axis >= 0) {
            nf = select(vec3<f32>(0.0), vec3<f32>(-sign(d[fh.axis])), axis_mask(fh.axis));
        }
        var n = nf;
        if (!h.cube) {
            n = face_mixed(field_normal(h.base, h.f, nf), nf);
        }
        let flags = HIT_FOUND | select(HIT_FIELD, 0u, h.cube) | (u32(fh.axis + 1) << HIT_AXIS_SHIFT);
        pos = vec4<f32>(o + d * h.t, h.t);
        info = vec4<u32>(octa16_encode(n), fh.material, flags);
    }
    var layer = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    let view = view_mode();
    if (view == VIEW_STEPS) {
        layer = vec4<f32>(vec3<f32>(counts), 1.0);
    } else if (view == VIEW_CRACKS) {
        layer = vec4<f32>(select(0.0, 1.0, h.found && h.leak), 0.0, 0.0, 1.0);
    }
    let px = vec2<i32>(gid);
    textureStore(hit_pos_out, px, pos);
    textureStore(hit_info_out, px, info);
    textureStore(layer_out, px, layer);
}
