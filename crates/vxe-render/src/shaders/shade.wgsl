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
//                                        sample (the sky's term is the temporal pass's), summed in rgb and their
//                                        luminance (`colour_weight`) in a, pre-exposed and
//                                        clamped as above; 0 on a miss. In the steps view the
//                                        key light's shadow ray's counters instead (see
//                                        `shade`)
//   4  storage, read      light_store     the store (`bake::Store`, temporal.wgsl's `store`),
//                                        read for `sky_baked`
//   5  storage texture   vis_out         rgba8unorm: the sky rays' visibility sample (ADR-0070,
//                                        `sky_visibility_sample`) in r, and 1 in a where the
//                                        sky rays were cast; 0 where none was (no sky,
//                                        `sky_rays` 0, a miss, the cutaway, the steps view, or a
//                                        face with a fresh baked visibility, `sky_baked`). The
//                                        key light's visibility sample (`key_visibility`) in g
//                                        and 1 in b where the key light falls on the face, else
//                                        0 in both; KEY_FROM_STORE in b and 0 in g where it
//                                        falls on the face and the store answers it with no ray
//                                        (`key_from_store`); the key light is not in
//                                        `direct_out`
//
@group(2) @binding(0) var hit_pos_in: texture_2d<f32>;

@group(2) @binding(1) var hit_info_in: texture_2d<u32>;

@group(2) @binding(2) var irradiance_out: texture_storage_2d<rgba16float, write>;

@group(2) @binding(3) var direct_out: texture_storage_2d<rgba16float, write>;

@group(2) @binding(4) var<storage, read> light_store: array<u32>;

@group(2) @binding(5) var vis_out: texture_storage_2d<rgba8unorm, write>;

// Whether the store holds a fresh sky visibility for face `face` of voxel `v` (ADR-0070): its
// cell's record has the direction and VIS_BAKED, and no SKY_STALE (a dig's ring before its
// re-bake lands). The sky rays are cast everywhere else; the temporal pass reads `vis_out`'s
// a to take the visibility pair's mean where they were and the baked term where not.
fn sky_baked(v: vec3<i32>, face: u32) -> bool {
    let shift = select(3u, 2u, (light_store[STORE_FLAGS] & 2u) != 0u);
    let per = 3u - shift;
    let c = v >> vec3<u32>(shift);
    let b = c >> vec3<u32>(per);
    let origin = vec3<i32>(
        i32(light_store[STORE_ORIGIN]),
        i32(light_store[STORE_ORIGIN + 1u]),
        i32(light_store[STORE_ORIGIN + 2u]),
    );
    let size = vec3<i32>(
        i32(light_store[STORE_SIZE]),
        i32(light_store[STORE_SIZE + 1u]),
        i32(light_store[STORE_SIZE + 2u]),
    );
    let w = (b >> vec3<u32>(3u)) - origin;
    if (any(w < vec3<i32>(0)) || any(w >= size)) {
        return false;
    }
    let block = light_store[STORE_TABLE + u32(w.x + size.x * (w.y + size.y * w.z))] & ~TABLE_COARSE;
    if (block == 0u) {
        return false;
    }
    let s = b & vec3<i32>(7);
    let dir = light_store[block + u32(s.x + 8 * (s.y + 8 * s.z))];
    if (dir == 0u) {
        return false;
    }
    let n = 1 << per;
    let l = c & vec3<i32>(n - 1);
    let sub = u32(l.x + n * (l.y + n * l.z));
    let cells = u32(n * n * n);
    let rec = block + (dir >> 11u) - BRICK_WORDS * cells + BRICK_WORDS * sub;
    let vis = light_store[rec + VIS_WORD + 1u];
    return ((light_store[rec + 6u] >> (16u + face)) & 1u) != 0u
        && (vis & VIS_BAKED) != 0u && (vis & SKY_STALE) == 0u;
}

// The first word of voxel `v`'s cell's record in the light pass's store binding; 0 where its
// brick has none.
fn light_record(v: vec3<i32>) -> u32 {
    let shift = select(3u, 2u, (light_store[STORE_FLAGS] & 2u) != 0u);
    let per = 3u - shift;
    let c = v >> vec3<u32>(shift);
    let b = c >> vec3<u32>(per);
    let block = light_block(b >> vec3<u32>(3u));
    if (block == 0u) {
        return 0u;
    }
    let s = b & vec3<i32>(7);
    let dir = light_store[block + u32(s.x + 8 * (s.y + 8 * s.z))];
    if (dir == 0u) {
        return 0u;
    }
    let n = 1 << per;
    let l = c & vec3<i32>(n - 1);
    let sub = u32(l.x + n * (l.y + n * l.z));
    return block + (dir >> 11u) - BRICK_WORDS * u32(n * n * n) + BRICK_WORDS * sub;
}

// Whether the store holds a fresh key light visibility for face `face` of voxel `v`
// (ADR-0079): its cell's record has the direction and KEY_BAKED, and no SKY_STALE (the
// channels share it). temporal.wgsl's `store_key_cell` reads the value by the same test.
fn key_baked(v: vec3<i32>, face: u32) -> bool {
    let rec = light_record(v);
    if (rec == 0u) {
        return false;
    }
    return ((light_store[rec + 6u] >> (16u + face)) & 1u) != 0u
        && (light_store[rec + KEY_WORD + 1u] & KEY_BAKED) != 0u
        && (light_store[rec + VIS_WORD + 1u] & SKY_STALE) == 0u;
}

// Whether the light pass leaves the key light's visibility at cube face hit `rec` (store voxel
// `v`, face `face`) of view `view` to the store, with no ray (ADR-0079): under
// LIGHTING_KEY_BAKED, where the far normal's weight is 1 (at least one half under
// LIGHTING_KEY_STEP) and the face's cell holds a fresh baked key visibility (`key_baked`).
fn key_from_store(rec: HitRecord, v: vec3<i32>, face: u32, view: View) -> bool {
    if ((params.lighting & LIGHTING_KEY_BAKED) == 0u || !reads_store(rec)) {
        return false;
    }
    let at = select(1.0, 0.5, (params.lighting & LIGHTING_KEY_STEP) != 0u);
    return far_normal_weight(view, rec.t) >= at && key_baked(v, face);
}

// Whether face `face` of voxel `v`, whose own cell holds no fresh baked visibility, takes the
// no-record prior's sky visibility instead of the sky rays: its cell holds no record of the
// direction baked under a sky and is not stale, under the `neighbour` or `chain` mode, and its
// chunk's own summary holds the direction baked under a sky and not SKY_STALE (no dig job is
// queued for the chunk), or LIGHTING_HOOK_COLUMN_PRIOR holds the column's answer. A face
// neighbour's summary does not answer the sky. temporal.wgsl's `store_sky_fallback` reads the
// value by the same test.
fn sky_chained(v: vec3<i32>, face: u32) -> bool {
    let mode = (light_store[STORE_FLAGS] >> 2u) & 3u;
    if (mode == 0u || mode == 2u || sky_stale(v)) {
        return false;
    }
    if ((params.lighting & LIGHTING_HOOK_COLUMN_PRIOR) != 0u) {
        return true;
    }
    let block = light_block(v >> vec3<u32>(6u));
    if (block == 0u) {
        return false;
    }
    let rec = block + STORE_SUMMARY_AT;
    let vis = light_store[rec + VIS_WORD + 1u];
    return ((light_store[rec + 6u] >> (16u + face)) & 1u) != 0u
        && (vis & VIS_BAKED) != 0u && (vis & SKY_STALE) == 0u;
}

// Chunk `c`'s block in the light pass's store binding; 0 for none or outside the table.
fn light_block(c: vec3<i32>) -> u32 {
    let origin = vec3<i32>(
        i32(light_store[STORE_ORIGIN]),
        i32(light_store[STORE_ORIGIN + 1u]),
        i32(light_store[STORE_ORIGIN + 2u]),
    );
    let size = vec3<i32>(
        i32(light_store[STORE_SIZE]),
        i32(light_store[STORE_SIZE + 1u]),
        i32(light_store[STORE_SIZE + 2u]),
    );
    let w = c - origin;
    if (any(w < vec3<i32>(0)) || any(w >= size)) {
        return 0u;
    }
    return light_store[STORE_TABLE + u32(w.x + size.x * (w.y + size.y * w.z))] & ~TABLE_COARSE;
}

// Whether voxel `v`'s cell's record holds a visibility marked SKY_STALE (a dig's ring, whose
// sky rays run until it is baked again).
fn sky_stale(v: vec3<i32>) -> bool {
    let shift = select(3u, 2u, (light_store[STORE_FLAGS] & 2u) != 0u);
    let per = 3u - shift;
    let c = v >> vec3<u32>(shift);
    let b = c >> vec3<u32>(per);
    let block = light_block(b >> vec3<u32>(3u));
    if (block == 0u) {
        return false;
    }
    let s = b & vec3<i32>(7);
    let dir = light_store[block + u32(s.x + 8 * (s.y + 8 * s.z))];
    if (dir == 0u) {
        return false;
    }
    let n = 1 << per;
    let l = c & vec3<i32>(n - 1);
    let sub = u32(l.x + n * (l.y + n * l.z));
    let cells = u32(n * n * n);
    let rec = block + (dir >> 11u) - BRICK_WORDS * cells + BRICK_WORDS * sub;
    return (light_store[rec + VIS_WORD + 1u] & SKY_STALE) != 0u;
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
        // The key light's shadow ray, cast as `key_visibility` casts it where the key light
        // reaches the hit (the approach penumbra's probes in its loads): its loop iterations,
        // storage loads and pool loads, and 1 in a; 0 where none is cast.
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
                var d = disc_direction(sky.key_dir, sky.key_radius, rand(gid.xy, KEY_DIM), rand(gid.xy, KEY_DIM + 1u));
                let approach = (params.lighting & LIGHTING_PENUMBRA_APPROACH) != 0u;
                if (approach) {
                    d = sky.key_dir;
                    approach_tan = max(tan(sky.key_radius), 1.0e-4);
                    approach_min = 1.0;
                }
                set_ray_lift(rec, params.views[view_of(full_px(gid.xy))]);
                walk_steps = 0u;
                loads = 0u;
                approach_on = approach;
                sky_shadowed(o + ray_lift, d, KEY_RAY_LEN);
                approach_on = false;
                key = vec4<f32>(f32(walk_steps), f32(counted_loads()), f32(counted_pool_reads()), 1.0);
            }
        }
        textureStore(irradiance_out, px, counts);
        textureStore(direct_out, px, key);
        textureStore(vis_out, px, vec4<f32>(0.0));
        return;
    }
    // The direct part: rgb, and its luminance in w.
    var e = vec4<f32>(0.0);
    var emissive = vec3<f32>(0.0);
    var vis = vec4<f32>(0.0);
    entity_fresh = false;
    if (rec.found) {
        let ray = pixel_ray(full_px(gid.xy));
        let n = rec.n;
        if ((params.flags & FLAG_CUTAWAY) != 0u) {
            e = vec4<f32>(PI * (0.3 + 0.7 * (0.5 + 0.5 * n.z)));
        } else {
            let ns = shadow_normal_of(rec, ray.d);
            e = PI * lights_lit_rgb(rec.p, n, ns);
            let view_here = params.views[view_of(full_px(gid.xy))];
            set_ray_lift(rec, view_here);
            // The key light's visibility sample (sky_light.wgsl) goes to the temporal pass, which
            // keeps its running mean and adds the key light by it; where the store answers it
            // (`key_from_store`) no ray is cast and the temporal pass reads the store. The sky
            // rays run where the hit's face holds no fresh baked visibility; their share that
            // reached the sky goes to the temporal pass too, which keeps its running mean.
            let sv = store_voxel(rec.p, ns);
            let sf = store_face(rec.axis, ns);
            if (key_from_store(rec, sv, sf, view_here)) {
                if (any(key_light_at(rec.p, n) > vec3<f32>(0.0))
                    && (params.lighting & LIGHTING_HOOK_KEY_OFF) == 0u) {
                    vis = vec4<f32>(0.0, 0.0, KEY_FROM_STORE, 0.0);
                }
            } else {
                let kv = key_visibility(rec.p, n, ns, gid.xy);
                if (kv >= 0.0) {
                    vis = vec4<f32>(0.0, kv, 1.0, 0.0);
                }
            }
            let baked = reads_store(rec) && (sky_baked(sv, sf) || sky_chained(sv, sf))
                && (params.lighting & LIGHTING_HOOK_SKY_RAY_ALWAYS) == 0u;
            if (!baked && sky_on() && sky.sky_rays > 0u) {
                vis = vec4<f32>(sky_visibility_sample(rec.p, n, ns, gid.xy), vis.y, vis.z, 1.0);
            }
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
    textureStore(vis_out, px, vis);
}
