// The emissive table's sweep (ADR-0043): when the camera's chunk changes, every cell whose
// key's voxel lies in a chunk beyond the light radius of the camera's chunk (squared chunk
// distance over `r2`, the light table's own bound, `MirrorConfig::light_radius`) is freed: its
// ten words cleared and the occupied counter lowered. Compiled after march.wgsl, whose key
// helpers it uses.
//
// One invocation per cell, in rows of at most a dispatch dimension's cap of groups.
//
// Bindings, group 2:
//   0  uniform  sweep       the camera's chunk and the squared radius
//   1  storage  cache_cells the table (light.wgsl)
//   2  storage  cache_stats the table's counters (`CacheStats`)

struct Sweep {
    chunk: vec3<i32>,
    r2: i32,
};

@group(2) @binding(0) var<uniform> sweep: Sweep;
@group(2) @binding(1) var<storage, read_write> cache_cells: array<u32>;
@group(2) @binding(2) var<storage, read_write> cache_stats: CacheStats;

@compute @workgroup_size(64, 1, 1)
fn sweep_cells(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let slot = gid.x + gid.y * groups.x * 64u;
    if (slot > params.cache_mask) {
        return;
    }
    let word = CACHE_WORDS * slot;
    let k1 = cache_cells[word + 1u];
    if (k1 == 0u) {
        return;
    }
    let v = cache_key_voxel(vec2<u32>(cache_cells[word], k1));
    // The voxel's chunk (64 voxels an edge), floored: `>>` on i32 is arithmetic.
    let d = (v >> vec3<u32>(6u)) - sweep.chunk;
    if (dot(d, d) > sweep.r2) {
        for (var k = 0u; k < CACHE_WORDS; k++) {
            cache_cells[word + k] = 0u;
        }
        atomicSub(&cache_stats.occupied, 1u);
    }
}
