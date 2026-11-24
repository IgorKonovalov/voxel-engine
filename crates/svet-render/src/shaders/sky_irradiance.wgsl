// The sky's irradiance about each axis (ADR-0070's `E_sky`): for each of the six directions in
// face order (2 k toward -k, 2 k + 1 toward +k), `pi` times the cosine-weighted mean of the
// sky's radiance over the hemisphere about it, a direction below the horizon counting 0, as a
// sky ray scores it. A stratified quadrature: IRR_SIDE^2 strata of equal cosine-weighted
// measure (a uniform grid in the disc's area and angle, lifted onto the hemisphere), the
// radiance read at each stratum's centre from the sky-view LUT (`sky_view_radiance`). Run
// after the sky-view pass every frame a sky is set; one workgroup a direction. Compiled after
// the prelude and `sky_atmosphere.wgsl`. `+z`'s w is the key light's irradiance on a
// horizontal face at the sky's base altitude in luminance (Rec. 709), `E T(base, l.z) l.z`, 0
// with no key light: the exposure pass's anchored ceiling reads it; every other w is 0.
//
// Bindings, group 0:
//   0  uniform          sky          the sky uniform
//   1  texture_2d<f32>  sky_view_in  the sky-view LUT
//   2  storage          sky_e_out    six vec4: `E_sky` in xyz, w as above
//   3  texture_2d<f32>  sky_trans_in the transmittance LUT

@group(0) @binding(0) var<uniform> sky: SkyUniform;
@group(0) @binding(1) var sky_view_in: texture_2d<f32>;
@group(0) @binding(2) var<storage, read_write> sky_e_out: array<vec4<f32>, 6>;
@group(0) @binding(3) var sky_trans_in: texture_2d<f32>;

// Strata a side: IRR_SIDE^2 directions a hemisphere.
const IRR_SIDE: u32 = 32u;

var<workgroup> irr_sum: array<vec3<f32>, 64>;

// The centre of stratum (`i`, `j`) of the hemisphere about unit `n` (`i` along the disc's
// squared radius, `j` its angle), density `cos / pi`.
fn stratum_direction(n: vec3<f32>, i: u32, j: u32) -> vec3<f32> {
    let u = (f32(i) + 0.5) / f32(IRR_SIDE);
    let r = sqrt(u);
    let phi = 2.0 * SKY_PI * (f32(j) + 0.5) / f32(IRR_SIDE);
    let a = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(n.x) > 0.9);
    let t = normalize(cross(a, n));
    let b = cross(n, t);
    return normalize(t * (r * cos(phi)) + b * (r * sin(phi)) + n * sqrt(max(1.0 - u, 0.0)));
}

@compute @workgroup_size(64, 1, 1)
fn sky_irradiance(
    @builtin(workgroup_id) wid: vec3<u32>,
    @builtin(local_invocation_index) lane: u32,
) {
    let f = wid.x;
    let axis = f / 2u;
    let s = select(-1.0, 1.0, (f & 1u) == 1u);
    let n = vec3<f32>(
        select(0.0, s, axis == 0u),
        select(0.0, s, axis == 1u),
        select(0.0, s, axis == 2u),
    );
    var sum = vec3<f32>(0.0);
    for (var k = lane; k < IRR_SIDE * IRR_SIDE; k += 64u) {
        let d = stratum_direction(n, k % IRR_SIDE, k / IRR_SIDE);
        if (d.z > 0.0) {
            sum += sky_view_radiance(sky_view_in, d);
        }
    }
    irr_sum[lane] = sum;
    for (var stride = 32u; stride > 0u; stride >>= 1u) {
        workgroupBarrier();
        if (lane < stride) {
            irr_sum[lane] += irr_sum[lane + stride];
        }
    }
    if (lane == 0u) {
        var key = 0.0;
        if (f == 5u && (sky.flags & SKY_KEY) != 0u && sky.key_dir.z > 0.0) {
            let r = sky.bottom_radius + max(sky.base_altitude_m, 0.0) / 1000.0;
            let e = sky.key_e * transmittance_from(sky_trans_in, r, sky.key_dir.z) * sky.key_dir.z;
            key = dot(e, vec3<f32>(0.2126, 0.7152, 0.0722));
        }
        sky_e_out[f] = vec4<f32>(SKY_PI * irr_sum[0] / f32(IRR_SIDE * IRR_SIDE), key);
    }
}
