// The mirror's layout (ADR-0002, ADR-0004, ADR-0018): the decode that needs no binding. Its
// word and bit constants are the prelude's (frame/compose.rs, from gpu_world/format.rs and
// field.rs): the table, record and entry bits, a Full entry's first unit and class
// (UNIT_MASK, CLASS_SHIFT, CLASS_WIDE, CLASS_UNIFORM), the u32 words of a pool unit and of a
// chunk record, the record word where the brick entries start, the slot words of the header,
// its translucent words and the payload, the band header's words and bits, the count bits of
// a crisp or smooth solid voxel's count byte (its count in the low five bits), and
// MaterialFlags' TRANSLUCENT and CRISP. The walk (walk.wgsl) and the field pass (field.wgsl)
// read the pool through their own bindings and decode through these.

fn floor_div(a: vec3<i32>, b: i32) -> vec3<i32> {
    return vec3<i32>(floor(vec3<f32>(a) / f32(b)));
}

// The shard and index of word `word` of the slot whose first unit is `unit`, over shards of
// `units_per_shard` units (a slot's words run on through its units, which lie in one page, so
// in one shard).
fn shard_index(unit: u32, word: u32, units_per_shard: u32) -> vec2<u32> {
    let shard = unit / units_per_shard;
    return vec2<u32>(shard, (unit - shard * units_per_shard) * UNIT_WORDS + word);
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
