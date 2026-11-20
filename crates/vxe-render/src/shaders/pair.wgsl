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
// as the direct part blends, capped at `key_history`.
fn key_mean(s: f32, g: f32, age: f32, reset: bool) -> f32 {
    if (reset || (params.lighting & LIGHTING_SKY_CHANGED) != 0u) {
        return s;
    }
    let n = clamp(age + 1.0, 1.0, f32(max(sky.key_history, 1u)));
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
