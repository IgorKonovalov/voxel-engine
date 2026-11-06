// The bake's shared-seed forms (Plan 0059 Phase 4): helpers appended to the bake's module for
// every seed form but the independent one, whose module stays march.wgsl + light.wgsl + bake.wgsl
// alone. `bake::bake_source_for` replaces bake.wgsl's stream line with one of these keys, and for
// the lattice form also the hashed `rand`'s return with one that adds `bake_shift`.
//
// - chunk-side: one sequence per chunk and side, `bake_side_key(chunk, face)`.
// - global-side: one sequence per side for every chunk, `bake_side_key(0, face)`.
// - lattice: the global-side sequence, rotated per face (a Cranley-Patterson rotation) by the
//   Kronecker lattice of its two plane coordinates (`bake_lattice`, the R2 sequence's constants),
//   each random dimension by that point plus the dimension times the golden ratio's fraction.
//
// Each form's samples remain a function of the face's key, `job.seed` and the sample's index, so
// a result does not depend on the batching or on the jobs' order.

// The lattice form's rotation of this entry's face, set before its first sample; 0 otherwise.
var<private> bake_shift: f32;

fn bake_side_key(chunk: vec3<i32>, face: u32) -> u32 {
    let c = bitcast<vec3<u32>>(chunk);
    return hash_u32(c.x ^ hash_u32(c.y ^ hash_u32(c.z ^ hash_u32(face + 0x2545F491u))));
}

// The face's point of the lattice in [0, 1): `fract(a u + b w)` over its two plane coordinates,
// in 32-bit fixed point (`a`, `b` the R2 sequence's 0.7548776662 and 0.5698402910).
fn bake_lattice(v: vec3<i32>, face: u32) -> f32 {
    let ax = face / 2u;
    let p = bitcast<vec3<u32>>(v);
    let u = select(select(p.x, p.z, ax == 1u), p.y, ax == 0u);
    let w = select(select(p.y, p.x, ax == 1u), p.z, ax == 0u);
    return f32((u * 3242174889u + w * 2447445414u) >> 8u) / 16777216.0;
}
