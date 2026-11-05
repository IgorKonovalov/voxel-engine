// The bake's sampler (ADR-0046): one round of emissive samples for each listed cube face of a
// chunk, at unit emission (EMISSION_UNIT), with its own random sequence (RAND_HASHED). Compiled
// after march.wgsl and light.wgsl, whose bindings (group 0), candidates (`draw_candidates`,
// `face_point`), shadow ray (`shadowed`) and the frame's candidate-table slots it uses: the job's
// table is written there before its submit (`bake::job_table`). The CPU keeps the running sums
// and decides each face's stop (`bake::Bake`); this pass only returns one round's sums.
//
// Bindings, group 3:
//   0  uniform  job    the submit's uniform (`bake::JobUniform`)
//   1  storage  faces  `job.count` faces (`bake::FaceWords`): voxel, face, the first sample and
//                      the samples of this round in; the sums out
//
// A face's samples are a function of its key, `job.seed` and the sample's index alone, and one
// invocation adds them in index order, so a result does not depend on the batching or on the
// jobs' order. Each sample is the light pass's estimate at a uniform point of the face with the
// face's normal, times the window `(1 - (d / L)^4)^2`, d the distance from the point to the kept
// emitter point and L `job.radius` in voxels: it falls to 0 at L, so the table's edge is no seam.

struct BakeJob {
    count: u32,
    seed: u32,
    radius: f32,
    // 1: the second moment and the kept emitter read the clamped estimate.
    clamp: u32,
};

struct BakeFace {
    voxel_x: i32,
    voxel_y: i32,
    voxel_z: i32,
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
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

@group(3) @binding(0) var<uniform> job: BakeJob;
@group(3) @binding(1) var<storage, read_write> faces: array<BakeFace>;

@compute @workgroup_size(64, 1, 1)
fn bake_round(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= job.count) {
        return;
    }
    reset_walk();
    var f = faces[gid.x];
    let v = vec3<i32>(f.voxel_x, f.voxel_y, f.voxel_z);
    let key = cache_key(v, f.face);
    let stream = hash_u32(job.seed ^ hash_u32(key.x ^ hash_u32(key.y)));
    let nf = face_dir(f.face);
    var sum = vec3<f32>(0.0);
    var clamped = vec3<f32>(0.0);
    var lum2 = 0.0;
    var best_lum = 0.0;
    var best_material = 0u;
    var rays = 0u;
    var ray_len = 0.0;
    for (var s = 0u; s < f.samples; s++) {
        let px = vec2<u32>(stream, f.first + s);
        let p = face_point(v, f.face, px);
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
            best_material = lookup(vec3<i32>(floor(r.x - 0.5 * face_dir(r.face)))).material;
        }
    }
    f.sum_r = sum.x;
    f.sum_g = sum.y;
    f.sum_b = sum.z;
    f.clamped_r = clamped.x;
    f.clamped_g = clamped.y;
    f.clamped_b = clamped.z;
    f.lum2 = lum2;
    f.best_lum = best_lum;
    f.best_material = best_material;
    f.rays = rays;
    f.ray_len = ray_len;
    faces[gid.x] = f;
}
