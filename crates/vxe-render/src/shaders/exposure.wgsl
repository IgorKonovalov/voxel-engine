// The exposure pass (ADR-0033): one workgroup of 256 at the end of the frame. It reads the
// composite's meter tiles, weights them, and writes the exposure state the next frame's passes
// read. Compiled after the prelude alone (frame/compose.rs): it joins none of the frame's
// modules and binds no world.
//
// Bindings, group 0:
//   0  uniform           ex              the pass's parameters (`ExposureUniform` in exposure.rs)
//   1  storage, rw       state           the exposure state (`ExposureState` in exposure.rs)
//   2  storage, read     tiles           the composite's tiles: per 8 x 8 workgroup of the
//                                        internal target, row-major, the sum of the clamped
//                                        log2 luminance of its pixels, their count, the count
//                                        of those that are sky, and 0
//   3  storage, read     sky_e           `E_sky` (sky_irradiance.wgsl): six vec4, `+z`'s w the
//                                        key light's horizontal irradiance in luminance
//
// Tile `i` weighs `w_i = 1 + strength exp(-r_i^2 / (2 sigma^2))`, `r_i` the distance of the
// tile's centre (its 8 x 8 square's centre, in pixels) from the centre of the view whose
// rectangle holds that point (the first view when none does), in units of half that view's
// height. The meter is `sum(w_i S_i) / sum(w_i n_i)` and the sky's share of the view
// `sum(w_i k_i) / sum(w_i n_i)` (`k_i` the tile's sky pixels). Without a metered pixel
// (another view than the shaded one, or an empty target) the state's adapted and metered
// levels and its share hold and the HELD flag is set.
//
// In auto, with a meter and `dt > 0`, the adapted level steps toward it (`step_capped`: the
// model's step clamped to the rate cap, `AutoExposure::step` in f32); with the reset bit it jumps to it; without a meter or with
// `dt` 0 it holds bit for bit. In manual the adapted level holds.
//
// `previous` becomes the exposure this frame's passes used (the state's `current` in auto,
// else the manual value: `exposure_now` in common.wgsl) and `current` the one the next frame
// uses: the manual value, or in auto `key 2^compensation / 2^adapted` clamped to the limits.
// The upper limit is the state's `ceiling`, the ceiling in force. Its target is `max_e`, or
// with `anchor` over 0 (`2^N`: the frame holds a sky and the model anchors its ceiling,
// `AutoExposure::ceiling`) and the sunlit ground's irradiance `E` over 0, the anchored
// `c_a = anchor key / (0.5 E / pi)`, `E` being `E_sky(+z)`'s luminance plus its w. With
// `sky_share` (`F`) over 0 the anchor holds by the sky in view (`AutoExposure::ceiling_target`):
// `log2 c = mix(log2 max(max_e, c_a), log2 c_a, min(f_sky / F, 1))`, so with no sky in view the
// anchor releases to `max_e` and never pulls the ceiling under `c_a`. Under the anchor with
// `F` and the rate over 0 the ceiling in force steps toward its target by at most the rate
// cap's step in log2 (ADR-0076 rule 5, `AutoExposure::step_ceiling`); else, on the reset bit
// or from a state with no ceiling yet, it is the target.

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
    // `2^N` under the sky anchor, else 0.
    anchor: f32,
    // The adapted level's and the ceiling's most step per second in EV (ADR-0076 rule 5; 0:
    // no cap), and the longest frame step it pays for.
    rate_ev_s: f32,
    max_rate_dt: f32,
    // The sky share `F` at and over which the anchor holds in full; 0 holds it whatever is in
    // view.
    sky_share: f32,
    _pad1: f32,
};

struct ExState {
    current: f32,
    previous: f32,
    adapted_log2: f32,
    metered_log2: f32,
    frame: u32,
    flags: u32,
    ceiling: f32,
    // The metered share of the view that is sky, under the meter's weights.
    sky: f32,
};

@group(0) @binding(0) var<uniform> ex: ExUniform;
@group(0) @binding(1) var<storage, read_write> state: ExState;
@group(0) @binding(2) var<storage, read> tiles: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read> sky_e: array<vec4<f32>, 6>;

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

// `step_adapted` with its step clamped to `rate_ev_s * min(dt, max_rate_dt)` EV (no clamp
// at a rate of 0), so the adapted level moves at a bounded rate toward the meter.
fn step_capped(a: f32, t: f32, dt: f32) -> f32 {
    let next = step_adapted(a, t, dt);
    if (ex.rate_ev_s <= 0.0) {
        return next;
    }
    let r = ex.rate_ev_s * min(dt, ex.max_rate_dt);
    return a + clamp(next - a, -r, r);
}

// The anchored ceiling `c_a`, or `max_e` without an anchor or a lit ground (the header).
fn anchored() -> f32 {
    if (ex.anchor <= 0.0) {
        return ex.max_e;
    }
    let up = sky_e[5];
    let e = dot(up.xyz, vec3<f32>(0.2126, 0.7152, 0.0722)) + up.w;
    if (e <= 0.0) {
        return ex.max_e;
    }
    return ex.anchor * ex.key / (0.5 * e / 3.14159265358979);
}

// The ceiling's target at the sky share `f_sky` (the header).
fn ceiling_target(f_sky: f32) -> f32 {
    let a = anchored();
    if (ex.anchor <= 0.0 || ex.sky_share <= 0.0) {
        return a;
    }
    // At a share of `F` and over the anchored ceiling exactly, bit for bit today's.
    if (f_sky >= ex.sky_share) {
        return a;
    }
    let open = max(ex.max_e, a);
    return exp2(mix(log2(open), log2(a), f_sky / ex.sky_share));
}

// The ceiling in force after a frame from `c` toward its target `goal` (the header).
fn step_ceiling(c: f32, goal: f32, reset: bool) -> f32 {
    if (ex.anchor <= 0.0 || ex.sky_share <= 0.0 || ex.rate_ev_s <= 0.0 || reset || c <= 0.0) {
        return goal;
    }
    let r = ex.rate_ev_s * min(ex.dt, ex.max_rate_dt);
    return exp2(log2(c) + clamp(log2(goal) - log2(c), -r, r));
}

var<workgroup> partial: array<vec4<f32>, 256>;

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
    var acc = vec4<f32>(0.0);
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
        s.sky = sum.z / sum.y;
    } else {
        s.flags |= STATE_HELD;
    }
    s.previous = used;
    if ((ex.mode & EX_AUTO) != 0u) {
        if (metered && (ex.mode & EX_RESET) != 0u) {
            s.adapted_log2 = s.metered_log2;
        } else if (metered && ex.dt > 0.0) {
            s.adapted_log2 = step_capped(s.adapted_log2, s.metered_log2, ex.dt);
        }
        let reset = (ex.mode & EX_RESET) != 0u;
        s.ceiling = step_ceiling(s.ceiling, ceiling_target(s.sky), reset);
        s.current = clamp(ex.key / exp2(s.adapted_log2), ex.min_e, max(s.ceiling, ex.min_e));
    } else {
        s.ceiling = ex.max_e;
        s.current = ex.manual;
    }
    s.frame += 1u;
    state = s;
}
