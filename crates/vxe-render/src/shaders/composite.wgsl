// The composite pass: turns the hit buffer, the translucent layers and the irradiance into
// the `rgba8unorm` target the presenter draws. Compiled after common.wgsl, hitrec.wgsl,
// sky_atmosphere.wgsl and sky.wgsl, whose bindings (group 0: the material rows, the sky),
// hit-record helpers and sky lookups it uses.
//
// Bindings, group 2:
//   0  texture_2d<f32>   hit_pos_in      the hit pass's rgba32float position + t
//   1  texture_2d<u32>   hit_info_in     the hit pass's rgba16uint normal, material, flags
//   2  texture_2d<f32>   layer_in        the hit pass's rgba16float translucent solid's
//                                        reflected light before exposure in rgb (ADR-0047), or
//                                        in the steps view its walk counters, in the cracks
//                                        view the field walk's leak
//   3  texture_2d<f32>   irradiance_in   the irradiance at each hit, rgb: the filter's
//                                        output, or with the filter off the temporal pass's
//                                        accumulated emissive part
//   4  storage texture   target_tex      rgba8unorm output
//   5  texture_2d<f32>   light_pos_in    the lighting grid's hit positions (the hit buffer at
//                                        full resolution)
//   6  texture_2d<u32>   light_info_in   the lighting grid's hit records
//   7  texture_2d<f32>   indirect_in     the probe gather's indirect irradiance on the
//                                        lighting grid, rgb; read with Params.lighting bit 12
//   8  storage, rw       meter_tiles     the exposure meter (exposure.rs): per workgroup, at
//                                        `workgroup_id.y * workgroups.x + workgroup_id.x`, the
//                                        sum over its pixels of `clamp(log2(max(Y, 2^-16)),
//                                        -16, 16)` of the shaded radiance before exposure and
//                                        the tone map, and their count; written in the shaded
//                                        view only
//   9  texture_2d<u32>   transmit_in     the hit pass's r32uint transmittance per channel,
//                                        rgb9e5 (`t9e5_unpack`)
//  10  texture_2d<f32>   direct_acc_in   the temporal pass's accumulated direct part (rgb),
//                                        read with the filter off
//  11  texture_2d<f32>   vis_in          this frame's visibility samples (1 in a where the sky
//                                        ray was cast), read with the filter off
//  12  texture_2d<f32>   pair_in         the visibility pair the temporal pass wrote this frame,
//                                        read with the filter off
//  13  storage, read     sky_e           `E_sky` per direction, read with the filter off
//  14  storage, read     marks           the entity mirror's marks (ADR-0074): two words per
//                                        instance slot, its owner and its mark
//                                        (`vxe_render::Mark::words`)
//
// Per view: `shaded` is the Lambertian `albedo * tint * E / PI` of a hit (E the direct
// irradiance plus, with Params.lighting bit 12, the probes' indirect irradiance) plus its
// material's
// emission under the frame's scale (none in the cutaway view; the cutaway's flat
// sky on a miss in that view, else `sky_radiance` plus `body_disc` of the pixel's ray with a sky set, else black), filtered by the translucent solid in front of it,
// `reflected * exposure + T * c` (ADR-0047), all
// pre-exposed (ADR-0033: the irradiance arrives pre-exposed, and the absolute terms, emission,
// the layers, the sky and the probes' indirect irradiance, are multiplied by `exposure_now()`
// where they join it), tone mapped (`Lighting::tone_map`: ACES or the identity clamp) and
// display-encoded by `pow(c, 1 / 2.2)`; `normals` writes a hit's shading normal as
// `n * 0.5 + 0.5`, linear, and a miss as `shaded` does; `hitmask` white on a hit;
// `steps` the counters over 255; `albedo` the hit's albedo through the solid's `T`, its
// reflected share at unit light added;
// `cracks` the field walk's leaks over the shaded image without its layers (`crack_colour`).
// In the shaded and albedo views a marked entity's pixels mix toward its mark's colour by
// its tint, and its outline replaces a pixel with the colour (`mark_at`).

// The cracks view's colour of a pixel whose shaded display colour is `shaded`, from the hit
// pass's leak in `layer.x`: a miss black; a leak red; any other hit `4/255 + 0.47 shaded` per
// channel (4 to 124 of 255: never black, never a channel at 255). Each colour decodes back
// (`vxe_render::CrackShares::from_rgba8`).
fn crack_colour(found: bool, shaded: vec3<f32>, layer: vec4<f32>) -> vec3<f32> {
    if (!found) {
        return vec3<f32>(0.0);
    }
    if (layer.x > 0.5) {
        return vec3<f32>(1.0, 0.0, 0.0);
    }
    return vec3<f32>(4.0 / 255.0) + 0.47 * clamp(shaded, vec3<f32>(0.0), vec3<f32>(1.0));
}

@group(2) @binding(0) var hit_pos_in: texture_2d<f32>;
@group(2) @binding(1) var hit_info_in: texture_2d<u32>;
@group(2) @binding(2) var layer_in: texture_2d<f32>;
@group(2) @binding(3) var irradiance_in: texture_2d<f32>;
@group(2) @binding(4) var target_tex: texture_storage_2d<rgba8unorm, write>;
// The shaded view stores linear radiance before exposure instead of `display` of it: FSR 2's
// variant of this pass (frame/upscale.rs) sets this true and declares `target_tex` rgba32float.
const HDR_TARGET: bool = false;
@group(2) @binding(5) var light_pos_in: texture_2d<f32>;
@group(2) @binding(6) var light_info_in: texture_2d<u32>;
@group(2) @binding(7) var indirect_in: texture_2d<f32>;
@group(2) @binding(8) var<storage, read_write> meter_tiles: array<vec2<f32>>;
@group(2) @binding(9) var transmit_in: texture_2d<u32>;
@group(2) @binding(10) var direct_acc_in: texture_2d<f32>;
@group(2) @binding(11) var vis_in: texture_2d<f32>;
@group(2) @binding(12) var pair_in: texture_2d<f32>;
@group(2) @binding(13) var<storage, read> sky_e: array<vec4<f32>, 6>;
@group(2) @binding(14) var<storage, read> marks: array<u32>;

// A slot's owner word (`vxe_render::Mark::words`): the owner's first slot in bits 0..8 and
// MARK_LIVE, so `& MARK_OWNER` is 0 for no entity and differs between two entities.
const MARK_LIVE: u32 = 512u;
const MARK_OWNER: u32 = 1023u;
const MARK_OUTLINE: u32 = 1024u;
const MARK_TINT_SHIFT: u32 = 11u;
const MARK_SLOT_MASK: u32 = 511u;

// The workgroup's meter partial sums: log2 luminance, pixel count.
var<workgroup> meter_sum: array<vec2<f32>, 64>;

// The prelude's LIGHTING_INDIRECT, a `Params::lighting` bit: the probe gather ran and its
// irradiance adds to the direct one.

// The irradiance at lighting texel `q`: the filter's rgb, or with the filter off the temporal
// pass's accumulation, its emissive rgb plus the accumulated direct rgb plus the pair's sky
// term where the ray was cast (`sky_pair_term`); with the indirect flag, the gathered indirect
// rgb added.
fn irradiance_at(q: vec2<i32>) -> vec3<f32> {
    let v = textureLoad(irradiance_in, q, 0);
    var e = v.xyz;
    if ((params.lighting & LIGHTING_FILTER_OFF) != 0u) {
        e += textureLoad(direct_acc_in, q, 0).xyz;
        let rec = unpack_hit(textureLoad(light_pos_in, q, 0), textureLoad(light_info_in, q, 0));
        e += sky_pair_term(sky_e, textureLoad(pair_in, q, 0).x, textureLoad(vis_in, q, 0).a, rec.n);
    }
    if ((params.lighting & LIGHTING_INDIRECT) != 0u) {
        e += textureLoad(indirect_in, q, 0).xyz * exposure_now();
    }
    return e;
}

// The irradiance at full-resolution pixel `px` (hit `rec`): the lighting grid's own texel at
// full resolution; at half resolution the four lighting pixels around it, bilinear weights
// times `max(n.n', 0)^32 exp(-plane distance)` against the full-resolution hit, or the
// nearest hit tap when every weight vanishes. The normals are `filter_normal`'s (the shading
// normal, or with Params.lighting bit 15 a cube face's face normal); the plane distance is
// along the shading normal.
fn lit_at(px: vec2<i32>, rec: HitRecord) -> vec3<f32> {
    if (params.light_scale == 1u) {
        return irradiance_at(px);
    }
    let o = vec2<f32>(unpack_offset(params.light_offset));
    let c = (vec2<f32>(px) - o) / f32(params.light_scale);
    let base = vec2<i32>(floor(c));
    let f = c - floor(c);
    var sum = vec3<f32>(0.0);
    var w_sum = 0.0;
    var nearest = vec3<f32>(0.0);
    var nearest_w = -1.0;
    let origin = params.views[view_of(vec2<u32>(px))].origin;
    let n = filter_normal(rec, origin);
    for (var i = 0; i < 4; i++) {
        let d = vec2<i32>(i & 1, i >> 1);
        let tap = base + d;
        if (!on_light_grid(tap)) {
            continue;
        }
        let other = unpack_hit(textureLoad(light_pos_in, tap, 0), textureLoad(light_info_in, tap, 0));
        if (!other.found) {
            continue;
        }
        let bil = select(1.0 - f.x, f.x, d.x == 1) * select(1.0 - f.y, f.y, d.y == 1);
        var cosn = max(dot(n, filter_normal(other, origin)), 0.0);
        cosn *= cosn;
        cosn *= cosn;
        cosn *= cosn;
        cosn *= cosn;
        cosn *= cosn;
        let w = bil * cosn * exp(-abs(dot(rec.n, other.p - rec.p)));
        let e = irradiance_at(tap);
        sum += w * e;
        w_sum += w;
        if (bil > nearest_w) {
            nearest_w = bil;
            nearest = e;
        }
    }
    if (w_sum > 1e-6) {
        return sum / w_sum;
    }
    return nearest;
}

// Bindings, group 3 of `downsample`, which fills the lighting grid's hit buffer at half
// resolution from the full one: lighting pixel `lp` takes full pixel `full_px(lp)`.
@group(3) @binding(0) var down_pos_in: texture_2d<f32>;
@group(3) @binding(1) var down_info_in: texture_2d<u32>;
@group(3) @binding(2) var down_pos_out: texture_storage_2d<rgba32float, write>;
@group(3) @binding(3) var down_info_out: texture_storage_2d<rgba16uint, write>;

@compute @workgroup_size(8, 8, 1)
fn downsample(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.light_size.x || gid.y >= params.light_size.y) {
        return;
    }
    let src = vec2<i32>(min(full_px(gid.xy), params.size - vec2<u32>(1u)));
    textureStore(down_pos_out, vec2<i32>(gid.xy), textureLoad(down_pos_in, src, 0));
    textureStore(down_info_out, vec2<i32>(gid.xy), textureLoad(down_info_in, src, 0));
}

// The ACES filmic curve fitted by Narkowicz (2015), per channel, clamped to [0, 1].
fn aces(x: vec3<f32>) -> vec3<f32> {
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

// The pre-exposed radiance `x` tone mapped and display-encoded.
fn display(x: vec3<f32>) -> vec3<f32> {
    var m = clamp(x, vec3<f32>(0.0), vec3<f32>(1.0));
    if ((params.lighting & LIGHTING_IDENTITY_TONE_MAP) == 0u) {
        m = aces(max(x, vec3<f32>(0.0)));
    }
    return pow(m, vec3<f32>(1.0 / 2.2));
}

@compute @workgroup_size(8, 8, 1)
fn composite(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(local_invocation_index) lane: u32,
    @builtin(workgroup_id) wid: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    var m = vec2<f32>(0.0);
    if (gid.x < params.size.x && gid.y < params.size.y) {
        m = composite_px(vec2<i32>(gid.xy));
    }
    meter_sum[lane] = m;
    workgroupBarrier();
    for (var stride = 32u; stride > 0u; stride >>= 1u) {
        if (lane < stride) {
            meter_sum[lane] += meter_sum[lane + stride];
        }
        workgroupBarrier();
    }
    if (lane == 0u && (view_mode() == VIEW_SHADED || view_mode() == VIEW_CRACKS)) {
        meter_tiles[wid.y * groups.x + wid.x] = meter_sum[0];
    }
}

// The meter's term of radiance `c` before exposure: its clamped log2 luminance and a count.
fn meter_of(c: vec3<f32>) -> vec2<f32> {
    return vec2<f32>(clamp(log2(max(luminance(c), exp2(-16.0))), -16.0, 16.0), 1.0);
}

// The two mark words of the entity hit at `px`, zeros for any other hit or a miss.
fn mark_words_at(px: vec2<i32>) -> vec2<u32> {
    let f = textureLoad(hit_info_in, px, 0).w;
    if ((f & (HIT_FOUND | HIT_ENTITY)) != (HIT_FOUND | HIT_ENTITY)) {
        return vec2<u32>(0u);
    }
    let s = 2u * ((f >> HIT_INSTANCE_SHIFT) & MARK_SLOT_MASK);
    return vec2<u32>(marks[s], marks[s + 1u]);
}

fn mark_colour(w: vec2<u32>) -> vec3<f32> {
    return vec3<f32>(vec3<u32>(w.y, w.y >> 8u, w.y >> 16u) & vec3<u32>(255u)) / 255.0;
}

// Pixel `px`'s mark, ADR-0074: the colour and how much of it. A pixel whose owner differs
// from a neighbour's (an entity against the world or another entity) takes the outline's
// colour of the marked side, its own side first; else a marked entity's pixel its tint.
fn mark_at(px: vec2<i32>) -> vec4<f32> {
    let w = mark_words_at(px);
    let own = w.x & MARK_OWNER;
    let size = vec2<i32>(params.size);
    for (var i = 0; i < 4; i++) {
        let q = px + select(vec2<i32>(0, 1 - 2 * (i & 1)), vec2<i32>(1 - 2 * (i & 1), 0), i < 2);
        if (any(q < vec2<i32>(0)) || any(q >= size)) {
            continue;
        }
        let n = mark_words_at(q);
        if ((n.x & MARK_OWNER) == own) {
            continue;
        }
        if ((w.x & MARK_OUTLINE) != 0u) {
            return vec4<f32>(mark_colour(w), 1.0);
        }
        if ((n.x & MARK_OUTLINE) != 0u) {
            return vec4<f32>(mark_colour(n), 1.0);
        }
    }
    return vec4<f32>(mark_colour(w), f32((w.x >> MARK_TINT_SHIFT) & 255u) / 255.0);
}

// The pre-exposed radiance `display` takes to display colour `d` (in [0, 1)): the encode
// undone, then the ACES curve's inverse (the root of its quadratic) unless the tone map is the
// identity. FSR 2's variant mixes a mark in radiance, before its tone pass.
fn undisplay(d: vec3<f32>) -> vec3<f32> {
    let m = pow(clamp(d, vec3<f32>(0.0), vec3<f32>(0.999)), vec3<f32>(2.2));
    if ((params.lighting & LIGHTING_IDENTITY_TONE_MAP) != 0u) {
        return m;
    }
    let a = 2.51 - 2.43 * m;
    let b = 0.03 - 0.59 * m;
    return (-b + sqrt(b * b + 4.0 * a * 0.14 * m)) / (2.0 * a);
}

// Writes pixel `px`'s colour; returns its meter term in the shaded view, else zero.
fn composite_px(px: vec2<i32>) -> vec2<f32> {
    let rec = unpack_hit(textureLoad(hit_pos_in, px, 0), textureLoad(hit_info_in, px, 0));
    let layer = textureLoad(layer_in, px, 0);
    let view = view_mode();
    if (view == VIEW_STEPS) {
        textureStore(target_tex, px, vec4<f32>(layer.xyz / 255.0, 1.0));
        return vec2<f32>(0.0);
    }
    if (view == VIEW_HITMASK) {
        textureStore(target_tex, px, vec4<f32>(vec3<f32>(select(0.0, 1.0, rec.found)), 1.0));
        return vec2<f32>(0.0);
    }
    if (view == VIEW_ALBEDO) {
        var a = vec3<f32>(0.0);
        if (rec.found) {
            a = materials[rec.material * 4u].xyz;
        }
        if (rec.translucent) {
            a = layer.xyz + t9e5_unpack(textureLoad(transmit_in, px, 0).x) * a;
        }
        let mk = mark_at(px);
        textureStore(target_tex, px, vec4<f32>(mix(a, mk.xyz, mk.w), 1.0));
        return vec2<f32>(0.0);
    }
    let ex = exposure_now();
    var c = vec3<f32>(0.0);
    if (!rec.found && (params.flags & FLAG_CUTAWAY) != 0u) {
        c = CUTAWAY_SKY * ex;
    } else if (!rec.found && sky_on()) {
        let d = pixel_ray(vec2<u32>(px)).d;
        c = (sky_radiance(d) + body_disc(d)) * ex;
    }
    if (rec.found) {
        if (view == VIEW_NORMALS) {
            textureStore(target_tex, px, vec4<f32>(rec.n * 0.5 + 0.5, 1.0));
            return vec2<f32>(0.0);
        }
        let albedo = materials[rec.material * 4u].xyz;
        c = albedo * lit_at(px, rec) * (1.0 / PI);
        if ((params.flags & FLAG_CUTAWAY) == 0u) {
            let e = emission_of(rec.material);
            c += e * ex;
        }
    }
    if (view == VIEW_CRACKS) {
        // The palette in display values, as the normals view writes its colours: the
        // upscalers copy a view other than shaded and FSR 2's tone pass stores it as it comes,
        // so the leaks reach the window unchanged.
        let k = crack_colour(rec.found, display(c), layer);
        textureStore(target_tex, px, vec4<f32>(k, 1.0));
        return meter_of(c / ex);
    }
    if (rec.translucent) {
        c = layer.xyz * ex + t9e5_unpack(textureLoad(transmit_in, px, 0).x) * c;
    }
    let meter = meter_of(c / ex);
    let mk = mark_at(px);
    if (HDR_TARGET) {
        c = mix(c, undisplay(mk.xyz), mk.w);
    } else {
        c = mix(display(c), mk.xyz, mk.w);
    }
    textureStore(target_tex, px, vec4<f32>(c, 1.0));
    return meter;
}
