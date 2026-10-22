// The attribute pass: fills the contour attributes of the bricks in a work list from the pool
// itself. One workgroup per brick: it resolves the brick and its 26 neighbours once (table and
// record reads), loads their non-air masks (16 words each) into workgroup memory, and fills
// the occupancy of the brick and a one-voxel shell around it (10^3 voxels) from those masks,
// reading voxel by voxel only a neighbour in a LOD-only chunk or a brick of more than 16
// materials holding a translucent voxel. Each invocation then computes the attributes of the
// two voxels it holds that lie on the brick's surface mask, nothing for the others, into a
// workgroup copy of the attribute words, which the invocations write out one word each.
//
// Bindings, group 0:
//   0  uniform  params         mirror window, units per shard, work-list length, the
//                              dispatch's workgroups along x
//   1  storage  brick_pool0    pool shard 0, read-write; 2, 3 and 4 are shards 1 to 3. Same
//                              layout as in march.wgsl; this pass writes only attribute units
//   5  storage  chunk_records  as in march.wgsl
//   6  storage  chunk_table    as in march.wgsl
//   7  storage  attrib_work    [n][4 x u32]: the brick's record entry less bit 31 (its slot's
//                              first unit, class and translucent flag), brick origin in
//                              voxels (x, y, z as i32 bits). Bit 31 of the first word marks a
//                              clear item: the brick's attribute is written all "no plane" and
//                              nothing is computed
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
// cell. The offset is (f - 0.5) * 3 voxels, f the solid fraction of the 27 voxels, clamped to
// [-7/8, 7/8] and rounded to the nearest 1/8.

struct Params {
    window_min: vec3<i32>,
    units_per_shard: u32,
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
// Bit 31 of a work item's slot word: a clear item (`attrib::CLEAR_BIT`).
const CLEAR_BIT: u32 = 0x80000000u;
// Invocations per workgroup, two voxels each and one attribute word each at most; must match
// `attrib::WORKGROUP`.
const WORKGROUP: u32 = 256u;
// The occupancy box: the brick and a one-voxel shell, 10 voxels to an edge.
const BOX: u32 = 10u;

var<workgroup> occ: array<u32, 1000>;
// The brick and its 26 neighbours, index (dx + 1) + 3 (dy + 1) + 9 (dz + 1): how each is read
// (NB_AIR, NB_SOLID, NB_MASK with its entry, NB_VOXELS), and the NB_MASK ones' non-air masks.
var<workgroup> nb_kind: array<u32, 27>;
var<workgroup> nb_entry: array<u32, 27>;
var<workgroup> nb_mask: array<u32, 432>;
// The brick's attribute words as computed, NO_PLANE where no surface voxel lands.
var<workgroup> words: array<atomic<u32>, 256>;

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

// How the brick at world brick coordinate `bw` is read, and its entry: NB_AIR outside the
// window, in an absent or empty chunk or for uniform air; NB_SOLID for a uniform brick of any
// other material; NB_MASK for a Full brick of a full chunk, read through its masks; NB_VOXELS
// for a LOD-only chunk's (read at level 1 by `solid`) or a Wide brick holding a translucent
// voxel.
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
    if ((t & LOD_BIT) != 0u) {
        return vec2<u32>(NB_VOXELS, 0u);
    }
    let b = bw - chunk * 8;
    let entry = chunk_records[
        (t & RECORD_MASK) * RECORD_WORDS + ENTRY_WORD + u32(b.x + 8 * (b.y + 8 * b.z))
    ];
    if ((entry & FULL_BIT) == 0u) {
        return vec2<u32>(select(NB_AIR, NB_SOLID, (entry & 0xFFFFu) != 0u), entry);
    }
    let cls = (entry >> CLASS_SHIFT) & 7u;
    if ((entry & TRANSLUCENT_BIT) != 0u && cls == CLASS_WIDE) {
        return vec2<u32>(NB_VOXELS, entry);
    }
    return vec2<u32>(NB_MASK, entry);
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
    let clear = (w.x & CLEAR_BIT) != 0u;
    let entry = w.x & ~CLEAR_BIT;
    let origin = bitcast<vec3<i32>>(w.yzw);
    let unit = entry & UNIT_MASK;
    // Attribute words of the brick; 0 for a brick without a surface.
    let n = (pool_word(unit, UNIT_WORDS + 14u) & 0xFFFFu) * UNIT_WORDS;
    let s = UNIT_WORDS * attr_offset(entry);
    let compute = !clear && n > 0u;
    // The barriers below stay in uniform control flow; the work between them is guarded, so
    // a clear item, or a brick without a surface, computes nothing.
    if (compute && li < 27u) {
        let d = vec3<i32>(i32(li % 3u), i32((li / 3u) % 3u), i32(li / 9u)) - vec3<i32>(1);
        let k = brick_kind(floor_div(origin, 8) + d);
        nb_kind[li] = k.x;
        nb_entry[li] = k.y;
    }
    workgroupBarrier();
    if (compute) {
        for (var j = li; j < 27u * UNIT_WORDS; j += WORKGROUP) {
            let k = j / UNIT_WORDS;
            let w = j % UNIT_WORDS;
            var m = 0u;
            if (nb_kind[k] == NB_MASK) {
                let e = nb_entry[k];
                let u = e & UNIT_MASK;
                m = pool_word(u, w);
                if ((e & TRANSLUCENT_BIT) != 0u) {
                    // A planar brick's translucent unit: its translucent voxels are non-air.
                    m |= pool_word(u, PAYLOAD_WORD + UNIT_WORDS * payload_units(e) + w);
                }
            }
            nb_mask[j] = m;
        }
    }
    workgroupBarrier();
    if (compute) {
        for (var j = li; j < BOX * BOX * BOX; j += WORKGROUP) {
            let c = vec3<i32>(i32(j % BOX), i32((j / BOX) % BOX), i32(j / (BOX * BOX)));
            // The neighbour on each axis: -1 for box cell 0, +1 for cell 9, else the brick.
            let d = select(vec3<i32>(0), vec3<i32>(-1), c < vec3<i32>(1))
                + select(vec3<i32>(0), vec3<i32>(1), c > vec3<i32>(8));
            let k = u32((d.x + 1) + 3 * (d.y + 1) + 9 * (d.z + 1));
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
                default: {}
            }
            occ[j] = select(0u, 1u, on);
        }
    }
    atomicStore(&words[li], 0xFFFFFFFFu);
    workgroupBarrier();
    if (compute) {
        for (var h = 0u; h < 2u; h++) {
            let i = 2u * li + h;
            let z = i / 64u;
            let b = i % 64u;
            let lo = pool_word(unit, s + 2u * z);
            let hi = pool_word(unit, s + 2u * z + 1u);
            var bit = 0u;
            var below = 0u;
            if (b < 32u) {
                bit = (lo >> b) & 1u;
                below = countOneBits(lo & ((1u << b) - 1u));
            } else {
                bit = (hi >> (b - 32u)) & 1u;
                below = countOneBits(lo) + countOneBits(hi & ((1u << (b - 32u)) - 1u));
            }
            if (bit != 0u) {
                let prefix = pool_word(unit, UNIT_WORDS + 10u + z / 2u);
                let k = ((prefix >> ((z & 1u) * 16u)) & 0xFFFFu) + below;
                let l = vec3<i32>(i32(i % 8u), i32((i / 8u) % 8u), i32(i / 64u));
                let a = voxel_attr(l + vec3<i32>(1));
                let shift = (k & 1u) * 16u;
                // Clears the u16 to the attribute, keeping the other half.
                atomicAnd(&words[k / 2u], (a << shift) | (0xFFFFu << (16u - shift)));
            }
        }
    }
    workgroupBarrier();
    if (li < n) {
        store_pool_word(unit, s + UNIT_WORDS + li, atomicLoad(&words[li]));
    }
}
