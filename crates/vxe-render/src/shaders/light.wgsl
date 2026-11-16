// The light pass: direct irradiance at each pixel's hit, read from the hit buffer, from the
// list of lights (`lights`, ADR-0044; the headlamp is slot 0 while it is on) and from one
// emissive voxel face. Compiled after the walk's modules, whose bindings
// (group 0), walk, hit-record helpers and random numbers it uses, and entity.wgsl; every
// shadow ray is `march`, and with ENTITY_SHADOWS the entity walk after it.
//
// Bindings, group 0 beyond common.wgsl's:
//  10  storage  light_samples  the mirror's light samples (see common.wgsl)
//
// Bindings, group 2:
//   0  texture_2d<f32>   hit_pos_in      the hit pass's rgba32float position + t
//   1  texture_2d<u32>   hit_info_in     the hit pass's rgba16uint normal, material, flags
//   2  storage texture   irradiance_out  rgba16float: the irradiance E at the hit (light
//                                        per unit area arriving over the hemisphere of the
//                                        shading normal), this frame's emissive sample in rgb
//                                        (0 on a cube face, whose light the store holds),
//                                        pre-exposed (`exposure_now`, clamped at HDR_MAX); in a
//                                        1 at a fresh pixel, else 0; 0 on a miss. In the steps
//                                        view the list's shadow rays' counters instead (see
//                                        `shade`)
//   3  storage texture   direct_out      rgba16float: the direct part, the list's exact terms
//                                        each times its colour summed in rgb and their
//                                        luminance (`colour_weight`) in a, pre-exposed and
//                                        clamped as above; 0 on a miss and in the steps view
//
// Each light of the list is a point or spot light: E = PI * intensity * cone * max(n.l, 0) /
// dist^2 times its window inside its reach and 0 beyond, so that the composite's Lambertian
// `albedo * E / PI` is the former march's `albedo * lit`; one shadow ray per lit pixel and
// light in reach with LIGHT_SHADOWS set, from SHADOW_OFFSET off the surface along the shadow
// normal toward the light.
//
// A pixel is fresh (ADR-0050) when one of its shadow rays entered the box of an entity with a
// moved part (`entity_fresh`), or its hit is a moved part's voxel: its direct light may differ
// from the frame before's at the same surface point, so the temporal pass takes this frame's
// direct sample as it is and the upscalers mark it changed.
//
// In the cutaway view a hit takes the hemisphere term `PI (0.3 + 0.7 (0.5 + 0.5 n.z))`
// instead, with no lamp, no emission and no shadow ray. Every shadow ray takes the frame's
// shadow walk (`shadowed`): the full walk, or the coarse walk that treats any 2^3 block with a
// set opaque bit as solid. Only the shaded and cracks views read this pass's irradiance; the
// steps view reads its shadow-ray counters; every other view skips the pass.

@group(0) @binding(10) var<storage, read> light_samples: array<vec4<i32>>;

// Whether the list's shadow rays also walk the entities (ADR-0050): on in the light and
// gradient passes, off in the probe rays and the bake, which see no entity.
override ENTITY_SHADOWS: bool = false;

@group(2) @binding(0) var hit_pos_in: texture_2d<f32>;

@group(2) @binding(1) var hit_info_in: texture_2d<u32>;

@group(2) @binding(2) var irradiance_out: texture_storage_2d<rgba16float, write>;

@group(2) @binding(3) var direct_out: texture_storage_2d<rgba16float, write>;

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
    shadow_scale = entity_shadow_scale(rec);
    if (view == VIEW_STEPS) {
        // The cost of each hit's shadow rays toward the lights in reach, lit or not, in the
        // frame's shadow walk: their loop iterations, storage loads and pool loads summed
        // (unclamped), and in a the rays cast; 0 where none is cast.
        var counts = vec4<f32>(0.0);
        if (rec.found && (params.flags & FLAG_CUTAWAY) == 0u) {
            let ray = pixel_ray(full_px(gid.xy));
            let o = shadow_origin(rec.p, shadow_normal_of(rec, ray.d));
            var steps = 0u;
            var storage = 0u;
            var pool = 0u;
            var rays = 0u;
            for (var i = 0u; i < lights.count; i++) {
                let li = lights.l[i];
                if ((li.flags & LIGHT_ON) == 0u || (li.flags & LIGHT_SHADOWS) == 0u || !light_reaches(li, rec.p)) {
                    continue;
                }
                let to_light = li.pos - o;
                let dist = max(length(to_light), 1e-2);
                walk_steps = 0u;
                loads = 0u;
                shadowed(o, to_light / dist, dist);
                steps += walk_steps;
                storage += counted_loads();
                pool += counted_pool_reads();
                rays += 1u;
            }
            counts = vec4<f32>(f32(steps), f32(storage), f32(pool), f32(rays));
        }
        textureStore(irradiance_out, px, counts);
        textureStore(direct_out, px, vec4<f32>(0.0));
        return;
    }
    // The direct part: rgb, and its luminance in w.
    var e = vec4<f32>(0.0);
    var emissive = vec3<f32>(0.0);
    entity_fresh = false;
    if (rec.found) {
        let ray = pixel_ray(full_px(gid.xy));
        let n = rec.n;
        if ((params.flags & FLAG_CUTAWAY) != 0u) {
            e = vec4<f32>(PI * (0.3 + 0.7 * (0.5 + 0.5 * n.z)));
        } else {
            let ns = shadow_normal_of(rec, ray.d);
            e = PI * lights_lit_rgb(rec.p, n, ns);
            // A cube face's emissive light is the store's (temporal.wgsl): no sample, no shadow
            // ray; so is an entity hit's under LIGHTING_ENTITY_WORLD. Any other hit draws its own.
            if (!reads_store(rec) && !entity_reads_store(rec)) {
                emissive = resolve_emissive(draw_candidates(gid.xy, rec.p, n), rec.p, ns);
            }
        }
    }
    // The emissive sample in rgb; the list's (or the cutaway's hemisphere) in the direct
    // target. Stored pre-exposed: times this frame's exposure, at most HDR_MAX (ADR-0033).
    let ex = exposure_now();
    let fresh = rec.found && (entity_fresh || entity_moved(rec));
    textureStore(irradiance_out, px, vec4<f32>(min(emissive * ex, vec3<f32>(HDR_MAX)), select(0.0, 1.0, fresh)));
    textureStore(direct_out, px, min(e * ex, vec4<f32>(HDR_MAX)));
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

// Light `li`'s `intensity * cone * max(n.l, 0) / dist^2` times its window at `p` with shading
// normal `n`, zero when its shadow ray finds rock before the light. The ray leaves the shadow
// origin (`p` offset along `ns`) aimed at the light itself, as the emissive and baked rays do:
// a ray along the direction from `p` would run parallel to the true one, half a voxel off the
// surface, and clear the edge of a step that shadows a face next to a near light. Out of its
// reach no ray is cast. With ENTITY_SHADOWS the ray then walks the entities to the light.
fn light_lit(li: Light, p: vec3<f32>, n: vec3<f32>, ns: vec3<f32>) -> f32 {
    var lit = light_term(li, p, n);
    if ((li.flags & LIGHT_SHADOWS) != 0u && lit > 0.0) {
        let o = shadow_origin(p, ns);
        let to_light = li.pos - o;
        let dist2 = max(dot(to_light, to_light), 1e-4);
        let l = to_light * inverseSqrt(dist2);
        let dist = sqrt(dist2);
        if (shadowed(o, l, dist) || (ENTITY_SHADOWS && entity_march(o, l, dist).found)) {
            lit = 0.0;
        }
    }
    return lit;
}

// The list's `light_lit` at `p`, each weighed by its colour's luminance.
fn lights_lit(p: vec3<f32>, n: vec3<f32>, ns: vec3<f32>) -> f32 {
    return lights_lit_rgb(p, n, ns).w;
}

// The list's `light_lit` at `p`, each times its colour summed in rgb, and weighed by its
// colour's luminance summed in w (a white light adds the same value to all four).
fn lights_lit_rgb(p: vec3<f32>, n: vec3<f32>, ns: vec3<f32>) -> vec4<f32> {
    var sum = vec4<f32>(0.0);
    for (var i = 0u; i < lights.count; i++) {
        let li = lights.l[i];
        let lit = light_lit(li, p, n, ns);
        sum += vec4<f32>(lit * li.colour, lit * colour_weight(li.colour));
    }
    return sum;
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
    march_material = false;
    let s = march(o, d, len, false);
    march_material = true;
    coarse_span = vec2<f32>(BIG, -BIG);
    return s.found;
}

// The unit normal of face `f` (2 k toward -k, 2 k + 1 toward +k).
fn face_dir(f: u32) -> vec3<f32> {
    let s = select(-1.0, 1.0, (f & 1u) == 1u);
    return select(vec3<f32>(0.0), vec3<f32>(s), axis_mask(i32(f / 2u)));
}
