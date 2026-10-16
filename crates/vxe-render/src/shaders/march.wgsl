// Primary-ray marcher over the brick-map mirror (ADR-0002), one invocation per pixel.
//
// Bindings, group 0:
//   0  uniform  params         camera, extent, light, target size, flags (bit 0: shadow ray)
//   1  storage  brick_pool     [slot][256 x u32], two u16 voxels per word, x fastest
//   2  storage  chunk_records  [record][512 x u32]; bit 31 set: pool slot, clear: uniform material
//   3  storage  chunk_table    [wrapped chunk]u32 record index, 0xFFFFFFFF when absent
//   4  storage  materials      [id][2 x vec4]: albedo rgb + roughness, emission rgb + flag bits
//   5  storage texture         rgba8unorm output
//
// World units are voxels. The walk is a DDA over cells of three sizes: a chunk that is absent
// is skipped as one 64^3 cell, a uniform-air brick as one 8^3 cell, and the voxels of a full
// brick one at a time. A uniform solid brick is hit where the ray enters it. Each step leaves
// the current cell through its nearest face; the coordinate on the exit axis is set exactly
// from the face, the others come from the float position, rounded toward the side the ray is
// heading so that a position on a boundary (a tie between axes) lands in the voxel entered.

struct Params {
    cam_pos: vec3<f32>,
    tan_half_fov: f32,
    cam_right: vec3<f32>,
    aspect: f32,
    cam_up: vec3<f32>,
    _pad0: f32,
    cam_fwd: vec3<f32>,
    _pad1: f32,
    extent_min: vec3<i32>,
    _pad2: i32,
    extent_size: vec3<i32>,
    _pad3: i32,
    light_pos: vec3<f32>,
    light_intensity: f32,
    light_dir: vec3<f32>,
    cos_outer: f32,
    size: vec2<u32>,
    cos_inner: f32,
    flags: u32,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> brick_pool: array<u32>;
@group(0) @binding(2) var<storage, read> chunk_records: array<u32>;
@group(0) @binding(3) var<storage, read> chunk_table: array<u32>;
@group(0) @binding(4) var<storage, read> materials: array<vec4<f32>>;
@group(0) @binding(5) var target_tex: texture_storage_2d<rgba8unorm, write>;

const ABSENT: u32 = 0xFFFFFFFFu;
const FULL_BIT: u32 = 0x80000000u;
const MAX_STEPS: u32 = 4096u;
const BIG: f32 = 3.0e38;
// Params.flags bits.
const FLAG_SHADOWS: u32 = 1u;

// What the walk found at a voxel: `size` is the edge of the empty cell to skip, or 0 for a
// solid voxel of `material`.
struct Cell {
    size: i32,
    material: u32,
};

struct Hit {
    found: bool,
    voxel: vec3<i32>,
    axis: i32,
    t: f32,
    material: u32,
};

fn floor_div(a: vec3<i32>, b: i32) -> vec3<i32> {
    return vec3<i32>(floor(vec3<f32>(a) / f32(b)));
}

fn lookup(v: vec3<i32>) -> Cell {
    let chunk = floor_div(v, 64);
    let rel = chunk - params.extent_min;
    let w = params.extent_size;
    if (any(rel < vec3<i32>(0)) || any(rel >= w)) {
        return Cell(64, 0u);
    }
    // Wrapped addressing; the window equals the extent here, so the wrap is the identity.
    let wr = ((chunk % w) + w) % w;
    let record = chunk_table[u32(wr.x + w.x * (wr.y + w.y * wr.z))];
    if (record == ABSENT) {
        return Cell(64, 0u);
    }
    let local = v - chunk * 64;
    let b = local / 8;
    let entry = chunk_records[record * 512u + u32(b.x + 8 * (b.y + 8 * b.z))];
    if ((entry & FULL_BIT) == 0u) {
        if (entry == 0u) {
            return Cell(8, 0u);
        }
        return Cell(0, entry);
    }
    let slot = entry & ~FULL_BIT;
    let l = local - b * 8;
    let i = u32(l.x + 8 * (l.y + 8 * l.z));
    let word = brick_pool[slot * 256u + i / 2u];
    let m = (word >> ((i & 1u) * 16u)) & 0xFFFFu;
    if (m == 0u) {
        return Cell(1, 0u);
    }
    return Cell(0, m);
}

// The voxel holding `p` on a ray of direction `d`: on an axis the ray travels down, a position
// exactly on a voxel boundary belongs to the voxel below.
fn voxel_at(p: vec3<f32>, d: vec3<f32>) -> vec3<i32> {
    return vec3<i32>(select(floor(p), ceil(p) - 1.0, d < vec3<f32>(0.0)));
}

// The first solid voxel along `o + d t` for `0 <= t <= t_limit`.
fn march(o: vec3<f32>, d: vec3<f32>, t_limit: f32) -> Hit {
    var miss = Hit(false, vec3<i32>(0), -1, 0.0, 0u);
    let bmin_i = params.extent_min * 64;
    let bmax_i = (params.extent_min + params.extent_size) * 64;
    let bmin = vec3<f32>(bmin_i);
    let bmax = vec3<f32>(bmax_i);
    let inv = select(vec3<f32>(BIG), 1.0 / d, d != vec3<f32>(0.0));
    let ta = (bmin - o) * inv;
    let tb = (bmax - o) * inv;
    let tmin = min(ta, tb);
    let tmax = max(ta, tb);
    let t0 = max(max(tmin.x, tmin.y), tmin.z);
    let t1 = min(min(tmax.x, tmax.y), tmax.z);
    if (t1 < max(t0, 0.0)) {
        return miss;
    }

    var t = max(t0, 0.0);
    var axis = -1;
    var v = voxel_at(o + d * t, d);
    if (t0 > 0.0) {
        axis = select(select(2, 1, tmin.y == t0), 0, tmin.x == t0);
        let p = o + d * t;
        let edge = i32(round(p[axis]));
        v[axis] = select(edge - 1, edge, d[axis] > 0.0);
    }
    v = clamp(v, bmin_i, bmax_i - vec3<i32>(1));

    for (var step = 0u; step < MAX_STEPS; step++) {
        if (any(v < bmin_i) || any(v >= bmax_i)) {
            return miss;
        }
        if (t > t_limit) {
            return miss;
        }
        let c = lookup(v);
        if (c.size == 0) {
            return Hit(true, v, axis, t, c.material);
        }
        let cell_min = floor_div(v, c.size) * c.size;
        let edge = cell_min + select(vec3<i32>(0), vec3<i32>(c.size), d > vec3<f32>(0.0));
        let tk = select(vec3<f32>(BIG), (vec3<f32>(edge) - o) * inv, d != vec3<f32>(0.0));
        var k = 0;
        if (tk.y < tk[k]) { k = 1; }
        if (tk.z < tk[k]) { k = 2; }
        t = tk[k];
        v = voxel_at(o + d * t, d);
        v[k] = select(edge[k] - 1, edge[k], d[k] > 0.0);
        axis = k;
    }
    return miss;
}

fn shade(o: vec3<f32>, d: vec3<f32>, h: Hit) -> vec3<f32> {
    var n = -d;
    if (h.axis >= 0) {
        n = vec3<f32>(0.0);
        n[h.axis] = -sign(d[h.axis]);
    }
    let p = o + d * h.t;
    let albedo = materials[h.material * 2u].xyz;
    let to_light = params.light_pos - p;
    let dist2 = max(dot(to_light, to_light), 1e-4);
    let l = to_light * inverseSqrt(dist2);
    let ndl = max(dot(n, l), 0.0);
    let cone = smoothstep(params.cos_outer, params.cos_inner, dot(-l, params.light_dir));
    var lit = params.light_intensity * cone * ndl / dist2;
    // One shadow ray, from half a voxel off the face toward the lamp; any rock before the lamp
    // removes its light.
    if ((params.flags & FLAG_SHADOWS) != 0u && lit > 0.0) {
        let s = march(p + n * 0.5, l, sqrt(dist2));
        if (s.found) {
            lit = 0.0;
        }
    }
    return albedo * lit;
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.size.x || gid.y >= params.size.y) {
        return;
    }
    let ndc = (vec2<f32>(gid.xy) + 0.5) / vec2<f32>(params.size) * 2.0 - 1.0;
    let d = normalize(
        params.cam_fwd
        + params.cam_right * (ndc.x * params.tan_half_fov * params.aspect)
        - params.cam_up * (ndc.y * params.tan_half_fov)
    );
    let o = params.cam_pos;
    let h = march(o, d, BIG);
    var c = vec3<f32>(0.0);
    if (h.found) {
        c = shade(o, d, h);
    }
    // Display encoding: clamp, then an approximate sRGB transfer.
    c = pow(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(1.0 / 2.2));
    textureStore(target_tex, vec2<i32>(gid.xy), vec4<f32>(c, 1.0));
}
