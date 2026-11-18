// The multiple-scattering LUT (ADR-0026, Hillaire 2020 section 5.5): per texel an altitude
// (y, bottom to top) and a light zenith cosine (x, -1 to 1), and the paper's isotropic
// infinite-order term `psi = L_2nd / (1 - f_ms)` per unit illuminance: from a point there, 64
// directions over the sphere (a Fibonacci lattice), each a midpoint march of 20 steps to the
// ground or the top; `L_2nd` the mean in-scattered light under an isotropic phase (the ground's
// Lambertian bounce added where a ray meets it) and `f_ms` the mean scattered fraction. The
// sky-view pass multiplies it by the scattering coefficient and the body's illuminance. Run
// when the atmosphere changes, after the transmittance LUT. Compiled after the prelude and
// `sky_atmosphere.wgsl`.
//
// Bindings, group 0:
//   0  uniform          sky                the sky uniform
//   1  texture_2d<f32>  transmittance_lut  the transmittance LUT
//   2  storage texture  multiscatter_out   rgba16float, SKY_MULTISCATTER_W x SKY_MULTISCATTER_H

@group(0) @binding(0) var<uniform> sky: SkyUniform;
@group(0) @binding(1) var transmittance_lut: texture_2d<f32>;
@group(0) @binding(2) var multiscatter_out: texture_storage_2d<rgba16float, write>;

const MS_DIRECTIONS: u32 = 64u;
const MS_STEPS: u32 = 20u;

@compute @workgroup_size(8, 8, 1)
fn sky_multiscatter(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<u32>(SKY_MULTISCATTER_W, SKY_MULTISCATTER_H);
    if (any(gid.xy >= size)) {
        return;
    }
    let uv = (vec2<f32>(gid.xy) + 0.5) / vec2<f32>(size);
    let mu_s = uv_to_unit(uv.x, f32(size.x)) * 2.0 - 1.0;
    let r = sky.bottom_radius + uv_to_unit(uv.y, f32(size.y)) * (sky.top_radius - sky.bottom_radius);
    let l = vec3<f32>(sqrt(max(1.0 - mu_s * mu_s, 0.0)), 0.0, mu_s);
    let o = vec3<f32>(0.0, 0.0, r);
    let iso = 1.0 / (4.0 * SKY_PI);
    var l2 = vec3<f32>(0.0);
    var fms = vec3<f32>(0.0);
    let golden = SKY_PI * (3.0 - sqrt(5.0));
    for (var k = 0u; k < MS_DIRECTIONS; k++) {
        let z = 1.0 - (f32(k) + 0.5) * 2.0 / f32(MS_DIRECTIONS);
        let rad = sqrt(max(1.0 - z * z, 0.0));
        let phi = golden * f32(k);
        let d = vec3<f32>(rad * cos(phi), rad * sin(phi), z);
        let g = ground_distance(r, d.z);
        var len = g;
        if (len < 0.0) {
            len = max(sphere_far(r, d.z, sky.top_radius), 0.0);
        }
        let dt = len / f32(MS_STEPS);
        var tv = vec3<f32>(1.0);
        for (var i = 0u; i < MS_STEPS; i++) {
            let p = o + d * ((f32(i) + 0.5) * dt);
            let rs = length(p);
            let m = medium_at(rs - sky.bottom_radius);
            let scat = m.rayleigh + m.mie;
            let ts = transmittance_from(transmittance_lut, rs, dot(p / rs, l));
            let step_t = exp(-m.extinction * dt);
            let ext = max(m.extinction, vec3<f32>(1e-9));
            let w = tv * one_minus_exp(m.extinction * dt) / ext;
            l2 += w * scat * ts * iso;
            fms += w * scat;
            tv *= step_t;
        }
        if (g >= 0.0) {
            let p = o + d * g;
            let up = normalize(p);
            let mu_g = dot(up, l);
            let ts = transmittance_from(transmittance_lut, sky.bottom_radius + 1e-4, mu_g);
            l2 += tv * ts * max(mu_g, 0.0) * sky.ground_albedo / SKY_PI;
        }
    }
    l2 /= f32(MS_DIRECTIONS);
    // f_ms is the integral against the isotropic phase: the lattice's mean is 4 pi / N times
    // the sum, times 1 / (4 pi).
    fms /= f32(MS_DIRECTIONS);
    let psi = l2 / max(vec3<f32>(1.0) - fms, vec3<f32>(1e-3));
    textureStore(multiscatter_out, vec2<i32>(gid.xy), vec4<f32>(psi, 1.0));
}
