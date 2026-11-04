// The gradient pass (A-SVGF, Schied 2018): how much the light changed since last frame, per
// 3 x 3 stratum of last frame's pixels; and the emissive table's world-space guard (ADR-0043).
// Compiled after march.wgsl and light.wgsl, whose bindings (group 0, and group 2's
// `cache_cells` and `cache_stats` at bindings 3 and 4), walk, lamp, shadow ray, candidate and
// kept-emitter helpers it uses; it runs after the table's edit-box reset and sweep and before
// this frame's light pass, while the irradiance texture still holds last frame's.
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
// The guard (`guard_cell`) takes the same pixel's cell: the cell's kept emitter is re-shaded
// from the face's centre (`kept_irradiance`, one shadow ray), and where it differs from the
// kept luminance by more than GUARD_DEAD_ZONE of the larger the cell reopens. A capped cell
// that keeps no emitter and whose sums are 0 (lit by nothing) draws one fresh emissive sample
// at the pixel's hit instead, and a non-zero one reopens it. A reopened cell's count, sums, kept key
// and luminance are cleared, and `cache_stats.reopens` counts it.
//
// Both lamp terms are compared pre-exposed (ADR-0033): the re-shade, computed in absolute
// units, times this frame's exposure, and last frame's estimate times `exposure_ratio()`; the
// 1e-4 floor acts on what the screen shows. The guard compares absolute irradiance.

@group(3) @binding(0) var grad_prev_pos: texture_2d<f32>;
@group(3) @binding(1) var grad_prev_info: texture_2d<u32>;
@group(3) @binding(2) var grad_prev_light: texture_2d<f32>;
@group(3) @binding(3) var strata_out: texture_storage_2d<rgba16float, write>;

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
            if (cacheable(rec)) {
                guard_cell(q, rec, ns);
            }
        }
    }
    textureStore(strata_out, vec2<i32>(gid.xy), out);
}

// The guard's dead zone: a kept emitter's re-shade within this share of the larger of it and
// the kept luminance leaves the cell alone (ADR-0036's LAMBDA_FLOOR). The packed luminance's
// truncation to seven mantissa bits is under 0.008 of it.
const GUARD_DEAD_ZONE: f32 = 0.05;

// Clears the cell at `word`'s count, sums, kept key and luminance (its key and stamp stay),
// counting the reopen when the cell held a sample.
fn reopen_cell(word: u32) {
    if (atomicExchange(&cache_cells[word + CACHE_COUNT], 0u) == 0u) {
        return;
    }
    for (var k = CACHE_COUNT + 1u; k <= CACHE_KEPT_LUM; k++) {
        atomicStore(&cache_cells[word + k], 0u);
    }
    atomicAdd(&cache_stats.reopens, 1u);
}

// The guard at last frame's hit `rec` of lighting pixel `q` (shadow normal `ns`).
fn guard_cell(q: vec2<u32>, rec: HitRecord, ns: vec3<f32>) {
    let v = cache_voxel(rec.p, ns);
    let face = cache_face(rec.axis, ns);
    let slot = cache_find(v, face);
    if (slot < 0) {
        return;
    }
    let word = CACHE_WORDS * u32(slot);
    let count = atomicLoad(&cache_cells[word + CACHE_COUNT]);
    if (count == 0u) {
        return;
    }
    let packed = atomicLoad(&cache_cells[word + CACHE_KEPT_LUM]);
    let k = vec2<u32>(atomicLoad(&cache_cells[word + CACHE_KEPT]), atomicLoad(&cache_cells[word + CACHE_KEPT + 1u]));
    let e_old = kept_lum(packed);
    if (e_old > 0.0 && k.y != 0u) {
        let e_new = kept_irradiance(v, face, cache_key_voxel(k), (k.y >> 28u) & 7u, -1.0);
        if ((packed & 0xFFFFu) != kept_tag(k)) {
            // A torn store: the key is another winner's or a mix of two. Its value now is kept
            // with it; a key that lights nothing reopens the cell.
            if (e_new > 0.0) {
                atomicStore(&cache_cells[word + CACHE_KEPT_LUM], kept_pack(e_new, k));
            } else {
                reopen_cell(word);
            }
            return;
        }
        if (abs(e_new - e_old) > GUARD_DEAD_ZONE * max(e_new, e_old)) {
            reopen_cell(word);
        }
        return;
    }
    let sums = atomicLoad(&cache_cells[word + CACHE_COUNT + 1u]) | atomicLoad(&cache_cells[word + CACHE_COUNT + 2u]) | atomicLoad(&cache_cells[word + CACHE_COUNT + 3u]);
    // An uncapped cell still draws its pixels' samples in the light pass.
    if (sums != 0u || count < cache_c_cap()) {
        return;
    }
    // Lit by nothing: one fresh sample, its random numbers apart from the light pass's.
    let e = emissive_estimate(q + vec2<u32>(37u, 91u), rec.p, rec.n, ns);
    if (luminance(e) > 0.0) {
        reopen_cell(word);
    }
}
