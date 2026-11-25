// The key light's horizon walked over every column-map cell (`KeyHorizon::Walk`, ADR-0084):
// the march's horizon (`column_key_visibility`, `pair.wgsl`) without its sampling. Composed
// after `pair.wgsl`, whose column readers and constants it shares.

// The block map's block edge in map cells and a tile's blocks per axis
// (`gpu_world::COLUMN_BLOCK`, `COLUMN_BLOCKS`).
const COLUMN_BLOCK: i32 = 8;
const COLUMN_BLOCKS: i32 = 4;

// The most steps the walk takes (a cell read or a block passed): past the cells on a line
// across the largest window, a guard only.
const KEY_WALK_STEPS: u32 = 8192u;

// `v` divided by the power of two `d`, rounded toward negative infinity.
fn key_walk_div(v: vec2<i32>, d: i32) -> vec2<i32> {
    return v >> vec2<u32>(countTrailingZeros(u32(d)));
}

// Whether map cell `c` lies in the window's footprint (`column_cell_height` reads 3e38 outside).
fn column_cell_in_window(c: vec2<i32>) -> bool {
    let x = c * COLUMN_CELL;
    let lo = params.window_min.xy * 64;
    let hi = (params.window_min.xy + params.window_size.xy) * 64;
    return all(x >= lo) && all(x < hi);
}

// A coarse level's height (the largest top's z plus 1, -1e9 for none) over block `b` (global,
// in blocks): `per` blocks a tile per axis, COLUMN_BLOCKS the block map's and 1 the chunk
// map's. `b` lies in the window's footprint.
fn column_coarse_height(b: vec2<i32>, per: i32) -> f32 {
    let w = params.window_size.xy;
    let k = key_walk_div(b, per);
    let at = ((k % w) + w) % w * per + (b - k * per);
    var top: i32;
    if (per == 1) {
        top = textureLoad(column_chunks, at, 0).x;
    } else {
        top = textureLoad(column_blocks, at, 0).x;
    }
    return select(f32(top) + 1.0, -1.0e9, top == COLUMN_NONE_I);
}

// The ray parameters at which the line from `q` along unit `u` enters and leaves the box
// `lo..hi` (an axis `u` does not move along bounds nothing).
fn key_walk_span(q: vec2<f32>, u: vec2<f32>, lo: vec2<f32>, hi: vec2<f32>) -> vec2<f32> {
    let a = (lo - q) / u;
    let b = (hi - q) / u;
    let moving = abs(u) > vec2<f32>(1.0e-6);
    let t0 = select(vec2<f32>(-3.0e38), min(a, b), moving);
    let t1 = select(vec2<f32>(3.0e38), max(a, b), moving);
    return vec2<f32>(max(t0.x, t0.y), min(t1.x, t1.y));
}

// The largest horizon tangent a cell of height at most `top` gives a point at height `qz`
// between ray parameters `t.x` and `t.y` (the walk reads no cell entered at 0.5 or nearer).
fn key_walk_bound(top: f32, qz: f32, t: vec2<f32>) -> f32 {
    let num = top - qz;
    return select(num / t.y, num / max(t.x, 0.5), num >= 0.0);
}

// Whether block `b` of `e` map cells an edge cannot raise the horizon over `floor_tan` from
// `q` along `u` (its height from the level of `per` blocks a tile).
fn key_walk_passes(q: vec3<f32>, u: vec2<f32>, b: vec2<i32>, e: i32, per: i32, floor_tan: f32) -> bool {
    let lo = vec2<f32>(b * e * COLUMN_CELL);
    let hi = lo + f32(e * COLUMN_CELL);
    return key_walk_bound(column_coarse_height(b, per), q.z, key_walk_span(q.xy, u, lo, hi)) < floor_tan;
}

// The first map cell past block `b` of `e` cells an edge along the line from `q` along `u`
// (steps `s`): across the face the line leaves through first (x on a tie, as the cell walk
// steps), at the cell holding the exit point on the other axis, kept inside the block.
fn key_walk_exit(q: vec2<f32>, u: vec2<f32>, s: vec2<i32>, b: vec2<i32>, e: i32) -> vec2<i32> {
    let cs = f32(COLUMN_CELL);
    let first = b * e;
    let lo = vec2<f32>(first) * cs;
    let far = select(lo, lo + f32(e) * cs, u > vec2<f32>(0.0));
    let tx = select(vec2<f32>(3.0e38), (far - q) / u, abs(u) > vec2<f32>(1.0e-6));
    let t1 = min(tx.x, tx.y);
    let inside = clamp(vec2<i32>(floor((q + u * t1) / cs)), first, first + e - 1);
    let across = select(first - 1, first + e, s > vec2<i32>(0));
    return select(vec2<i32>(inside.x, across.y), vec2<i32>(across.x, inside.y), tx.x <= tx.y);
}

// The key light's visibility at hit `p` with face normal `n` (`column_key_visibility`'s lift,
// own-cell rule and ramp), the horizon's tangent the largest `(height - q.z) / t` over every
// map cell the sun's horizontal line from `q` crosses to the window's side, `t` the cell's entry.
// A chunk (32 x 32 cells) or block (8 x 8) whose largest top cannot raise the horizon over
// `max(tan h, tan e - tan r)` (under it the visibility is 1 whatever the cell) is passed whole,
// the chunk map's texel or the block map's read once on entering it; the walk ends where the
// visibility is 0 (`tan h >= tan e + tan r`) or the window's highest top (`params.column_top`)
// cannot raise the horizon from the cell's entry on.
fn column_key_walk(p: vec3<f32>, n: vec3<f32>) -> f32 {
    let l = sky.key_dir;
    if (l.z <= 0.0) {
        return 0.0;
    }
    let q = p + n * SEED_LIFT;
    let flat = length(l.xy);
    let tan_e = l.z / max(flat, 1.0e-6);
    let u = select(vec2<f32>(1.0, 0.0), l.xy / flat, flat > 1.0e-6);
    let r = max(tan(sky.key_radius), 1.0e-4);
    let cs = f32(COLUMN_CELL);
    var c = vec2<i32>(floor(q.xy / cs));
    let own = column_cell_height(c);
    if (own < 1.0e38 && own > q.z) {
        return 0.0;
    }
    let s = select(vec2<i32>(-1), vec2<i32>(1), u > vec2<f32>(0.0));
    let ahead = select(vec2<f32>(0.0), vec2<f32>(cs), u > vec2<f32>(0.0));
    let top_max = select(f32(params.column_top) + 1.0, -1.0e9, params.column_top == COLUMN_NONE_I);
    var tan_h = -3.0e38;
    // The chunk and the block the walk last entered (each read once), and whether `c` is the
    // cell after the last one read (false: a pass put the walk on its first cell past a block).
    var chunk_at = vec2<i32>(COLUMN_NONE_I);
    var block_at = vec2<i32>(COLUMN_NONE_I);
    var step = true;
    for (var i = 0u; i < KEY_WALK_STEPS; i++) {
        if (step) {
            let far = vec2<f32>(c) * cs + ahead;
            let tx = select(vec2<f32>(3.0e38), (far - q.xy) / u, abs(u) > vec2<f32>(1.0e-6));
            c += select(vec2<i32>(0, s.y), vec2<i32>(s.x, 0), tx.x <= tx.y);
        }
        step = true;
        if (!column_cell_in_window(c)) {
            break;
        }
        let te = column_cell_entry(q.xy, u, c);
        let floor_tan = max(tan_h, tan_e - r);
        if (tan_h >= tan_e + r || key_walk_bound(top_max, q.z, vec2<f32>(te, 3.0e38)) < floor_tan) {
            break;
        }
        let bc = key_walk_div(c, COLUMN_CELLS);
        if (any(bc != chunk_at)) {
            chunk_at = bc;
            if (key_walk_passes(q, u, bc, COLUMN_CELLS, 1, floor_tan)) {
                c = key_walk_exit(q.xy, u, s, bc, COLUMN_CELLS);
                step = false;
                continue;
            }
        }
        let bb = key_walk_div(c, COLUMN_BLOCK);
        if (any(bb != block_at)) {
            block_at = bb;
            if (key_walk_passes(q, u, bb, COLUMN_BLOCK, COLUMN_BLOCKS, floor_tan)) {
                c = key_walk_exit(q.xy, u, s, bb, COLUMN_BLOCK);
                step = false;
                continue;
            }
        }
        if (te > 0.5) {
            tan_h = max(tan_h, (column_cell_height(c) - q.z) / te);
        }
    }
    return clamp((tan_e - tan_h) / (2.0 * r) + 0.5, 0.0, 1.0);
}
