// The reopened fill (ADR-0043, the dug hole): F emissive samples a frame for each cell an edit
// box or the guard reopened, from random points on its face, so a reopened face converges at F
// samples a frame however few pixels it covers. A face seen for the first time with no edit is
// never listed. And the camera-radius preload: a sweep of the mirror's bricks around the eye
// that lists every lit face it finds, and a fill that brings each listed cell to C_PRE samples
// whatever the view. Compiled after march.wgsl and light.wgsl, whose bindings (group 0, and
// group 2's `cache_cells` and `cache_stats`, which holds both lists, at bindings 3 and 4),
// candidates, shadow ray and cell helpers (`fill_face`, `face_sample`, `cache_take`) it uses.
// Five entry points, all after the light pass (which lists the reopened cells, `fill_offer`)
// and before the temporal pass:
//
//   fill_args      one invocation: the fill's indirect dispatch into `fill_dispatch` (group 3,
//                  binding 0, a buffer of its own: a dispatch may not read its indirect words
//                  from a buffer it binds for storage), `min(offered, FILL_LIST_MAX)`
//                  workgroups by 1 by 1
//   fill           one workgroup of FILL_LANES lanes per listed cell; the lanes under
//                  `fill_samples()` each draw one sample (`fill_face`): a uniform point on the
//                  cell's face (its key's voxel and entry face), the candidates resampled
//                  toward it, one shadow ray, added to the cell (to C_CAP) and offered as its
//                  kept emitter. A cell at C_FILL leaves the list: its reopened bit is cleared.
//   preload_args   one invocation: swaps the preload's two lists (the last frame's output is
//                  this frame's input), and writes the preload fill's and the sweep's indirect
//                  dispatches into `preload_dispatch` (group 3, binding 2; words 0..3 and 3..6)
//   preload_fill   one workgroup per input entry. About the nearest `draw_cells` entries
//                  draw this frame, by the distance bands their entries carry
//                  (`preload_draws`), so which cells draw is a function of the list's set,
//                  never of the order its entries were appended in; a drawing entry's
//                  FILL_LANES lanes each draw one sample into the cell (`fill_face`). An entry whose cell lost its CACHE_PRELOADING bit (an
//                  edit box, the sweep or a whole-table clear emptied it) is dropped; one at
//                  C_PRE clears the bit and leaves; any other carries to the output list.
//   preload_sweep  one workgroup of 64 lanes per brick: the bricks of the frame's edit boxes
//                  within the radius first, then `pre_visit` offsets of the sweep's table from
//                  its cursor around the eye's brick. A brick of air, absent, LOD-only or
//                  uniform solid inside six uniform solid neighbours ends at once; otherwise
//                  each lane takes a row of 8 voxels and each opaque voxel's face toward air
//                  (the light pass's cacheable faces) is looked up (`cache_find`): a cell under
//                  C_PRE not yet listed is listed into the output list; a face with no cell
//                  draws `probe` samples and claims a cell only if one is lit, the table holds
//                  under `load_max` cells and this frame's light pass counted no miss, then
//                  takes the samples and is listed.
//
// Random numbers: `rand` at a pseudo-pixel hashed from the slot (or the key) and the lane (or
// the sample), so each lane of each cell draws its own sequence each frame.

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
    let h = hash_u32(slot * 0x9E3779B9u ^ (lane + 1u) * 0x85EBCA6Bu);
    fill_face(slot, cache_key_voxel(k), (k.y >> 28u) & 7u, vec2<u32>(h & 0xFFFFu, h >> 16u));
    if (lane == 0u && atomicLoad(&cache_cells[word + CACHE_COUNT]) >= fill_until()) {
        atomicAnd(&cache_cells[word + CACHE_STAMP], ~CACHE_REOPENED);
    }
}

// The preload's frame (`marcher::PreloadUniform`): the eye's brick, the edit boxes' bricks
// within the radius (`brick_list`, the first `edit_bricks`), the sweep's offsets in its table,
// the bricks it visits a frame (B, or `trickle` while `still` and the input list is empty),
// C_PRE (`until`), the cells the fill draws a frame (S_PRE / FILL_LANES), the occupied cells
// past which the sweep claims none (PRE_LOAD_MAX of the table) and the samples a face with no
// cell draws (`probe`, at most PRELOAD_PROBE_MAX).
struct Preload {
    eye_brick: vec3<i32>,
    edit_bricks: u32,
    offsets: u32,
    bricks: u32,
    trickle: u32,
    still: u32,
    until: u32,
    draw_cells: u32,
    load_max: u32,
    probe: u32,
    brick_list: array<vec4<i32>, 256>,
};

const PRELOAD_PROBE_MAX: u32 = 8u;

@group(3) @binding(1) var<uniform> preload: Preload;
@group(3) @binding(2) var<storage, read_write> preload_dispatch: array<u32, 6>;

@compute @workgroup_size(1, 1, 1)
fn preload_args() {
    let p = cache_stats.pre_parity & 1u;
    let n_in = min(atomicLoad(&cache_stats.pre_count[p]), PRELOAD_LIST_MAX);
    let draw = min(n_in, preload.draw_cells);
    cache_stats.pre_in = p;
    cache_stats.pre_n_in = n_in;
    cache_stats.pre_draw = draw;
    cache_stats.pre_parity = p ^ 1u;
    atomicStore(&cache_stats.pre_count[p ^ 1u], 0u);
    // The nearest `draw` entries draw: every entry of the distance bands under `pre_cut`, and
    // `pre_rest` of band `pre_cut`'s by hash (`preload_draws`).
    var cum = 0u;
    var cut = PRELOAD_BANDS;
    var rest = 0u;
    for (var b = 0u; b < PRELOAD_BANDS; b++) {
        let c = atomicLoad(&cache_stats.pre_hist[p][b]);
        if (cum + c > draw) {
            cut = b;
            rest = draw - cum;
            break;
        }
        cum += c;
    }
    cache_stats.pre_cut = cut;
    cache_stats.pre_rest = rest;
    for (var b = 0u; b < PRELOAD_BANDS; b++) {
        atomicStore(&cache_stats.pre_hist[p ^ 1u][b], 0u);
    }
    var visit = select(preload.bricks, preload.trickle, preload.still != 0u && n_in == 0u);
    visit = min(visit, preload.offsets);
    let cursor = select(0u, cache_stats.pre_cursor, cache_stats.pre_cursor < preload.offsets);
    cache_stats.pre_base = cursor;
    cache_stats.pre_visit = visit;
    cache_stats.pre_cursor = select(0u, (cursor + visit) % max(preload.offsets, 1u), preload.offsets > 0u);
    preload_dispatch = array<u32, 6>(
        n_in,
        1u,
        1u,
        preload.edit_bricks + visit,
        1u,
        1u,
    );
}

// Distance bands of PRELOAD_BAND_VOXELS from the eye's brick's centre, PRELOAD_BANDS of them
// (the last open): an entry's band orders the fill nearest first.
const PRELOAD_BANDS: u32 = 64u;
const PRELOAD_BAND_VOXELS: f32 = 2.0;
// An entry: the cell's slot in its low PRELOAD_SLOT_BITS bits, its band above.
const PRELOAD_SLOT_BITS: u32 = 26u;

fn preload_band(v: vec3<i32>) -> u32 {
    let eye = vec3<f32>(preload.eye_brick) * 8.0 + vec3<f32>(4.0);
    let d = length(vec3<f32>(v) + vec3<f32>(0.5) - eye);
    return min(u32(d / PRELOAD_BAND_VOXELS), PRELOAD_BANDS - 1u);
}

// Appends the cell `slot` of voxel `v`'s face to this frame's output list in `v`'s band; a full
// list drops it and clears its bit, so the sweep lists it again when it next passes.
fn preload_append(slot: u32, v: vec3<i32>) {
    let out = cache_stats.pre_in ^ 1u;
    let i = atomicAdd(&cache_stats.pre_count[out], 1u);
    if (i < PRELOAD_LIST_MAX) {
        let band = preload_band(v);
        cache_stats.pre_lists[out * PRELOAD_LIST_MAX + i] = slot | (band << PRELOAD_SLOT_BITS);
        atomicAdd(&cache_stats.pre_hist[out][band], 1u);
    } else {
        atomicAdd(&cache_stats.pre_drops, 1u);
        atomicAnd(&cache_cells[CACHE_WORDS * slot + CACHE_STAMP], ~CACHE_PRELOADING);
    }
}

// An input entry after this frame's samples, `count` its cell's count: at C_PRE it leaves the
// list, else it carries to the output list in its band from this frame's eye.
fn preload_carry(slot: u32, v: vec3<i32>, count: u32) {
    if (count >= preload.until) {
        atomicAnd(&cache_cells[CACHE_WORDS * slot + CACHE_STAMP], ~CACHE_PRELOADING);
    } else {
        preload_append(slot, v);
    }
}

// Whether the input entry `slot` still stands in the list: its cell holds a key and its
// CACHE_PRELOADING bit. A cell freed and claimed again by another face may be listed twice
// (the stale entry and the sweep's new one); both fill it, and the first at C_PRE clears the
// bit the other then lacks.
fn preload_standing(slot: u32) -> bool {
    let word = CACHE_WORDS * slot;
    return (atomicLoad(&cache_cells[word + CACHE_STAMP]) & CACHE_PRELOADING) != 0u
        && (atomicLoad(&cache_cells[word + 1u]) & 0x80000000u) != 0u;
}

// Whether the input entry of `slot` in `band` draws this frame: the bands nearer than the
// args pass's cut, and of the cut's band `pre_rest` of its entries by a hash of the slot and
// the frame. So the frame's draws are about the nearest `draw_cells` entries, a function of
// the list's set and never of the order its entries were appended in.
fn preload_draws(slot: u32, band: u32) -> bool {
    let cut = cache_stats.pre_cut;
    if (band != cut) {
        return band < cut;
    }
    let n = max(atomicLoad(&cache_stats.pre_hist[cache_stats.pre_in][cut]), 1u);
    return hash_u32(slot ^ hash_u32(params.frame ^ 0x27D4EB2Fu)) % n < cache_stats.pre_rest;
}

@compute @workgroup_size(16, 1, 1)
fn preload_fill(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    let list = cache_stats.pre_in * PRELOAD_LIST_MAX;
    if (wg.x >= cache_stats.pre_n_in) {
        return;
    }
    let entry = cache_stats.pre_lists[list + wg.x];
    let slot = entry & ((1u << PRELOAD_SLOT_BITS) - 1u);
    if (!preload_standing(slot)) {
        return;
    }
    let word = CACHE_WORDS * slot;
    let k = vec2<u32>(atomicLoad(&cache_cells[word]), atomicLoad(&cache_cells[word + 1u]));
    let v = cache_key_voxel(k);
    let before = atomicLoad(&cache_cells[word + CACHE_COUNT]);
    var after = before;
    if (before < preload.until && preload_draws(slot, entry >> PRELOAD_SLOT_BITS)) {
        let h = hash_u32((slot * 0x9E3779B9u ^ (lane + 1u) * 0x85EBCA6Bu) ^ 0x5BD1E995u);
        fill_face(slot, v, (k.y >> 28u) & 7u, vec2<u32>(h & 0xFFFFu, h >> 16u));
        after = before + FILL_LANES;
    }
    if (lane == 0u) {
        if (after != before) {
            atomicAdd(&cache_stats.pre_samples, FILL_LANES);
        }
        preload_carry(slot, v, after);
    }
}

// What the sweep finds at brick `b` (in bricks): 0 nothing opaque (uniform air or translucent),
// 1 one opaque material throughout, 2 a Full brick, 3 absent (rock to the walk), 4 LOD-only.
fn preload_brick(b: vec3<i32>) -> u32 {
    let d = descend(b * 8);
    if (d.size != 0) {
        return select(0u, 3u, !d.resident);
    }
    if (d.level != 0u) {
        return 4u;
    }
    if ((d.entry & FULL_BIT) != 0u) {
        return 2u;
    }
    return select(1u, 0u, (d.entry & TRANSLUCENT_BIT) != 0u);
}

// Whether the sweep visits brick `b`'s voxels: a Full brick, or a uniform solid one with a
// neighbour brick that is neither uniform solid nor absent.
fn preload_brick_open(b: vec3<i32>) -> bool {
    let c = preload_brick(b);
    if (c == 2u) {
        return true;
    }
    if (c != 1u) {
        return false;
    }
    for (var k = 0u; k < 6u; k++) {
        let n = preload_brick(b + vec3<i32>(face_dir(k)));
        if (n != 1u && n != 3u) {
            return true;
        }
    }
    return false;
}

// Whether voxel `v` is air to the light pass's cacheable rule: resident, empty to the opaque
// walk and not translucent.
fn preload_air(v: vec3<i32>) -> bool {
    let c = lookup(v);
    return c.size != 0 && c.air && tlookup(v).size != 0;
}

// Adds `n` samples whose quantized sums are `q` to `slot` while its count is under C_CAP
// (`cache_add`'s arithmetic, `n` at a time).
fn cache_add_sums(slot: u32, q: vec3<u32>, n: u32) {
    let word = CACHE_WORDS * slot + CACHE_COUNT;
    let before = atomicAdd(&cache_cells[word], n);
    if (before < cache_c_cap()) {
        atomicAdd(&cache_cells[word + 1u], q.x);
        atomicAdd(&cache_cells[word + 2u], q.y);
        atomicAdd(&cache_cells[word + 3u], q.z);
    }
}

// The sweep's step for face `face` of the opaque voxel `v`, whose neighbour across it is air.
fn preload_face(v: vec3<i32>, face: u32) {
    let found = cache_find(v, face);
    if (found >= 0) {
        let slot = u32(found);
        let word = CACHE_WORDS * slot;
        if (atomicLoad(&cache_cells[word + CACHE_COUNT]) >= preload.until) {
            return;
        }
        let old = atomicOr(&cache_cells[word + CACHE_STAMP], CACHE_PRELOADING);
        if ((old & CACHE_PRELOADING) == 0u) {
            atomicAdd(&cache_stats.pre_listed, 1u);
            preload_append(slot, v);
        }
        return;
    }
    // No cell: the probe samples, summed as `cache_add` would add them one by one.
    let key = cache_key(v, face);
    let n = min(preload.probe, PRELOAD_PROBE_MAX);
    var q = vec3<u32>(0u);
    var best = FaceSample(vec3<f32>(0.0), Reservoir());
    for (var j = 0u; j < n; j++) {
        let h = hash_u32(key.x ^ hash_u32(key.y ^ ((j + 1u) * 0x9E3779B9u)));
        let s = face_sample(v, face, vec2<u32>(h & 0xFFFFu, h >> 16u));
        q += vec3<u32>(s.e * CACHE_Q + 0.5);
        if (luminance(s.e) > luminance(best.e)) {
            best = s;
        }
    }
    atomicAdd(&cache_stats.pre_samples, n);
    if (luminance(best.e) <= 0.0) {
        atomicAdd(&cache_stats.pre_refused_unlit, 1u);
        return;
    }
    if (atomicLoad(&cache_stats.occupied) >= preload.load_max) {
        atomicAdd(&cache_stats.pre_refused_load, 1u);
        return;
    }
    if (atomicLoad(&cache_stats.misses) > 0u) {
        atomicAdd(&cache_stats.pre_refused_miss, 1u);
        return;
    }
    let c = cache_claim(v, face);
    if (c < 0) {
        atomicAdd(&cache_stats.pre_refused_miss, 1u);
        return;
    }
    let slot = u32(c);
    cache_add_sums(slot, q, n);
    cache_offer(slot, v, face, best.r);
    atomicAdd(&cache_stats.pre_claimed, 1u);
    if (n < preload.until) {
        let old = atomicOr(&cache_cells[CACHE_WORDS * slot + CACHE_STAMP], CACHE_PRELOADING);
        if ((old & CACHE_PRELOADING) == 0u) {
            atomicAdd(&cache_stats.pre_listed, 1u);
            preload_append(slot, v);
        }
    }
}

@compute @workgroup_size(64, 1, 1)
fn preload_sweep(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    reset_walk();
    var b: vec3<i32>;
    if (wg.x < preload.edit_bricks) {
        b = preload.brick_list[wg.x].xyz;
    } else {
        let j = (cache_stats.pre_base + wg.x - preload.edit_bricks) % max(preload.offsets, 1u);
        let o = cache_stats.pre_lists[2u * PRELOAD_LIST_MAX + j];
        b = preload.eye_brick + vec3<i32>(vec3<u32>(o, o >> 10u, o >> 20u) & vec3<u32>(1023u)) - vec3<i32>(512);
    }
    if (!preload_brick_open(b)) {
        return;
    }
    if (lane == 0u) {
        atomicAdd(&cache_stats.pre_scanned, 1u);
    }
    let row = b * 8 + vec3<i32>(0, i32(lane % 8u), i32(lane / 8u));
    for (var x = 0; x < 8; x++) {
        let v = row + vec3<i32>(x, 0, 0);
        if (lookup(v).size != 0) {
            continue;
        }
        for (var face = 0u; face < 6u; face++) {
            if (preload_air(v + vec3<i32>(face_dir(face)))) {
                preload_face(v, face);
            }
        }
    }
}
