// The gradient pass (A-SVGF, Schied 2018): how much the light changed since last frame, per
// 3 x 3 stratum of last frame's pixels. Compiled after march.wgsl and light.wgsl, whose
// bindings (groups 0 and 2; group 2 is unused here), walk, lamp, shadow ray and reservoir
// helpers it uses; it runs before this frame's light pass, while the irradiance texture still
// holds last frame's.
//
// Bindings, group 3:
//   0  texture_2d<f32>   grad_prev_pos     last frame's hit positions
//   1  texture_2d<u32>   grad_prev_info    last frame's hit records
//   2  texture_2d<f32>   grad_prev_light   last frame's irradiance from the light pass
//   3  texture_2d<u32>   grad_prev_res     last frame's reservoirs
//   4  storage texture   strata_out        rgba16float, one texel per stratum: the gradient
//                                          lambda = |new - old| / max(new, old), then the new
//                                          and old luminances
//
// One pixel per stratum, picked by a hash of the stratum and the frame, is re-shaded at last
// frame's hit with this frame's lights and last frame's sample: the lamp at its current pose
// with its shadow ray, and the reservoir's kept emitter point with its current radiance and
// visibility under last frame's W. Nothing changed, the re-shade equals last frame's estimate
// (`target * W = w_sum / M`), so lambda is 0 but for f16 rounding. A pixel with no hit, or no
// valid history, reads 0.

@group(3) @binding(0) var grad_prev_pos: texture_2d<f32>;
@group(3) @binding(1) var grad_prev_info: texture_2d<u32>;
@group(3) @binding(2) var grad_prev_light: texture_2d<f32>;
@group(3) @binding(3) var grad_prev_res: texture_2d<u32>;
@group(3) @binding(4) var strata_out: texture_storage_2d<rgba16float, write>;

// Strata edge in pixels; must match `GRADIENT_STRATUM` in `marcher.rs` and temporal.wgsl.
const STRATUM: u32 = 3u;

@compute @workgroup_size(8, 8, 1)
fn gradient(@builtin(global_invocation_id) gid: vec3<u32>) {
    let strata = (params.light_size + vec2<u32>(STRATUM - 1u)) / STRATUM;
    if (gid.x >= strata.x || gid.y >= strata.y) {
        return;
    }
    var out = vec4<f32>(0.0);
    let h = hash_u32(gid.x * 73856093u ^ gid.y * 19349663u ^ params.frame * 83492791u);
    let q = gid.xy * STRATUM + vec2<u32>(h % STRATUM, (h / STRATUM) % STRATUM);
    if ((params.history & 1u) != 0u && all(q < params.light_size)) {
        let qi = vec2<i32>(q);
        let rec = unpack_hit(textureLoad(grad_prev_pos, qi, 0), textureLoad(grad_prev_info, qi, 0));
        if (rec.found && (params.flags & FLAG_CUTAWAY) == 0u) {
            let v = params.prev_views[view_of(q * params.light_scale + unpack_offset(params.prev_light_offset))];
            let d = normalize(rec.p - v.origin);
            let n = lighting_normal(rec);
            let ns = shadow_normal_of(rec, d);
            var e = 0.0;
            if ((params.lighting & LIGHTING_LAMP) != 0u) {
                e = PI * lamp_lit(rec.p, n, ns);
            }
            let stored = textureLoad(grad_prev_res, qi, 0);
            if ((stored.w & 0x8000000u) != 0u) {
                let x = bitcast<vec3<f32>>(stored.xyz);
                let w_big = bitcast<f32>((stored.w & 0xFFFFu) << 16u);
                let face = (stored.w >> 24u) & 7u;
                let c = lookup(vec3<i32>(floor(x - 0.5 * face_dir(face))));
                if (c.size == 0) {
                    let le = emission_of(c.material);
                    let t = target_at(rec.p, n, x, face, le);
                    if (t > 0.0) {
                        let o = rec.p + ns * SHADOW_OFFSET;
                        let to = x - o;
                        let dist = length(to);
                        if (!shadowed(o, to / dist, dist - 1e-3)) {
                            e += min(t * w_big, FIREFLY_CLAMP * PI * luminance(le));
                        }
                    }
                }
            }
            let prev_light = textureLoad(grad_prev_light, qi, 0);
            let old = luminance(prev_light.xyz) + prev_light.w;
            let lambda = abs(e - old) / max(max(e, old), 1e-4);
            out = vec4<f32>(lambda, e, old, 0.0);
        }
    }
    textureStore(strata_out, vec2<i32>(gid.xy), out);
}
