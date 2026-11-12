// The gradient pass (A-SVGF, Schied 2018): how much the lamp's light changed since last frame,
// per 3 x 3 stratum of last frame's pixels. Compiled after the walk's modules and light.wgsl, whose
// bindings (group 0), walk, lamp and shadow ray it uses; it runs before this frame's light pass,
// while the irradiance texture still holds last frame's.
//
// Bindings, group 3:
//   0  texture_2d<f32>   grad_prev_pos     last frame's hit positions
//   1  texture_2d<u32>   grad_prev_info    last frame's hit records
//   2  texture_2d<f32>   grad_prev_light   last frame's irradiance from the light pass
//   3  storage texture   strata_out        rgba16float, one texel per stratum: the gradient
//                                          lambda = |new - old| / max(new, old) of the lamp's
//                                          light, then its new and old luminances, then 0
//
// One pixel per stratum, picked by a hash of the stratum and the frame, is re-shaded at last
// frame's hit with this frame's lamp at its current pose, with its shadow ray (with
// `Params.lighting`'s gradient bit only: the temporal pass reads it in that mode alone).
// Nothing changed, the re-shade equals last frame's lamp estimate (the light pass's irradiance
// in a), so lambda is 0 but for f16 rounding. A pixel with no hit, or no valid history, reads 0.
//
// Both lamp terms are compared pre-exposed (ADR-0033): the re-shade, computed in absolute
// units, times this frame's exposure, and last frame's estimate times `exposure_ratio()`; the
// 1e-4 floor acts on what the screen shows.

@group(3) @binding(0) var grad_prev_pos: texture_2d<f32>;
@group(3) @binding(1) var grad_prev_info: texture_2d<u32>;
@group(3) @binding(2) var grad_prev_light: texture_2d<f32>;
@group(3) @binding(3) var strata_out: texture_storage_2d<rgba16float, write>;

// Strata edge in pixels; must match `GRADIENT_STRATUM` in `frame/settings.rs` and temporal.wgsl.
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
            let n = rec.n;
            let ns = shadow_normal_of(rec, d);
            if ((params.lighting & LIGHTING_GRADIENT) != 0u) {
                var lamp = 0.0;
                if ((params.lighting & LIGHTING_LAMP) != 0u) {
                    lamp = PI * lamp_lit(rec.p, n, ns);
                }
                let prev_light = textureLoad(grad_prev_light, qi, 0);
                let e = lamp * exposure_now();
                let old = prev_light.w * exposure_ratio();
                let lambda = abs(e - old) / max(max(e, old), 1e-4);
                out = vec4<f32>(lambda, e, old, 0.0);
            }
        }
    }
    textureStore(strata_out, vec2<i32>(gid.xy), out);
}
