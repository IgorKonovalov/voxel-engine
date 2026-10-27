// The exposure pass (ADR-0033): one workgroup of 256 at the end of the frame. It reads the
// composite's meter tiles, weights them, and writes the exposure state the next frame's passes
// read. Standalone: it is not compiled after march.wgsl and binds no world.
//
// Bindings, group 0:
//   0  uniform           ex              the pass's parameters (`ExposureUniform` in exposure.rs)
//   1  storage, rw       state           the exposure state (`ExposureState` in exposure.rs)
//   2  storage, read     tiles           the composite's tiles: per 8 x 8 workgroup of the
//                                        internal target, row-major, the sum of the clamped
//                                        log2 luminance of its pixels and their count
//
// Tile `i` weighs `w_i = 1 + strength exp(-r_i^2 / (2 sigma^2))`, `r_i` the distance of the
// tile's centre (its 8 x 8 square's centre, in pixels) from the centre of the view whose
// rectangle holds that point (the first view when none does), in units of half that view's
// height. The meter is `sum(w_i S_i) / sum(w_i n_i)`. Without a metered pixel (another view
// than the shaded one, or an empty target) the state's adapted and metered levels hold and
// the HELD flag is set.
//
// In auto, with a meter and `dt > 0`, the adapted level steps toward it (`step_adapted`,
// `AutoExposure::step` in f32); with the reset bit it jumps to it; without a meter or with
// `dt` 0 it holds bit for bit. In manual the adapted level holds.
//
// `previous` becomes the exposure this frame's passes used (the state's `current` in auto,
// else the manual value: `exposure_now` in march.wgsl) and `current` the one the next frame
// uses: the manual value, or in auto `key 2^compensation / 2^adapted` clamped to the limits.

struct ExUniform {
    tiles: vec2<u32>,
    view_count: u32,
    // Bit 0: auto; bit 1: meter (the shaded view ran); bit 2: reset (jump to the meter).
    mode: u32,
    // The views' rectangles (x, y, w, h) in internal pixels.
    rects: array<vec4<f32>, 2>,
    // The manual exposure, the metering's strength and sigma, and the frame's step in
    // seconds.
    manual: f32,
    strength: f32,
    sigma: f32,
    dt: f32,
    // The auto model (`AutoExposure` in exposure.rs): `key 2^compensation`, the limits, the
    // time constants in seconds and log2 of the cone floor.
    key: f32,
    min_e: f32,
    max_e: f32,
    tau_light: f32,
    tau_cone: f32,
    tau_rod: f32,
    floor_log2: f32,
    _pad: f32,
};

struct ExState {
    current: f32,
    previous: f32,
    adapted_log2: f32,
    metered_log2: f32,
    frame: u32,
    flags: u32,
    _pad0: u32,
    _pad1: u32,
};

@group(0) @binding(0) var<uniform> ex: ExUniform;
@group(0) @binding(1) var<storage, read_write> state: ExState;
@group(0) @binding(2) var<storage, read> tiles: array<vec2<f32>>;

const EX_AUTO: u32 = 1u;
const EX_METER: u32 = 2u;
const EX_RESET: u32 = 4u;
const STATE_HELD: u32 = 1u;
const TILE: f32 = 8.0;

// `exp(-dt / tau)`, 0 for a constant of 0.
fn decay(dt: f32, tau: f32) -> f32 {
    if (tau <= 0.0) {
        return 0.0;
    }
    return exp(-dt / tau);
}

// The adapted level `a` after `dt` seconds toward the meter `t`: toward bright at tau_light;
// toward dark at tau_cone above the floor and tau_rod below it, a step that crosses the floor
// split where it reaches it.
fn step_adapted(a: f32, t: f32, dt: f32) -> f32 {
    if (t >= a) {
        return t + (a - t) * decay(dt, ex.tau_light);
    }
    let f = ex.floor_log2;
    if (a > f) {
        if (t >= f) {
            return t + (a - t) * decay(dt, ex.tau_cone);
        }
        var to_floor = 0.0;
        if (ex.tau_cone > 0.0) {
            to_floor = ex.tau_cone * log((a - t) / (f - t));
        }
        if (dt <= to_floor) {
            return t + (a - t) * decay(dt, ex.tau_cone);
        }
        return t + (f - t) * decay(dt - to_floor, ex.tau_rod);
    }
    return t + (a - t) * decay(dt, ex.tau_rod);
}

var<workgroup> partial: array<vec2<f32>, 256>;

// Tile `i`'s weight under the metering.
fn tile_weight(i: u32) -> f32 {
    let t = vec2<f32>(f32(i % ex.tiles.x), f32(i / ex.tiles.x));
    let c = (t + 0.5) * TILE;
    var r = ex.rects[0];
    for (var v = 1u; v < ex.view_count; v++) {
        let q = ex.rects[v];
        if (all(c >= q.xy) && all(c < q.xy + q.zw)) {
            r = q;
        }
    }
    let d = (c - (r.xy + 0.5 * r.zw)) / (0.5 * r.w);
    let s = max(ex.sigma, 1e-6);
    return 1.0 + ex.strength * exp(-dot(d, d) / (2.0 * s * s));
}

@compute @workgroup_size(256, 1, 1)
fn exposure(@builtin(local_invocation_index) lane: u32) {
    let n = ex.tiles.x * ex.tiles.y;
    var acc = vec2<f32>(0.0);
    if ((ex.mode & EX_METER) != 0u) {
        for (var i = lane; i < n; i += 256u) {
            let w = tile_weight(i);
            acc += w * tiles[i];
        }
    }
    partial[lane] = acc;
    workgroupBarrier();
    for (var stride = 128u; stride > 0u; stride >>= 1u) {
        if (lane < stride) {
            partial[lane] += partial[lane + stride];
        }
        workgroupBarrier();
    }
    if (lane != 0u) {
        return;
    }
    let sum = partial[0];
    var s = state;
    let used = select(ex.manual, s.current, (ex.mode & EX_AUTO) != 0u);
    s.flags = 0u;
    let metered = sum.y > 0.0;
    if (metered) {
        s.metered_log2 = sum.x / sum.y;
    } else {
        s.flags |= STATE_HELD;
    }
    s.previous = used;
    if ((ex.mode & EX_AUTO) != 0u) {
        if (metered && (ex.mode & EX_RESET) != 0u) {
            s.adapted_log2 = s.metered_log2;
        } else if (metered && ex.dt > 0.0) {
            s.adapted_log2 = step_adapted(s.adapted_log2, s.metered_log2, ex.dt);
        }
        s.current = clamp(ex.key / exp2(s.adapted_log2), ex.min_e, ex.max_e);
    } else {
        s.current = ex.manual;
    }
    s.frame += 1u;
    state = s;
}
