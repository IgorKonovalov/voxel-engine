// The entity walk (ADR-0050): the tree over the entities' boxes, then each part's voxel grid in
// its own space, after the world march and limited by its hit.
//
// Bindings, group 0 (the world's group):
//   15 storage  entity_data   the entity mirror (`entities.rs`): the tree's nodes and header,
//                             the instances from ENTITY_INSTANCE_WORD, the pool's volumes

@group(0) @binding(15) var<storage, read> entity_data: array<u32>;

// An entity hit: the instance, the ray parameter, the face normal in world space and the
// material. `found` false on a miss.
struct EntityHit {
    found: bool,
    t: f32,
    instance: u32,
    n: vec3<f32>,
    material: u32,
};

fn entity_f(w: u32) -> f32 {
    return bitcast<f32>(entity_data[w]);
}

// Row `r` (three rows from word `w`) of a 3 x 4 transform applied to point `p`.
fn entity_row(w: u32, p: vec3<f32>, one: f32) -> f32 {
    return entity_f(w) * p.x + entity_f(w + 1u) * p.y + entity_f(w + 2u) * p.z + entity_f(w + 3u) * one;
}

// The 3 x 4 transform at word `w` applied to `p`, as a point (`one` 1) or a direction (0).
fn entity_xform(w: u32, p: vec3<f32>, one: f32) -> vec3<f32> {
    return vec3<f32>(entity_row(w, p, one), entity_row(w + 4u, p, one), entity_row(w + 8u, p, one));
}

// The 3 x 3 linear part at word `w`, transposed, applied to `n`.
fn entity_xform_t(w: u32, n: vec3<f32>) -> vec3<f32> {
    return n.x * vec3<f32>(entity_f(w), entity_f(w + 1u), entity_f(w + 2u))
        + n.y * vec3<f32>(entity_f(w + 4u), entity_f(w + 5u), entity_f(w + 6u))
        + n.z * vec3<f32>(entity_f(w + 8u), entity_f(w + 9u), entity_f(w + 10u));
}

// The entry and exit parameters of `o + d t` through box `[lo, hi]` (`inv` = 1 / d), and the
// entry's axis; `x > y` when it misses.
struct Span {
    t0: f32,
    t1: f32,
    axis: i32,
};

fn entity_span(o: vec3<f32>, inv: vec3<f32>, lo: vec3<f32>, hi: vec3<f32>) -> Span {
    let a = (lo - o) * inv;
    let b = (hi - o) * inv;
    let n = min(a, b);
    let f = max(a, b);
    var axis = 0;
    if (n.y > n[axis]) { axis = 1; }
    if (n.z > n[axis]) { axis = 2; }
    return Span(n[axis], min(min(f.x, f.y), f.z), axis);
}

// Bit `i` of the bit array from word `w`.
fn entity_bit(w: u32, i: u32) -> bool {
    return ((entity_data[w + (i >> 5u)] >> (i & 31u)) & 1u) != 0u;
}

// Voxel steps a part's walk takes at most: across a 64-voxel volume on all three axes.
const ENTITY_MAX_STEPS: u32 = 200u;

// The first solid voxel of instance `k` along `o + d t` for `t < limit`: the ray carried into
// the part's model voxels, a cell walk that steps over empty 4-voxel blocks.
fn entity_part(k: u32, o: vec3<f32>, d: vec3<f32>, limit: f32) -> EntityHit {
    var miss = EntityHit(false, limit, k, vec3<f32>(0.0), 0u);
    let base = ENTITY_INSTANCE_WORD + k * ENTITY_INSTANCE_WORDS;
    let vol = entity_data[base + 24u];
    let size = vec3<i32>(vec3<u32>(entity_data[base + 28u], entity_data[base + 29u], entity_data[base + 30u]));
    let po = entity_xform(base, o, 1.0);
    let pd = entity_xform(base, d, 0.0);
    let inv = select(vec3<f32>(BIG), 1.0 / pd, pd != vec3<f32>(0.0));
    let span = entity_span(po, inv, vec3<f32>(0.0), vec3<f32>(size));
    let t_start = max(span.t0, 0.0);
    if (span.t0 > span.t1 || span.t1 <= 0.0 || t_start >= limit) {
        return miss;
    }
    var axis = select(-1, span.axis, span.t0 > 0.0);
    let occ = vol + entity_data[vol + 3u];
    let blk = vol + entity_data[vol + 4u];
    let idx = vol + entity_data[vol + 5u];
    let pal = vol + entity_data[vol + 6u];
    let blocks = (size + vec3<i32>(3)) >> vec3<u32>(2u);
    let p0 = po + pd * t_start;
    var v = clamp(vec3<i32>(floor(p0)), vec3<i32>(0), size - 1);
    var t = t_start;
    for (var step = 0u; step < ENTITY_MAX_STEPS; step++) {
        if (any(v < vec3<i32>(0)) || any(v >= size) || t >= limit) {
            return miss;
        }
        let b = v >> vec3<u32>(2u);
        let full = entity_bit(blk, u32(b.x + blocks.x * (b.y + blocks.y * b.z)));
        let i = u32(v.x + size.x * (v.y + size.y * v.z));
        if (full && entity_bit(occ, i)) {
            let pi = (entity_data[idx + (i >> 2u)] >> (8u * (i & 3u))) & 0xFFu;
            var n = -d;
            if (axis >= 0) {
                let np = select(vec3<f32>(0.0), vec3<f32>(-sign(pd[axis])), axis_mask(axis));
                n = normalize(entity_xform_t(base, np));
            }
            return EntityHit(true, t, k, n, entity_data[pal + pi]);
        }
        // Leave the cell: the 4-voxel block when it is empty, else the voxel.
        let s = select(1, 4, !full);
        let cmin = (v / s) * s;
        let edge = cmin + select(vec3<i32>(0), vec3<i32>(s), pd > vec3<f32>(0.0));
        let tk = select(vec3<f32>(BIG), (vec3<f32>(edge) - po) * inv, pd != vec3<f32>(0.0));
        var a = 0;
        if (tk.y < tk[a]) { a = 1; }
        if (tk.z < tk[a]) { a = 2; }
        t = max(tk[a], t);
        let p = po + pd * t;
        // The other axes stay inside the cell's span; the exit axis steps over the edge.
        var nv = clamp(vec3<i32>(floor(p)), cmin, cmin + vec3<i32>(s - 1));
        let out = select(edge[a] - 1, edge[a], pd[a] > 0.0);
        nv = select(nv, vec3<i32>(out), axis_mask(a));
        v = nv;
        axis = a;
    }
    return miss;
}

// The nearest entity voxel along `o + d t` for `0 <= t < t_limit` (`d` need not be unit; `t`
// is in its units): the tree's boxes from the root, a fixed stack of 8, then each leaf's parts.
// With no entity drawn it returns at once.
fn entity_march(o: vec3<f32>, d: vec3<f32>, t_limit: f32) -> EntityHit {
    var best = EntityHit(false, t_limit, 0u, vec3<f32>(0.0), 0u);
    if (entity_data[ENTITY_HEADER_WORD] == 0u) {
        return best;
    }
    let inv = select(vec3<f32>(BIG), 1.0 / d, d != vec3<f32>(0.0));
    var stack: array<u32, 8>;
    var top = 1;
    stack[0] = 0u;
    loop {
        if (top == 0) {
            break;
        }
        top -= 1;
        let w = stack[top] * ENTITY_NODE_WORDS;
        let lo = vec3<f32>(entity_f(w), entity_f(w + 1u), entity_f(w + 2u));
        let hi = vec3<f32>(entity_f(w + 3u), entity_f(w + 4u), entity_f(w + 5u));
        let s = entity_span(o, inv, lo, hi);
        if (s.t0 > s.t1 || s.t1 < 0.0 || s.t0 >= best.t) {
            continue;
        }
        let a = entity_data[w + 6u];
        let b = entity_data[w + 7u];
        if ((a & ENTITY_LEAF_BIT) != 0u) {
            let first = a & ~ENTITY_LEAF_BIT;
            for (var k = first; k < first + b; k++) {
                let h = entity_part(k, o, d, best.t);
                if (h.found && h.t < best.t) {
                    best = h;
                }
            }
        } else if (top <= 6) {
            stack[top] = b;
            stack[top + 1] = a;
            top += 2;
        }
    }
    return best;
}

// The hit buffer's record of entity hit `e` on the ray `o + d t`: its point and `t`, and the
// info word with HIT_ENTITY, the instance and no cube face (axis bits 0).
fn entity_pos(o: vec3<f32>, d: vec3<f32>, e: EntityHit) -> vec4<f32> {
    return vec4<f32>(o + d * e.t, e.t);
}

fn entity_info(e: EntityHit) -> vec4<u32> {
    let flags = HIT_FOUND | HIT_ENTITY | (e.instance << HIT_INSTANCE_SHIFT);
    return vec4<u32>(octa16_encode(e.n), e.material, flags);
}

// The shadow offset's scale for hit `rec`: an entity part's `1 / detail`, else 1.
fn entity_shadow_scale(rec: HitRecord) -> f32 {
    if (!rec.entity) {
        return 1.0;
    }
    return entity_f(ENTITY_INSTANCE_WORD + rec.instance * ENTITY_INSTANCE_WORDS + 27u);
}
