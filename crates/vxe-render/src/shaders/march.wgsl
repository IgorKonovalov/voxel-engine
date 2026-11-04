// The hit pass over the brick-map mirror (ADR-0002, ADR-0004, ADR-0018), one invocation per
// pixel: the primary ray's walk and a hit record per pixel, no lighting. light.wgsl and
// composite.wgsl are compiled after this file and share its bindings, walk and helpers.
//
// Bindings, group 0 (every pass of the frame):
//   0  uniform  params         views, mirror window, light, target size, flags (bit 0: shadow ray,
//                              bit 1: occupancy masks, bits 2..4: normal mode,
//                              bits 5..8: view, bit 9: skip empty cells larger than a voxel,
//                              bit 10: cutaway, bit 19: the split field walk), the lighting
//                              flags, exposure and the face mix
//   1  storage  brick_pool0    shard 0 of the pool: [unit][16 x u32], 64-byte units; unit u is
//                              unit u % units_per_shard of shard u / units_per_shard; 7, 8 and
//                              9 are shards 1 to 3. A Full brick's slot is a run of units
//                              (gpu_world.rs): unit 0 its opaque mask, bit i % 32 of word
//                              i / 32 set when voxel i (x + 8 y + 64 z) is neither air nor
//                              TRANSLUCENT; unit 1 its header (words 0..8 the palette, 16 u16
//                              ascending; words 8, 9 the translucent 2^3 block bits, bit
//                              bx + 4 by + 16 bz); then 0, 1, 2 or 4 bit planes in the mask's
//                              layout (plane k is bit k of a voxel's palette index), or for
//                              class 5 (Wide) 16 units of u16 ids, voxel i in half i % 2 of
//                              word i / 2; then, for a planar brick holding a TRANSLUCENT
//                              voxel, one unit of its translucent voxels. A band brick's
//                              header names its band slot (word 15) and holds its band
//                              prefix (words 12 and 13, see the field walk)
//   2  storage  chunk_records  [record][528 x u32]: 16 words of brick bits (bit b of word
//                              b / 32 set when entry b is not uniform air), then 512 brick
//                              entries. Entry bit 31 set: the slot's first unit in bits 0..26
//                              and its class in bits 26..29 (1 to 4: 0, 1, 2 or 4 planes; 5:
//                              Wide); clear: the uniform material in bits 0..16; bit 29 set
//                              either way when the brick holds a TRANSLUCENT voxel
//   3  storage  chunk_table    [wrapped chunk]u32 record index in bits 0..29, bit 29 set when
//                              the record's brick bits are all clear, bit 30 set for a LOD
//                              record (entries 0..64 mip level 1, 64..72 level 2, 72 level 3);
//                              0xFFFFFFFF when absent
//   4  storage  materials      [id][4 x vec4]: albedo rgb + roughness, emission rgb + flag bits,
//                              2 spare + opacity + 1 spare, 4 spare
//   7  storage  brick_pool1    pool shard 1
//   8  storage  brick_pool2    pool shard 2
//   9  storage  brick_pool3    pool shard 3
//  10  storage  light_samples  [sample]vec4<i32>: an exposed emissive voxel's world coordinate,
//                              then its material (bits 0..16) and the faces it shows (bit
//                              16 + 2 k toward -k, the next toward +k); material 0 is a hole.
//                              Declared by light.wgsl
//  11  storage  emission_scale [material]f32, the frame's emission scale in the light the
//                              pixels sample, then EMISSION_ROWS more in the light the probe
//                              rays sample (`emission_base`)
//  12  texture  blue_noise     128 x 128 r8unorm blue noise (void and cluster)
//  13  uniform  exposure_state the exposure state the last frame's exposure pass wrote
//                              (exposure.wgsl, ADR-0033): `exposure_now` and `exposure_ratio`
//
// Bindings, group 1 (the hit pass's outputs, the hit buffer):
//   0  storage texture  hit_pos_out    rgba32float: the hit's world position (voxels) and t;
//                                      (0, 0, 0, BIG) on a miss
//   1  storage texture  hit_info_out   rgba16uint: the shading normal as two
//                                      snorm16 octahedral coordinates (exact for the six axis
//                                      normals), the material id, and flags: bit 0 hit, bit 1
//                                      field-surface hit, bit 2 translucent layers in front,
//                                      bits 3..5 the entry face's axis plus one (0: the ray
//                                      started inside rock)
//   2  storage texture  layer_out      rgba16float: the translucent layers in front of the hit
//                                      (premultiplied colour, transmittance), lit by the lamp's
//                                      cone and distance term except in the albedo view; in the
//                                      steps view the primary ray's counters instead; in the
//                                      cracks view 1 in r for a field walk's leak, else 0
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
// flagged Full brick's clear 2^3 blocks from its two translucent words, and composites the
// translucent voxels it enters front to back, each albedo times opacity under the lamp's cone
// and distance term with no shadow ray, up to `translucent_layers` of them; what remains of
// the transmittance carries the opaque hit's colour.
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

struct View {
    origin: vec3<f32>,
    tan_left: f32,
    right: vec3<f32>,
    tan_right: f32,
    up: vec3<f32>,
    tan_up: f32,
    fwd: vec3<f32>,
    tan_down: f32,
    rect: vec4<u32>,
    aspect: f32,
    // (L + R) / unit and L / unit, rounded once on the CPU.
    unit_scale: f32,
    unit_offset: f32,
    _pad0: f32,
    // The rays' sub-pixel offset in internal pixels (`View::jitter`).
    jitter: vec2<f32>,
    _pad1: vec2<f32>,
};

const MAX_VIEWS: u32 = 2u;

struct Params {
    views: array<View, 2>,
    view_count: u32,
    translucent_layers: u32,
    frame: u32,
    lighting: u32,
    window_min: vec3<i32>,
    // A field-surface hit's shading normal is mixed this far toward its entry face's normal
    // (`SurfaceTuning::face_mix`); 0 keeps the field's.
    face_mix: f32,
    window_size: vec3<i32>,
    _pad3: i32,
    light_pos: vec3<f32>,
    light_intensity: f32,
    light_dir: vec3<f32>,
    cos_outer: f32,
    size: vec2<u32>,
    cos_inner: f32,
    flags: u32,
    units_per_shard: u32,
    forced_level: u32,
    lod1_r2: i32,
    lod2_r2: i32,
    exposure: f32,
    // The emissive table (`Lighting::table_cells`): bit 0 the temporal pass reads it (off only in
    // a test), bits 14..28 C_CAP.
    cache: u32,
    candidates: u32,
    // The slot of light_samples where the frame's candidate table starts (light.wgsl).
    light_table_base: u32,
    // Bit 0: the previous frame's hit buffer and history are valid.
    history: u32,
    _pad5: u32,
    _pad6: u32,
    _pad7: u32,
    // The views of the previous frame, for reprojection.
    prev_views: array<View, 2>,
    // The lighting grid: its size, the full-resolution pixels per lighting pixel on an edge
    // (1 or 2), and this and last frame's offsets of the sampled pixel in each 2 x 2 (x in
    // bits 0..8, y in bits 8..16).
    light_size: vec2<u32>,
    light_scale: u32,
    light_offset: u32,
    prev_light_offset: u32,
    // The table's cells less one (a power of two less one): a slot index's mask.
    cache_mask: u32,
    // The size of the frame's image (the upscale pass's output); `size` without the upscaler.
    output_size: vec2<u32>,
    // The reopened fill (`Lighting::fill_samples`, `fill_until`): F in bits 0..8, C_FILL in
    // bits 8..21; F 0 is off.
    fill: u32,
    // The face look's far shading normal (`LookParams::far_normal`), in pixels per voxel:
    // `far_hi <= far_lo` is off.
    far_lo: f32,
    far_hi: f32,
    _pad9: u32,
    // FSR 2's cap on an emitting hit's pre-exposed colour (`Fsr2Settings::emitter_cap`; 0 is
    // none) and its tone pass's static dither (1 on), both 0 without FSR 2.
    emitter_cap: f32,
    dither: u32,
    _tail0: u32,
    _tail1: u32,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> brick_pool0: array<u32>;
@group(0) @binding(2) var<storage, read> chunk_records: array<u32>;
@group(0) @binding(3) var<storage, read> chunk_table: array<u32>;
@group(0) @binding(4) var<storage, read> materials: array<vec4<f32>>;
@group(0) @binding(7) var<storage, read> brick_pool1: array<u32>;
@group(0) @binding(8) var<storage, read> brick_pool2: array<u32>;
@group(0) @binding(9) var<storage, read> brick_pool3: array<u32>;
@group(0) @binding(11) var<storage, read> emission_scale: array<f32>;
@group(0) @binding(12) var blue_noise: texture_2d<f32>;

// `ExposureState` in exposure.rs: the exposure this frame uses in auto (`current`) and the one
// the last frame used (`previous`), then the adapted and metered levels, a counter and flags.
struct ExposureState {
    current: f32,
    previous: f32,
    adapted_log2: f32,
    metered_log2: f32,
    frame: u32,
    flags: u32,
    _pad0: u32,
    _pad1: u32,
};

@group(0) @binding(13) var<uniform> exposure_state: ExposureState;
@group(1) @binding(0) var hit_pos_out: texture_storage_2d<rgba32float, write>;
@group(1) @binding(1) var hit_info_out: texture_storage_2d<rgba16uint, write>;
@group(1) @binding(2) var layer_out: texture_storage_2d<rgba16float, write>;

// The split field walk (FLAG_FIELD_SPLIT): the hit pipeline walks every ray as `march` does,
// its cells cubes, until the ray enters a full chunk's brick that the field can hit in (a band
// brick without BAND_CUBES_BIT), and appends the ray there to `split_list` with what the field
// walk resumes from; `field_resume`, in a dispatch of `split_args.groups` workgroups, walks
// the field from it. A ray entry: the pixel (x in bits 0..16, y in 16..32), `t` as bits, the
// entered voxel relative to the window's first voxel (x, y in the third word, z and the
// entry axis plus one in the fourth). The counts start at zero each frame (the frame clears
// them); the first pixel writes the dispatch's y and z.
struct SplitList {
    count: atomic<u32>,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    rays: array<vec4<u32>>,
};
struct SplitArgs {
    groups: atomic<u32>,
    y: u32,
    z: u32,
};
@group(1) @binding(3) var<storage, read_write> split_list: SplitList;
@group(1) @binding(4) var<storage, read_write> split_args: SplitArgs;
// `field_resume`'s workgroup size (marcher.rs: SPLIT_GROUP).
const SPLIT_GROUP: u32 = 128u;
// A `march` miss's material when the split walk hands the ray on.
const SPLIT_HANDOFF: u32 = 0xFFFFFFFFu;

const ABSENT: u32 = 0xFFFFFFFFu;
const FULL_BIT: u32 = 0x80000000u;
const LOD_BIT: u32 = 0x40000000u;
const EMPTY_BIT: u32 = 0x20000000u;
const RECORD_MASK: u32 = 0x1FFFFFFFu;
const TRANSLUCENT_BIT: u32 = 0x20000000u;
const MATERIAL_MASK: u32 = 0xFFFFu;
// A Full entry's first unit and class (gpu_world.rs: UNIT_MASK, CLASS_SHIFT, CLASS_WIDE).
const UNIT_MASK: u32 = 0x3FFFFFFu;
const CLASS_SHIFT: u32 = 26u;
const CLASS_WIDE: u32 = 5u;
const CLASS_UNIFORM: u32 = 6u;
// MaterialFlags::TRANSLUCENT, in the material row's flag bits.
const MATERIAL_TRANSLUCENT: u32 = 2u;
// u32 words of a pool unit and of a chunk record, and the record word where the brick entries
// start; the slot words of the header, its translucent words, and the payload.
const UNIT_WORDS: u32 = 16u;
const RECORD_WORDS: u32 = 528u;
const ENTRY_WORD: u32 = 16u;
const HEADER_WORD: u32 = 16u;
const HEADER_TRANSLUCENT_WORD: u32 = 24u;
const PAYLOAD_WORD: u32 = 32u;
const MAX_STEPS: u32 = 4096u;
const BIG: f32 = 3.0e38;
// Params.flags bits.
const FLAG_SHADOWS: u32 = 1u;
const FLAG_MASKS: u32 = 2u;
const NORMAL_SHIFT: u32 = 2u;
const VIEW_SHIFT: u32 = 5u;
const VIEW_BITS: u32 = 7u;
const FLAG_SKIPS: u32 = 512u;
const FLAG_CUTAWAY: u32 = 1024u;
// The field look's split walk: the hit pipeline's `main` walks to the first field brick and
// hands the ray to `field_resume` (`split_pixel`).
const FLAG_FIELD_SPLIT: u32 = 524288u;
// Normal modes (flags bits 2..4) and views (flags bits 5..8).
const NORMAL_FACE: u32 = 0u;
const NORMAL_TRILINEAR8: u32 = 1u;
const NORMAL_GRADIENT27: u32 = 2u;
const NORMAL_CONTOUR: u32 = 3u;
const VIEW_SHADED: u32 = 0u;
const VIEW_NORMALS: u32 = 1u;
const VIEW_HITMASK: u32 = 2u;
const VIEW_STEPS: u32 = 3u;
const VIEW_ALBEDO: u32 = 4u;
const VIEW_CRACKS: u32 = 5u;
// Params.lighting bits.
const LIGHTING_LAMP: u32 = 1u;
const LIGHTING_COARSE_SHADOW: u32 = 2u;
const LIGHTING_GRADIENT: u32 = 8u;
const LIGHTING_FILTER_OFF: u32 = 32u;
const LIGHTING_EMISSION_CHANGED: u32 = 256u;
const LIGHTING_LAMP_CHANGED: u32 = 512u;
// The table's guard runs this frame (`marcher::LIGHTING_GUARD`, `GUARD_WINDOW`).
const LIGHTING_GUARD: u32 = 1024u;
const LIGHTING_IDENTITY_TONE_MAP: u32 = 64u;
// The exposure is automatic: this frame's is the state's `current`, not `params.exposure`.
const LIGHTING_AUTO_EXPOSURE: u32 = 8192u;
// Hit record flags.
const HIT_FOUND: u32 = 1u;
const HIT_FIELD: u32 = 2u;
const HIT_TRANSLUCENT: u32 = 4u;
const HIT_AXIS_SHIFT: u32 = 3u;
const PI: f32 = 3.14159265358979;

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

// Bit planes of a planar class: 0, 1, 2 or 4 for classes 1 to 4, none for a uniform band
// brick's slot (class 6, laid out as class 1).
fn class_planes(cls: u32) -> u32 {
    return select(select(cls - 1u, 4u, cls == 4u), 0u, cls == CLASS_UNIFORM);
}

// Units of a Full entry's payload: its planes, or 16 of raw ids for class Wide.
fn payload_units(entry: u32) -> u32 {
    let cls = (entry >> CLASS_SHIFT) & 7u;
    return select(class_planes(cls), 16u, cls == CLASS_WIDE);
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

// The lamp's irradiance at `p` before any surface term: intensity over squared distance inside
// the cone, the terms `shade` applies to an opaque hit.
fn lamp(p: vec3<f32>) -> f32 {
    let to_light = params.light_pos - p;
    let dist2 = max(dot(to_light, to_light), 1e-4);
    let l = to_light * inverseSqrt(dist2);
    let cone = smoothstep(params.cos_outer, params.cos_inner, dot(-l, params.light_dir));
    return params.light_intensity * cone / dist2;
}

// The translucent walk: along `o + d t` for `0 <= t < t_end` (the opaque hit's `t`, or BIG on
// a miss), the same DDA as `march` over `tlookup`'s cells, compositing the translucent voxels
// it enters front to back, nearest first, up to `params.translucent_layers` of them. Returns
// the accumulated colour, premultiplied, in xyz and the transmittance left for what lies
// behind in w. A layer is albedo times opacity, under the lamp's cone and distance term when
// `lit` and unlit otherwise; a thick volume contributes one layer per voxel entered.
fn translucent_march(o: vec3<f32>, d: vec3<f32>, t_end: f32, lit: bool) -> vec4<f32> {
    var acc = vec4<f32>(0.0, 0.0, 0.0, 1.0);
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
        return acc;
    }
    reset_walk();
    var t = max(t0, 0.0);
    var v = voxel_at(o + d * t, d);
    if (t0 > 0.0) {
        let axis = select(select(2, 1, tmin.y == t0), 0, tmin.x == t0);
        let face = select(bmax_i[axis], bmin_i[axis], d[axis] > 0.0);
        v = advance(o, d, inv, t, axis, face);
    }
    v = clamp(v, bmin_i, bmax_i - vec3<i32>(1));
    var layers = 0u;

    for (var step = 0u; step < MAX_STEPS; step++) {
        if (any(v < bmin_i) || any(v >= bmax_i) || t >= t_end) {
            return acc;
        }
        let c = tlookup(v);
        var size = c.size;
        if (size == 0) {
            let row = c.material * 4u;
            let albedo = materials[row].xyz;
            let opacity = clamp(materials[row + 2u].z, 0.0, 1.0);
            var light = 1.0;
            if (lit) {
                light = lamp(o + d * t);
            }
            acc = vec4<f32>(acc.xyz + acc.w * opacity * albedo * light, acc.w * (1.0 - opacity));
            layers += 1u;
            if (layers >= params.translucent_layers) {
                return acc;
            }
            size = c.cell;
        }
        let cell_min = floor_div(v, size) * size;
        let edge = cell_min + select(vec3<i32>(0), vec3<i32>(size), d > vec3<f32>(0.0));
        let tk = select(vec3<f32>(BIG), (vec3<f32>(edge) - o) * inv, d != vec3<f32>(0.0));
        var k = 0;
        if (tk.y < tk[k]) { k = 1; }
        if (tk.z < tk[k]) { k = 2; }
        t = tk[k];
        v = advance(o, d, inv, t, k, edge[k]);
    }
    return acc;
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

// The cutaway view's miss colour, linear: a flat grey-blue the rooms read against.
const CUTAWAY_SKY: vec3<f32> = vec3<f32>(0.30, 0.38, 0.50);

// A packed lighting-grid offset (x in bits 0..8, y in 8..16).
fn unpack_offset(o: u32) -> vec2<u32> {
    return vec2<u32>(o & 0xFFu, (o >> 8u) & 0xFFu);
}

// The full-resolution pixel lighting pixel `lp` samples this frame, inside the target.
fn full_px(lp: vec2<u32>) -> vec2<u32> {
    return min(lp * params.light_scale + unpack_offset(params.light_offset), params.size - vec2<u32>(1u));
}

// A fractional full-resolution pixel position of last frame (centres at + 0.5) on last
// frame's lighting grid, centres at + 0.5.
fn prev_light_grid(q: vec2<f32>) -> vec2<f32> {
    let o = vec2<f32>(unpack_offset(params.prev_light_offset));
    return (q - 0.5 - o) / f32(params.light_scale) + 0.5;
}

// Whether lighting pixel `q` lies on the lighting grid.
fn on_light_grid(q: vec2<i32>) -> bool {
    return all(q >= vec2<i32>(0)) && all(q < vec2<i32>(params.light_size));
}

// The ray of one pixel.
struct Ray {
    o: vec3<f32>,
    d: vec3<f32>,
};

// The index of the first view whose rectangle holds pixel `px`, 0 when none does.
fn view_of(px: vec2<u32>) -> u32 {
    for (var i = 0u; i < min(params.view_count, MAX_VIEWS); i++) {
        let r = params.views[i].rect;
        if (all(px >= r.xy) && all(px < r.xy + r.zw)) {
            return i;
        }
    }
    return 0u;
}

// The primary ray of pixel `px` through the centre of the pixel moved by the view's jitter
// (zero: the centre itself, bit for bit), in its view. Horizontal
// tangents are taken in units of `unit = hv aspect` (hv the mean vertical tangent): with
// `u = (px - x + 0.5) / w`, `x = ((u (L + R) / unit - L / unit) hv) aspect`, the two ratios
// rounded on the CPU (`View::unit_scale`, `unit_offset`). A symmetric
// view of `View::of_camera` has the factors 2 and 1 exactly, so it rounds as
// `((2 u - 1) tan) aspect`; two views of one aspect whose rectangles cover the same pixels of
// the same frustum (a view and its left half over half the pixels) take bit-identical rays.
fn pixel_ray(px: vec2<u32>) -> Ray {
    let v = params.views[view_of(px)];
    let u = (vec2<f32>(px - v.rect.xy) + 0.5 + v.jitter) / vec2<f32>(v.rect.zw);
    let hv = (v.tan_up + v.tan_down) * 0.5;
    let cv = (v.tan_up - v.tan_down) * 0.5;
    let nx = u.x * v.unit_scale - v.unit_offset;
    let ny = u.y * 2.0 - 1.0;
    let d = normalize(v.fwd + v.right * (nx * hv * v.aspect) - v.up * (ny * hv - cv));
    return Ray(v.origin, d);
}

// Octahedral encoding of unit `n` into two snorm16 values (as u16 bits). The six axis
// normals encode to -1, 0 and 1 exactly, so they decode exactly.
fn octa16_encode(n: vec3<f32>) -> vec2<u32> {
    var p = n.xy / (abs(n.x) + abs(n.y) + abs(n.z));
    if (n.z < 0.0) {
        let s = select(vec2<f32>(-1.0), vec2<f32>(1.0), p >= vec2<f32>(0.0));
        p = (1.0 - abs(p.yx)) * s;
    }
    let q = vec2<i32>(round(clamp(p, vec2<f32>(-1.0), vec2<f32>(1.0)) * 32767.0));
    return bitcast<vec2<u32>>(q) & vec2<u32>(0xFFFFu);
}

fn octa16_decode(e: vec2<u32>) -> vec3<f32> {
    let q = bitcast<vec2<i32>>(e << vec2<u32>(16u)) >> vec2<u32>(16u);
    let p = vec2<f32>(q) / 32767.0;
    var n = vec3<f32>(p, 1.0 - abs(p.x) - abs(p.y));
    if (n.z < 0.0) {
        let s = select(vec2<f32>(-1.0), vec2<f32>(1.0), n.xy >= vec2<f32>(0.0));
        n = vec3<f32>((1.0 - abs(n.yx)) * s, n.z);
    }
    return normalize(n);
}

// One pixel's hit record, decoded from the hit buffer: `n` is the shading normal,
// `axis` the entry face's axis (-1: the ray started inside rock).
struct HitRecord {
    found: bool,
    field: bool,
    translucent: bool,
    axis: i32,
    material: u32,
    p: vec3<f32>,
    t: f32,
    n: vec3<f32>,
};

fn unpack_hit(pos: vec4<f32>, info: vec4<u32>) -> HitRecord {
    let flags = info.w;
    return HitRecord(
        (flags & HIT_FOUND) != 0u,
        (flags & HIT_FIELD) != 0u,
        (flags & HIT_TRANSLUCENT) != 0u,
        i32((flags >> HIT_AXIS_SHIFT) & 3u) - 1,
        info.z,
        pos.xyz,
        pos.w,
        octa16_decode(info.xy),
    );
}

// The shadow normal of a hit seen along `d`: a field-surface hit's field normal, unmixed
// (`face_unmixed`); else the entry face's normal, `-d` for a ray that started inside rock.
fn shadow_normal_of(rec: HitRecord, d: vec3<f32>) -> vec3<f32> {
    if (rec.axis < 0) {
        return select(-d, rec.n, rec.field);
    }
    let face = select(vec3<f32>(0.0), vec3<f32>(-sign(d[rec.axis])), axis_mask(rec.axis));
    if (rec.field) {
        return face_unmixed(rec.n, face);
    }
    return face;
}

// Where the shadow rays of a hit at `p` with shadow normal `ns` start: SHADOW_OFFSET along
// `ns`. The shadow rays walk the voxels (`march`), crisp and smooth alike solid, and a
// field-surface hit lies up to half a voxel off its voxels' faces, so the offset is a step out
// of the voxels' rock as much as off the surface.
fn shadow_origin(p: vec3<f32>, ns: vec3<f32>) -> vec3<f32> {
    return p + ns * SHADOW_OFFSET;
}

// `p` projected through previous view `v` to a fractional pixel of the target (pixel
// centres at integer + 0.5, the view's jitter ignored); a negative x when it lies behind the
// view.
fn project_prev(v: View, p: vec3<f32>) -> vec2<f32> {
    let dir = p - v.origin;
    let z = dot(dir, v.fwd);
    if (z <= 1e-4) {
        return vec2<f32>(-1.0);
    }
    let x = dot(dir, v.right) / z;
    let y = dot(dir, v.up) / z;
    let u = (x + v.tan_left) / (v.tan_left + v.tan_right);
    let w = (v.tan_up - y) / (v.tan_up + v.tan_down);
    return vec2<f32>(v.rect.xy) + vec2<f32>(u, w) * vec2<f32>(v.rect.zw);
}

// The pixel of the previous frame a hit `p` of this frame's pixel reprojects to, as if both
// frames' rays went through their pixels' centres: `p` projected through previous view `v`,
// less this frame's jitter `j` (the offset of this pixel's ray from its centre). On a still
// camera every pixel maps onto its own centre whatever the two jitters, so history is never
// resampled across the moving sample lattice; with the jitter off it is `project_prev`. A
// negative x when `p` lies behind the view.
fn reproject_centred(v: View, j: vec2<f32>, p: vec3<f32>) -> vec2<f32> {
    let q = project_prev(v, p);
    if (q.x < 0.0) {
        return q;
    }
    return q - j;
}

// The emissive table (ADR-0043, `Lighting::table_cells`): `params.cache_mask + 1` cells of
// CACHE_WORDS u32s, in a storage buffer of its own. A cell: the key in words 0 and 1
// (`cache_key`; word 1 is 0 in a free cell), the count of samples, the three sums of the
// absolute emissive irradiance in Q17.15, then the kept emitter's face key (two words) and its
// packed luminance (light.wgsl's `cache_offer`, the guard's), and a frame stamp (reserved). A key is a voxel and an
// entry face (2 axis + 1 toward +axis) and lives in the first of CACHE_PROBES slots from
// `cache_slot` holding it. The table's counters live in a second buffer (`CacheStats`).
const CACHE_WORDS: u32 = 10u;
const CACHE_PROBES: u32 = 4u;
const CACHE_Q: f32 = 32768.0;
// The cell's word of the count; the sums follow it.
const CACHE_COUNT: u32 = 2u;
// The cell's stamp word: CACHE_REOPENED, set where an edit box or the guard reopened the cell
// and cleared once the fill has brought it to C_FILL, over the frame (its low 31 bits) the
// light pass last listed it for the fill (cache_fill.wgsl).
const CACHE_STAMP: u32 = 9u;
const CACHE_REOPENED: u32 = 0x80000000u;

// The reopened fill's samples a frame per listed cell (F, 0 off) and the count at which a
// reopened cell leaves the list (C_FILL).
fn fill_samples() -> u32 {
    return params.fill & 0xFFu;
}

fn fill_until() -> u32 {
    return (params.fill >> 8u) & 0x1FFFu;
}

// The table's counters: lighting pixels whose key found neither its cell nor a free slot this
// frame (cleared each encode), the cells holding a key, and the cells the guard reopened
// (gradient.wgsl) since the table was last cleared.
struct CacheStats {
    misses: atomic<u32>,
    occupied: atomic<u32>,
    reopens: atomic<u32>,
    // The last re-shade reopen's kept luminance, as f32 bits (a reading; racing stores).
    last_e_old: u32,
    // The reopens by the guard's branch: a kept emitter's re-shade out of the dead zone, a
    // torn tag whose key lights nothing, a dark capped cell whose fresh sample drew light.
    reopens_by: array<atomic<u32>, 3>,
    // The last re-shade reopen's new luminance, as f32 bits.
    last_e_new: u32,
    // The reopened fill's list for this frame (cache_fill.wgsl; cleared each encode with the
    // fill on): the cells offered (past FILL_LIST_MAX they are dropped, and the excess is their
    // count), three words of padding, then the listed slots.
    fill_offered: atomic<u32>,
    _fill_pad: array<u32, 3>,
    fill_cells: array<u32>,
};

// Cells the fill list holds; `marcher::FILL_LIST_MAX`.
const FILL_LIST_MAX: u32 = 16384u;

fn cache_read_on() -> bool {
    return (params.cache & 1u) != 0u;
}

fn cache_c_cap() -> u32 {
    return (params.cache >> 14u) & 0x3FFFu;
}

fn cache_mix(v: vec3<i32>, face: u32, a: u32, b: u32) -> u32 {
    let c = bitcast<vec3<u32>>(v);
    var h = (c.x * a) ^ (c.y * 0xD8163841u) ^ (c.z * b) ^ ((face + 1u) * 0x165667B1u);
    h ^= h >> 15u;
    h *= 0x2C1B3C6Du;
    h ^= h >> 12u;
    h *= 0x297A2D39u;
    h ^= h >> 15u;
    return h;
}

// The first slot of the key (`marcher::cache_slot`).
fn cache_slot(v: vec3<i32>, face: u32) -> u32 {
    return cache_mix(v, face, 0x8DA6B343u, 0xCB1AB31Fu) & params.cache_mask;
}

// The key's two words (`marcher::cache_key`): each coordinate's low 20 bits (the key wraps
// every 2^20 voxels, past any light radius), x then y's low 12 bits in word 0; y's high 8
// bits, z, the face in bits 28..31 and a valid bit 31 in word 1, so word 1 is never 0.
fn cache_key(v: vec3<i32>, face: u32) -> vec2<u32> {
    let c = bitcast<vec3<u32>>(v) & vec3<u32>(0xFFFFFu);
    return vec2<u32>(c.x | (c.y << 20u), (c.y >> 12u) | (c.z << 8u) | (face << 28u) | 0x80000000u);
}

// The voxel of a key's two words: each 20-bit coordinate sign-extended.
fn cache_key_voxel(k: vec2<u32>) -> vec3<i32> {
    let x = k.x & 0xFFFFFu;
    let y = (k.x >> 20u) | ((k.y & 0xFFu) << 12u);
    let z = (k.y >> 8u) & 0xFFFFFu;
    return bitcast<vec3<i32>>(vec3<u32>(x, y, z) << vec3<u32>(12u)) >> vec3<u32>(12u);
}

// Whether a hit takes the table's path: an opaque cube face entered from air.
fn cacheable(rec: HitRecord) -> bool {
    return rec.found && rec.axis >= 0 && !rec.field && !rec.translucent;
}

// The voxel behind hit point `p` with face normal `nf`.
fn cache_voxel(p: vec3<f32>, nf: vec3<f32>) -> vec3<i32> {
    return vec3<i32>(floor(p - 0.5 * nf));
}

// The entry face of a hit on `axis` with face normal `nf`.
fn cache_face(axis: i32, nf: vec3<f32>) -> u32 {
    return 2u * u32(axis) + select(0u, 1u, nf[axis] > 0.0);
}

// A 32-bit integer hash (Wellons' lowbias32).
fn hash_u32(x: u32) -> u32 {
    var h = x;
    h ^= h >> 16u;
    h *= 0x7feb352du;
    h ^= h >> 15u;
    h *= 0x846ca68bu;
    h ^= h >> 16u;
    return h;
}

// Random number `dim` of pixel `px` in this frame, in [0, 1). The blue-noise texel at `px`
// shifted by an offset hashed from `dim` (blue in space), plus a Weyl rotation by the frame
// times a per-dimension multiple of the golden ratio in 32-bit fixed point (low-discrepancy
// in time). A function of (pixel, frame, dim) alone, so a frame is deterministic on one
// adapter.
fn rand(px: vec2<u32>, dim: u32) -> f32 {
    let h = hash_u32(dim * 0x9E3779B9u + 0x632BE5ABu);
    let at = (px + vec2<u32>(h & 127u, (h >> 7u) & 127u)) & vec2<u32>(127u);
    let texel = u32(textureLoad(blue_noise, vec2<i32>(at), 0).x * 255.0 + 0.5);
    let v = (texel << 24u) + params.frame * (0x9E3779B9u * (dim + 1u)) + ((h >> 14u) & 0xFFFFFFu);
    return f32(v >> 8u) / 16777216.0;
}

// The exposure this frame's passes use: the state's `current` in auto, else the manual
// `params.exposure`.
fn exposure_now() -> f32 {
    if ((params.lighting & LIGHTING_AUTO_EXPOSURE) != 0u) {
        return exposure_state.current;
    }
    return params.exposure;
}

// The ratio of this frame's exposure to the last frame's: a pass that reads a value an earlier
// frame stored pre-exposed multiplies it by this (its square for a second moment).
fn exposure_ratio() -> f32 {
    return exposure_now() / exposure_state.previous;
}

// Every store into an rgba16float HDR target is at most this (ADR-0033), and a second moment is
// of a luminance at most MOMENT_LUM_MAX (its square, 65 025, stays finite in f16).
const HDR_MAX: f32 = 60000.0;
const MOMENT_LUM_MAX: f32 = 255.0;

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// Material ids the emission-scale buffer holds per half (`marcher::EMISSION_ROWS`).
const EMISSION_ROWS: u32 = 65536u;
// The half of the emission-scale buffer `emission_of` reads: 0 for the pixels' light,
// EMISSION_ROWS once an entry point (the probe trace) sets it for the probe rays' light.
var<private> emission_base: u32 = 0u;

// A material's emitted radiance under the frame's emission scale.
fn emission_of(m: u32) -> vec3<f32> {
    return materials[m * 4u + 1u].xyz * emission_scale[emission_base + m];
}

// ---- The field surface (the Contour normal mode, the hit pipeline with FIELD_SURFACE) ----
//
// ADR-0041: the surface is the zero set of `phi = count / 27 - 1/2` trilinearly interpolated
// over the voxel centres, `count` a voxel's solid voxels among the 27 around it, itself
// included (solid is not air, a translucent voxel included). A band brick's header names its
// band slot (HEADER_BAND_WORD: HEADER_BAND_BIT and the slot's first unit; a uniform band brick
// has a class 6 slot for it): the band mask (unit 0, the opaque mask's layout), then the band
// voxels' u8 counts in band-index order, the k-th in byte k % 4 of word k / 4 of unit 1 on;
// the brick's header holds its band prefix (HEADER_BAND_PREFIX_WORD, `layer_prefix`), so a
// voxel's band index is its layer's prefix and the band bits below it in its layer. A
// band voxel's count is read there, any other voxel's is 27 when it is solid and 0 when it is
// air. The walk is `march`'s DDA over the cells that hold a band voxel (0 < count < 27) or a
// solid one: a band brick's band mask ORed with its opaque mask (all set for a uniform solid
// brick, a translucent one included), and a chunk or a group of bricks is skipped only when
// its brick bits are clear. In a band
// voxel the ray's segment through the voxel is split at the voxel's three mid-planes into its
// pieces in the dual cells it crosses (the cubes between eight voxel centres, at most four),
// and on a piece whose cell's eight corners mix signs `phi` along the ray is a cubic in `t`:
// the cubic's extrema split the piece into monotone runs, and the first run that rises to
// zero holds the hit, found by eight steps of the Illinois method. A piece that starts at or
// above zero is a hit at its start. A leak is a ray that reaches a solid voxel outside the
// band, a piece whose corners all lie above zero, or a piece that starts more than
// FIELD_LEAK_EPS above zero: each is hit there as a cube and flagged, in the cracks view's
// place. A LOD-only chunk is walked as `march` walks it, its cells cubes.
//
// A crisp voxel (a voxel of a `MaterialFlags::CRISP` row, its band count carrying
// CRISP_COUNT_BIT) counts as solid in the field and is hit as its cube where the ray enters
// it, with its entry face's normal; no field piece inside it is tested. A dual cell with a
// crisp corner gives a field hit only when one of its corners carries SMOOTH_COUNT_BIT (a
// smooth solid band voxel), so two crisp faces meeting in a concave corner stay a sharp
// corner, and a field hit takes the material of its largest-weight smooth solid corner. A band
// brick whose header carries BAND_CUBES_BIT (no smooth solid voxel in it or its shell, set by
// the field pass) can give no field hit and is walked as `march` walks a brick, its solid
// voxels cubes.
//
// The shading normal is the normalised gradient of the trilinear `phi` at the hit, mixed
// toward the entry face by the face mix (`face_mixed`). The hit record flags a field hit
// (HIT_FIELD), so the light pass starts its shadow rays along the field normal
// (`shadow_normal_of`); they walk the voxels (`march`), not the field. Translucent voxels are
// solid to the field, and the field walk composites no translucent layer.

override FIELD_SURFACE: bool = false;

const HEADER_BAND_WORD: u32 = 15u;
const HEADER_BAND_BIT: u32 = 0x80000000u;
const HEADER_BAND_PREFIX_WORD: u32 = 12u;
const BAND_CUBES_BIT: u32 = 0x40000000u;
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
// A crisp solid voxel's count byte carries this bit (`field::CRISP_COUNT_BIT`), a smooth solid
// voxel's SMOOTH_COUNT_BIT (`field::SMOOTH_COUNT_BIT`); the count is in the low five bits.
const CRISP_COUNT_BIT: u32 = 0x80u;
const SMOOTH_COUNT_BIT: u32 = 0x40u;

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

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.size.x || gid.y >= params.size.y) {
        return;
    }
    if (FIELD_SURFACE) {
        field_pixel(gid.xy, false, 0.0, vec3<i32>(0), -1);
        return;
    }
    if ((params.flags & FLAG_FIELD_SPLIT) != 0u) {
        split_pixel(gid.xy);
        return;
    }
    let ray = pixel_ray(gid.xy);
    let o = ray.o;
    let d = ray.d;
    walk_steps = 0u;
    loads = 0u;
    let h = march(o, d, BIG, (params.flags & FLAG_CUTAWAY) != 0u);
    // Read before the normal modes' lookups touch the walk and the counters.
    let translucent_seen = walk.saw_translucent && params.translucent_layers > 0u;
    let counts = vec3<u32>(min(walk_steps, 255u), min(counted_loads(), 255u), min(counted_pool_reads(), 255u));
    let t_end = select(BIG, h.t, h.found);
    let view = view_mode();
    let px = vec2<i32>(gid.xy);
    var pos = vec4<f32>(0.0, 0.0, 0.0, BIG);
    var info = vec4<u32>(0u);
    if (h.found) {
        let n = surface_normal(params.views[view_of(gid.xy)], o, d, h);
        let flags = HIT_FOUND | (u32(h.axis + 1) << HIT_AXIS_SHIFT);
        pos = vec4<f32>(o + d * h.t, h.t);
        info = vec4<u32>(octa16_encode(n), h.material, flags);
    }
    var layer = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    if (view == VIEW_STEPS) {
        // The primary ray's cost: its loop iterations, its storage loads and the pool loads
        // among them, each clamped at 255. No translucent walk in this view.
        layer = vec4<f32>(vec3<f32>(counts), 1.0);
    } else if (view == VIEW_CRACKS) {
        // The voxel walk has no leak. No translucent walk in this view.
        layer = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    } else if (translucent_seen) {
        info.w |= HIT_TRANSLUCENT;
        layer = translucent_march(o, d, t_end, view != VIEW_ALBEDO);
    }
    textureStore(hit_pos_out, px, pos);
    textureStore(hit_info_out, px, info);
    textureStore(layer_out, px, layer);
}

// The split walk's pixel (FLAG_FIELD_SPLIT, in the hit pipeline without FIELD_SURFACE): the ray
// walks as `march` walks, every solid voxel a cube hit recorded as `field_pixel` records one
// (its entry face's normal, the voxel's material, no translucent walk), until it enters a field
// brick; there it is appended to `split_list` and its record is left to `field_resume`. The
// cracks and steps views do not split (marcher.rs).
fn split_pixel(gid: vec2<u32>) {
    if (all(gid == vec2<u32>(0u))) {
        split_args.y = 1u;
        split_args.z = 1u;
    }
    let ray = pixel_ray(gid);
    let d = ray.d;
    split_walk = true;
    let h = march(ray.o, d, BIG, false);
    split_walk = false;
    if (!h.found && h.material == SPLIT_HANDOFF) {
        let k = atomicAdd(&split_list.count, 1u);
        if (k % SPLIT_GROUP == 0u) {
            atomicAdd(&split_args.groups, 1u);
        }
        let r = vec3<u32>(h.voxel - params.window_min * 64);
        split_list.rays[k] = vec4<u32>(
            gid.x | (gid.y << 16u),
            bitcast<u32>(h.t),
            r.x | (r.y << 16u),
            r.z | (u32(h.axis + 1) << 16u),
        );
        return;
    }
    var pos = vec4<f32>(0.0, 0.0, 0.0, BIG);
    var info = vec4<u32>(0u);
    if (h.found) {
        let flags = HIT_FOUND | (u32(h.axis + 1) << HIT_AXIS_SHIFT);
        pos = vec4<f32>(ray.o + d * h.t, h.t);
        info = vec4<u32>(octa16_encode(face_normal(d, h)), h.material, flags);
    }
    let px = vec2<i32>(gid);
    textureStore(hit_pos_out, px, pos);
    textureStore(hit_info_out, px, info);
    textureStore(layer_out, px, vec4<f32>(0.0, 0.0, 0.0, 1.0));
}

// The split walk's field half: one ray of `split_list` per invocation, the field walk from
// its hand-off, its record written as `field_pixel` writes it.
@compute @workgroup_size(128, 1, 1)
fn field_resume(@builtin(global_invocation_id) gid: vec3<u32>) {
    let k = gid.x;
    if (k >= atomicLoad(&split_list.count)) {
        return;
    }
    let e = split_list.rays[k];
    let px = vec2<u32>(e.x & 0xFFFFu, e.x >> 16u);
    let r = vec3<u32>(e.z & 0xFFFFu, e.z >> 16u, e.w & 0xFFFFu);
    let v = vec3<i32>(r) + params.window_min * 64;
    field_pixel(px, true, bitcast<f32>(e.y), v, i32(e.w >> 16u) - 1);
}
