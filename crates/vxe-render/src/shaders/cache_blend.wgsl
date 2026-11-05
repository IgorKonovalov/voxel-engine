// The emissive table's blend (ADR-0043): a world-space interpolation over the table, computed once
// a seen cell a frame, the value the temporal pass reads. Its inputs are cell sums, which do not
// move while the world and the light do not, so a still world reads bit-identical words. Compiled
// after cache_cell.wgsl alone: no world group.
//
// `blend_cells`: one lane per entry of the blend list, which the light pass appends each cell to
// on its first touch of the frame (light.wgsl's `blend_list`) and the walk's args pass empties
// (cache_sweep.wgsl). The list lives past the table's cells: its entry count, the dirty word
// (`CACHE_BLEND_DIRTY`), the last frame's (`CACHE_BLEND_WAS_DIRTY`), a word of padding, then the
// slots. `blend_args` sizes the dispatch to the list.
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
// The kernel's centre is the listed cell itself, read from its own slot with no lookup: no hole
// hides a cell from its own blend. A neighbour's lookup ends at the first free slot of its probes,
// though a claim steps over one (light.wgsl): a sweep's hole before a neighbour's slot hides that
// neighbour from this kernel until a claim fills it, a fixed result that does not move; a table of
// the blend's size is mostly free, so an absent neighbour costs one slot, not CACHE_PROBES.
//
// The result is `blend_pack`ed into the cell's CACHE_BLEND word.
//
// Bindings, group 2:
//   0  uniform  blend       the pass's frame (`Blend`, `marcher::BlendUniform`)
//   1  storage  cache_cells the table (light.wgsl)
//   2  storage  blend_dispatch the blend's indirect dispatch (`blend_args` only: `blend_cells`
//      takes it as its indirect argument)

struct Blend {
    cache_mask: u32,
    c_cap: u32,
    pool_min: u32,
    // 1 where the 5 x 5 may take the 3 x 3's place.
    wide: u32,
    // `params.frame + 1`, the seen words' clock.
    frame: u32,
    // 1 where the mode changed: the blend words the list reads are stale whatever is dirty.
    force: u32,
    _pad1: u32,
    _pad2: u32,
};

@group(2) @binding(0) var<uniform> blend: Blend;
@group(2) @binding(1) var<storage, read_write> cache_cells: array<atomic<u32>>;
@group(2) @binding(2) var<storage, read_write> blend_dispatch: array<u32, 3>;

// The sums and the count of the cell at `word`, its seen word stamped.
fn cell_at(word: u32) -> vec4<f32> {
    atomicStore(&cache_cells[word + CACHE_SEEN], blend.frame);
    let n = min(atomicLoad(&cache_cells[word + CACHE_COUNT]), blend.c_cap);
    return vec4<f32>(
        f32(atomicLoad(&cache_cells[word + CACHE_COUNT + 1u])),
        f32(atomicLoad(&cache_cells[word + CACHE_COUNT + 2u])),
        f32(atomicLoad(&cache_cells[word + CACHE_COUNT + 3u])),
        f32(n),
    );
}

// The sums and the count of the key's cell (`cell_at`); a w of -1 with no cell.
fn cell_read(v: vec3<i32>, face: u32) -> vec4<f32> {
    let base = cache_mix(v, face, 0x8DA6B343u, 0xCB1AB31Fu) & blend.cache_mask;
    let key = cache_key(v, face);
    for (var i = 0u; i < CACHE_PROBES; i++) {
        let word = CACHE_WORDS * ((base + i) & blend.cache_mask);
        let w1 = atomicLoad(&cache_cells[word + 1u]);
        if (w1 == 0u) {
            break;
        }
        if (w1 == key.y && atomicLoad(&cache_cells[word]) == key.x) {
            return cell_at(word);
        }
    }
    return vec4<f32>(0.0, 0.0, 0.0, -1.0);
}

// The offset's neighbour face (the coplanar voxel, then a step toward the air, then a step into
// the rock): sums in xyz and count in w, a stepped one at half weight; a w of -1 with none.
fn neighbour(at: vec3<i32>, air: vec3<i32>, face: u32) -> vec4<f32> {
    let c = cell_read(at, face);
    if (c.w >= 0.0) {
        return c;
    }
    let up = cell_read(at + air, face);
    if (up.w >= 0.0) {
        return up * 0.5;
    }
    let down = cell_read(at - air, face);
    if (down.w >= 0.0) {
        return down * 0.5;
    }
    return vec4<f32>(0.0, 0.0, 0.0, -1.0);
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

// One workgroup at a frame's start of the pass: this frame's dispatch, `ceil(entries / 64)`
// workgroups, or none once nothing the blend reads has changed for two frames
// (`CACHE_BLEND_DIRTY`).
@compute @workgroup_size(1, 1, 1)
fn blend_args() {
    let list = CACHE_WORDS * (blend.cache_mask + 1u);
    let dirty = atomicExchange(&cache_cells[list + CACHE_BLEND_DIRTY], 0u) != 0u;
    let was = atomicExchange(&cache_cells[list + CACHE_BLEND_WAS_DIRTY], u32(dirty)) != 0u;
    let groups = (atomicLoad(&cache_cells[list]) + 63u) / 64u;
    blend_dispatch = array<u32, 3>(select(0u, groups, dirty || was || blend.force != 0u), 1u, 1u);
}

@compute @workgroup_size(64, 1, 1)
fn blend_cells(@builtin(global_invocation_id) gid: vec3<u32>) {
    let list = CACHE_WORDS * (blend.cache_mask + 1u);
    if (gid.x >= atomicLoad(&cache_cells[list])) {
        return;
    }
    let slot = atomicLoad(&cache_cells[list + 4u + gid.x]);
    let word = CACHE_WORDS * slot;
    // A cell with a blend refreshes on its slot's parity of frames; `blend_args` runs one frame
    // past the last change, so every cell's last blend reads the final table.
    if (blend.force == 0u && ((slot ^ blend.frame) & 1u) != 0u && atomicLoad(&cache_cells[word + CACHE_BLEND]) != 0u) {
        return;
    }
    let k = vec2<u32>(atomicLoad(&cache_cells[word]), atomicLoad(&cache_cells[word + 1u]));
    let v = cache_key_voxel(k);
    let face = (k.y >> 28u) & 7u;
    let axis = face / 2u;
    let air = unit(axis) * select(-1, 1, (face & 1u) != 0u);
    let u = unit((axis + 1u) % 3u);
    let w = unit((axis + 2u) % 3u);
    // The 3 x 3, kept for the 5 x 5's inner weights.
    var inner: array<vec4<f32>, 9>;
    var pool = vec4<f32>(0.0);
    var raw = 0.0;
    for (var dw = -1; dw <= 1; dw++) {
        for (var du = -1; du <= 1; du++) {
            var c: vec4<f32>;
            if (du == 0 && dw == 0) {
                c = cell_at(word);
            } else {
                c = max(neighbour(v + u * du + w * dw, air, face), vec4<f32>(0.0));
            }
            inner[u32((dw + 1) * 3 + du + 1)] = c;
            raw += c.w;
            pool += c * tent3(du) * tent3(dw);
        }
    }
    if (blend.wide != 0u && raw < f32(blend.pool_min)) {
        pool = vec4<f32>(0.0);
        for (var dw = -2; dw <= 2; dw++) {
            for (var du = -2; du <= 2; du++) {
                var c: vec4<f32>;
                if (abs(du) <= 1 && abs(dw) <= 1) {
                    c = inner[u32((dw + 1) * 3 + du + 1)];
                } else {
                    c = max(neighbour(v + u * du + w * dw, air, face), vec4<f32>(0.0));
                }
                pool += c * tent5(du) * tent5(dw);
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
