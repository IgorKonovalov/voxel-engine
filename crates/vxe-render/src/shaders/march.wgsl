// Primary-ray marcher over the brick-map mirror (ADR-0002, ADR-0004), one invocation per pixel.
//
// Bindings, group 0:
//   0  uniform  params         camera, mirror window, light, target size, flags (bit 0: shadow ray,
//                              bit 1: occupancy masks)
//   1  storage  brick_pool0    shard 0 of the pool: [slot][128 x u32], four u8 palette indices
//                              per word, x fastest; a wide brick is two slots, [256 x u32] of
//                              u16 voxel pairs. Slot s is slot s % slots_per_shard of shard
//                              s / slots_per_shard; 7, 8 and 9 are shards 1 to 3
//   2  storage  chunk_records  [record][640 x u32]: 512 brick entries, then the palette as
//                              256 u16 (index 0 air). Entry bit 31 set: pool slot in bits
//                              0..30, bit 30 wide; clear: the uniform material
//   3  storage  chunk_table    [wrapped chunk]u32 record index in bits 0..30, bit 30 set for a
//                              LOD record (entries 0..64 mip level 1, 64..72 level 2, 72
//                              level 3); 0xFFFFFFFF when absent
//   4  storage  materials      [id][2 x vec4]: albedo rgb + roughness, emission rgb + flag bits
//   5  storage texture         rgba8unorm output
//   6  storage  brick_masks    [slot][2 x u32]; bit b set: block (b & 3, (b >> 2) & 3, b >> 4)
//                              of the slot's brick holds a non-air voxel
//   7  storage  brick_pool1    pool shard 1
//   8  storage  brick_pool2    pool shard 2
//   9  storage  brick_pool3    pool shard 3
//
// World units are voxels. The walk is a DDA over cells of four sizes: a chunk that is absent
// is skipped as one 64^3 cell, a uniform-air brick as one 8^3 cell, an empty 2^3 block of a
// full brick (its mask bit clear) as one 2^3 cell when the masks flag is set, and the voxels
// of an occupied block one at a time. A uniform solid brick is hit where the ray enters it.
// A LOD-only chunk is walked the same way at its mip level L, every size scaled by 2^L: a
// uniform-air mip-brick is one (8 * 2^L)^3 cell, an empty mask block one (2 * 2^L)^3 cell,
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
const WIDE_BIT: u32 = 0x40000000u;
const SLOT_MASK: u32 = 0x3FFFFFFFu;
// u32 words of a pool slot and of a chunk record, and the record word where the palette
// starts.
const SLOT_WORDS: u32 = 128u;
const RECORD_WORDS: u32 = 640u;
const PALETTE_WORD: u32 = 512u;
const MAX_STEPS: u32 = 4096u;
const BIG: f32 = 3.0e38;
// Params.flags bits.
const FLAG_SHADOWS: u32 = 1u;
const FLAG_MASKS: u32 = 2u;

// What the walk found at a voxel: `size` is the edge of the empty cell to skip, or 0 for a
// solid voxel of `material`.
struct Cell {
    size: i32,
    material: u32,
};

struct Hit {
    found: bool,
    voxel: vec3<i32>,
    axis: i32,
    t: f32,
    material: u32,
};

// Word `word` of the brick in global pool slot `slot` (a wide brick's words run on into the
// next slot, which is in the same shard).
fn pool_word(slot: u32, word: u32) -> u32 {
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

fn lookup(v: vec3<i32>) -> Cell {
    let chunk = floor_div(v, 64);
    let rel = chunk - params.window_min;
    let w = params.window_size;
    if (any(rel < vec3<i32>(0)) || any(rel >= w)) {
        return Cell(64, 0u);
    }
    // Wrapped addressing: the table is indexed by the chunk coordinate modulo the window size,
    // and the range check above keeps every lookup inside the current window.
    let wr = ((chunk % w) + w) % w;
    let t = chunk_table[u32(wr.x + w.x * (wr.y + w.y * wr.z))];
    if (t == ABSENT) {
        return Cell(64, 0u);
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
    let base = (t & SLOT_MASK) * RECORD_WORDS;
    let c = (v - chunk * 64) >> vec3<u32>(level);
    let b = c / 8;
    let entry = chunk_records[base + first + u32(b.x + e * (b.y + e * b.z))];
    if ((entry & FULL_BIT) == 0u) {
        if (entry == 0u) {
            return Cell(8 * cell, 0u);
        }
        return Cell(0, entry);
    }
    let slot = entry & SLOT_MASK;
    let l = c - b * 8;
    let k = l / 2;
    let bit = u32(k.x + 4 * k.y + 16 * k.z);
    if ((params.flags & FLAG_MASKS) != 0u
        && (brick_masks[slot * 2u + bit / 32u] & (1u << (bit % 32u))) == 0u) {
        return Cell(2 * cell, 0u);
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
        return Cell(cell, 0u);
    }
    return Cell(0, m);
}
// The voxel holding `p` on a ray of direction `d`: on an axis the ray travels down, a position
// exactly on a voxel boundary belongs to the voxel below.
fn voxel_at(p: vec3<f32>, d: vec3<f32>) -> vec3<i32> {
    return vec3<i32>(select(floor(p), ceil(p) - 1.0, d < vec3<f32>(0.0)));
}

// The first solid voxel along `o + d t` for `0 <= t <= t_limit`.
fn march(o: vec3<f32>, d: vec3<f32>, t_limit: f32) -> Hit {
    var miss = Hit(false, vec3<i32>(0), -1, 0.0, 0u);
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
        v[axis] = select(edge - 1, edge, d[axis] > 0.0);
    }
    v = clamp(v, bmin_i, bmax_i - vec3<i32>(1));

    for (var step = 0u; step < MAX_STEPS; step++) {
        if (any(v < bmin_i) || any(v >= bmax_i)) {
            return miss;
        }
        if (t > t_limit) {
            return miss;
        }
        let c = lookup(v);
        if (c.size == 0) {
            return Hit(true, v, axis, t, c.material);
        }
        let cell_min = floor_div(v, c.size) * c.size;
        let edge = cell_min + select(vec3<i32>(0), vec3<i32>(c.size), d > vec3<f32>(0.0));
        let tk = select(vec3<f32>(BIG), (vec3<f32>(edge) - o) * inv, d != vec3<f32>(0.0));
        var k = 0;
        if (tk.y < tk[k]) { k = 1; }
        if (tk.z < tk[k]) { k = 2; }
        t = tk[k];
        v = voxel_at(o + d * t, d);
        v[k] = select(edge[k] - 1, edge[k], d[k] > 0.0);
        axis = k;
    }
    return miss;
}

fn shade(o: vec3<f32>, d: vec3<f32>, h: Hit) -> vec3<f32> {
    var n = -d;
    if (h.axis >= 0) {
        n = vec3<f32>(0.0);
        n[h.axis] = -sign(d[h.axis]);
    }
    let p = o + d * h.t;
    let albedo = materials[h.material * 2u].xyz;
    let to_light = params.light_pos - p;
    let dist2 = max(dot(to_light, to_light), 1e-4);
    let l = to_light * inverseSqrt(dist2);
    let ndl = max(dot(n, l), 0.0);
    let cone = smoothstep(params.cos_outer, params.cos_inner, dot(-l, params.light_dir));
    var lit = params.light_intensity * cone * ndl / dist2;
    // One shadow ray, from half a voxel off the face toward the lamp; any rock before the lamp
    // removes its light.
    if ((params.flags & FLAG_SHADOWS) != 0u && lit > 0.0) {
        let s = march(p + n * 0.5, l, sqrt(dist2));
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
    let h = march(o, d, BIG);
    var c = vec3<f32>(0.0);
    if (h.found) {
        c = shade(o, d, h);
    }
    // Display encoding: clamp, then an approximate sRGB transfer.
    c = pow(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(1.0 / 2.2));
    textureStore(target_tex, vec2<i32>(gid.xy), vec4<f32>(c, 1.0));
}
