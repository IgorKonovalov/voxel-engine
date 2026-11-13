// The frame's edit boxes, which a pass declares as `edits` at its own binding (temporal.wgsl,
// upscale.wgsl, fsr2/input.wgsl), and the test of a point against them.

// Up to MAX_EDITS boxes, inclusive voxel bounds: box i is `boxes[2 i]` to `boxes[2 i + 1]`.
struct Edits {
    count: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    boxes: array<vec4<i32>, 128>,
};

// Whether `p` lies inside an edit box dilated by one voxel: the continuous span of voxels
// `min - 1 ..= max + 1`, faces included.
fn in_edit(p: vec3<f32>) -> bool {
    for (var i = 0u; i < edits.count; i++) {
        let lo = vec3<f32>(edits.boxes[2u * i].xyz - 1);
        let hi = vec3<f32>(edits.boxes[2u * i + 1u].xyz + 2);
        if (all(p >= lo) && all(p <= hi)) {
            return true;
        }
    }
    return false;
}
