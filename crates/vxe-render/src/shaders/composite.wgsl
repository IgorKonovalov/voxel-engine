// The composite pass: turns the hit buffer, the translucent layers and the irradiance into
// the `rgba8unorm` target the presenter draws. Compiled after march.wgsl, whose bindings
// (group 0: the material rows), hit-record helpers and detail it uses.
//
// Bindings, group 2:
//   0  texture_2d<f32>   hit_pos_in      the hit pass's rgba32float position + t
//   1  texture_2d<u32>   hit_info_in     the hit pass's rgba16uint normal, material, flags
//   2  texture_2d<f32>   layer_in        the hit pass's rgba16float translucent layers
//                                        (premultiplied colour, transmittance), or in the
//                                        steps view its walk counters
//   3  texture_2d<f32>   irradiance_in   the irradiance at each hit, rgb
//   4  storage texture   target_tex      rgba8unorm output
//
// Per view: `shaded` is the Lambertian `albedo * tint * E / PI` of a hit (the cutaway's flat
// sky on a miss in that view, else black), the translucent layers composited over it, times
// `params.exposure`, tone mapped (`Lighting::tone_map`: ACES or the identity clamp) and
// display-encoded by `pow(c, 1 / 2.2)`; `normals` writes a hit's shading normal as
// `n * 0.5 + 0.5`, linear, and a miss as `shaded` does; `hitmask` white on a hit;
// `steps` the counters over 255; `albedo` the hit's albedo with the unlit layers over it.

@group(2) @binding(0) var hit_pos_in: texture_2d<f32>;
@group(2) @binding(1) var hit_info_in: texture_2d<u32>;
@group(2) @binding(2) var layer_in: texture_2d<f32>;
@group(2) @binding(3) var irradiance_in: texture_2d<f32>;
@group(2) @binding(4) var target_tex: texture_storage_2d<rgba8unorm, write>;

// The ACES filmic curve fitted by Narkowicz (2015), per channel, clamped to [0, 1].
fn aces(x: vec3<f32>) -> vec3<f32> {
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn display(c: vec3<f32>) -> vec3<f32> {
    let x = c * params.exposure;
    var m = clamp(x, vec3<f32>(0.0), vec3<f32>(1.0));
    if ((params.lighting & LIGHTING_IDENTITY_TONE_MAP) == 0u) {
        m = aces(max(x, vec3<f32>(0.0)));
    }
    return pow(m, vec3<f32>(1.0 / 2.2));
}

@compute @workgroup_size(8, 8, 1)
fn composite(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.size.x || gid.y >= params.size.y) {
        return;
    }
    let px = vec2<i32>(gid.xy);
    let rec = unpack_hit(textureLoad(hit_pos_in, px, 0), textureLoad(hit_info_in, px, 0));
    let layer = textureLoad(layer_in, px, 0);
    let view = view_mode();
    if (view == VIEW_STEPS) {
        textureStore(target_tex, px, vec4<f32>(layer.xyz / 255.0, 1.0));
        return;
    }
    if (view == VIEW_HITMASK) {
        textureStore(target_tex, px, vec4<f32>(vec3<f32>(select(0.0, 1.0, rec.found)), 1.0));
        return;
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
        return;
    }
    var c = vec3<f32>(0.0);
    if (!rec.found && (params.flags & FLAG_CUTAWAY) != 0u) {
        c = CUTAWAY_SKY;
    }
    if (rec.found) {
        let st = surface_detail(rec);
        if (view == VIEW_NORMALS) {
            textureStore(target_tex, px, vec4<f32>(st.xyz * 0.5 + 0.5, 1.0));
            return;
        }
        let albedo = materials[rec.material * 4u].xyz * st.w;
        c = albedo * textureLoad(irradiance_in, px, 0).xyz * (1.0 / PI);
    }
    if (rec.translucent) {
        c = layer.xyz + layer.w * c;
    }
    textureStore(target_tex, px, vec4<f32>(display(c), 1.0));
}
