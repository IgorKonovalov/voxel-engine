// The sky as the frame's passes read it (ADR-0026): the sky uniform and the LUTs in group 0,
// and the lookups later plans rely on. Compiled after `common.wgsl` and
// `sky_atmosphere.wgsl`. With no sky set the uniform is all zero and `sky_on()` false.
//
// Bindings, group 0 (beside common.wgsl's):
//  16  uniform  sky                    the sky uniform (sky.rs `SkyUniform`)
//  17  texture  sky_transmittance_lut  the transmittance LUT, by radius and zenith cosine
//  18  texture  sky_view_lut           the sky-view LUT, radiance times `sky.view_scale`

@group(0) @binding(16) var<uniform> sky: SkyUniform;
@group(0) @binding(17) var sky_transmittance_lut: texture_2d<f32>;
@group(0) @binding(18) var sky_view_lut: texture_2d<f32>;

fn sky_on() -> bool {
    return (sky.flags & SKY_ON) != 0u;
}

// The sky's mean irradiance over the part above the horizon of the hemisphere about direction
// `dir`'s axis, from `e` (`E_sky` per direction, the sky irradiance pass's buffer as a pass
// binds it): `E_sky` over that part's cosine-weighted share, 1 for +z, a half sideways and 0
// for -z (`E_sky` is 0 there). A visibility counts a ray below the horizon blocked, so a face's
// sky term is its visibility times this, which is the sky ray's expectation.
fn sky_open_irradiance_in(e: array<vec4<f32>, 6>, dir: u32) -> vec3<f32> {
    if (dir == 4u) {
        return vec3<f32>(0.0);
    }
    var a = e;
    return a[dir].xyz * select(2.0, 1.0, dir == 5u);
}

// The sky's irradiance above the horizon about shading normal `n` (`sky_open_irradiance_in` of
// the directions `n` leans toward, mixed by `n_a^2`): a face's own direction's for an axis
// normal.
fn sky_open_by_normal_in(e: array<vec4<f32>, 6>, n: vec3<f32>) -> vec3<f32> {
    var s = vec3<f32>(0.0);
    var weight = 0.0;
    for (var a = 0u; a < 3u; a++) {
        let w = n[a] * n[a];
        s += sky_open_irradiance_in(e, 2u * a + select(0u, 1u, n[a] > 0.0)) * w;
        weight += w;
    }
    return s / max(weight, 1e-6);
}

// The pair's sky term at a lighting pixel (ADR-0070): the visibility pair's long mean `pair_x`
// times the sky's open irradiance about `n`, pre-exposed, where this frame's visibility sample
// `vis_a` says the sky ray was cast; 0 elsewhere and with no sky. The direct part does not
// carry it: the filter's input and the composite's unfiltered sum add it each frame, so the
// pair is the term's only history.
fn sky_pair_term(e: array<vec4<f32>, 6>, pair_x: f32, vis_a: f32, n: vec3<f32>) -> vec3<f32> {
    if (vis_a <= 0.5 || !sky_on()) {
        return vec3<f32>(0.0);
    }
    return pair_x * sky_open_by_normal_in(e, n) * exposure_now();
}

// World position `p` in voxels to metres above the planet's surface.
fn sky_altitude(p: vec3<f32>) -> f32 {
    return (p.z - sky.altitude_zero_z) * sky.metres_per_voxel + sky.base_altitude_m;
}

// The transmittance from altitude `alt_m` metres along zenith cosine `mu` to space.
fn transmittance(alt_m: f32, mu: f32) -> vec3<f32> {
    let r = sky.bottom_radius + max(alt_m, 0.0) / 1000.0;
    return transmittance_from(sky_transmittance_lut, r, mu);
}

// Linear radiance in frame units, not pre-exposed, from unit world direction `d`: the sky-view
// LUT's in-scattering (the bodies' discs not included); the uniform test sky's radiance above
// the horizon and 0 below; 0 with no sky.
fn sky_radiance(d: vec3<f32>) -> vec3<f32> {
    if (!sky_on()) {
        return vec3<f32>(0.0);
    }
    return sky_view_radiance(sky_view_lut, d);
}

// The sun's and the moon's discs as radiance from unit world direction `d`: inside a body's
// angular radius `E T / Omega`, T the transmittance from the camera's altitude toward it and
// Omega its disc's solid angle, with an edge one pixel of view 0 wide; 0 with no sky or the
// uniform test sky.
fn body_disc(d: vec3<f32>) -> vec3<f32> {
    if (!sky_on() || (sky.flags & SKY_UNIFORM) != 0u) {
        return vec3<f32>(0.0);
    }
    let v = params.views[0];
    let pix = (v.tan_up + v.tan_down) / f32(max(v.rect.w, 1u));
    let alt = sky.camera_alt * 1000.0;
    return disc_of(d, sky.sun_dir, sky.sun_e, sky.sun_radius, alt, pix)
        + disc_of(d, sky.moon_dir, sky.moon_e, sky.moon_radius, alt, pix);
}

// One body's disc ([`body_disc`]): the angle to its centre from `|d x l|`, which keeps its
// digits at the disc's size where `acos` of the dot does not.
fn disc_of(d: vec3<f32>, l: vec3<f32>, e: vec3<f32>, radius: f32, alt_m: f32, pix: f32) -> vec3<f32> {
    if (all(e <= vec3<f32>(0.0)) || dot(d, l) <= 0.0) {
        return vec3<f32>(0.0);
    }
    let a = asin(min(length(cross(d, l)), 1.0));
    let cover = clamp((radius - a) / max(pix, 1e-6) + 0.5, 0.0, 1.0);
    if (cover <= 0.0) {
        return vec3<f32>(0.0);
    }
    let h = sin(0.5 * radius);
    let omega = 4.0 * SKY_PI * h * h;
    return cover * e * transmittance(alt_m, l.z) / max(omega, 1e-12);
}

// The key light's unshadowed irradiance at a hit at `p` with normal `n` (ADR-0027): `E T(alt,
// l.z) max(n.l, 0)`, E its illuminance, T the transmittance from the hit's altitude toward its
// centre `l`; this frame's (`old` false) or the frame before's; 0 with no key light.
fn key_light_of(old: bool, p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let bit = select(SKY_KEY, SKY_PREV_KEY, old);
    if ((sky.flags & bit) == 0u) {
        return vec3<f32>(0.0);
    }
    let l = select(sky.key_dir, sky.prev_key_dir, old);
    let e = select(sky.key_e, sky.prev_key_e, old);
    return e * transmittance(sky_altitude(p), l.z) * max(dot(n, l), 0.0);
}

fn key_light_at(p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    return key_light_of(false, p, n);
}

// Whether the sky's light at a hit at `p`, normal `n`, changed past the upscalers' thresholds
// `change_abs` and `change_rel` between the frame before and this one: the key light's
// unshadowed luminance over pi (the list's `lights_term` units: irradiance over pi), or the
// sky's scale. A drift moves neither past them; a jump moves the key light's at every lit
// hit.
fn sky_light_changed(p: vec3<f32>, n: vec3<f32>, change_abs: f32, change_rel: f32) -> bool {
    if (!sky_on()) {
        return false;
    }
    let ds = abs(sky.sky_scale - sky.prev_sky_scale);
    if (ds > change_rel * max(sky.sky_scale, sky.prev_sky_scale)) {
        return true;
    }
    let now = luminance(key_light_of(false, p, n)) / PI;
    let then = luminance(key_light_of(true, p, n)) / PI;
    let d = abs(now - then);
    return d > change_abs && d > change_rel * max(now, then);
}
