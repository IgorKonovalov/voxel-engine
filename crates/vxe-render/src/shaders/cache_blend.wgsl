// The emissive table's blend (ADR-0043): a world-space interpolation over the table, computed once
// a seen cell a frame, the value the temporal pass reads. Its inputs are cell sums, which do not
// move while the world and the light do not, so a still world reads bit-identical words. Compiled
// after cache_cell.wgsl alone: no world group.
//
// `blend_cells`: one lane per entry of the blend list, which the light pass appends each cell to
// on its first touch of the frame (light.wgsl's `blend_list`) and the walk's args pass empties
// (cache_sweep.wgsl). The list lives past the table's cells: its entry count, three words of
// padding, then the slots. Rows of `ceil(grid / 64)` workgroups cover the longest list; a lane
// past the count returns.
//
// The kernel, for a cell of face `(v, axis, sign)` with tangents u and w: each offset (du, dw)
// looks for one neighbour face of the same axis and sign, the coplanar voxel, then the voxel a
// step toward the air, then the voxel a step into the rock; the first with a cell taken, the two
// stepped candidates at half weight (a voxel staircase is how a cave wall at an angle is made).
// No face of another orientation is a candidate, so no light crosses a crease. The weights are
// the binomial tent (4, 2, 1) times the cell's count, so the mean is a pool of the faces' samples,
// `sum(t s) / sum(t n)`, not a mean of their means, and a young cell reads its wall's light.
// Where the 3 x 3 pool holds under `pool_min` samples and the widening is on, the 5 x 5's tent
// (1 4 6 4 1) takes its place. Every cell read is stamped seen: the age-out does not free a face
// a seen face's kernel reads.
//
// The result is `blend_pack`ed into the cell's CACHE_BLEND word.
//
// Bindings, group 2:
//   0  uniform  blend       the pass's frame (`Blend`, `marcher::BlendUniform`)
//   1  storage  cache_cells the table (light.wgsl)

struct Blend {
    cache_mask: u32,
    c_cap: u32,
    pool_min: u32,
    // 1 where the 5 x 5 may take the 3 x 3's place.
    wide: u32,
    // `params.frame + 1`, the seen words' clock.
    frame: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

@group(2) @binding(0) var<uniform> blend: Blend;
@group(2) @binding(1) var<storage, read_write> cache_cells: array<atomic<u32>>;

// The sums and the count of the key's cell, its seen word stamped; a w of -1 with no cell.
fn cell_read(v: vec3<i32>, face: u32) -> vec4<f32> {
    let base = cache_mix(v, face, 0x8DA6B343u, 0xCB1AB31Fu) & blend.cache_mask;
    let key = cache_key(v, face);
    for (var i = 0u; i < CACHE_PROBES; i++) {
        let word = CACHE_WORDS * ((base + i) & blend.cache_mask);
        if (atomicLoad(&cache_cells[word + 1u]) == key.y && atomicLoad(&cache_cells[word]) == key.x) {
            atomicStore(&cache_cells[word + CACHE_SEEN], blend.frame);
            let n = min(atomicLoad(&cache_cells[word + CACHE_COUNT]), blend.c_cap);
            return vec4<f32>(
                f32(atomicLoad(&cache_cells[word + CACHE_COUNT + 1u])),
                f32(atomicLoad(&cache_cells[word + CACHE_COUNT + 2u])),
                f32(atomicLoad(&cache_cells[word + CACHE_COUNT + 3u])),
                f32(n),
            );
        }
    }
    return vec4<f32>(0.0, 0.0, 0.0, -1.0);
}

// Adds the offset's neighbour face (the coplanar voxel, then a step toward the air, then a step
// into the rock) to `pool`, sums in xyz and count in w, a stepped one at half weight, and its
// samples unweighted to `raw`; nothing with none.
fn neighbour(at: vec3<i32>, air: vec3<i32>, face: u32, weight: f32, pool: ptr<function, vec4<f32>>, raw: ptr<function, f32>) {
    var c = cell_read(at, face);
    var half_weight = 1.0;
    if (c.w < 0.0) {
        c = cell_read(at + air, face);
        half_weight = 0.5;
    }
    if (c.w < 0.0) {
        c = cell_read(at - air, face);
    }
    if (c.w < 0.0) {
        return;
    }
    *raw += c.w;
    *pool += c * (weight * half_weight);
}

// The tents' weights by the offset from the centre: 1 2 1 and 1 4 6 4 1.
fn tent3(d: i32) -> f32 {
    return select(1.0, 2.0, d == 0);
}

fn tent5(d: i32) -> f32 {
    return select(select(1.0, 4.0, abs(d) == 1), 6.0, d == 0);
}

// The unit vector along `axis`.
fn unit(axis: u32) -> vec3<i32> {
    return vec3<i32>(i32(axis == 0u), i32(axis == 1u), i32(axis == 2u));
}

@compute @workgroup_size(64, 1, 1)
fn blend_cells(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let list = CACHE_WORDS * (blend.cache_mask + 1u);
    let entry = gid.x + gid.y * groups.x * 64u;
    if (entry >= atomicLoad(&cache_cells[list])) {
        return;
    }
    let word = CACHE_WORDS * atomicLoad(&cache_cells[list + 4u + entry]);
    let k = vec2<u32>(atomicLoad(&cache_cells[word]), atomicLoad(&cache_cells[word + 1u]));
    let v = cache_key_voxel(k);
    let face = (k.y >> 28u) & 7u;
    let axis = face / 2u;
    let air = unit(axis) * select(-1, 1, (face & 1u) != 0u);
    let u = unit((axis + 1u) % 3u);
    let w = unit((axis + 2u) % 3u);
    var pool = vec4<f32>(0.0);
    var raw = 0.0;
    for (var dw = -1; dw <= 1; dw++) {
        for (var du = -1; du <= 1; du++) {
            neighbour(v + u * du + w * dw, air, face, tent3(du) * tent3(dw), &pool, &raw);
        }
    }
    if (blend.wide != 0u && raw < f32(blend.pool_min)) {
        pool = vec4<f32>(0.0);
        for (var dw = -2; dw <= 2; dw++) {
            for (var du = -2; du <= 2; du++) {
                neighbour(v + u * du + w * dw, air, face, tent5(du) * tent5(dw), &pool, &raw);
            }
        }
    }
    var packed = 0u;
    if (pool.w > 0.0) {
        packed = blend_pack(pool.xyz / (pool.w * CACHE_Q));
    }
    atomicStore(&cache_cells[word + CACHE_BLEND], packed);
}

// `blend_unpack`'s word for a non-negative colour: the shared exponent of its largest channel,
// each channel's square root over it in nine bits.
fn blend_pack(c: vec3<f32>) -> u32 {
    let m = max(c.x, max(c.y, c.z));
    if (m < BLEND_MIN) {
        return 0u;
    }
    let e = clamp(i32(ceil(log2(m))) + BLEND_EXP_BIAS, 0, 31);
    let scale = ldexp(1.0, e - BLEND_EXP_BIAS);
    let q = vec3<u32>(round(sqrt(min(c / scale, vec3<f32>(1.0))) * 511.0));
    return q.x | (q.y << 9u) | (q.z << 18u) | (u32(e) << 27u);
}
