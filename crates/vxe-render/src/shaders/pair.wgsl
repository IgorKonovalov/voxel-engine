// The visibility pair (ADR-0070) of the temporal pass: its running means and the no-ray
// value's fade. Compiled after `sky.wgsl` and before
// `temporal.wgsl`, whose uniforms (`params`, `sky`) and helpers (`above_column`) it uses.

// The visibility pair (ADR-0070). The short mean's frames, and the age past which a sample is
// clamped.
const SKY_SHORT: f32 = 8.0;
// A sample is clamped at this many times the larger of the long and short means (Plan 0086's
// readings log, Phase 1: the smallest integer that leaves every pixel of a hit rate over 0.3
// unclamped).
const FIREFLY_K: f32 = 4.0;
// The band about the long mean the short mean stays in: this many binomial standard deviations
// of an 8-frame mean at the long mean's rate, plus SKY_BAND_EPS (a half's rounding and a
// still pixel's drift).
const SKY_BAND_SIGMAS: f32 = 3.0;
const SKY_BAND_EPS: f32 = 0.02;
// The band's variance is at least a rate of P_MIN's, so a pair at 0 or 1 (a seed, or a
// settled pixel at the extremes) is not the band's degenerate point: one disagreeing sample
// (1 / SKY_SHORT) stays inside it, two in a row leave it.
const P_MIN: f32 = 0.02;
// Out of the band the age falls to SKY_SHORT times the band over the distance, at least this.
const SKY_FALL_MIN: f32 = 2.0;
// The samples a reset pair's seed counts as: the next sample blends at `1 / (SEED_AGE + 1)`,
// past SKY_SHORT, so the firefly clamp holds from the frame after the reset.
const SEED_AGE: f32 = 8.0;
// The samples a reset pair's seed counts as where it is the column's answer (no record): a
// coarse prior that does not resolve a hole narrower than the column map's cell, so the
// samples outweigh it sooner.
const SEED_COLUMN: f32 = 2.0;

// How far a seed's column test lifts the hit along the face normal, in voxels: past the
// column map's cell, so the point is not read in the wall's own cell (whose top is the wall's).
const SEED_LIFT: f32 = f32(COLUMN_CELL) + 0.5;

// A reset pair's seed with no record: the column map's answer (ADR-0070's sky rule) at hit `p`
// lifted SEED_LIFT voxels along `n` into the air the face looks into, 1 above its column's
// top of solid, else 0.
fn column_estimate(p: vec3<f32>, n: vec3<f32>) -> f32 {
    return select(0.0, 1.0, above_column(p + n * SEED_LIFT));
}

// The key light's horizon over the column map (ADR-0079, `column_key_visibility`): the coarse
// march's distances, COLUMN_KEY_STEPS of them in a geometric series from COLUMN_KEY_D0 to
// COLUMN_KEY_D1 voxels, and the most cells the refinement walks inside one of its intervals.
const COLUMN_KEY_STEPS: u32 = 12u;
const COLUMN_KEY_D0: f32 = 4.0;
const COLUMN_KEY_D1: f32 = 512.0;
const COLUMN_KEY_REFINE: u32 = 8u;

// The column map's texel for a column with no opaque voxel (`gpu_world::COLUMN_NONE`).
const COLUMN_NONE_I: i32 = -2147483647 - 1;

// The column map's height (the top voxel's z plus 1) over map cell `c` (global cell
// coordinates, COLUMN_CELL voxels an edge): -1e9 where the column holds no opaque voxel, and
// 3e38 outside the window's footprint, which the march reads as its end.
fn column_cell_height(c: vec2<i32>) -> f32 {
    let x = c * COLUMN_CELL;
    let lo = params.window_min.xy * 64;
    let hi = (params.window_min.xy + params.window_size.xy) * 64;
    if (any(x < lo) || any(x >= hi)) {
        return 3.0e38;
    }
    let k = x >> vec2<u32>(6u);
    let w = params.window_size.xy;
    let tile = ((k % w) + w) % w;
    let top = textureLoad(column_map, tile * COLUMN_CELLS + (x - k * 64) / COLUMN_CELL, 0).x;
    return select(f32(top) + 1.0, -1.0e9, top == COLUMN_NONE_I);
}

// The ray parameter at which a horizontal ray from `q` along unit `u` enters map cell `c`
// (the larger of the axes' near-plane crossings; an axis `u` does not move along is entered
// at -3e38).
fn column_cell_entry(q: vec2<f32>, u: vec2<f32>, c: vec2<i32>) -> f32 {
    let lo = vec2<f32>(c * COLUMN_CELL);
    let near = select(lo + f32(COLUMN_CELL), lo, u > vec2<f32>(0.0));
    let t = select(vec2<f32>(-3.0e38), (near - q) / u, abs(u) > vec2<f32>(1.0e-6));
    return max(t.x, t.y);
}

// The key light's visibility at hit `p` with face normal `n` from the column map's tops
// (ADR-0079's horizon, no state): the hit lifted SEED_LIFT voxels along `n` to `q`; the
// horizon's tangent toward the sun the largest `(height - q.z) / t` over the map cells along the
// sun's horizontal direction, `t` the distance at which the ray from `q` enters the cell (so a
// box heightfield's edge stands where its cell begins): the cell at each of the coarse march's
// distances, then every cell (at most COLUMN_KEY_REFINE) of the one interval between two of
// those distances whose far cell's height over its near end's distance is largest. A point
// under its own cell's top reads 0 (inside the heightfield). A cell outside the
// window's footprint ends the march (the rest is lit); a column with no opaque voxel casts
// nothing. The visibility is `clamp((tan e - tan h) / (2 tan r) + 1/2, 0, 1)`, `e` the sun's
// elevation, `h` the horizon's, `r` the disc's radius: 1 above every top, 0 under one, the
// disc's width of ramp between. 0 where the sun is at or under the horizon.
fn column_key_visibility(p: vec3<f32>, n: vec3<f32>) -> f32 {
    let l = sky.key_dir;
    if (l.z <= 0.0) {
        return 0.0;
    }
    let q = p + n * SEED_LIFT;
    let flat = length(l.xy);
    let tan_e = l.z / max(flat, 1.0e-6);
    let u = select(vec2<f32>(1.0, 0.0), l.xy / flat, flat > 1.0e-6);
    // Under its own column's top the lifted point is inside the heightfield: shadowed at any
    // elevation, as the sky's column test reads it (`column_estimate`).
    let own = column_cell_height(vec2<i32>(floor(q.xy / f32(COLUMN_CELL))));
    if (own < 1.0e38 && own > q.z) {
        return 0.0;
    }
    var tan_h = -3.0e38;
    let ratio = pow(COLUMN_KEY_D1 / COLUMN_KEY_D0, 1.0 / f32(COLUMN_KEY_STEPS - 1u));
    var d = COLUMN_KEY_D0;
    var d_prev = f32(COLUMN_CELL);
    var best = -3.0e38;
    var seg = vec2<f32>(0.0, -1.0);
    for (var k = 0u; k < COLUMN_KEY_STEPS; k++) {
        let c = vec2<i32>(floor((q.xy + u * d) / f32(COLUMN_CELL)));
        let h = column_cell_height(c);
        if (h > 1.0e38) {
            break;
        }
        let t = max(column_cell_entry(q.xy, u, c), 0.5);
        tan_h = max(tan_h, (h - q.z) / t);
        let score = (h - q.z) / d_prev;
        if (score > best) {
            best = score;
            seg = vec2<f32>(d_prev, d);
        }
        d_prev = d;
        d *= ratio;
    }
    // The refinement: the interval's cells in order, from the one holding its start.
    if (seg.y > seg.x) {
        var c = vec2<i32>(floor((q.xy + u * seg.x) / f32(COLUMN_CELL)));
        let s = vec2<i32>(sign(u));
        for (var i = 0u; i < COLUMN_KEY_REFINE; i++) {
            let t = column_cell_entry(q.xy, u, c);
            if (t >= seg.y) {
                break;
            }
            let h = column_cell_height(c);
            if (h > 1.0e38) {
                break;
            }
            if (t > 0.5) {
                tan_h = max(tan_h, (h - q.z) / t);
            }
            // The next cell: across the face the ray leaves through first.
            let far = vec2<f32>(c * COLUMN_CELL) + select(vec2<f32>(0.0), vec2<f32>(f32(COLUMN_CELL)), u > vec2<f32>(0.0));
            let tx = select(vec2<f32>(3.0e38), (far - q.xy) / u, abs(u) > vec2<f32>(1.0e-6));
            c += select(vec2<i32>(0, s.y), vec2<i32>(s.x, 0), tx.x <= tx.y);
        }
    }
    let r = max(tan(sky.key_radius), 1.0e-4);
    return clamp((tan_e - tan_h) / (2.0 * r) + 0.5, 0.0, 1.0);
}

// The key slot (the pair's g) at a pixel where the light pass cast no key ray holds the far
// estimate tagged by its source, so the next frame tells a source's switch from the source's
// own motion; a cast pixel's g is the ray's running mean, in 0 to 1 (atrous.wgsl pools it).
// A record's value `v` is stored as `KEY_TAG_RECORD - v` (-2 to -1), the horizon's as
// `KEY_TAG_HORIZON + v` (2 to 3), a value in a landing's fade as `KEY_TAG_FADE + v` (4 to 5).
// A reprojected slot between the ranges (taps of two kinds blended) reads as no history.
const KEY_TAG_RECORD: f32 = -1.0;
const KEY_TAG_HORIZON: f32 = 2.0;
const KEY_TAG_FADE: f32 = 4.0;
const KEY_SRC_MEAN: u32 = 0u;
const KEY_SRC_RECORD: u32 = 1u;
const KEY_SRC_HORIZON: u32 = 2u;
const KEY_SRC_FADE: u32 = 3u;
const KEY_SRC_NONE: u32 = 4u;

// Slot `y`'s value in x and its source (`KEY_SRC_*`, as f32) in y; KEY_SRC_NONE between ranges.
fn key_slot_decode(y: f32) -> vec2<f32> {
    let e = 1.0 / 512.0;
    if (y >= -e && y <= 1.0 + e) {
        return vec2<f32>(clamp(y, 0.0, 1.0), f32(KEY_SRC_MEAN));
    }
    if (y >= -2.0 - e && y <= -1.0 + e) {
        return vec2<f32>(clamp(KEY_TAG_RECORD - y, 0.0, 1.0), f32(KEY_SRC_RECORD));
    }
    if (y >= KEY_TAG_HORIZON - e && y <= KEY_TAG_HORIZON + 1.0 + e) {
        return vec2<f32>(clamp(y - KEY_TAG_HORIZON, 0.0, 1.0), f32(KEY_SRC_HORIZON));
    }
    if (y >= KEY_TAG_FADE - e && y <= KEY_TAG_FADE + 1.0 + e) {
        return vec2<f32>(clamp(y - KEY_TAG_FADE, 0.0, 1.0), f32(KEY_SRC_FADE));
    }
    return vec2<f32>(0.0, f32(KEY_SRC_NONE));
}

// The slot of value `v` from source `src` (KEY_SRC_RECORD, _HORIZON or _FADE).
fn key_slot_encode(v: f32, src: u32) -> f32 {
    if (src == KEY_SRC_RECORD) {
        return KEY_TAG_RECORD - v;
    }
    return select(KEY_TAG_FADE, KEY_TAG_HORIZON, src == KEY_SRC_HORIZON) + v;
}

// The far key value shown at a pixel with no key ray and the slot it leaves (ADR-0079's
// landing fade, rule 3's form): `t` the far estimate this frame from `src` (KEY_SRC_RECORD or
// _HORIZON), `hy` the reprojected slot, `accepted` whether the history is taken. A pixel whose
// history is not taken, or whose slot was the same source's, shows `t` at once (the source's
// own value follows the hit); a slot of the other source, or one in a fade, moves toward `t`
// by at most 1 / FADE_F a frame, so a landing is one monotonic fade within FADE_F frames; a
// cast pixel's mean hands over at once under the blend (the band already showed the far
// value at its end) and fades under LIGHTING_KEY_STEP. LIGHTING_HOOK_NO_FADE shows `t` at
// once. Returns the value and the slot.
fn key_far_fade(t: f32, src: u32, hy: f32, accepted: bool) -> vec2<f32> {
    let steady = vec2<f32>(t, key_slot_encode(t, src));
    if (!accepted || (params.lighting & LIGHTING_HOOK_NO_FADE) != 0u) {
        return steady;
    }
    let prev = key_slot_decode(hy);
    let was = u32(prev.y);
    if (was == KEY_SRC_NONE || was == src
        || (was == KEY_SRC_MEAN && (params.lighting & LIGHTING_KEY_STEP) == 0u)
        || abs(t - prev.x) <= FADE_EPS) {
        return steady;
    }
    let x = prev.x + clamp(t - prev.x, -1.0 / FADE_F, 1.0 / FADE_F);
    if (abs(t - x) <= FADE_EPS) {
        return steady;
    }
    return vec2<f32>(x, key_slot_encode(x, KEY_SRC_FADE));
}

// The visibility pair after sample `s` (the sky rays' share this frame) on history `hp`
// (`accepted`: the direct part's taps agreed and nothing resets it), its cap `sky.sky_history`.
// A reset pair holds the estimate `e` alone in both means at age `seed_age` (SEED_AGE for a
// record's value, SEED_COLUMN for the column's); this frame's sample joins from the next frame.
fn pair_step(s: f32, hp: vec4<f32>, accepted: bool, e: f32, seed_age: f32) -> vec4<f32> {
    let age = floor(hp.w + 0.5);
    if (!accepted || age < 1.0 || (params.lighting & LIGHTING_SKY_CHANGED) != 0u) {
        return vec4<f32>(e, 0.0, e, seed_age);
    }
    let cap = f32(max(sky.sky_history, 1u));
    let n0 = min(age + 1.0, cap);
    let b = hp.z + (s - hp.z) / min(n0, SKY_SHORT);
    let r = hp.x;
    let in_band = abs(b - r) <= sky_band(r);
    var sc = s;
    if (n0 > SKY_SHORT && (params.lighting & LIGHTING_HOOK_NO_CLAMP) == 0u) {
        sc = min(s, FIREFLY_K * max(r, b));
    }
    var n = n0;
    if (!in_band) {
        n = min(n, max(SKY_FALL_MIN, floor(SKY_SHORT * sky_band(r) / abs(b - r))));
    }
    return vec4<f32>(r + (sc - r) / n, 0.0, b, n);
}

// The band about a mean `g` of binary samples (`pair_step`'s, with SKY_BAND_SIGMAS above).
fn sky_band(g: f32) -> f32 {
    return SKY_BAND_SIGMAS * sqrt(max(g * (1.0 - g), P_MIN * (1.0 - P_MIN)) / SKY_SHORT)
        + SKY_BAND_EPS;
}

// The frames of a no-ray value's fade (rule 3's F, ADR-0046's FADE), and the samples a reset
// pair seeded from a stale record counts under LIGHTING_HOOK_SEED_STALE_F.
const FADE_F: f32 = 16.0;

// The key light's visibility mean after sample `s` on history `g`, which holds `age` samples
// (the direct part's history age; `reset`: the history is not taken): over `age + 1` samples,
// as the direct part blends, capped at `key_history`. With a fresh baked value `seed` (0 to 1;
// negative for none, ADR-0079) a reset mean starts from the seed instead of the sample, and
// the seed counts as SEED_AGE samples of the mean from then on (a pixel whose record landed
// after its reset is counted the same). On the sky-change bit the sample restarts it: the
// record predates the jump.
fn key_mean(s: f32, g: f32, age: f32, reset: bool, seed: f32) -> f32 {
    if ((params.lighting & LIGHTING_SKY_CHANGED) != 0u) {
        return s;
    }
    if (reset) {
        return select(s, seed, seed >= 0.0);
    }
    let seeded = select(0.0, SEED_AGE, seed >= 0.0);
    let n = clamp(age + 1.0 + seeded, 1.0, f32(max(sky.key_history, 1u)));
    return g + (s - g) / n;
}

// The no-ray value's fade (ADR-0076 rule 3, one fade of FADE_F frames for every source): a
// pixel where no sky ray was cast shows its source's visibility `t` (a record, its chunk's
// summary, the column) through its pair's x. Where its history `hp` is taken (`accepted`) and
// x differs from `t` by more than FADE_EPS, x ramps to `t` linearly over FADE_F frames, the
// frames left in the pair's w past the cap `cap`; a target that moves mid-ramp is reached by
// the same frame. A reset pixel shows `t` at once. Returns x and w.
fn no_ray_fade(t: f32, hp: vec4<f32>, accepted: bool, cap: f32) -> vec2<f32> {
    if (!accepted || (params.lighting & LIGHTING_HOOK_NO_FADE) != 0u) {
        return vec2<f32>(t, cap);
    }
    let left = clamp(floor(hp.w - cap + 0.5), 0.0, FADE_F);
    let x = clamp(hp.x, 0.0, 1.0);
    if (left < 1.0) {
        if (abs(t - x) <= FADE_EPS) {
            return vec2<f32>(t, cap);
        }
        return vec2<f32>(x + (t - x) / FADE_F, cap + FADE_F - 1.0);
    }
    return vec2<f32>(x + (t - x) / left, cap + left - 1.0);
}

// A no-ray value's change under this is not faded: a stored byte's rounding.
const FADE_EPS: f32 = 1.0 / 255.0;
