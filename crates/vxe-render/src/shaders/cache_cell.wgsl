// The emissive table's cell (ADR-0043): its layout and key helpers, compiled with march.wgsl and,
// without it, with the table's passes that read no world (cache_sweep.wgsl, cache_blend.wgsl).
//
// The emissive table (ADR-0043, `Lighting::table_cells`): `params.cache_mask + 1` cells of
// CACHE_WORDS u32s, in a storage buffer of its own. A cell: the key in words 0 and 1
// (`cache_key`; word 1 is 0 in a free cell), the count of samples, the three sums of the
// absolute emissive irradiance in Q17.15, then the kept emitter's face key (two words) and its
// packed luminance (light.wgsl's `cache_offer`, the guard's), a frame stamp, the frame the cell
// was last seen (CACHE_SEEN) and its blended mean (CACHE_BLEND). A key is a voxel and an entry
// face (2 axis + 1 toward +axis) and lives in the first of CACHE_PROBES slots from `cache_slot`
// holding it. The table's counters live in a second buffer (`CacheStats`).
const CACHE_WORDS: u32 = 12u;
const CACHE_PROBES: u32 = 16u;
const CACHE_Q: f32 = 32768.0;
// The cell's word of the count; the sums follow it.
const CACHE_COUNT: u32 = 2u;
// The cell's stamp word: CACHE_REOPENED, set where an edit box or the guard reopened the cell
// and cleared once the fill has brought it to C_FILL; CACHE_PRELOADING, set while the cell
// stands in the preload's list (cache_fill.wgsl) and cleared when it leaves it; over the frame
// (its low 30 bits, CACHE_STAMP_FRAME) the light pass last listed it for the fill.
const CACHE_STAMP: u32 = 9u;
const CACHE_REOPENED: u32 = 0x80000000u;
const CACHE_PRELOADING: u32 = 0x40000000u;
const CACHE_STAMP_FRAME: u32 = 0x3FFFFFFFu;
// The cell's seen word: `params.frame + 1` of the last frame a lighting pixel's hit took the
// cell (light.wgsl's `cache_seen`) or a seen cell's blend read it (cache_blend.wgsl); 0 never.
// The age-out (cache_sweep.wgsl) frees a cell beyond its radius unseen for its age.
const CACHE_SEEN: u32 = 10u;
// The cell's blended mean (cache_blend.wgsl; `blend_unpack`); the temporal pass reads it.
const CACHE_BLEND: u32 = 11u;

fn cache_mix(v: vec3<i32>, face: u32, a: u32, b: u32) -> u32 {
    let c = bitcast<vec3<u32>>(v);
    var h = (c.x * a) ^ (c.y * 0xD8163841u) ^ (c.z * b) ^ ((face + 1u) * 0x165667B1u);
    h ^= h >> 15u;
    h *= 0x2C1B3C6Du;
    h ^= h >> 12u;
    h *= 0x297A2D39u;
    h ^= h >> 15u;
    return h;
}

// The key's two words (`marcher::cache_key`): each coordinate's low 20 bits (the key wraps
// every 2^20 voxels, past any light radius), x then y's low 12 bits in word 0; y's high 8
// bits, z, the face in bits 28..31 and a valid bit 31 in word 1, so word 1 is never 0.
fn cache_key(v: vec3<i32>, face: u32) -> vec2<u32> {
    let c = bitcast<vec3<u32>>(v) & vec3<u32>(0xFFFFFu);
    return vec2<u32>(c.x | (c.y << 20u), (c.y >> 12u) | (c.z << 8u) | (face << 28u) | 0x80000000u);
}

// The voxel of a key's two words: each 20-bit coordinate sign-extended.
fn cache_key_voxel(k: vec2<u32>) -> vec3<i32> {
    let x = k.x & 0xFFFFFu;
    let y = (k.x >> 20u) | ((k.y & 0xFFu) << 12u);
    let z = (k.y >> 8u) & 0xFFFFFu;
    return bitcast<vec3<i32>>(vec3<u32>(x, y, z) << vec3<u32>(12u)) >> vec3<u32>(12u);
}

// The blend word's exponent bias and the smallest colour it holds, 2^-26: under it, black.
const BLEND_EXP_BIAS: i32 = 26;
const BLEND_MIN: f32 = 1.4901161e-8;

// A blend word (`marcher::blend_unpack`): nine bits of each channel's square root over a shared
// power of two, the exponent in the top five. Display levels are about even in the square root,
// so a step is about half a level through any channel, at any share of the largest.
fn blend_unpack(word: u32) -> vec3<f32> {
    let q = vec3<f32>(vec3<u32>(word & 511u, (word >> 9u) & 511u, (word >> 18u) & 511u)) / 511.0;
    return q * q * ldexp(1.0, i32(word >> 27u) - BLEND_EXP_BIAS);
}
