// The emissive cache's edit-box reset (`Lighting::emissive_cache`): every face of every voxel in
// the frame's edit boxes, each dilated by one voxel, has its cell's count and sums cleared so it
// fills from young again. The cell keeps its check word: its key's slot stays its own. Compiled
// after march.wgsl, whose cell key helpers it uses.
//
// One invocation per voxel, numbered through the boxes in order; the dispatch is the sum of the
// dilated boxes' volumes over the workgroup's 64.
//
// Bindings, group 2:
//   0  uniform  edits       the frame's edit boxes (temporal.wgsl's `Edits`)
//   1  storage  cache_cells the table (light.wgsl)

struct Edits {
    count: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    boxes: array<vec4<i32>, 128>,
};

@group(2) @binding(0) var<uniform> edits: Edits;
@group(2) @binding(1) var<storage, read_write> cache_cells: array<atomic<u32>>;

fn clear_cell(v: vec3<i32>, face: u32) {
    let base = cache_slot(v, face);
    let check = cache_check(v, face);
    for (var i = 0u; i < CACHE_PROBES; i++) {
        let word = CACHE_WORDS * ((base + i) & (CACHE_CELLS - 1u));
        if (atomicLoad(&cache_cells[word]) == check) {
            for (var k = 1u; k < CACHE_WORDS; k++) {
                atomicStore(&cache_cells[word + k], 0u);
            }
            return;
        }
    }
}

@compute @workgroup_size(64, 1, 1)
fn clear_cells(@builtin(global_invocation_id) gid: vec3<u32>) {
    var index = gid.x;
    for (var b = 0u; b < edits.count; b++) {
        let lo = edits.boxes[2u * b].xyz - vec3<i32>(1);
        let size = vec3<u32>(edits.boxes[2u * b + 1u].xyz + vec3<i32>(1) - lo + vec3<i32>(1));
        let voxels = size.x * size.y * size.z;
        if (index < voxels) {
            let at = vec3<u32>(index % size.x, (index / size.x) % size.y, index / (size.x * size.y));
            let v = lo + vec3<i32>(at);
            for (var face = 0u; face < 6u; face++) {
                clear_cell(v, face);
            }
            return;
        }
        index -= voxels;
    }
}
