//! The probe volume (ADR-0023), as ADR-0043's lamp-bounce channel: an N x N x N grid of probes
//! (N a power of two, [`probes_per_axis`]) at a fixed spacing that follows the camera in whole
//! probe steps, each holding an octahedral irradiance map and an octahedral visibility map
//! (mean distance and its square). The probe rays see the headlamp alone; only the probes
//! within [`Lighting::lamp_reach`](crate::Lighting::lamp_reach) of the lamp are updated, and
//! the gather reads the others as zero. `probe_gather.wgsl` reads the eight probes around a
//! point; `probe_update.wgsl` refreshes a slice of them per frame.
//!
//! Storage is toroidal: world probe `w` (probe steps) lives in slot `w mod N` per axis, so a
//! scroll moves no data. Each slot's state word carries a tag of `floor(w / N)` per axis, so a
//! slot whose tag is another probe's reads as never traced (and gathers with zero weight) until
//! its first update rewrites it.
//!
//! The atlases are storage buffers of `u32` words, one buffer for both maps and the state (the
//! update reads a probe's previous texels and blends into them, a read-modify-write that
//! storage textures of these formats do not offer everywhere):
//!
//! - irradiance: probe `s`, texel `x + 8 y` of its 8 x 8 tile (6 x 6 plus a one-texel border)
//!   at words `2 (64 s + x + 8 y)` and `+ 1`: rgb as three `f16` (`pack2x16float`), then a pad;
//! - visibility: words `vis_base + 256 s + x + 16 y` of its 16 x 16 tile (14 x 14 plus the
//!   border): mean distance and mean squared distance in voxels, two `f16`;
//! - state: word `state_base + s`: bit 0 inside rock, bits 1..8 the age in updates (capped at
//!   127; 0 is never traced), bits 8..32 the tag (`floor(w / N) & 0xFF` per axis, x lowest).
//!
//! A border texel repeats the interior texel the octahedral fold puts next to it, so a bilinear
//! tap anywhere in the interior's span reads only its own tile.

use crate::readback;

/// Probes on each axis of the volume at most.
pub(crate) const MAX_PROBES_PER_AXIS: u32 = 32;
/// Probes in the largest volume.
pub(crate) const MAX_PROBE_COUNT: u32 =
    MAX_PROBES_PER_AXIS * MAX_PROBES_PER_AXIS * MAX_PROBES_PER_AXIS;
/// The spacing between probes in voxels, by default (1.6 m at 10 cm voxels).
pub const DEFAULT_PROBE_SPACING: u32 = 16;
/// The irradiance map's interior edge in texels; the tile adds a border on each side.
pub(crate) const IRRADIANCE_TEXELS: u32 = 6;
/// The visibility map's interior edge in texels.
pub(crate) const VISIBILITY_TEXELS: u32 = 14;
/// Rays each updated probe traces per update at most, and by default
/// ([`crate::Lighting::probe_rays`]).
pub const RAYS_PER_PROBE: u32 = 64;
/// The round robin: each frame updates the probes of one of this many slices, by default
/// ([`crate::Lighting::probe_slices`]); the fewest slices allowed.
pub const SLICES: u32 = 8;
/// The most slices allowed: one probe in 32 a frame.
pub(crate) const MAX_SLICES: u32 = 32;
/// Probes an edit or the lamp's reach can add to one frame's update beyond its slice; the rest
/// wait for a later frame.
pub(crate) const MAX_TOUCHED: u32 = 4096;
/// Entries of one frame's update list at most: the largest volume's window slice and
/// [`MAX_TOUCHED`].
pub(crate) const MAX_UPDATES: u32 = MAX_PROBE_COUNT / WINDOW_SLICES;
/// Slices while a light-change window is open: one probe in 4 a frame, whatever
/// [`crate::Lighting::probe_slices`] says.
pub(crate) const WINDOW_SLICES: u32 = 4;
/// Visits of every probe a light-change window lasts.
pub(crate) const WINDOW_VISITS: u32 = 4;
/// Encoded frames with a probe update a light-change window lasts (16). A lamp switch-on or a
/// change of its parameters opens one, after clearing every probe's state, so the channel
/// fills from zero at one probe in 4 a frame.
pub const WINDOW_FRAMES: u32 = WINDOW_SLICES * WINDOW_VISITS;
/// Storage buffers the probe passes bind beyond the frame's world group.
pub(crate) const PROBE_STORAGE_BUFFERS: u32 = 2;
/// The age a probe's state word saturates at.
pub const MAX_AGE: u32 = 127;
/// The update-list flag of a restarted probe (an edit touched it, or it entered the lamp's
/// reach since the last update): it blends at hysteresis 0 and its age restarts at 1.
pub(crate) const TOUCHED_BIT: u32 = 1 << 31;
/// The update-list flag of a probe the lamp moved past since its last visit (the lamp's
/// position or direction changed while the probe lay within its reach): it blends at
/// [`crate::Lighting::probe_move_blend`] and its age restarts at 1, so it runs a fresh mean
/// once the lamp is still.
pub(crate) const MOVED_BIT: u32 = 1 << 30;

/// The lamp's reach in voxels by default ([`crate::Lighting::lamp_reach`]): beyond it the
/// lamp's bounce at a probe falls under 1 % of the direct irradiance on the wall the lamp
/// lights at arm's length (a reading, `lamp_reach_readings`), rounded up to whole default
/// spacings.
pub(crate) const LAMP_REACH: f32 = 64.0;
/// A probe's blend while the lamp moves, by default ([`crate::Lighting::probe_move_blend`]): the
/// lowest flicker of the flight of 0.5, 0.8 and 0.9 (`flicker_readings`), against the slowest
/// follow of a lamp step (`lamp_step_readings`).
pub const DEFAULT_H_MOVE: f32 = 0.9;

/// The probes per axis of a volume covering `reach` voxels at `spacing`: the smallest power of
/// two, at least 4 and at most [`MAX_PROBES_PER_AXIS`], whose half-side covers the reach; the
/// largest for a reach of 0 or less (no cut) or past the largest volume.
pub(crate) fn probes_per_axis(reach: f32, spacing: u32) -> u32 {
    let half = reach / spacing.max(1) as f32;
    if half.is_nan() || half <= 0.0 || half > (MAX_PROBES_PER_AXIS / 2) as f32 {
        return MAX_PROBES_PER_AXIS;
    }
    (2 * (half.ceil() as u32).max(2)).next_power_of_two()
}

/// Texels of one irradiance tile and one visibility tile, borders included.
const IRR_TILE: u32 = (IRRADIANCE_TEXELS + 2) * (IRRADIANCE_TEXELS + 2);
const VIS_TILE: u32 = (VISIBILITY_TEXELS + 2) * (VISIBILITY_TEXELS + 2);
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
    /// Rays each updated probe traces, 1 ..= [`RAYS_PER_PROBE`].
    pub rays: u32,
    /// Bits 0..8 the visits a probe freezes at ([`crate::Lighting::probe_freeze`]).
    pub steady: u32,
    /// Probes per axis.
    pub axis: u32,
    /// The lamp's reach in voxels; 0 is no cut (a preset volume).
    pub reach: f32,
    /// A probe's blend in reach while the lamp moves.
    pub h_move: f32,
    pub _pad: [u32; 2],
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
    axis: u32,
    /// By slot, whether it was listed within the lamp's reach at the last update (a slot
    /// entering the reach is listed restarted), and whether the lamp moved while it lay in
    /// reach since its last listing (it is listed with [`MOVED_BIT`]).
    in_reach: std::sync::Mutex<(Vec<bool>, Vec<bool>)>,
    /// A fixed origin, or `None` to follow the camera.
    pinned: Option<[i32; 3]>,
    /// Whether frames update probes; off holds what tests preset.
    updates: bool,
}

/// `w` divided by `n`, rounded toward negative infinity.
fn floor_div(w: i32, n: i32) -> i32 {
    w.div_euclid(n)
}

/// A bijection of a volume's slot numbers (`count` of them, a power of two) whose low bits
/// depend on every bit of the slot: two xorshift-multiply rounds (an xor with a right shift,
/// and a product with an odd constant modulo the count, are each invertible).
fn mix_slot(slot: u32, count: u32) -> u32 {
    let mask = count - 1;
    let mut x = slot & mask;
    x ^= x >> 7;
    x = x.wrapping_mul(0x6B4D) & mask;
    x ^= x >> 8;
    x = x.wrapping_mul(0x2C9B) & mask;
    x ^= x >> 7;
    x
}

/// The slice of `slices` (a power of two) that slot `slot` of a volume of `count` slots
/// updates in: the low bits of [`mix_slot`]. Every slice holds `count / slices` slots, and
/// neighbouring probes, whose slots differ in their low bits (x) or by a multiple of the axis
/// (y, z), fall in slices spread by the hash rather than in one slice per x column, so the
/// probes lighting one view update on different frames, each with its own frame's rotation.
pub(crate) fn slot_slice(slot: u32, slices: u32, count: u32) -> u32 {
    mix_slot(slot, count) & (slices - 1)
}

/// The storage slot of world probe `w` in a volume of `axis` probes a side.
pub(crate) fn probe_slot(w: [i32; 3], axis: u32) -> u32 {
    let m = w.map(|v| v.rem_euclid(axis as i32) as u32);
    m[0] + axis * (m[1] + axis * m[2])
}

/// The tag a slot holding world probe `w` carries in its state word's bits 8..32.
pub(crate) fn probe_tag(w: [i32; 3], axis: u32) -> u32 {
    let t = w.map(|v| (floor_div(v, axis as i32) as u32) & 0xFF);
    t[0] | t[1] << 8 | t[2] << 16
}

/// The state word of world probe `w` at `age` updates, inside rock or not.
pub(crate) fn state_word(w: [i32; 3], axis: u32, age: u32, inside: bool) -> u32 {
    u32::from(inside) | age.min(MAX_AGE) << 1 | probe_tag(w, axis) << 8
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
    /// A volume of `axis` probes a side (a power of two in 4 ..= [`MAX_PROBES_PER_AXIS`]),
    /// `spacing` voxels apart, every slot never traced.
    pub fn new(device: &wgpu::Device, spacing: u32, axis: u32) -> Self {
        assert!(spacing > 0, "probe spacing 0");
        assert!(
            axis.is_power_of_two() && (4..=MAX_PROBES_PER_AXIS).contains(&axis),
            "{axis} probes a side"
        );
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
        let count = axis * axis * axis;
        let data_words = count * (IRR_TILE * 2 + VIS_TILE + 1);
        Self {
            data: storage("probe atlases", data_words),
            scratch: storage("probe scratch", SCRATCH_WORDS),
            uniform: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("probe params"),
                size: size_of::<ProbeUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            spacing,
            axis,
            in_reach: std::sync::Mutex::new((
                vec![false; count as usize],
                vec![false; count as usize],
            )),
            pinned: None,
            updates: true,
        }
    }

    pub fn spacing(&self) -> u32 {
        self.spacing
    }

    /// Probes per axis.
    pub fn axis(&self) -> u32 {
        self.axis
    }

    /// Probes in the volume.
    pub fn count(&self) -> u32 {
        self.axis * self.axis * self.axis
    }

    /// The word offset of the visibility atlas in [`ProbeVolume::data`].
    pub fn vis_base(&self) -> u32 {
        self.count() * IRR_TILE * 2
    }

    /// The word offset of the state words in [`ProbeVolume::data`].
    pub fn state_base(&self) -> u32 {
        self.vis_base() + self.count() * VIS_TILE
    }

    /// The storage slot of world probe `w`.
    pub fn slot(&self, w: [i32; 3]) -> u32 {
        probe_slot(w, self.axis)
    }

    /// The world probe slot `slot` holds in the volume at `origin`.
    pub fn slot_probe(&self, origin: [i32; 3], slot: u32) -> [i32; 3] {
        let a = self.axis;
        let m = [slot % a, (slot / a) % a, slot / (a * a)];
        std::array::from_fn(|k| origin[k] + (m[k] as i32 - origin[k]).rem_euclid(a as i32))
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
        u64::from(self.state_base()) * 4
    }

    /// Bytes of the state words.
    pub fn state_bytes(&self) -> u64 {
        u64::from(self.count()) * 4
    }

    /// Fixes the volume's lowest world probe at `origin` (probe steps), or with `None` lets it
    /// follow the camera.
    pub fn set_origin(&mut self, origin: Option<[i32; 3]>) {
        self.pinned = origin;
    }

    /// The pinned origin, if any.
    pub fn pinned_origin(&self) -> Option<[i32; 3]> {
        self.pinned
    }

    /// Turns the per-frame update off (tests preset the probes) or on (the default). With the
    /// update off the gather runs whatever the lamp and reads every preset probe.
    pub fn set_updates(&mut self, on: bool) {
        self.updates = on;
    }

    pub fn updates(&self) -> bool {
        self.updates
    }

    /// The volume's lowest world probe for a camera at `eye` (voxels): the probe cell holding
    /// the eye is cell `N / 2 - 1` of `N - 1` on each axis, unless the origin is pinned.
    pub fn origin_for(&self, eye: [f32; 3]) -> [i32; 3] {
        self.pinned.unwrap_or_else(|| {
            let s = self.spacing as f32;
            eye.map(|v| ((v - 0.5) / s).floor() as i32 - (self.axis as i32 / 2 - 1))
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
        let hi: [i32; 3] =
            std::array::from_fn(|k| floor_div(max[k] + s, s).min(origin[k] + self.axis as i32 - 1));
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

    /// Forgets which slots were within the lamp's reach, so the next update lists every probe
    /// in reach restarted (after the states are cleared).
    pub(crate) fn forget_reach(&self) {
        let mut r = self
            .in_reach
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        r.0.iter_mut().for_each(|v| *v = false);
        r.1.iter_mut().for_each(|v| *v = false);
    }

    /// The update list of `frame` for the volume at `origin` cut into `slices` slices (a power
    /// of two in [`WINDOW_SLICES`] ..= [`MAX_SLICES`]), of the probes `in_reach` accepts: the
    /// probes of slice `frame % slices` (the slots whose [`slot_slice`] it is, in slot order),
    /// then the probes `touched` names and the probes that entered the reach since the last
    /// list, each slot once, a restarted one flagged with [`TOUCHED_BIT`]; the restarted ones
    /// outside the slice up to [`MAX_TOUCHED`] and the list's [`MAX_UPDATES`] entries. A probe
    /// that entered the reach and found no room is listed restarted on a later frame. With
    /// `lamp_moved` every probe in reach is marked moved, and a listed probe marked moved since
    /// its last listing carries [`MOVED_BIT`].
    pub fn update_list(
        &self,
        origin: [i32; 3],
        frame: u32,
        slices: u32,
        touched: &[[i32; 3]],
        lamp_moved: bool,
        in_reach: impl Fn([i32; 3]) -> bool,
    ) -> Vec<u32> {
        assert!(
            slices.is_power_of_two() && (WINDOW_SLICES..=MAX_SLICES).contains(&slices),
            "{slices} slices"
        );
        let count = self.count();
        let slice = frame % slices;
        let room = MAX_TOUCHED.min(MAX_UPDATES - count / slices) as usize;
        let mut state = self
            .in_reach
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (was, moved) = &mut *state;
        let reach: Vec<bool> = (0..count)
            .map(|s| in_reach(self.slot_probe(origin, s)))
            .collect();
        let mut restart = vec![false; count as usize];
        for &w in touched {
            let s = self.slot(w) as usize;
            restart[s] = reach[s];
        }
        for s in 0..count as usize {
            restart[s] |= reach[s] && !was[s];
        }
        let mut extra = Vec::new();
        let mut list = Vec::new();
        for s in 0..count {
            let i = s as usize;
            if !reach[i] {
                was[i] = false;
                moved[i] = false;
                continue;
            }
            moved[i] |= lamp_moved;
            let flag = if restart[i] {
                TOUCHED_BIT
            } else if moved[i] {
                MOVED_BIT
            } else {
                0
            };
            if slot_slice(s, slices, count) == slice {
                list.push(s | flag);
                was[i] = true;
                moved[i] = false;
            } else if restart[i] && extra.len() < room {
                extra.push(s | flag);
                was[i] = true;
                moved[i] = false;
            }
        }
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
        let mut words = vec![0u32; (self.state_base() + self.count()) as usize];
        let vis = pack2x16(
            TEST_VISIBLE_DISTANCE,
            TEST_VISIBLE_DISTANCE * TEST_VISIBLE_DISTANCE,
        );
        let n = self.axis as i32;
        let (vis_base, state_base) = (self.vis_base() as usize, self.state_base() as usize);
        for z in 0..n {
            for y in 0..n {
                for x in 0..n {
                    let w = [origin[0] + x, origin[1] + y, origin[2] + z];
                    let p = f(w);
                    let s = self.slot(w) as usize;
                    let rg = pack2x16(p.irradiance[0], p.irradiance[1]);
                    let ba = pack2x16(p.irradiance[2], 0.0);
                    for t in 0..IRR_TILE as usize {
                        words[2 * (IRR_TILE as usize * s + t)] = rg;
                        words[2 * (IRR_TILE as usize * s + t) + 1] = ba;
                    }
                    for t in 0..VIS_TILE as usize {
                        words[vis_base + VIS_TILE as usize * s + t] = vis;
                    }
                    words[state_base + s] = state_word(w, self.axis, MAX_AGE, p.inside);
                }
            }
        }
        queue.write_buffer(&self.data, 0, bytemuck::cast_slice(&words));
    }

    /// Sets every texel of slot `slot`'s irradiance tile to `irradiance`, leaving its
    /// visibility and state.
    pub fn write_test_tile(&self, queue: &wgpu::Queue, slot: u32, irradiance: [f32; 3]) {
        let rg = pack2x16(irradiance[0], irradiance[1]);
        let ba = pack2x16(irradiance[2], 0.0);
        let words: Vec<u32> = (0..IRR_TILE).flat_map(|_| [rg, ba]).collect();
        queue.write_buffer(
            &self.data,
            u64::from(2 * IRR_TILE * slot) * 4,
            bytemuck::cast_slice(&words),
        );
    }

    /// Every slot's state word, by slot.
    pub fn read_states(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Vec<u32> {
        let bytes = readback::read_buffer(
            device,
            queue,
            &self.data,
            u64::from(self.state_base()) * 4,
            u64::from(self.count()) * 4,
        );
        bytemuck::cast_slice(&bytes).to_vec()
    }

    /// Whether `state` is world probe `w`'s, traced at least once and not inside rock.
    pub fn live(&self, state: u32, w: [i32; 3]) -> bool {
        state >> 8 == probe_tag(w, self.axis) && (state >> 1) & MAX_AGE > 0 && state & 1 == 0
    }

    /// The age of world probe `w` that its slot's `state` gives: 0 when the slot holds
    /// another probe.
    pub fn age(&self, state: u32, w: [i32; 3]) -> u32 {
        if state >> 8 == probe_tag(w, self.axis) {
            (state >> 1) & MAX_AGE
        } else {
            0
        }
    }

    /// Probes of the volume at `origin` that are live (see [`ProbeVolume::live`]).
    pub fn live_count(&self, states: &[u32], origin: [i32; 3]) -> u32 {
        (0..self.count())
            .map(|s| {
                let w = self.slot_probe(origin, s);
                u32::from(self.live(states[s as usize], w))
            })
            .sum()
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
        let vis = read(self.vis_base() + VIS_TILE * slot, VIS_TILE);
        let state = read(self.state_base() + slot, 1)[0];
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

    /// Each slice count's slices partition the slots evenly at each volume side, and the slots
    /// of one slice are spread over x: slice 0 of 8 of the largest volume holds between 384 and
    /// 640 of its 4096 slots at each value of `x mod 8` (a slice by `slot mod 8` would hold all
    /// of them at one).
    #[test]
    fn the_slices_partition_the_slots_spread_by_a_hash() {
        for axis in [8, 16, MAX_PROBES_PER_AXIS] {
            let count = axis * axis * axis;
            let mut seen = std::collections::HashSet::new();
            for s in 0..count {
                assert!(
                    seen.insert(mix_slot(s, count)),
                    "mix_slot is not a bijection at {s} of {count}"
                );
            }
            for slices in [WINDOW_SLICES, SLICES, 16, MAX_SLICES] {
                let mut n = vec![0u32; slices as usize];
                for s in 0..count {
                    n[slot_slice(s, slices, count) as usize] += 1;
                }
                assert!(
                    n.iter().all(|&c| c == count / slices),
                    "{axis}, {slices}: {n:?}"
                );
            }
        }
        let mut by_x = [0u32; 8];
        for s in (0..MAX_PROBE_COUNT).filter(|&s| slot_slice(s, SLICES, MAX_PROBE_COUNT) == 0) {
            by_x[(s % 8) as usize] += 1;
        }
        assert!(by_x.iter().all(|&c| (384..=640).contains(&c)), "{by_x:?}");
    }

    #[test]
    fn slots_wrap_and_tags_tell_the_wraps_apart() {
        assert_eq!(probe_slot([0, 0, 0], 32), 0);
        assert_eq!(probe_slot([33, -1, 64], 32), 1 + 32 * 31);
        assert_eq!(probe_slot([-32, 0, 0], 32), 0);
        assert_eq!(probe_slot([17, -1, 32], 16), 1 + 16 * 15);
        assert_ne!(probe_tag([-32, 0, 0], 32), probe_tag([0, 0, 0], 32));
        assert_eq!(probe_tag([31, 31, 31], 32), 0);
        assert_eq!(probe_tag([32, 0, -1], 32), 1 | 0xFF << 16);
        assert_eq!(probe_tag([16, 15, -1], 16), 1 | 0xFF << 16);
        let s = state_word([5, -40, 70], 32, 3, true);
        assert_eq!(s & 1, 1);
        assert_eq!((s >> 1) & MAX_AGE, 3);
    }

    /// The volume's side covers the reach with its half: 64 voxels at the spacing of 16 is 8
    /// probes a side, 65 is 16, 8 is 4, and no cut (a reach of 0) or a reach past the largest
    /// volume is 32.
    #[test]
    fn the_side_covers_the_reach() {
        assert_eq!(probes_per_axis(64.0, 16), 8);
        assert_eq!(probes_per_axis(65.0, 16), 16);
        assert_eq!(probes_per_axis(128.0, 16), 16);
        assert_eq!(probes_per_axis(8.0, 16), 4);
        assert_eq!(probes_per_axis(0.0, 16), MAX_PROBES_PER_AXIS);
        assert_eq!(probes_per_axis(10_000.0, 16), MAX_PROBES_PER_AXIS);
    }
}
