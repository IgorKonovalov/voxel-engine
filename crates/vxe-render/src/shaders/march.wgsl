// Primary-ray marcher over the brick-map mirror (ADR-0002, ADR-0004), one invocation per pixel.
//
// Bindings, group 0:
//   0  uniform  params         camera, mirror window, light, target size, flags (bit 0: shadow ray,
//                              bit 1: occupancy masks, bits 2..4: normal mode, bit 4: detail,
//                              bits 5..8: view, bit 8: the mirror has attribute pages)
//   1  storage  brick_pool0    shard 0 of the pool: [slot][128 x u32], four u8 palette indices
//                              per word, x fastest; a wide brick is two slots, [256 x u32] of
//                              u16 voxel pairs. Slot s is slot s % slots_per_shard of shard
//                              s / slots_per_shard; 7, 8 and 9 are shards 1 to 3
//   2  storage  chunk_records  [record][656 x u32]: 16 words of brick bits (bit b of word
//                              b / 32 set when entry b is not uniform air), 512 brick entries,
//                              then the palette as 256 u16 (index 0 air). Entry bit 31 set:
//                              pool slot in bits 0..30, bit 30 wide; clear: the uniform
//                              material
//   3  storage  chunk_table    [wrapped chunk]u32 record index in bits 0..29, bit 29 set when
//                              the record's brick bits are all clear, bit 30 set for a LOD
//                              record (entries 0..64 mip level 1, 64..72 level 2, 72 level 3);
//                              0xFFFFFFFF when absent
//   4  storage  materials      [id][4 x vec4]: albedo rgb + roughness, emission rgb + flag bits,
//                              detail scale (voxels) + detail strength + 2 spare, 4 spare
//   5  storage texture         rgba8unorm output
//   6  storage  brick_masks    [slot][2 x u32]; bit b set: block (b & 3, (b >> 2) & 3, b >> 4)
//                              of the slot's brick holds a non-air voxel
//   7  storage  brick_pool1    pool shard 1
//   8  storage  brick_pool2    pool shard 2
//   9  storage  brick_pool3    pool shard 3
//
// With attribute pages (flag bit 8), a full chunk's Full brick in slot s of the page
// p = s - s % 4096 has its 512 u16 contour attributes in slots p + 1364 + 2 (s - p) and the
// one after, two per word, x fastest: bits 0..6 octahedral u, 6..12 octahedral v, 12..16 the
// plane offset code k for (k - 7) / 8 voxel along the normal from the voxel centre; 0xFFFF is
// no plane. In the Contour normal mode the walk tests a solid voxel that has a plane against
// the ray: it is hit where the ray enters the plane's solid half-space inside the voxel's
// cube, with the plane normal, and skipped when the ray's segment through the cube misses
// that half-space. A solid voxel with no plane, of a uniform brick or of a mip-brick, hits
// as a cube.
//
// World units are voxels. The walk is a DDA over cells of five sizes: a chunk that is absent
// or empty (its table entry's EMPTY_BIT) is skipped as one 64^3 cell, a 2 x 2 x 2 group of
// bricks whose brick bits are all clear as one 16^3 cell, a uniform-air brick as one 8^3 cell, an empty 2^3 block of a
// full brick (its mask bit clear) as one 2^3 cell when the masks flag is set, and the voxels
// of an occupied block one at a time. A uniform solid brick is hit where the ray enters it.
// A LOD-only chunk is walked the same way at its mip level L, every size scaled by 2^L: a
// clear group of mip-bricks is one (16 * 2^L)^3 cell below level 3, a uniform-air mip-brick
// one (8 * 2^L)^3 cell, an empty mask block one (2 * 2^L)^3 cell,
// and each mip cell one (2^L)^3 cell. Every skip is aligned to its size, so a solid cell is
// always entered on its face. Each step leaves
// the current cell through its nearest face; the coordinate on the exit axis is set exactly
// from the face, the others come from the float position, rounded toward the side the ray is
// heading so that a position on a boundary (a tie between axes) lands in the voxel entered.

struct Params {
    cam_pos: vec3<f32>,
    tan_half_fov: f32,
    cam_right: vec3<f32>,
    aspect: f32,
    cam_up: vec3<f32>,
    _pad0: f32,
    cam_fwd: vec3<f32>,
    _pad1: f32,
    window_min: vec3<i32>,
    _pad2: i32,
    window_size: vec3<i32>,
    _pad3: i32,
    light_pos: vec3<f32>,
    light_intensity: f32,
    light_dir: vec3<f32>,
    cos_outer: f32,
    size: vec2<u32>,
    cos_inner: f32,
    flags: u32,
    slots_per_shard: u32,
    forced_level: u32,
    lod1_r2: i32,
    lod2_r2: i32,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> brick_pool0: array<u32>;
@group(0) @binding(2) var<storage, read> chunk_records: array<u32>;
@group(0) @binding(3) var<storage, read> chunk_table: array<u32>;
@group(0) @binding(4) var<storage, read> materials: array<vec4<f32>>;
@group(0) @binding(5) var target_tex: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(6) var<storage, read> brick_masks: array<u32>;
@group(0) @binding(7) var<storage, read> brick_pool1: array<u32>;
@group(0) @binding(8) var<storage, read> brick_pool2: array<u32>;
@group(0) @binding(9) var<storage, read> brick_pool3: array<u32>;

const ABSENT: u32 = 0xFFFFFFFFu;
const FULL_BIT: u32 = 0x80000000u;
const LOD_BIT: u32 = 0x40000000u;
const EMPTY_BIT: u32 = 0x20000000u;
const RECORD_MASK: u32 = 0x1FFFFFFFu;
const WIDE_BIT: u32 = 0x40000000u;
const SLOT_MASK: u32 = 0x3FFFFFFFu;
// u32 words of a pool slot and of a chunk record, and the record words where the brick
// entries and the palette start.
const SLOT_WORDS: u32 = 128u;
const RECORD_WORDS: u32 = 656u;
const ENTRY_WORD: u32 = 16u;
const PALETTE_WORD: u32 = 528u;
const MAX_STEPS: u32 = 4096u;
const BIG: f32 = 3.0e38;
// Params.flags bits.
const FLAG_SHADOWS: u32 = 1u;
const FLAG_MASKS: u32 = 2u;
const NORMAL_SHIFT: u32 = 2u;
const FLAG_DETAIL: u32 = 16u;
const VIEW_SHIFT: u32 = 5u;
const VIEW_BITS: u32 = 7u;
const FLAG_ATTR_PAGES: u32 = 256u;
// Normal modes (flags bits 2..4) and views (flags bits 5..8).
const NORMAL_FACE: u32 = 0u;
const NORMAL_TRILINEAR8: u32 = 1u;
const NORMAL_GRADIENT27: u32 = 2u;
const NORMAL_CONTOUR: u32 = 3u;
const VIEW_SHADED: u32 = 0u;
const VIEW_NORMALS: u32 = 1u;
const VIEW_HITMASK: u32 = 2u;
const VIEW_STEPS: u32 = 3u;
// Attribute pages: brick slots per page, slots per page, and the attribute of no plane.
const PAGE_SLOTS: u32 = 4096u;
const ATTR_PAGE_BRICK_SLOTS: u32 = 1364u;
const NO_PLANE: u32 = 0xFFFFu;

// What the walk found at a voxel: `size` is the edge of the empty cell to skip, or 0 for a
// solid voxel of `material`. A solid voxel of a full chunk's Full brick carries its brick's
// pool slot and its index in the brick; any other cell carries ABSENT.
struct Cell {
    size: i32,
    material: u32,
    slot: u32,
    index: u32,
};

// `plane`: the hit is on a contour plane, `n` its normal.
struct Hit {
    found: bool,
    voxel: vec3<i32>,
    axis: i32,
    t: f32,
    material: u32,
    plane: bool,
    n: vec3<f32>,
};

// The walk's cost counters: loop iterations of `march` and `pool_word` calls since the last
// reset. `main` resets them before the primary ray and reads them after it in the steps view.
var<private> walk_steps: u32;
var<private> pool_reads: u32;

// Word `word` of the brick in global pool slot `slot` (a wide brick's words run on into the
// next slot, which is in the same shard).
fn pool_word(slot: u32, word: u32) -> u32 {
    pool_reads += 1u;
    let shard = slot / params.slots_per_shard;
    let i = (slot - shard * params.slots_per_shard) * SLOT_WORDS + word;
    switch shard {
        case 0u: { return brick_pool0[i]; }
        case 1u: { return brick_pool1[i]; }
        case 2u: { return brick_pool2[i]; }
        default: { return brick_pool3[i]; }
    }
}

fn floor_div(a: vec3<i32>, b: i32) -> vec3<i32> {
    return vec3<i32>(floor(vec3<f32>(a) / f32(b)));
}

// True in the lane of axis `k`. A component store through a runtime index (`v[k] = x`) is
// not addressable under FXC, the D3D12 HLSL compiler, so writes go through
// `select(v, vec3(x), axis_mask(k))` instead; reads through a runtime index are fine.
fn axis_mask(k: i32) -> vec3<bool> {
    return vec3<i32>(0, 1, 2) == vec3<i32>(k);
}

// The mip level a LOD-only chunk is walked at: forced, or by its squared chunk distance to the
// camera's chunk.
fn lod_level(chunk: vec3<i32>) -> u32 {
    if (params.forced_level != 0u) {
        return params.forced_level;
    }
    let d = chunk - floor_div(vec3<i32>(floor(params.cam_pos)), 64);
    let d2 = dot(d, d);
    if (d2 <= params.lod1_r2) {
        return 1u;
    }
    if (d2 <= params.lod2_r2) {
        return 2u;
    }
    return 3u;
}

// Whether the 2 x 2 x 2 group of bricks holding brick `b` of the record at word `base`, walked
// at `level`, has a brick that is not uniform air, from the record's brick bits. At level 0
// brick (x, y, z) is bit x + 8 (y % 4) of word 2 z + y / 4, so a group's bits are two bytes
// in each of two words; at level 1 (entries x + 4 y + 16 z) one word holds the group; at
// level 2 the group is the whole level, entries 64..72; level 3 has no group.
fn group_occupied(base: u32, level: u32, b: vec3<i32>) -> bool {
    let g = vec3<u32>(b / 2);
    switch level {
        case 0u: {
            let w = base + 4u * g.z + g.y / 2u;
            let m = 0x0303u << (2u * g.x + 16u * (g.y & 1u));
            return ((chunk_records[w] | chunk_records[w + 2u]) & m) != 0u;
        }
        case 1u: {
            return (chunk_records[base + g.z] & (0x00330033u << (2u * g.x + 8u * g.y))) != 0u;
        }
        case 2u: {
            return (chunk_records[base + 2u] & 0xFFu) != 0u;
        }
        default: {
            return true;
        }
    }
}

fn lookup(v: vec3<i32>) -> Cell {
    let chunk = floor_div(v, 64);
    let rel = chunk - params.window_min;
    let w = params.window_size;
    if (any(rel < vec3<i32>(0)) || any(rel >= w)) {
        return Cell(64, 0u, ABSENT, 0u);
    }
    // Wrapped addressing: the table is indexed by the chunk coordinate modulo the window size,
    // and the range check above keeps every lookup inside the current window.
    let wr = ((chunk % w) + w) % w;
    let t = chunk_table[u32(wr.x + w.x * (wr.y + w.y * wr.z))];
    if (t == ABSENT || (t & EMPTY_BIT) != 0u) {
        return Cell(64, 0u, ABSENT, 0u);
    }
    // A full chunk is level 0: voxels in 8^3 bricks, 8 bricks to an edge. A LOD record holds
    // level L's cells in 8^3 mip-bricks, 8 >> L to an edge, from entry 0, 64 or 72.
    var level = 0u;
    var first = 0u;
    if ((t & LOD_BIT) != 0u) {
        level = lod_level(chunk);
        first = select(select(72u, 64u, level == 2u), 0u, level == 1u);
    }
    let cell = 1 << level;
    let e = 8 >> level;
    let base = (t & RECORD_MASK) * RECORD_WORDS;
    let c = (v - chunk * 64) >> vec3<u32>(level);
    let b = c / 8;
    if (!group_occupied(base, level, b)) {
        return Cell(16 * cell, 0u, ABSENT, 0u);
    }
    let entry = chunk_records[base + ENTRY_WORD + first + u32(b.x + e * (b.y + e * b.z))];
    if ((entry & FULL_BIT) == 0u) {
        if (entry == 0u) {
            return Cell(8 * cell, 0u, ABSENT, 0u);
        }
        return Cell(0, entry, ABSENT, 0u);
    }
    let slot = entry & SLOT_MASK;
    let l = c - b * 8;
    let k = l / 2;
    let bit = u32(k.x + 4 * k.y + 16 * k.z);
    if ((params.flags & FLAG_MASKS) != 0u
        && (brick_masks[slot * 2u + bit / 32u] & (1u << (bit % 32u))) == 0u) {
        return Cell(2 * cell, 0u, ABSENT, 0u);
    }
    let i = u32(l.x + 8 * (l.y + 8 * l.z));
    var m = 0u;
    if ((entry & WIDE_BIT) != 0u) {
        // A wide brick: slots `slot` and `slot + 1` hold 512 u16 voxels.
        let word = pool_word(slot, i / 2u);
        m = (word >> ((i & 1u) * 16u)) & 0xFFFFu;
    } else {
        // A narrow brick: one byte per voxel, an index into the record's palette; 0 is air.
        let word = pool_word(slot, i / 4u);
        let index = (word >> ((i & 3u) * 8u)) & 0xFFu;
        if (index != 0u) {
            let pair = chunk_records[base + PALETTE_WORD + index / 2u];
            m = (pair >> ((index & 1u) * 16u)) & 0xFFFFu;
        }
    }
    if (m == 0u) {
        return Cell(cell, 0u, ABSENT, 0u);
    }
    return Cell(0, m, select(ABSENT, slot, level == 0u), i);
}
// The voxel holding `p` on a ray of direction `d`: on an axis the ray travels down, a position
// exactly on a voxel boundary belongs to the voxel below.
fn voxel_at(p: vec3<f32>, d: vec3<f32>) -> vec3<i32> {
    return vec3<i32>(select(floor(p), ceil(p) - 1.0, d < vec3<f32>(0.0)));
}

// Octahedral decoding of 6 + 6 bits into a unit vector.
fn decode_octahedral(e: u32) -> vec3<f32> {
    let p = vec2<f32>(f32(e & 63u), f32((e >> 6u) & 63u)) / 63.0 * 2.0 - 1.0;
    var n = vec3<f32>(p, 1.0 - abs(p.x) - abs(p.y));
    if (n.z < 0.0) {
        let s = select(vec2<f32>(-1.0), vec2<f32>(1.0), n.xy >= vec2<f32>(0.0));
        n = vec3<f32>((1.0 - abs(n.yx)) * s, n.z);
    }
    return normalize(n);
}

// The contour attribute of voxel `index` of the Full brick in pool slot `slot`.
fn contour_attr(slot: u32, index: u32) -> u32 {
    let page = slot - slot % PAGE_SLOTS;
    let attr_slot = page + ATTR_PAGE_BRICK_SLOTS + 2u * (slot - page);
    return (pool_word(attr_slot, index / 2u) >> ((index & 1u) * 16u)) & 0xFFFFu;
}

// The contour test of solid voxel `v`, entered at `t`: `kind` 0 when the voxel has no plane,
// 1 for a hit at `t` with normal `n`, 2 when the ray misses the plane's solid half-space
// inside the voxel's cube.
struct Contour {
    kind: u32,
    t: f32,
    n: vec3<f32>,
};

fn contour_test(o: vec3<f32>, d: vec3<f32>, inv: vec3<f32>, v: vec3<i32>, t: f32, c: Cell) -> Contour {
    var none = Contour(0u, t, vec3<f32>(0.0));
    if (c.slot == ABSENT) {
        return none;
    }
    let a = contour_attr(c.slot, c.index);
    if (a == NO_PLANE) {
        return none;
    }
    let n = decode_octahedral(a);
    let offset = (f32(a >> 12u) - 7.0) / 8.0;
    // Signed distance along n from the plane, negative inside the solid half-space.
    let centre = vec3<f32>(v) + 0.5;
    let s_in = dot(n, o + d * t - centre) - offset;
    if (s_in <= 0.0) {
        return Contour(1u, t, n);
    }
    let edge = vec3<f32>(v) + select(vec3<f32>(0.0), vec3<f32>(1.0), d > vec3<f32>(0.0));
    let tk = select(vec3<f32>(BIG), (edge - o) * inv, d != vec3<f32>(0.0));
    let t_out = min(min(tk.x, tk.y), tk.z);
    let s_out = dot(n, o + d * t_out - centre) - offset;
    if (s_out > 0.0) {
        return Contour(2u, t, n);
    }
    return Contour(1u, t + (t_out - t) * s_in / (s_in - s_out), n);
}

fn contour_active() -> bool {
    return ((params.flags >> NORMAL_SHIFT) & 3u) == NORMAL_CONTOUR
        && (params.flags & FLAG_ATTR_PAGES) != 0u;
}

// The first solid voxel along `o + d t` for `0 <= t <= t_limit`.
fn march(o: vec3<f32>, d: vec3<f32>, t_limit: f32) -> Hit {
    var miss = Hit(false, vec3<i32>(0), -1, 0.0, 0u, false, vec3<f32>(0.0));
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
        let p = o + d * t;
        let edge = i32(round(p[axis]));
        v = select(v, vec3<i32>(select(edge - 1, edge, d[axis] > 0.0)), axis_mask(axis));
    }
    v = clamp(v, bmin_i, bmax_i - vec3<i32>(1));
    let contour = contour_active();

    for (var step = 0u; step < MAX_STEPS; step++) {
        walk_steps += 1u;
        if (any(v < bmin_i) || any(v >= bmax_i)) {
            return miss;
        }
        if (t > t_limit) {
            return miss;
        }
        let c = lookup(v);
        var size = c.size;
        if (size == 0) {
            if (!contour) {
                return Hit(true, v, axis, t, c.material, false, vec3<f32>(0.0));
            }
            let p = contour_test(o, d, inv, v, t, c);
            if (p.kind == 0u) {
                return Hit(true, v, axis, t, c.material, false, vec3<f32>(0.0));
            }
            if (p.kind == 1u) {
                return Hit(true, v, axis, p.t, c.material, true, p.n);
            }
            // The ray passes the voxel outside its plane: walk on as through air.
            size = 1;
        }
        let cell_min = floor_div(v, size) * size;
        let edge = cell_min + select(vec3<i32>(0), vec3<i32>(size), d > vec3<f32>(0.0));
        let tk = select(vec3<f32>(BIG), (vec3<f32>(edge) - o) * inv, d != vec3<f32>(0.0));
        var k = 0;
        if (tk.y < tk[k]) { k = 1; }
        if (tk.z < tk[k]) { k = 2; }
        t = tk[k];
        v = voxel_at(o + d * t, d);
        v = select(v, vec3<i32>(select(edge[k] - 1, edge[k], d[k] > 0.0)), axis_mask(k));
        axis = k;
    }
    return miss;
}

fn normal_mode() -> u32 {
    return (params.flags >> NORMAL_SHIFT) & 3u;
}

fn view_mode() -> u32 {
    return (params.flags >> VIEW_SHIFT) & VIEW_BITS;
}

// The normal of the face the ray entered the hit voxel through; `-d` when the ray started
// inside rock.
fn face_normal(d: vec3<f32>, h: Hit) -> vec3<f32> {
    var n = -d;
    if (h.axis >= 0) {
        n = select(vec3<f32>(0.0), vec3<f32>(-sign(d[h.axis])), axis_mask(h.axis));
    }
    return n;
}

// 1 for a solid voxel, 0 for air, read through `lookup` (a LOD-only chunk answers with its
// mip cell).
fn occupancy(v: vec3<i32>) -> f32 {
    return select(0.0, 1.0, lookup(v).size == 0);
}

// `-normalize(g)`, or `fallback` when the gradient `g` vanishes.
fn normal_from_gradient(g: vec3<f32>, fallback: vec3<f32>) -> vec3<f32> {
    let l2 = dot(g, g);
    if (l2 < 1e-12) {
        return fallback;
    }
    return -g * inverseSqrt(l2);
}

// Central differences of occupancy over the 3 x 3 x 3 voxels around the hit voxel: each axis
// sums the nine voxels on its positive face minus the nine on its negative face.
fn gradient27_normal(h: Hit, fallback: vec3<f32>) -> vec3<f32> {
    var g = vec3<f32>(0.0);
    for (var z = -1; z <= 1; z++) {
        for (var y = -1; y <= 1; y++) {
            for (var x = -1; x <= 1; x++) {
                let o = vec3<i32>(x, y, z);
                if (any(o != vec3<i32>(0))) {
                    g += vec3<f32>(o) * occupancy(h.voxel + o);
                }
            }
        }
    }
    return normal_from_gradient(g, fallback);
}

// The gradient of the trilinearly interpolated occupancy of the 2 x 2 x 2 voxel centres
// around the hit point `p`. Along the hit axis the stencil is exactly the hit voxel and the
// voxel across the entry face, at weight 1/2 each, so a flat wall reads its face normal; on
// the other axes it is the two voxel centres either side of `p`.
fn trilinear8_normal(p: vec3<f32>, h: Hit, nf: vec3<f32>) -> vec3<f32> {
    var base = vec3<i32>(floor(p - 0.5));
    var f = p - 0.5 - vec3<f32>(base);
    let k = h.axis;
    base = select(base, vec3<i32>(min(h.voxel[k], h.voxel[k] + i32(nf[k]))), axis_mask(k));
    f = select(f, vec3<f32>(0.5), axis_mask(k));
    var c: array<f32, 8>;
    for (var i = 0; i < 8; i++) {
        c[i] = occupancy(base + vec3<i32>(i & 1, (i >> 1) & 1, i >> 2));
    }
    let gx = mix(mix(c[1] - c[0], c[3] - c[2], f.y), mix(c[5] - c[4], c[7] - c[6], f.y), f.z);
    let gy = mix(mix(c[2] - c[0], c[3] - c[1], f.x), mix(c[6] - c[4], c[7] - c[5], f.x), f.z);
    let gz = mix(mix(c[4] - c[0], c[5] - c[1], f.x), mix(c[6] - c[2], c[7] - c[3], f.x), f.y);
    return normal_from_gradient(vec3<f32>(gx, gy, gz), nf);
}

// The shading normal of hit `h` under the current normal mode. A ray that starts inside rock
// (no entry face) keeps the face-mode normal in every mode.
fn surface_normal(o: vec3<f32>, d: vec3<f32>, h: Hit) -> vec3<f32> {
    let nf = face_normal(d, h);
    if (h.axis < 0) {
        return nf;
    }
    if (h.plane) {
        return h.n;
    }
    let mode = normal_mode();
    if (mode == NORMAL_TRILINEAR8) {
        return trilinear8_normal(o + d * h.t, h, nf);
    }
    if (mode == NORMAL_GRADIENT27) {
        return gradient27_normal(h, nf);
    }
    return nf;
}

// A hash of lattice point `c` to [0, 1).
fn hash3(c: vec3<i32>) -> f32 {
    let x = bitcast<vec3<u32>>(c) * vec3<u32>(1597334677u, 3812015801u, 2798796415u);
    var h = (x.x ^ x.y ^ x.z) * 2654435761u;
    h ^= h >> 15u;
    h *= 2246822519u;
    h ^= h >> 13u;
    return f32(h >> 8u) / 16777216.0;
}

// Value noise at `p` with a quintic fade: the value in [0, 1) and its gradient.
fn value_noise(p: vec3<f32>) -> vec4<f32> {
    let fl = floor(p);
    let i = vec3<i32>(fl);
    let f = p - fl;
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let du = 30.0 * f * f * (f * (f - 2.0) + 1.0);
    let a = hash3(i);
    let b = hash3(i + vec3<i32>(1, 0, 0));
    let c = hash3(i + vec3<i32>(0, 1, 0));
    let dd = hash3(i + vec3<i32>(1, 1, 0));
    let e = hash3(i + vec3<i32>(0, 0, 1));
    let ff = hash3(i + vec3<i32>(1, 0, 1));
    let g = hash3(i + vec3<i32>(0, 1, 1));
    let hh = hash3(i + vec3<i32>(1, 1, 1));
    let k1 = b - a;
    let k2 = c - a;
    let k3 = e - a;
    let k4 = a - b - c + dd;
    let k5 = a - c - e + g;
    let k6 = a - b - e + ff;
    let k7 = -a + b + c - dd + e - ff - g + hh;
    let v = a + k1 * u.x + k2 * u.y + k3 * u.z + k4 * u.x * u.y + k5 * u.y * u.z
        + k6 * u.z * u.x + k7 * u.x * u.y * u.z;
    let grad = du * vec3<f32>(
        k1 + k4 * u.y + k6 * u.z + k7 * u.y * u.z,
        k2 + k5 * u.z + k4 * u.x + k7 * u.z * u.x,
        k3 + k6 * u.x + k5 * u.y + k7 * u.x * u.y,
    );
    return vec4<f32>(v, grad);
}

// Normal tilt of the detail term per unit strength and unit noise gradient in feature units.
const DETAIL_TILT: f32 = 4.0;

// Procedural detail at world point `p` (voxels) on a surface of normal `n`: two octaves of
// value noise at feature scale `scale` and `scale / 2`, weights 1 and 1/2. Returns the normal
// tilted against the noise's tangential gradient (in feature units) times `strength` and
// `DETAIL_TILT`, and the
// albedo factor `1 + strength (2 v - 1)` for the noise value `v` in [0, 1). A pure function of
// `p`, so a wall point shades the same from any camera.
fn detail(p: vec3<f32>, n: vec3<f32>, scale: f32, strength: f32) -> vec4<f32> {
    let q = p / scale;
    let o1 = value_noise(q);
    let o2 = value_noise(q * 2.0 + vec3<f32>(17.0, 31.0, 47.0));
    let v = (o1.x + 0.5 * o2.x) / 1.5;
    // Gradient with respect to q: octave 2 is sampled at 2q.
    let g = (o1.yzw + o2.yzw) / 1.5;
    let gt = g - dot(g, n) * n;
    let tilted = normalize(n - (strength * DETAIL_TILT) * gt);
    return vec4<f32>(tilted, 1.0 + strength * (2.0 * v - 1.0));
}

fn shade(o: vec3<f32>, d: vec3<f32>, h: Hit, n: vec3<f32>, tint: f32) -> vec3<f32> {
    // The shadow ray leaves along the face normal in every mode: it is a robustness offset
    // out of the hit voxel, not part of the look.
    let nf = face_normal(d, h);
    let p = o + d * h.t;
    let albedo = materials[h.material * 4u].xyz * tint;
    let to_light = params.light_pos - p;
    let dist2 = max(dot(to_light, to_light), 1e-4);
    let l = to_light * inverseSqrt(dist2);
    let ndl = max(dot(n, l), 0.0);
    let cone = smoothstep(params.cos_outer, params.cos_inner, dot(-l, params.light_dir));
    var lit = params.light_intensity * cone * ndl / dist2;
    // One shadow ray, from half a voxel off the face toward the lamp; any rock before the lamp
    // removes its light.
    if ((params.flags & FLAG_SHADOWS) != 0u && lit > 0.0) {
        let s = march(p + nf * 0.5, l, sqrt(dist2));
        if (s.found) {
            lit = 0.0;
        }
    }
    return albedo * lit;
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.size.x || gid.y >= params.size.y) {
        return;
    }
    let ndc = (vec2<f32>(gid.xy) + 0.5) / vec2<f32>(params.size) * 2.0 - 1.0;
    let d = normalize(
        params.cam_fwd
        + params.cam_right * (ndc.x * params.tan_half_fov * params.aspect)
        - params.cam_up * (ndc.y * params.tan_half_fov)
    );
    let o = params.cam_pos;
    walk_steps = 0u;
    pool_reads = 0u;
    let h = march(o, d, BIG);
    let view = view_mode();
    if (view == VIEW_STEPS) {
        // The primary ray's cost: red its loop iterations, green its pool reads, each
        // clamped at 255. No shading, so no shadow ray.
        let cost = vec2<f32>(vec2<u32>(min(walk_steps, 255u), min(pool_reads, 255u))) / 255.0;
        textureStore(target_tex, vec2<i32>(gid.xy), vec4<f32>(cost, 0.0, 1.0));
        return;
    }
    if (view == VIEW_HITMASK) {
        textureStore(target_tex, vec2<i32>(gid.xy), vec4<f32>(vec3<f32>(select(0.0, 1.0, h.found)), 1.0));
        return;
    }
    var c = vec3<f32>(0.0);
    if (h.found) {
        var n = surface_normal(o, d, h);
        var tint = 1.0;
        if ((params.flags & FLAG_DETAIL) != 0u) {
            // Exactly no change at strength 0.
            let row = materials[h.material * 4u + 2u];
            if (row.y > 0.0) {
                let dt = detail(o + d * h.t, n, row.x, row.y);
                n = dt.xyz;
                tint = dt.w;
            }
        }
        if (view == VIEW_NORMALS) {
            // The normal itself, linear: n * 0.5 + 0.5 per channel.
            textureStore(target_tex, vec2<i32>(gid.xy), vec4<f32>(n * 0.5 + 0.5, 1.0));
            return;
        }
        c = shade(o, d, h, n, tint);
    }
    // Display encoding: clamp, then an approximate sRGB transfer.
    c = pow(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(1.0 / 2.2));
    textureStore(target_tex, vec2<i32>(gid.xy), vec4<f32>(c, 1.0));
}
