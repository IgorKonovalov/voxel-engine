//! The frame's list of analytic lights (ADR-0044): point and spot lights the consumer hands
//! the frame, shaded exactly at every lit pixel within each light's reach with one shadow ray
//! each. The headlamp ([`LightParams`]) is slot 0 while [`Lighting::lamp`] is on.
//!
//! Not the emissive light-sample mirror (`gpu_world/lights.rs`, `LightSample`, ADR-0025): a
//! list light is never an emitter, never reaches the mirror and is never baked.

use super::*;

/// Lights the list holds, the headlamp's slot included while it is on.
pub const MAX_LIGHTS: usize = 8;

/// One analytic light of the frame's list, field for field with `Light` in `common.wgsl`.
///
/// Its term at a surface point `p` with normal `n` is `colour * PI * intensity * cone *
/// max(n . l, 0) / d^2`, times the window `(1 - (d / r)^4)^2` inside its reach `r =
/// sqrt(intensity / Lighting::light_floor)` and zero beyond it, and zero where its shadow ray
/// finds rock ([`Light::SHADOWS`]). Not the mirror's `LightSample` (`gpu_world`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Light {
    /// Position in voxels.
    pub pos: [f32; 3],
    /// Radiance on the axis at one voxel of distance, as [`LightParams::intensity`].
    pub intensity: f32,
    /// The cone's axis, unit; ignored for a point light.
    pub dir: [f32; 3],
    /// Full intensity inside this cosine of the axis.
    pub cos_inner: f32,
    /// Linear colour; multiplies the term. The direct part is luminance until it is rgb.
    pub colour: [f32; 3],
    /// None outside this cosine of the axis; -1 or less is a point light (cone 1).
    pub cos_outer: f32,
    /// [`Light::ON`], [`Light::SHADOWS`], [`Light::BOUNCE`].
    pub flags: u32,
    pub _pad: [u32; 3],
}

const _: () = assert!(size_of::<Light>() == 64);

impl Light {
    /// The light is shaded; clear, its slot is kept and lights nothing.
    pub const ON: u32 = 1;
    /// Its shadow ray is cast; clear, it lights through rock.
    pub const SHADOWS: u32 = 1 << 1;
    /// It feeds the probe channel (ADR-0043's lamp channel).
    pub const BOUNCE: u32 = 1 << 2;

    /// A white point light at `pos` (voxels) of `intensity`, on and casting shadows.
    pub fn point(pos: [f32; 3], intensity: f32) -> Self {
        Self {
            pos,
            intensity,
            dir: [0.0, 0.0, -1.0],
            cos_inner: -1.0,
            colour: [1.0; 3],
            cos_outer: -1.0,
            flags: Self::ON | Self::SHADOWS,
            _pad: [0; 3],
        }
    }

    /// The headlamp of `light` on `camera`: the slot-0 light while [`Lighting::lamp`] is on.
    pub fn headlamp(camera: &Camera, light: &LightParams) -> Self {
        let (fwd, right, up) = camera.basis();
        Self {
            pos: (camera.position + right * light.offset_right - up * light.offset_down).to_array(),
            intensity: light.intensity,
            dir: (fwd * light.turn.cos() + right * light.turn.sin()).to_array(),
            cos_inner: light.inner.cos(),
            colour: [1.0; 3],
            cos_outer: light.outer.cos(),
            flags: Self::ON | if light.shadows { Self::SHADOWS } else { 0 },
            _pad: [0; 3],
        }
    }

    /// Its reach in voxels at `floor` (`sqrt(intensity / floor)`); infinite at a floor of 0
    /// or less, where the window is 1.
    pub fn reach(&self, floor: f32) -> f32 {
        if floor > 0.0 {
            (self.intensity.max(0.0) / floor).sqrt()
        } else {
            f32::INFINITY
        }
    }

    /// The light at `p`, normal `n`, before the shadow ray, in the light pass's irradiance
    /// units (`PI` times the term), at light floor `floor`: the shader's `light_term`.
    pub fn irradiance(&self, floor: f32, p: [f32; 3], n: [f32; 3]) -> f32 {
        if self.flags & Self::ON == 0 {
            return 0.0;
        }
        let to: [f32; 3] = std::array::from_fn(|k| self.pos[k] - p[k]);
        let d2 = to.iter().map(|v| v * v).sum::<f32>().max(1e-4);
        let w = window(floor, self.intensity, d2);
        if w <= 0.0 {
            return 0.0;
        }
        let d = d2.sqrt();
        let l = to.map(|v| v / d);
        let ndl = (0..3).map(|k| n[k] * l[k]).sum::<f32>().max(0.0);
        let cone = if self.cos_outer <= -1.0 {
            1.0
        } else {
            let along = -(0..3).map(|k| l[k] * self.dir[k]).sum::<f32>();
            let (e0, e1) = (self.cos_outer, self.cos_inner);
            let t = ((along - e0) / (e1 - e0)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        std::f32::consts::PI * self.intensity * cone * ndl / d2 * w
    }
}

/// ADR-0044's window `(1 - (d / r)^4)^2` at squared distance `d2` for a light of `intensity`
/// at light floor `floor` (`r^2 = intensity / floor`): 1 at a floor of 0 or less, 0 at and past
/// the reach.
pub(crate) fn window(floor: f32, intensity: f32, d2: f32) -> f32 {
    if floor <= 0.0 {
        return 1.0;
    }
    let x = d2 * floor / intensity.max(1e-30);
    if x >= 1.0 {
        return 0.0;
    }
    let q = 1.0 - x * x;
    q * q
}

/// A frame's slots: the lights in slots `0..count`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct LightList {
    pub(super) l: [Light; MAX_LIGHTS],
    pub(super) count: u32,
}

impl LightList {
    /// The slot rule: with `lamp`, it takes slot 0 and the first seven of `given` slots 1 to
    /// 7; without, the first eight of `given` slots 0 to 7.
    pub(super) fn slots(lamp: Option<Light>, given: &[Light]) -> Self {
        let mut s = Self::default();
        let mut n = 0;
        if let Some(l) = lamp {
            s.l[0] = l;
            n = 1;
        }
        for l in given.iter().take(MAX_LIGHTS - n) {
            s.l[n] = *l;
            n += 1;
        }
        s.count = n as u32;
        s
    }

    /// The consumer's lights, the headlamp's slot left out.
    pub(super) fn listed(&self, lamp: bool) -> &[Light] {
        &self.l[usize::from(lamp)..self.count as usize]
    }
}

/// `Lights` in `common.wgsl`, field for field: the list now, the list of the older frame the
/// upscalers compare against, both counts, whether the two differ, and 1 when slot 0 is the
/// headlamp.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct LightsUniform {
    pub(super) l: [Light; MAX_LIGHTS],
    pub(super) old: [Light; MAX_LIGHTS],
    pub(super) count: u32,
    pub(super) old_count: u32,
    pub(super) changed: u32,
    pub(super) lamp: u32,
}

const _: () = assert!(size_of::<LightsUniform>() == 1040);

impl LightsUniform {
    pub(super) fn of(now: &LightList, old: &LightList, lamp: bool) -> Self {
        Self {
            l: now.l,
            old: old.l,
            count: now.count,
            old_count: old.count,
            changed: u32::from(now != old),
            lamp: u32::from(lamp),
        }
    }
}

/// The frame's side of the list: what the consumer gave and the uniform the passes read.
pub(super) struct LightsState {
    /// The first [`MAX_LIGHTS`] lights of the last [`Frame::set_lights`], and how many it was
    /// given.
    pub(super) given: Vec<Light>,
    pub(super) given_len: usize,
    /// `lights` (group 0, binding 14).
    pub(super) buffer: wgpu::Buffer,
    /// What the buffer holds, so an unchanged list is not written again.
    pub(super) written: std::sync::Mutex<Option<LightsUniform>>,
    /// Whether the last encode with history set [`LIGHTING_LIGHTS_CHANGED`].
    pub(super) changed: std::sync::atomic::AtomicBool,
}

impl LightsState {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        Self {
            given: Vec::new(),
            given_len: 0,
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("lights"),
                size: size_of::<LightsUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            written: std::sync::Mutex::new(None),
            changed: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// The slots for the frame `p`: the headlamp's (`p.lamp`) first while `p` lights it.
    pub(super) fn slots(&self, p: &Params) -> LightList {
        let lamp = p.lighting & LIGHTING_LAMP != 0;
        LightList::slots(lamp.then_some(p.lamp), &self.given)
    }

    /// Writes `u` unless the buffer already holds it.
    pub(super) fn write(&self, queue: &wgpu::Queue, u: LightsUniform) {
        let mut w = self
            .written
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *w != Some(u) {
            queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&u));
            *w = Some(u);
        }
    }
}

impl Frame {
    /// The consumer's lights for the next encodes, by the slot rule (ADR-0044): while
    /// [`Lighting::lamp`] is on the headlamp takes slot 0 and the first seven of `lights` slots
    /// 1 to 7; while it is off the first eight take slots 0 to 7. The rest are counted
    /// ([`Frame::lights_over_cap`]) and not shaded. The list is uploaded by the encode, when it
    /// differs from the last.
    pub fn set_lights(&mut self, lights: &[Light]) {
        self.state.lights.given.clear();
        self.state
            .lights
            .given
            .extend_from_slice(&lights[..lights.len().min(MAX_LIGHTS)]);
        self.state.lights.given_len = lights.len();
    }

    /// The lights of the last [`Frame::set_lights`] past the cap: past seven while the lamp is
    /// on, past eight while it is off.
    pub fn lights_over_cap(&self) -> u32 {
        let cap = MAX_LIGHTS - usize::from(self.state.lighting.lamp);
        self.state.lights.given_len.saturating_sub(cap) as u32
    }

    /// Whether the last encode with history found the list changed ([`LIGHTING_LIGHTS_CHANGED`]):
    /// a list light added, removed or changed in any field, the lamp switched, its parameters
    /// or its shadow walk changed. The headlamp following the camera is not a change.
    pub fn lights_changed(&self) -> bool {
        self.state
            .lights
            .changed
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(i: u32) -> Light {
        Light::point([i as f32, 0.0, 0.0], 100.0)
    }

    #[test]
    fn the_slot_rule_keeps_seven_beside_the_lamp_and_eight_without() {
        let given: Vec<Light> = (1..=9).map(lit).collect();
        let lamp = Light::point([0.0, 0.0, 9.0], 1200.0);
        let on = LightList::slots(Some(lamp), &given);
        assert_eq!(on.count, 8);
        assert_eq!(on.l[0], lamp);
        assert_eq!(on.listed(true), &given[..7]);
        let off = LightList::slots(None, &given);
        assert_eq!(off.count, 8);
        assert_eq!(off.listed(false), &given[..8]);
        let none = LightList::slots(None, &[]);
        assert_eq!(none.count, 0);
    }

    #[test]
    fn the_analytic_figures_hold() {
        // ADR-0044's point light of intensity 1200 at 20 voxels above a floor, floor 0.1.
        let l = Light::point([0.0, 20.0, 0.0], 1200.0);
        let below = l.irradiance(0.1, [0.0; 3], [0.0, 1.0, 0.0]);
        let side = l.irradiance(0.1, [20.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        assert!((below - 9.40).abs() < 0.01, "{below}");
        assert!((side - 3.30).abs() < 0.01, "{side}");
        assert!((l.reach(0.1) - 109.545).abs() < 0.01);
        assert_eq!(l.irradiance(0.1, [0.0, -100.0, 0.0], [0.0, 1.0, 0.0]), 0.0);
        // At floor 0 the window is 1: the lamp's arithmetic before the list.
        let raw = std::f32::consts::PI * 1200.0 / 400.0;
        assert_eq!(l.irradiance(0.0, [0.0; 3], [0.0, 1.0, 0.0]), raw);
        // 64 voxels to the side: Phase 1 done-when 11's 0.185.
        let far = l.irradiance(0.1, [64.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        assert!((far - 0.185).abs() < 0.002, "{far}");
    }

    #[test]
    fn the_uniform_marks_a_list_that_differs_from_the_older_one() {
        let a = LightList::slots(None, &[lit(1)]);
        let b = LightList::slots(None, &[lit(2)]);
        assert_eq!(LightsUniform::of(&a, &a, false).changed, 0);
        assert_eq!(LightsUniform::of(&b, &a, false).changed, 1);
        assert_eq!(LightsUniform::of(&a, &a, true).lamp, 1);
    }
}
