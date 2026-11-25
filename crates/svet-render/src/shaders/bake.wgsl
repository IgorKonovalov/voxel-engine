// The bake's sampler (ADR-0046): one round of emissive samples for each entry, a listed cube face
// of some job's chunk, at unit emission (EMISSION_UNIT), with its own random sequence
// (RAND_HASHED). Compiled after the walk's modules, light.wgsl and emissive.wgsl, whose
// bindings (group 0), candidates (`draw_candidates`, `face_point`) and shadow ray (`shadowed`)
// it uses, with two of their reads
// replaced in the bake's module (`bake::bake_source`): the candidate table comes from the entry's
// job slot (`bake_table_slot`, the job's `bake::job_table`) and the LOD walk's camera chunk is the
// job's chunk (`bake_origin_chunk`). So one dispatch serves faces of many jobs. The CPU keeps the
// running sums and decides each face's stop (`bake::run`); this pass only returns each entry's
// round of sums, which the CPU may read back frames after the submit (`bake::JobLoop`).
//
// Bindings, group 3:
//   0  uniform  job    the dispatch's job: its entries' count and first, seed, radius, clamp
//   1  storage  faces  the submit's entries, one round of one face each (`FaceState::input` in
//                      `bake/job.rs`): voxel, face, the round's first sample and its samples in;
//                      the round's sums out; `slot` names the entry's job slot
//   2  storage  tables the open jobs' slots, TABLE_SLOT words each: the candidate table's
//                      header and entries, then the job's chunk (xyz)
//
// A face's samples are a function of its key, `job.seed` and the sample's index alone. One
// workgroup of BAKE_LANES lanes takes an entry: lane `l` draws the round's samples `l`,
// `l + BAKE_LANES`, ..., and the lanes' sums reduce in workgroup memory, so a result does not
// depend on the batching or on the jobs' order, and differs from one lane drawing them in turn
// only by the order of its float sums (the round's largest luminance is the earliest sample's on a
// tie, as in turn). The entries span the dispatch's x then y: entry = y * BAKE_ROW + x.
//
// Under a sky (`job.sky` bit 0, ADR-0070) a face's samples below `sky_n` each also draw one
// cosine-weighted ray about the face's normal from the sample's point, through the frame's
// shadow walk (its coarse mode and shadow styles from `job.sky`'s bits 1 and 2), capped at
// `job.radius`: a ray that finds no rock reached the sky iff it ended above its column's top
// (`sky_ray_open`; with bit 3, a reading hook, any end is sky); one pointing below the horizon
// is blocked. A ray that passed SHADOW_ABSORB materials (walk.wgsl's `absorb_on`) counts as
// open with probability its transmittance. The sky ray draws its own dimensions, so the emissive
// sums are the same with the sky on and off. `sky_open` returns the round's open rays.
//
// Under a key light (`job.key` 1, ADR-0079) a face's samples below `key_n` each also draw one
// ray from the sample's point toward a uniform point of the light's disc (`disc_direction`
// about `job.key_dir`, radius `job.key_radius`), through the same shadow walk as the sky rays,
// capped at `job.radius`, read by the sky rays' clause (`sky_ray_open`, ADR-0084): a ray that
// finds no rock reached the light iff it ended (at its cap, on entering an absent chunk or on
// leaving the window) at or above its column's top, so a ray that ends in air under a roof is
// unlit (with `job.sky` bit 3, the reading hook, any end is lit); counted open with probability its transmittance through SHADOW_ABSORB materials. A face that looks away from the light (`n . l <= 0`) is
// unlit and draws none. Its dimensions are its own, so the emissive and sky sums are the same
// with the key light on and off. `key_open` returns the round's open rays.
//
// A key-only job's entry (`face` bit 8, FACE_KEY_ONLY: `bake::FACE_KEY_ONLY`, ADR-0079) draws
// the key ray alone at each sample: no sky ray and no emissive candidate, its sums 0.
//
// Each sample is the light pass's estimate at a uniform point of the face with the face's normal, times the window `(1 - (d / L)^4)^2`, d the distance from the point to the kept
// emitter point and L `job.radius` in voxels: it falls to 0 at L, so the table's edge is no seam.

struct BakeJob {
    count: u32,
    seed: u32,
    radius: f32,
    // 1: the second moment and the kept emitter read the clamped estimate.
    clamp: u32,
    // The sky rays' bits (`bake::BAKE_SKY_ON`, `_COARSE`, `_STYLES`); 0 draws none.
    sky: u32,
    // 1: the key rays are drawn toward `key_dir`'s disc of `key_radius` radians.
    key: u32,
    _pad2: u32,
    _pad3: u32,
    key_dir_x: f32,
    key_dir_y: f32,
    key_dir_z: f32,
    key_radius: f32,
};

struct BakeFace {
    voxel_x: i32,
    voxel_y: i32,
    voxel_z: i32,
    // The face (0..5), with FACE_KEY_ONLY.
    face: u32,
    first: u32,
    samples: u32,
    // Out: the sums of the estimate unclamped, and at the light pass's FIREFLY_CLAMP.
    sum_r: f32,
    sum_g: f32,
    sum_b: f32,
    clamped_r: f32,
    clamped_g: f32,
    clamped_b: f32,
    // Out: the sum of the squared luminance, the round's largest luminance and its emitter's
    // material, and the shadow rays cast with the sum of their lengths.
    lum2: f32,
    best_lum: f32,
    best_material: u32,
    rays: u32,
    ray_len: f32,
    // In: the entry's job slot in `tables`.
    slot: u32,
    // Out: the round's sky rays that reached the sky.
    sky_open: u32,
    // In: the face's samples below this index draw a sky ray.
    sky_n: u32,
    // Out: the round's key light rays that reached the light's disc.
    key_open: u32,
    // In: the face's samples below this index draw a key light ray.
    key_n: u32,
};

// `BakeFace.face`'s flag of a key-only job's entry.
const FACE_KEY_ONLY: u32 = 256u;

// The random-number dimensions of a sample's sky ray, two, above the candidates'.
const BAKE_SKY_DIM: u32 = 1002u;

// The random-number dimensions of a sample's key light ray, three, above the sky ray's.
const BAKE_KEY_DIM: u32 = 1005u;

@group(3) @binding(0) var<uniform> job: BakeJob;
@group(3) @binding(1) var<storage, read_write> faces: array<BakeFace>;
@group(3) @binding(2) var<storage, read> tables: array<vec4<u32>>;

// A slot's words: the table's header and LIGHT_TABLE_ENTRIES entries (1024, `frame/state.rs`), then
// the job's chunk.
const TABLE_SLOT: u32 = 1026u;

// The entry's slot, set by `bake_round` before any sample.
var<private> bake_slot: u32;

fn bake_table_slot(i: u32) -> vec4<u32> {
    return tables[bake_slot * TABLE_SLOT + i];
}

fn bake_origin_chunk() -> vec3<i32> {
    return bitcast<vec3<i32>>(tables[bake_slot * TABLE_SLOT + TABLE_SLOT - 1u].xyz);
}

// Lanes an entry's samples spread over (`bake::BAKE_LANES`), and workgroups a dispatch row holds
// (`bake::BAKE_ROW`).
const BAKE_LANES: u32 = 64u;
const BAKE_ROW: u32 = 32768u;

// Each lane's sums, reduced in place to lane 0's.
var<workgroup> red_sum: array<vec3<f32>, 64>;
var<workgroup> red_clamped: array<vec3<f32>, 64>;
var<workgroup> red_lum2: array<f32, 64>;
var<workgroup> red_best: array<f32, 64>;
// The sample index of the lane's largest luminance (0xFFFFFFFF: none over 0), and its material.
var<workgroup> red_best_at: array<u32, 64>;
var<workgroup> red_material: array<u32, 64>;
var<workgroup> red_rays: array<u32, 64>;
var<workgroup> red_len: array<f32, 64>;
var<workgroup> red_sky: array<u32, 64>;
var<workgroup> red_key: array<u32, 64>;
var<workgroup> red_samples: u32;

@compute @workgroup_size(64, 1, 1)
fn bake_round(
    @builtin(workgroup_id) wid: vec3<u32>,
    @builtin(local_invocation_index) lane: u32,
) {
    let at = wid.y * BAKE_ROW + wid.x;
    if (at >= job.count) {
        return;
    }
    var f = faces[at];
    if (lane == 0u) {
        red_samples = f.samples;
    }
    // The round's length, uniform across the workgroup, bounds every lane's loop.
    let samples = workgroupUniformLoad(&red_samples);
    bake_slot = f.slot;
    reset_walk();
    let v = vec3<i32>(f.voxel_x, f.voxel_y, f.voxel_z);
    let face = f.face & 7u;
    let key_only = (f.face & FACE_KEY_ONLY) != 0u;
    let key = face_seed_key(v, face);
    let stream = hash_u32(job.seed ^ hash_u32(key.x ^ hash_u32(key.y)));
    let nf = face_dir(face);
    var sum = vec3<f32>(0.0);
    var clamped = vec3<f32>(0.0);
    var lum2 = 0.0;
    var best_lum = 0.0;
    var best_at = 0xFFFFFFFFu;
    var best_material = 0u;
    var rays = 0u;
    var ray_len = 0.0;
    var sky_open = 0u;
    var key_open = 0u;
    let key_dir = vec3<f32>(job.key_dir_x, job.key_dir_y, job.key_dir_z);
    let key_faces = job.key != 0u && dot(nf, key_dir) > 0.0;
    let sky_coarse = (job.sky & 2u) != 0u;
    let sky_styles = (job.sky & 4u) != 0u;
    let sky_end = (job.sky & 8u) != 0u;
    for (var s = lane; s < samples; s += BAKE_LANES) {
        let px = vec2<u32>(stream, f.first + s);
        let p = face_point(v, face, px);
        if (!key_only && (job.sky & 1u) != 0u && f.first + s < f.sky_n) {
            let d = cosine_direction(nf, rand(px, BAKE_SKY_DIM), rand(px, BAKE_SKY_DIM + 1u));
            absorb_on = sky_styles;
            absorb_tau = 0.0;
            let open = d.z > 0.0
                && sky_ray_open(shadow_origin(p, nf), d, job.radius, sky_coarse, sky_styles, sky_end);
            absorb_on = false;
            // A ray through SHADOW_ABSORB materials counts as open with probability its
            // transmittance, so the count's expectation is the transmitted share.
            if (open && (absorb_tau == 0.0 || rand(px, BAKE_SKY_DIM + 2u) < exp(-absorb_tau))) {
                sky_open += 1u;
            }
        }
        if (key_faces && f.first + s < f.key_n) {
            let d = disc_direction(key_dir, job.key_radius, rand(px, BAKE_KEY_DIM), rand(px, BAKE_KEY_DIM + 1u));
            absorb_on = sky_styles;
            absorb_tau = 0.0;
            let lit = sky_ray_open(shadow_origin(p, nf), d, job.radius, sky_coarse, sky_styles, sky_end);
            absorb_on = false;
            if (lit && (absorb_tau == 0.0 || rand(px, BAKE_KEY_DIM + 2u) < exp(-absorb_tau))) {
                key_open += 1u;
            }
        }
        if (key_only) {
            continue;
        }
        let r = draw_candidates(px, p, nf);
        if (!(r.w_sum > 0.0 && r.target_pdf > 0.0 && r.m > 0.0)) {
            continue;
        }
        let q = distance(p, r.x) / job.radius;
        if (q >= 1.0) {
            continue;
        }
        let o = shadow_origin(p, nf);
        let to = r.x - o;
        let dist = length(to);
        rays += 1u;
        ray_len += dist;
        if (shadowed(o, to / dist, dist - 1e-3)) {
            continue;
        }
        let q2 = q * q;
        let fall = 1.0 - q2 * q2;
        let window = fall * fall;
        let lr = luminance(r.le);
        let l = r.w_sum / r.m * window;
        let lc = min(r.w_sum / r.m, FIREFLY_CLAMP * PI * lr) * window;
        sum += r.le / lr * l;
        clamped += r.le / lr * lc;
        let kept = select(l, lc, job.clamp != 0u);
        lum2 += kept * kept;
        if (kept > best_lum) {
            best_lum = kept;
            best_at = s;
            best_material = lookup(vec3<i32>(floor(r.x - 0.5 * face_dir(r.face)))).material;
        }
    }
    red_sum[lane] = sum;
    red_clamped[lane] = clamped;
    red_lum2[lane] = lum2;
    red_best[lane] = best_lum;
    red_best_at[lane] = best_at;
    red_material[lane] = best_material;
    red_rays[lane] = rays;
    red_len[lane] = ray_len;
    red_sky[lane] = sky_open;
    red_key[lane] = key_open;
    for (var stride = BAKE_LANES / 2u; stride > 0u; stride >>= 1u) {
        workgroupBarrier();
        if (lane < stride) {
            let o = lane + stride;
            red_sum[lane] += red_sum[o];
            red_clamped[lane] += red_clamped[o];
            red_lum2[lane] += red_lum2[o];
            red_rays[lane] += red_rays[o];
            red_len[lane] += red_len[o];
            red_sky[lane] += red_sky[o];
            red_key[lane] += red_key[o];
            let b = red_best[o];
            let b_at = red_best_at[o];
            if (b > red_best[lane] || (b == red_best[lane] && b_at < red_best_at[lane])) {
                red_best[lane] = b;
                red_best_at[lane] = b_at;
                red_material[lane] = red_material[o];
            }
        }
    }
    if (lane != 0u) {
        return;
    }
    f.sum_r = red_sum[0].x;
    f.sum_g = red_sum[0].y;
    f.sum_b = red_sum[0].z;
    f.clamped_r = red_clamped[0].x;
    f.clamped_g = red_clamped[0].y;
    f.clamped_b = red_clamped[0].z;
    f.lum2 = red_lum2[0];
    f.best_lum = red_best[0];
    f.best_material = red_material[0];
    f.rays = red_rays[0];
    f.ray_len = red_len[0];
    f.sky_open = red_sky[0];
    f.key_open = red_key[0];
    faces[at] = f;
}
