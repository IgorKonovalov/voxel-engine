// The light pass's shading: direct irradiance at a hit from the list of lights (`lights`,
// ADR-0044; the headlamp is slot 0 while it is on) and from one emissive voxel face, and the
// shadow walk; its entry point is shade.wgsl's. Compiled after the walk's modules, whose bindings
// (group 0), walk, hit-record helpers and random numbers it uses, and entity.wgsl; every
// shadow ray is `march`, and with ENTITY_SHADOWS the entity walk after it.
//
// Bindings, group 0 beyond common.wgsl's:
//  10  storage  light_samples  the mirror's light samples (see common.wgsl)
//
// Each light of the list is a point or spot light: E = PI * intensity * cone * max(n.l, 0) /
// dist^2 times its window inside its reach and 0 beyond, so that the composite's Lambertian
// `albedo * E / PI` is the former march's `albedo * lit`; one shadow ray per lit pixel and
// light in reach with LIGHT_SHADOWS set, from SHADOW_OFFSET off the surface along the shadow
// normal toward the light.
//
// A pixel is fresh (ADR-0050) when one of its shadow rays entered the box of an entity with a
// moved part (`entity_fresh`), or its hit is a moved part's voxel: its direct light may differ
// from the frame before's at the same surface point, so the temporal pass takes this frame's
// direct sample as it is and the upscalers mark it changed.
//
// In the cutaway view a hit takes the hemisphere term `PI (0.3 + 0.7 (0.5 + 0.5 n.z))`
// instead, with no lamp, no emission and no shadow ray. Every shadow ray takes the frame's
// shadow walk (`shadowed`): the full walk, or the coarse walk that treats any 2^3 block with a
// set opaque bit as solid. Only the shaded and cracks views read this pass's irradiance; the
// steps view reads its shadow-ray counters; every other view skips the pass.

@group(0) @binding(10) var<storage, read> light_samples: array<vec4<i32>>;

// Whether the list's shadow rays also walk the entities (ADR-0050): on in the light and
// gradient passes, off in the probe rays and the bake, which see no entity.
override ENTITY_SHADOWS: bool = false;

// The frame's candidate table (ADR-0025), in light_samples from `params.light_table_base`:
// a header slot (count, total weight as f32 bits), then per entry the start and length of an
// emitter chunk's range and the cumulative weight up to and including it (f32 bits), in
// chunk-coordinate order, for the chunks within the light radius of the camera.
fn table_slot(i: u32) -> vec4<u32> {
    return bitcast<vec4<u32>>(light_samples[params.light_table_base + i]);
}

// A uniform point of face `face` of voxel `v`, from `rand` at pixel `px` (dimensions 0 and 1;
// the candidates take 2 on).
fn face_point(v: vec3<i32>, face: u32, px: vec2<u32>) -> vec3<f32> {
    let ax = face / 2u;
    let t0 = select(vec3<f32>(0.0), vec3<f32>(1.0), axis_mask(i32((ax + 1u) % 3u)));
    let t1 = select(vec3<f32>(0.0), vec3<f32>(1.0), axis_mask(i32((ax + 2u) % 3u)));
    return face_centre(v, face) + t0 * (rand(px, 0u) - 0.5) + t1 * (rand(px, 1u) - 0.5);
}

// The unit face `face` of voxel `v`'s centre.
fn face_centre(v: vec3<i32>, face: u32) -> vec3<f32> {
    return vec3<f32>(v) + vec3<f32>(0.5) + 0.5 * face_dir(face);
}

// Light `li`'s `intensity * cone * max(n.l, 0) / dist^2` times its window at `p` with shading
// normal `n`, zero when its shadow ray finds rock before the light. The ray leaves the shadow
// origin (`p` offset along `ns`) aimed at the light itself, as the emissive and baked rays do:
// a ray along the direction from `p` would run parallel to the true one, half a voxel off the
// surface, and clear the edge of a step that shadows a face next to a near light. Out of its
// reach no ray is cast. With ENTITY_SHADOWS the ray then walks the entities to the light.
fn light_lit(li: Light, p: vec3<f32>, n: vec3<f32>, ns: vec3<f32>) -> f32 {
    var lit = light_term(li, p, n);
    if ((li.flags & LIGHT_SHADOWS) != 0u && lit > 0.0) {
        let o = shadow_origin(p, ns);
        let to_light = li.pos - o;
        let dist2 = max(dot(to_light, to_light), 1e-4);
        let l = to_light * inverseSqrt(dist2);
        let dist = sqrt(dist2);
        if (shadowed_styled(o, l, dist) || (ENTITY_SHADOWS && entity_march(o, l, dist).found)) {
            lit = 0.0;
        }
    }
    return lit;
}

// The list's `light_lit` at `p`, each weighed by its colour's luminance.
fn lights_lit(p: vec3<f32>, n: vec3<f32>, ns: vec3<f32>) -> f32 {
    return lights_lit_rgb(p, n, ns).w;
}

// The list's `light_lit` at `p`, each times its colour summed in rgb, and weighed by its
// colour's luminance summed in w (a white light adds the same value to all four).
fn lights_lit_rgb(p: vec3<f32>, n: vec3<f32>, ns: vec3<f32>) -> vec4<f32> {
    var sum = vec4<f32>(0.0);
    for (var i = 0u; i < lights.count; i++) {
        let li = lights.l[i];
        let lit = light_lit(li, p, n, ns);
        sum += vec4<f32>(lit * li.colour, lit * colour_weight(li.colour));
    }
    return sum;
}

// The walk a shadow ray leaves to the full walk at each end under the coarse mode, in voxels:
// more than a 2^3 block's diagonal (2 sqrt 3), so the blocks of the surface it leaves and of
// the face it aims at do not stop it.
const COARSE_MARGIN: f32 = 3.5;

// Whether rock lies on `o + d t` for `0 <= t <= len`: the full walk, or with
// `Params.lighting` bit 1 the coarse walk between COARSE_MARGIN of either end.
fn shadowed(o: vec3<f32>, d: vec3<f32>, len: f32) -> bool {
    return shadowed_walk(o, d, len, (params.lighting & LIGHTING_COARSE_SHADOW) != 0u);
}

// `shadowed` with the coarse walk on where `coarse`, whatever `Params.lighting` says.
fn shadowed_walk(o: vec3<f32>, d: vec3<f32>, len: f32, coarse: bool) -> bool {
    if (coarse) {
        coarse_span = vec2<f32>(COARSE_MARGIN, len - COARSE_MARGIN);
    }
    march_material = false;
    let s = march(o, d, len, false);
    march_material = true;
    coarse_span = vec2<f32>(BIG, -BIG);
    return s.found;
}

// `shadowed` for a light's shadow ray (the list's, the lamp's, the key light's, a sky ray):
// while some material's style word has SHADOW_AS_BRICK (LIGHTING_SHADOW_STYLES), a brick
// holding such a material stops the ray where it enters it, the brick of `o` aside. The
// bake's emissive samples' rays take `shadowed`.
fn shadowed_styled(o: vec3<f32>, d: vec3<f32>, len: f32) -> bool {
    return shadowed_styled_walk(
        o,
        d,
        len,
        (params.lighting & LIGHTING_COARSE_SHADOW) != 0u,
        (params.lighting & LIGHTING_SHADOW_STYLES) != 0u,
    );
}

// `shadowed_styled` with the coarse walk and the styles on where `coarse` and `styles`
// (the bake's sky rays, which take the frame's choice from their job).
fn shadowed_styled_walk(o: vec3<f32>, d: vec3<f32>, len: f32, coarse: bool, styles: bool) -> bool {
    if (!styles) {
        return shadowed_walk(o, d, len, coarse);
    }
    shadow_styles = true;
    styled_from = voxel_at(o, d) >> vec3<u32>(3u);
    let s = shadowed_walk(o, d, len, coarse);
    shadow_styles = false;
    return s;
}

// Whether a sky ray from `o` along `d`, capped at `len`, reached the sky (ADR-0070 and its
// amendment of 2026-11-18): `shadowed_styled_walk` finds no rock, and where the ray ended (the
// cap, an absent chunk's face or the window's) lies above its column's top (`above_column`).
// Under the reading hook `end_is_sky` (LIGHTING_HOOK_SKY_END_IS_SKY, or the bake's
// BAKE_SKY_END_IS_SKY) any end is sky and an absent chunk is air, the rule before the column map.
fn sky_ray_open(o: vec3<f32>, d: vec3<f32>, len: f32, coarse: bool, styles: bool, end_is_sky: bool) -> bool {
    sky_rule = !end_is_sky;
    let s = shadowed_styled_walk(o, d, len, coarse, styles);
    sky_rule = false;
    return !s && (end_is_sky || above_column(march_end));
}

// A unit direction drawn uniformly over the solid angle of the disc of angular radius `r`
// about unit `l` (the key light's, sky_light.wgsl, and the bake's key rays), from `u` and `v` in [0, 1): `1 - cos` taken as `2 sin^2(r / 2) u`, which
// keeps its digits at the sun's 4.65 mrad.
fn disc_direction(l: vec3<f32>, r: f32, u: f32, v: f32) -> vec3<f32> {
    let h = sin(0.5 * r);
    let omc = 2.0 * h * h * u;
    let sin_t = sqrt(max(omc * (2.0 - omc), 0.0));
    let phi = 2.0 * PI * v;
    let a = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(l.x) > 0.9);
    let t = normalize(cross(a, l));
    let b = cross(l, t);
    return normalize(l * (1.0 - omc) + (t * cos(phi) + b * sin(phi)) * sin_t);
}

// A unit direction about unit `n` drawn with density `cos / pi` from `u` and `v` in [0, 1).
fn cosine_direction(n: vec3<f32>, u: f32, v: f32) -> vec3<f32> {
    let r = sqrt(u);
    let phi = 2.0 * PI * v;
    let a = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(n.x) > 0.9);
    let t = normalize(cross(a, n));
    let b = cross(n, t);
    return normalize(t * (r * cos(phi)) + b * (r * sin(phi)) + n * sqrt(max(1.0 - u, 0.0)));
}

// The unit normal of face `f` (2 k toward -k, 2 k + 1 toward +k).
fn face_dir(f: u32) -> vec3<f32> {
    let s = select(-1.0, 1.0, (f & 1u) == 1u);
    return select(vec3<f32>(0.0), vec3<f32>(s), axis_mask(i32(f / 2u)));
}
