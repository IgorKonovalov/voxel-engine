// The temporal pass: accumulates each pixel's irradiance and its moments over frames by
// reprojecting the hit through the previous frame's views (the world does not move; only the
// views, the edits and the entities do), and kills history where the world was edited.
// Compiled after the walk's modules and entity.wgsl, whose bindings (group 0), hit-record
// helpers and entity transforms it uses.
//
// Bindings, group 2:
//   0  texture_2d<f32>   hit_pos_in      this frame's hit positions
//   1  texture_2d<u32>   hit_info_in     this frame's hit records
//   2  texture_2d<f32>   prev_pos_in     last frame's hit positions
//   3  texture_2d<u32>   prev_info_in    last frame's hit records
//   4  texture_2d<f32>   irradiance_in   this frame's irradiance from the light pass (a: 1
//                                        at a fresh pixel)
//   5  texture_2d<f32>   prev_accum_in   last frame's accumulated irradiance
//   6  texture_2d<f32>   prev_moments_in last frame's moments: luminance, its square, age
//   7  storage texture   accum_out       rgba16float: the accumulated irradiance, the
//                                        emissive part in rgb; in a 1 at a fresh pixel, else 0
//                                        (the upscalers' `fresh_lands`)
//   8  storage texture   moments_out     rgba16float: first and second moments of the
//                                        filtered irradiance's luminance (the direct part and a
//                                        fallback hit's emissive part), then the age: frames of
//                                        history reused, 0 after a reset; then 1 where the
//                                        emissive part is the store's (the a-trous passes it
//                                        through), 2 where the pair's sky term rides with it,
//                                        else 0; with COUNT_STORE plus the sky term's source
//                                        word over 16 (`sky_source`)
//   9  uniform           edits           the frame's edit boxes (edits.wgsl's `Edits`)
//  10  texture_2d<f32>   strata_in       the gradient pass's strata (gradient.wgsl), read in
//                                        the gradient mode (Params.lighting bit 3): the
//                                        direct part's blend weight rises to twice the lambda averaged
//                                        over the 3 x 3 strata around the reprojected pixel's
//                                        (clamped to 1), and the age falls by that fraction
//  11  storage, read      store           the store (ADR-0046, `bake::Store`)
//  12  storage           counters        nine words, written only where the pipeline's
//                                        COUNT_STORE is true: word 0 counts this frame's
//                                        cube-face hits with no brick record value
//                                        (`store_missing`), word 1 those that read one, word 2
//                                        those whose voxel's chunk holds a coarse block
//                                        (TABLE_COARSE); of word 0's, word 3 those the
//                                        missing-light chain gave a value, word 4 those it left
//                                        black (every one of them under `black`); word 5 the
//                                        hits the history test ran on, and of those whose
//                                        history reset, the nearest previous pixel's first
//                                        failing test: word 6 the material, word 7 the
//                                        normal, word 8 any other (`count_rejection`)
//  13  texture_2d<f32>   direct_in       this frame's direct part from the light pass
//  14  texture_2d<f32>   prev_direct_in  last frame's accumulated direct part
//  15  storage texture   direct_out      rgba16float: the accumulated direct part, the list's
//                                        rgb and its luminance in a
//  16  storage, read      sky_e           `E_sky`, the sky's cosine-weighted irradiance about
//                                        each axis in face order (`sky_irradiance.wgsl`), xyz
//  17  texture_2d<f32>   vis_in          this frame's visibility samples from the light pass
//                                        (shade.wgsl's `vis_out`: the sky rays' share in r, 1 in
//                                        a where they were cast; the key light's visibility in
//                                        g, 1 in b where it falls on the face)
//  18  texture_2d<f32>   prev_pair_in    last frame's visibility pair
//  19  storage texture   pair_out        rgba16float: the visibility pair (ADR-0070): the sky
//                                        visibility's running mean in r, the key light's in g
//                                        (0 where it falls on no face), the sky's short mean in
//                                        b, the pair's age in a (the samples its mean holds, 0
//                                        where it holds none; past the cap, the settle's count,
//                                        `pair_step`); where no ray was cast, the key samples'
//                                        short mean as `-1 - b` in b and the cap plus the key
//                                        mean's settle count in a (`key_settle`)
//
// The sky's term (ADR-0070) is a visibility times the sky's irradiance above the horizon about
// the shading normal now (`sky_open_by_normal`). Where the light pass cast no sky ray (a cube
// face whose own cell holds a fresh baked visibility, shade.wgsl's `sky_baked`), the
// visibility is the brick record's for the face's direction, read bilinear as the emissive
// value , with no screen history, and the pair is seeded with it (`seed`).
// Where the rays were cast, it is the visibility pair's running mean (`pair_step`): reprojected
// with the direct part's taps and reset where the direct part resets and on the sky-change
// bit, a reset pair seeded at age SEED_AGE from the best world-space estimate (the face's
// record visibility, a stale one included, else the column map's answer at the hit lifted into
// the face's air, `column_estimate`) and this frame's sample left out, a sample clamped at FIREFLY_K times the larger of the long and short means once the
// age passes SKY_SHORT, and the age falling toward SKY_FALL_MIN where the short mean leaves
// the long mean's band. At a cube face whose own cell holds a record (a stale ring brick's,
// the ray cast at it until its re-bake lands) the pair's mean is coloured by the baked term's
// form (`sky_baked_term`: the smoothed normal's mix and the bounce floor) with the pair's mean
// as the face's own visibility, so the switch at the stale bit is between two estimates of one
// term (ADR-0070), and where the pair is at its cap it rides with the stored part around the
// filter as the baked term does (the moments' w 2; atrous.wgsl adds no pair term there);
// elsewhere the term is `sky_pair_term`, which the a-trous and composite passes add from the
// pair. The ray's term joins no screen history: its luminance joins the
// moments, and the filter's input (atrous.wgsl `variance`) or, with the filter off, the
// composite adds it from this frame's pair (`sky_pair_term`, sky.wgsl).
// The key light's term is `key_light_at(p, n)` times its visibility's running mean in the
// pair's g (`key_mean`): over the direct part's history age capped at `key_history`, reset
// where the direct part's history resets (no accepted taps, an edit box, a fresh pixel) and on
// the sky-change bit, but not on a change of the list of lights; no clamp (the term is
// bounded). It joins the direct part at the direct part's blend weight. At a stored face where
// no sky ray was cast it settles at its cap as the pair does (`key_settle`, `settle_at_cap`).
//
// A cube face's emissive part (`reads_store`) is its brick record's value for its direction
// (`store_brick_value`; with Params.lighting bit 14, where the shading normal is not the face
// normal, the mix by the normal of `store_value_by_normal`), times the dominant material's
// emission scale now and this frame's exposure, with no screen history, the same value at every
// pixel of the face. With no value, the flags word's missing-light mode (ADR-0053,
// `store_fallback`) reads a chain of summaries, nearest first: the chunk's own block's, the
// mean of its six face neighbours', the level-1 then level-2 parent record; 0 where none holds
// the direction, and 0 at once under `black`.
// An entity hit under LIGHTING_ENTITY_WORLD (`entity_reads_store`) reads the store the same
// way, as if it were a face of the shading normal's dominant axis `a`, pointing the normal's
// way: the voxel half a voxel behind the hit along that axis (in the entity, so in the world's
// air beside the faces around it), the brick value of its cell's faces of that direction, else
// of the first of ENTITY_STORE_CELLS cells behind it against that direction that holds one (the
// nearest world face facing the same way), else the missing-light chain's at the first voxel, then mixed by the shading normal (`store_value_by_normal`) whatever
// bit 14 says, and it counts in no store counter.
// Any other hit (no face: the field look, a translucent hit, a ray from inside rock) is a
// fallback: its own emissive samples' running mean over the pixel's age, `1 / (age + 1)`, until
// the age reaches FALLBACK_CAP, then held, so a still fallback pixel holds still; it restarts
// where the pixel resets (no accepted history, no history, an edit box), and follows an
// emission-scale change at ALPHA.
//
// A hit is reprojected from where it was in the frame before (`prev_world`: a moved entity
// part's voxel through the part's transform of the frame before, any other hit where it is)
// through the previous frame's view of the same index to a fractional pixel as if both
// frames' rays went through their pixels' centres (`reproject_centred`: a still camera maps
// each pixel onto itself whatever the jitter); each of its four bilinear taps is accepted when
// that pixel hit the same thing (both the world, or both the same entity instance:
// `same_thing`), with the same material, a normal within HISTORY_COS of this one, and a
// position within `HISTORY_DIST + HISTORY_DIST_PER_T t` voxels of that previous point. The
// accepted taps' history, moments and age are blended by their bilinear weights; with none
// accepted, the accepted pixel of the 3 x 3 around the nearest whose position lies nearest
// this hit's is taken alone, unless the hit is the world's and the nearest previous pixel
// showed an entity (the surface was hidden behind it); with none of those either, with no
// history (`Params.history` bit 0 clear), or with the hit inside an edit box dilated by
// one voxel, the pixel resets: age 0, accumulation this frame's sample. Otherwise the age is
// the taps' age plus one, capped at AGE_MAX. The direct part (rgba, the same blend on every
// channel) and the moments blend the new sample in at `max(ALPHA, 1 / (age + 1))`, a
// fallback's emissive part as above. On the frame LIGHTING_LIGHTS_CHANGED says the list of
// lights changed, or LIGHTING_SKY_CHANGED that the sky jumped (its key light turned by more
// than its radius, switched body or changed illuminance, or the sky was set or cleared; a
// drift sets neither), the direct part takes this frame's sample as it is: the light pass's
// estimate of each light is exact up to its shadow ray, so a light switched off, moved,
// recoloured or removed leaves nothing behind (and the moments take this frame's). The same
// rule holds for the direct part per pixel at a fresh pixel (light.wgsl: a shadow ray crossed
// a moving entity's box, or the hit is a moved part's), so a moving shadow or a moving part's
// lit face leaves no stale direct light; its moments blend as anywhere else, so the filter
// keeps a variance there.
//
// A cube face's pixel with no light of the list shaded (LIGHTING_DIRECT clear: the lamp off and
// the list empty; no cutaway, not the steps view) skips the reprojection: its emissive part is
// the store's and its direct part, moments and their history are zero (the frame the last
// light went took its sample, zero, at once, the list having changed), so it
// writes the store's value and zero moments. Its age is carried on the pixel: last frame's age at
// the same pixel plus one, 0 where the pixel would reset; a still camera maps each pixel onto
// itself, so there it is the reprojected age.
//
// Every value is pre-exposed (ADR-0033): the history read from last frame is rescaled by
// `exposure_ratio()` (the second moment by its square) before it blends, the accumulation's
// store is clamped at HDR_MAX, and the moments are of the luminance clamped at MOMENT_LUM_MAX
// and stored at most MOMENT_LUM_MAX and its square.

// Cells an entity hit's store read steps back for a face of its direction.
const ENTITY_STORE_CELLS: u32 = 4u;

@group(2) @binding(0) var hit_pos_in: texture_2d<f32>;
@group(2) @binding(1) var hit_info_in: texture_2d<u32>;
@group(2) @binding(2) var prev_pos_in: texture_2d<f32>;
@group(2) @binding(3) var prev_info_in: texture_2d<u32>;
@group(2) @binding(4) var irradiance_in: texture_2d<f32>;
@group(2) @binding(5) var prev_accum_in: texture_2d<f32>;
@group(2) @binding(6) var prev_moments_in: texture_2d<f32>;
@group(2) @binding(7) var accum_out: texture_storage_2d<rgba16float, write>;
@group(2) @binding(8) var moments_out: texture_storage_2d<rgba16float, write>;

@group(2) @binding(9) var<uniform> edits: Edits;
@group(2) @binding(10) var strata_in: texture_2d<f32>;
@group(2) @binding(11) var<storage, read> store: array<u32>;
@group(2) @binding(12) var<storage, read_write> counters: array<atomic<u32>, 9>;
@group(2) @binding(13) var direct_in: texture_2d<f32>;
@group(2) @binding(14) var prev_direct_in: texture_2d<f32>;
@group(2) @binding(15) var direct_out: texture_storage_2d<rgba16float, write>;
@group(2) @binding(16) var<storage, read> sky_e: array<vec4<f32>, 6>;
@group(2) @binding(17) var vis_in: texture_2d<f32>;
@group(2) @binding(18) var prev_pair_in: texture_2d<f32>;
@group(2) @binding(19) var pair_out: texture_storage_2d<rgba16float, write>;

// Whether this pipeline counts the store's reads into `counters` (the HUD's and
// `--read-missing`'s pipeline); off, `counters` is never written.
override COUNT_STORE: bool = false;

// The store's words and a record's layout: hitrec.wgsl's STORE_ORIGIN to TABLE_COARSE.

// The value of an rgb9e5 word: three 9-bit mantissas over a shared exponent, biased by 15 + 9.
fn rgb9e5_decode(w: u32) -> vec3<f32> {
    let m = vec3<u32>(w & 511u, (w >> 9u) & 511u, (w >> 18u) & 511u);
    return vec3<f32>(m) * ldexp(1.0, i32(w >> 27u) - 24);
}

// The store's window in chunks and its brick level's cell as a shift, read once a pixel.
struct StoreHead {
    origin: vec3<i32>,
    size: vec3<i32>,
    // 2 (4 voxels, flags bit 1) or 3 (the brick).
    shift: u32,
};

fn store_head() -> StoreHead {
    let origin = vec3<i32>(
        i32(store[STORE_ORIGIN]),
        i32(store[STORE_ORIGIN + 1u]),
        i32(store[STORE_ORIGIN + 2u]),
    );
    let size = vec3<i32>(
        i32(store[STORE_SIZE]),
        i32(store[STORE_SIZE + 1u]),
        i32(store[STORE_SIZE + 2u]),
    );
    return StoreHead(origin, size, select(3u, 2u, (store[STORE_FLAGS] & 2u) != 0u));
}

// Chunk `c`'s table word (world chunks): its block's first word with the TABLE_COARSE flag, 0 for
// none or outside the window.
fn store_table(head: StoreHead, c: vec3<i32>) -> u32 {
    let w = c - head.origin;
    if (any(w < vec3<i32>(0)) || any(w >= head.size)) {
        return 0u;
    }
    return store[STORE_TABLE + u32(w.x + head.size.x * (w.y + head.size.y * w.z))];
}

// Cell `c`'s (world cells of 1 << `head.shift` voxels) record's first word; 0 where its brick
// has no record. A brick's record is its cells' BRICK_WORDS records, `x + n (y + n z)` with n
// cells a side, just before its run.
fn store_cell_record(head: StoreHead, c: vec3<i32>) -> u32 {
    let per = 3u - head.shift;
    let b = c >> vec3<u32>(per);
    let block = store_table(head, b >> vec3<u32>(3u)) & ~TABLE_COARSE;
    if (block == 0u) {
        return 0u;
    }
    let s = b & vec3<i32>(7);
    let dir = store[block + u32(s.x + 8 * (s.y + 8 * s.z))];
    if (dir == 0u) {
        return 0u;
    }
    let n = 1 << per;
    let l = c & vec3<i32>(n - 1);
    let sub = u32(l.x + n * (l.y + n * l.z));
    let cells = u32(n * n * n);
    return block + (dir >> 11u) - BRICK_WORDS * cells + BRICK_WORDS * sub;
}

// Cell `c`'s value for direction `face` times its material's emission scale now in rgb, with a
// 1 in a; 0 where its brick has no record or the cell no face of that direction.
fn store_cell(head: StoreHead, c: vec3<i32>, face: u32) -> vec4<f32> {
    let rec = store_cell_record(head, c);
    if (rec == 0u) {
        return vec4<f32>(0.0);
    }
    let tail = store[rec + 6u];
    if (((tail >> (16u + face)) & 1u) == 0u) {
        return vec4<f32>(0.0);
    }
    let x = rgb9e5_decode(store[rec + face]);
    return vec4<f32>(x * emission_scale[tail & 0xFFFFu], 1.0);
}

// Cell `c`'s sky visibility for direction `face` (ADR-0070) in x, with a 1 in y; 0 where its
// record lacks the direction or was baked with no sky (VIS_BAKED clear).
fn store_sky_cell(head: StoreHead, c: vec3<i32>, face: u32) -> vec2<f32> {
    let rec = store_cell_record(head, c);
    if (rec == 0u || ((store[rec + 6u] >> (16u + face)) & 1u) == 0u
        || (store[rec + VIS_WORD + 1u] & VIS_BAKED) == 0u) {
        return vec2<f32>(0.0);
    }
    let byte = (store[rec + VIS_WORD + face / 4u] >> (8u * (face % 4u))) & 255u;
    return vec2<f32>(f32(byte) / 255.0, 1.0);
}

// Face `face` of voxel `v`'s sky visibility: the 2 x 2 cells' (`store_tap`) bilinear over
// those that hold it, as `store_brick_value`; the voxel's own cell's where none does with
// weight. 0 in y where its own cell holds none: the light pass casts the sky ray there
// (shade.wgsl's `sky_baked`, the same test).
fn store_sky_visibility(head: StoreHead, v: vec3<i32>, face: u32) -> vec2<f32> {
    let own = store_sky_cell(head, v >> vec3<u32>(head.shift), face);
    if (own.y == 0.0) {
        return own;
    }
    var sum = 0.0;
    var weight = 0.0;
    for (var i = 0; i < 4; i++) {
        let tap = store_tap(v, face, head.shift, i);
        let r = store_sky_cell(head, tap.cell, face);
        sum += r.x * (tap.w * r.y);
        weight += tap.w * r.y;
    }
    if (weight > 0.0) {
        return vec2<f32>(sum / weight, 1.0);
    }
    return own;
}

// Chunk `c`'s summary's sky visibility for `face` in x, with a 1 in y; 0 where the chunk holds
// no block, or its summary lacks the direction or was baked with no sky.
fn store_summary_sky(head: StoreHead, c: vec3<i32>, face: u32) -> vec2<f32> {
    let block = store_table(head, c) & ~TABLE_COARSE;
    if (block == 0u) {
        return vec2<f32>(0.0);
    }
    let rec = block + STORE_SUMMARY_AT;
    if (((store[rec + 6u] >> (16u + face)) & 1u) == 0u
        || (store[rec + VIS_WORD + 1u] & VIS_BAKED) == 0u
        || (store[rec + VIS_WORD + 1u] & SKY_STALE) != 0u) {
        return vec2<f32>(0.0);
    }
    let byte = (store[rec + VIS_WORD + face / 4u] >> (8u * (face % 4u))) & 255u;
    return vec2<f32>(f32(byte) / 255.0, 1.0);
}

// The no-record prior's sky visibility for face `face` of voxel `v` (hit `p`, lifted along `n`
// for the column), whose own cell holds none: the chunk's own summary's while no dig job is
// queued for the chunk (its summary not SKY_STALE); else, with LIGHTING_HOOK_COLUMN_PRIOR, the
// column's answer held (`column_estimate`); 0 in y for none (the light pass casts the sky ray),
// and under the `black` and `parent` modes. A face neighbour's summary does not answer the sky.
// shade.wgsl's `sky_chained` is the same test.
fn store_sky_fallback(head: StoreHead, v: vec3<i32>, face: u32, p: vec3<f32>, n: vec3<f32>) -> vec2<f32> {
    let mode = store_mode();
    if (mode == 0u || mode == MISSING_PARENT) {
        return vec2<f32>(0.0);
    }
    let own = store_summary_sky(head, v >> vec3<u32>(6u), face);
    if (own.y > 0.0 || (params.lighting & LIGHTING_HOOK_COLUMN_PRIOR) == 0u) {
        return own;
    }
    return vec2<f32>(column_estimate(p, n), 1.0);
}

// `sky_open_irradiance_in` and `sky_open_by_normal_in` (sky.wgsl) over this pass's `sky_e`.
fn sky_open_irradiance(dir: u32) -> vec3<f32> {
    return sky_open_irradiance_in(sky_e, dir);
}

fn sky_open_by_normal(n: vec3<f32>) -> vec3<f32> {
    return sky_open_by_normal_in(sky_e, n);
}

// A cube face's baked sky term (ADR-0070) at hit `rec` of voxel `v`, face `face` (face normal
// `nf`), whose visibility for its own direction is `vis`: `vis` times the sky's irradiance
// above the horizon about the shading normal, or under LIGHTING_SKY_SMOOTHED about the 27-count
// gradient's normal `g`, the mix over the directions `g` leans toward (`g_a^2` above 1e-4) of
// each direction's visibility (`store_sky_visibility`; the face's own is `vis`) times
// `sky_open_irradiance` of it, over the directions that hold one (the face's alone where none
// does). With `params.sky_bounce` `b` over 0 the visibility part is `(1 - b)` of that plus `b`
// times the face's cell's six visibilities' mean weighted `(1 + n.d) / 2` (`sky_cell_mean`)
// times the irradiance about the normal.
fn sky_baked_term(head: StoreHead, v: vec3<i32>, face: u32, vis: f32, rec: HitRecord, nf: vec3<f32>) -> vec3<f32> {
    var n = rec.n;
    var term = vis * sky_open_by_normal(n);
    if ((params.lighting & LIGHTING_SKY_SMOOTHED) != 0u && rec.axis >= 0) {
        n = gradient27_normal(Hit(true, v, rec.axis, rec.t, rec.material), nf);
        var sum = vec3<f32>(0.0);
        var weight = 0.0;
        for (var a = 0u; a < 3u; a++) {
            let w = n[a] * n[a];
            if (w < 1e-4) {
                continue;
            }
            let dir = 2u * a + select(0u, 1u, n[a] > 0.0);
            var r = vec2<f32>(vis, 1.0);
            if (dir != face) {
                r = store_sky_visibility(head, v, dir);
            }
            sum += r.x * sky_open_irradiance(dir) * (w * r.y);
            weight += w * r.y;
        }
        if (weight > 0.0) {
            term = sum / weight;
        }
    }
    let b = params.sky_bounce;
    if (b > 0.0) {
        term = (1.0 - b) * term + b * sky_cell_mean(head, v, n) * sky_open_by_normal(n);
    }
    return term;
}

// The six direction visibilities of voxel `v`'s own cell, weighted `(1 + n.d) / 2` about
// normal `n`, over the directions the cell holds; 0 where it holds none.
fn sky_cell_mean(head: StoreHead, v: vec3<i32>, n: vec3<f32>) -> f32 {
    let c = v >> vec3<u32>(head.shift);
    var sum = 0.0;
    var weight = 0.0;
    for (var f = 0u; f < 6u; f++) {
        let r = store_sky_cell(head, c, f);
        let d = select(-1.0, 1.0, (f & 1u) == 1u) * n[f / 2u];
        let w = 0.5 * (1.0 + d) * r.y;
        sum += r.x * w;
        weight += w;
    }
    return select(0.0, sum / weight, weight > 0.0);
}

// Face `face` of voxel `v` at the brick level: the direction values of the 2 x 2 cells around the
// voxel's centre in the face's plane (hitrec.wgsl's `store_tap`), bilinear over those that have
// the direction; its own cell's alone where none has it with weight. 0 in a where that has none
// either. hit.wgsl's `glass_store_value` is the same read over the hit pass's binding.
fn store_brick_value(head: StoreHead, v: vec3<i32>, face: u32) -> vec4<f32> {
    var sum = vec3<f32>(0.0);
    var weight = 0.0;
    for (var i = 0; i < 4; i++) {
        let tap = store_tap(v, face, head.shift, i);
        let r = store_cell(head, tap.cell, face);
        sum += r.xyz * (tap.w * r.w);
        weight += tap.w * r.w;
    }
    if (weight > 0.0) {
        return vec4<f32>(sum / weight, 1.0);
    }
    return store_cell(head, v >> vec3<u32>(head.shift), face);
}

// The flags word's missing-light mode, bits 2 and 3 (`bake::MissingLight::bits`): 0 black.
const MISSING_NEIGHBOUR: u32 = 1u;
const MISSING_PARENT: u32 = 2u;

fn store_mode() -> u32 {
    return (store[STORE_FLAGS] >> 2u) & 3u;
}

// The record at word `rec`'s value for direction `face` times its material's emission scale now
// in rgb, with a 1 in a; 0 where its mask lacks the direction.
fn store_record(rec: u32, face: u32) -> vec4<f32> {
    let tail = store[rec + 6u];
    if (((tail >> (16u + face)) & 1u) == 0u) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(rgb9e5_decode(store[rec + face]) * emission_scale[tail & 0xFFFFu], 1.0);
}

// Chunk `c`'s summary's value for `face`; 0 where the chunk holds no block.
fn store_summary(head: StoreHead, c: vec3<i32>, face: u32) -> vec4<f32> {
    let block = store_table(head, c) & ~TABLE_COARSE;
    if (block == 0u) {
        return vec4<f32>(0.0);
    }
    return store_record(block + STORE_SUMMARY_AT, face);
}

// The level-`level` (1 or 2) parent record over chunk `c` for `face` (`bake::ParentTable`): after
// the chunk table, level 1 over cells of 2 chunks a side, `(size + 1) / 2 + 1` cells a side from
// the window origin's cell, then level 2 over 4, `(size + 3) / 4 + 1`; cells world-aligned.
fn store_parent(head: StoreHead, c: vec3<i32>, level: u32, face: u32) -> vec4<f32> {
    let first = STORE_TABLE + u32(head.size.x * head.size.y * head.size.z);
    let s1 = ((head.size + vec3<i32>(1)) >> vec3<u32>(1u)) + vec3<i32>(1);
    var base = first;
    var side = s1;
    if (level == 2u) {
        base = first + u32(s1.x * s1.y * s1.z) * BRICK_WORDS;
        side = ((head.size + vec3<i32>(3)) >> vec3<u32>(2u)) + vec3<i32>(1);
    }
    let w = (c >> vec3<u32>(level)) - (head.origin >> vec3<u32>(level));
    if (any(w < vec3<i32>(0)) || any(w >= side)) {
        return vec4<f32>(0.0);
    }
    return store_record(base + u32(w.x + side.x * (w.y + side.y * w.z)) * BRICK_WORDS, face);
}

// The missing-light chain (ADR-0053) for face `face` of voxel `v`, which has no brick value:
// the first source holding the direction, nearest first, under the flags word's mode: the
// chunk's own summary; the mean of its six face neighbours' summaries that hold it (not under
// `parent`); the level-1 then level-2 parent record (not under `neighbour`). 0 in a for none,
// and at once under `black`. One value a chunk and direction, never per voxel.
fn store_fallback(head: StoreHead, v: vec3<i32>, face: u32) -> vec4<f32> {
    let mode = store_mode();
    if (mode == 0u) {
        return vec4<f32>(0.0);
    }
    let c = v >> vec3<u32>(6u);
    let own = store_summary(head, c, face);
    if (own.w > 0.0) {
        return own;
    }
    if (mode != MISSING_PARENT) {
        var sum = vec3<f32>(0.0);
        var n = 0.0;
        for (var f = 0u; f < 6u; f++) {
            // Face f's axis f / 2, toward -1 for an even face; built per component, since a
            // store through a runtime vector index is not portable.
            let sign = select(1, -1, (f & 1u) == 0u);
            let axis = f / 2u;
            let o = vec3<i32>(
                select(0, sign, axis == 0u),
                select(0, sign, axis == 1u),
                select(0, sign, axis == 2u),
            );
            let r = store_summary(head, c + o, face);
            sum += r.xyz * r.w;
            n += r.w;
        }
        if (n > 0.0) {
            return vec4<f32>(sum / n, 1.0);
        }
    }
    if (mode != MISSING_NEIGHBOUR) {
        let p1 = store_parent(head, c, 1u, face);
        if (p1.w > 0.0) {
            return p1;
        }
        return store_parent(head, c, 2u, face);
    }
    return vec4<f32>(0.0);
}

// The stored part by shading normal `n` (ADR-0049) at voxel `v` whose hit face `face` read
// `c`: the cell's direction values for the directions `n` leans toward, `2a + (n_a > 0)` on
// each axis `a` with `n_a^2` above 1e-4, mixed by `n_a^2` over those that hold a value and
// renormalised; `c` where none does. The hit face's term is `c` itself.
fn store_value_by_normal(head: StoreHead, v: vec3<i32>, face: u32, c: vec4<f32>, n: vec3<f32>) -> vec4<f32> {
    var sum = vec3<f32>(0.0);
    var weight = 0.0;
    for (var a = 0u; a < 3u; a++) {
        let w = n[a] * n[a];
        if (w < 1e-4) {
            continue;
        }
        let dir = 2u * a + select(0u, 1u, n[a] > 0.0);
        var r = c;
        if (dir != face) {
            r = store_brick_value(head, v, dir);
        }
        sum += r.xyz * (w * r.w);
        weight += w * r.w;
    }
    if (weight > 0.0) {
        return vec4<f32>(sum / weight, 1.0);
    }
    return c;
}

// The gradient's gain: lambda = |new - old| / max(new, old) reads 0.9 where light fell to a
// tenth, and a blend at 0.9 would keep a tenth of the old light; twice lambda, clamped, takes
// such a pixel to its new light at once.
const GRADIENT_GAIN: f32 = 2.0;
// Strata edge in pixels; gradient.wgsl's STRATUM.
const GRADIENT_STRATUM: f32 = 3.0;

// The gradient pass's lambdas at fractional pixel `q` of last frame, averaged over the 3 x 3
// strata around its stratum.
fn gradient_at(q: vec2<f32>) -> f32 {
    let s = vec2<i32>(floor(q / GRADIENT_STRATUM));
    let n = vec2<i32>(textureDimensions(strata_in));
    var sum = 0.0;
    var count = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let t = s + vec2<i32>(x, y);
            if (all(t >= vec2<i32>(0)) && all(t < n)) {
                sum += textureLoad(strata_in, t, 0).x;
                count += 1.0;
            }
        }
    }
    return sum / max(count, 1.0);
}

// The far normal's blend weight of this frame's hit where it is over one half (ADR-0042: a voxel
// smaller on screen than the blend's lower bound): set per pixel by `temporal` for a world
// cube-face hit, else 0. Over one half, the history test is the stair test (`history_agrees`).
var<private> stair_w: f32 = 0.0;

// The smoothed normal `g` of a cube-face hit whose stored shading normal `n` is the far normal's
// blend `normalize(mix(f, g, w))` toward it from face normal `f` (the hit's axis, signed as `n`
// leans): `w g = l n - (1 - w) f` with `l = |mix|` the root of
// `l^2 - 2 l (1 - w) (n.f) + (1 - w)^2 - w^2 = 0`, real for `w >= 1/2`; `n` itself at 1.
fn far_unmixed(rec: HitRecord, w: f32) -> vec3<f32> {
    if (w >= 1.0 || rec.axis < 0) {
        return rec.n;
    }
    let f = select(vec3<f32>(0.0), vec3<f32>(select(-1.0, 1.0, rec.n[rec.axis] >= 0.0)), axis_mask(rec.axis));
    let k = 1.0 - w;
    let nf = dot(rec.n, f);
    let l = k * nf + sqrt(max(k * k * nf * nf - k * k + w * w, 0.0));
    return normalize(l * rec.n - k * f);
}

// Whether view `a` is view `b` but for the jitter: the same origin, axes and frustum.
fn view_still(a: View, b: View) -> bool {
    return all(a.origin == b.origin) && all(a.right == b.right) && all(a.up == b.up)
        && all(a.fwd == b.fwd) && a.tan_left == b.tan_left && a.tan_right == b.tan_right
        && a.tan_up == b.tan_up && a.tan_down == b.tan_down;
}

// Whether previous hit `prev` agrees with this frame's hit `rec` (reprojected to `pp`) within
// `tol` voxels: a hit of the same thing, with the same material and a normal within
// HISTORY_COS, or with LIGHTING_HOOK_ANY_HISTORY whatever the material and normal. Where
// `stair_w` is over one half (the stair test) the smoothed normals (`far_unmixed`, both at this
// hit's weight) are compared within HISTORY_COS and the material is not: a tread and its
// riser under the jitter share the irradiance history, which is before albedo. With `own_still`
// (a world hit's own pixel at a still camera, `view_still`) neither is tested: the pixel's
// footprint is the same as the frame before's, and under the jitter its hit moves between the
// faces inside it (a trunk's side and its top, two faces of a corner), whose irradiance the
// footprint's history averages; the position test stays.
fn history_agrees(prev: HitRecord, rec: HitRecord, pp: vec3<f32>, tol: f32, own_still: bool) -> bool {
    if (!prev.found || !same_thing(prev, rec)) {
        return false;
    }
    if (own_still) {
        return true;
    }
    if (distance(prev.p, pp) > tol) {
        return false;
    }
    if ((params.lighting & LIGHTING_HOOK_ANY_HISTORY) != 0u) {
        return true;
    }
    if (stair_w > 0.5) {
        return dot(far_unmixed(prev, stair_w), far_unmixed(rec, stair_w)) >= HISTORY_COS;
    }
    return prev.material == rec.material && dot(prev.n, rec.n) >= HISTORY_COS;
}

// Counts a reset pixel's cause into `counters` (COUNT_STORE only): the first test previous
// hit `prev` fails, in the order material (word 6; not under the stair test), normal (word 7;
// the smoothed normals under it), anything else (word 8; every cause where `own_still`, which
// tests neither material nor normal).
fn count_rejection(prev: HitRecord, rec: HitRecord, own_still: bool) {
    if (own_still) {
        atomicAdd(&counters[8], 1u);
        return;
    }
    let stair = stair_w > 0.5;
    var cos = dot(prev.n, rec.n);
    if (stair) {
        cos = dot(far_unmixed(prev, stair_w), far_unmixed(rec, stair_w));
    }
    if (prev.found && same_thing(prev, rec) && !stair && prev.material != rec.material) {
        atomicAdd(&counters[6], 1u);
    } else if (prev.found && same_thing(prev, rec) && cos < HISTORY_COS) {
        atomicAdd(&counters[7], 1u);
    } else {
        atomicAdd(&counters[8], 1u);
    }
}

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

const ALPHA: f32 = 0.2;
// The age at which a fallback's emissive mean stops taking samples: 64 samples, its noise an
// eighth of one sample's, then held until the pixel resets.
const FALLBACK_CAP: f32 = 64.0;
// A held fallback's history is its heaviest tap's alone where that tap's bilinear weight is at
// least this: a still camera reprojects each pixel onto its own centre up to rounding, and the
// blend of four taps would re-round the stored half every frame.
const HELD_TAP_W: f32 = 0.999;
const AGE_MAX: f32 = 2047.0;
const HISTORY_COS: f32 = 0.8;
const HISTORY_DIST: f32 = 0.5;
const HISTORY_DIST_PER_T: f32 = 0.03;

// The visibility pair's functions are pair.wgsl's.

@compute @workgroup_size(8, 8, 1)
fn temporal(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.light_size.x || gid.y >= params.light_size.y) {
        return;
    }
    let px = vec2<i32>(gid.xy);
    let rec = unpack_hit(textureLoad(hit_pos_in, px, 0), textureLoad(hit_info_in, px, 0));
    // The emissive irradiance in rgb; the direct part in rgb, its luminance in a.
    var sample = textureLoad(irradiance_in, px, 0);
    var direct = textureLoad(direct_in, px, 0);
    // This frame's sky visibility sample, where the light pass cast the sky rays.
    let vis = textureLoad(vis_in, px, 0);
    let sky_cast = vis.a > 0.5;
    // The pair where no ray was cast: the baked visibility at age `sky_history`, or none.
    var seed = vec4<f32>(0.0);
    // A reset pair's seed where the ray was cast (`pair_step`): the face's record visibility,
    // a stale ring brick's included, else the column test at the hit lifted into the air the
    // face looks into (`column_estimate`). The chain is not consulted.
    var estimate = -1.0;
    // The normal the column test lifts the hit along: the store face's, else the shading one.
    var lift_n = rec.n;
    // A cube face whose own cell holds a sky record: its voxel, face and face normal, for the
    // cast path's term in the baked term's form.
    var rec_face = false;
    var rec_voxel = vec3<i32>(0);
    var rec_dir = 0u;
    var rec_nf = vec3<f32>(0.0);
    // A store face where no sky ray was cast whose sky term comes from the store or the prior
    // (`no_ray`): its voxel, face and face normal, for the term after the fade.
    var no_ray = false;
    var nr_voxel = vec3<i32>(0);
    var nr_face = 0u;
    var nr_nf = vec3<f32>(0.0);
    let fresh = sample.w > 0.5 && view_mode() != VIEW_STEPS;
    sample.w = 0.0;
    var stored = false;
    // The sky term's source (COUNT_STORE only; `SOURCE_*`), carried in the moments' w.
    var source = SOURCE_NONE;
    let on_entity = entity_reads_store(rec);
    if (reads_store(rec) || on_entity) {
        let ray = pixel_ray(full_px(gid.xy));
        var nf = vec3<f32>(0.0);
        var axis = rec.axis;
        if (on_entity) {
            let an = abs(rec.n);
            axis = select(select(2, 1, an.y >= an.z), 0, an.x >= an.y && an.x >= an.z);
            nf = select(vec3<f32>(0.0), sign(rec.n), axis_mask(axis));
        } else {
            nf = shadow_normal_of(rec, ray.d);
        }
        lift_n = nf;
        var voxel = store_voxel(rec.p, nf);
        let face = store_face(axis, nf);
        let head = store_head();
        var c = store_brick_value(head, voxel, face);
        // An entity hit whose cell holds no face of its direction looks for the nearest that
        // does behind it, a cell at a time against that direction, before the chain.
        if (on_entity) {
            let step = vec3<i32>(nf) * (1 << head.shift);
            for (var k = 0u; k < ENTITY_STORE_CELLS && c.w == 0.0; k++) {
                voxel -= step;
                c = store_brick_value(head, voxel, face);
            }
            if (c.w == 0.0) {
                voxel = store_voxel(rec.p, nf);
            }
        }
        var value = c;
        if (c.w == 0.0) {
            value = store_fallback(head, voxel, face);
        }
        if (COUNT_STORE && !on_entity) {
            if (c.w > 0.0) {
                atomicAdd(&counters[1], 1u);
            }
            if (c.w == 0.0) {
                atomicAdd(&counters[0], 1u);
                if (value.w > 0.0) {
                    atomicAdd(&counters[3], 1u);
                } else {
                    atomicAdd(&counters[4], 1u);
                }
            } else if ((store_table(head, voxel >> vec3<u32>(6u)) & TABLE_COARSE) != 0u) {
                atomicAdd(&counters[2], 1u);
            }
        }
        stored = true;
        // The hit face's term is its brick value or, with none, the missing-light chain's.
        var e = value;
        if (((params.lighting & LIGHTING_STORE_BY_NORMAL) != 0u || on_entity) && any(rec.n != nf)) {
            e = store_value_by_normal(head, voxel, face, value, rec.n);
        }
        // The sky's term (ADR-0070) where no ray was cast: a face's baked visibility, else the
        // missing-light chain's, coloured by the sky now. An entity keeps its sky rays.
        var sky_term = vec3<f32>(0.0);
        if (!on_entity && sky_on()) {
            let rv = store_sky_visibility(head, store_voxel(rec.p, nf), face);
            if (rv.y > 0.0) {
                estimate = rv.x;
                rec_face = true;
                rec_voxel = store_voxel(rec.p, nf);
                rec_dir = face;
                rec_nf = nf;
            }
            var sv = rv;
            if (sv.y == 0.0) {
                sv = store_sky_fallback(head, store_voxel(rec.p, nf), face, rec.p, nf);
            }
            if (COUNT_STORE) {
                source = sky_source(head, store_voxel(rec.p, nf), face, rv.y > 0.0, sv.y > 0.0, sky_cast);
            }
            if (sv.y > 0.0 && !sky_cast) {
                sky_term = sky_baked_term(head, store_voxel(rec.p, nf), face, sv.x, rec, nf);
                seed = vec4<f32>(sv.x, 0.0, sv.x, f32(max(sky.sky_history, 1u)));
                no_ray = true;
                nr_voxel = store_voxel(rec.p, nf);
                nr_face = face;
                nr_nf = nf;
            }
        }
        // The no-ray sky term joins the stored part after the history, at its faded value
        // (`no_ray_fade`); the return below, which keeps no history, takes it as it is.
        sample = vec4<f32>(e.xyz * exposure_now(), 0.0);
        if ((params.lighting & LIGHTING_DIRECT) == 0u && (params.flags & FLAG_CUTAWAY) == 0u
            && view_mode() != VIEW_STEPS) {
            var held = 0.0;
            if ((params.history & 1u) != 0u && !in_edit(rec.p)) {
                held = min(floor(textureLoad(prev_moments_in, px, 0).z + 0.5) + 1.0, AGE_MAX);
            }
            // A face's pixel is never fresh here (no light, no shadow ray); a moved part's is.
            let fresh_w = select(0.0, 1.0, fresh);
            let lit = sample.xyz + sky_term * exposure_now();
            textureStore(accum_out, px, min(vec4<f32>(lit, fresh_w), vec4<f32>(HDR_MAX)));
            textureStore(direct_out, px, vec4<f32>(0.0));
            textureStore(moments_out, px, vec4<f32>(0.0, 0.0, held, 1.0 + source_fraction(source, on_entity)));
            textureStore(pair_out, px, seed);
            return;
        }
    }
    // The filtered part's luminance: the direct part's, and a fallback's emissive part.
    let l = min(select(luminance(sample.xyz), 0.0, stored) + direct.w, MOMENT_LUM_MAX);
    var accum = sample;
    var acc_direct = direct;
    var moments = vec2<f32>(l, l * l);
    var age = 0.0;
    // The visibility pair's history and whether it is taken; the direct part's and its
    // moments' blend weights and the moments' history, for the pair's term (1, 1 and none at a
    // reset pixel).
    var hp = vec4<f32>(0.0);
    // Whether this pixel's pair carries the key mean's settle (`key_settle`), for its taps.
    let own_key = vis.z > 0.5 && settle_on() && !sky_cast && seed.w > 0.0;
    let cap_sky = f32(max(sky.sky_history, 1u));
    var accepted = false;
    var a_direct = 1.0;
    var a_mom = 1.0;
    var m_hist = vec2<f32>(0.0);
    if (rec.found && (params.history & 1u) != 0u && !in_edit(rec.p)) {
        let vi = view_of(full_px(gid.xy));
        if (!rec.entity && !rec.field && rec.axis >= 0 && normal_mode() == NORMAL_FACE
            && (params.lighting & LIGHTING_HOOK_NO_STAIR) == 0u) {
            stair_w = far_normal_weight(params.views[vi], rec.t);
        }
        let pp = prev_world(rec);
        // A world hit at a still camera tests its own pixel's previous hit without the
        // material and normal tests (`history_agrees`).
        let still = !rec.entity && view_still(params.prev_views[vi], params.views[vi]);
        let full = reproject_centred(params.prev_views[vi], params.views[vi].jitter, pp);
        let q = select(prev_light_grid(full), full, full.x < 0.0) - 0.5;
        let base = vec2<i32>(floor(q));
        let f = q - floor(q);
        var w_sum = 0.0;
        var h = vec4<f32>(0.0);
        var hd = vec4<f32>(0.0);
        var m = vec3<f32>(0.0);
        // The accepted tap of the largest weight and its history: a held fallback copies it
        // where it carries nearly all the weight, so a still pixel's value is not re-rounded.
        var best_w = 0.0;
        var best_h = vec3<f32>(0.0);
        let tol = HISTORY_DIST + HISTORY_DIST_PER_T * rec.t;
        for (var i = 0; i < 4; i++) {
            let o = vec2<i32>(i & 1, i >> 1);
            let tap = base + o;
            if (!on_light_grid(tap)) {
                continue;
            }
            let prev = unpack_hit(textureLoad(prev_pos_in, tap, 0), textureLoad(prev_info_in, tap, 0));
            if (!history_agrees(prev, rec, pp, tol, still && all(tap == px))) {
                continue;
            }
            let wx = select(1.0 - f.x, f.x, o.x == 1);
            let wy = select(1.0 - f.y, f.y, o.y == 1);
            let w = wx * wy;
            w_sum += w;
            let ht = textureLoad(prev_accum_in, tap, 0);
            h += w * ht;
            hd += w * textureLoad(prev_direct_in, tap, 0);
            hp += w * pair_tap(textureLoad(prev_pair_in, tap, 0), own_key, cap_sky);
            if (w > best_w) {
                best_w = w;
                best_h = ht.xyz;
            }
            m += w * textureLoad(prev_moments_in, tap, 0).xyz;
        }
        // A world hit whose previous pixel nearest the reprojection showed an entity: the
        // surface was hidden behind it, so no neighbour's history is borrowed (a floor point
        // a part uncovers would take a neighbour's, lit or shadowed apart from it). An entity
        // hit searches its own instance's pixels: under the jitter its silhouette's pixels
        // fall on the world or another thing every other frame.
        let near = vec2<i32>(floor(q + 0.5));
        var hidden = false;
        if (on_light_grid(near) && !rec.entity) {
            let n = unpack_hit(textureLoad(prev_pos_in, near, 0), textureLoad(prev_info_in, near, 0));
            hidden = n.found && !same_thing(n, rec);
        }
        if (w_sum <= 1e-3 && !hidden) {
            // No bilinear tap agrees: the best-agreeing pixel of the 3 x 3 around the nearest
            // (the nearest position), so a jittered hit beside a face's edge finds its face.
            var best = tol;
            for (var i = 0; i < 9; i++) {
                let tap = near + vec2<i32>(i % 3 - 1, i / 3 - 1);
                if (!on_light_grid(tap)) {
                    continue;
                }
                let prev = unpack_hit(textureLoad(prev_pos_in, tap, 0), textureLoad(prev_info_in, tap, 0));
                let d = distance(prev.p, pp);
                if (!history_agrees(prev, rec, pp, best, still && all(tap == px))) {
                    continue;
                }
                best = d;
                w_sum = 1.0;
                h = textureLoad(prev_accum_in, tap, 0);
                hd = textureLoad(prev_direct_in, tap, 0);
                hp = pair_tap(textureLoad(prev_pair_in, tap, 0), own_key, cap_sky);
                best_w = 1.0;
                best_h = h.xyz;
                m = textureLoad(prev_moments_in, tap, 0).xyz;
            }
        }
        if (COUNT_STORE) {
            atomicAdd(&counters[5], 1u);
            if (w_sum <= 1e-3) {
                var prev = HitRecord();
                if (on_light_grid(near)) {
                    prev = unpack_hit(textureLoad(prev_pos_in, near, 0), textureLoad(prev_info_in, near, 0));
                }
                count_rejection(prev, rec, still && all(near == px));
            }
        }
        if (w_sum > 1e-3) {
            let ratio = exposure_ratio();
            h = h / w_sum * ratio;
            hd = hd / w_sum * ratio;
            hp = hp / w_sum;
            accepted = true;
            m = vec3<f32>(m.xy / w_sum * vec2<f32>(ratio, ratio * ratio), m.z / w_sum);
            age = min(floor(m.z + 0.5) + 1.0, AGE_MAX);
            // A fallback's emissive part: the running mean, held from FALLBACK_CAP samples.
            // Within an emission-scale change's frames (Params.lighting bit 8) it blends at no
            // less than ALPHA, so a held mean follows the new emission.
            var a_emissive = select(1.0 / (age + 1.0), 0.0, age >= FALLBACK_CAP);
            if ((params.lighting & LIGHTING_EMISSION_CHANGED) != 0u) {
                a_emissive = max(a_emissive, ALPHA);
            }
            var a = max(ALPHA, 1.0 / (age + 1.0));
            if ((params.lighting & LIGHTING_GRADIENT) != 0u) {
                // Where the lamp's light changed, the new sample weighs as much as the change.
                let lambda = clamp(GRADIENT_GAIN * gradient_at(q + 0.5), 0.0, 1.0);
                a = max(a, lambda);
                age = floor(age * (1.0 - lambda) + 0.5);
            }
            // The moments blend at `a` but at a fresh pixel: a moving part's every pixel is
            // fresh while it moves, and moments taken as they are hold no variance, which shuts
            // the filter's luminance weight on its fallback's emissive part.
            var a_moments = a;
            let reset = (params.lighting & (LIGHTING_LIGHTS_CHANGED | LIGHTING_SKY_CHANGED)) != 0u;
            if (reset) {
                a_moments = 1.0;
            }
            if (reset || fresh) {
                a = 1.0;
            }
            var e_fallback = mix(h.xyz, sample.xyz, a_emissive);
            if (a_emissive == 0.0 && best_w >= HELD_TAP_W) {
                e_fallback = best_h * ratio;
            }
            let e_new = select(e_fallback, sample.xyz, stored);
            accum = vec4<f32>(e_new, 0.0);
            acc_direct = mix(hd, direct, a);
            moments = mix(m.xy, vec2<f32>(l, l * l), a_moments);
            a_direct = a;
            a_mom = a_moments;
            m_hist = m.xy;
        }
    }
    // The visibility pair, and its sky term where the rays were cast. The direct part does not
    // take the sky term: the filter's input (atrous.wgsl `variance`) and the composite's
    // unfiltered sum add it from this frame's pair, so its only history is the pair's, and the
    // switch between the stored part and the ray at the stale bit has no second history to lag
    // behind it. Its luminance joins the moments, so the filter's variance counts it. The key
    // light's term, sampled every frame where the key light is on, joins the direct part.
    // A record's value seeds a reset pair at SEED_AGE, the column's at SEED_COLUMN.
    let seed_age = select(SEED_AGE, SEED_COLUMN, estimate < 0.0);
    if (estimate < 0.0) {
        estimate = column_estimate(rec.p, lift_n);
    }
    var pair = select(seed, pair_step(vis.x, hp, accepted, estimate, seed_age), sky_cast);
    if (no_ray && !own_key) {
        // The no-ray value rides one fade; the key settle's pixel keeps its pair's z and w.
        let fx = no_ray_fade(seed.x, hp, accepted, seed.w);
        pair = vec4<f32>(fx.x, pair.y, fx.x, fx.y);
    }
    if (no_ray) {
        let t_nr = sky_baked_term(store_head(), nr_voxel, nr_face, pair.x, rec, nr_nf) * exposure_now();
        accum = vec4<f32>(accum.xyz + t_nr, accum.w);
    }
    var t_sky = sky_pair_term(sky_e, pair.x, vis.a, rec.n);
    // The moments' w: 0 a pixel the filter takes whole, 1 a stored pixel (its stored part passes
    // around the filter), 2 a stored pixel whose pair's sky term passes around it too.
    var kind = select(0.0, 1.0, stored);
    if (sky_cast && rec_face && sky_on() && (params.lighting & LIGHTING_HOOK_SKY_RAY_ALWAYS) == 0u
        && pair.w >= f32(max(sky.sky_history, 1u))) {
        // A stale ring face whose pair holds the record's estimate: its term in the baked
        // term's form with the pair's mean as the face's own visibility, carried by the stored
        // part as the baked term is. With the filter off the composite adds `sky_pair_term`
        // from the pair, so the stored part carries the difference instead.
        let t_baked = sky_baked_term(store_head(), rec_voxel, rec_dir, pair.x, rec, rec_nf) * exposure_now();
        if ((params.lighting & LIGHTING_FILTER_OFF) != 0u) {
            accum = vec4<f32>(max(accum.xyz + t_baked - t_sky, vec3<f32>(0.0)), accum.w);
        } else {
            accum = vec4<f32>(accum.xyz + t_baked, accum.w);
            kind = 2.0;
        }
        t_sky = t_baked;
    }
    var t_key = vec3<f32>(0.0);
    if (own_key) {
        let k = key_settle(vis.y, hp, age, !accepted || fresh, seed.w);
        pair = vec4<f32>(pair.x, k.x, k.y, k.z);
        t_key = key_light_at(rec.p, rec.n) * pair.y * exposure_now();
    } else if (vis.z > 0.5) {
        pair.y = key_mean(vis.y, hp.y, age, !accepted || fresh);
        t_key = key_light_at(rec.p, rec.n) * pair.y * exposure_now();
    }
    if (any(t_key > vec3<f32>(0.0))) {
        acc_direct += a_direct * vec4<f32>(t_key, luminance(t_key));
    }
    let t = select(t_sky, vec3<f32>(0.0), kind > 1.5) + t_key;
    if (any(t > vec3<f32>(0.0))) {
        let l2 = min(l + luminance(t), MOMENT_LUM_MAX);
        moments = mix(m_hist, vec2<f32>(l2, l2 * l2), a_mom);
    }
    textureStore(pair_out, px, pair);
    accum.w = select(0.0, 1.0, fresh);
    textureStore(accum_out, px, min(accum, vec4<f32>(HDR_MAX)));
    textureStore(direct_out, px, min(acc_direct, vec4<f32>(HDR_MAX)));
    // The moments of a luminance at most MOMENT_LUM_MAX: the rescaled history can carry them
    // past it, and the second moment's bound, 65 025, is still a finite half.
    let m_max = vec2<f32>(MOMENT_LUM_MAX, MOMENT_LUM_MAX * MOMENT_LUM_MAX);
    textureStore(moments_out, px, vec4<f32>(min(moments, m_max), age, kind + source_fraction(source, on_entity)));
}
