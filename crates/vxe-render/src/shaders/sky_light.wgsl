// The sky's light at a hit (ADR-0027): the key light's directional sample, one shadow ray
// toward a point of its disc. Compiled into the light pass alone, after light.wgsl (the
// shadow walk), sky_atmosphere.wgsl and sky.wgsl; never into the bake, which takes no sky
// term, nor the probe or gradient passes.
//
// A lighting ray walks with the residency ball set (`absent_ball_r2`, walk.wgsl): an absent
// chunk inside it is solid; a ray that leaves the ball or the window has reached the sky.

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

// Whether a lighting ray from `o` along `d` finds rock (or, with ENTITY_SHADOWS, an entity)
// before the sky, with the residency ball set.
fn sky_shadowed(o: vec3<f32>, d: vec3<f32>, len: f32) -> bool {
    absent_ball_r2 = sky.ball_r2;
    let hit = shadowed(o, d, len);
    absent_ball_r2 = -1;
    return hit || (ENTITY_SHADOWS && entity_march(o, d, len).found);
}

// The key light's sample at a hit at `p`, normal `n`, shadow normal `ns`, lighting pixel `px`:
// `key_light_at(p, n)` where one shadow ray toward a uniform point of its disc reaches the
// sky, else 0; 0 with no key light. No firefly clamp: the term is bounded.
fn key_sample(p: vec3<f32>, n: vec3<f32>, ns: vec3<f32>, px: vec2<u32>) -> vec3<f32> {
    let k = key_light_at(p, n);
    if (all(k <= vec3<f32>(0.0))) {
        return vec3<f32>(0.0);
    }
    let d = disc_direction(sky.key_dir, sky.key_radius, rand(px, KEY_DIM), rand(px, KEY_DIM + 1u));
    if (sky_shadowed(shadow_origin(p, ns), d, KEY_RAY_LEN)) {
        return vec3<f32>(0.0);
    }
    return k;
}
