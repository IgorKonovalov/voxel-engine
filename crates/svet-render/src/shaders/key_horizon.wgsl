// The key light's column horizon (ADR-0079, ADR-0084): the march over the column map, the
// switch to the walk (`key_walk.wgsl`) and the point on a cube face it is read at
// (`Lighting::key_at`). Composed into the temporal pass and the light pass, which reads it for
// the near cover source (`key_cover_near`, sky_light.wgsl), after `common.wgsl` and
// `hitrec.wgsl`.

// How far a seed's column test (`column_estimate`, pair.wgsl) and the key light's horizon lift
// the hit along the face normal, in voxels: past the column map's cell, so the point is not
// read in the wall's own cell (whose top is the wall's).
const SEED_LIFT: f32 = f32(COLUMN_CELL) + 0.5;

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

// The column horizon at hit `p` with face normal `n`: under LIGHTING2_KEY_WALK the walk over
// every cell (`column_key_walk`, `key_walk.wgsl`), else the march (`column_key_visibility`).
fn column_key_horizon(p: vec3<f32>, n: vec3<f32>) -> f32 {
    if ((params.lighting2 & LIGHTING2_KEY_WALK) != 0u) {
        return column_key_walk(p, n);
    }
    return column_key_visibility(p, n);
}

// The column horizon (`column_key_horizon`) of a cube face's hit `p` with face normal `nf`
// on `axis` (`Lighting::key_at`): at the hit; under LIGHTING2_KEY_AT_FACE at the centre of the
// hit's voxel face (the voxel's centre half a voxel along `nf`), one value a face whatever the
// jitter; under LIGHTING2_KEY_AT_SMOOTH the bilinear blend of the horizons at the four face
// centres of the face's plane nearest the hit (the lattice at half-integer coordinates across
// the plane), a ramp one voxel wide fixed in the world. A hit with no face axis reads the hit.
fn key_horizon_at(p: vec3<f32>, nf: vec3<f32>, axis: i32) -> f32 {
    let bits = params.lighting2 & (LIGHTING2_KEY_AT_FACE | LIGHTING2_KEY_AT_SMOOTH);
    if (bits == 0u || axis < 0) {
        return column_key_horizon(p, nf);
    }
    let along = axis_mask(axis);
    let centre = vec3<f32>(store_voxel(p, nf)) + 0.5 + 0.5 * nf;
    if ((bits & LIGHTING2_KEY_AT_FACE) != 0u) {
        return column_key_horizon(centre, nf);
    }
    // Across the plane: the lattice point at or below the hit on each tangent axis, and the
    // hit's place between it and the next; on the face's axis the plane itself.
    let g = p - 0.5;
    let lo = select(floor(g) + 0.5, centre, along);
    let f = select(fract(g), vec3<f32>(0.0), along);
    let a = (axis + 1) % 3;
    let b = (axis + 2) % 3;
    let ea = select(vec3<f32>(0.0), vec3<f32>(1.0), axis_mask(a));
    let eb = select(vec3<f32>(0.0), vec3<f32>(1.0), axis_mask(b));
    let fa = dot(f, ea);
    let fb = dot(f, eb);
    let k00 = column_key_horizon(lo, nf);
    let k10 = column_key_horizon(lo + ea, nf);
    let k01 = column_key_horizon(lo + eb, nf);
    let k11 = column_key_horizon(lo + ea + eb, nf);
    return mix(mix(k00, k10, fa), mix(k01, k11, fa), fb);
}
