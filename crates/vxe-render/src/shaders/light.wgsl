// The light pass: direct irradiance at each pixel's hit, read from the hit buffer, from the
// headlamp and from one emissive voxel face. Compiled after march.wgsl, whose bindings
// (group 0), walk, hit-record helpers and random numbers it uses; every shadow ray is
// `march`.
//
// Bindings, group 0 beyond march.wgsl's:
//  10  storage  light_samples  the mirror's light samples (see march.wgsl)
//
// Bindings, group 2:
//   0  texture_2d<f32>   hit_pos_in      the hit pass's rgba32float position + t
//   1  texture_2d<u32>   hit_info_in     the hit pass's rgba16uint normal, material, flags
//   2  storage texture   irradiance_out  rgba16float: the irradiance E at the hit (light
//                                        per unit area arriving over the hemisphere of the
//                                        shading normal), this frame's emissive sample in rgb
//                                        (0 on a cube face, whose light the store holds) and the lamp's
//                                        (white) in a, pre-exposed (`exposure_now`, clamped at
//                                        HDR_MAX); 0 on a miss. In the steps view the lamp
//                                        shadow ray's counters instead (see `shade`)
//
// The headlamp (Params.lighting bit 0) is a point light: E = PI * intensity * cone *
// max(n.l, 0) / dist^2, so that the composite's Lambertian `albedo * E / PI` is the former
// march's `albedo * lit`; one shadow ray per lit pixel, from SHADOW_OFFSET off the surface
// along the shadow normal toward the lamp.
//
// In the cutaway view a hit takes the hemisphere term `PI (0.3 + 0.7 (0.5 + 0.5 n.z))`
// instead, with no lamp, no emission and no shadow ray. Every shadow ray takes the frame's
// shadow walk (`shadowed`): the full walk, or the coarse walk that treats any 2^3 block with a
// set opaque bit as solid. Only the shaded and cracks views read this pass's irradiance; the
// steps view reads its shadow-ray counters; every other view skips the pass.

@group(0) @binding(10) var<storage, read> light_samples: array<vec4<i32>>;

@group(2) @binding(0) var hit_pos_in: texture_2d<f32>;

@group(2) @binding(1) var hit_info_in: texture_2d<u32>;

@group(2) @binding(2) var irradiance_out: texture_storage_2d<rgba16float, write>;

// The frame's candidate table (ADR-0025), in light_samples from `params.light_table_base`:
// a header slot (count, total weight as f32 bits), then per entry the start and length of an
// emitter chunk's range and the cumulative weight up to and including it (f32 bits), in
// chunk-coordinate order, for the chunks within the light radius of the camera.
fn table_slot(i: u32) -> vec4<u32> {
    return bitcast<vec4<u32>>(light_samples[params.light_table_base + i]);
}

@compute @workgroup_size(8, 8, 1)
fn shade(@builtin(global_invocation_id) gid: vec3<u32>) {
    shade_px(gid);
}

// The light pass at lighting pixel `gid.xy`.
fn shade_px(gid: vec3<u32>) {
    let view = view_mode();
    if (gid.x >= params.light_size.x || gid.y >= params.light_size.y || (view != VIEW_SHADED && view != VIEW_STEPS && view != VIEW_CRACKS)) {
        return;
    }
    let px = vec2<i32>(gid.xy);
    let rec = unpack_hit(textureLoad(hit_pos_in, px, 0), textureLoad(hit_info_in, px, 0));
    if (view == VIEW_STEPS) {
        // The cost of each hit's shadow ray toward the lamp, lit or not, in the frame's
        // shadow walk: its loop iterations, storage loads and pool loads (unclamped), a = 1;
        // a = 0 where no ray is cast.
        var counts = vec4<f32>(0.0);
        if (rec.found && (params.flags & FLAG_CUTAWAY) == 0u) {
            let ray = pixel_ray(full_px(gid.xy));
            let to_light = params.light_pos - rec.p;
            let dist = max(length(to_light), 1e-2);
            walk_steps = 0u;
            loads = 0u;
            shadowed(shadow_origin(rec.p, shadow_normal_of(rec, ray.d)), to_light / dist, dist);
            counts = vec4<f32>(f32(walk_steps), f32(counted_loads()), f32(counted_pool_reads()), 1.0);
        }
        textureStore(irradiance_out, px, counts);
        return;
    }
    var e = vec3<f32>(0.0);
    var emissive = vec3<f32>(0.0);
    if (rec.found) {
        let ray = pixel_ray(full_px(gid.xy));
        let n = rec.n;
        if ((params.flags & FLAG_CUTAWAY) != 0u) {
            e = vec3<f32>(PI * (0.3 + 0.7 * (0.5 + 0.5 * n.z)));
        } else {
            let ns = shadow_normal_of(rec, ray.d);
            if ((params.lighting & LIGHTING_LAMP) != 0u) {
                e = vec3<f32>(PI * lamp_lit(rec.p, n, ns));
            }
            // A cube face's emissive light is the store's (temporal.wgsl): no sample, no shadow
            // ray. Any other hit draws its own.
            if (!reads_store(rec)) {
                emissive = resolve_emissive(draw_candidates(gid.xy, rec.p, n), rec.p, ns);
            }
        }
    }
    // The emissive sample in rgb, the lamp's (white, or the cutaway's hemisphere) in a. Stored
    // pre-exposed: times this frame's exposure, at most HDR_MAX (ADR-0033).
    let ex = exposure_now();
    textureStore(
        irradiance_out,
        px,
        min(vec4<f32>(emissive * ex, e.x * ex), vec4<f32>(HDR_MAX)),
    );
}

// A uniform point of face `face` of voxel `v`, from `rand` at pixel `px` (dimensions 0 and 1;
// the candidates take 2 on).
fn face_point(v: vec3<i32>, face: u32, px: vec2<u32>) -> vec3<f32> {
    let ax = face / 2u;
    let t0 = select(vec3<f32>(0.0), vec3<f32>(1.0), axis_mask(i32((ax + 1u) % 3u)));
    let t1 = select(vec3<f32>(0.0), vec3<f32>(1.0), axis_mask(i32((ax + 2u) % 3u)));
    return face_centre(v, face) + t0 * (rand(px, 0u) - 0.5) + t1 * (rand(px, 1u) - 0.5);
}

// The unit face `face` of voxel `v`'s centre.
fn face_centre(v: vec3<i32>, face: u32) -> vec3<f32> {
    return vec3<f32>(v) + vec3<f32>(0.5) + 0.5 * face_dir(face);
}

// The headlamp's `intensity * cone * max(n.l, 0) / dist^2` at `p` with shading normal `n`,
// zero when the shadow ray (offset along `ns`) finds rock before the lamp.
fn lamp_lit(p: vec3<f32>, n: vec3<f32>, ns: vec3<f32>) -> f32 {
    let to_light = params.light_pos - p;
    let dist2 = max(dot(to_light, to_light), 1e-4);
    let l = to_light * inverseSqrt(dist2);
    let ndl = max(dot(n, l), 0.0);
    let cone = smoothstep(params.cos_outer, params.cos_inner, dot(-l, params.light_dir));
    var lit = params.light_intensity * cone * ndl / dist2;
    if ((params.flags & FLAG_SHADOWS) != 0u && lit > 0.0) {
        if (shadowed(shadow_origin(p, ns), l, sqrt(dist2))) {
            lit = 0.0;
        }
    }
    return lit;
}

// The walk a shadow ray leaves to the full walk at each end under the coarse mode, in voxels:
// more than a 2^3 block's diagonal (2 sqrt 3), so the blocks of the surface it leaves and of
// the face it aims at do not stop it.
const COARSE_MARGIN: f32 = 3.5;

// Whether rock lies on `o + d t` for `0 <= t <= len`: the full walk, or with
// `Params.lighting` bit 1 the coarse walk between COARSE_MARGIN of either end.
fn shadowed(o: vec3<f32>, d: vec3<f32>, len: f32) -> bool {
    if ((params.lighting & LIGHTING_COARSE_SHADOW) != 0u) {
        coarse_span = vec2<f32>(COARSE_MARGIN, len - COARSE_MARGIN);
    }
    let s = march(o, d, len, false);
    coarse_span = vec2<f32>(BIG, -BIG);
    return s.found;
}

// The unit normal of face `f` (2 k toward -k, 2 k + 1 toward +k).
fn face_dir(f: u32) -> vec3<f32> {
    let s = select(-1.0, 1.0, (f & 1u) == 1u);
    return select(vec3<f32>(0.0), vec3<f32>(s), axis_mask(i32(f / 2u)));
}
