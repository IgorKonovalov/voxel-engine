// The mirror's layout (ADR-0002, ADR-0004, ADR-0018): its word and bit constants and the
// decode that needs no binding. The walk (walk.wgsl) and the field pass (field.wgsl) read the
// pool through their own bindings and decode through these.

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

fn floor_div(a: vec3<i32>, b: i32) -> vec3<i32> {
    return vec3<i32>(floor(vec3<f32>(a) / f32(b)));
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

const HEADER_BAND_WORD: u32 = 15u;

const HEADER_BAND_BIT: u32 = 0x80000000u;

const HEADER_BAND_PREFIX_WORD: u32 = 12u;

const BAND_CUBES_BIT: u32 = 0x40000000u;

// A crisp solid voxel's count byte carries this bit (`field::CRISP_COUNT_BIT`), a smooth solid
// voxel's SMOOTH_COUNT_BIT (`field::SMOOTH_COUNT_BIT`); the count is in the low five bits.
const CRISP_COUNT_BIT: u32 = 0x80u;

const SMOOTH_COUNT_BIT: u32 = 0x40u;
