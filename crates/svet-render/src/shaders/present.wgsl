// Blit of the marcher's rgba8 target to a surface, scaled to fill it.
//
// Bindings, group 0:
//   0  texture  source   the marcher's rgba8unorm target, display-encoded (gamma 1/2.2)
//   1  sampler  linear   bilinear, clamp to edge
//
// `fs_main` copies the display-encoded values to a non-sRGB surface; `fs_srgb` decodes them
// first with the exact sRGB curve, because an sRGB surface encodes on write with that curve:
// the pair round-trips every 8-bit value, so both surfaces show the target's bytes as stored.

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var linear: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// One triangle covering the screen: vertices 0, 1, 2 at (-1,-1), (3,-1), (-1,3).
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let xy = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: VsOut;
    out.pos = vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(xy.x, 1.0 - xy.y);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(textureSampleLevel(source, linear, in.uv, 0.0).rgb, 1.0);
}

// The sRGB decode (IEC 61966-2-1): linear below 0.04045, the 2.4 power curve above. The
// inverse of the encode an `*Srgb` format applies on write; `pow(c, 2.2)` is not, and moves
// dark values by several steps.
fn srgb_decode(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

@fragment
fn fs_srgb(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(source, linear, in.uv, 0.0).rgb;
    return vec4<f32>(srgb_decode(c), 1.0);
}
