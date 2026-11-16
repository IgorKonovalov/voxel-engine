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
    return lut_tap(sky_view_lut, uv, true).xyz / sky.view_scale;
}
