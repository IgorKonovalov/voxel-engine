// The visibility pair (ADR-0070) of the temporal pass: its running means, the no-ray value's
// fade, the sky term's source words, a landing's switch of source's fade and the dig hold's
// box tests. Compiled after `sky.wgsl` and `edits.wgsl` and before `temporal.wgsl`, whose
// uniforms (`params`, `sky`), bindings (`store`, `edits`) and helpers (`above_column`, the
// store's readers) it uses.

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

// A reset pair's seed with no record: the column map's answer (ADR-0070's sky rule) at hit `p`
// lifted SEED_LIFT voxels along `n` into the air the face looks into, 1 above its column's
// top of solid, else 0.
fn column_estimate(p: vec3<f32>, n: vec3<f32>) -> f32 {
    return select(0.0, 1.0, above_column(p + n * SEED_LIFT));
}

// The far estimate where no record answers (ADR-0084, `Lighting::key_cover`), its source
// (KEY_SRC_HORIZON) in y: the column horizon at hit `p` with face normal `nf`
// (`key_horizon_at`), or, where the light pass read a near cover answer (`near`, from
// `key_cover_near`, already the larger of the horizon and its ray; -1 where it read none), that
// answer.
fn key_cover_far(p: vec3<f32>, nf: vec3<f32>, near: f32) -> vec2<f32> {
    if (near >= 0.0) {
        return vec2<f32>(near, f32(KEY_SRC_HORIZON));
    }
    return vec2<f32>(key_horizon_at(p, nf), f32(KEY_SRC_HORIZON));
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

// The sky term's source at a hit, a reading's word (COUNT_STORE only), added to the moments'
// w as `source / 16` (`source_fraction`): every reader of that w compares it against 0.5 or
// 1.5, which a fraction under 0.5 leaves as it was. SOURCE_NONE: no store face, an entity or
// the sky off; RECORD: the face's own cell holds a fresh record (no ray); RECORD_STALE: it
// holds one marked stale (the ray cast); OWN_SUMMARY and COLUMN: no record, the prior's value
// from the chunk's own summary or the column held (`store_sky_fallback`; no ray); NO_VALUE: no
// record and no prior's value (the ray cast); MISMATCH: a prior's value where the ray was cast.
const SOURCE_NONE: u32 = 0u;
const SOURCE_RECORD: u32 = 1u;
const SOURCE_RECORD_STALE: u32 = 2u;
const SOURCE_OWN_SUMMARY: u32 = 3u;
const SOURCE_COLUMN: u32 = 4u;
const SOURCE_NO_VALUE: u32 = 5u;
const SOURCE_MISMATCH: u32 = 6u;

// The source word of face `face` of voxel `v`: `record` its own cell holds a record, `chained`
// the chain gave a value, `ray` the light pass cast the sky ray there.
fn sky_source(head: StoreHead, v: vec3<i32>, face: u32, record: bool, chained: bool, ray: bool) -> u32 {
    if (record) {
        return select(SOURCE_RECORD, SOURCE_RECORD_STALE, ray);
    }
    if (chained) {
        if (ray) {
            return SOURCE_MISMATCH;
        }
        return select(SOURCE_COLUMN, SOURCE_OWN_SUMMARY,
            store_summary_sky(head, v >> vec3<u32>(6u), face).y > 0.0);
    }
    return SOURCE_NO_VALUE;
}

fn source_fraction(source: u32, on_entity: bool) -> f32 {
    if (!COUNT_STORE || on_entity) {
        return 0.0;
    }
    return f32(source) / 16.0;
}

// A landing's switch of source fades (`Params::history` bit 4, `Lighting::source_fade`; ADR-0076
// rule 3): a face whose sky term comes from its own cell's record with no ray, at a pixel that
// takes no light history, shows `term` (the record's baked term) mixed from what it showed
// before its chunk held a block, the column's answer at the hit lifted along `n`
// (`column_estimate`) times the sky's irradiance about the shading normal, by `(d + 1) /
// FADE_F`, `d` the frames since the chunk's first block landed (its block's word at
// STORE_LANDED_AT, that frame plus one; 0 none); `term` alone from FADE_F frames on, with no
// word, or with the bit clear.
fn source_faded(term: vec3<f32>, head: StoreHead, v: vec3<i32>, rec: HitRecord, n: vec3<f32>) -> vec3<f32> {
    if ((params.history & 16u) == 0u) {
        return term;
    }
    let block = store_table(head, v >> vec3<u32>(6u)) & ~TABLE_COARSE;
    if (block == 0u) {
        return term;
    }
    let w = store[block + STORE_LANDED_AT];
    let d = params.frame - (w - 1u);
    if (w == 0u || d >= u32(FADE_F) - 1u) {
        return term;
    }
    let before = column_estimate(rec.p, n) * sky_open_by_normal(rec.n);
    return mix(before, term, f32(d + 1u) / FADE_F);
}

// A dig holds its light (`Params::history` bit 5, `Lighting::dig_hold`; ADR-0082 with no light
// history). Whether `p` lies in a held dig box dilated by one voxel (as `in_edit`): the edit
// boxes the encode keeps after the kill boxes from the edit's frame until the live queue holds
// no dig and a fade's frames more. There a pixel with no light history reprojects the sky's
// visibility pair alone (`pair_step` over the cast rays, then `no_ray_fade` at the landing),
// as a pixel with the history does; its other parts keep none. Under bit 7 (`DigHold::On`) no
// box is held here: the store holds the box's value as a record (`bake::held`).
fn in_dig_hold(p: vec3<f32>) -> bool {
    if ((params.history & 32u) == 0u || (params.history & 128u) != 0u) {
        return false;
    }
    for (var i = edits.count; i < edits.count + edits.held; i++) {
        let lo = vec3<f32>(edits.boxes[2u * i].xyz - 1);
        let hi = vec3<f32>(edits.boxes[2u * i + 1u].xyz + 2);
        if (all(p >= lo) && all(p <= hi)) {
            return true;
        }
    }
    return false;
}

// Whether voxel `v` lies in a held dig box (`Lighting::dig_hold`'s boxes after the frame's kill
// boxes, edits.wgsl) dilated by one voxel, whatever the hold's bits: the voxel test the CPU's
// held record splits a cell's faces by (`bake::held`), so the faces read here are its dig faces.
fn in_held_box(v: vec3<i32>) -> bool {
    for (var i = edits.count; i < edits.count + edits.held; i++) {
        if (all(v >= edits.boxes[2u * i].xyz - 1) && all(v <= edits.boxes[2u * i + 1u].xyz + 1)) {
            return true;
        }
    }
    return false;
}

// Face `face` of voxel `v`'s held dig value (ADR-0082, `DigEstimate::Depth`) in x, with a 1 in
// y: where the voxel lies in a held dig box, its own cell's record holds no KEY_BAKED, no
// SKY_STALE and the first key word's bit `face` (`bake::DIG_HELD`), the direction's sky byte,
// which no other read takes (the record's mask lacks the direction or it holds no VIS_BAKED,
// so `store_sky_cell` reads none). 0 in y for none. Read for the pixel's own cell alone, where
// `store_sky_visibility` finds no direction; no 2 x 2 tap reads it.
fn store_dig_held(head: StoreHead, v: vec3<i32>, face: u32) -> vec2<f32> {
    if (edits.held == 0u || !in_held_box(v)) {
        return vec2<f32>(0.0);
    }
    let rec = store_cell_record(head, v >> vec3<u32>(head.shift));
    if (rec == 0u || (store[rec + KEY_WORD + 1u] & KEY_BAKED) != 0u
        || ((store[rec + KEY_WORD] >> face) & 1u) == 0u
        || (store[rec + VIS_WORD + 1u] & SKY_STALE) != 0u) {
        return vec2<f32>(0.0);
    }
    let byte = (store[rec + VIS_WORD + face / 4u] >> (8u * (face % 4u))) & 255u;
    return vec2<f32>(f32(byte) / 255.0, 1.0);
}

// Whether a stale ring face's pair holds its record's estimate at the cap under the dig hold:
// the bit set, the pixel with no light history (`hist`) and outside a held box. Its term is
// then the record's baked term (temporal.wgsl's stale-ring path), the one it showed before the
// dig marked the record stale, until the re-bake lands.
fn dig_ring_held(hist: bool, p: vec3<f32>) -> bool {
    return (params.history & 32u) != 0u && !hist && !in_dig_hold(p);
}

// Whether hit `rec` of lighting pixel `gid` reads the light passes' history: `Params::history`
// bit 0, and under bit 2 (`Lighting::light_history_far`, a spike arm) only where the far
// normal's weight is 1.
fn light_history_at(gid: vec2<u32>, rec: HitRecord) -> bool {
    if ((params.history & 1u) == 0u) {
        return false;
    }
    return (params.history & 4u) == 0u
        || far_normal_weight(params.views[view_of(full_px(gid))], rec.t) >= 1.0;
}
