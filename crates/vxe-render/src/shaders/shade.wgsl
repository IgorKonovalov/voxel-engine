// The light pass's entry (`shade`): at each lighting pixel's hit, read from the hit buffer,
// the list's lights and one emissive face (light.wgsl), the key light's directional sample
// and the sky rays (sky_light.wgsl). Compiled after light.wgsl, emissive.wgsl and the sky's
// modules.
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
//                                        each times its colour, and the key light's
//                                        sample, summed in rgb and their
//                                        luminance (`colour_weight`) in a, pre-exposed and
//                                        clamped as above; 0 on a miss. In the steps view the
//                                        key light's shadow ray's counters instead (see
//                                        `shade`)
//
@group(2) @binding(0) var hit_pos_in: texture_2d<f32>;

@group(2) @binding(1) var hit_info_in: texture_2d<u32>;

@group(2) @binding(2) var irradiance_out: texture_storage_2d<rgba16float, write>;

@group(2) @binding(3) var direct_out: texture_storage_2d<rgba16float, write>;

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
        // The key light's shadow ray, cast as `key_sample` casts it where the key light
        // reaches the hit: its loop iterations, storage loads and pool loads, and 1 in a; 0
        // where none is cast.
        var counts = vec4<f32>(0.0);
        var key = vec4<f32>(0.0);
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
                shadowed_styled(o, to_light / dist, dist);
                steps += walk_steps;
                storage += counted_loads();
                pool += counted_pool_reads();
                rays += 1u;
            }
            counts = vec4<f32>(f32(steps), f32(storage), f32(pool), f32(rays));
            if (any(key_light_at(rec.p, rec.n) > vec3<f32>(0.0))) {
                let d = disc_direction(sky.key_dir, sky.key_radius, rand(gid.xy, KEY_DIM), rand(gid.xy, KEY_DIM + 1u));
                set_ray_lift(rec, params.views[view_of(full_px(gid.xy))]);
                walk_steps = 0u;
                loads = 0u;
                sky_shadowed(o + ray_lift, d, KEY_RAY_LEN);
                key = vec4<f32>(f32(walk_steps), f32(counted_loads()), f32(counted_pool_reads()), 1.0);
            }
        }
        textureStore(irradiance_out, px, counts);
        textureStore(direct_out, px, key);
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
            set_ray_lift(rec, params.views[view_of(full_px(gid.xy))]);
            // The key light's directional sample and the sky rays (sky_light.wgsl), with the list's.
            let k = key_sample(rec.p, n, ns, gid.xy) + sky_ray_sample(rec.p, n, ns, gid.xy);
            e += vec4<f32>(k, luminance(k));
            // A cube face's emissive light is the store's (temporal.wgsl): no sample, no shadow
            // ray; so is an entity hit's under LIGHTING_ENTITY_WORLD. Any other hit draws its own.
            if (!reads_store(rec) && !entity_reads_store(rec)) {
                emissive = resolve_emissive(draw_candidates(gid.xy, rec.p, n), rec.p, ns);
            }
        }
    }
    // The emissive sample in rgb; the list's and the sky's (or the cutaway's hemisphere) in
    // the direct target. Stored pre-exposed: times this frame's exposure, at most HDR_MAX (ADR-0033).
    let ex = exposure_now();
    let fresh = rec.found && (entity_fresh || entity_moved(rec));
    textureStore(irradiance_out, px, vec4<f32>(min(emissive * ex, vec3<f32>(HDR_MAX)), select(0.0, 1.0, fresh)));
    textureStore(direct_out, px, min(e * ex, vec4<f32>(HDR_MAX)));
}
