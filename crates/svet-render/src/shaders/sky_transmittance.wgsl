// The transmittance LUT (ADR-0026): per texel, a radius and a zenith cosine
// (`transmittance_r_mu`), and the transmittance from there to the top of the atmosphere,
// `exp(-optical depth)`, the depth a midpoint sum of 40 steps. Run when the atmosphere
// changes. Compiled after the prelude and `sky_atmosphere.wgsl`.
//
// Bindings, group 0:
//   0  uniform          sky                the sky uniform
//   1  storage texture  transmittance_out  rgba16float, vec2<u32>(SKY_TRANSMITTANCE_W, SKY_TRANSMITTANCE_H)

@group(0) @binding(0) var<uniform> sky: SkyUniform;
@group(0) @binding(1) var transmittance_out: texture_storage_2d<rgba16float, write>;

const TRANSMITTANCE_STEPS: u32 = 40u;

@compute @workgroup_size(8, 8, 1)
fn sky_transmittance(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (any(gid.xy >= vec2<u32>(SKY_TRANSMITTANCE_W, SKY_TRANSMITTANCE_H))) {
        return;
    }
    let uv = (vec2<f32>(gid.xy) + 0.5) / vec2<f32>(vec2<u32>(SKY_TRANSMITTANCE_W, SKY_TRANSMITTANCE_H));
    let rm = transmittance_r_mu(uv);
    let r = rm.x;
    let mu = rm.y;
    let len = max(sphere_far(r, mu, sky.top_radius), 0.0);
    let dt = len / f32(TRANSMITTANCE_STEPS);
    var depth = vec3<f32>(0.0);
    for (var i = 0u; i < TRANSMITTANCE_STEPS; i++) {
        let t = (f32(i) + 0.5) * dt;
        let ri = sqrt(r * r + t * t + 2.0 * r * mu * t);
        depth += medium_at(ri - sky.bottom_radius).extinction * dt;
    }
    textureStore(transmittance_out, vec2<i32>(gid.xy), vec4<f32>(exp(-depth), 1.0));
}
