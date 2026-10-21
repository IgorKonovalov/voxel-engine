// The light pass: direct irradiance at each pixel's hit, read from the hit buffer. Compiled
// after march.wgsl, whose bindings (group 0), walk and hit-record helpers it uses; the shadow
// ray is `march`.
//
// Bindings, group 2:
//   0  texture_2d<f32>   hit_pos_in      the hit pass's rgba32float position + t
//   1  texture_2d<u32>   hit_info_in     the hit pass's rgba16uint normal, material, flags
//   2  storage texture   irradiance_out  rgba16float: rgb the irradiance E at the hit (light
//                                        per unit area arriving over the hemisphere of the
//                                        shading normal), a unused; 0 on a miss
//
// The headlamp is a point light: E = PI * intensity * cone * max(n.l, 0) / dist^2, so that the
// composite's Lambertian `albedo * E / PI` is the march's former `albedo * lit`. One shadow ray
// per lit pixel, from SHADOW_OFFSET off the surface along the shadow normal toward the lamp;
// any rock before the lamp removes its light. In the cutaway view a hit takes the hemisphere
// term `PI (0.3 + 0.7 (0.5 + 0.5 n.z))` instead, with no lamp and no shadow ray. Only the
// shaded view reads this pass's output, so every other view skips it.

@group(2) @binding(0) var hit_pos_in: texture_2d<f32>;
@group(2) @binding(1) var hit_info_in: texture_2d<u32>;
@group(2) @binding(2) var irradiance_out: texture_storage_2d<rgba16float, write>;

// The largest finite f16.
const HALF_MAX: f32 = 65504.0;

@compute @workgroup_size(8, 8, 1)
fn shade(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.size.x || gid.y >= params.size.y || view_mode() != VIEW_SHADED) {
        return;
    }
    let px = vec2<i32>(gid.xy);
    let rec = unpack_hit(textureLoad(hit_pos_in, px, 0), textureLoad(hit_info_in, px, 0));
    var e = vec3<f32>(0.0);
    if (rec.found) {
        let ray = pixel_ray(gid.xy);
        let n = surface_detail(rec).xyz;
        if ((params.flags & FLAG_CUTAWAY) != 0u) {
            e = vec3<f32>(PI * (0.3 + 0.7 * (0.5 + 0.5 * n.z)));
        } else {
            e = vec3<f32>(PI * lamp_lit(rec, n, ray.d));
        }
    }
    textureStore(irradiance_out, px, vec4<f32>(min(e, vec3<f32>(HALF_MAX)), 0.0));
}

// The headlamp's `intensity * cone * max(n.l, 0) / dist^2` at hit `rec` with shading normal `n`,
// zero when the shadow ray finds rock before the lamp.
fn lamp_lit(rec: HitRecord, n: vec3<f32>, d: vec3<f32>) -> f32 {
    let p = rec.p;
    let to_light = params.light_pos - p;
    let dist2 = max(dot(to_light, to_light), 1e-4);
    let l = to_light * inverseSqrt(dist2);
    let ndl = max(dot(n, l), 0.0);
    let cone = smoothstep(params.cos_outer, params.cos_inner, dot(-l, params.light_dir));
    var lit = params.light_intensity * cone * ndl / dist2;
    if ((params.flags & FLAG_SHADOWS) != 0u && lit > 0.0) {
        let s = march(p + shadow_normal_of(rec, d) * SHADOW_OFFSET, l, sqrt(dist2), false);
        if (s.found) {
            lit = 0.0;
        }
    }
    return lit;
}
