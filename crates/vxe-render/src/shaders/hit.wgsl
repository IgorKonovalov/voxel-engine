// The hit pass (ADR-0002, ADR-0004, ADR-0018), one invocation per pixel: the primary ray's
// walk and a hit record per pixel, no lighting; its outputs, the split walk's list, the
// translucent walk and the entry points.
//
// Bindings, group 1 (the hit pass's outputs, the hit buffer):
//   0  storage texture  hit_pos_out    rgba32float: the hit's world position (voxels) and t;
//                                      (0, 0, 0, BIG) on a miss
//   1  storage texture  hit_info_out   rgba16uint: the shading normal as two
//                                      snorm16 octahedral coordinates (exact for the six axis
//                                      normals), the material id, and flags: bit 0 hit, bit 1
//                                      field-surface hit, bit 2 translucent layers in front,
//                                      bits 3..5 the entry face's axis plus one (0: the ray
//                                      started inside rock)
//   2  storage texture  layer_out      rgba16float: the translucent solid in front of the hit
//                                      (ADR-0047): its reflected light `R * face_light` before
//                                      exposure in rgb, the luminance of its transmittance in a
//                                      (FSR 2's reactive mask reads it); in the steps view the
//                                      primary ray's counters instead; in the cracks view 1 in r
//                                      for a field walk's leak, else 0
//   3, 4                               the split walk's list and arguments (below)
//   5  storage texture  transmit_out   r32uint: the translucent solid's transmittance `T(L)` per
//                                      channel as rgb9e5 (`t9e5_pack`), 1 where none is in front
//   6  storage  glass_store            the store (`bake::Store`, temporal.wgsl's `store`), read
//                                      only: a translucent entry face's brick record

@group(1) @binding(0) var hit_pos_out: texture_storage_2d<rgba32float, write>;

@group(1) @binding(1) var hit_info_out: texture_storage_2d<rgba16uint, write>;

@group(1) @binding(2) var layer_out: texture_storage_2d<rgba16float, write>;

@group(1) @binding(5) var transmit_out: texture_storage_2d<r32uint, write>;

@group(1) @binding(6) var<storage, read> glass_store: array<u32>;

// The split field walk (FLAG_FIELD_SPLIT): the hit pipeline walks every ray as `march` does,
// its cells cubes, until the ray enters a full chunk's brick that the field can hit in (a band
// brick without BAND_CUBES_BIT), and appends the ray there to `split_list` with what the field
// walk resumes from; `field_resume`, in a dispatch of `split_args.groups` workgroups, walks
// the field from it. A ray entry: the pixel (x in bits 0..16, y in 16..32), `t` as bits, the
// entered voxel relative to the window's first voxel (x, y in the third word, z and the
// entry axis plus one in the fourth). The counts start at zero each frame (the frame clears
// them); the first pixel writes the dispatch's y and z.
struct SplitList {
    count: atomic<u32>,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    rays: array<vec4<u32>>,
};

struct SplitArgs {
    groups: atomic<u32>,
    y: u32,
    z: u32,
};

@group(1) @binding(3) var<storage, read_write> split_list: SplitList;

@group(1) @binding(4) var<storage, read_write> split_args: SplitArgs;

// `field_resume`'s workgroup size (frame/settings.rs: SPLIT_GROUP).
const SPLIT_GROUP: u32 = 128u;

// A `march` miss's material when the split walk hands the ray on.
const SPLIT_HANDOFF: u32 = 0xFFFFFFFFu;

// temporal.wgsl's `store_cell` over `glass_store`: cell `c`'s value for direction `face` times its
// material's emission scale now in rgb, 1 in a; 0 where its brick has no record or the cell no
// face of that direction.
fn glass_store_cell(c: vec3<i32>, face: u32, shift: u32) -> vec4<f32> {
    let per = 3u - shift;
    let b = c >> vec3<u32>(per);
    let origin = vec3<i32>(
        i32(glass_store[STORE_ORIGIN]),
        i32(glass_store[STORE_ORIGIN + 1u]),
        i32(glass_store[STORE_ORIGIN + 2u]),
    );
    let size = vec3<i32>(
        i32(glass_store[STORE_SIZE]),
        i32(glass_store[STORE_SIZE + 1u]),
        i32(glass_store[STORE_SIZE + 2u]),
    );
    let w = (b >> vec3<u32>(3u)) - origin;
    if (any(w < vec3<i32>(0)) || any(w >= size)) {
        return vec4<f32>(0.0);
    }
    let block = glass_store[STORE_TABLE + u32(w.x + size.x * (w.y + size.y * w.z))] & ~TABLE_COARSE;
    if (block == 0u) {
        return vec4<f32>(0.0);
    }
    let s = b & vec3<i32>(7);
    let dir = glass_store[block + u32(s.x + 8 * (s.y + 8 * s.z))];
    if (dir == 0u) {
        return vec4<f32>(0.0);
    }
    let n = 1 << per;
    let l = c & vec3<i32>(n - 1);
    let sub = u32(l.x + n * (l.y + n * l.z));
    let cells = u32(n * n * n);
    let rec = block + (dir >> 11u) - BRICK_WORDS * cells + BRICK_WORDS * sub;
    let tail = glass_store[rec + 6u];
    if (((tail >> (16u + face)) & 1u) == 0u) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(t9e5_unpack(glass_store[rec + face]) * emission_scale[tail & 0xFFFFu], 1.0);
}

// temporal.wgsl's `store_brick_value` over `glass_store`, in rgb alone: face `face` of voxel `v`,
// bilinear over the 2 x 2 cells that have the direction, else its own cell's, else 0.
fn glass_store_value(v: vec3<i32>, face: u32) -> vec3<f32> {
    let shift = select(3u, 2u, (glass_store[STORE_FLAGS] & 2u) != 0u);
    var sum = vec3<f32>(0.0);
    var weight = 0.0;
    for (var i = 0; i < 4; i++) {
        let tap = store_tap(v, face, shift, i);
        let r = glass_store_cell(tap.cell, face, shift);
        sum += r.xyz * (tap.w * r.w);
        weight += tap.w * r.w;
    }
    if (weight > 0.0) {
        return sum / weight;
    }
    return glass_store_cell(v >> vec3<u32>(shift), face, shift).xyz;
}

// What a translucent solid in front of a hit leaves to the composite (ADR-0047): the pixel is
// `reflected * exposure + transmit * behind`.
struct Glass {
    // `R * face_light` before exposure.
    reflected: vec3<f32>,
    // `T(L)` per channel.
    transmit: vec3<f32>,
};

// Schlick's Fresnel of a dielectric at F0 0.04.
const GLASS_F0: f32 = 0.04;

// A translucent row's transmittance through one voxel, per channel, from its albedo alone:
// `0.8 * (0.5 + 0.5 * albedo / max(albedo))`.
fn glass_tint(albedo: vec3<f32>) -> vec3<f32> {
    let top = max(max(albedo.x, max(albedo.y, albedo.z)), 1e-6);
    return 0.8 * (0.5 + 0.5 * albedo / top);
}

// The reflected share of the entry face of translucent voxel `v`, entered at `o + d t` across
// `axis`: Schlick's `R` at the view angle times `face_light = E / PI`, untinted. `E` is the face's
// brick record (`glass_store_value`, the read an opaque face's temporal pass makes) plus, while
// the lamp is on, the lamp's `PI * intensity * cone * n.l / dist^2` with no shadow ray (the lamp
// sits at the eye, whose ray reached the face). Unlit (`lit` false, the albedo view)
// `face_light` is 1.
fn glass_reflected(o: vec3<f32>, d: vec3<f32>, t: f32, v: vec3<i32>, axis: i32, lit: bool) -> vec3<f32> {
    let nf = select(vec3<f32>(0.0), vec3<f32>(-sign(d[axis])), axis_mask(axis));
    let c = 1.0 - min(abs(dot(normalize(d), nf)), 1.0);
    let c2 = c * c;
    let r = GLASS_F0 + (1.0 - GLASS_F0) * c2 * c2 * c;
    if (!lit) {
        return vec3<f32>(r);
    }
    var e = glass_store_value(v, store_face(axis, nf));
    if ((params.lighting & LIGHTING_LAMP) != 0u) {
        let p = o + d * t;
        let l = normalize(params.light_pos - p);
        e += vec3<f32>(PI * lamp(p) * max(dot(nf, l), 0.0));
    }
    return r * e * (1.0 / PI);
}

// The translucent walk: along `o + d t` for `0 <= t < t_end` (the opaque hit's `t`, or BIG on
// a miss), the same DDA as `march` over `tlookup`'s cells. It sums the path length `L` through the
// translucent voxels it enters, up to `params.translucent_layers` voxels of length, and filters
// by it per voxel's row: `T *= glass_tint(albedo)^segment`. The first translucent voxel's entry
// face reflects (`glass_reflected`); a ray that starts inside a translucent voxel has no entry
// face and reflects nothing.
fn translucent_march(o: vec3<f32>, d: vec3<f32>, t_end: f32, lit: bool) -> Glass {
    var out = Glass(vec3<f32>(0.0), vec3<f32>(1.0));
    let bmin_i = params.window_min * 64;
    let bmax_i = (params.window_min + params.window_size) * 64;
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
        return out;
    }
    reset_walk();
    var t = max(t0, 0.0);
    var v = voxel_at(o + d * t, d);
    var axis = -1;
    if (t0 > 0.0) {
        axis = select(select(2, 1, tmin.y == t0), 0, tmin.x == t0);
        let face = select(bmax_i[axis], bmin_i[axis], d[axis] > 0.0);
        v = advance(o, d, inv, t, axis, face);
    }
    v = clamp(v, bmin_i, bmax_i - vec3<i32>(1));
    let cap = f32(params.translucent_layers);
    var len = 0.0;
    var entered = false;

    for (var step = 0u; step < MAX_STEPS; step++) {
        if (any(v < bmin_i) || any(v >= bmax_i) || t >= t_end) {
            return out;
        }
        let c = tlookup(v);
        let size = select(c.size, c.cell, c.size == 0);
        let cell_min = floor_div(v, size) * size;
        let edge = cell_min + select(vec3<i32>(0), vec3<i32>(size), d > vec3<f32>(0.0));
        let tk = select(vec3<f32>(BIG), (vec3<f32>(edge) - o) * inv, d != vec3<f32>(0.0));
        var k = 0;
        if (tk.y < tk[k]) { k = 1; }
        if (tk.z < tk[k]) { k = 2; }
        if (c.size == 0) {
            if (!entered) {
                entered = true;
                if (axis >= 0) {
                    out.reflected = glass_reflected(o, d, t, v, axis, lit);
                }
            }
            let seg = max(min(min(tk[k], t_end) - t, cap - len), 0.0);
            out.transmit *= pow(glass_tint(materials[c.material * 4u].xyz), vec3<f32>(seg));
            len += seg;
            if (len >= cap) {
                return out;
            }
        }
        t = tk[k];
        axis = k;
        v = advance(o, d, inv, t, k, edge[k]);
    }
    return out;
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.size.x || gid.y >= params.size.y) {
        return;
    }
    if (FIELD_SURFACE) {
        field_pixel(gid.xy, false, 0.0, vec3<i32>(0), -1);
        return;
    }
    if ((params.flags & FLAG_FIELD_SPLIT) != 0u) {
        split_pixel(gid.xy);
        return;
    }
    let ray = pixel_ray(gid.xy);
    let o = ray.o;
    let d = ray.d;
    walk_steps = 0u;
    loads = 0u;
    let h = march(o, d, BIG, (params.flags & FLAG_CUTAWAY) != 0u);
    // Read before the normal modes' lookups touch the walk and the counters.
    let translucent_seen = walk.saw_translucent && params.translucent_layers > 0u;
    let counts = vec3<u32>(min(walk_steps, 255u), min(counted_loads(), 255u), min(counted_pool_reads(), 255u));
    let t_end = select(BIG, h.t, h.found);
    let view = view_mode();
    let px = vec2<i32>(gid.xy);
    var pos = vec4<f32>(0.0, 0.0, 0.0, BIG);
    var info = vec4<u32>(0u);
    if (h.found) {
        let n = surface_normal(params.views[view_of(gid.xy)], o, d, h);
        let flags = HIT_FOUND | (u32(h.axis + 1) << HIT_AXIS_SHIFT);
        pos = vec4<f32>(o + d * h.t, h.t);
        info = vec4<u32>(octa16_encode(n), h.material, flags);
    }
    var layer = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    var transmit = vec3<f32>(1.0);
    if (view == VIEW_STEPS) {
        // The primary ray's cost: its loop iterations, its storage loads and the pool loads
        // among them, each clamped at 255. No translucent walk in this view.
        layer = vec4<f32>(vec3<f32>(counts), 1.0);
    } else if (view == VIEW_CRACKS) {
        // The voxel walk has no leak. No translucent walk in this view.
        layer = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    } else if (translucent_seen) {
        info.w |= HIT_TRANSLUCENT;
        let glass = translucent_march(o, d, t_end, view != VIEW_ALBEDO);
        transmit = glass.transmit;
        layer = vec4<f32>(glass.reflected, luminance(glass.transmit));
    }
    textureStore(hit_pos_out, px, pos);
    textureStore(hit_info_out, px, info);
    textureStore(layer_out, px, layer);
    textureStore(transmit_out, px, vec4<u32>(t9e5_pack(transmit), 0u, 0u, 0u));
}

// The split walk's pixel (FLAG_FIELD_SPLIT, in the hit pipeline without FIELD_SURFACE): the ray
// walks as `march` walks, every solid voxel a cube hit recorded as `field_pixel` records one
// (its entry face's normal, the voxel's material, no translucent walk), until it enters a field
// brick; there it is appended to `split_list` and its record is left to `field_resume`. The
// cracks and steps views do not split (frame/encode.rs).
fn split_pixel(gid: vec2<u32>) {
    if (all(gid == vec2<u32>(0u))) {
        split_args.y = 1u;
        split_args.z = 1u;
    }
    let ray = pixel_ray(gid);
    let d = ray.d;
    split_walk = true;
    let h = march(ray.o, d, BIG, false);
    split_walk = false;
    if (!h.found && h.material == SPLIT_HANDOFF) {
        let k = atomicAdd(&split_list.count, 1u);
        if (k % SPLIT_GROUP == 0u) {
            atomicAdd(&split_args.groups, 1u);
        }
        let r = vec3<u32>(h.voxel - params.window_min * 64);
        split_list.rays[k] = vec4<u32>(
            gid.x | (gid.y << 16u),
            bitcast<u32>(h.t),
            r.x | (r.y << 16u),
            r.z | (u32(h.axis + 1) << 16u),
        );
        return;
    }
    var pos = vec4<f32>(0.0, 0.0, 0.0, BIG);
    var info = vec4<u32>(0u);
    if (h.found) {
        let flags = HIT_FOUND | (u32(h.axis + 1) << HIT_AXIS_SHIFT);
        pos = vec4<f32>(ray.o + d * h.t, h.t);
        info = vec4<u32>(octa16_encode(face_normal(d, h)), h.material, flags);
    }
    let px = vec2<i32>(gid);
    textureStore(hit_pos_out, px, pos);
    textureStore(hit_info_out, px, info);
    textureStore(layer_out, px, vec4<f32>(0.0, 0.0, 0.0, 1.0));
}

// The split walk's field half: one ray of `split_list` per invocation, the field walk from
// its hand-off, its record written as `field_pixel` writes it.
@compute @workgroup_size(128, 1, 1)
fn field_resume(@builtin(global_invocation_id) gid: vec3<u32>) {
    let k = gid.x;
    if (k >= atomicLoad(&split_list.count)) {
        return;
    }
    let e = split_list.rays[k];
    let px = vec2<u32>(e.x & 0xFFFFu, e.x >> 16u);
    let r = vec3<u32>(e.z & 0xFFFFu, e.z >> 16u, e.w & 0xFFFFu);
    let v = vec3<i32>(r) + params.window_min * 64;
    field_pixel(px, true, bitcast<f32>(e.y), v, i32(e.w >> 16u) - 1);
}
