// The FSR 2 tone pass (ours, ADR-0034): FSR 2's output, linear radiance at the output size,
// exposed, tone mapped and display-encoded into the frame's rgba8 image, one invocation per
// output pixel. The same arithmetic as composite.wgsl's `display`, applied after the upscaler
// instead of before it. Compiled after march.wgsl, whose parameters (`params.exposure`, the
// tone-map flag, the view and `params.output_size`) it uses.
//
// Bindings, group 2:
//   0  texture_2d<f32>   tone_in    FSR 2's output (rgb), or with the upscaler bypassed the
//                                   composite's linear colour (equal sizes)
//   1  storage texture   tone_out   rgba8unorm: the frame's image
//
// Every view but the shaded one reaches the upscaler already in display values and is stored
// as it comes, clamped to [0, 1].

@group(2) @binding(0) var tone_in: texture_2d<f32>;
@group(2) @binding(1) var tone_out: texture_storage_2d<rgba8unorm, write>;

// composite.wgsl's `aces`.
fn tone_aces(x: vec3<f32>) -> vec3<f32> {
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

// composite.wgsl's `display`.
fn tone_display(c: vec3<f32>) -> vec3<f32> {
    let x = c * params.exposure;
    var m = clamp(x, vec3<f32>(0.0), vec3<f32>(1.0));
    if ((params.lighting & LIGHTING_IDENTITY_TONE_MAP) == 0u) {
        m = tone_aces(max(x, vec3<f32>(0.0)));
    }
    return pow(m, vec3<f32>(1.0 / 2.2));
}

@compute @workgroup_size(8, 8, 1)
fn tone(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.output_size.x || gid.y >= params.output_size.y) {
        return;
    }
    let px = vec2<i32>(gid.xy);
    let c = textureLoad(tone_in, px, 0).xyz;
    var out = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    if (view_mode() == VIEW_SHADED) {
        out = tone_display(c);
    }
    textureStore(tone_out, px, vec4<f32>(out, 1.0));
}
