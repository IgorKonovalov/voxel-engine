// The HUD overlay: a grid of 8 x 8 glyph cells at an integer scale, alpha-blended over
// whatever the target already holds. One fullscreen triangle; every decision is integer
// pixel math on the fragment's own pixel, so the result is exact on every adapter.
//
// Bindings, group 0:
//   0  uniform  params   scale, and the strip's sample count, ceiling and line
//   1  storage  cells    256 x 128 u32 cells, row-major with a fixed stride of 256:
//                        bits 0..8 the ASCII byte (below 32 is blank), bits 8..16 the colour
//   2  storage  font     95 glyphs x 8 row bytes packed little-endian into 190 u32s;
//                        bit 0 of a row byte is the glyph's leftmost pixel
//   3  storage  samples  up to 256 f32 strip samples, oldest first
//
// The draw's first instance carries the target's height in pixels (see `vs_main`).
//
// A set glyph bit is opaque in its cell's colour. Otherwise, inside the strip, a sample's
// bar is opaque white and the line row is opaque red where no bar covers it. Otherwise a
// pixel whose up-left neighbour (-1, -1) is a set glyph bit is the shadow: black at alpha
// 0.75. Everything else is transparent.
//
// The strip: sample i is column i, `scale` pixels wide, from the left edge. Its bottom row
// sits directly above the last text row of the target (8 scale pixels up from the bottom
// edge) and it is 64 scale pixels tall. A sample of `ceiling_ms` fills the height; larger
// ones clamp. The bar of sample s is round(min(s / ceiling_ms, 1) * 64 scale) pixels tall.

struct Params {
    scale: u32,
    sample_count: u32,
    ceiling_ms: f32,
    // Pixel rows above the strip's bottom row of the red line; NONE for no line.
    line_px: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> cells: array<u32>;
@group(0) @binding(2) var<storage, read> font: array<u32, 190>;
@group(0) @binding(3) var<storage, read> samples: array<f32, 256>;

const GRID_COLS: u32 = 256u;
const GRID_ROWS: u32 = 128u;
const FIRST: u32 = 32u;
const LAST: u32 = 126u;
const NONE: u32 = 0xffffffffu;
const SHADOW_ALPHA: f32 = 0.75;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) @interpolate(flat) height: u32,
}

// One triangle covering the screen: vertices 0, 1, 2 at (-1,-1), (3,-1), (-1,3). The
// instance index is the draw's first instance, which the encoder sets to the target height
// so the fragment stage can anchor things to the bottom edge without a uniform write.
@vertex
fn vs_main(@builtin(vertex_index) i: u32, @builtin(instance_index) height: u32) -> VsOut {
    let xy = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: VsOut;
    out.pos = vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
    out.height = height;
    return out;
}

// Row byte `row` of glyph `g`.
fn font_row(g: u32, row: u32) -> u32 {
    let i = g * 8u + row;
    return (font[i / 4u] >> ((i % 4u) * 8u)) & 0xffu;
}

// The colour index of the text pixel at `q`, or NONE where no glyph bit is set.
fn text_at(q: vec2<u32>) -> u32 {
    let cell_px = 8u * params.scale;
    let col = q.x / cell_px;
    let row = q.y / cell_px;
    if col >= GRID_COLS || row >= GRID_ROWS {
        return NONE;
    }
    let cell = cells[row * GRID_COLS + col];
    let code = cell & 0xffu;
    if code < FIRST || code > LAST {
        return NONE;
    }
    let gx = (q.x % cell_px) / params.scale;
    let gy = (q.y % cell_px) / params.scale;
    if ((font_row(code - FIRST, gy) >> gx) & 1u) == 0u {
        return NONE;
    }
    return (cell >> 8u) & 0xffu;
}

// The strip's colour at `q` on a target `height` pixels tall: 0 white (a bar), 2 red (the
// line), or NONE outside the strip or where neither is.
fn strip_at(q: vec2<u32>, height: u32) -> u32 {
    let s = params.scale;
    let strip_px = 64u * s;
    let bottom = 8u * s;
    if height < bottom + strip_px || q.x >= params.sample_count * s {
        return NONE;
    }
    let base = height - bottom; // the first pixel row below the strip
    if q.y >= base || q.y < base - strip_px {
        return NONE;
    }
    let up = base - 1u - q.y; // 0 on the strip's bottom row
    let v = clamp(samples[q.x / s] / params.ceiling_ms, 0.0, 1.0);
    let bar = u32(floor(v * f32(strip_px) + 0.5));
    if up < bar {
        return 0u;
    }
    if up == params.line_px {
        return 2u;
    }
    return NONE;
}

// 0 white, 1 yellow, 2 red; anything else white.
fn palette(c: u32) -> vec3<f32> {
    switch c {
        case 1u: { return vec3<f32>(1.0, 0.8, 0.0); }
        case 2u: { return vec3<f32>(1.0, 0.25, 0.25); }
        default: { return vec3<f32>(1.0, 1.0, 1.0); }
    }
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let q = vec2<u32>(in.pos.xy);
    let lit = text_at(q);
    if lit != NONE {
        return vec4<f32>(palette(lit), 1.0);
    }
    let bar = strip_at(q, in.height);
    if bar != NONE {
        return vec4<f32>(palette(bar), 1.0);
    }
    if q.x >= 1u && q.y >= 1u && text_at(q - vec2<u32>(1u, 1u)) != NONE {
        return vec4<f32>(0.0, 0.0, 0.0, SHADOW_ALPHA);
    }
    return vec4<f32>(0.0);
}
