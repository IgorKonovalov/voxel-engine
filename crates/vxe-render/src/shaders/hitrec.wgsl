// The hit record a hit pass writes and the passes after it read: the octahedral normal, the
// record's unpacking and flags, the shadow ray's origin, reprojection, and the store's taps.

// Hit record flags.
const HIT_FOUND: u32 = 1u;

const HIT_FIELD: u32 = 2u;

const HIT_TRANSLUCENT: u32 = 4u;

const HIT_AXIS_SHIFT: u32 = 3u;

// An entity's voxel (entity.wgsl): no cube face (axis bits 0), the instance from bit
// HIT_INSTANCE_SHIFT: its slot (9 bits) and its slot's generation's low bit.
const HIT_ENTITY: u32 = 32u;

const HIT_INSTANCE_SHIFT: u32 = 6u;

// The store's words (`bake::Store`): three unused words, the window's origin and size in
// chunks, the flags, then a word per window chunk, its block's first word or 0. A block is 512
// directory words (a brick's offset from the block's start just past its record << 11; 0 none),
// then each surface brick's record.
const STORE_ORIGIN: u32 = 4u;
const STORE_SIZE: u32 = 8u;
// The prelude's STORE_FLAGS is the flags word (`bake::STORE_FLAGS`): bit 1, the records' cells
// are 4 voxels (eight a brick), else 8. A record is BRICK_WORDS a cell: six rgb9e5 direction
// values in face order, then `material | direction mask << 16`. STORE_TABLE is the first
// window chunk's word. TABLE_COARSE is a chunk table word's flag (`bake::TABLE_COARSE`): the
// block is the live queue's coarse pass; the rest of the word is the block's first word.

// One of the 2 x 2 cells (world cells of 1 << `shift` voxels) around voxel `v`'s centre in the
// plane of face `face` (its own cell's layer along the normal), and its bilinear weight by the
// centre's place between the cells' centres; `i` in 0..4. Cells are addressed in world cells, so
// the blend crosses a brick's border as it crosses a cell's.
struct StoreTap {
    cell: vec3<i32>,
    w: f32,
};

fn store_tap(v: vec3<i32>, face: u32, shift: u32, i: i32) -> StoreTap {
    let size = f32(1u << shift);
    let axis = face / 2u;
    let ta = (axis + 1u) % 3u;
    let tb = (axis + 2u) % 3u;
    let own = v >> vec3<u32>(shift);
    let g = (vec3<f32>(v) + 0.5) / size - 0.5;
    let lo = vec3<i32>(floor(g));
    let f = g - floor(g);
    // The two in-plane axes as masks: a store through a runtime vector index is not portable
    // (`shader_portability`), so the cell's coordinate is chosen per component.
    let axes = vec3<u32>(0u, 1u, 2u);
    let ma = axes == vec3<u32>(ta);
    let mb = axes == vec3<u32>(tb);
    let oa = i & 1;
    let ob = i >> 1u;
    let cell = select(select(own, lo + vec3<i32>(oa), ma), lo + vec3<i32>(ob), mb);
    let wa = select(1.0 - f[ta], f[ta], oa == 1);
    let wb = select(1.0 - f[tb], f[tb], ob == 1);
    return StoreTap(cell, wa * wb);
}

// The octahedral coordinates in [-1, 1]^2 of unit `d`.
fn octa_fold(d: vec3<f32>) -> vec2<f32> {
    var p = d.xy / (abs(d.x) + abs(d.y) + abs(d.z));
    if (d.z < 0.0) {
        let s = select(vec2<f32>(-1.0), vec2<f32>(1.0), p >= vec2<f32>(0.0));
        p = (1.0 - abs(p.yx)) * s;
    }
    return p;
}

// The unit direction of octahedral coordinates `p` in [-1, 1]^2.
fn octa_unfold(p: vec2<f32>) -> vec3<f32> {
    var d = vec3<f32>(p, 1.0 - abs(p.x) - abs(p.y));
    if (d.z < 0.0) {
        let s = select(vec2<f32>(-1.0), vec2<f32>(1.0), d.xy >= vec2<f32>(0.0));
        d = vec3<f32>((1.0 - abs(d.yx)) * s, d.z);
    }
    return normalize(d);
}

// Octahedral encoding of unit `n` into two snorm16 values (as u16 bits). The six axis
// normals encode to -1, 0 and 1 exactly, so they decode exactly.
fn octa16_encode(n: vec3<f32>) -> vec2<u32> {
    let p = octa_fold(n);
    let q = vec2<i32>(round(clamp(p, vec2<f32>(-1.0), vec2<f32>(1.0)) * 32767.0));
    return bitcast<vec2<u32>>(q) & vec2<u32>(0xFFFFu);
}

fn octa16_decode(e: vec2<u32>) -> vec3<f32> {
    let q = bitcast<vec2<i32>>(e << vec2<u32>(16u)) >> vec2<u32>(16u);
    return octa_unfold(vec2<f32>(q) / 32767.0);
}

// How far off the hit the shadow ray starts, in voxels, along `shadow_normal`.
const SHADOW_OFFSET: f32 = 0.5;

// SHADOW_OFFSET's scale for the hit being shaded: 1, or an entity part's world voxels per model
// voxel, `coarse / detail` (`entity_shadow_scale`), so a shadow ray leaves a small or a large
// voxel by the same share of it.
var<private> shadow_scale: f32 = 1.0;

// The field normal `n` of a field-surface hit whose stored shading normal is `m`, the face mix
// of `n` toward its entry face's normal `f`: `(1 - k) n = l m - k f` with `l = |mix|` the root
// of `l^2 - 2 l k (m.f) + k^2 - (1 - k)^2 = 0`, real for `k <= 1/2`; `m` itself at 0.
fn face_unmixed(m: vec3<f32>, f: vec3<f32>) -> vec3<f32> {
    let k = params.face_mix;
    if (k <= 0.0) {
        return m;
    }
    let mf = dot(m, f);
    let l = k * mf + sqrt(max(k * k * mf * mf - k * k + (1.0 - k) * (1.0 - k), 0.0));
    return normalize(l * m - k * f);
}

// One pixel's hit record, decoded from the hit buffer: `n` is the shading normal,
// `axis` the entry face's axis (-1: the ray started inside rock, or an entity's voxel, whose
// faces are not on the world's grid); `entity` an entity hit and `instance` its part's instance.
struct HitRecord {
    found: bool,
    field: bool,
    translucent: bool,
    axis: i32,
    material: u32,
    p: vec3<f32>,
    t: f32,
    n: vec3<f32>,
    entity: bool,
    instance: u32,
};

fn unpack_hit(pos: vec4<f32>, info: vec4<u32>) -> HitRecord {
    let flags = info.w;
    return HitRecord(
        (flags & HIT_FOUND) != 0u,
        (flags & HIT_FIELD) != 0u,
        (flags & HIT_TRANSLUCENT) != 0u,
        i32((flags >> HIT_AXIS_SHIFT) & 3u) - 1,
        info.z,
        pos.xyz,
        pos.w,
        octa16_decode(info.xy),
        (flags & HIT_ENTITY) != 0u,
        flags >> HIT_INSTANCE_SHIFT,
    );
}

// The shadow normal of a hit seen along `d`: an entity hit's turned face normal; a
// field-surface hit's field normal, unmixed
// (`face_unmixed`); else the entry face's normal, `-d` for a ray that started inside rock.
fn shadow_normal_of(rec: HitRecord, d: vec3<f32>) -> vec3<f32> {
    if (rec.entity) {
        return rec.n;
    }
    if (rec.axis < 0) {
        return select(-d, rec.n, rec.field);
    }
    let face = select(vec3<f32>(0.0), vec3<f32>(-sign(d[rec.axis])), axis_mask(rec.axis));
    if (rec.field) {
        return face_unmixed(rec.n, face);
    }
    return face;
}

// Where the shadow rays of a hit at `p` with shadow normal `ns` start: SHADOW_OFFSET (times
// `shadow_scale`) along `ns`. The shadow rays walk the voxels (`march`), crisp and smooth alike solid, and a
// field-surface hit lies up to half a voxel off its voxels' faces, so the offset is a step out
// of the voxels' rock as much as off the surface.
fn shadow_origin(p: vec3<f32>, ns: vec3<f32>) -> vec3<f32> {
    return p + ns * (SHADOW_OFFSET * shadow_scale);
}

// `p` projected through previous view `v` to a fractional pixel of the target (pixel
// centres at integer + 0.5, the view's jitter ignored); a negative x when it lies behind the
// view.
fn project_prev(v: View, p: vec3<f32>) -> vec2<f32> {
    let dir = p - v.origin;
    let z = dot(dir, v.fwd);
    if (z <= 1e-4) {
        return vec2<f32>(-1.0);
    }
    let x = dot(dir, v.right) / z;
    let y = dot(dir, v.up) / z;
    let u = (x + v.tan_left) / (v.tan_left + v.tan_right);
    let w = (v.tan_up - y) / (v.tan_up + v.tan_down);
    return vec2<f32>(v.rect.xy) + vec2<f32>(u, w) * vec2<f32>(v.rect.zw);
}

// The pixel of the previous frame a hit `p` of this frame's pixel reprojects to, as if both
// frames' rays went through their pixels' centres: `p` projected through previous view `v`,
// less this frame's jitter `j` (the offset of this pixel's ray from its centre). On a still
// camera every pixel maps onto its own centre whatever the two jitters, so history is never
// resampled across the moving sample lattice; with the jitter off it is `project_prev`. A
// negative x when `p` lies behind the view.
fn reproject_centred(v: View, j: vec2<f32>, p: vec3<f32>) -> vec2<f32> {
    let q = project_prev(v, p);
    if (q.x < 0.0) {
        return q;
    }
    return q - j;
}

// The key of face `face` of voxel `v`, two words: each coordinate's low 20 bits (the key wraps
// every 2^20 voxels, past any light radius), x then y's low 12 bits in word 0; y's high 8 bits,
// z, the face in bits 28..31 and a valid bit 31 in word 1, so word 1 is never 0. The bake's
// random sequence of a face is keyed by it (bake.wgsl).
fn face_seed_key(v: vec3<i32>, face: u32) -> vec2<u32> {
    let c = bitcast<vec3<u32>>(v) & vec3<u32>(0xFFFFFu);
    return vec2<u32>(c.x | (c.y << 20u), (c.y >> 12u) | (c.z << 8u) | (face << 28u) | 0x80000000u);
}

// Whether a hit reads the store (ADR-0046): an opaque cube face entered from air, a translucent
// solid in front of it or not (the solid filters what the face sends, ADR-0047). The light pass
// draws no emissive sample for it, and the temporal pass reads its brick record.
fn reads_store(rec: HitRecord) -> bool {
    return rec.found && rec.axis >= 0 && !rec.field;
}

// Whether an entity hit's emissive part is the store's (Params.lighting LIGHTING_ENTITY_WORLD,
// `EntityLight::World`): the light pass draws no emissive sample for it, and the temporal pass
// reads the store at the world faces around it (temporal.wgsl).
fn entity_reads_store(rec: HitRecord) -> bool {
    return rec.found && rec.entity && (params.lighting & LIGHTING_ENTITY_WORLD) != 0u;
}

// The voxel behind hit point `p` with face normal `nf`.
fn store_voxel(p: vec3<f32>, nf: vec3<f32>) -> vec3<i32> {
    return vec3<i32>(floor(p - 0.5 * nf));
}

// The entry face of a hit on `axis` with face normal `nf`.
fn store_face(axis: i32, nf: vec3<f32>) -> u32 {
    return 2u * u32(axis) + select(0u, 1u, nf[axis] > 0.0);
}

// The normal the light's filters (composite.wgsl's `lit_at`, atrous.wgsl's `edge_weight`)
// compare for hit `rec` seen from `origin`: its shading normal; with Params.lighting bit 15, a
// cube-face hit's face normal instead, its axis signed toward the side of the face's plane
// `origin` lies on (the eye's side: `shadow_normal_of`'s sign of the ray `p - origin`). A hit
// with no face axis or a field hit keeps its shading normal.
fn filter_normal(rec: HitRecord, origin: vec3<f32>) -> vec3<f32> {
    if ((params.lighting & LIGHTING_FACE_WEIGHTS) == 0u || rec.axis < 0 || rec.field) {
        return rec.n;
    }
    let s = select(1.0, -1.0, rec.p[rec.axis] > origin[rec.axis]);
    return select(vec3<f32>(0.0), vec3<f32>(s), axis_mask(rec.axis));
}

// Whether an emitter's light lands at internal pixel `px` (hit `rec`): the hit's material
// emits, or the temporal pass's emissive part in `emissive` (the store's value at a cube face,
// a fallback's sample elsewhere) is non-zero at any of the lighting texels around it. The
// upscalers mark an emission scale change at these pixels alone; the rest keeps its history.
fn emission_lands(emissive: texture_2d<f32>, px: vec2<i32>, rec: HitRecord) -> bool {
    if (any(materials[rec.material * 4u + 1u].xyz != vec3<f32>(0.0))) {
        return true;
    }
    let o = vec2<f32>(unpack_offset(params.light_offset));
    let base = vec2<i32>(floor((vec2<f32>(px) - o) / f32(params.light_scale)));
    for (var i = 0; i < 4; i++) {
        let q = base + vec2<i32>(i & 1, i >> 1);
        if (on_light_grid(q) && any(textureLoad(emissive, q, 0).xyz > vec3<f32>(0.0))) {
            return true;
        }
    }
    return false;
}

// Whether a fresh pixel (light.wgsl: a shadow ray crossed a moving entity's box, or the hit is
// a moved part's) lies at internal pixel `px`: the temporal pass's accumulation `accum` carries
// 1 in a at any of the lighting texels around it. The upscalers mark such a pixel changed.
fn fresh_lands(accum: texture_2d<f32>, px: vec2<i32>) -> bool {
    let o = vec2<f32>(unpack_offset(params.light_offset));
    let base = vec2<i32>(floor((vec2<f32>(px) - o) / f32(params.light_scale)));
    for (var i = 0; i < 4; i++) {
        let q = base + vec2<i32>(i & 1, i >> 1);
        if (on_light_grid(q) && textureLoad(accum, q, 0).w > 0.5) {
            return true;
        }
    }
    return false;
}
