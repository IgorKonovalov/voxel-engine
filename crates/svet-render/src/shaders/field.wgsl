// The field pass (`field.rs`, ADR-0041): fills the band slots of a work list of bricks of any
// kind, per band voxel its count: the solid voxels among the 27 around it with itself (solid
// is not air: a translucent voxel and a crisp one are solid). A band slot is its band mask
// (one unit, the mask's layout: bit i % 32 of word i / 32 for voxel i = x + 8 y + 64 z), which
// the upload wrote, then ceil(n / 64) units of u8 counts for its n band voxels in band-index
// order: the k-th band voxel's in byte k % 4 of word k / 4. Every count unit is written whole,
// past the n-th byte as 0.
//
// Four bricks a workgroup, 64 invocations a brick: item 4 g + b is brick b of workgroup g, and
// the last workgroup may run partly empty. A brick resolves itself and its 26 neighbours once
// (table and record reads), loads their non-air masks (16 words each) into workgroup memory,
// and sets the occupancy of the brick and a two-voxel shell around it (12^3 voxels, two 12-bit
// rows a word, a brick's row run from one mask word) from those masks, a LOD-only chunk's
// neighbour through its level-1 mip brick's, reading voxel by voxel only a brick of more than
// 16 materials holding a translucent voxel. The counts read the box's 10^3 core.
//
// A count byte holds the count in its low five bits, SMOOTH_COUNT_BIT on a solid voxel of a
// smooth material and CRISP_COUNT_BIT on a solid voxel of a crisp one (`MaterialFlags::CRISP`
// on a row that is not translucent): a crisp voxel is solid in every count. The material is
// read only in a brick whose palette holds a crisp row. The pass also sets BAND_CUBES_BIT in
// the brick's header band word (HEADER_BAND_WORD of its own slot) when no voxel of the brick
// and its one-voxel shell is a smooth solid band voxel (a solid voxel with air among its 26
// neighbours), and clears it otherwise: no dual cell with a corner in such a brick has a
// smooth solid band corner, so `field_walk.wgsl`'s field walk crosses it as cubes. The brick's own
// band voxels carry the answer in their count bytes. A shell voxel's band test reads the full
// 12^3 occupancy box; its material is known from its brick's palette when that holds only
// smooth or only crisp solid materials, read when it holds both, and taken as smooth in a
// brick read voxel by voxel or a LOD-only chunk's mip brick. When neither the brick nor its
// 26 neighbours hold a smooth solid material by their palettes no shell voxel is read.
//
// Bindings, group 0:
//   0  uniform  params         mirror window, units per shard, work-list length, the
//                              dispatch's workgroups along x
//   1  storage  brick_pool0    pool shard 0, read-write; 2, 3 and 4 are shards 1 to 3. Same
//                              layout as in common.wgsl; this pass writes only band slots and
//                              the header band word
//   5  storage  chunk_records  as in common.wgsl
//   6  storage  chunk_table    as in common.wgsl
//  11  storage  field_work     [n][4 x u32]: the band slot's first unit, the brick's origin in
//                              voxels (x, y, z as i32 bits)
//  12  storage  materials      as in common.wgsl: [id][4 x vec4], word 7 of a row the flag bits

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
@group(0) @binding(11) var<storage, read> field_work: array<vec4<u32>>;
@group(0) @binding(12) var<storage, read> materials: array<vec4<f32>>;

// The mirror's layout constants and decode are mirror.wgsl's and the prelude's.

// Bricks a workgroup and invocations a brick; `field::BRICKS_PER_GROUP` and the
// `@workgroup_size` below must match.
const BRICKS: u32 = 4u;
const LANES: u32 = 64u;
// The occupancy box: the brick and a two-voxel shell, 12 to an edge, its cell c at voxel
// c - 2 from the brick's origin.
const OCC: u32 = 12u;

// Per brick of the workgroup (brick b's at b times the per-brick length): its occupancy box as
// 144 rows of 12 bits, two a word, row y + 12 z in half (row % 2) of word row / 2, x in bit x
// (each word set by one invocation); it and its 26 neighbours, index (dx + 1) + 3 (dy + 1) +
// 9 (dz + 1), how each is read (NB_AIR, NB_SOLID, NB_MASK or NB_MIP with its entry,
// NB_VOXELS) and the NB_MASK and NB_MIP ones' non-air masks; and its packed counts, 128 words.
var<workgroup> occ: array<u32, 288>;
var<workgroup> nb_kind: array<u32, 108>;
var<workgroup> nb_entry: array<u32, 108>;
var<workgroup> nb_mask: array<u32, 1728>;
var<workgroup> cells: array<atomic<u32>, 512>;
// The band mask words of the workgroup's four bricks, 16 each.
var<workgroup> band_words: array<atomic<u32>, 64>;
// Per brick of the workgroup: bit 0, a smooth solid material in its palette or a neighbour's;
// bit 1, a crisp material in its own palette; bit 2, a smooth solid band voxel in it or its
// one-voxel shell.
var<workgroup> brick_near: array<atomic<u32>, 4>;
// `brick_surfaces` of each brick's 27 neighbours, index as `nb_kind`.
var<workgroup> nb_surf: array<u32, 108>;

// Word `word` of the slot whose first unit is `unit`.
fn load_pool_word(unit: u32, word: u32) -> u32 {
    let a = shard_index(unit, word, params.units_per_shard);
    switch a.x {
        case 0u: { return brick_pool0[a.y]; }
        case 1u: { return brick_pool1[a.y]; }
        case 2u: { return brick_pool2[a.y]; }
        default: { return brick_pool3[a.y]; }
    }
}

fn store_pool_word(unit: u32, word: u32, value: u32) {
    let a = shard_index(unit, word, params.units_per_shard);
    switch a.x {
        case 0u: { brick_pool0[a.y] = value; }
        case 1u: { brick_pool1[a.y] = value; }
        case 2u: { brick_pool2[a.y] = value; }
        default: { brick_pool3[a.y] = value; }
    }
}

// Whether voxel `v` is solid: walk.wgsl's lookup with every LOD-only chunk at level 1.
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
    if (((load_pool_word(unit, i / 32u) >> (i % 32u)) & 1u) != 0u) {
        // An opaque voxel.
        return true;
    }
    if ((entry & TRANSLUCENT_BIT) == 0u) {
        return false;
    }
    // Air or a translucent voxel: a Wide brick's id, else the translucent unit after the planes.
    let cls = (entry >> CLASS_SHIFT) & 7u;
    if (cls == CLASS_WIDE) {
        let word = load_pool_word(unit, PAYLOAD_WORD + i / 2u);
        return ((word >> ((i & 1u) * 16u)) & 0xFFFFu) != 0u;
    }
    let tw = load_pool_word(unit, PAYLOAD_WORD + UNIT_WORDS * payload_units(entry) + i / 32u);
    return ((tw >> (i % 32u)) & 1u) != 0u;
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

// Row `r` (y + 12 z) of brick `b`'s occupancy box, 12 bits.
fn occ_row(b: u32, r: u32) -> u32 {
    return (occ[b * 72u + r / 2u] >> ((r & 1u) * 16u)) & 0xFFFu;
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
        let m = (load_pool_word(unit, UNIT_WORDS + j / 2u) >> ((j & 1u) * 16u)) & 0xFFFFu;
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
        let word = load_pool_word(unit, PAYLOAD_WORD + i / 2u);
        return material_crisp((word >> ((i & 1u) * 16u)) & 0xFFFFu);
    }
    var index = 0u;
    for (var k = 0u; k < payload_units(entry); k++) {
        let w = load_pool_word(unit, PAYLOAD_WORD + UNIT_WORDS * k + i / 32u);
        index |= ((w >> (i % 32u)) & 1u) << k;
    }
    let pair = load_pool_word(unit, UNIT_WORDS + index / 2u);
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
            m = load_pool_word(unit, t);
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
                m = load_pool_word(u, w);
                if ((e & TRANSLUCENT_BIT) != 0u) {
                    m |= load_pool_word(u, PAYLOAD_WORD + UNIT_WORDS * payload_units(e) + w);
                }
            }
            nb_mask[b * 432u + j] = m;
        }
    }
    workgroupBarrier();
    // The 12^3 occupancy box: the counts read its 10^3 core, the shell voxels' band test all
    // of it.
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
            let h = load_pool_word(u, UNIT_WORDS + HEADER_BAND_WORD) & ~BAND_CUBES_BIT;
            let cubes = (atomicLoad(&brick_near[b]) & 4u) == 0u;
            store_pool_word(u, UNIT_WORDS + HEADER_BAND_WORD, h | select(0u, BAND_CUBES_BIT, cubes));
        }
    }
}
