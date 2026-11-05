// The emissive table's walk (ADR-0043): the chunk sweep and the age-out, in slices of at most
// `sweep.slice` cells a frame. Compiled after march.wgsl, whose key helpers it uses; no world
// group (the table, the counters, the walk's state and its dispatch are four of the stage's
// storage buffers).
//
// - The sweep: when the camera's chunk changes (`sweep.request` moves), a walk over every
//   cell frees each whose key's voxel lies in a chunk beyond the light radius of that chunk
//   (squared chunk distance over `r2`, the light table's own bound,
//   `MirrorConfig::light_radius`).
// - The age-out: when the occupied cells cross `evict_load` upward, again once they have grown
//   by `evict_step` since the last pass while still over, and again `age` frames after a pass
//   that freed a cell and left them over, a walk over every cell frees each whose voxel's
//   centre lies beyond the preload's radius of the eye (`reach2`, squared) and whose seen word
//   (march.wgsl's CACHE_SEEN) is older than `age` frames or 0. A pass that leaves the occupied
//   cells over the mark halves `age`, down to `age_min`; once under it returns to `age_max`.
//   A still camera over the mark so runs no pass after the last one that freed nothing.
//
// A walk that starts while another runs restarts from the first cell with both tests. A freed
// cell's words are cleared and the occupied counter lowered: a hole in a probe run, which
// light.wgsl's `cache_claim` steps over (it reads every probe for its key before it takes a
// free slot). Nothing claims during the walk.
//
// `walk_args`, one invocation at the frame's start: advances the last frame's slice, ends a
// finished pass, starts the requested ones and writes this frame's slice and its dispatch.
// `walk_cells`: one invocation per cell of the slice.
//
// Bindings, group 2:
//   0  uniform  sweep       the frame's request, radii, eye and clock (`Sweep`)
//   1  storage  cache_cells the table (light.wgsl)
//   2  storage  cache_stats the table's counters (`CacheStats`)
//   3  storage  walk        the walk's state across frames (`Walk`; `marcher::WalkStats`)
//   4  storage  walk_args   the slice's indirect dispatch (three words)

struct Sweep {
    chunk: vec3<i32>,
    r2: i32,
    eye: vec3<f32>,
    reach2: f32,
    request: u32,
    cells: u32,
    evict_load: u32,
    evict_step: u32,
    age_max: u32,
    age_min: u32,
    slice: u32,
    // `params.frame + 1`, the seen words' clock.
    frame: u32,
    age_out: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

// The walk's state, kept across frames (zeroed with the frame).
struct Walk {
    // The last sweep request taken.
    request: u32,
    // Cells the walk has left, its next cell, and this frame's slice (first cell, count).
    left: u32,
    cursor: u32,
    base: u32,
    n: u32,
    // WALK_SWEEP | WALK_EVICT: the running walk's tests.
    tests: u32,
    // The age-out's age (0 before the first frame), the occupied cells at the last pass's end,
    // whether a pass ran since the cells were last under the mark, and the frame a pass that
    // freed cells re-runs at (0 none).
    age: u32,
    mark: u32,
    armed: u32,
    rearm_at: u32,
    // Passes started, cells the running pass freed, every cell the age-out freed, the cells it
    // freed this frame and the cells the sweep freed.
    passes: u32,
    evicted_pass: atomic<u32>,
    evicted: atomic<u32>,
    evicted_frame: atomic<u32>,
    swept: atomic<u32>,
    _pad: u32,
    // The sweep's chunk and squared radius, taken with its request.
    chunk: vec3<i32>,
    r2: i32,
};

const WALK_SWEEP: u32 = 1u;
const WALK_EVICT: u32 = 2u;

@group(2) @binding(0) var<uniform> sweep: Sweep;
@group(2) @binding(1) var<storage, read_write> cache_cells: array<u32>;
@group(2) @binding(2) var<storage, read_write> cache_stats: CacheStats;
@group(2) @binding(3) var<storage, read_write> walk: Walk;
@group(2) @binding(4) var<storage, read_write> walk_args: array<u32, 3>;

// The end of an age-out pass: the mark, and the age the next pass takes.
fn end_evict() {
    let occ = atomicLoad(&cache_stats.occupied);
    walk.mark = occ;
    if (occ >= sweep.evict_load) {
        walk.age = max(walk.age / 2u, sweep.age_min);
        if (atomicLoad(&walk.evicted_pass) > 0u) {
            walk.rearm_at = max(sweep.frame + walk.age, 1u);
        }
    } else {
        walk.age = sweep.age_max;
        walk.armed = 0u;
    }
}

@compute @workgroup_size(1, 1, 1)
fn walk_args() {
    if (walk.n > 0u) {
        walk.cursor += walk.n;
        walk.left -= walk.n;
        walk.n = 0u;
        if (walk.left == 0u) {
            if ((walk.tests & WALK_EVICT) != 0u) {
                end_evict();
            }
            walk.tests = 0u;
        }
    }
    atomicStore(&walk.evicted_frame, 0u);
    if (walk.age == 0u || walk.age > sweep.age_max) {
        walk.age = sweep.age_max;
    }
    if (sweep.request != walk.request) {
        walk.request = sweep.request;
        walk.chunk = sweep.chunk;
        walk.r2 = sweep.r2;
        walk.tests |= WALK_SWEEP;
        walk.left = sweep.cells;
        walk.cursor = 0u;
    }
    if (sweep.age_out != 0u && (walk.tests & WALK_EVICT) == 0u) {
        let occ = atomicLoad(&cache_stats.occupied);
        let due = walk.rearm_at != 0u && sweep.frame - walk.rearm_at < 0x80000000u;
        if (occ < sweep.evict_load) {
            walk.armed = 0u;
            walk.rearm_at = 0u;
            walk.age = sweep.age_max;
        } else if (walk.armed == 0u || occ >= walk.mark + sweep.evict_step || due) {
            walk.tests |= WALK_EVICT;
            walk.left = sweep.cells;
            walk.cursor = 0u;
            walk.armed = 1u;
            walk.rearm_at = 0u;
            walk.passes += 1u;
            atomicStore(&walk.evicted_pass, 0u);
        }
    }
    if (walk.tests != 0u) {
        walk.base = walk.cursor;
        walk.n = min(walk.left, sweep.slice);
    }
    walk_args[0] = (walk.n + 63u) / 64u;
    walk_args[1] = 1u;
    walk_args[2] = 1u;
}

@compute @workgroup_size(64, 1, 1)
fn walk_cells(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= walk.n) {
        return;
    }
    let slot = walk.base + gid.x;
    let word = CACHE_WORDS * slot;
    let k1 = cache_cells[word + 1u];
    if (k1 == 0u) {
        return;
    }
    let v = cache_key_voxel(vec2<u32>(cache_cells[word], k1));
    var free = false;
    if ((walk.tests & WALK_SWEEP) != 0u) {
        // The voxel's chunk (64 voxels an edge), floored: `>>` on i32 is arithmetic.
        let d = (v >> vec3<u32>(6u)) - walk.chunk;
        free = dot(d, d) > walk.r2;
        if (free) {
            atomicAdd(&walk.swept, 1u);
        }
    }
    if (!free && (walk.tests & WALK_EVICT) != 0u) {
        let c = vec3<f32>(v) + vec3<f32>(0.5) - sweep.eye;
        let seen = cache_cells[word + CACHE_SEEN];
        if (dot(c, c) > sweep.reach2 && (seen == 0u || sweep.frame - seen > walk.age)) {
            free = true;
            atomicAdd(&walk.evicted_pass, 1u);
            atomicAdd(&walk.evicted, 1u);
            atomicAdd(&walk.evicted_frame, 1u);
        }
    }
    if (free) {
        for (var k = 0u; k < CACHE_WORDS; k++) {
            cache_cells[word + k] = 0u;
        }
        atomicSub(&cache_stats.occupied, 1u);
    }
}
