// The attribute pass: fills the contour attributes of the bricks in a work list from the pool
// itself. Four bricks a workgroup, 64 invocations a brick: item 4 g + b is brick b of
// workgroup g, and the last workgroup may run partly empty. A brick resolves itself and its 26
// neighbours once (table and record reads), loads their non-air masks (16 words each) into
// workgroup memory, lists its surface voxels in attribute-index order (each invocation scans
// one byte of the surface mask), and sets the occupancy of the brick and a one-voxel shell
// around it (10^3 voxels, as bits) from those masks, a LOD-only chunk's neighbour through
// its level-1 mip brick's, reading voxel by voxel only a brick of more than 16 materials
// holding a translucent voxel. Then
// invocation t computes attribute words t, t + 64, ... (indices 2 w and 2 w + 1) up to the
// surface's count and writes each whole; the words past it keep the "no plane" the upload
// wrote. The four bricks share the workgroup's three barriers.
//
// Bindings, group 0:
//   0  uniform  params         mirror window, units per shard, work-list length, the
//                              dispatch's workgroups along x, the plane fit's bits
//   1  storage  brick_pool0    pool shard 0, read-write; 2, 3 and 4 are shards 1 to 3. Same
//                              layout as in march.wgsl; this pass writes only attribute units
//   5  storage  chunk_records  as in march.wgsl
//   6  storage  chunk_table    as in march.wgsl
//   7  storage  attrib_work    [n][4 x u32]: the brick's record entry less bit 31 (its slot's
//                              first unit, class and translucent flag), brick origin in
//                              voxels (x, y, z as i32 bits)
//
// A Full brick's slot holds, after its mask, header, payload and translucent unit (march.wgsl's
// attr_offset), its surface mask (one unit, the mask's layout), then a u16 attribute for each
// surface voxel, two per word, voxel (x, y, z)'s at index prefix[z] (header words 10..14) plus
// the surface bits below it in its layer; header word 14's low half is the attribute units, 0
// for a brick without attributes, which the pass leaves alone. A voxel's u16 is
// bits 0..6 octahedral u, 6..12 octahedral v, 12..16 the plane offset code k in 0..=14 for
// (k - 7) / 8 voxel along the normal from the voxel centre; 0xFFFF is "no plane": air, a voxel
// whose six face neighbours are all solid, or one whose occupancy gradient vanishes.
//
// The normal is the Gradient27 normal of occupancy (central differences over the 3 x 3 x 3
// voxels around the voxel), read through the chunk table: neighbours across brick and chunk
// boundaries count, a voxel in an absent chunk is air, one in a LOD-only chunk its level-1 mip
// cell. With params.fit bit 0 (`PlaneFit::Weighted27`) each neighbour at offset o weighs
// 1 / |o|^2 (face 1, edge 1/2, corner 1/3); without it every neighbour weighs 1. The offset
// is (f - 0.5) * 3 voxels, f the solid fraction of the 27 voxels; with params.fit bit 1
// (`PlaneOffset::VolumeMatched`) it is instead the d at which the half-space n.(x - c) <= d
// cuts the 3 x 3 x 3 block about the voxel centre c into a solid volume equal to the solid
// count, found by 8 bisection steps on [-1.5, 1.5]. Either is clamped to [-7/8, 7/8] and
// rounded to the nearest 1/8.

struct Params {
    window_min: vec3<i32>,
    units_per_shard: u32,
    window_size: vec3<i32>,
    items: u32,
    groups_x: u32,
    // Bit 0: the weighted gradient; bit 1: the volume-matched offset.
    fit: u32,
    _pad1: u32,
    _pad2: u32,
};

const FIT_WEIGHTED: u32 = 1u;
const FIT_VOLUME: u32 = 2u;

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> brick_pool0: array<u32>;
@group(0) @binding(2) var<storage, read_write> brick_pool1: array<u32>;
@group(0) @binding(3) var<storage, read_write> brick_pool2: array<u32>;
@group(0) @binding(4) var<storage, read_write> brick_pool3: array<u32>;
@group(0) @binding(5) var<storage, read> chunk_records: array<u32>;
@group(0) @binding(6) var<storage, read> chunk_table: array<u32>;
@group(0) @binding(7) var<storage, read> attrib_work: array<vec4<u32>>;

const ABSENT: u32 = 0xFFFFFFFFu;
const FULL_BIT: u32 = 0x80000000u;
const LOD_BIT: u32 = 0x40000000u;
// Bit 29 of an entry flags a brick holding a translucent voxel; a Full entry's first unit
// takes bits 0..26 and its class bits 26..29 (5: Wide, raw u16 ids).
const TRANSLUCENT_BIT: u32 = 0x20000000u;
const UNIT_MASK: u32 = 0x3FFFFFFu;
const CLASS_SHIFT: u32 = 26u;
const CLASS_WIDE: u32 = 5u;
const EMPTY_BIT: u32 = 0x20000000u;
const RECORD_MASK: u32 = 0x1FFFFFFFu;
const UNIT_WORDS: u32 = 16u;
const RECORD_WORDS: u32 = 528u;
const ENTRY_WORD: u32 = 16u;
const PAYLOAD_WORD: u32 = 32u;
const NO_PLANE: u32 = 0xFFFFu;
// Bricks a workgroup and invocations a brick; `attrib::BRICKS_PER_GROUP` and
// `attrib::WORKGROUP` must match.
const BRICKS: u32 = 4u;
const LANES: u32 = 64u;
// The occupancy box: the brick and a one-voxel shell, 10 voxels to an edge.
const BOX: u32 = 10u;

// Per brick of the workgroup (brick b's at b times the per-brick length), 12 384 B in all:
// its occupancy box as 1000 bits (word w set by invocation w alone); it and its 26
// neighbours, index (dx + 1) + 3 (dy + 1) + 9 (dz + 1), how each is read (NB_AIR, NB_SOLID,
// NB_MASK or NB_MIP with its entry, NB_VOXELS) and the NB_MASK and NB_MIP ones' non-air
// masks; and its surface voxels' cells in attribute-index order, two u16 a word.
var<workgroup> occ: array<u32, 128>;
var<workgroup> nb_kind: array<u32, 108>;
var<workgroup> nb_entry: array<u32, 108>;
var<workgroup> nb_mask: array<u32, 1728>;
var<workgroup> cells: array<atomic<u32>, 1024>;

// The shard and index of word `word` of the slot whose first unit is `unit`.
fn shard_index(unit: u32, word: u32) -> vec2<u32> {
    let shard = unit / params.units_per_shard;
    return vec2<u32>(shard, (unit - shard * params.units_per_shard) * UNIT_WORDS + word);
}

fn pool_word(unit: u32, word: u32) -> u32 {
    let a = shard_index(unit, word);
    switch a.x {
        case 0u: { return brick_pool0[a.y]; }
        case 1u: { return brick_pool1[a.y]; }
        case 2u: { return brick_pool2[a.y]; }
        default: { return brick_pool3[a.y]; }
    }
}

fn store_pool_word(unit: u32, word: u32, value: u32) {
    let a = shard_index(unit, word);
    switch a.x {
        case 0u: { brick_pool0[a.y] = value; }
        case 1u: { brick_pool1[a.y] = value; }
        case 2u: { brick_pool2[a.y] = value; }
        default: { brick_pool3[a.y] = value; }
    }
}

fn floor_div(a: vec3<i32>, b: i32) -> vec3<i32> {
    return vec3<i32>(floor(vec3<f32>(a) / f32(b)));
}

// Whether voxel `v` is solid: march.wgsl's lookup with every LOD-only chunk at level 1.
fn solid(v: vec3<i32>) -> bool {
    let chunk = floor_div(v, 64);
    let rel = chunk - params.window_min;
    let w = params.window_size;
    if (any(rel < vec3<i32>(0)) || any(rel >= w)) {
        return false;
    }
    let wr = ((chunk % w) + w) % w;
    let t = chunk_table[u32(wr.x + w.x * (wr.y + w.y * wr.z))];
    if (t == ABSENT || (t & EMPTY_BIT) != 0u) {
        return false;
    }
    var level = 0u;
    if ((t & LOD_BIT) != 0u) {
        level = 1u;
    }
    let e = 8 >> level;
    let base = (t & RECORD_MASK) * RECORD_WORDS;
    let c = (v - chunk * 64) >> vec3<u32>(level);
    let b = c / 8;
    let entry = chunk_records[base + ENTRY_WORD + u32(b.x + e * (b.y + e * b.z))];
    if ((entry & FULL_BIT) == 0u) {
        // A uniform brick: its material in the low 16 bits, a translucent one solid here as
        // for every reader but the opaque walk.
        return (entry & 0xFFFFu) != 0u;
    }
    let unit = entry & UNIT_MASK;
    let l = c - b * 8;
    let i = u32(l.x + 8 * (l.y + 8 * l.z));
    if (((pool_word(unit, i / 32u) >> (i % 32u)) & 1u) != 0u) {
        // An opaque voxel.
        return true;
    }
    if ((entry & TRANSLUCENT_BIT) == 0u) {
        return false;
    }
    // Air or a translucent voxel: a Wide brick's id, else the translucent unit after the planes.
    let cls = (entry >> CLASS_SHIFT) & 7u;
    if (cls == CLASS_WIDE) {
        let word = pool_word(unit, PAYLOAD_WORD + i / 2u);
        return ((word >> ((i & 1u) * 16u)) & 0xFFFFu) != 0u;
    }
    let tw = pool_word(unit, PAYLOAD_WORD + UNIT_WORDS * payload_units(entry) + i / 32u);
    return ((tw >> (i % 32u)) & 1u) != 0u;
}

// Units of a Full entry's payload: 0, 1, 2 or 4 planes for classes 1 to 4, 16 for Wide.
fn payload_units(entry: u32) -> u32 {
    let cls = (entry >> CLASS_SHIFT) & 7u;
    return select(select(cls - 1u, 4u, cls == 4u), 16u, cls == CLASS_WIDE);
}

// The unit offset of a Full entry's attributes from its first unit, as march.wgsl's.
fn attr_offset(entry: u32) -> u32 {
    let cls = (entry >> CLASS_SHIFT) & 7u;
    let t = select(0u, 1u, (entry & TRANSLUCENT_BIT) != 0u && cls != CLASS_WIDE);
    return 2u + payload_units(entry) + t;
}

const NB_AIR: u32 = 0u;
const NB_SOLID: u32 = 1u;
const NB_MASK: u32 = 2u;
const NB_VOXELS: u32 = 3u;
// NB_MIP + o: a brick of a LOD-only chunk read through its level-1 mip brick's masks, `o`
// (0..8) the octant of that mip brick the brick covers (x in bit 0, y in 1, z in 2).
const NB_MIP: u32 = 4u;

// How the brick at world brick coordinate `bw` is read, and its entry: NB_AIR outside the
// window, in an absent or empty chunk or for uniform air; NB_SOLID for a uniform brick of any
// other material; NB_MASK for a Full brick of a full chunk, read through its masks; NB_MIP
// plus its octant for a brick of a LOD-only chunk whose level-1 mip brick is Full, read
// through that brick's masks (a uniform mip brick reads as NB_AIR or NB_SOLID); NB_VOXELS,
// read voxel by voxel by `solid`, for a Wide brick holding a translucent voxel at either
// level.
fn brick_kind(bw: vec3<i32>) -> vec2<u32> {
    let chunk = floor_div(bw, 8);
    let rel = chunk - params.window_min;
    let w = params.window_size;
    if (any(rel < vec3<i32>(0)) || any(rel >= w)) {
        return vec2<u32>(NB_AIR, 0u);
    }
    let wr = ((chunk % w) + w) % w;
    let t = chunk_table[u32(wr.x + w.x * (wr.y + w.y * wr.z))];
    if (t == ABSENT || (t & EMPTY_BIT) != 0u) {
        return vec2<u32>(NB_AIR, 0u);
    }
    // A LOD-only chunk's level-1 bricks each cover 2^3 of its level-0 bricks: `solid` maps
    // voxel l of level-0 brick b to voxel 4 (b % 2) + l / 2 of level-1 brick b / 2.
    let lod = (t & LOD_BIT) != 0u;
    let b = bw - chunk * 8;
    var slot = u32(b.x + 8 * (b.y + 8 * b.z));
    var kind = NB_MASK;
    if (lod) {
        let h = b / 2;
        let o = b - h * 2;
        slot = u32(h.x + 4 * (h.y + 4 * h.z));
        kind = NB_MIP + u32(o.x + 2 * o.y + 4 * o.z);
    }
    let entry = chunk_records[(t & RECORD_MASK) * RECORD_WORDS + ENTRY_WORD + slot];
    if ((entry & FULL_BIT) == 0u) {
        return vec2<u32>(select(NB_AIR, NB_SOLID, (entry & 0xFFFFu) != 0u), entry);
    }
    let cls = (entry >> CLASS_SHIFT) & 7u;
    if ((entry & TRANSLUCENT_BIT) != 0u && cls == CLASS_WIDE) {
        return vec2<u32>(NB_VOXELS, entry);
    }
    return vec2<u32>(kind, entry);
}

// Octahedral encoding of the unit vector `n` into 6 + 6 bits: each coordinate `p` in [-1, 1]
// on codes 0..=62, `round((p + 1) * 31)`, so `p = 0` and `p = +-1` are codes 31, 0 and 62 and
// every axis normal and zero component decodes exactly (ADR-0038). Code 63 is unused.
fn encode_octahedral(n: vec3<f32>) -> u32 {
    var p = n.xy / (abs(n.x) + abs(n.y) + abs(n.z));
    if (n.z < 0.0) {
        p = (1.0 - abs(p.yx)) * select(vec2<f32>(-1.0), vec2<f32>(1.0), p >= vec2<f32>(0.0));
    }
    let q = vec2<u32>(round(clamp(p + 1.0, vec2<f32>(0.0), vec2<f32>(2.0)) * 31.0));
    return q.x | (q.y << 6u);
}

// Occupancy of box cell `c` (each component in 0..10) of the workgroup's brick `b`.
fn occ_at(b: u32, c: vec3<i32>) -> bool {
    let j = u32(c.x) + BOX * (u32(c.y) + BOX * u32(c.z));
    return ((occ[b * 32u + j / 32u] >> (j % 32u)) & 1u) != 0u;
}

// The attribute of the voxel at box cell `c` (each component in 1..9) of brick `b`.
fn voxel_attr(b: u32, c: vec3<i32>) -> u32 {
    if (!occ_at(b, c)) {
        return NO_PLANE;
    }
    let weighted = (params.fit & FIT_WEIGHTED) != 0u;
    var g = vec3<f32>(0.0);
    var count = 0.0;
    var faces = 0u;
    for (var z = -1; z <= 1; z++) {
        for (var y = -1; y <= 1; y++) {
            for (var x = -1; x <= 1; x++) {
                let o = vec3<i32>(x, y, z);
                if (occ_at(b, c + o)) {
                    let m = abs(x) + abs(y) + abs(z);
                    if (weighted && m > 1) {
                        g += vec3<f32>(o) / f32(m);
                    } else {
                        g += vec3<f32>(o);
                    }
                    count += 1.0;
                    if (m == 1) {
                        faces += 1u;
                    }
                }
            }
        }
    }
    let l2 = dot(g, g);
    if (faces == 6u || l2 < 1e-12) {
        return NO_PLANE;
    }
    let n = -g * inverseSqrt(l2);
    var offset = 0.0;
    if ((params.fit & FIT_VOLUME) != 0u) {
        offset = volume_matched_offset(n, count);
    } else {
        offset = (count / 27.0 - 0.5) * 3.0;
    }
    offset = clamp(offset, -0.875, 0.875);
    let k = u32(round(offset * 8.0) + 7.0);
    return encode_octahedral(n) | (k << 12u);
}

// Components of the plane normal below this take the limit of the slab-volume formula along
// them (a derivative in place of a difference quotient), so the formula never divides by a
// vanishing component.
const SLAB_EPS: f32 = 0.05;

// One difference quotient of the slab-volume formula along a step `h`: `(F(t) - F(t - h)) / h`
// of the antiderivative F of order `order` (F_k(x) = max(x, 0)^k / k!), or for h below
// SLAB_EPS its derivative F_{order - 1} at the step's midpoint.
fn ramp(order: u32, x: f32) -> f32 {
    let p = max(x, 0.0);
    switch order {
        case 0u: { return select(0.0, 1.0, x > 0.0); }
        case 1u: { return p; }
        case 2u: { return p * p * 0.5; }
        default: { return p * p * p / 6.0; }
    }
}

// The solid fraction of the unit cube under the half-space `a x + b y + c z <= t` for
// a, b, c >= 0: the third difference of F_3 over the three steps, `Δa Δb Δc F_3 (t) / (a b c)`,
// each difference along a component below SLAB_EPS replaced by the derivative at its
// midpoint (exact in the limit, within O(h^2) below it).
fn slab_fraction(a: vec3<f32>, t: f32) -> f32 {
    var v = 0.0;
    // Inclusion-exclusion over the 8 corners; a small component's corner pair collapses to
    // one evaluation at its midpoint one order lower.
    for (var i = 0u; i < 8u; i++) {
        var x = t;
        var sign = 1.0;
        var order = 3u;
        var skip = false;
        for (var k = 0u; k < 3u; k++) {
            let bit = (i >> k) & 1u;
            if (a[k] < SLAB_EPS) {
                // The derivative: one evaluation at the midpoint, from the corner with bit 0.
                if (bit == 1u) {
                    skip = true;
                }
                x -= 0.5 * a[k];
                order -= 1u;
            } else if (bit == 1u) {
                x -= a[k];
                sign = -sign;
            }
        }
        if (!skip) {
            v += sign * ramp(order, x);
        }
    }
    var denom = 1.0;
    for (var k = 0u; k < 3u; k++) {
        if (a[k] >= SLAB_EPS) {
            denom *= a[k];
        }
    }
    return v / denom;
}

// The offset d (voxels along the unit normal `n` from the voxel centre) at which the
// half-space `n.(x - c) <= d` cuts the 3 x 3 x 3 block `[-1.5, 1.5]^3` about the centre into a
// solid volume of `count` voxels: 8 bisection steps on [-1.5, 1.5] over the exact slab volume.
// In the block's unit-cube coordinates u = (x - c) / 3 + 0.5, mirrored so every component of
// the normal is non-negative, the half-space is `|n|.u <= (d + 1.5 sum |n|) / 3`.
fn volume_matched_offset(n: vec3<f32>, count: f32) -> f32 {
    let a = abs(n);
    let s = a.x + a.y + a.z;
    let want = count / 27.0;
    var lo = -1.5;
    var hi = 1.5;
    for (var i = 0; i < 8; i++) {
        let mid = 0.5 * (lo + hi);
        if (slab_fraction(a, (mid + 1.5 * s) / 3.0) < want) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    return 0.5 * (lo + hi);
}

@compute @workgroup_size(256, 1, 1)
fn main(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let b = li / LANES;
    let t = li % LANES;
    let item = (wg.y * params.groups_x + wg.x) * BRICKS + b;
    // Every invocation reaches the three barriers; an empty lane (past the list) or a brick
    // without a surface does no work between them.
    var entry = 0u;
    var origin = vec3<i32>(0);
    var n = 0u;
    if (item < params.items) {
        let w = attrib_work[item];
        entry = w.x;
        origin = bitcast<vec3<i32>>(w.yzw);
        // The surface voxels: the last layer's prefix plus its own count.
        let u = entry & UNIT_MASK;
        if ((pool_word(u, UNIT_WORDS + 14u) & 0xFFFFu) != 0u) {
            let s = UNIT_WORDS * attr_offset(entry);
            let last = pool_word(u, UNIT_WORDS + 13u) >> 16u;
            n = last + countOneBits(pool_word(u, s + 14u)) + countOneBits(pool_word(u, s + 15u));
        }
    }
    let unit = entry & UNIT_MASK;
    let s = UNIT_WORDS * attr_offset(entry);
    let compute = n > 0u;
    for (var j = t; j < 256u; j += LANES) {
        atomicStore(&cells[b * 256u + j], 0u);
    }
    if (compute && t < 27u) {
        let d = vec3<i32>(i32(t % 3u), i32((t / 3u) % 3u), i32(t / 9u)) - vec3<i32>(1);
        let k = brick_kind(floor_div(origin, 8) + d);
        nb_kind[b * 27u + t] = k.x;
        nb_entry[b * 27u + t] = k.y;
    }
    workgroupBarrier();
    if (compute) {
        for (var j = t; j < 27u * UNIT_WORDS; j += LANES) {
            let k = j / UNIT_WORDS;
            let w = j % UNIT_WORDS;
            var m = 0u;
            let kind = nb_kind[b * 27u + k];
            if (kind == NB_MASK || kind >= NB_MIP) {
                let e = nb_entry[b * 27u + k];
                let u = e & UNIT_MASK;
                m = pool_word(u, w);
                if ((e & TRANSLUCENT_BIT) != 0u) {
                    // A planar brick's translucent unit: its translucent voxels are non-air.
                    m |= pool_word(u, PAYLOAD_WORD + UNIT_WORDS * payload_units(e) + w);
                }
            }
            nb_mask[b * 432u + j] = m;
        }
        // Byte t of the surface mask: layer t / 8, row t % 8 (x in its bits). Its voxels'
        // indices start at the layer's prefix plus the surface bits below the row in the
        // layer.
        let z = t / 8u;
        let lo = pool_word(unit, s + 2u * z);
        let hi = pool_word(unit, s + 2u * z + 1u);
        let byte_at = (t % 8u) * 8u;
        var below = 0u;
        var bits = 0u;
        if (byte_at < 32u) {
            below = countOneBits(lo & ((1u << byte_at) - 1u));
            bits = (lo >> byte_at) & 0xFFu;
        } else {
            below = countOneBits(lo) + countOneBits(hi & ((1u << (byte_at - 32u)) - 1u));
            bits = (hi >> (byte_at - 32u)) & 0xFFu;
        }
        let prefix = pool_word(unit, UNIT_WORDS + 10u + z / 2u);
        var k = ((prefix >> ((z & 1u) * 16u)) & 0xFFFFu) + below;
        while (bits != 0u) {
            let x = firstTrailingBit(bits);
            bits &= bits - 1u;
            let cell = 64u * z + byte_at + x;
            atomicOr(&cells[b * 256u + k / 2u], cell << ((k & 1u) * 16u));
            k += 1u;
        }
    }
    workgroupBarrier();
    // Occupancy word t of the box (cells 32 t to 32 t + 31), set by one invocation each, so
    // no word is shared between invocations.
    if (compute && t < 32u) {
        var word = 0u;
        for (var j = 32u * t; j < min(32u * t + 32u, BOX * BOX * BOX); j++) {
            let c = vec3<i32>(i32(j % BOX), i32((j / BOX) % BOX), i32(j / (BOX * BOX)));
            // The neighbour on each axis: -1 for box cell 0, +1 for cell 9, else the brick.
            let d = select(vec3<i32>(0), vec3<i32>(-1), c < vec3<i32>(1))
                + select(vec3<i32>(0), vec3<i32>(1), c > vec3<i32>(8));
            let k = b * 27u + u32((d.x + 1) + 3 * (d.y + 1) + 9 * (d.z + 1));
            let l = vec3<u32>(c - vec3<i32>(1) - d * 8);
            let i = l.x + 8u * (l.y + 8u * l.z);
            var on = false;
            switch nb_kind[k] {
                case 1u: {
                    on = true;
                }
                case 2u: {
                    on = ((nb_mask[k * UNIT_WORDS + i / 32u] >> (i % 32u)) & 1u) != 0u;
                }
                case 3u: {
                    on = solid(origin + c - vec3<i32>(1));
                }
                case 0u: {}
                default: {
                    // NB_MIP + o: voxel 4 o + l / 2 of the staged mip brick.
                    let o = nb_kind[k] - NB_MIP;
                    let m = (vec3<u32>(o, o >> 1u, o >> 2u) & vec3<u32>(1u)) * 4u + l / 2u;
                    let mi = m.x + 8u * (m.y + 8u * m.z);
                    on = ((nb_mask[k * UNIT_WORDS + mi / 32u] >> (mi % 32u)) & 1u) != 0u;
                }
            }
            word |= select(0u, 1u, on) << (j % 32u);
        }
        occ[b * 32u + t] = word;
    }
    workgroupBarrier();
    if (compute) {
        for (var w = t; 2u * w < n; w += LANES) {
            let pair = atomicLoad(&cells[b * 256u + w]);
            var packed = 0u;
            for (var h = 0u; h < 2u; h++) {
                var a = NO_PLANE;
                if (2u * w + h < n) {
                    let i = (pair >> (16u * h)) & 0xFFFFu;
                    let l = vec3<i32>(i32(i % 8u), i32((i / 8u) % 8u), i32(i / 64u));
                    a = voxel_attr(b, l + vec3<i32>(1));
                }
                packed |= a << (16u * h);
            }
            store_pool_word(unit, s + UNIT_WORDS + w, packed);
        }
    }
}
