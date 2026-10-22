//! The irradiance probe volume (ADR-0023): a 32 x 32 x 32 grid of probes at a fixed spacing
//! that follows the camera in whole probe steps, each holding an octahedral irradiance map and
//! an octahedral visibility map (mean distance and its square). `probe_gather.wgsl` reads the
//! eight probes around a point; `probe_update.wgsl` refreshes a slice of them per frame.
//!
//! Storage is toroidal: world probe `w` (probe steps) lives in slot `w mod 32` per axis, so a
//! scroll moves no data. Each slot's state word carries a tag of `floor(w / 32)` per axis, so a
//! slot whose tag is another probe's reads as never traced (and gathers with zero weight) until
//! its first update rewrites it.
//!
//! The atlases are storage buffers of `u32` words, one buffer for both maps and the state (the
//! update reads a probe's previous texels and blends into them, a read-modify-write that
//! storage textures of these formats do not offer everywhere):
//!
//! - irradiance: probe `s`, texel `x + 8 y` of its 8 x 8 tile (6 x 6 plus a one-texel border)
//!   at words `2 (64 s + x + 8 y)` and `+ 1`: rgb as three `f16` (`pack2x16float`), then a pad;
//! - visibility: words `VIS_BASE + 256 s + x + 16 y` of its 16 x 16 tile (14 x 14 plus the
//!   border): mean distance and mean squared distance in voxels, two `f16`;
//! - state: word `STATE_BASE + s`: bit 0 inside rock, bits 1..8 the age in updates (capped at
//!   127; 0 is never traced), bits 8..32 the tag (`floor(w / 32) & 0xFF` per axis, x lowest).
//!
//! A border texel repeats the interior texel the octahedral fold puts next to it, so a bilinear
//! tap anywhere in the interior's span reads only its own tile.

use crate::readback;

/// Probes on each axis of the volume.
pub const PROBES_PER_AXIS: u32 = 32;
/// Probes in the volume.
pub const PROBE_COUNT: u32 = PROBES_PER_AXIS * PROBES_PER_AXIS * PROBES_PER_AXIS;
/// The spacing between probes in voxels, by default (1.6 m at 10 cm voxels).
pub const DEFAULT_PROBE_SPACING: u32 = 16;
/// The irradiance map's interior edge in texels; the tile adds a border on each side.
pub const IRRADIANCE_TEXELS: u32 = 6;
/// The visibility map's interior edge in texels.
pub const VISIBILITY_TEXELS: u32 = 14;
/// Rays each updated probe traces per update.
pub const RAYS_PER_PROBE: u32 = 64;
/// The round robin: each frame updates the probes of one of this many slices.
pub const SLICES: u32 = 8;
/// Probes of one slice.
pub const SLICE_PROBES: u32 = PROBE_COUNT / SLICES;
/// Probes an edit can add to one frame's update beyond its slice; the rest wait for theirs.
pub const MAX_TOUCHED: u32 = 4096;
/// Entries of one frame's update list at most.
pub const MAX_UPDATES: u32 = SLICE_PROBES + MAX_TOUCHED;
/// Storage buffers the probe passes bind beyond the frame's world group.
pub const PROBE_STORAGE_BUFFERS: u32 = 2;
/// Updates during which a probe blends at [`YOUNG_HYSTERESIS`] after its first.
pub const YOUNG_UPDATES: u32 = 8;
/// Hysteresis of a young or edit-touched probe's blend.
pub const YOUNG_HYSTERESIS: f32 = 0.5;
/// Hysteresis of a settled probe's blend.
pub const HYSTERESIS: f32 = 0.97;
/// The age a probe's state word saturates at.
pub const MAX_AGE: u32 = 127;
/// The update-list flag of a probe an edit touched: it blends at [`YOUNG_HYSTERESIS`] and its
/// age restarts at 1.
pub const TOUCHED_BIT: u32 = 1 << 31;

/// Texels of one irradiance tile and one visibility tile, borders included.
const IRR_TILE: u32 = (IRRADIANCE_TEXELS + 2) * (IRRADIANCE_TEXELS + 2);
const VIS_TILE: u32 = (VISIBILITY_TEXELS + 2) * (VISIBILITY_TEXELS + 2);
/// Word offsets of the visibility atlas and the state in the data buffer.
pub const VIS_BASE: u32 = PROBE_COUNT * IRR_TILE * 2;
pub const STATE_BASE: u32 = VIS_BASE + PROBE_COUNT * VIS_TILE;
/// Words of the data buffer.
const DATA_WORDS: u32 = STATE_BASE + PROBE_COUNT;
/// Words of the scratch buffer: the update list, then per entry and ray its radiance and
/// signed distance as four `f16`.
const SCRATCH_WORDS: u32 = MAX_UPDATES + MAX_UPDATES * RAYS_PER_PROBE * 2;

/// The probe passes' uniform block, field for field with `ProbeParams` in
/// `probe_gather.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ProbeUniform {
    /// The volume's lowest world probe on each axis.
    pub origin: [i32; 3],
    /// Voxels between probes.
    pub spacing: u32,
    /// Entries of this frame's update list.
    pub updates: u32,
    pub _pad: [u32; 3],
}

/// One probe as [`ProbeVolume::write_test_irradiance`] presets it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TestProbe {
    /// Every irradiance texel's value.
    pub irradiance: [f32; 3],
    /// The inside flag.
    pub inside: bool,
}

/// One probe read back by [`ProbeVolume::read_probe`].
#[derive(Clone, Debug, PartialEq)]
pub struct ProbeRead {
    pub state: u32,
    /// The irradiance tile, `x + 8 y`, borders included.
    pub irradiance: Vec<[f32; 3]>,
    /// The visibility tile, `x + 16 y`, borders included: mean and mean squared distance.
    pub visibility: Vec<[f32; 2]>,
}

impl ProbeRead {
    pub fn inside(&self) -> bool {
        self.state & 1 != 0
    }

    pub fn age(&self) -> u32 {
        (self.state >> 1) & MAX_AGE
    }

    /// The interior irradiance texels, row by row.
    pub fn interior_irradiance(&self) -> Vec<[f32; 3]> {
        let e = IRRADIANCE_TEXELS + 2;
        (1..=IRRADIANCE_TEXELS)
            .flat_map(|y| (1..=IRRADIANCE_TEXELS).map(move |x| (x + e * y) as usize))
            .map(|i| self.irradiance[i])
            .collect()
    }
}

/// The volume's GPU buffers and its placement. One per [`crate::Frame`], shared by its views.
pub struct ProbeVolume {
    /// Both atlases and the state words.
    data: wgpu::Buffer,
    /// The frame's update list and the traced rays.
    scratch: wgpu::Buffer,
    uniform: wgpu::Buffer,
    spacing: u32,
    /// A fixed origin, or `None` to follow the camera.
    pinned: Option<[i32; 3]>,
    /// Whether frames update probes; off holds what tests preset.
    updates: bool,
}

/// `w` divided by `n`, rounded toward negative infinity.
fn floor_div(w: i32, n: i32) -> i32 {
    w.div_euclid(n)
}

/// The storage slot of world probe `w`.
pub fn probe_slot(w: [i32; 3]) -> u32 {
    let m = w.map(|v| v.rem_euclid(PROBES_PER_AXIS as i32) as u32);
    m[0] + PROBES_PER_AXIS * (m[1] + PROBES_PER_AXIS * m[2])
}

/// The tag a slot holding world probe `w` carries in its state word's bits 8..32.
pub fn probe_tag(w: [i32; 3]) -> u32 {
    let t = w.map(|v| (floor_div(v, PROBES_PER_AXIS as i32) as u32) & 0xFF);
    t[0] | t[1] << 8 | t[2] << 16
}

/// The state word of world probe `w` at `age` updates, inside rock or not.
pub fn state_word(w: [i32; 3], age: u32, inside: bool) -> u32 {
    u32::from(inside) | age.min(MAX_AGE) << 1 | probe_tag(w) << 8
}

/// `v` as IEEE binary16 bits, rounded to nearest (ties away from zero); values past the
/// largest finite `f16` become infinity and values under the smallest normal flush to zero.
fn f32_to_f16(v: f32) -> u16 {
    let bits = v.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xFF) as i32 - 127 + 15;
    if v.is_nan() {
        return 0x7E00;
    }
    if exp >= 31 {
        return sign | 0x7C00;
    }
    if exp <= 0 {
        return sign;
    }
    let mant = bits & 0x7F_FFFF;
    let h = (u32::from(sign) | (exp as u32) << 10 | mant >> 13) + ((mant >> 12) & 1);
    h as u16
}

fn pack2x16(a: f32, b: f32) -> u32 {
    u32::from(f32_to_f16(a)) | u32::from(f32_to_f16(b)) << 16
}

fn unpack2x16(w: u32) -> [f32; 2] {
    [
        readback::f16_to_f32((w & 0xFFFF) as u16),
        readback::f16_to_f32((w >> 16) as u16),
    ]
}

/// The mean distance a test probe's visibility map holds: far enough that no gathered point
/// lies behind it.
const TEST_VISIBLE_DISTANCE: f32 = 200.0;

impl ProbeVolume {
    /// A volume of probes `spacing` voxels apart, every slot never traced.
    pub fn new(device: &wgpu::Device, spacing: u32) -> Self {
        assert!(spacing > 0, "probe spacing 0");
        let storage = |label, words: u32| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: u64::from(words) * 4,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        Self {
            data: storage("probe atlases", DATA_WORDS),
            scratch: storage("probe scratch", SCRATCH_WORDS),
            uniform: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("probe params"),
                size: size_of::<ProbeUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            spacing,
            pinned: None,
            updates: true,
        }
    }

    pub fn spacing(&self) -> u32 {
        self.spacing
    }

    /// The atlases and state (see the module docs for the layout).
    pub fn data(&self) -> &wgpu::Buffer {
        &self.data
    }

    pub(crate) fn scratch(&self) -> &wgpu::Buffer {
        &self.scratch
    }

    pub(crate) fn uniform(&self) -> &wgpu::Buffer {
        &self.uniform
    }

    /// Bytes the volume's buffers occupy: the atlases, the state, the update list, the rays
    /// and the uniform.
    pub fn bytes(&self) -> u64 {
        self.data.size() + self.scratch.size() + self.uniform.size()
    }

    /// Bytes of the two atlases alone.
    pub fn atlas_bytes(&self) -> u64 {
        u64::from(STATE_BASE) * 4
    }

    /// Bytes of the state words.
    pub fn state_bytes(&self) -> u64 {
        u64::from(PROBE_COUNT) * 4
    }

    /// Fixes the volume's lowest world probe at `origin` (probe steps), or with `None` lets it
    /// follow the camera.
    pub fn set_origin(&mut self, origin: Option<[i32; 3]>) {
        self.pinned = origin;
    }

    /// Turns the per-frame update off (tests preset the probes) or on (the default).
    pub fn set_updates(&mut self, on: bool) {
        self.updates = on;
    }

    pub fn updates(&self) -> bool {
        self.updates
    }

    /// The volume's lowest world probe for a camera at `eye` (voxels): the probe cell holding
    /// the eye is cell 15 of 31 on each axis, unless the origin is pinned.
    pub fn origin_for(&self, eye: [f32; 3]) -> [i32; 3] {
        self.pinned.unwrap_or_else(|| {
            let s = self.spacing as f32;
            eye.map(|v| ((v - 0.5) / s).floor() as i32 - (PROBES_PER_AXIS as i32 / 2 - 1))
        })
    }

    /// World probe `w`'s position in voxels: the centre of voxel `w * spacing`.
    pub fn probe_position(&self, w: [i32; 3]) -> [f32; 3] {
        w.map(|v| (v * self.spacing as i32) as f32 + 0.5)
    }

    /// The world probes of the volume at `origin` whose cells an edit box of voxels
    /// `min ..= max` touches: those within one spacing of the box on every axis.
    pub fn touched(&self, origin: [i32; 3], min: [i32; 3], max: [i32; 3]) -> Vec<[i32; 3]> {
        let s = self.spacing as i32;
        // Probe w at w s + 0.5 touches the box when w s + 0.5 - s < max + 1 and
        // w s + 0.5 + s > min.
        let lo: [i32; 3] = std::array::from_fn(|k| floor_div(min[k] - s, s).max(origin[k]));
        let hi: [i32; 3] = std::array::from_fn(|k| {
            floor_div(max[k] + s, s).min(origin[k] + PROBES_PER_AXIS as i32 - 1)
        });
        let mut out = Vec::new();
        for z in lo[2]..=hi[2] {
            for y in lo[1]..=hi[1] {
                for x in lo[0]..=hi[0] {
                    out.push([x, y, z]);
                }
            }
        }
        out
    }

    /// The update list of `frame` for the volume at `origin`: the probes of slice
    /// `frame % SLICES` (slots `SLICES k + slice`) and the probes `touched` names, each slot
    /// once, a touched one flagged with [`TOUCHED_BIT`]; at most [`MAX_TOUCHED`] touched ones
    /// outside the slice.
    pub fn update_list(&self, frame: u32, touched: &[[i32; 3]]) -> Vec<u32> {
        let slice = frame % SLICES;
        let mut flagged = vec![false; PROBE_COUNT as usize];
        let mut extra = Vec::new();
        for &w in touched {
            let s = probe_slot(w);
            if flagged[s as usize] {
                continue;
            }
            if s % SLICES != slice {
                if extra.len() == MAX_TOUCHED as usize {
                    continue;
                }
                extra.push(s | TOUCHED_BIT);
            }
            flagged[s as usize] = true;
        }
        let mut list: Vec<u32> = (0..SLICE_PROBES)
            .map(|k| {
                let s = SLICES * k + slice;
                if flagged[s as usize] {
                    s | TOUCHED_BIT
                } else {
                    s
                }
            })
            .collect();
        list.extend(extra);
        list
    }

    /// Presets every probe of the volume at `origin`: irradiance and inside flag from `f` of
    /// its world probe, visibility clear to 200 voxels, the age at its cap.
    pub fn write_test_irradiance(
        &self,
        queue: &wgpu::Queue,
        origin: [i32; 3],
        f: impl Fn([i32; 3]) -> TestProbe,
    ) {
        let mut words = vec![0u32; DATA_WORDS as usize];
        let vis = pack2x16(
            TEST_VISIBLE_DISTANCE,
            TEST_VISIBLE_DISTANCE * TEST_VISIBLE_DISTANCE,
        );
        let n = PROBES_PER_AXIS as i32;
        for z in 0..n {
            for y in 0..n {
                for x in 0..n {
                    let w = [origin[0] + x, origin[1] + y, origin[2] + z];
                    let p = f(w);
                    let s = probe_slot(w) as usize;
                    let rg = pack2x16(p.irradiance[0], p.irradiance[1]);
                    let ba = pack2x16(p.irradiance[2], 0.0);
                    for t in 0..IRR_TILE as usize {
                        words[2 * (IRR_TILE as usize * s + t)] = rg;
                        words[2 * (IRR_TILE as usize * s + t) + 1] = ba;
                    }
                    for t in 0..VIS_TILE as usize {
                        words[VIS_BASE as usize + VIS_TILE as usize * s + t] = vis;
                    }
                    words[STATE_BASE as usize + s] = state_word(w, MAX_AGE, p.inside);
                }
            }
        }
        queue.write_buffer(&self.data, 0, bytemuck::cast_slice(&words));
    }

    /// Every slot's state word, by slot.
    pub fn read_states(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Vec<u32> {
        let bytes = readback::read_buffer(
            device,
            queue,
            &self.data,
            u64::from(STATE_BASE) * 4,
            u64::from(PROBE_COUNT) * 4,
        );
        bytemuck::cast_slice(&bytes).to_vec()
    }

    /// Whether `state` is world probe `w`'s, traced at least once and not inside rock.
    pub fn live(state: u32, w: [i32; 3]) -> bool {
        state >> 8 == probe_tag(w) && (state >> 1) & MAX_AGE > 0 && state & 1 == 0
    }

    /// Probes of the volume at `origin` that are live (see [`ProbeVolume::live`]).
    pub fn live_count(&self, states: &[u32], origin: [i32; 3]) -> u32 {
        let n = PROBES_PER_AXIS as i32;
        let mut live = 0;
        for z in 0..n {
            for y in 0..n {
                for x in 0..n {
                    let w = [origin[0] + x, origin[1] + y, origin[2] + z];
                    live += u32::from(Self::live(states[probe_slot(w) as usize], w));
                }
            }
        }
        live
    }

    /// Slot `slot`'s state and both tiles.
    pub fn read_probe(&self, device: &wgpu::Device, queue: &wgpu::Queue, slot: u32) -> ProbeRead {
        let read = |word: u32, words: u32| -> Vec<u32> {
            let bytes = readback::read_buffer(
                device,
                queue,
                &self.data,
                u64::from(word) * 4,
                u64::from(words) * 4,
            );
            bytemuck::cast_slice(&bytes).to_vec()
        };
        let irr = read(2 * IRR_TILE * slot, 2 * IRR_TILE);
        let vis = read(VIS_BASE + VIS_TILE * slot, VIS_TILE);
        let state = read(STATE_BASE + slot, 1)[0];
        ProbeRead {
            state,
            irradiance: irr
                .chunks(2)
                .map(|p| {
                    let [r, g] = unpack2x16(p[0]);
                    let [b, _] = unpack2x16(p[1]);
                    [r, g, b]
                })
                .collect(),
            visibility: vis.iter().map(|&w| unpack2x16(w)).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f16_round_trips() {
        for v in [
            0.0f32,
            1.0,
            std::f32::consts::PI,
            0.5,
            50.0,
            200.0,
            40000.0,
            0.001,
        ] {
            let back = readback::f16_to_f32(f32_to_f16(v));
            assert!((back - v).abs() <= v * 1e-3, "{v} -> {back}");
        }
        assert_eq!(f32_to_f16(1.0e6), 0x7C00);
    }

    #[test]
    fn slots_wrap_and_tags_tell_the_wraps_apart() {
        assert_eq!(probe_slot([0, 0, 0]), 0);
        assert_eq!(probe_slot([33, -1, 64]), 1 + 32 * 31);
        assert_eq!(probe_slot([-32, 0, 0]), 0);
        assert_ne!(probe_tag([-32, 0, 0]), probe_tag([0, 0, 0]));
        assert_eq!(probe_tag([31, 31, 31]), 0);
        assert_eq!(probe_tag([32, 0, -1]), 1 | 0xFF << 16);
        let s = state_word([5, -40, 70], 3, true);
        assert_eq!(s & 1, 1);
        assert_eq!((s >> 1) & MAX_AGE, 3);
        assert!(!ProbeVolume::live(
            state_word([5, 6, 7], 0, false),
            [5, 6, 7]
        ));
        assert!(ProbeVolume::live(
            state_word([5, 6, 7], 1, false),
            [5, 6, 7]
        ));
        assert!(!ProbeVolume::live(
            state_word([5, 6, 7], 1, false),
            [37, 6, 7]
        ));
        assert!(!ProbeVolume::live(
            state_word([5, 6, 7], 9, true),
            [5, 6, 7]
        ));
    }
}
