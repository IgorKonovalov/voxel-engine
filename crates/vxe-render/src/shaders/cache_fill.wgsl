// The reopened fill (ADR-0043, the dug hole): F emissive samples a frame for each cell an edit
// box or the guard reopened, from random points on its face, so a reopened face converges at F
// samples a frame however few pixels it covers. A face seen for the first time with no edit is
// never listed. Compiled after march.wgsl and light.wgsl, whose bindings (group 0, and group
// 2's `cache_cells` and `cache_stats`, which holds the list, at bindings 3 and 4), candidates,
// shadow ray and kept-emitter helpers it uses. Two entry points, both after the light pass (which lists the
// cells, `fill_offer`) and before the temporal pass:
//
//   fill_args  one invocation: the fill's indirect dispatch into `fill_dispatch` (group 3,
//              binding 0, a buffer of its own: a dispatch may not read its indirect words
//              from a buffer it binds for storage), `min(offered, FILL_LIST_MAX)` workgroups
//              by 1 by 1
//   fill       one workgroup of FILL_LANES lanes per listed cell; the lanes under
//              `fill_samples()` each draw one sample: a uniform point on the cell's face (its
//              key's voxel and entry face), the candidates resampled toward it, one shadow ray.
//              The sample is added to the cell (`cache_add`, to C_CAP) and offered as its kept
//              emitter (`cache_offer`). A cell at C_FILL leaves the list: its reopened bit is
//              cleared.
//
// Random numbers: `rand` at a pseudo-pixel hashed from the slot and the lane, so each lane of
// each cell draws its own sequence each frame.

// Lanes a workgroup runs: the most samples a frame a cell takes (`marcher::FILL_SAMPLES_MAX`).
const FILL_LANES: u32 = 16u;

@group(3) @binding(0) var<storage, read_write> fill_dispatch: array<u32, 3>;

@compute @workgroup_size(1, 1, 1)
fn fill_args() {
    fill_dispatch = array<u32, 3>(min(atomicLoad(&cache_stats.fill_offered), FILL_LIST_MAX), 1u, 1u);
}

@compute @workgroup_size(16, 1, 1)
fn fill(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    if (wg.x >= min(atomicLoad(&cache_stats.fill_offered), FILL_LIST_MAX) || lane >= fill_samples()) {
        return;
    }
    let slot = cache_stats.fill_cells[wg.x];
    let word = CACHE_WORDS * slot;
    let k = vec2<u32>(atomicLoad(&cache_cells[word]), atomicLoad(&cache_cells[word + 1u]));
    if ((k.y & 0x80000000u) == 0u) {
        return;
    }
    let v = cache_key_voxel(k);
    let face = (k.y >> 28u) & 7u;
    let nf = face_dir(face);
    let h = hash_u32(slot * 0x9E3779B9u ^ (lane + 1u) * 0x85EBCA6Bu);
    let px = vec2<u32>(h & 0xFFFFu, h >> 16u);
    let p = face_point(v, face, px);
    let r = draw_candidates(px, p, nf);
    let e = resolve_emissive(r, p, nf);
    cache_add(slot, e);
    if (luminance(e) > 0.0) {
        cache_offer(slot, v, face, r);
    }
    if (lane == 0u && atomicLoad(&cache_cells[word + CACHE_COUNT]) >= fill_until()) {
        atomicAnd(&cache_cells[word + CACHE_STAMP], ~CACHE_REOPENED);
    }
}
