// The visibility pair (ADR-0070) of the temporal pass: its running means, the settle, the
// reprojection taps' reading and the no-ray value's fade. Compiled after `sky.wgsl` and before
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

// The visibility pair after sample `s` (the sky rays' share this frame) on history `hp`
// (`accepted`: the direct part's taps agreed and nothing resets it), its cap `sky.sky_history`.
// A reset pair holds the estimate `e` alone in both means at age `seed_age` (SEED_AGE for a
// record's value, SEED_COLUMN for the column's); this frame's sample joins from the next frame.
//
// The settle (unless LIGHTING_NO_SETTLE; `settle_at_cap`): a pair at its cap carries the
// settle's state in its age past the cap. Below the cap, and at the cap before it has settled,
// the short mean leaving the band makes the age fall as above.
fn pair_step(s: f32, hp: vec4<f32>, accepted: bool, e: f32, seed_age: f32) -> vec4<f32> {
    let age = floor(hp.w + 0.5);
    if (!accepted || age < 1.0 || (params.lighting & LIGHTING_SKY_CHANGED) != 0u) {
        return vec4<f32>(e, 0.0, e, seed_age);
    }
    let cap = f32(max(sky.sky_history, 1u));
    let n0 = min(age + 1.0, cap);
    let b = hp.z + (s - hp.z) / min(n0, SKY_SHORT);
    let r = hp.x;
    let in_band = abs(b - r) <= settle_band(r);
    var sc = s;
    if (n0 > SKY_SHORT && (params.lighting & LIGHTING_HOOK_NO_CLAMP) == 0u) {
        sc = min(s, FIREFLY_K * max(r, b));
    }
    let c = age - cap;
    if (settle_on() && age >= cap && (in_band || c >= SETTLE_F)) {
        let st = settle_at_cap(r, r + (sc - r) / cap, b, in_band, c);
        return vec4<f32>(st.x, 0.0, b, cap + st.y);
    }
    var n = n0;
    if (!in_band) {
        n = min(n, max(SKY_FALL_MIN, floor(SKY_SHORT * settle_band(r) / abs(b - r))));
    }
    return vec4<f32>(r + (sc - r) / n, 0.0, b, n);
}

// The band about a mean `g` of binary samples (`pair_step`'s, with SKY_BAND_SIGMAS above).
fn settle_band(g: f32) -> f32 {
    return SKY_BAND_SIGMAS * sqrt(max(g * (1.0 - g), P_MIN * (1.0 - P_MIN)) / SKY_SHORT)
        + SKY_BAND_EPS;
}

// The in-band frames a converged mean counts before it settles, and the frames of the fade a
// released mean takes toward its short mean (rule 3's F, ADR-0046's FADE).
const SETTLE_F: f32 = 16.0;
// The consecutive out-of-band frames that release a settled mean: a 3-sigma run of this length
// at rest is about 1 in 10^10 a pixel, and a real change past the band shows within it.
const SETTLE_RELEASE: f32 = 4.0;

fn settle_on() -> bool {
    return (params.lighting & LIGHTING_NO_SETTLE) == 0u;
}

// One frame of the settle of a mean `g` at its cap, whose plain blend this frame is `blended`,
// its short mean `b` (`in_band`: within `settle_band(g)` of it), its state `c` (the frames past
// the cap the caller stores): from 0 to SETTLE_F the in-band frames counted (the mean blends,
// and an out-of-band frame restarts the count); from SETTLE_F the mean holds, with
// `c - SETTLE_F` the consecutive out-of-band frames; at SETTLE_RELEASE of them the mean is
// released, and from SETTLE_F + SETTLE_RELEASE it fades toward its short mean at `1 / SETTLE_F`
// a frame until the short mean is in the band again, where the count restarts. Returns the
// mean and the new state.
fn settle_at_cap(g: f32, blended: f32, b: f32, in_band: bool, c: f32) -> vec2<f32> {
    let fading = SETTLE_F + SETTLE_RELEASE;
    if (c >= fading) {
        if (in_band) {
            return vec2<f32>(blended, 0.0);
        }
        return vec2<f32>(g + (b - g) / SETTLE_F, fading);
    }
    if (c >= SETTLE_F) {
        if (in_band) {
            return vec2<f32>(g, SETTLE_F);
        }
        if (c + 1.0 >= fading) {
            return vec2<f32>(g + (b - g) / SETTLE_F, fading);
        }
        return vec2<f32>(g, c + 1.0);
    }
    return vec2<f32>(blended, select(0.0, c + 1.0, in_band));
}

// A previous pair tap for a pixel of kind `own_key` (its pair carries `key_settle`'s; a tap's
// kind is the sign of its z): a tap of the other kind reads as its long means at age `cap`, so
// no blend mixes a key short mean with a sky one or a settle state with a plain age.
fn pair_tap(t: vec4<f32>, own_key: bool, cap: f32) -> vec4<f32> {
    let tap_key = t.z < 0.0;
    if (tap_key == own_key) {
        return t;
    }
    if (own_key) {
        return vec4<f32>(t.x, t.y, -1.0 - t.y, cap);
    }
    return vec4<f32>(t.x, t.y, t.x, cap);
}

// The key mean with the settle at a stored face where no sky ray was cast (its pair the baked
// seed at age `cap_w`), after sample `s` on history `hp` (`pair_tap`'s), the direct part's age
// `age` (read, never written). The key samples' short mean rides in z as `-1 - b` (negative:
// the tap's kind), the settle's state in w as `cap_w + c`. Below the key cap it is `key_mean`;
// at the cap it settles (`settle_at_cap`). Returns the mean, the z and the w.
fn key_settle(s: f32, hp: vec4<f32>, age: f32, reset: bool, cap_w: f32) -> vec3<f32> {
    if (reset || (params.lighting & LIGHTING_SKY_CHANGED) != 0u) {
        return vec3<f32>(s, -1.0 - s, cap_w);
    }
    let kc = f32(max(sky.key_history, 1u));
    let g = hp.y;
    let n = clamp(age + 1.0, 1.0, kc);
    let hb = clamp(-1.0 - hp.z, 0.0, 1.0);
    let b = hb + (s - hb) / min(n, SKY_SHORT);
    let blended = g + (s - g) / n;
    if (n < kc) {
        return vec3<f32>(blended, -1.0 - b, cap_w);
    }
    let c = max(floor(hp.w - cap_w + 0.5), 0.0);
    let st = settle_at_cap(g, blended, b, abs(b - g) <= settle_band(g), c);
    return vec3<f32>(st.x, -1.0 - b, cap_w + st.y);
}

// The key light's visibility mean after sample `s` on history `g`, which holds `age` samples
// (the direct part's history age; `reset`: the history is not taken): over `age + 1` samples,
// as the direct part blends, capped at `key_history`.
fn key_mean(s: f32, g: f32, age: f32, reset: bool) -> f32 {
    if (reset || (params.lighting & LIGHTING_SKY_CHANGED) != 0u) {
        return s;
    }
    let n = clamp(age + 1.0, 1.0, f32(max(sky.key_history, 1u)));
    return g + (s - g) / n;
}

// The no-ray value's fade (ADR-0076 rule 3, one fade of SETTLE_F frames for every source): a
// pixel where no sky ray was cast shows its source's visibility `t` (a record, its chunk's
// summary, the column) through its pair's x. Where its history `hp` is taken (`accepted`) and
// x differs from `t` by more than FADE_EPS, x ramps to `t` linearly over SETTLE_F frames, the
// frames left in the pair's w past the cap `cap`; a target that moves mid-ramp is reached by
// the same frame. A reset pixel shows `t` at once. Returns x and w.
fn no_ray_fade(t: f32, hp: vec4<f32>, accepted: bool, cap: f32) -> vec2<f32> {
    if (!accepted || (params.lighting & LIGHTING_HOOK_NO_FADE) != 0u) {
        return vec2<f32>(t, cap);
    }
    let left = clamp(floor(hp.w - cap + 0.5), 0.0, SETTLE_F);
    let x = clamp(hp.x, 0.0, 1.0);
    if (left < 1.0) {
        if (abs(t - x) <= FADE_EPS) {
            return vec2<f32>(t, cap);
        }
        return vec2<f32>(x + (t - x) / SETTLE_F, cap + SETTLE_F - 1.0);
    }
    return vec2<f32>(x + (t - x) / left, cap + left - 1.0);
}

// A no-ray value's change under this is not faded: a stored byte's rounding.
const FADE_EPS: f32 = 1.0 / 255.0;
