// The loading frame (ADR-0059): one fullscreen triangle whose fragment draws the background,
// a 3 x 3 x 3 block of voxels with gaps between them turning about a tilted axis, ray-traced
// against the boxes with flat per-face shading, and a bordered progress bar. Every silhouette,
// face edge and bar edge takes a one-pixel coverage ramp, so nothing crawls as the cube turns.
// The output is premultiplied by `alpha` for the fade over the first world frames.
//
// Bindings (group 0):
//   0: uniform `Params`: the target size, the time, the progress, the alpha, the bar's rect.

struct Params {
    size: vec2<f32>,
    seconds: f32,
    progress: f32,
    // The bar's outer rect in target pixels: min x, min y, max x, max y.
    bar: vec4<f32>,
    alpha: f32,
    // Target pixels per border pixel.
    border: f32,
    // The cube's centre in target pixels and its half-extent on screen.
    cube_px: f32,
    cube_y: f32,
};

@group(0) @binding(0) var<uniform> params: Params;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: VsOut;
    out.pos = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    return out;
}

const BG_TOP: vec3<f32> = vec3<f32>(0.035, 0.038, 0.050);
const BG_BOTTOM: vec3<f32> = vec3<f32>(0.012, 0.012, 0.018);
const CUBE: vec3<f32> = vec3<f32>(0.78, 0.62, 0.38);
const BAR_FILL: vec3<f32> = vec3<f32>(0.85, 0.72, 0.42);
const BAR_EMPTY: vec3<f32> = vec3<f32>(0.08, 0.08, 0.10);
const BAR_BORDER: vec3<f32> = vec3<f32>(0.55, 0.55, 0.60);
// One turn in three seconds.
const TURN_S: f32 = 3.0;
// A voxel's half-edge and the pitch between voxel centres (a quarter-voxel gap).
const HALF: f32 = 0.5;
const PITCH: f32 = 1.25;
// The block's bounding radius: the corner of the outer voxels.
const BOUND: f32 = 3.04;
// The eye's distance from the block's centre, in voxels.
const EYE: f32 = 9.0;

// Coverage of a signed distance in pixels: 0 outside, 1 inside, a one-pixel ramp between.
fn ramp(d: f32) -> f32 {
    return clamp(d + 0.5, 0.0, 1.0);
}

// Rotation about the unit axis `a` by `t` radians.
fn rotate(a: vec3<f32>, t: f32) -> mat3x3<f32> {
    let c = cos(t);
    let s = sin(t);
    let k = 1.0 - c;
    return mat3x3<f32>(
        vec3<f32>(c + a.x * a.x * k, a.y * a.x * k + a.z * s, a.z * a.x * k - a.y * s),
        vec3<f32>(a.x * a.y * k - a.z * s, c + a.y * a.y * k, a.z * a.y * k + a.x * s),
        vec3<f32>(a.x * a.z * k + a.y * s, a.y * a.z * k - a.x * s, c + a.z * a.z * k),
    );
}

// One layer of the cube under a pixel: its colour and coverage and the depth it enters at.
struct Layer {
    colour: vec3<f32>,
    cover: f32,
    t: f32,
};

// The cube's colour over `bg` along the ray `o + t d` (object space) at pixel footprint `px`
// (object units per pixel at the block's distance). The two nearest voxels the ray passes
// within a pixel of are composited, the far one under the near one; each voxel's coverage
// is its slab overlap in pixels, and the edge between two of its faces blends their shades
// over a pixel.
fn cube(o: vec3<f32>, d: vec3<f32>, px: f32, light: vec3<f32>, bg: vec3<f32>) -> vec3<f32> {
    var near = Layer(vec3<f32>(0.0), 0.0, 1e9);
    var far = Layer(vec3<f32>(0.0), 0.0, 1e9);
    let inv = 1.0 / d;
    for (var i = 0; i < 27; i++) {
        let c = (vec3<f32>(f32(i % 3), f32((i / 3) % 3), f32(i / 9)) - 1.0) * PITCH;
        let t0 = (c - HALF - o) * inv;
        let t1 = (c + HALF - o) * inv;
        let lo = min(t0, t1);
        let hi = max(t0, t1);
        let enter = max(max(lo.x, lo.y), lo.z);
        let exit = min(min(hi.x, hi.y), hi.z);
        // The ray's distance outside the box, in object units, from the slab overlap: the
        // overlap shrinks by about a footprint's worth per pixel across a silhouette.
        let cover = ramp((exit - enter) / px);
        if (cover <= 0.0) {
            continue;
        }
        // The face entered and the runner-up: their gap in pixels blends the two shades.
        var n1 = vec3<f32>(0.0);
        var n2 = vec3<f32>(0.0);
        var second = 0.0;
        if (enter == lo.x) {
            n1 = vec3<f32>(-sign(d.x), 0.0, 0.0);
            if (lo.y > lo.z) { second = lo.y; n2 = vec3<f32>(0.0, -sign(d.y), 0.0); } else { second = lo.z; n2 = vec3<f32>(0.0, 0.0, -sign(d.z)); }
        } else if (enter == lo.y) {
            n1 = vec3<f32>(0.0, -sign(d.y), 0.0);
            if (lo.x > lo.z) { second = lo.x; n2 = vec3<f32>(-sign(d.x), 0.0, 0.0); } else { second = lo.z; n2 = vec3<f32>(0.0, 0.0, -sign(d.z)); }
        } else {
            n1 = vec3<f32>(0.0, 0.0, -sign(d.z));
            if (lo.x > lo.y) { second = lo.x; n2 = vec3<f32>(-sign(d.x), 0.0, 0.0); } else { second = lo.y; n2 = vec3<f32>(0.0, -sign(d.y), 0.0); }
        }
        let s1 = 0.30 + 0.70 * max(dot(n1, light), 0.0);
        let s2 = 0.30 + 0.70 * max(dot(n2, light), 0.0);
        let w = ramp((enter - second) / px);
        let shade = mix(0.5 * (s1 + s2), s1, w);
        let layer = Layer(CUBE * shade, cover, enter);
        if (enter < near.t) {
            far = near;
            near = layer;
        } else if (enter < far.t) {
            far = layer;
        }
    }
    var out = bg;
    if (far.cover > 0.0) {
        out = mix(out, far.colour, far.cover);
    }
    if (near.cover > 0.0) {
        out = mix(out, near.colour, near.cover);
    }
    return out;
}

// The bar over `col` at pixel `p`: the border, then the inner rect filled left of the
// progress, each edge on a one-pixel ramp.
fn bar(p: vec2<f32>, col: vec3<f32>) -> vec3<f32> {
    let r = params.bar;
    let b = params.border;
    // Signed distances inside the outer rect and the inner one, in pixels.
    let outer = min(min(p.x - r.x, r.z - p.x), min(p.y - r.y, r.w - p.y));
    let inner = outer - 2.0 * b;
    let inner_x0 = r.x + 2.0 * b;
    let inner_x1 = r.z - 2.0 * b;
    let fill_x = inner_x0 + (inner_x1 - inner_x0) * clamp(params.progress, 0.0, 1.0);
    var out = mix(col, BAR_BORDER, ramp(outer) * (1.0 - ramp(outer - b)));
    let filled = ramp(fill_x - p.x);
    out = mix(out, mix(BAR_EMPTY, BAR_FILL, filled), ramp(inner));
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let p = in.pos.xy;
    let h = params.size.y;
    var col = mix(BG_TOP, BG_BOTTOM, p.y / max(h, 1.0));
    // The cube: a pinhole eye at `EYE` voxels, its view scaled so the block's bound spans
    // `cube_px` pixels.
    let centre = vec2<f32>(0.5 * params.size.x, params.cube_y);
    let q = (p - centre) / params.cube_px * (BOUND / EYE);
    if (dot(q, q) < (BOUND / EYE) * (BOUND / EYE) * 1.6) {
        let spin = rotate(normalize(vec3<f32>(0.35, 1.0, 0.2)), 6.2831853 * params.seconds / TURN_S);
        let tilt = rotate(vec3<f32>(1.0, 0.0, 0.0), 0.45);
        // Object from view: the transpose of view from object.
        let m = transpose(tilt * spin);
        let o = m * vec3<f32>(0.0, 0.0, -EYE);
        let d = m * normalize(vec3<f32>(q.x, -q.y, 1.0));
        let light = m * normalize(vec3<f32>(-0.4, 0.8, -0.6));
        let px = BOUND / params.cube_px;
        col = cube(o, d, px, light, col);
    }
    col = bar(p, col);
    return vec4<f32>(col * params.alpha, params.alpha);
}
