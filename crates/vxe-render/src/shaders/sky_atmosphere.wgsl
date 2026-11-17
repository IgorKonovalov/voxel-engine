// The atmosphere's model (ADR-0026, Hillaire 2020), shared by the LUT passes and by
// `sky.wgsl`: the sky uniform, the media's densities and coefficients, the ray-sphere
// distances and the LUTs' parameterisations, and a bilinear tap of an rgba16float LUT. Every
// distance is in kilometres from the planet's centre. A source that joins this module
// declares the uniform `sky: SkyUniform` at its own binding.

struct SkyUniform {
    rayleigh_scattering: vec3<f32>,
    bottom_radius: f32,
    mie_scattering: vec3<f32>,
    top_radius: f32,
    mie_extinction: vec3<f32>,
    rayleigh_scale_height: f32,
    ozone_absorption: vec3<f32>,
    mie_scale_height: f32,
    ground_albedo: vec3<f32>,
    mie_g: f32,
    sun_dir: vec3<f32>,
    sun_radius: f32,
    sun_e: vec3<f32>,
    flags: u32,
    moon_dir: vec3<f32>,
    moon_radius: f32,
    moon_e: vec3<f32>,
    // The sky-view LUT holds radiance times this.
    view_scale: f32,
    uniform_l: vec3<f32>,
    altitude_zero_z: f32,
    metres_per_voxel: f32,
    base_altitude_m: f32,
    // The camera's altitude above the surface, km.
    camera_alt: f32,
    // The sky rays a lighting pixel casts (sky_light.wgsl).
    sky_rays: u32,
    // The key light (SKY_KEY): direction, angular radius, illuminance.
    key_dir: vec3<f32>,
    key_radius: f32,
    key_e: vec3<f32>,
    // The visibility pair's running mean's cap in frames (temporal.wgsl).
    sky_history: u32,
    // The frame before's key light (SKY_PREV_KEY), and the sky's scale now and then.
    prev_key_dir: vec3<f32>,
    sky_scale: f32,
    prev_key_e: vec3<f32>,
    prev_sky_scale: f32,
};

const SKY_PI: f32 = 3.14159265358979;

// The prelude's SKY_ON, SKY_UNIFORM and SKY_MULTISCATTER are the uniform's flags, and
// SKY_TRANSMITTANCE_W/H and SKY_VIEW_W/H the LUTs' sizes.

// Scattering and extinction at altitude `h` km.
struct Medium {
    rayleigh: vec3<f32>,
    mie: vec3<f32>,
    extinction: vec3<f32>,
};

fn medium_at(h: f32) -> Medium {
    let dr = exp(-max(h, 0.0) / sky.rayleigh_scale_height);
    let dm = exp(-max(h, 0.0) / sky.mie_scale_height);
    // Ozone's tent: 0 at 10 and 40 km, 1 at 25 km.
    let doz = max(0.0, 1.0 - abs(h - 25.0) / 15.0);
    var m: Medium;
    m.rayleigh = sky.rayleigh_scattering * dr;
    m.mie = sky.mie_scattering * dm;
    m.extinction = m.rayleigh + sky.mie_extinction * dm + sky.ozone_absorption * doz;
    return m;
}

// The distance along a ray from radius `r` at zenith cosine `mu` to the sphere of radius
// `radius` (the far root), or -1 when it misses.
fn sphere_far(r: f32, mu: f32, radius: f32) -> f32 {
    let disc = r * r * (mu * mu - 1.0) + radius * radius;
    if (disc < 0.0) {
        return -1.0;
    }
    return -r * mu + sqrt(disc);
}

// The near root against the planet: the distance to the ground, or -1 when the ray misses it.
fn ground_distance(r: f32, mu: f32) -> f32 {
    let b = sky.bottom_radius;
    let disc = r * r * (mu * mu - 1.0) + b * b;
    if (disc < 0.0 || mu > 0.0) {
        return -1.0;
    }
    return max(-r * mu - sqrt(disc), 0.0);
}

// A unit range onto texel centres of `n` texels and back (Bruneton 2017): x = 0 is texel 0's
// centre and x = 1 the last's.
fn unit_to_uv(x: f32, n: f32) -> f32 {
    return 0.5 / n + x * (1.0 - 1.0 / n);
}

fn uv_to_unit(u: f32, n: f32) -> f32 {
    return (u - 0.5 / n) / (1.0 - 1.0 / n);
}

// The transmittance LUT's uv of radius `r` and zenith cosine `mu` (Bruneton 2017, section 4).
fn transmittance_uv(r: f32, mu: f32) -> vec2<f32> {
    let b = sky.bottom_radius;
    let t = sky.top_radius;
    let h = sqrt(t * t - b * b);
    let rho = sqrt(max(r * r - b * b, 0.0));
    let d = max(sphere_far(r, mu, t), 0.0);
    let d_min = t - r;
    let d_max = rho + h;
    let x_mu = (d - d_min) / max(d_max - d_min, 1e-6);
    let x_r = rho / h;
    return vec2<f32>(
        unit_to_uv(clamp(x_mu, 0.0, 1.0), f32(SKY_TRANSMITTANCE_W)),
        unit_to_uv(clamp(x_r, 0.0, 1.0), f32(SKY_TRANSMITTANCE_H)),
    );
}

// The radius and zenith cosine of transmittance LUT coordinate `uv`.
fn transmittance_r_mu(uv: vec2<f32>) -> vec2<f32> {
    let b = sky.bottom_radius;
    let t = sky.top_radius;
    let x_mu = uv_to_unit(uv.x, f32(SKY_TRANSMITTANCE_W));
    let x_r = uv_to_unit(uv.y, f32(SKY_TRANSMITTANCE_H));
    let h = sqrt(t * t - b * b);
    let rho = h * x_r;
    let r = sqrt(rho * rho + b * b);
    let d_min = t - r;
    let d_max = rho + h;
    let d = d_min + x_mu * (d_max - d_min);
    var mu = 1.0;
    if (d > 0.0) {
        mu = clamp((h * h - rho * rho - d * d) / (2.0 * r * d), -1.0, 1.0);
    }
    return vec2<f32>(r, mu);
}

// A bilinear tap of `lut` at `uv` in [0, 1]^2, clamped at the edges; with `wrap_x` the x axis
// wraps (the sky-view LUT's azimuth).
fn lut_tap(lut: texture_2d<f32>, uv: vec2<f32>, wrap_x: bool) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(lut));
    let p = uv * vec2<f32>(size) - 0.5;
    let base = vec2<i32>(floor(p));
    let f = p - floor(p);
    var out = vec4<f32>(0.0);
    for (var i = 0; i < 4; i++) {
        let d = vec2<i32>(i & 1, i >> 1);
        var q = base + d;
        if (wrap_x) {
            q.x = (q.x % size.x + size.x) % size.x;
        } else {
            q.x = clamp(q.x, 0, size.x - 1);
        }
        q.y = clamp(q.y, 0, size.y - 1);
        let w = select(1.0 - f.x, f.x, d.x == 1) * select(1.0 - f.y, f.y, d.y == 1);
        out += w * textureLoad(lut, q, 0);
    }
    return out;
}

// The transmittance from radius `r` along zenith cosine `mu` to the top of the atmosphere,
// read from `lut`; 0 where the ray meets the ground.
fn transmittance_from(lut: texture_2d<f32>, r: f32, mu: f32) -> vec3<f32> {
    if (ground_distance(r, mu) >= 0.0) {
        return vec3<f32>(0.0);
    }
    return lut_tap(lut, transmittance_uv(r, mu), false).xyz;
}

// The sky-view LUT's horizon-weighted elevation mapping (Hillaire 2020, section 5.3): the
// zenith angle of row coordinate `v` for a camera at radius `r`, and back.
fn sky_view_zenith(v: f32, r: f32) -> f32 {
    let b = sky.bottom_radius;
    let cos_beta = sqrt(max(r * r - b * b, 0.0)) / r;
    let beta = acos(clamp(cos_beta, -1.0, 1.0));
    let zha = SKY_PI - beta;
    if (v < 0.5) {
        var c = 1.0 - 2.0 * v;
        c = 1.0 - c * c;
        return zha * c;
    }
    let c = 2.0 * v - 1.0;
    return zha + beta * c * c;
}

fn sky_view_v(zenith: f32, r: f32) -> f32 {
    let b = sky.bottom_radius;
    let cos_beta = sqrt(max(r * r - b * b, 0.0)) / r;
    let beta = acos(clamp(cos_beta, -1.0, 1.0));
    let zha = SKY_PI - beta;
    if (zenith < zha) {
        let c = 1.0 - sqrt(max(1.0 - zenith / zha, 0.0));
        return 0.5 * c;
    }
    let c = sqrt(clamp((zenith - zha) / max(beta, 1e-6), 0.0, 1.0));
    return 0.5 + 0.5 * c;
}

// The camera's radius for the sky-view LUT: at least 10 m above the surface, so the horizon's
// row has a band under it.
fn sky_camera_r() -> f32 {
    return sky.bottom_radius + max(sky.camera_alt, 0.01);
}

// Rayleigh's phase function.
fn rayleigh_phase(c: f32) -> f32 {
    return 3.0 / (16.0 * SKY_PI) * (1.0 + c * c);
}

// The Cornette-Shanks Mie phase function of asymmetry `g`.
fn mie_phase(c: f32, g: f32) -> f32 {
    let g2 = g * g;
    let k = 3.0 / (8.0 * SKY_PI) * (1.0 - g2) / (2.0 + g2);
    return k * (1.0 + c * c) / pow(max(1.0 + g2 - 2.0 * g * c, 1e-6), 1.5);
}

// `1 - exp(-x)` per channel for x >= 0, by its series under 0.01, where the subtraction would
// cancel most of f32's digits (a step of a ray that meets the ground a few metres away).
fn one_minus_exp(x: vec3<f32>) -> vec3<f32> {
    let series = x * (1.0 - x * (0.5 - x / 6.0));
    return select(1.0 - exp(-x), series, x < vec3<f32>(0.01));
}

// `sky.wgsl`'s `sky_radiance` with a sky set, over the sky-view LUT `lut`: the uniform test
// sky's radiance above the horizon and 0 below, else the LUT's radiance from unit world
// direction `d` over `sky.view_scale`.
fn sky_view_radiance(lut: texture_2d<f32>, d: vec3<f32>) -> vec3<f32> {
    if ((sky.flags & SKY_UNIFORM) != 0u) {
        return select(vec3<f32>(0.0), sky.uniform_l, d.z > 0.0);
    }
    let r = sky_camera_r();
    let zen = acos(clamp(d.z, -1.0, 1.0));
    var az = atan2(d.y, d.x);
    if (az < 0.0) {
        az += 2.0 * SKY_PI;
    }
    let uv = vec2<f32>(
        az / (2.0 * SKY_PI),
        unit_to_uv(sky_view_v(zen, r), f32(vec2<u32>(SKY_VIEW_W, SKY_VIEW_H).y)),
    );
    return lut_tap(lut, uv, true).xyz / sky.view_scale;
}
