// The sky-view LUT (ADR-0026, Hillaire 2020 section 5.3): per texel a direction from the
// camera's altitude, by absolute azimuth (x: 0 toward +x east, increasing toward +y north) and
// a horizon-weighted elevation (y: `sky_view_zenith`), and the radiance scattered toward the
// camera along it, the sun's and the moon's summed, times `sky.view_scale`: single scattering,
// and with SKY_MULTISCATTER the multiple-scattering LUT's term. A midpoint march of
// 30 steps to the ground or the top of the atmosphere, each step's in-scattering integrated
// against its extinction. Run every frame a sky is set. Compiled after the prelude and
// `sky_atmosphere.wgsl`.
//
// Bindings, group 0:
//   0  uniform          sky                the sky uniform
//   1  texture_2d<f32>  transmittance_lut  the transmittance LUT
//   2  storage texture  sky_view_out       rgba16float, vec2<u32>(SKY_VIEW_W, SKY_VIEW_H)

@group(0) @binding(0) var<uniform> sky: SkyUniform;
@group(0) @binding(1) var transmittance_lut: texture_2d<f32>;
@group(0) @binding(2) var multiscatter_lut: texture_2d<f32>;
@group(0) @binding(3) var sky_view_out: texture_storage_2d<rgba32float, write>;

const SKY_VIEW_STEPS: u32 = 30u;

// The light a sample at `p` (km from the planet's centre, radius `rs`) scatters toward `-d`
// from the body toward `l` of illuminance `e`, per unit length, before extinction.
fn body_scatter(p: vec3<f32>, rs: f32, d: vec3<f32>, l: vec3<f32>, e: vec3<f32>, m: Medium) -> vec3<f32> {
    if (all(e <= vec3<f32>(0.0))) {
        return vec3<f32>(0.0);
    }
    let mu_l = dot(p / rs, l);
    let t = transmittance_from(transmittance_lut, rs, mu_l);
    let c = dot(d, l);
    var s = t * (m.rayleigh * rayleigh_phase(c) + m.mie * mie_phase(c, sky.mie_g));
    if ((sky.flags & SKY_MULTISCATTER) != 0u) {
        let ms_uv = vec2<f32>(
            unit_to_uv(clamp(mu_l * 0.5 + 0.5, 0.0, 1.0), f32(SKY_MULTISCATTER_W)),
            unit_to_uv(
                clamp((rs - sky.bottom_radius) / (sky.top_radius - sky.bottom_radius), 0.0, 1.0),
                f32(SKY_MULTISCATTER_H),
            ),
        );
        s += (m.rayleigh + m.mie) * lut_tap(multiscatter_lut, ms_uv, false).xyz;
    }
    return e * s;
}

// The scaled radiance reaching the camera at radius `r` along `d`.
fn sky_march(r: f32, d: vec3<f32>) -> vec3<f32> {
    let mu = d.z;
    var len = ground_distance(r, mu);
    if (len < 0.0) {
        len = max(sphere_far(r, mu, sky.top_radius), 0.0);
    }
    let dt = len / f32(SKY_VIEW_STEPS);
    let o = vec3<f32>(0.0, 0.0, r);
    var lum = vec3<f32>(0.0);
    var tv = vec3<f32>(1.0);
    for (var i = 0u; i < SKY_VIEW_STEPS; i++) {
        let p = o + d * ((f32(i) + 0.5) * dt);
        let rs = length(p);
        let m = medium_at(rs - sky.bottom_radius);
        let s = (body_scatter(p, rs, d, sky.sun_dir, sky.sun_e, m)
            + body_scatter(p, rs, d, sky.moon_dir, sky.moon_e, m)) * sky.view_scale;
        let ext = max(m.extinction, vec3<f32>(1e-9));
        lum += tv * s * one_minus_exp(m.extinction * dt) / ext;
        tv *= exp(-m.extinction * dt);
    }
    return lum;
}

@compute @workgroup_size(8, 8, 1)
fn sky_view(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (any(gid.xy >= vec2<u32>(SKY_VIEW_W, SKY_VIEW_H))) {
        return;
    }
    let uv = (vec2<f32>(gid.xy) + 0.5) / vec2<f32>(vec2<u32>(SKY_VIEW_W, SKY_VIEW_H));
    let r = sky_camera_r();
    let az = uv.x * 2.0 * SKY_PI;
    let zen = sky_view_zenith(uv_to_unit(uv.y, f32(vec2<u32>(SKY_VIEW_W, SKY_VIEW_H).y)), r);
    let d = vec3<f32>(sin(zen) * cos(az), sin(zen) * sin(az), cos(zen));
    textureStore(sky_view_out, vec2<i32>(gid.xy), vec4<f32>(sky_march(r, d), 1.0));
}
