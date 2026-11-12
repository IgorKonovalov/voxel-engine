// The voxel walk over the brick-map mirror: the DDA, `descend`, `lookup`, `march`, the
// translucent walk's cells, and the hit's shading normals.
//
// A pixel takes the first view whose rectangle holds it (the first when none does). Its ray
// runs from the view's origin along `fwd + right x - up y`, x and y moving linearly across the
// rectangle from minus the left tangent to the right one and from the up tangent to minus the
// down one.
//
// In the Contour normal mode over a mirror with band slots the primary ray walks the field
// surface (see the field walk below), outside the cutaway view; every other ray (the shadow,
// probe and translucent rays among them), and the primary ray in the other modes, walks the
// voxels: a solid voxel hits as its cube.
//
// World units are voxels. The walk is a DDA over cells of seven sizes: a chunk that is absent
// or empty (its table entry's EMPTY_BIT) is skipped as one 64^3 cell, a 2 x 2 x 2 group of
// bricks whose brick bits are all clear as one 16^3 cell, a uniform-air brick as one 8^3
// cell, and, when the masks flag is set, an empty 4^3 or 2^3 block or an empty voxel of a
// full brick as one cell of its size, each read from the brick's opaque line (its slot's unit
// 0), so the rest of the slot is read only for a voxel whose bit is set. With the masks flag
// clear every voxel of a full brick is one cell whose opaque bit is read as a pool word; with
// the skips flag clear every cell is one voxel (one mip
// cell in a LOD-only chunk), the comparison walk. A uniform solid brick is hit where the ray
// enters it. A LOD-only chunk is walked the same way at its mip level L, every size scaled by
// 2^L: a clear group of mip-bricks is one (16 * 2^L)^3 cell below level 3, a uniform-air
// mip-brick one (8 * 2^L)^3 cell, an empty block of a mip-brick one (4 * 2^L)^3 or
// (2 * 2^L)^3 cell, and each mip cell one (2^L)^3 cell. Every skip is aligned to its size, so
// a solid cell is always entered on its face. Each step leaves the current cell through its
// nearest face, the lower axis on a tie; the coordinate on the exit axis is set exactly from
// the face, and each other coordinate follows plane order (see `advance`): the voxel entered
// is a function of the plane crossings alone, never of the size of the cell left, so the
// walk finds the same voxel through the same face with every skip on or off. Across steps
// the walk holds its chunk, bits words, brick entry and mask words (see `Walk`), so the
// table is read on entering a chunk, the record entry on entering a brick, and the bits and
// mask words when the ray leaves the row or slab they cover.
//
// Translucent voxels are air to that walk. When it has read an entry flagged translucent and
// `translucent_layers` is not 0, a second walk runs from the ray origin to the opaque hit
// (`translucent_march`): it skips the same empty cells, an unflagged brick as a whole, and a
// flagged Full brick's clear 2^3 blocks from its two translucent words, and filters by the path
// length through the translucent voxels it enters (ADR-0047), up to `translucent_layers` voxels of
// them; what remains of the transmittance carries the opaque hit's colour.
//
// Cutaway (flag bit 10): the primary ray starts in a `passing` state that steps through solid
// cells (a uniform solid brick as one 8^3 cell, scaled by the mip cell; a solid voxel of a Full
// brick as one voxel) until it enters an empty cell of a resident chunk: an empty chunk, a
// clear group of bricks, a uniform-air or uniform-translucent brick, a clear block or an empty
// voxel of a Full brick. A chunk outside the window or absent from the table is not air, so a
// ray that crosses rock from absent space to absent space misses. Once cleared, the walk is
// the plain walk; a ray that starts in air clears it on its first cell. Shadow rays never pass.
// With the flag set a hit is shaded by a fixed hemisphere light, `albedo (0.3 + 0.7 (0.5 + 0.5
// n.z))` with world z up, with no lamp and no shadow ray, and a miss is the flat `CUTAWAY_SKY`.

const MAX_STEPS: u32 = 4096u;

// What the walk found at a voxel: `size` is the edge of the empty cell to skip, or 0 for a
// solid voxel of `material`. A solid voxel of a full chunk's Full brick carries its brick's
// record entry (`slot`) and its index in the brick; any other cell carries ABSENT. `through` is the edge of
// the aligned solid cell a cutaway ray in its passing state steps through (0 for an empty
// cell), and `air` is set for an empty cell of a resident chunk, the cell that ends passing.
struct Cell {
    size: i32,
    material: u32,
    slot: u32,
    index: u32,
    through: i32,
    air: bool,
};

struct Hit {
    found: bool,
    voxel: vec3<i32>,
    axis: i32,
    t: f32,
    material: u32,
};

// The walk's state across the steps of one `march`: the chunk it is in with its table entry
// decoded, the two brick bits words it last read, the brick it is in with its record entry,
// and the four opaque mask words it last read. Each is re-read only when its key changes, so
// per ray segment a level costs one read; what is held changes which loads are issued, never
// what `lookup` answers. Entering a chunk drops the bits words and the brick; the mask words
// are keyed by their global word index and stay until another slab is tested.
struct Walk {
    chunk: vec3<i32>,
    in_chunk: bool,
    // The table entry of `chunk`, and its decoded mip level, first entry and record base.
    entry: u32,
    level: u32,
    first: u32,
    base: u32,
    // `bits.x` is record word `bits_word` and `bits.y` the word two on; ABSENT holds nothing.
    bits_word: u32,
    bits: vec2<u32>,
    brick: vec3<i32>,
    in_brick: bool,
    brick_entry: u32,
    // `mask` holds mask words `mask_word`, + 2, + 4 and + 6 (keyed by the pool word index,
    // the slot's first unit times 16 plus the word): the four words of one 4^3 block row of a
    // brick (its x-slab of four layers and one y-half); ABSENT holds nothing.
    mask_word: u32,
    mask: vec4<u32>,
    // Set when a brick entry read since the reset carries TRANSLUCENT_BIT.
    saw_translucent: bool,
    // With `split_walk`: `brick` is a field brick (a band brick without BAND_CUBES_BIT).
    brick_field: bool,
};

var<private> walk: Walk;

// Set by `split_pixel` around its `march`: `descend` reads each full chunk's Full brick's band
// word as it enters the brick, and sets `split_entered` when the voxel it answers for lies in
// a field brick, where `march` stops and hands the ray on. Clear for every other walk.
var<private> split_walk: bool = false;

var<private> split_entered: bool;

// The stretch of the ray parameter over which `march` takes the coarse shadow walk's cells
// (`lookup_walk`); empty (BIG, -BIG) for the full walk. The light pass sets it around a
// shadow ray, leaving the first and last COARSE_MARGIN voxels to the full walk so the
// blocks the ray starts and ends in do not shadow it.
var<private> coarse_span: vec2<f32> = vec2<f32>(3.0e38, -3.0e38);

// The walk's cost counters since the last reset: loop iterations of `march`, 32-bit storage
// loads from the mirror (table, record bits and entries, mask lines, the header's translucent
// words, the rest of the slots), and the pool loads among them: every slot word but a mask
// line's or a translucent word's. `main` resets them before the primary ray and reads them after it in the
// steps view. The loads count in the low half of `loads` and the pool loads in its high half
// (one register; a ray past 65535 loads would carry into the high half, far above the 255 the
// views clamp at).
var<private> walk_steps: u32;

var<private> loads: u32;

fn counted_loads() -> u32 {
    return loads & 0xFFFFu;
}

fn counted_pool_reads() -> u32 {
    return loads >> 16u;
}

fn reset_walk() {
    walk.in_chunk = false;
    walk.in_brick = false;
    walk.bits_word = ABSENT;
    walk.mask_word = ABSENT;
    walk.saw_translucent = false;
}

// Word `word` of the slot whose first unit is `unit` (a slot's words run on through its units,
// which lie in one page, so in one shard), uncounted.
fn slot_word(unit: u32, word: u32) -> u32 {
    let shard = unit / params.units_per_shard;
    let i = (unit - shard * params.units_per_shard) * UNIT_WORDS + word;
    switch shard {
        case 0u: { return brick_pool0[i]; }
        case 1u: { return brick_pool1[i]; }
        case 2u: { return brick_pool2[i]; }
        default: { return brick_pool3[i]; }
    }
}

// `slot_word`, counted as a pool load.
fn pool_word(unit: u32, word: u32) -> u32 {
    loads += 0x10001u;
    return slot_word(unit, word);
}

// `slot_word` of a mask line or a translucent word, counted as a load only.
fn mask_word_at(unit: u32, word: u32) -> u32 {
    loads += 1u;
    return slot_word(unit, word);
}

fn record_word(w: u32) -> u32 {
    loads += 1u;
    return chunk_records[w];
}

// The mip level a LOD-only chunk is walked at: forced, or by its squared chunk distance to the
// camera's chunk.
fn lod_level(chunk: vec3<i32>) -> u32 {
    if (params.forced_level != 0u) {
        return params.forced_level;
    }
    let d = chunk - floor_div(vec3<i32>(floor(params.views[0].origin)), 64);
    let d2 = dot(d, d);
    if (d2 <= params.lod1_r2) {
        return 1u;
    }
    if (d2 <= params.lod2_r2) {
        return 2u;
    }
    return 3u;
}

// Whether the 2 x 2 x 2 group of bricks holding brick `b` of the walk's chunk has a brick that
// is not uniform air, from the record's brick bits. At level 0 brick (x, y, z) is bit
// x + 8 (y % 4) of word 2 z + y / 4, so a group's bits are two bytes in each of two words; at
// level 1 (entries x + 4 y + 16 z) one word holds the group; at level 2 the group is the whole
// level, entries 64..72; level 3 has no group. The words are read only when the group's first
// word is not the one held, so the groups of a row along x share one read.
fn group_occupied(b: vec3<i32>) -> bool {
    let g = vec3<u32>(b / 2);
    let base = walk.base;
    switch walk.level {
        case 0u: {
            let w = base + 4u * g.z + g.y / 2u;
            if (w != walk.bits_word) {
                walk.bits_word = w;
                walk.bits = vec2<u32>(record_word(w), record_word(w + 2u));
            }
            let m = 0x0303u << (2u * g.x + 16u * (g.y & 1u));
            return ((walk.bits.x | walk.bits.y) & m) != 0u;
        }
        case 1u: {
            let w = base + g.z;
            if (w != walk.bits_word) {
                walk.bits_word = w;
                walk.bits = vec2<u32>(record_word(w), 0u);
            }
            return (walk.bits.x & (0x00330033u << (2u * g.x + 8u * g.y))) != 0u;
        }
        case 2u: {
            let w = base + 2u;
            if (w != walk.bits_word) {
                walk.bits_word = w;
                walk.bits = vec2<u32>(record_word(w), 0u);
            }
            return (walk.bits.x & 0xFFu) != 0u;
        }
        default: {
            return true;
        }
    }
}

// The edge of the empty cell holding voxel `l` (index `i`) of the brick whose slot starts at
// `unit` (its opaque mask is the unit's 16 words): 4 or 2 for an empty block, 1 for an empty
// voxel, 0 for a set voxel.
// Layer z is words 2 z (rows 0..4) and 2 z + 1 (rows 4..8), bit x + 8 (y % 4); a 4^3 block
// (bx, by, bz) is the byte-wide nibbles 0x0F0F0F0F << 4 bx of words 2 (4 bz + k) + by, and a
// 2^3 block (cx, cy, cz) the bits 0x0303 << (2 cx + 16 (cy % 2)) of words
// 2 (2 cz + k) + cy / 2. The four 4^3 words are held across steps, and the 2^3 block's two
// words and the voxel's word are among them (layers 2 cz, 2 cz + 1 and z lie in the block's
// four layers, with the same y-half), so a step inside the slab issues no mask load.
fn mask_skip(unit: u32, l: vec3<u32>, i: u32) -> i32 {
    let q = l / 4u;
    let w = 8u * q.z + q.y;
    let w4 = unit * UNIT_WORDS + w;
    if (w4 != walk.mask_word) {
        walk.mask_word = w4;
        walk.mask = vec4<u32>(
            mask_word_at(unit, w),
            mask_word_at(unit, w + 2u),
            mask_word_at(unit, w + 4u),
            mask_word_at(unit, w + 6u),
        );
    }
    let mk = walk.mask;
    if (((mk.x | mk.y | mk.z | mk.w) & (0x0F0F0F0Fu << (4u * q.x))) == 0u) {
        return 4;
    }
    let h = l / 2u;
    let j = 2u * ((l.z / 2u) & 1u);
    if (((mk[j] | mk[j + 1u]) & (0x0303u << (2u * h.x + 16u * (h.y & 1u)))) == 0u) {
        return 2;
    }
    if ((mk[l.z & 3u] & (1u << (i % 32u))) == 0u) {
        return 1;
    }
    return 0;
}

// Where voxel `v` lies in the hierarchy. `size` > 0 is an empty cell of that edge: a chunk
// outside the window, an absent or empty chunk, a clear 2 x 2 x 2 group of bricks or a
// uniform-air brick, each scaled by the mip cell, or one cell with the skips flag clear.
// Otherwise `entry` is the brick's record entry, `l` and `i` the voxel's position and index
// in the brick, `cell` the mip cell's edge and `level` the mip level. The table is read on
// entering a chunk and the record entry on entering a brick; a chunk outside the window
// touches no state. An entry carrying TRANSLUCENT_BIT marks the walk. `resident` is clear for
// a chunk outside the window or absent from the table.
struct Descent {
    size: i32,
    entry: u32,
    cell: i32,
    level: u32,
    l: vec3<u32>,
    i: u32,
    resident: bool,
};

fn empty_descent(size: i32) -> Descent {
    return Descent(size, 0u, 1, 0u, vec3<u32>(0u), 0u, true);
}

fn absent_descent(size: i32) -> Descent {
    return Descent(size, 0u, 1, 0u, vec3<u32>(0u), 0u, false);
}

fn descend(v: vec3<i32>) -> Descent {
    split_entered = false;
    let skips = (params.flags & FLAG_SKIPS) != 0u;
    let chunk = floor_div(v, 64);
    let rel = chunk - params.window_min;
    let w = params.window_size;
    if (any(rel < vec3<i32>(0)) || any(rel >= w)) {
        return absent_descent(64);
    }
    if (!walk.in_chunk || any(chunk != walk.chunk)) {
        walk.in_chunk = true;
        walk.chunk = chunk;
        walk.in_brick = false;
        walk.bits_word = ABSENT;
        // Wrapped addressing: the table is indexed by the chunk coordinate modulo the window
        // size, and the range check above keeps every lookup inside the current window.
        let wr = ((chunk % w) + w) % w;
        loads += 1u;
        let t = chunk_table[u32(wr.x + w.x * (wr.y + w.y * wr.z))];
        walk.entry = t;
        // A full chunk is level 0: voxels in 8^3 bricks, 8 bricks to an edge. A LOD record
        // holds level L's cells in 8^3 mip-bricks, 8 >> L to an edge, from entry 0, 64 or 72.
        walk.level = 0u;
        walk.first = 0u;
        if (t != ABSENT && (t & LOD_BIT) != 0u) {
            walk.level = lod_level(chunk);
            walk.first = select(select(72u, 64u, walk.level == 2u), 0u, walk.level == 1u);
        }
        walk.base = (t & RECORD_MASK) * RECORD_WORDS;
    }
    let t = walk.entry;
    if (t == ABSENT) {
        return absent_descent(select(1, 64, skips));
    }
    if (skips && (t & EMPTY_BIT) != 0u) {
        return empty_descent(64);
    }
    let level = walk.level;
    let cell = 1 << level;
    let e = 8 >> level;
    let c = (v - chunk * 64) >> vec3<u32>(level);
    let b = c / 8;
    if (skips && !group_occupied(b)) {
        return empty_descent(16 * cell);
    }
    if (!walk.in_brick || any(b != walk.brick)) {
        walk.in_brick = true;
        walk.brick = b;
        walk.brick_entry =
            record_word(walk.base + ENTRY_WORD + walk.first + u32(b.x + e * (b.y + e * b.z)));
        walk.saw_translucent = walk.saw_translucent || (walk.brick_entry & TRANSLUCENT_BIT) != 0u;
        walk.brick_field = false;
        if (split_walk && level == 0u && (walk.brick_entry & FULL_BIT) != 0u) {
            let h = pool_word(walk.brick_entry & UNIT_MASK, HEADER_WORD + HEADER_BAND_WORD);
            walk.brick_field = (h & (HEADER_BAND_BIT | BAND_CUBES_BIT)) == HEADER_BAND_BIT;
        }
    }
    let entry = walk.brick_entry;
    if (entry == 0u) {
        return empty_descent(select(cell, 8 * cell, skips));
    }
    split_entered = walk.brick_field;
    let l = c - b * 8;
    return Descent(0, entry, cell, level, vec3<u32>(l), u32(l.x + 8 * (l.y + 8 * l.z)), true);
}

// The material of voxel `i` of the Full brick with record entry `entry`, a voxel that is not
// air: a Wide brick's u16 id directly; a planar brick's palette entry, its index assembled
// from one bit of each plane, read from the header.
fn voxel_material(entry: u32, i: u32) -> u32 {
    let unit = entry & UNIT_MASK;
    let cls = (entry >> CLASS_SHIFT) & 7u;
    if (cls == CLASS_WIDE) {
        let word = pool_word(unit, PAYLOAD_WORD + i / 2u);
        return (word >> ((i & 1u) * 16u)) & 0xFFFFu;
    }
    var index = 0u;
    let planes = class_planes(cls);
    for (var k = 0u; k < planes; k++) {
        let w = pool_word(unit, PAYLOAD_WORD + UNIT_WORDS * k + i / 32u);
        index |= ((w >> (i % 32u)) & 1u) << k;
    }
    let pair = pool_word(unit, HEADER_WORD + index / 2u);
    return (pair >> ((index & 1u) * 16u)) & 0xFFFFu;
}

fn is_translucent(m: u32) -> bool {
    return (bitcast<u32>(materials[m * 4u + 1u].w) & MATERIAL_TRANSLUCENT) != 0u;
}

// The cell the opaque walk finds at voxel `v`. A translucent voxel is air to it: a uniform
// translucent brick is one empty 8^3 cell, a translucent voxel of a Full brick has no bit in
// the opaque mask, and with the mask tests off its material is read and found translucent.
fn lookup(v: vec3<i32>) -> Cell {
    return lookup_walk(v, false);
}

// `lookup`, or with `coarse` the coarse shadow walk's cell: a Full brick's voxel in a 2^3
// block with a set opaque bit is solid without a pool read (the block's other voxels too),
// so the walk reads chunk bits, brick bits and one mask line per brick and never a voxel.
// Without the masks flag the coarse walk is the full one.
fn lookup_walk(v: vec3<i32>, coarse: bool) -> Cell {
    let dsc = descend(v);
    if (dsc.size != 0) {
        return Cell(dsc.size, 0u, ABSENT, 0u, 0, dsc.resident);
    }
    let skips = (params.flags & FLAG_SKIPS) != 0u;
    let entry = dsc.entry;
    let cell = dsc.cell;
    if ((entry & FULL_BIT) == 0u) {
        let brick = select(cell, 8 * cell, skips);
        if ((entry & TRANSLUCENT_BIT) != 0u) {
            return Cell(brick, 0u, ABSENT, 0u, 0, true);
        }
        return Cell(0, entry & MATERIAL_MASK, ABSENT, 0u, brick, false);
    }
    let unit = entry & UNIT_MASK;
    let masked = skips && (params.flags & FLAG_MASKS) != 0u;
    if (masked) {
        let size = mask_skip(unit, dsc.l, dsc.i);
        if (coarse && size < 2) {
            return Cell(0, 0u, ABSENT, 0u, cell, false);
        }
        if (size != 0) {
            return Cell(size * cell, 0u, ABSENT, 0u, 0, true);
        }
    } else if (((pool_word(unit, dsc.i / 32u) >> (dsc.i % 32u)) & 1u) == 0u) {
        // Without the mask tests the voxel's opaque bit is read alone: clear is air or a
        // translucent voxel, both air to this walk.
        return Cell(cell, 0u, ABSENT, 0u, 0, true);
    }
    let m = voxel_material(entry, dsc.i);
    return Cell(0, m, select(ABSENT, entry, dsc.level == 0u), dsc.i, cell, false);
}

// What the translucent walk finds at voxel `v`: `size` > 0 is a cell holding no translucent
// voxel (the hierarchy's empty cells; an unflagged brick, uniform or Full, as 8^3; a flagged
// Full brick's clear 2^3 block from its translucent words; an air or opaque voxel as one
// cell), else `material` is the translucent voxel's. `cell` is the mip cell's edge.
struct TCell {
    size: i32,
    material: u32,
    cell: i32,
};

fn tlookup(v: vec3<i32>) -> TCell {
    let dsc = descend(v);
    if (dsc.size != 0) {
        return TCell(dsc.size, 0u, dsc.cell);
    }
    let skips = (params.flags & FLAG_SKIPS) != 0u;
    let entry = dsc.entry;
    let cell = dsc.cell;
    if ((entry & TRANSLUCENT_BIT) == 0u) {
        return TCell(select(cell, 8 * cell, skips), 0u, cell);
    }
    if ((entry & FULL_BIT) == 0u) {
        return TCell(0, entry & MATERIAL_MASK, cell);
    }
    let unit = entry & UNIT_MASK;
    if (skips) {
        let l = dsc.l;
        let block = l.x / 2u + 4u * (l.y / 2u) + 16u * (l.z / 2u);
        let tw = mask_word_at(unit, HEADER_TRANSLUCENT_WORD + block / 32u);
        if ((tw & (1u << (block % 32u))) == 0u) {
            return TCell(2 * cell, 0u, cell);
        }
    }
    let i = dsc.i;
    if (((entry >> CLASS_SHIFT) & 7u) == CLASS_WIDE) {
        // A Wide brick's id says it all: 0 is air.
        let m = voxel_material(entry, i);
        if (m == 0u || !is_translucent(m)) {
            return TCell(cell, 0u, cell);
        }
        return TCell(0, m, cell);
    }
    // A planar brick's translucent unit follows its planes: a clear bit is air or opaque.
    let t = pool_word(unit, PAYLOAD_WORD + UNIT_WORDS * payload_units(entry) + i / 32u);
    if (((t >> (i % 32u)) & 1u) == 0u) {
        return TCell(cell, 0u, cell);
    }
    return TCell(0, voxel_material(entry, i), cell);
}

// The voxel holding `p` on a ray of direction `d`: on an axis the ray travels down, a position
// exactly on a voxel boundary belongs to the voxel below.
fn voxel_at(p: vec3<f32>, d: vec3<f32>) -> vec3<i32> {
    return vec3<i32>(select(floor(p), ceil(p) - 1.0, d < vec3<f32>(0.0)));
}

// The voxel the ray `o + d t` is in after crossing the plane `face` of axis `k` at `t`, by
// plane order. On axis `k` the coordinate is the voxel on the far side of `face`. On each
// other axis `j` the ray has crossed every voxel plane `B` whose crossing
// `t_j(B) = (B - o_j) inv_j` sorts before `(t, k)` in `(t, axis)` lexicographic order, and no
// other: the order the exit test in `march` takes the planes in (nearest first, the lower
// axis on a tie), so the result does not depend on the size of the cell the ray has left,
// and a walk over any aligned empty cells enters the same voxels as the walk over single
// voxels. The off-axis coordinates are estimated from the float position, which rounds and
// may be contracted into an FMA, then moved by at most one plane each way: back one when the
// plane into the estimated voxel is not crossed, forward one when the plane out of it is.
// The planes are tested with the exit test's own expression, so the two agree bit for bit.
// An axis the ray does not move along keeps its estimate.
fn advance(o: vec3<f32>, d: vec3<f32>, inv: vec3<f32>, t: f32, k: i32, face: i32) -> vec3<i32> {
    var v = voxel_at(o + d * t, d);
    let up = d > vec3<f32>(0.0);
    let moving = d != vec3<f32>(0.0);
    let step = select(vec3<i32>(-1), vec3<i32>(1), up);
    // The plane the ray crosses into the estimated voxel, and the one it crosses out of it.
    let into = select(v + 1, v, up);
    let out = into + step;
    let t_into = (vec3<f32>(into) - o) * inv;
    let t_out = (vec3<f32>(out) - o) * inv;
    let lower = vec3<i32>(0, 1, 2) < vec3<i32>(k);
    let crossed_into = (t_into < vec3<f32>(t)) | ((t_into == vec3<f32>(t)) & lower);
    let crossed_out = (t_out < vec3<f32>(t)) | ((t_out == vec3<f32>(t)) & lower);
    v = select(v, v - step, moving & !crossed_into);
    v = select(v, v + step, moving & crossed_out);
    return select(v, vec3<i32>(select(face - 1, face, d[k] > 0.0)), axis_mask(k));
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

// The ray parameter where it leaves cube `v`.
fn cube_exit(o: vec3<f32>, d: vec3<f32>, inv: vec3<f32>, v: vec3<i32>) -> f32 {
    let edge = vec3<f32>(v) + select(vec3<f32>(0.0), vec3<f32>(1.0), d > vec3<f32>(0.0));
    let tk = select(vec3<f32>(BIG), (edge - o) * inv, d != vec3<f32>(0.0));
    return min(min(tk.x, tk.y), tk.z);
}

// The first solid voxel along `o + d t` for `0 <= t <= t_limit`. With `cutaway` set the walk
// starts passing: solid cells are stepped through until the first empty cell of a resident
// chunk.
fn march(o: vec3<f32>, d: vec3<f32>, t_limit: f32, cutaway: bool) -> Hit {
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

    reset_walk();
    var t = max(t0, 0.0);
    var axis = -1;
    var v = voxel_at(o + d * t, d);
    if (t0 > 0.0) {
        // Entering the window through its face on `axis`: the window's minimum plane on the
        // way up, its maximum plane on the way down.
        axis = select(select(2, 1, tmin.y == t0), 0, tmin.x == t0);
        let face = select(bmax_i[axis], bmin_i[axis], d[axis] > 0.0);
        v = advance(o, d, inv, t, axis, face);
    }
    v = clamp(v, bmin_i, bmax_i - vec3<i32>(1));
    var passing = cutaway;

    for (var step = 0u; step < MAX_STEPS; step++) {
        walk_steps += 1u;
        if (any(v < bmin_i) || any(v >= bmax_i)) {
            return miss;
        }
        if (t > t_limit) {
            return miss;
        }
        let c = lookup_walk(v, t >= coarse_span.x && t <= coarse_span.y);
        if (split_entered) {
            // The split walk's hand-off: the ray enters a field brick at `v`.
            return Hit(false, v, axis, t, SPLIT_HANDOFF);
        }
        var size = c.size;
        if (passing) {
            if (size == 0) {
                size = c.through;
            } else if (c.air) {
                passing = false;
            }
        }
        if (size == 0) {
            return Hit(true, v, axis, t, c.material);
        }
        // Leave the cell through its nearest face, the lower axis on a tie: the same
        // `(t, axis)` order `advance` crosses the other axes' planes in.
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
// sums the nine voxels on its positive face minus the nine on its negative face. For a hit voxel
// at local 1 to 6 on every axis of a full-detail Full brick (with the walk's skips and masks on)
// the 27 voxels lie in that brick, and their occupancy is their bits in its opaque mask: a voxel's
// mask bit is `lookup(v).size == 0` (the mask holds the opaque voxels; a translucent voxel is air
// to both), so the six mask words of the three layers replace 26 lookups. The sums are of small
// integers, exact in any order, so both paths give the same normal bit for bit.
fn gradient27_normal(h: Hit, fallback: vec3<f32>) -> vec3<f32> {
    let flags = FLAG_SKIPS | FLAG_MASKS;
    if ((params.flags & flags) == flags) {
        let dsc = descend(h.voxel);
        if (dsc.size == 0 && dsc.level == 0u && (dsc.entry & FULL_BIT) != 0u
            && all(dsc.l >= vec3<u32>(1u)) && all(dsc.l <= vec3<u32>(6u))) {
            let unit = dsc.entry & UNIT_MASK;
            var g = vec3<f32>(0.0);
            for (var z = -1; z <= 1; z++) {
                // Layer lz's 64 bits: word 2 lz holds rows 0..3 (bit x + 8 y), word 2 lz + 1
                // rows 4..7.
                let lz = dsc.l.z + u32(z + 1) - 1u;
                let lo = mask_word_at(unit, 2u * lz);
                let hi = mask_word_at(unit, 2u * lz + 1u);
                for (var y = -1; y <= 1; y++) {
                    let ly = dsc.l.y + u32(y + 1) - 1u;
                    let row = select(lo, hi, ly >= 4u) >> (8u * (ly & 3u));
                    for (var x = -1; x <= 1; x++) {
                        let lx = dsc.l.x + u32(x + 1) - 1u;
                        let o = vec3<i32>(x, y, z);
                        if (any(o != vec3<i32>(0))) {
                            g += vec3<f32>(o) * f32((row >> lx) & 1u);
                        }
                    }
                }
            }
            return normal_from_gradient(g, fallback);
        }
    }
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

// The weight of the far normal for a hit at distance `t` voxels in view `v`: 1 where a voxel
// spans at most `far_lo` pixels across the view's width, 0 at `far_hi` and over, a smoothstep
// between; 0 with the knob off.
fn far_normal_weight(v: View, t: f32) -> f32 {
    if (params.far_hi <= params.far_lo) {
        return 0.0;
    }
    let px = f32(v.rect.z) / ((v.tan_left + v.tan_right) * max(t, 1e-3));
    return 1.0 - smoothstep(params.far_lo, params.far_hi, px);
}

// The shading normal of hit `h` of a ray of view `v` under the current normal mode. A ray that
// starts inside rock (no entry face) keeps the face-mode normal in every mode. Face mode blends
// toward the 27-count gradient by `far_normal_weight`; at weight 0 it reads no neighbour.
fn surface_normal(v: View, o: vec3<f32>, d: vec3<f32>, h: Hit) -> vec3<f32> {
    let nf = face_normal(d, h);
    if (h.axis < 0) {
        return nf;
    }
    let mode = normal_mode();
    if (mode == NORMAL_TRILINEAR8) {
        return trilinear8_normal(o + d * h.t, h, nf);
    }
    if (mode == NORMAL_GRADIENT27) {
        return gradient27_normal(h, nf);
    }
    let w = far_normal_weight(v, h.t);
    if (w <= 0.0) {
        return nf;
    }
    let g = gradient27_normal(h, nf);
    if (w >= 1.0) {
        return g;
    }
    return normalize(mix(nf, g, w));
}

// A field-surface hit's shading normal: its field normal `n` mixed `params.face_mix` of the
// way toward its entry face's normal `f`, `normalize(mix(n, f, k))`; exactly `n` at 0.
fn face_mixed(n: vec3<f32>, f: vec3<f32>) -> vec3<f32> {
    if (params.face_mix > 0.0) {
        return normalize(mix(n, f, params.face_mix));
    }
    return n;
}

// The field normal `n` of a field-surface hit whose stored shading normal is `m`, the face mix
// of `n` toward its entry face's normal `f`: `(1 - k) n = l m - k f` with `l = |mix|` the root
// of `l^2 - 2 l k (m.f) + k^2 - (1 - k)^2 = 0`, real for `k <= 1/2`; `m` itself at 0.
fn face_unmixed(m: vec3<f32>, f: vec3<f32>) -> vec3<f32> {
    let k = params.face_mix;
    if (k <= 0.0) {
        return m;
    }
    let mf = dot(m, f);
    let l = k * mf + sqrt(max(k * k * mf * mf - k * k + (1.0 - k) * (1.0 - k), 0.0));
    return normalize(l * m - k * f);
}

// How far off the hit the shadow ray starts, in voxels, along `shadow_normal`.
const SHADOW_OFFSET: f32 = 0.5;

// The direction the shadow ray of a voxel-walk hit `h` leaves along: its entry face's normal:
// the offset is a robustness step out of the surface, not part of the look.
fn shadow_normal(d: vec3<f32>, h: Hit) -> vec3<f32> {
    return face_normal(d, h);
}
