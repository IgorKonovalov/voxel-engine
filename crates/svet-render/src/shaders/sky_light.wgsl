// The sky's light at a hit (ADR-0027): the key light's directional sample, one shadow ray
// toward a point of its disc, and the sky rays' visibility, cosine-weighted about the shading
// normal (ADR-0070).
// Compiled into the light pass alone, after light.wgsl (the shadow walk), sky_atmosphere.wgsl
// and sky.wgsl; never into the bake, which takes no sky term, nor the probe or gradient
// passes.
//
// An absent chunk is air to the key light's ray: one that leaves the resident world or the
// window has reached the sun. A sky ray that ends without a hit (its cap, an absent chunk, the
// window's face) reached the sky iff it ended above its column's top (`sky_ray_open`).

// The random-number dimensions of the key light's disc sample, above the candidates'.
const KEY_DIM: u32 = 1000u;

// A key light's shadow ray runs until it leaves the window (or MAX_STEPS).
const KEY_RAY_LEN: f32 = 1.0e6;

// The key and sky rays' origin offset at the light pass's pixel beyond its shadow origin:
// `LIFT` voxels along the shading normal times the far normal's blend weight where that is over
// one half, else 0 (`set_ray_lift`). A nonzero lift also sets the walk's `skip_origin_cell`.
var<private> ray_lift: vec3<f32> = vec3<f32>(0.0);

// The key and sky rays' lift in voxels at a far normal's full weight; an estimate.
const LIFT: f32 = 1.0;

// Sets `ray_lift` for a world cube-face hit `rec` seen by view `v` in the face normal mode: its
// shading normal times LIFT and the far normal's weight, where the weight is over one half.
fn set_ray_lift(rec: HitRecord, v: View) {
    ray_lift = vec3<f32>(0.0);
    if (rec.entity || rec.field || rec.axis < 0 || normal_mode() != NORMAL_FACE
        || (params.lighting & LIGHTING_HOOK_NO_STAIR) != 0u) {
        return;
    }
    let w = far_normal_weight(v, rec.t);
    if (w > 0.5) {
        ray_lift = rec.n * (LIFT * w);
    }
}

// Where the key and sky rays of a hit at `p` with shadow normal `ns` start: its shadow origin,
// lifted by `ray_lift`.
fn sky_origin(p: vec3<f32>, ns: vec3<f32>) -> vec3<f32> {
    return shadow_origin(p, ns) + ray_lift;
}

// Whether a lighting ray from `o` along `d` finds rock (or, with ENTITY_SHADOWS, an entity)
// before the sky. A lifted ray (`ray_lift`) steps out of a solid voxel it starts in. Leaves in
// `absorb_tau` the optical depth through SHADOW_ABSORB materials the ray passed (walk.wgsl's
// `absorb_on`), 0 with no style set.
fn sky_shadowed(o: vec3<f32>, d: vec3<f32>, len: f32) -> bool {
    skip_origin_cell = any(ray_lift != vec3<f32>(0.0));
    absorb_on = (params.lighting & LIGHTING_SHADOW_STYLES) != 0u;
    absorb_tau = 0.0;
    let s = shadowed_styled(o, d, len);
    absorb_on = false;
    skip_origin_cell = false;
    return s || (ENTITY_SHADOWS && entity_march(o, d, len).found);
}

// The key light's shadow ray's length: `params.key_cap` voxels where it is set (a ray clear that
// far counts lit), else KEY_RAY_LEN.
fn key_ray_len() -> f32 {
    return select(KEY_RAY_LEN, params.key_cap, params.key_cap > 0.0);
}

// The key light's sample at a hit at `p`, normal `n`, shadow normal `ns`, lighting pixel `px`:
// The key light's visibility at a hit, the share of `key_light_at(p, n)` that arrives: 1 where
// one shadow ray toward a uniform point of its disc reaches the sky, times its transmittance
// through SHADOW_ABSORB materials, else 0; -1 where no key light falls on the face (none set, it
// faces away, or LIGHTING_HOOK_KEY_OFF). Under `approach` (`key_approach_at`) the ray runs toward
// the disc's centre and the visibility is its closest-approach visibility (walk.wgsl's
// `approach_on`) times its transmittance; under LIGHTING_HOOK_KEY_NO_JITTER (a reading hook) the
// one ray runs toward the disc's centre. The temporal pass keeps its running mean in the
// visibility pair's g and lights the face by `key_light_at` times that mean (ADR-0070's pair,
// ADR-0027's ray).
fn key_visibility(p: vec3<f32>, n: vec3<f32>, ns: vec3<f32>, px: vec2<u32>, approach: bool) -> f32 {
    let k = key_light_at(p, n);
    if (all(k <= vec3<f32>(0.0)) || (params.lighting & LIGHTING_HOOK_KEY_OFF) != 0u) {
        return -1.0;
    }
    if (approach) {
        approach_on = true;
        approach_tan = max(tan(sky.key_radius), 1.0e-4);
        approach_min = 1.0;
        let hit = sky_shadowed(sky_origin(p, ns), sky.key_dir, key_ray_len());
        approach_on = false;
        return select(clamp(approach_min, 0.0, 1.0) * exp(-absorb_tau), 0.0, hit);
    }
    var d = disc_direction(sky.key_dir, sky.key_radius, rand(px, KEY_DIM), rand(px, KEY_DIM + 1u));
    if ((params.lighting & LIGHTING_HOOK_KEY_NO_JITTER) != 0u) {
        d = sky.key_dir;
    }
    if (sky_shadowed(sky_origin(p, ns), d, key_ray_len())) {
        return 0.0;
    }
    return exp(-absorb_tau);
}

// How far off the face's centre the near cover ray starts, in voxels: inside the air voxel in
// front, so a caster a voxel from the face shades it (SHADOW_OFFSET's half voxel steps past it).
const KEY_COVER_OFFSET: f32 = 0.01;

// The near cover source (`Lighting::key_cover` `ray`, Plan 0100 Phase 6, ADR-0084) at cube-face
// hit `rec`, shadow normal `ns`, seen by view `v`: where the far normal's
// weight is under one half, the column horizon at the hit
// (`key_horizon_at`, the temporal pass's own read) and, where that reads under 1, the larger of
// it and one ray from the face's centre (the level-0 voxel's behind the hit, half a voxel along
// `ns`) toward the disc's centre, times its transmittance; under LIGHTING_PENUMBRA_APPROACH the
// ray's closest-approach score. No random number, no history: a function of the face and the
// frame's key direction. -1 where the arm is off or the face is past the near.
fn key_cover_near(rec: HitRecord, ns: vec3<f32>, v: View) -> f32 {
    if ((params.lighting2 & LIGHTING2_KEY_COVER_RAY) == 0u || rec.axis < 0 || rec.entity
        || rec.field) {
        return -1.0;
    }
    if (far_normal_weight(v, rec.t) >= 0.5) {
        return -1.0;
    }
    let h = key_horizon_at(rec.p, ns);
    if (h >= 1.0) {
        return h;
    }
    let centre = vec3<f32>(store_voxel(rec.p, ns)) + 0.5 + 0.5 * ns;
    let lift = ray_lift;
    ray_lift = vec3<f32>(0.0);
    let approach = (params.lighting & LIGHTING_PENUMBRA_APPROACH) != 0u;
    approach_on = approach;
    approach_tan = max(tan(sky.key_radius), 1.0e-4);
    approach_min = 1.0;
    let hit = sky_shadowed(centre + ns * KEY_COVER_OFFSET, sky.key_dir, key_ray_len());
    approach_on = false;
    ray_lift = lift;
    let r = select(select(1.0, clamp(approach_min, 0.0, 1.0), approach) * exp(-absorb_tau), 0.0, hit);
    return max(h, r);
}

// The random-number dimensions of the sky rays, two a ray, above the key light's.
const SKY_DIM: u32 = 1002u;

// A sky ray's length cap in voxels: a ray that reaches it reached the sky iff it ends above
// its column's top.
const SKY_RAY_VOXELS: f32 = 512.0;

// Whether a sky ray from `o` along `d` finds rock (or, with ENTITY_SHADOWS, an entity) or ends
// at or under its column's top (`sky_ray_open`). A lifted ray (`ray_lift`) steps out of a solid
// voxel it starts in. Leaves `absorb_tau` as `sky_shadowed` does.
fn sky_ray_blocked(o: vec3<f32>, d: vec3<f32>, len: f32) -> bool {
    skip_origin_cell = any(ray_lift != vec3<f32>(0.0));
    absorb_on = (params.lighting & LIGHTING_SHADOW_STYLES) != 0u;
    absorb_tau = 0.0;
    let open = sky_ray_open(
        o,
        d,
        len,
        (params.lighting & LIGHTING_COARSE_SHADOW) != 0u,
        (params.lighting & LIGHTING_SHADOW_STYLES) != 0u,
        (params.lighting & LIGHTING_HOOK_SKY_END_IS_SKY) != 0u,
    );
    absorb_on = false;
    skip_origin_cell = false;
    return !open || (ENTITY_SHADOWS && entity_march(o, d, len).found);
}

// The sky rays' visibility at a hit at `p`, normal `n`, shadow normal `ns`, lighting pixel
// `px` (ADR-0070): the share of `sky.sky_rays` cosine-weighted rays about `n` that reach the
// sky above the horizon (`sky_ray_blocked` clear), each weighed by its transmittance through
// SHADOW_ABSORB materials; a ray below the horizon counts as
// blocked (the ground there is unknown, and it is rock more often than lit sky). Its
// expectation is the visibility the bake stores, so the temporal pass colours it as the baked
// one: times the sky's irradiance above the horizon about the normal. 0 with no ray.
fn sky_visibility_sample(p: vec3<f32>, n: vec3<f32>, ns: vec3<f32>, px: vec2<u32>) -> f32 {
    let count = sky.sky_rays;
    // A face looking down reaches no sky through the ground under the column rule: no ray
    // under LIGHTING2_SKY_DOWN_SKIP.
    if (count == 0u || ((params.lighting2 & LIGHTING2_SKY_DOWN_SKIP) != 0u && n.z < -0.5)) {
        return 0.0;
    }
    let o = sky_origin(p, ns);
    var open = 0.0;
    for (var i = 0u; i < count; i++) {
        let d = cosine_direction(n, rand(px, SKY_DIM + 2u * i), rand(px, SKY_DIM + 2u * i + 1u));
        if (d.z > 0.0 && !sky_ray_blocked(o, d, SKY_RAY_VOXELS)) {
            open += exp(-absorb_tau);
        }
    }
    return open / f32(count);
}
