// The reopened fill (ADR-0043, the dug hole): F emissive samples a frame for each cell an edit
// box or the guard reopened, from random points on its face, so a reopened face converges at F
// samples a frame however few pixels it covers. A face seen for the first time with no edit is
// never listed. And the camera-radius preload: a sweep of the mirror's bricks around the eye
// that lists every lit face it finds, and a fill that brings each listed cell to C_PRE samples
// whatever the view. Compiled after march.wgsl and light.wgsl, whose bindings (group 0, and
// group 2's `cache_cells` and `cache_stats`, which holds both lists, at bindings 3 and 4),
// candidates, shadow ray and cell helpers (`fill_face`, `face_sample`, `cache_take`) it uses.
// Five entry points: the reopened fill's two after the light pass (which lists the reopened
// cells, `fill_offer`) and before the temporal pass, and the preload's three, which take the
// light pass's place (`shade_preload`) and the two dispatches before it:
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
//   preload_args   one workgroup of a lane per distance band, before the light pass: applies
//                  the last frame's left and claimed cells' bits, swaps the preload's lists
//                  (the last frame's outputs are this frame's inputs), and writes the sweep's
//                  and the light pass's indirect dispatches into `preload_dispatch` (group 3,
//                  binding 2)
//   preload_sweep  after the args pass, eight workgroups a brick, a lane a voxel: the bricks
//                  of the frame's edit boxes within the radius first, then `pre_visit` offsets
//                  of the sweep's table from its cursor around the eye's brick. A slab stamped
//                  done at this epoch ends at once; each opaque voxel's face toward air (the
//                  light pass's cacheable faces) is looked up (`cache_find`): a cell the sweep
//                  wants (`preload_wants`) not yet listed is listed into the output list; a face
//                  with no cell, on the slab's first visit this epoch, goes to the output probe
//                  list
//   shade_preload  the light pass's rows (`shade_px`), then in the same dispatch:
//                  - the fill (`preload_fill_entry`), four input entries a workgroup: about
//                    the nearest `draw_cells` entries draw, by the distance bands their entries
//                    carry (`preload_draws`), so which cells draw is a function of the list's
//                    set, never of the order its entries were appended in; a drawing entry's
//                    FILL_LANES lanes each draw one sample into the cell (`fill_face`);
//                  - the probes (`preload_probe_group`), eight input faces a workgroup, a lane
//                    a sample: the faces with no cell the sweep found, which claim a cell when
//                    the last frame's table held under `load_max` cells and its light pass
//                    counted no miss, and, beyond the claim radius, only if a sample is lit.
//
// The claim radius (`preload.claim`, r0 voxels from the eye to the face's voxel centre): a face
// within it claims a cell whatever its probes read and is filled to C_PRE lit or unlit, so a
// face no emitter reaches is filled once with zeros and then skipped, not probed again each
// epoch; from r0 to the radius a face claims a cell only if one of its probes is lit.

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
// the bricks it visits a frame (B; none while idle, `preload_args`), whether the camera is
// still (`still`: it moved under a voxel and turned under a degree, and the guard's window is
// closed), C_PRE (`until`), the cells the fill draws a frame (S_PRE / FILL_LANES), the occupied cells
// past which a probe claims none (PRE_LOAD_MAX of the table), the samples a face with no cell
// draws (`probe`, at most PRELOAD_PROBE_LANES), the world's epoch (`epoch`, the low
// PRELOAD_EPOCH_BITS of a count the CPU raises on every frame the world or the table changes
// as a whole: an edit, an upload, a table clear, the table's chunk sweep, a C_PRE change), and
// the light pass's grid of workgroups (`light_cols` by `light_rows`), the eye in voxels and the
// claim radius r0 in voxels (`claim`, 0 none).
struct Preload {
    eye_brick: vec3<i32>,
    edit_bricks: u32,
    offsets: u32,
    bricks: u32,
    still: u32,
    _pad0: u32,
    until: u32,
    draw_cells: u32,
    load_max: u32,
    probe: u32,
    epoch: u32,
    light_cols: u32,
    light_rows: u32,
    _pad1: u32,
    eye: vec3<f32>,
    claim: f32,
    brick_list: array<vec4<i32>, 256>,
};

// Lanes a face's probe takes, one sample each: the most samples a face with no cell draws.
const PRELOAD_PROBE_LANES: u32 = 8u;

@group(3) @binding(1) var<uniform> preload: Preload;
// Two indirect dispatches: the sweep's (words 0..3), then the light pass's with the fill and
// the probes beside it (words 3..6).
@group(3) @binding(2) var<storage, read_write> preload_dispatch: array<u32, 6>;

// The preload's store in `pre_lists`: the two fill lists (PRELOAD_LIST_MAX entries each, an
// entry two words: the cell's slot and band, then the draws it has left), the sweep's offsets
// (PRELOAD_OFFSETS_MAX, `marcher::preload_offsets` at the largest radius), the two probe lists
// (PRELOAD_PROBE_LIST_MAX faces each, a face its key's two words), the list of cells that left
// the fill and the list of cells the probes claimed (PRELOAD_LIST_MAX slots each), then the
// slab stamps.
const PRELOAD_OFFSETS_MAX: u32 = 17077u;
const PRELOAD_PROBE_LIST_MAX: u32 = 16384u;
const PRELOAD_OFFSETS_AT: u32 = 4u * PRELOAD_LIST_MAX;
const PRELOAD_PROBES_AT: u32 = PRELOAD_OFFSETS_AT + PRELOAD_OFFSETS_MAX;
const PRELOAD_LEFT_AT: u32 = PRELOAD_PROBES_AT + 4u * PRELOAD_PROBE_LIST_MAX;
const PRELOAD_CLAIMED_AT: u32 = PRELOAD_LEFT_AT + PRELOAD_LIST_MAX;
const PRELOAD_STAMPS_AT: u32 = PRELOAD_CLAIMED_AT + PRELOAD_LIST_MAX;

// Who writes what, so a frame is a function of its inputs although the fill and the probes run
// in the light pass's dispatch, beside pixels that add to the same cells: a cell's
// CACHE_PRELOADING bit (it stands in a fill list) and every decision that reads a cell's count
// belong to the args pass and the sweep, separate dispatches before the light pass. The fill and
// the probes, in the light pass's dispatch, only add samples, which commute, and append to
// lists: an entry draws while it has draws left (set by the sweep from the count it read, or
// fixed for a probe's new cell), a cell whose entry ran out goes to the left list and a cell
// a probe claimed to the claimed list, whose bits the next frame's args pass clears and sets.

// The slab stamps: one word per slab (a z layer of 8 x 8 voxels of a brick) of the bricks
// within the largest radius, at the brick's coordinates modulo PRELOAD_STAMP_EDGE on each axis
// (33 bricks: no two bricks within 16 of the eye's share a stamp), eight a brick. A stamp is
// state per slab of the world, not a table cell: the epoch it was written at
// (PRELOAD_EPOCH_BITS), the brick's coordinate divided by the edge, modulo 16 on each axis (so a
// stamp left by a brick 33 away reads as another's), and its state: PRELOAD_PROBED, every face
// of the slab without a cell was sent to the probes at that epoch (which claim a cell for a lit
// one); PRELOAD_DONE, besides, no face of it holds a cell the sweep lists, or the brick has no
// face the sweep reads. A stamp of another epoch or brick is none. So the sweep probes a face
// no emitter reaches once an epoch, not on every pass, and at rest it reads one word a slab.
const PRELOAD_STAMP_EDGE: i32 = 33;
const PRELOAD_EPOCH_BITS: u32 = 18u;
const PRELOAD_PROBED: u32 = 1u;
const PRELOAD_DONE: u32 = 2u;
// Done whatever the eye: the slab has no face the sweep reads, or every face of it lay within
// the claim radius when it was read and holds a cell the sweep does not list. A slab stamped
// PRELOAD_DONE that comes within the claim radius is read again (`PRELOAD_SWEEP_NEAR`), since
// its faces beyond r0 were left without a cell when unlit.
const PRELOAD_ALL: u32 = 3u;

fn preload_stamp_index(b: vec3<i32>, slab: u32) -> u32 {
    let m = ((b % PRELOAD_STAMP_EDGE) + PRELOAD_STAMP_EDGE) % PRELOAD_STAMP_EDGE;
    let e = u32(PRELOAD_STAMP_EDGE);
    return PRELOAD_STAMPS_AT + 8u * (u32(m.x) + e * (u32(m.y) + e * u32(m.z))) + slab;
}

// A stamp's word without its state: the epoch and the brick's tag.
fn preload_stamp_tag(b: vec3<i32>) -> u32 {
    let q = bitcast<vec3<u32>>(floor_div(b, PRELOAD_STAMP_EDGE)) & vec3<u32>(15u);
    let epoch = preload.epoch & ((1u << PRELOAD_EPOCH_BITS) - 1u);
    return epoch | ((q.x | (q.y << 4u) | (q.z << 8u)) << PRELOAD_EPOCH_BITS);
}

// The slab's state at this epoch: 0 none, PRELOAD_PROBED or PRELOAD_DONE.
fn preload_stamp_state(b: vec3<i32>, slab: u32) -> u32 {
    let w = cache_stats.pre_lists[preload_stamp_index(b, slab)];
    return select(0u, w >> 30u, (w & 0x3FFFFFFFu) == preload_stamp_tag(b));
}

fn preload_stamp(b: vec3<i32>, slab: u32, state: u32) {
    cache_stats.pre_lists[preload_stamp_index(b, slab)] = preload_stamp_tag(b) | (state << 30u);
}

// The args pass's band counts and their running sums, a lane a band, and whether the frame
// draws fewer entries than the input list holds (else every entry draws and the bands are
// not summed).
var<workgroup> args_hist: array<u32, 64>;
var<workgroup> args_cut: u32;

// One workgroup of PRELOAD_BANDS lanes, first of the preload's passes: clears the bits of the
// cells that left the fill last frame and sets those of the cells the probes claimed, swaps
// the preload's lists (the last frame's outputs are this frame's inputs), picks the fill's
// cells, moves the sweep's cursor and writes the two indirect dispatches.
@compute @workgroup_size(64, 1, 1)
fn preload_args(@builtin(local_invocation_index) lane: u32) {
    let p = cache_stats.pre_parity & 1u;
    let left = min(atomicLoad(&cache_stats.pre_left_n), PRELOAD_LIST_MAX);
    for (var i = lane; i < left; i += PRELOAD_BANDS) {
        let slot = cache_stats.pre_lists[PRELOAD_LEFT_AT + i];
        atomicAnd(&cache_cells[CACHE_WORDS * slot + CACHE_STAMP], ~CACHE_PRELOADING);
    }
    let claimed = min(atomicLoad(&cache_stats.pre_claims_n), PRELOAD_LIST_MAX);
    for (var i = lane; i < claimed; i += PRELOAD_BANDS) {
        let slot = cache_stats.pre_lists[PRELOAD_CLAIMED_AT + i];
        atomicOr(&cache_cells[CACHE_WORDS * slot + CACHE_STAMP], CACHE_PRELOADING);
    }
    let n_in = min(atomicLoad(&cache_stats.pre_count[p]), PRELOAD_LIST_MAX);
    let n_probe = min(atomicLoad(&cache_stats.pre_probe_count[p]), PRELOAD_PROBE_LIST_MAX);
    let draw = min(n_in, preload.draw_cells);
    // The nearest `draw` entries draw: every entry of the distance bands under `pre_cut`, and
    // `pre_rest` of band `pre_cut`'s by hash (`preload_draws`). An inclusive scan of the bands,
    // when the list holds more than draw.
    if (lane == 0u) {
        args_cut = u32(n_in > draw);
    }
    let cuts = workgroupUniformLoad(&args_cut) != 0u;
    let c = atomicLoad(&cache_stats.pre_hist[p][lane]);
    atomicStore(&cache_stats.pre_hist[p ^ 1u][lane], 0u);
    if (cuts) {
        args_hist[lane] = c;
        for (var step = 1u; step < PRELOAD_BANDS; step *= 2u) {
            workgroupBarrier();
            let add = select(0u, args_hist[max(lane, step) - step], lane >= step);
            workgroupBarrier();
            args_hist[lane] += add;
        }
        workgroupBarrier();
        // The cut: the first band whose running sum passes `draw`.
        let before = select(0u, args_hist[max(lane, 1u) - 1u], lane > 0u);
        if (args_hist[lane] > draw && before <= draw) {
            cache_stats.pre_cut = lane;
            cache_stats.pre_rest = draw - before;
        }
    }
    if (lane != 0u) {
        return;
    }
    if (!cuts) {
        cache_stats.pre_cut = PRELOAD_BANDS;
        cache_stats.pre_rest = 0u;
    }
    // The last frame's light-pass misses and occupancy, for the probes' claim rule, read
    // before this frame's light pass runs beside them; the misses then count this frame's.
    cache_stats.pre_last_misses = atomicExchange(&cache_stats.misses, 0u);
    cache_stats.pre_last_occupied = atomicLoad(&cache_stats.occupied);
    atomicStore(&cache_stats.pre_left_n, 0u);
    atomicStore(&cache_stats.pre_claims_n, 0u);
    cache_stats.pre_in = p;
    cache_stats.pre_n_in = n_in;
    cache_stats.pre_probe_n = n_probe;
    cache_stats.pre_draw = draw;
    cache_stats.pre_parity = p ^ 1u;
    atomicStore(&cache_stats.pre_count[p ^ 1u], 0u);
    atomicStore(&cache_stats.pre_probe_count[p ^ 1u], 0u);
    // Idle: still with both lists empty, after a whole cycle of the sweep at this epoch found
    // every slab done; then the sweep visits nothing until the camera moves, a list fills or
    // the epoch changes (it visits B bricks a frame otherwise). A cycle starts at a new epoch and after each whole pass over the
    // offsets, and counts the slabs the last frames' sweeps found other than done.
    let quiet = preload.still != 0u && n_in == 0u && n_probe == 0u;
    let epoch = preload.epoch + 1u;
    if (cache_stats.pre_cycle_epoch != epoch) {
        cache_stats.pre_cycle_epoch = epoch;
        cache_stats.pre_cycle_left = preload.offsets;
        atomicStore(&cache_stats.pre_busy, 0u);
    } else {
        cache_stats.pre_cycle_left -= min(cache_stats.pre_visit, cache_stats.pre_cycle_left);
        if (cache_stats.pre_cycle_left == 0u) {
            if (atomicLoad(&cache_stats.pre_busy) == 0u) {
                cache_stats.pre_idle_epoch = epoch;
            }
            cache_stats.pre_cycle_left = preload.offsets;
            atomicStore(&cache_stats.pre_busy, 0u);
        }
    }
    if (!quiet) {
        cache_stats.pre_idle_epoch = 0u;
    }
    let visit = select(min(preload.bricks, preload.offsets), 0u, quiet && cache_stats.pre_idle_epoch == epoch);
    let cursor = select(0u, cache_stats.pre_cursor, cache_stats.pre_cursor < preload.offsets);
    cache_stats.pre_base = cursor;
    cache_stats.pre_visit = visit;
    cache_stats.pre_cursor = select(0u, (cursor + visit) % max(preload.offsets, 1u), preload.offsets > 0u);
    let fill_groups = (n_in + 3u) / 4u;
    let probe_groups = (n_probe + 7u) / 8u;
    cache_stats.pre_fill_groups = fill_groups;
    cache_stats.pre_probe_groups = probe_groups;
    let groups = fill_groups + probe_groups;
    cache_stats.pre_groups = groups;
    let cols = max(preload.light_cols, 1u);
    preload_dispatch = array<u32, 6>(
        8u * (preload.edit_bricks + visit),
        1u,
        1u,
        cols,
        preload.light_rows + (groups + cols - 1u) / cols,
        1u,
    );
}

// Distance bands of PRELOAD_BAND_VOXELS from the eye's brick's centre, PRELOAD_BANDS of them
// (the last open): an entry's band orders the fill nearest first.
const PRELOAD_BANDS: u32 = 64u;
const PRELOAD_BAND_VOXELS: f32 = 2.0;
// An entry's first word: the cell's slot in its low PRELOAD_SLOT_BITS bits, its band above.
const PRELOAD_SLOT_BITS: u32 = 26u;

fn preload_band(v: vec3<i32>) -> u32 {
    let eye = vec3<f32>(preload.eye_brick) * 8.0 + vec3<f32>(4.0);
    let d = length(vec3<f32>(v) + vec3<f32>(0.5) - eye);
    return min(u32(d / PRELOAD_BAND_VOXELS), PRELOAD_BANDS - 1u);
}

// Whether voxel `v`'s faces lie within the claim radius: the eye's distance to its centre at
// most r0 (the same test on every frame for the same eye).
fn preload_near(v: vec3<i32>) -> bool {
    let d = vec3<f32>(v) + vec3<f32>(0.5) - preload.eye;
    return preload.claim > 0.0 && dot(d, d) <= preload.claim * preload.claim;
}

// Whether the last frame's table and light pass let a face claim a cell: 0 yes, else the cause
// (1 the table's load, 2 a miss).
fn preload_guard() -> u32 {
    if (cache_stats.pre_last_occupied >= preload.load_max) {
        return 1u;
    }
    return select(0u, 2u, cache_stats.pre_last_misses > 0u);
}

// The draws a cell holding `count` samples takes to reach C_PRE, FILL_LANES at a time.
fn preload_draws_for(count: u32) -> u32 {
    return (preload.until - min(count, preload.until) + FILL_LANES - 1u) / FILL_LANES;
}

// Appends the cell `slot` of voxel `v`'s face, with `draws` draws left, to this frame's output
// list in `v`'s band; false when the list is full (the entry is dropped and counted).
fn preload_append(slot: u32, v: vec3<i32>, draws: u32) -> bool {
    let out = cache_stats.pre_in ^ 1u;
    let i = atomicAdd(&cache_stats.pre_count[out], 1u);
    if (i >= PRELOAD_LIST_MAX) {
        atomicAdd(&cache_stats.pre_drops, 1u);
        return false;
    }
    let band = preload_band(v);
    let at = 2u * (out * PRELOAD_LIST_MAX + i);
    cache_stats.pre_lists[at] = slot | (band << PRELOAD_SLOT_BITS);
    cache_stats.pre_lists[at + 1u] = draws;
    atomicAdd(&cache_stats.pre_hist[out][band], 1u);
    return true;
}

// Appends `slot` to the left list (`claimed` false) or the claimed list, PRELOAD_LIST_MAX
// slots each.
fn preload_note(claimed: bool, slot: u32) {
    var i = 0u;
    if (claimed) {
        i = atomicAdd(&cache_stats.pre_claims_n, 1u);
    } else {
        i = atomicAdd(&cache_stats.pre_left_n, 1u);
    }
    if (i < PRELOAD_LIST_MAX) {
        cache_stats.pre_lists[select(PRELOAD_LEFT_AT, PRELOAD_CLAIMED_AT, claimed) + i] = slot;
    }
}

// The sweep's listing of the cell `slot` of voxel `v`'s face (no other pass runs beside the
// sweep): unless it stands in a list already, its bit is set and it is listed with the draws
// its count leaves to C_PRE; a full list leaves its bit clear, so a later pass lists it.
fn preload_list(slot: u32, v: vec3<i32>) {
    let word = CACHE_WORDS * slot;
    let old = atomicOr(&cache_cells[word + CACHE_STAMP], CACHE_PRELOADING);
    if ((old & CACHE_PRELOADING) == 0u) {
        atomicAdd(&cache_stats.pre_listed, 1u);
        if (!preload_append(slot, v, preload_draws_for(atomicLoad(&cache_cells[word + CACHE_COUNT])))) {
            atomicAnd(&cache_cells[word + CACHE_STAMP], ~CACHE_PRELOADING);
        }
    }
}

// Whether the input entry `slot` still stands in the list: its cell holds a key and its
// CACHE_PRELOADING bit (which only the args pass and the sweep change). A cell freed and
// claimed again by another face may be listed twice (the stale entry and the sweep's new one);
// both fill it, and the first to run out of draws clears the bit the other then lacks.
fn preload_standing(slot: u32) -> bool {
    let word = CACHE_WORDS * slot;
    return (atomicLoad(&cache_cells[word + CACHE_STAMP]) & CACHE_PRELOADING) != 0u
        && (atomicLoad(&cache_cells[word + 1u]) & 0x80000000u) != 0u;
}

// Whether the input entry of the cell keyed `k` in `band` draws this frame: the bands nearer
// than the args pass's cut, and of the cut's band `pre_rest` of its entries by a hash of the
// key and the frame. So the frame's draws are about the nearest `draw_cells` entries, a
// function of the list's set and never of the order its entries were appended in.
fn preload_draws(k: vec2<u32>, band: u32) -> bool {
    let cut = cache_stats.pre_cut;
    if (band != cut) {
        return band < cut;
    }
    let n = max(atomicLoad(&cache_stats.pre_hist[cache_stats.pre_in][cut]), 1u);
    return hash_u32(k.x ^ hash_u32(k.y ^ params.frame ^ 0x27D4EB2Fu)) % n < cache_stats.pre_rest;
}

// The fill: input entry `e`'s FILL_LANES lanes (`sub` the lane among them) each draw one
// sample into its cell when the entry draws this frame. An entry whose cell lost its
// CACHE_PRELOADING bit (an edit box, the table's chunk sweep or a whole-table clear emptied it)
// is dropped; one whose draws ran out goes to the left list; any other carries to the output
// list.
fn preload_fill_entry(e: u32, sub: u32) {
    if (e >= cache_stats.pre_n_in) {
        return;
    }
    let at = 2u * (cache_stats.pre_in * PRELOAD_LIST_MAX + e);
    let entry = cache_stats.pre_lists[at];
    var draws = cache_stats.pre_lists[at + 1u];
    let slot = entry & ((1u << PRELOAD_SLOT_BITS) - 1u);
    if (!preload_standing(slot)) {
        return;
    }
    let word = CACHE_WORDS * slot;
    let k = vec2<u32>(atomicLoad(&cache_cells[word]), atomicLoad(&cache_cells[word + 1u]));
    let v = cache_key_voxel(k);
    if (draws > 0u && preload_draws(k, entry >> PRELOAD_SLOT_BITS)) {
        let h = hash_u32((hash_u32(k.x ^ hash_u32(k.y)) ^ (sub + 1u) * 0x85EBCA6Bu) ^ 0x5BD1E995u);
        fill_face(slot, v, (k.y >> 28u) & 7u, vec2<u32>(h & 0xFFFFu, h >> 16u));
        draws -= 1u;
        if (sub == 0u) {
            atomicAdd(&cache_stats.pre_samples, FILL_LANES);
        }
    }
    if (sub == 0u) {
        if (draws == 0u || !preload_append(slot, v, draws)) {
            preload_note(false, slot);
        }
    }
}

// The probes' per-face sums in a workgroup: the quantized irradiance, whether a sample was lit,
// and the cell claimed (-1 none).
var<workgroup> probe_q: array<array<atomic<u32>, 3>, 8>;
var<workgroup> probe_lit: array<atomic<u32>, 8>;
var<workgroup> probe_slot: array<i32, 8>;

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

// The probes: workgroup `g`'s eight input faces (the faces with no cell the sweep found),
// PRELOAD_PROBE_LANES lanes each, a lane one sample (`face_sample`). A face beyond the claim
// radius whose samples are all dark takes no cell; any other takes a cell (its own if the light
// pass has claimed it since, else a claimed one) when the last frame's table held under
// `load_max` cells and its light pass counted no miss, takes the samples (summed as `cache_add`
// would add them one by one),
// is offered each lit sample's emitter (the cell keeps the brightest) and is listed for the
// fill with the draws that bring `probe` samples to C_PRE; its bit is set by the next args pass.
fn preload_probe_group(g: u32, lane: u32) {
    let f = lane / PRELOAD_PROBE_LANES;
    let j = lane % PRELOAD_PROBE_LANES;
    let i = g * 8u + f;
    let n = min(preload.probe, PRELOAD_PROBE_LANES);
    if (j == 0u) {
        atomicStore(&probe_q[f][0], 0u);
        atomicStore(&probe_q[f][1], 0u);
        atomicStore(&probe_q[f][2], 0u);
        atomicStore(&probe_lit[f], 0u);
        probe_slot[f] = -1;
    }
    workgroupBarrier();
    let has = i < cache_stats.pre_probe_n;
    var v = vec3<i32>(0);
    var face = 0u;
    var key = vec2<u32>(0u);
    var s = FaceSample(vec3<f32>(0.0), Reservoir());
    if (has) {
        let at = PRELOAD_PROBES_AT + 2u * (cache_stats.pre_in * PRELOAD_PROBE_LIST_MAX + i);
        key = vec2<u32>(cache_stats.pre_lists[at], cache_stats.pre_lists[at + 1u]);
        v = cache_key_voxel(key);
        face = (key.y >> 28u) & 7u;
        if (j < n) {
            let h = hash_u32(key.x ^ hash_u32(key.y ^ ((j + 1u) * 0x9E3779B9u)));
            s = face_sample(v, face, vec2<u32>(h & 0xFFFFu, h >> 16u));
            let q = vec3<u32>(s.e * CACHE_Q + 0.5);
            atomicAdd(&probe_q[f][0], q.x);
            atomicAdd(&probe_q[f][1], q.y);
            atomicAdd(&probe_q[f][2], q.z);
            if (luminance(s.e) > 0.0) {
                atomicStore(&probe_lit[f], 1u);
            }
        }
    }
    workgroupBarrier();
    if (has && j == 0u) {
        atomicAdd(&cache_stats.pre_probed, 1u);
        atomicAdd(&cache_stats.pre_samples, n);
        let lit = atomicLoad(&probe_lit[f]) != 0u;
        let near = preload_near(v);
        let guard = preload_guard();
        if (!lit && !near) {
            atomicAdd(&cache_stats.pre_refused_unlit, 1u);
        } else if (guard == 1u) {
            atomicAdd(&cache_stats.pre_refused_load, 1u);
        } else if (guard == 2u) {
            atomicAdd(&cache_stats.pre_refused_miss, 1u);
        } else {
            let c = cache_claim(v, face);
            if (c < 0) {
                atomicAdd(&cache_stats.pre_refused_miss, 1u);
            } else {
                let slot = u32(c);
                let q = vec3<u32>(
                    atomicLoad(&probe_q[f][0]),
                    atomicLoad(&probe_q[f][1]),
                    atomicLoad(&probe_q[f][2]),
                );
                cache_add_sums(slot, q, n);
                atomicAdd(&cache_stats.pre_claimed, 1u);
                if (near) {
                    atomicAdd(&cache_stats.pre_claimed_near[u32(!lit)], 1u);
                }
                if (preload_append(slot, v, preload_draws_for(n))) {
                    preload_note(true, slot);
                }
                probe_slot[f] = c;
            }
        }
    }
    workgroupBarrier();
    let c = probe_slot[f];
    if (has && j < n && c >= 0 && luminance(s.e) > 0.0) {
        cache_offer(u32(c), v, face, s.r);
    }
}

// Appends the face `face` of voxel `v`, which has no cell, to this frame's output probe list;
// false when the list is full (the face is dropped and counted).
fn preload_probe_append(v: vec3<i32>, face: u32) -> bool {
    let out = cache_stats.pre_in ^ 1u;
    let i = atomicAdd(&cache_stats.pre_probe_count[out], 1u);
    if (i >= PRELOAD_PROBE_LIST_MAX) {
        atomicAdd(&cache_stats.pre_probe_drops, 1u);
        return false;
    }
    let key = cache_key(v, face);
    let at = PRELOAD_PROBES_AT + 2u * (out * PRELOAD_PROBE_LIST_MAX + i);
    cache_stats.pre_lists[at] = key.x;
    cache_stats.pre_lists[at + 1u] = key.y;
    return true;
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

// Whether the sweep lists the cell `slot` of voxel `v`'s face: under C_PRE, and within the claim
// radius (lit or not), or lit (it holds a kept emitter, which only a lit sample offers; a test of
// the sums would depend on the emission's scale), or holding fewer samples than a probe draws.
// Beyond r0 a cell whose first `probe` samples or more were all dark (the light pass's cell of a
// face in view that no emitter reaches) is a face the probes would refuse, and is left to the
// light pass.
fn preload_wants(slot: u32, v: vec3<i32>) -> bool {
    let word = CACHE_WORDS * slot;
    let count = atomicLoad(&cache_cells[word + CACHE_COUNT]);
    if (count >= preload.until) {
        return false;
    }
    if (preload_near(v)) {
        return true;
    }
    let dark = atomicLoad(&cache_cells[word + CACHE_KEPT_LUM]) == 0u;
    return !dark || count < min(preload.probe, PRELOAD_PROBE_LANES);
}

// The sweep's per-slab state in a workgroup: its mode (0 skip, PRELOAD_SWEEP_PROBE,
// PRELOAD_SWEEP_CHECK, PRELOAD_SWEEP_NEAR), whether every voxel of the slab lies within the
// claim radius, the claim guard's cause (`preload_guard`) and what its lanes found
// (PRELOAD_FOUND_*).
var<workgroup> sweep_mode: u32;
var<workgroup> sweep_all_near: u32;
var<workgroup> sweep_guard: u32;
var<workgroup> sweep_found: atomic<u32>;
// Probe every face with no cell (a slab's first read this epoch); probe only the faces with no
// cell within the claim radius (a slab probed this epoch, or done and come within r0).
const PRELOAD_SWEEP_PROBE: u32 = 1u;
const PRELOAD_SWEEP_CHECK: u32 = 2u;
const PRELOAD_SWEEP_NEAR: u32 = 3u;
// A cell under C_PRE; a face sent to the probes; a face the full probe list dropped; a face
// within the claim radius with no cell the guard kept from the probes.
const PRELOAD_FOUND_CELL: u32 = 1u;
const PRELOAD_FOUND_PROBE: u32 = 2u;
const PRELOAD_FOUND_DROP: u32 = 4u;
const PRELOAD_FOUND_HELD: u32 = 8u;

// The eye's least and greatest distance squared to the voxel centres of slab `slab` of brick
// `b`.
fn preload_slab_reach(b: vec3<i32>, slab: u32) -> vec2<f32> {
    let lo = vec3<f32>(b * 8 + vec3<i32>(0, 0, i32(slab))) + vec3<f32>(0.5);
    let hi = lo + vec3<f32>(7.0, 7.0, 0.0);
    let e = preload.eye;
    let near = clamp(e, lo, hi) - e;
    let far = max(abs(lo - e), abs(hi - e));
    return vec2<f32>(dot(near, near), dot(far, far));
}

// The sweep: workgroup `g` takes slab `g % 8` of the frame's brick `g / 8` (the edit boxes'
// bricks first, then the table's offsets from the cursor), a lane per voxel. Lane 0 reads the
// slab's stamp: all done, or done beyond the claim radius, the slab is skipped; done within it,
// its lanes list the cells under C_PRE and probe the faces with no cell within r0; probed, the
// same; none, and a brick the early-out leaves (air, absent, LOD-only, or uniform solid inside
// six uniform solid neighbours) is stamped all done, else its lanes also send every face with
// no cell to the probes. Each opaque voxel's face toward air (the light pass's cacheable faces)
// is looked up (`cache_find`). A face within r0 with no cell goes to the probes only while the
// guard lets it claim (else it is counted refused by cause, and the slab is read again on a
// later pass). Then lane 0 stamps the slab: when it found nothing, all done if every voxel lay
// within r0, else done; probed when it was a first read and the probe list dropped none; else
// unchanged. A slab counts toward the sweep's busy cycle when it was a first read or listed or
// sent a face.
fn preload_sweep_group(g: u32, lane: u32) {
    reset_walk();
    let bi = g / 8u;
    let slab = g % 8u;
    var b: vec3<i32>;
    if (bi < preload.edit_bricks) {
        b = preload.brick_list[bi].xyz;
    } else {
        let j = (cache_stats.pre_base + bi - preload.edit_bricks) % max(preload.offsets, 1u);
        let o = cache_stats.pre_lists[PRELOAD_OFFSETS_AT + j];
        b = preload.eye_brick + vec3<i32>(vec3<u32>(o, o >> 10u, o >> 20u) & vec3<u32>(1023u)) - vec3<i32>(512);
    }
    if (lane == 0u) {
        atomicStore(&sweep_found, 0u);
        let state = preload_stamp_state(b, slab);
        let reach = preload_slab_reach(b, slab);
        let r2 = preload.claim * preload.claim;
        let near = preload.claim > 0.0 && reach.x <= r2;
        var mode = 0u;
        if (state == PRELOAD_ALL || (state == PRELOAD_DONE && !near)) {
            atomicAdd(&cache_stats.pre_done, 1u);
        } else if (state == PRELOAD_DONE) {
            mode = PRELOAD_SWEEP_NEAR;
        } else if (state == PRELOAD_PROBED) {
            mode = PRELOAD_SWEEP_CHECK;
        } else if (preload_brick_open(b)) {
            mode = PRELOAD_SWEEP_PROBE;
        } else {
            preload_stamp(b, slab, PRELOAD_ALL);
        }
        if (mode != 0u) {
            atomicAdd(&cache_stats.pre_scanned, 1u);
        }
        sweep_mode = mode;
        sweep_all_near = u32(preload.claim > 0.0 && reach.y <= r2);
        sweep_guard = preload_guard();
    }
    let mode = workgroupUniformLoad(&sweep_mode);
    if (mode == 0u) {
        return;
    }
    let guard = sweep_guard;
    let v = b * 8 + vec3<i32>(i32(lane % 8u), i32(lane / 8u), i32(slab));
    if (lookup(v).size == 0) {
        let near = preload_near(v);
        for (var face = 0u; face < 6u; face++) {
            if (!preload_air(v + vec3<i32>(face_dir(face)))) {
                continue;
            }
            let found = cache_find(v, face);
            if (found >= 0) {
                let slot = u32(found);
                if (preload_wants(slot, v)) {
                    atomicOr(&sweep_found, PRELOAD_FOUND_CELL);
                    preload_list(slot, v);
                }
            } else if (near && guard != 0u) {
                atomicOr(&sweep_found, PRELOAD_FOUND_HELD);
                if (guard == 1u) {
                    atomicAdd(&cache_stats.pre_refused_load, 1u);
                } else {
                    atomicAdd(&cache_stats.pre_refused_miss, 1u);
                }
            } else if (mode == PRELOAD_SWEEP_PROBE || near) {
                if (preload_probe_append(v, face)) {
                    atomicOr(&sweep_found, PRELOAD_FOUND_PROBE);
                } else {
                    atomicOr(&sweep_found, PRELOAD_FOUND_DROP);
                }
            }
        }
    }
    workgroupBarrier();
    if (lane == 0u) {
        let found = atomicLoad(&sweep_found);
        // Any slab with work keeps the sweep's cycle from reading idle.
        if (mode == PRELOAD_SWEEP_PROBE || (found & (PRELOAD_FOUND_CELL | PRELOAD_FOUND_PROBE | PRELOAD_FOUND_DROP)) != 0u) {
            atomicAdd(&cache_stats.pre_busy, 1u);
        }
        if ((found & PRELOAD_FOUND_DROP) != 0u) {
            return;
        }
        if (found == 0u) {
            preload_stamp(b, slab, select(PRELOAD_DONE, PRELOAD_ALL, sweep_all_near != 0u));
        } else if (mode == PRELOAD_SWEEP_PROBE) {
            preload_stamp(b, slab, PRELOAD_PROBED);
        }
    }
}

// The sweep's dispatch, after the args pass and before the light pass (so its reads of the
// cells' counts and bits see no pass beside them): eight workgroups a brick.
@compute @workgroup_size(64, 1, 1)
fn preload_sweep(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    preload_sweep_group(wg.x, lane);
}

// The args pass's split of the fill's and the probes' groups, read once by lane 0 (a storage
// read is not uniform): the fill's, and both.
var<workgroup> preload_groups: array<u32, 2>;

// The light pass, the preload's fill and its probes in one dispatch, so the preload's samples
// run beside the light pass's pixels rather than after them: `light_rows` rows of `light_cols`
// workgroups over the lighting grid (`shade_px`), then the fill's and the probes' groups,
// row-major; the last row's groups past them return.
@compute @workgroup_size(8, 8, 1)
fn shade_preload(
    @builtin(workgroup_id) wg: vec3<u32>,
    @builtin(local_invocation_index) lane: u32,
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(num_workgroups) n: vec3<u32>,
) {
    if (wg.y < preload.light_rows) {
        shade_px(gid);
        return;
    }
    let g = (wg.y - preload.light_rows) * n.x + wg.x;
    if (lane == 0u) {
        preload_groups = array<u32, 2>(cache_stats.pre_fill_groups, cache_stats.pre_groups);
    }
    let groups = workgroupUniformLoad(&preload_groups);
    if (g >= groups[1]) {
        return;
    }
    if (g < groups[0]) {
        preload_fill_entry(4u * g + lane / FILL_LANES, lane % FILL_LANES);
        return;
    }
    preload_probe_group(g - groups[0], lane);
}
