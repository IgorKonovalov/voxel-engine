// The composite pass: turns the hit buffer, the translucent layers and the irradiance into
// the `rgba8unorm` target the presenter draws. Compiled after march.wgsl, whose bindings
// (group 0: the material rows), hit-record helpers and detail it uses.
//
// Bindings, group 2:
//   0  texture_2d<f32>   hit_pos_in      the hit pass's rgba32float position + t
//   1  texture_2d<u32>   hit_info_in     the hit pass's rgba16uint normal, material, flags
//   2  texture_2d<f32>   layer_in        the hit pass's rgba16float translucent layers
//                                        (premultiplied colour, transmittance), or in the
//                                        steps view its walk counters, in the cracks view
//                                        the crack class and the self-hit
//   3  texture_2d<f32>   irradiance_in   the irradiance at each hit, rgb: the temporal
//                                        pass's accumulation
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
//
// Per view: `shaded` is the Lambertian `albedo * tint * E / PI` of a hit (E the direct
// irradiance plus, with Params.lighting bit 12, the probes' indirect irradiance; the direct
// part times the detail's lamp ratio when the detail is applied after lighting, march.wgsl's
// `detail_lamp_ratio`, while the indirect part carries the detail from the gather's own
// normal) plus its material's
// emission under the frame's scale (none in the cutaway view; the cutaway's flat
// sky on a miss in that view, else black), the translucent layers composited over it, all
// pre-exposed (ADR-0033: the irradiance arrives pre-exposed, and the absolute terms, emission,
// the layers, the sky and the probes' indirect irradiance, are multiplied by `exposure_now()`
// where they join it), tone mapped (`Lighting::tone_map`: ACES or the identity clamp) and
// display-encoded by `pow(c, 1 / 2.2)`; `normals` writes a hit's shading normal as
// `n * 0.5 + 0.5`, linear, and a miss as `shaded` does; `hitmask` white on a hit;
// `steps` the counters over 255; `albedo` the hit's albedo with the unlit layers over it;
// `cracks` the hit's crack class (march.wgsl's `crack_class`) over the shaded image without
// its layers (`crack_colour`).

// The cracks view's colour of a pixel whose shaded display colour is `shaded`, from the hit
// pass's class in `layer.x` and self-hit in `layer.y`: a miss black; a class 0 hit
// `4/255 + 0.47 shaded` per channel (4 to 124 of 255: never black, never a channel at 255);
// class 1 red, 2 yellow, 3 blue; a self-hit magenta over class 0, and over classes 1 to 3 its
// class's colour with the channel that is 0 at 128 (128 for blue's red). Each colour decodes
// back to its class and self-hit (`vxe_render::CrackShares::from_rgba8`).
fn crack_colour(found: bool, shaded: vec3<f32>, layer: vec4<f32>) -> vec3<f32> {
    if (!found) {
        return vec3<f32>(0.0);
    }
    let cls = u32(layer.x + 0.5);
    let self_hit = layer.y > 0.5;
    let mark = select(0.0, 128.0 / 255.0, self_hit);
    switch cls {
        case 1u: { return vec3<f32>(1.0, 0.0, mark); }
        case 2u: { return vec3<f32>(1.0, 1.0, mark); }
        case 3u: { return vec3<f32>(mark, 0.0, 1.0); }
        default: {
            if (self_hit) {
                return vec3<f32>(1.0, 0.0, 1.0);
            }
            return vec3<f32>(4.0 / 255.0) + 0.47 * clamp(shaded, vec3<f32>(0.0), vec3<f32>(1.0));
        }
    }
}

@group(2) @binding(0) var hit_pos_in: texture_2d<f32>;
@group(2) @binding(1) var hit_info_in: texture_2d<u32>;
@group(2) @binding(2) var layer_in: texture_2d<f32>;
@group(2) @binding(3) var irradiance_in: texture_2d<f32>;
@group(2) @binding(4) var target_tex: texture_storage_2d<rgba8unorm, write>;
// The shaded view stores linear radiance before exposure instead of `display` of it: FSR 2's
// variant of this pass (marcher.rs) sets this true and declares `target_tex` rgba32float.
const HDR_TARGET: bool = false;
@group(2) @binding(5) var light_pos_in: texture_2d<f32>;
@group(2) @binding(6) var light_info_in: texture_2d<u32>;
@group(2) @binding(7) var indirect_in: texture_2d<f32>;
@group(2) @binding(8) var<storage, read_write> meter_tiles: array<vec2<f32>>;

// The workgroup's meter partial sums: log2 luminance, pixel count.
var<workgroup> meter_sum: array<vec2<f32>, 64>;

// `Params::lighting` bit: the probe gather ran and its irradiance adds to the direct one.
const LIGHTING_INDIRECT: u32 = 4096u;

// The irradiance of a lighting pixel or of its upsampled value, split: `direct` the
// filter's rgb, or with the filter off the light pass's emissive rgb plus its lamp's white a;
// `indirect` the gathered indirect rgb when it ran, else zero; `all` their sum, accumulated
// as one value so it is exactly the unsplit irradiance.
struct Lit {
    all: vec3<f32>,
    direct: vec3<f32>,
    indirect: vec3<f32>,
}

fn irradiance_at(q: vec2<i32>) -> Lit {
    let v = textureLoad(irradiance_in, q, 0);
    var d = v.xyz;
    if ((params.lighting & LIGHTING_FILTER_OFF) != 0u) {
        d += vec3<f32>(v.w);
    }
    var e = d;
    var ind = vec3<f32>(0.0);
    if ((params.lighting & LIGHTING_INDIRECT) != 0u) {
        ind = textureLoad(indirect_in, q, 0).xyz * exposure_now();
        e += ind;
    }
    return Lit(e, d, ind);
}

// The irradiance at full-resolution pixel `px` (hit `rec`): the lighting grid's own texel at
// full resolution; at half resolution the four lighting pixels around it, bilinear weights
// times `max(n.n', 0)^32 exp(-plane distance)` against the full-resolution hit, or the
// nearest hit tap when every weight vanishes. Each part of [`Lit`] takes the same weights.
fn lit_at(px: vec2<i32>, rec: HitRecord) -> Lit {
    if (params.light_scale == 1u) {
        return irradiance_at(px);
    }
    let o = vec2<f32>(unpack_offset(params.light_offset));
    let c = (vec2<f32>(px) - o) / f32(params.light_scale);
    let base = vec2<i32>(floor(c));
    let f = c - floor(c);
    var sum = vec3<f32>(0.0);
    var sum_d = vec3<f32>(0.0);
    var sum_i = vec3<f32>(0.0);
    var w_sum = 0.0;
    var nearest = Lit(vec3<f32>(0.0), vec3<f32>(0.0), vec3<f32>(0.0));
    var nearest_w = -1.0;
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
        var cosn = max(dot(rec.n, other.n), 0.0);
        cosn *= cosn;
        cosn *= cosn;
        cosn *= cosn;
        cosn *= cosn;
        cosn *= cosn;
        let w = bil * cosn * exp(-abs(dot(rec.n, other.p - rec.p)));
        let e = irradiance_at(tap);
        sum += w * e.all;
        sum_d += w * e.direct;
        sum_i += w * e.indirect;
        w_sum += w;
        if (bil > nearest_w) {
            nearest_w = bil;
            nearest = e;
        }
    }
    if (w_sum > 1e-6) {
        return Lit(sum / w_sum, sum_d / w_sum, sum_i / w_sum);
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
            a = layer.xyz + layer.w * a;
        }
        textureStore(target_tex, px, vec4<f32>(a, 1.0));
        return vec2<f32>(0.0);
    }
    let ex = exposure_now();
    var c = vec3<f32>(0.0);
    if (!rec.found && (params.flags & FLAG_CUTAWAY) != 0u) {
        c = CUTAWAY_SKY * ex;
    }
    if (rec.found) {
        let st = surface_detail(rec);
        if (view == VIEW_NORMALS) {
            textureStore(target_tex, px, vec4<f32>(st.xyz * 0.5 + 0.5, 1.0));
            return vec2<f32>(0.0);
        }
        let albedo = materials[rec.material * 4u].xyz * st.w;
        let lit = lit_at(px, rec);
        var e = lit.all;
        if (detail_after_lighting()) {
            // The ratio is the lamp's term; the indirect part was gathered with the detailed
            // normal already, so it is added unscaled.
            e = lit.direct * detail_lamp_ratio(rec, st.xyz) + lit.indirect;
        }
        c = albedo * e * (1.0 / PI);
        if ((params.flags & FLAG_CUTAWAY) == 0u) {
            c += emission_of(rec.material) * ex;
        }
    }
    if (view == VIEW_CRACKS) {
        // The palette in display values, as the normals view writes its colours: the
        // upscalers copy a view other than shaded and FSR 2's tone pass stores it as it comes,
        // so the classes reach the window unchanged.
        let k = crack_colour(rec.found, display(c), layer);
        textureStore(target_tex, px, vec4<f32>(k, 1.0));
        return meter_of(c / ex);
    }
    if (rec.translucent) {
        c = layer.xyz * ex + layer.w * c;
    }
    textureStore(target_tex, px, vec4<f32>(select(display(c), c, HDR_TARGET), 1.0));
    return meter_of(c / ex);
}
