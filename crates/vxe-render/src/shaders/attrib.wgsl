// The attribute pass: fills the contour attributes of the bricks in a work list from the pool
// itself. Four bricks a workgroup, 64 invocations a brick: item 4 g + b is brick b of
// workgroup g, and the last workgroup may run partly empty. A brick resolves itself and its 26
// neighbours once (table and record reads), loads their non-air masks (16 words each) into
// workgroup memory, lists its surface voxels in attribute-index order (each invocation scans
// one byte of the surface mask), and sets the occupancy of the brick and a two-voxel shell
// around it (12^3 voxels, two 12-bit rows a word, a brick's row run from one mask word) from
// those masks, a LOD-only chunk's
// neighbour through its level-1 mip brick's, reading voxel by voxel only a brick of more than
// 16 materials holding a translucent voxel; without the majority filter only the brick and
// its one-voxel shell (10^3) are set, all the fit reads. With the majority filter (params.fit
// bit 2) it then sets the filtered occupancy of the brick and a one-voxel shell (10^3, one
// 10-bit row a word, in the staged masks' words, counted a row at a time in bit-sliced adds): a voxel is solid there when at least 14 of the 27 voxels around it,
// itself included, are solid. Then invocation t computes attribute words t, t + 64, ...
// (indices 2 w and 2 w + 1) up to the surface's count and writes each whole; the words past
// it keep the "no plane" the upload wrote. The four bricks share the workgroup's three
// barriers, four with the filter.
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
// whose six face neighbours are all solid, or one whose occupancy gradient vanishes; under the
// majority filter, a voxel whose filtered gradient vanishes.
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
//
// With params.fit bit 2 (`OccupancyFilter::Majority`, ADR-0040) the normal, the solid count
// and the face test read the filtered occupancy instead; the voxel itself is still one of the
// surface mask's. A voxel whose six face neighbours are all solid in the filtered occupancy
// (the filter buries it: a one-voxel groove's bottom) takes the unfiltered normal and the
// offset's clamp, 7/8, so it hits as its cube face shaded with the surface's normal; one whose
// filtered gradient vanishes (an empty filtered neighbourhood: a free one-voxel pillar's upper
// voxels) has no plane.

struct Params {
    window_min: vec3<i32>,
    units_per_shard: u32,
    window_size: vec3<i32>,
    items: u32,
    groups_x: u32,
    // Bit 0: the weighted gradient; bit 1: the volume-matched offset; bit 2: the majority
    // filter.
    fit: u32,
    _pad1: u32,
    _pad2: u32,
};

const FIT_WEIGHTED: u32 = 1u;
const FIT_VOLUME: u32 = 2u;
const FIT_MAJORITY: u32 = 4u;

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
const CLASS_UNIFORM: u32 = 6u;
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
// The fit's box: the brick and a one-voxel shell, 10 voxels to an edge, its cell c at
// occupancy cell c + 1. The occupancy box: the brick and a two-voxel shell, 12 to an edge.
const BOX: u32 = 10u;
const OCC: u32 = 12u;

// Per brick of the workgroup (brick b's at b times the per-brick length), 13 024 B in all:
// its occupancy box as 144 rows of 12 bits, two a word, row y + 12 z in half (row % 2) of
// word row / 2, x in bit x (each word set by one invocation); it and its 26 neighbours,
// index (dx + 1) + 3 (dy + 1) + 9 (dz + 1), how each is read (NB_AIR, NB_SOLID, NB_MASK or
// NB_MIP with its entry, NB_VOXELS) and the NB_MASK and NB_MIP ones' non-air masks, whose
// words, once the occupancy box is set, hold its filtered box instead: 100 rows of 10 bits,
// row y + 10 z in word row; and its surface voxels' cells in attribute-index order, two u16
// a word.
var<workgroup> occ: array<u32, 288>;
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

// Units of a Full entry's payload: 0, 1, 2 or 4 planes for classes 1 to 4, 16 for Wide, none
// for a uniform band brick's slot (class 6, laid out as class 1).
fn payload_units(entry: u32) -> u32 {
    let cls = (entry >> CLASS_SHIFT) & 7u;
    return select(select(select(cls - 1u, 4u, cls == 4u), 16u, cls == CLASS_WIDE), 0u, cls == CLASS_UNIFORM);
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

// Layer `z`'s prefix (`z` in 0..8) in a packed prefix of two header words `lo`, `hi`
// (`gpu_world::pack_prefix`): layers 1 to 4 in `lo` at bits 0, 7, 15 and 23 (7, 8, 8 and 9
// bits wide), layers 5 to 7 in `hi` at bits 0, 9 and 18 (9 bits each); layer 0's is 0.
fn layer_prefix(lo: u32, hi: u32, z: u32) -> u32 {
    if (z == 0u) {
        return 0u;
    }
    if (z >= 5u) {
        return (hi >> (9u * (z - 5u))) & 0x1FFu;
    }
    let shift = select(8u * z - 9u, 0u, z == 1u);
    let width = select(select(9u, 8u, z < 4u), 7u, z == 1u);
    return (lo >> shift) & ((1u << width) - 1u);
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

// Occupancy of fit-box cell `c` (each component in 0..10) of the workgroup's brick `b`.
fn occ_at(b: u32, c: vec3<i32>) -> bool {
    return ((occ_row(b, u32(c.y + 1) + OCC * u32(c.z + 1)) >> u32(c.x + 1)) & 1u) != 0u;
}

// Row `r` (y + 12 z) of brick `b`'s occupancy box, 12 bits.
fn occ_row(b: u32, r: u32) -> u32 {
    return (occ[b * 72u + r / 2u] >> ((r & 1u) * 16u)) & 0xFFFu;
}

// Filtered occupancy of fit-box cell `c` (each component in 0..10) of brick `b`.
fn occf_at(b: u32, c: vec3<i32>) -> bool {
    let r = nb_mask[b * 432u + u32(c.y) + BOX * u32(c.z)];
    return ((r >> u32(c.x)) & 1u) != 0u;
}

// The fit's gather over the 27 voxels around fit-box cell `c` of brick `b`, of the filtered
// occupancy when `filtered` is set: the gradient's sum (each solid neighbour's offset, at
// 1 / |o|^2 under the weighted fit), the solid count and the solid face neighbours.
struct Gather {
    g: vec3<f32>,
    count: f32,
    faces: u32,
};

fn gather(b: u32, c: vec3<i32>, filtered: bool) -> Gather {
    let weighted = (params.fit & FIT_WEIGHTED) != 0u;
    var r = Gather(vec3<f32>(0.0), 0.0, 0u);
    for (var z = -1; z <= 1; z++) {
        for (var y = -1; y <= 1; y++) {
            for (var x = -1; x <= 1; x++) {
                let o = vec3<i32>(x, y, z);
                let on = select(occ_at(b, c + o), occf_at(b, c + o), filtered);
                if (on) {
                    let m = abs(x) + abs(y) + abs(z);
                    if (weighted && m > 1) {
                        r.g += vec3<f32>(o) / f32(m);
                    } else {
                        r.g += vec3<f32>(o);
                    }
                    r.count += 1.0;
                    if (m == 1) {
                        r.faces += 1u;
                    }
                }
            }
        }
    }
    return r;
}

// The attribute of the voxel at box cell `c` (each component in 1..9) of brick `b`.
fn voxel_attr(b: u32, c: vec3<i32>) -> u32 {
    if (!occ_at(b, c)) {
        return NO_PLANE;
    }
    let majority = (params.fit & FIT_MAJORITY) != 0u;
    var f = gather(b, c, majority);
    // Buried by the filter: the unfiltered normal at the clamp.
    let buried = majority && f.faces == 6u;
    if (buried) {
        f = gather(b, c, false);
    }
    let l2 = dot(f.g, f.g);
    if (f.faces == 6u || l2 < 1e-12) {
        return NO_PLANE;
    }
    let n = -f.g * inverseSqrt(l2);
    var offset = 0.0;
    if (buried) {
        offset = 0.875;
    } else if ((params.fit & FIT_VOLUME) != 0u) {
        offset = volume_matched_offset(n, f.count);
    } else {
        offset = (f.count / 27.0 - 0.5) * 3.0;
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

// The occupancy of box cells x0..x1 (all in one neighbour along x) of row (ry, rz) of brick
// `b` at `origin`, as bits x0..x1: a uniform neighbour's run at once, a Full brick's from one
// mask word (a row of a brick is one byte of its mask), and a mip brick's or a voxel-read
// brick's cell by cell.
fn occ_run(b: u32, origin: vec3<i32>, x0: i32, x1: i32, ry: i32, rz: i32) -> u32 {
    if (x1 <= x0) {
        return 0u;
    }
    let ones = ((1u << u32(x1 - x0)) - 1u) << u32(x0);
    // The neighbour on each axis: -1 for box cells 0 and 1, +1 for 10 and 11, else the brick.
    let c0 = vec3<i32>(x0, ry, rz);
    let d = select(vec3<i32>(0), vec3<i32>(-1), c0 < vec3<i32>(2))
        + select(vec3<i32>(0), vec3<i32>(1), c0 > vec3<i32>(9));
    let k = b * 27u + u32((d.x + 1) + 3 * (d.y + 1) + 9 * (d.z + 1));
    let kind = nb_kind[k];
    if (kind == NB_AIR) {
        return 0u;
    }
    if (kind == NB_SOLID) {
        return ones;
    }
    let l0 = vec3<u32>(c0 - vec3<i32>(2) - d * 8);
    if (kind == NB_MASK) {
        let i = l0.x + 8u * (l0.y + 8u * l0.z);
        let m = nb_mask[k * UNIT_WORDS + i / 32u] >> (i % 32u);
        return (m << u32(x0)) & ones;
    }
    var bits = 0u;
    for (var x = x0; x < x1; x++) {
        let l = vec3<u32>(vec3<i32>(x, ry, rz) - vec3<i32>(2) - d * 8);
        var on = false;
        if (kind == NB_VOXELS) {
            on = solid(origin + vec3<i32>(x, ry, rz) - vec3<i32>(2));
        } else {
            // NB_MIP + o: voxel 4 o + l / 2 of the staged mip brick.
            let o = kind - NB_MIP;
            let m = (vec3<u32>(o, o >> 1u, o >> 2u) & vec3<u32>(1u)) * 4u + l / 2u;
            let mi = m.x + 8u * (m.y + 8u * m.z);
            on = ((nb_mask[k * UNIT_WORDS + mi / 32u] >> (mi % 32u)) & 1u) != 0u;
        }
        bits |= select(0u, 1u, on) << u32(x);
    }
    return bits;
}

// A bit-sliced full adder over 32 lanes: the sum bits of `a + b + c` in x, the carries in y.
fn full_add(a: u32, b: u32, c: u32) -> vec2<u32> {
    let p = a ^ b;
    return vec2<u32>(p ^ c, (a & b) | (p & c));
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
            let last = layer_prefix(pool_word(u, UNIT_WORDS + 10u), pool_word(u, UNIT_WORDS + 11u), 7u);
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
        var k = layer_prefix(pool_word(unit, UNIT_WORDS + 10u), pool_word(unit, UNIT_WORDS + 11u), z)
            + below;
        while (bits != 0u) {
            let x = firstTrailingBit(bits);
            bits &= bits - 1u;
            let cell = 64u * z + byte_at + x;
            atomicOr(&cells[b * 256u + k / 2u], cell << ((k & 1u) * 16u));
            k += 1u;
        }
    }
    workgroupBarrier();
    // Occupancy words t and t + 64 of the box (72 words, two rows of 12 cells each), each set
    // by one invocation, so no word is shared between invocations. Without the filter the
    // outer shell's rows and cells stay clear: the fit reads only the inner 10^3.
    let majority = (params.fit & FIT_MAJORITY) != 0u;
    let lo = select(1, 0, majority);
    let hi = select(11, 12, majority);
    if (compute) {
        for (var w = t; w < OCC * OCC / 2u; w += LANES) {
            var word = 0u;
            for (var h = 0u; h < 2u; h++) {
                let r = 2u * w + h;
                let ry = i32(r % OCC);
                let rz = i32(r / OCC);
                var bits = 0u;
                if (ry >= lo && ry < hi && rz >= lo && rz < hi) {
                    // The row's three runs along x: the -x neighbour's cells 0 and 1, the
                    // brick's 2..10, the +x neighbour's 10 and 11.
                    bits = occ_run(b, origin, lo, 2, ry, rz) | occ_run(b, origin, 2, 10, ry, rz)
                        | occ_run(b, origin, 10, hi, ry, rz);
                }
                word |= bits << (16u * h);
            }
            occ[b * 72u + w] = word;
        }
    }
    workgroupBarrier();
    if (majority) {
        // Filtered rows t and t + 64 of the fit box (100 rows of 10 cells), all ten cells of a
        // row at once in bit-sliced arithmetic: cell (x, y, z) sums the occupancy box's cells
        // x..x + 2 of the nine rows (y + dy, z + dz), dy and dz in 0..3. The nine rows' column
        // sums (0..9, four bit planes) go through full adders, then the three columns' sums
        // through two ripple adders (0..27, five bit planes), compared with 14.
        if (compute) {
            for (var r = t; r < BOX * BOX; r += LANES) {
                let ry = r % BOX;
                let rz = r / BOX;
                var rows: array<u32, 9>;
                for (var j = 0u; j < 9u; j++) {
                    rows[j] = occ_row(b, ry + j % 3u + OCC * (rz + j / 3u));
                }
                let a = full_add(rows[0], rows[1], rows[2]);
                let c = full_add(rows[3], rows[4], rows[5]);
                let e = full_add(rows[6], rows[7], rows[8]);
                let ones = full_add(a.x, c.x, e.x);
                let twos = full_add(a.y, c.y, e.y);
                // Column sums: v0 + 2 v1 + 4 v2 + 8 v3.
                let v0 = ones.x;
                let v1 = ones.y ^ twos.x;
                let c4 = ones.y & twos.x;
                let v2 = twos.y ^ c4;
                let v3 = twos.y & c4;
                // Cell x sums columns x, x + 1 and x + 2: the planes shifted by 0, 1 and 2,
                // added in two four-plane ripple adds into five planes (at most 27).
                let p0 = full_add(v0, v0 >> 1u, 0u);
                let p1 = full_add(v1, v1 >> 1u, p0.y);
                let p2 = full_add(v2, v2 >> 1u, p1.y);
                let p3 = full_add(v3, v3 >> 1u, p2.y);
                let q0 = full_add(p0.x, v0 >> 2u, 0u);
                let q1 = full_add(p1.x, v1 >> 2u, q0.y);
                let q2 = full_add(p2.x, v2 >> 2u, q1.y);
                let q3 = full_add(p3.x, v3 >> 2u, q2.y);
                let s4 = p3.y ^ q3.y;
                // At least 14 of 27: 16 and over, or 14 and 15.
                let word = (s4 | (q3.x & q2.x & q1.x)) & 0x3FFu;
                // The neighbours' staged masks are read only while the occupancy box is set:
                // the filtered rows take their words.
                nb_mask[b * 432u + r] = word;
            }
        }
        workgroupBarrier();
    }
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

// ---- The field pass (`field.rs`, ADR-0041) ----
//
// Fills the band slots of a work list of bricks of any kind: per band voxel its count, the
// solid voxels among the 27 around it with itself (solid is not air, as for the fit). A band
// slot is its band mask (one unit, the mask's layout: bit i % 32 of word i / 32 for voxel
// i = x + 8 y + 64 z), which the upload wrote, then ceil(n / 64) units of u8 counts for its n
// band voxels in band-index order: the k-th band voxel's in byte k % 4 of word k / 4. Four
// bricks a workgroup, 64 invocations a brick, as `main`; the occupancy is `main`'s 10^3 box
// without the filter. Every count unit is written whole, past the n-th byte as 0.
//
// A count byte holds the count in its low five bits, SMOOTH_COUNT_BIT on a solid voxel of a
// smooth material and CRISP_COUNT_BIT on a solid voxel of a crisp one (`MaterialFlags::CRISP`
// on a row that is not translucent): a crisp voxel is solid in every count. The material is
// read only in a brick whose palette holds a crisp row. The pass also sets BAND_CUBES_BIT in
// the brick's header band word (HEADER_BAND_WORD of its own slot) when no voxel of the brick
// and its one-voxel shell is a smooth solid band voxel (a solid voxel with air among its 26
// neighbours), and clears it otherwise: no dual cell with a corner in such a brick has a
// smooth solid band corner, so `march.wgsl`'s field walk crosses it as cubes. The brick's own
// band voxels carry the answer in their count bytes. A shell voxel's band test reads the full
// 12^3 occupancy box; its material is known from its brick's palette when that holds only
// smooth or only crisp solid materials, read when it holds both, and taken as smooth in a
// brick read voxel by voxel or a LOD-only chunk's mip brick. When neither the brick nor its
// 26 neighbours hold a smooth solid material by their palettes no shell voxel is read.
//
// Bindings, group 0: 0, 1..4, 5 and 6 as `main`'s, then:
//  11  storage  field_work    [n][4 x u32]: the band slot's first unit, the brick's origin in
//                             voxels (x, y, z as i32 bits)
//  12  storage  materials     as in march.wgsl: [id][4 x vec4], word 7 of a row the flag bits

@group(0) @binding(11) var<storage, read> field_work: array<vec4<u32>>;
@group(0) @binding(12) var<storage, read> materials: array<vec4<f32>>;

const MATERIAL_TRANSLUCENT: u32 = 2u;
const MATERIAL_CRISP: u32 = 4u;
const HEADER_BAND_WORD: u32 = 15u;
const BAND_CUBES_BIT: u32 = 0x40000000u;
const SMOOTH_COUNT_BIT: u32 = 0x40u;
const CRISP_COUNT_BIT: u32 = 0x80u;

// The band mask words of the workgroup's four bricks, 16 each.
var<workgroup> band_words: array<atomic<u32>, 64>;
// Per brick of the workgroup: bit 0, a smooth solid material in its palette or a neighbour's;
// bit 1, a crisp material in its own palette; bit 2, a smooth solid band voxel in it or its
// one-voxel shell.
var<workgroup> brick_near: array<atomic<u32>, 4>;
// `brick_surfaces` of each brick's 27 neighbours, index as `nb_kind`.
var<workgroup> nb_surf: array<u32, 108>;

// Whether material `m` is drawn as cubes: a crisp row that is not translucent.
fn material_crisp(m: u32) -> bool {
    let f = bitcast<u32>(materials[m * 4u + 1u].w);
    return (f & (MATERIAL_CRISP | MATERIAL_TRANSLUCENT)) == MATERIAL_CRISP;
}

// The solid materials brick (`kind`, `entry`) holds by its palette: bit 0 a smooth one, bit 1
// a crisp one. A brick of more than 16 materials or read voxel by voxel holds both; a LOD-only
// chunk's brick that is not air holds a smooth one (its mip cells are cubes for every
// material, and a band voxel beside it counts it as solid either way).
fn brick_surfaces(kind: u32, entry: u32) -> u32 {
    if (kind == NB_AIR) {
        return 0u;
    }
    if ((entry & FULL_BIT) == 0u) {
        return select(1u, 2u, material_crisp(entry & 0xFFFFu));
    }
    if (kind >= NB_MIP) {
        return 1u;
    }
    let cls = (entry >> CLASS_SHIFT) & 7u;
    if (kind == NB_VOXELS || cls == CLASS_WIDE) {
        return 3u;
    }
    let unit = entry & UNIT_MASK;
    var out = 0u;
    for (var j = 0u; j < (1u << payload_units(entry)); j++) {
        let m = (pool_word(unit, UNIT_WORDS + j / 2u) >> ((j & 1u) * 16u)) & 0xFFFFu;
        if (m != 0u) {
            out |= select(1u, 2u, material_crisp(m));
        }
    }
    return out;
}

// Whether voxel `i`, a solid voxel of the brick with record entry `entry`, is crisp.
fn voxel_crisp(entry: u32, i: u32) -> bool {
    if ((entry & FULL_BIT) == 0u) {
        return material_crisp(entry & 0xFFFFu);
    }
    let unit = entry & UNIT_MASK;
    let cls = (entry >> CLASS_SHIFT) & 7u;
    if (cls == CLASS_WIDE) {
        let word = pool_word(unit, PAYLOAD_WORD + i / 2u);
        return material_crisp((word >> ((i & 1u) * 16u)) & 0xFFFFu);
    }
    var index = 0u;
    for (var k = 0u; k < payload_units(entry); k++) {
        let w = pool_word(unit, PAYLOAD_WORD + UNIT_WORDS * k + i / 32u);
        index |= ((w >> (i % 32u)) & 1u) << k;
    }
    let pair = pool_word(unit, UNIT_WORDS + index / 2u);
    return material_crisp((pair >> ((index & 1u) * 16u)) & 0xFFFFu);
}

@compute @workgroup_size(256, 1, 1)
fn field_main(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let b = li / LANES;
    let t = li % LANES;
    let item = (wg.y * params.groups_x + wg.x) * BRICKS + b;
    let listed = item < params.items;
    var unit = 0u;
    var origin = vec3<i32>(0);
    if (listed) {
        let w = field_work[item];
        unit = w.x;
        origin = bitcast<vec3<i32>>(w.yzw);
    }
    // The packed counts of brick b: words b * 128 + k / 4 of `cells`.
    atomicStore(&cells[b * 128u + t], 0u);
    atomicStore(&cells[b * 128u + 64u + t], 0u);
    if (t < 16u) {
        var m = 0u;
        if (listed) {
            m = pool_word(unit, t);
        }
        atomicStore(&band_words[b * 16u + t], m);
    }
    if (t == 0u) {
        atomicStore(&brick_near[b], 0u);
    }
    if (listed && t < 27u) {
        let d = vec3<i32>(i32(t % 3u), i32((t / 3u) % 3u), i32(t / 9u)) - vec3<i32>(1);
        let k = brick_kind(floor_div(origin, 8) + d);
        nb_kind[b * 27u + t] = k.x;
        nb_entry[b * 27u + t] = k.y;
    }
    workgroupBarrier();
    if (listed && t < 27u) {
        let s = brick_surfaces(nb_kind[b * 27u + t], nb_entry[b * 27u + t]);
        nb_surf[b * 27u + t] = s;
        atomicOr(&brick_near[b], (s & 1u) | select(0u, s & 2u, t == 13u));
    }
    if (listed) {
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
                    m |= pool_word(u, PAYLOAD_WORD + UNIT_WORDS * payload_units(e) + w);
                }
            }
            nb_mask[b * 432u + j] = m;
        }
    }
    workgroupBarrier();
    // The 12^3 occupancy box, as `main` without the filter: the counts read its 10^3 core,
    // the shell voxels' band test all of it.
    if (listed) {
        for (var w = t; w < OCC * OCC / 2u; w += LANES) {
            var word = 0u;
            for (var h = 0u; h < 2u; h++) {
                let r = 2u * w + h;
                let ry = i32(r % OCC);
                let rz = i32(r / OCC);
                var bits = 0u;
                bits = occ_run(b, origin, 0, 2, ry, rz) | occ_run(b, origin, 2, 10, ry, rz)
                    | occ_run(b, origin, 10, 12, ry, rz);
                word |= bits << (16u * h);
            }
            occ[b * 72u + w] = word;
        }
    }
    workgroupBarrier();
    // Invocation t counts row (y, z) = (t % 8, t / 8) of the brick: voxel l is occupancy
    // cell l + 2, its 27 neighbours cells l + 1 to l + 3 on each axis. The row's voxels are
    // bits 8 (y % 4) .. 8 (y % 4) + 8 of mask word y / 4 + 2 z; its first band voxel's index is
    // the band voxels of the words before and of the bits below in its word.
    if (listed) {
        let y = t % 8u;
        let z = t / 8u;
        var rows: array<u32, 9>;
        for (var j = 0u; j < 9u; j++) {
            rows[j] = occ_row(b, (y + 1u + j % 3u) + OCC * (z + 1u + j / 3u));
        }
        let mw = y / 4u + 2u * z;
        let shift = 8u * (y % 4u);
        let word = atomicLoad(&band_words[b * 16u + mw]);
        var k = countOneBits(word & ((1u << shift) - 1u));
        for (var j = 0u; j < mw; j++) {
            k += countOneBits(atomicLoad(&band_words[b * 16u + j]));
        }
        let row = (word >> shift) & 0xFFu;
        let crisp_here = (atomicLoad(&brick_near[b]) & 2u) != 0u;
        let entry = nb_entry[b * 27u + 13u];
        for (var x = 0u; x < 8u; x++) {
            if (((row >> x) & 1u) == 0u) {
                continue;
            }
            var n = 0u;
            for (var j = 0u; j < 9u; j++) {
                n += countOneBits((rows[j] >> (x + 1u)) & 7u);
            }
            // The voxel itself: bit x + 2 of the centre row.
            if (((rows[4] >> (x + 2u)) & 1u) != 0u) {
                let i = x + 8u * y + 64u * z;
                let crisp = crisp_here && voxel_crisp(entry, i);
                n |= select(SMOOTH_COUNT_BIT, CRISP_COUNT_BIT, crisp);
                if (!crisp) {
                    atomicOr(&brick_near[b], 4u);
                }
            }
            atomicOr(&cells[b * 128u + k / 4u], n << (8u * (k % 4u)));
            k += 1u;
        }
        // The shell's smooth solid band voxels, box cells 1..10 outside the brick's 2..9.
        if ((atomicLoad(&brick_near[b]) & 1u) != 0u) {
            for (var j = t; j < 1000u; j += LANES) {
                if ((atomicLoad(&brick_near[b]) & 4u) != 0u) {
                    break;
                }
                let c = vec3<u32>(j % 10u, (j / 10u) % 10u, j / 100u) + vec3<u32>(1u);
                if (all(c >= vec3<u32>(2u)) && all(c <= vec3<u32>(9u))) {
                    continue;
                }
                if (((occ_row(b, c.y + OCC * c.z) >> c.x) & 1u) == 0u) {
                    continue;
                }
                var n = 0u;
                for (var q = 0u; q < 9u; q++) {
                    let r = occ_row(b, (c.y - 1u + q % 3u) + OCC * (c.z - 1u + q / 3u));
                    n += countOneBits((r >> (c.x - 1u)) & 7u);
                }
                if (n == 27u) {
                    continue;
                }
                let d = select(vec3<i32>(0), vec3<i32>(-1), c < vec3<u32>(2u))
                    + select(vec3<i32>(0), vec3<i32>(1), c > vec3<u32>(9u));
                let nk = b * 27u + u32((d.x + 1) + 3 * (d.y + 1) + 9 * (d.z + 1));
                let sf = nb_surf[nk];
                var is_smooth = (sf & 1u) != 0u;
                if (sf == 3u && nb_kind[nk] == NB_MASK) {
                    let l = vec3<u32>(vec3<i32>(c) - vec3<i32>(2) - d * 8);
                    is_smooth = !voxel_crisp(nb_entry[nk], l.x + 8u * (l.y + 8u * l.z));
                }
                if (is_smooth) {
                    atomicOr(&brick_near[b], 4u);
                }
            }
        }
    }
    workgroupBarrier();
    if (listed) {
        var n = 0u;
        for (var j = 0u; j < 16u; j++) {
            n += countOneBits(atomicLoad(&band_words[b * 16u + j]));
        }
        let words = UNIT_WORDS * ((n + 63u) / 64u);
        for (var j = t; j < words; j += LANES) {
            store_pool_word(unit, UNIT_WORDS + j, atomicLoad(&cells[b * 128u + j]));
        }
        let e = nb_entry[b * 27u + 13u];
        if (t == 0u && (e & FULL_BIT) != 0u) {
            let u = e & UNIT_MASK;
            let h = pool_word(u, UNIT_WORDS + HEADER_BAND_WORD) & ~BAND_CUBES_BIT;
            let cubes = (atomicLoad(&brick_near[b]) & 4u) == 0u;
            store_pool_word(u, UNIT_WORDS + HEADER_BAND_WORD, h | select(0u, BAND_CUBES_BIT, cubes));
        }
    }
}
