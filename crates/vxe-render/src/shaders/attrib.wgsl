// The attribute pass: fills the contour attributes of the bricks in a work list from the pool
// itself. One workgroup per brick: it loads the occupancy of the brick and a one-voxel shell
// around it (10^3 voxels) into workgroup memory, then each invocation writes one pool word,
// the attributes of two voxels.
//
// Bindings, group 0:
//   0  uniform  params         mirror window, slots per shard, work-list length, the
//                              dispatch's workgroups along x
//   1  storage  brick_pool0    pool shard 0, read-write; 2, 3 and 4 are shards 1 to 3. Same
//                              layout as in march.wgsl; this pass writes only attribute slots
//   5  storage  chunk_records  as in march.wgsl
//   6  storage  chunk_table    as in march.wgsl
//   7  storage  attrib_work    [n][4 x u32]: brick slot (in an attribute page), brick origin
//                              in voxels (x, y, z as i32 bits)
//
// The attribute of brick slot s in attribute page p = s - s % 4096 lives in the two slots
// p + 1364 + 2 (s - p) and the one after: 512 u16, two per word, x fastest. A voxel's u16 is
// bits 0..6 octahedral u, 6..12 octahedral v, 12..16 the plane offset code k in 0..=14 for
// (k - 7) / 8 voxel along the normal from the voxel centre; 0xFFFF is "no plane": air, a voxel
// whose six face neighbours are all solid, or one whose occupancy gradient vanishes.
//
// The normal is the Gradient27 normal of occupancy (central differences over the 3 x 3 x 3
// voxels around the voxel), read through the chunk table: neighbours across brick and chunk
// boundaries count, a voxel in an absent chunk is air, one in a LOD-only chunk its level-1 mip
// cell. The offset is (f - 0.5) * 3 voxels, f the solid fraction of the 27 voxels, clamped to
// [-7/8, 7/8] and rounded to the nearest 1/8.

struct Params {
    window_min: vec3<i32>,
    slots_per_shard: u32,
    window_size: vec3<i32>,
    items: u32,
    groups_x: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

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
const WIDE_BIT: u32 = 0x40000000u;
// Bit 29 of an entry flags a brick holding a translucent voxel; a slot takes bits 0..29.
const SLOT_MASK: u32 = 0x1FFFFFFFu;
const EMPTY_BIT: u32 = 0x20000000u;
const RECORD_MASK: u32 = 0x1FFFFFFFu;
const SLOT_WORDS: u32 = 128u;
const RECORD_WORDS: u32 = 656u;
const ENTRY_WORD: u32 = 16u;
const PAGE_SLOTS: u32 = 4096u;
const ATTR_PAGE_BRICK_SLOTS: u32 = 1364u;
const NO_PLANE: u32 = 0xFFFFu;
// Invocations per workgroup, one per pool word of a brick's attribute; must match
// `attrib::WORKGROUP`.
const WORKGROUP: u32 = 256u;
// The occupancy box: the brick and a one-voxel shell, 10 voxels to an edge.
const BOX: u32 = 10u;

var<workgroup> occ: array<u32, 1000>;

fn shard_index(slot: u32, word: u32) -> vec2<u32> {
    let shard = slot / params.slots_per_shard;
    return vec2<u32>(shard, (slot - shard * params.slots_per_shard) * SLOT_WORDS + word);
}

fn pool_word(slot: u32, word: u32) -> u32 {
    let a = shard_index(slot, word);
    switch a.x {
        case 0u: { return brick_pool0[a.y]; }
        case 1u: { return brick_pool1[a.y]; }
        case 2u: { return brick_pool2[a.y]; }
        default: { return brick_pool3[a.y]; }
    }
}

fn store_pool_word(slot: u32, word: u32, value: u32) {
    let a = shard_index(slot, word);
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
    let slot = entry & SLOT_MASK;
    let l = c - b * 8;
    let i = u32(l.x + 8 * (l.y + 8 * l.z));
    if ((entry & WIDE_BIT) != 0u) {
        let word = pool_word(slot, i / 2u);
        return ((word >> ((i & 1u) * 16u)) & 0xFFFFu) != 0u;
    }
    // A narrow brick: palette index 0 is air in every palette.
    let word = pool_word(slot, i / 4u);
    return ((word >> ((i & 3u) * 8u)) & 0xFFu) != 0u;
}

// Octahedral encoding of the unit vector `n` into 6 + 6 bits.
fn encode_octahedral(n: vec3<f32>) -> u32 {
    var p = n.xy / (abs(n.x) + abs(n.y) + abs(n.z));
    if (n.z < 0.0) {
        p = (1.0 - abs(p.yx)) * select(vec2<f32>(-1.0), vec2<f32>(1.0), p >= vec2<f32>(0.0));
    }
    let q = vec2<u32>(round(clamp(p * 0.5 + 0.5, vec2<f32>(0.0), vec2<f32>(1.0)) * 63.0));
    return q.x | (q.y << 6u);
}

// Occupancy of box cell `c` (each component in 0..10), from workgroup memory.
fn occ_at(c: vec3<i32>) -> bool {
    return occ[u32(c.x) + BOX * (u32(c.y) + BOX * u32(c.z))] != 0u;
}

// The attribute of the voxel at box cell `c` (each component in 1..9).
fn voxel_attr(c: vec3<i32>) -> u32 {
    if (!occ_at(c)) {
        return NO_PLANE;
    }
    var g = vec3<f32>(0.0);
    var count = 0.0;
    var faces = 0u;
    for (var z = -1; z <= 1; z++) {
        for (var y = -1; y <= 1; y++) {
            for (var x = -1; x <= 1; x++) {
                let o = vec3<i32>(x, y, z);
                if (occ_at(c + o)) {
                    g += vec3<f32>(o);
                    count += 1.0;
                    if (abs(x) + abs(y) + abs(z) == 1) {
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
    let f = count / 27.0;
    let offset = clamp((f - 0.5) * 3.0, -0.875, 0.875);
    let k = u32(round(offset * 8.0) + 7.0);
    return encode_octahedral(n) | (k << 12u);
}

@compute @workgroup_size(256, 1, 1)
fn main(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let item = wg.y * params.groups_x + wg.x;
    if (item >= params.items) {
        return;
    }
    let w = attrib_work[item];
    let slot = w.x;
    let origin = bitcast<vec3<i32>>(w.yzw);
    for (var j = li; j < BOX * BOX * BOX; j += WORKGROUP) {
        let c = vec3<i32>(i32(j % BOX), i32((j / BOX) % BOX), i32(j / (BOX * BOX)));
        occ[j] = select(0u, 1u, solid(origin + c - vec3<i32>(1)));
    }
    workgroupBarrier();
    let page = slot - slot % PAGE_SLOTS;
    let attr_slot = page + ATTR_PAGE_BRICK_SLOTS + 2u * (slot - page);
    var packed = 0u;
    for (var h = 0u; h < 2u; h++) {
        let i = 2u * li + h;
        let l = vec3<i32>(i32(i % 8u), i32((i / 8u) % 8u), i32(i / 64u));
        packed |= voxel_attr(l + vec3<i32>(1)) << (16u * h);
    }
    store_pool_word(attr_slot, li, packed);
}
