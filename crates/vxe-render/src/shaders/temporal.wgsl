// The temporal pass: accumulates each pixel's irradiance and its moments over frames by
// reprojecting the hit through the previous frame's views (the world does not move; only the
// views and the edits do), and kills history where the world was edited. Compiled after
// march.wgsl, whose bindings (group 0) and hit-record helpers it uses.
//
// Bindings, group 2:
//   0  texture_2d<f32>   hit_pos_in      this frame's hit positions
//   1  texture_2d<u32>   hit_info_in     this frame's hit records
//   2  texture_2d<f32>   prev_pos_in     last frame's hit positions
//   3  texture_2d<u32>   prev_info_in    last frame's hit records
//   4  texture_2d<f32>   irradiance_in   this frame's irradiance from the light pass
//   5  texture_2d<f32>   prev_accum_in   last frame's accumulated irradiance
//   6  texture_2d<f32>   prev_moments_in last frame's moments: luminance, its square, age
//   7  storage texture   accum_out       rgba16float: the accumulated irradiance, rgb
//   8  storage texture   moments_out     rgba16float: first and second moments of the
//                                        irradiance's luminance, then the age: frames of
//                                        history reused, 0 after a reset
//   9  uniform           edits           the frame's edit boxes (see `Edits`)
//
// A hit is projected through the previous frame's view of the same index to a fractional
// pixel; each of its four bilinear taps is accepted when that pixel hit, with the same
// material, a normal within HISTORY_COS of this one, and a position within
// `HISTORY_DIST + HISTORY_DIST_PER_T t` voxels of this hit. The accepted taps' history,
// moments and age are blended by their bilinear weights; with none accepted, with no
// history (`Params.history` bit 0 clear), or with the hit inside an edit box dilated by
// one voxel, the pixel resets: age 0, accumulation this frame's sample. Otherwise the age is
// the taps' age plus one, capped at AGE_MAX, and the new sample is blended in at
// `max(ALPHA, 1 / (age + 1))`, the moments alike.

@group(2) @binding(0) var hit_pos_in: texture_2d<f32>;
@group(2) @binding(1) var hit_info_in: texture_2d<u32>;
@group(2) @binding(2) var prev_pos_in: texture_2d<f32>;
@group(2) @binding(3) var prev_info_in: texture_2d<u32>;
@group(2) @binding(4) var irradiance_in: texture_2d<f32>;
@group(2) @binding(5) var prev_accum_in: texture_2d<f32>;
@group(2) @binding(6) var prev_moments_in: texture_2d<f32>;
@group(2) @binding(7) var accum_out: texture_storage_2d<rgba16float, write>;
@group(2) @binding(8) var moments_out: texture_storage_2d<rgba16float, write>;

// Up to MAX_EDITS boxes, inclusive voxel bounds: box i is `boxes[2 i]` to `boxes[2 i + 1]`.
struct Edits {
    count: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    boxes: array<vec4<i32>, 128>,
};

@group(2) @binding(9) var<uniform> edits: Edits;

const ALPHA: f32 = 0.2;
const AGE_MAX: f32 = 2047.0;
const HISTORY_COS: f32 = 0.8;
const HISTORY_DIST: f32 = 0.5;
const HISTORY_DIST_PER_T: f32 = 0.03;

// Whether `p` lies inside an edit box dilated by one voxel: the continuous span of voxels
// `min - 1 ..= max + 1`, faces included.
fn in_edit(p: vec3<f32>) -> bool {
    for (var i = 0u; i < edits.count; i++) {
        let lo = vec3<f32>(edits.boxes[2u * i].xyz - 1);
        let hi = vec3<f32>(edits.boxes[2u * i + 1u].xyz + 2);
        if (all(p >= lo) && all(p <= hi)) {
            return true;
        }
    }
    return false;
}

@compute @workgroup_size(8, 8, 1)
fn temporal(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.size.x || gid.y >= params.size.y) {
        return;
    }
    let px = vec2<i32>(gid.xy);
    let rec = unpack_hit(textureLoad(hit_pos_in, px, 0), textureLoad(hit_info_in, px, 0));
    let sample = textureLoad(irradiance_in, px, 0).xyz;
    let l = luminance(sample);
    var accum = sample;
    var moments = vec2<f32>(l, l * l);
    var age = 0.0;
    if (rec.found && (params.history & 1u) != 0u && !in_edit(rec.p)) {
        let v = params.prev_views[view_of(gid.xy)];
        let q = project_prev(v, rec.p) - 0.5;
        let base = vec2<i32>(floor(q));
        let f = q - floor(q);
        var w_sum = 0.0;
        var h = vec3<f32>(0.0);
        var m = vec3<f32>(0.0);
        let tol = HISTORY_DIST + HISTORY_DIST_PER_T * rec.t;
        for (var i = 0; i < 4; i++) {
            let o = vec2<i32>(i & 1, i >> 1);
            let tap = base + o;
            if (any(tap < vec2<i32>(0)) || any(tap >= vec2<i32>(params.size))) {
                continue;
            }
            let prev = unpack_hit(textureLoad(prev_pos_in, tap, 0), textureLoad(prev_info_in, tap, 0));
            if (!prev.found || prev.material != rec.material || dot(prev.n, rec.n) < HISTORY_COS
                || distance(prev.p, rec.p) > tol) {
                continue;
            }
            let wx = select(1.0 - f.x, f.x, o.x == 1);
            let wy = select(1.0 - f.y, f.y, o.y == 1);
            let w = wx * wy;
            w_sum += w;
            h += w * textureLoad(prev_accum_in, tap, 0).xyz;
            m += w * textureLoad(prev_moments_in, tap, 0).xyz;
        }
        if (w_sum > 1e-3) {
            h /= w_sum;
            m /= w_sum;
            age = min(floor(m.z + 0.5) + 1.0, AGE_MAX);
            let a = max(ALPHA, 1.0 / (age + 1.0));
            accum = mix(h, sample, a);
            moments = mix(m.xy, vec2<f32>(l, l * l), a);
        }
    }
    textureStore(accum_out, px, vec4<f32>(accum, 0.0));
    textureStore(moments_out, px, vec4<f32>(moments, age, 0.0));
}
