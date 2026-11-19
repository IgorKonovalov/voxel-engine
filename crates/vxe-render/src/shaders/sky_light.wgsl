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

// A unit direction drawn uniformly over the solid angle of the disc of angular radius `r`
// about unit `l`, from `u` and `v` in [0, 1): `1 - cos` taken as `2 sin^2(r / 2) u`, which
// keeps its digits at the sun's 4.65 mrad.
fn disc_direction(l: vec3<f32>, r: f32, u: f32, v: f32) -> vec3<f32> {
    let h = sin(0.5 * r);
    let omc = 2.0 * h * h * u;
    let sin_t = sqrt(max(omc * (2.0 - omc), 0.0));
    let phi = 2.0 * PI * v;
    let a = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(l.x) > 0.9);
    let t = normalize(cross(a, l));
    let b = cross(l, t);
    return normalize(l * (1.0 - omc) + (t * cos(phi) + b * sin(phi)) * sin_t);
}

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

// The key light's sample at a hit at `p`, normal `n`, shadow normal `ns`, lighting pixel `px`:
// The key light's visibility at a hit, the share of `key_light_at(p, n)` that arrives: 1 where
// one shadow ray toward a uniform point of its disc reaches the sky, times its transmittance
// through SHADOW_ABSORB materials, else 0; -1 where no key light falls on the face (none set, it
// faces away, or LIGHTING_HOOK_KEY_OFF). Under LIGHTING_PENUMBRA_APPROACH the ray runs toward
// the disc's centre and the visibility is its closest-approach visibility (walk.wgsl's
// `approach_on`) times its transmittance; under LIGHTING_HOOK_KEY_NO_JITTER (a reading hook) the
// one ray runs toward the disc's centre. The temporal pass keeps its running mean in the
// visibility pair's g and lights the face by `key_light_at` times that mean (ADR-0070's pair,
// ADR-0027's ray).
fn key_visibility(p: vec3<f32>, n: vec3<f32>, ns: vec3<f32>, px: vec2<u32>) -> f32 {
    let k = key_light_at(p, n);
    if (all(k <= vec3<f32>(0.0)) || (params.lighting & LIGHTING_HOOK_KEY_OFF) != 0u) {
        return -1.0;
    }
    if ((params.lighting & LIGHTING_PENUMBRA_APPROACH) != 0u) {
        approach_on = true;
        approach_tan = max(tan(sky.key_radius), 1.0e-4);
        approach_min = 1.0;
        let hit = sky_shadowed(sky_origin(p, ns), sky.key_dir, KEY_RAY_LEN);
        approach_on = false;
        return select(clamp(approach_min, 0.0, 1.0) * exp(-absorb_tau), 0.0, hit);
    }
    var d = disc_direction(sky.key_dir, sky.key_radius, rand(px, KEY_DIM), rand(px, KEY_DIM + 1u));
    if ((params.lighting & LIGHTING_HOOK_KEY_NO_JITTER) != 0u) {
        d = sky.key_dir;
    }
    if (sky_shadowed(sky_origin(p, ns), d, KEY_RAY_LEN)) {
        return 0.0;
    }
    return exp(-absorb_tau);
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
    if (count == 0u) {
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
