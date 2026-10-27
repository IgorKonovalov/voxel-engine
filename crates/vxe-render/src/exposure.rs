//! The frame's exposure (ADR-0033): the composite's meter, the exposure pass that turns it into
//! the state the next frame reads, and a ring of read-back buffers that hands that state to the
//! CPU a few frames late without waiting.
//!
//! The meter: in the shaded view the composite adds, per 8 x 8 workgroup of the internal
//! target, `clamp(log2(max(Y, 2^-16)), -16, 16)` of each pixel's radiance before exposure and
//! before the tone map (Rec. 709 luminance `Y`) and a count of 1, into one tile each. The
//! exposure pass (`exposure.wgsl`, one workgroup of 256, run last in the frame) weights tile
//! `i` by [`Metering::weight`] and writes `sum(w S) / sum(w n)` into the state's
//! `metered_log2`.
//!
//! The exposure is [`Exposure::Manual`] or [`Exposure::Auto`]. In auto the pass steps an adapted
//! level `a` (log2 luminance) toward the meter `t` by [`AutoExposure::step`] over the caller's
//! `dt` (`Frame::set_dt`), and the next frame's exposure is [`AutoExposure::exposure`] of it.
//!
//! **Pre-exposure.** Lighting math is in absolute units. Every value a pass stores into an
//! `rgba16float` HDR target is multiplied by the frame's exposure (`exposure_now` in
//! `march.wgsl`) and clamped at 60 000; a pass that reads a value an earlier frame stored
//! multiplies it by `current / previous` (`exposure_ratio`) first, a second moment by its
//! square, and a second moment is of the luminance clamped at 255. Terms that stay absolute
//! (emission, translucent layers, the probe atlases and their gathered irradiance, reservoirs)
//! are multiplied by the exposure where they join a pre-exposed value. A new pass that keeps HDR
//! history follows the same rule, or its history pulses on every exposure change.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};

/// The exposure pass's source.
pub const EXPOSURE_WGSL: &str = include_str!("shaders/exposure.wgsl");

/// Pixels on a meter tile's edge: the composite's workgroup.
pub const METER_TILE: u32 = 8;
/// The log2 luminance a pixel adds to its tile is clamped to `[-METER_LOG2_LIMIT,
/// METER_LOG2_LIMIT]`.
pub const METER_LOG2_LIMIT: f32 = 16.0;

/// How the exposure pass weights the meter's tiles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Metering {
    /// Tile `i` weighs `1 + strength exp(-r_i^2 / (2 sigma^2))`, `r_i` its centre's distance
    /// from its view's centre in units of half the view's height.
    CentreWeighted { strength: f32, sigma: f32 },
    /// Every tile weighs 1 (`strength` 0).
    Average,
}

impl Default for Metering {
    /// Centre-weighted, `strength` 3 and `sigma` 0.5.
    fn default() -> Self {
        Self::CentreWeighted {
            strength: 3.0,
            sigma: 0.5,
        }
    }
}

impl Metering {
    /// `strength` and `sigma`; `Average` is strength 0.
    pub fn params(self) -> (f32, f32) {
        match self {
            Self::CentreWeighted { strength, sigma } => (strength, sigma),
            Self::Average => (0.0, 1.0),
        }
    }

    /// The weight of a tile whose centre lies `r` half-heights from its view's centre.
    pub fn weight(self, r: f32) -> f32 {
        let (strength, sigma) = self.params();
        let s = sigma.max(1e-6);
        1.0 + strength * (-r * r / (2.0 * s * s)).exp()
    }

    /// The lower-case name the fly-through prints and parses.
    pub fn name(self) -> &'static str {
        match self {
            Self::CentreWeighted { .. } => "centre",
            Self::Average => "average",
        }
    }
}

/// The frame's exposure: the radiance the tone map sees is the scene's times it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Exposure {
    /// A fixed multiplier (the default, 1.0). It applies from the frame it is set on.
    Manual(f32),
    /// Adapted on the GPU from the frame's meter, one frame late (ADR-0033).
    Auto(AutoExposure),
}

impl Default for Exposure {
    fn default() -> Self {
        Self::Manual(1.0)
    }
}

/// The automatic exposure's model (ADR-0033). In log2 luminance, with `k(tau) = 1 - exp(-dt /
/// tau)` (1 for `tau` 0): toward bright `a += (t - a) k(tau_light)`; toward dark `a` decays
/// toward `t` at `tau_cone` while `a > log2 cone_floor` and at `tau_rod` once below, a step that
/// crosses the floor spending `tau_cone ln((a - t) / (f - t))` of its `dt` reaching it. The
/// exposure is `key 2^compensation_ev / 2^a` clamped to `[min, max]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoExposure {
    /// The frame luminance a settled scene is exposed to (0.18).
    pub key: f32,
    /// The game's bias in EV (0).
    pub compensation_ev: f32,
    pub min: f32,
    pub max: f32,
    /// Time constants in seconds: toward bright, toward dark above the floor, below it.
    pub tau_light: f32,
    pub tau_cone: f32,
    pub tau_rod: f32,
    /// The frame luminance where dark adaptation turns from `tau_cone` to `tau_rod`; 0 is one
    /// dark stage at `tau_cone`.
    pub cone_floor: f32,
    pub metering: Metering,
}

impl Default for AutoExposure {
    fn default() -> Self {
        Self::eye()
    }
}

impl AutoExposure {
    /// The eye: key 0.18, limits 2^-6 and 2^6, constants 0.15, 1 and 4 s, floor 2^-6,
    /// centre-weighted.
    pub fn eye() -> Self {
        Self {
            key: 0.18,
            compensation_ev: 0.0,
            min: 2f32.powi(-6),
            max: 2f32.powi(6),
            tau_light: 0.15,
            tau_cone: 1.0,
            tau_rod: 4.0,
            cone_floor: 2f32.powi(-6),
            metering: Metering::default(),
        }
    }

    /// One rate both ways: every constant `1 / speed` seconds, no floor.
    pub fn symmetric(speed: f32) -> Self {
        let tau = 1.0 / speed;
        Self {
            tau_light: tau,
            tau_cone: tau,
            tau_rod: tau,
            cone_floor: 0.0,
            ..Self::eye()
        }
    }

    /// Every constant 0: the exposure follows the meter at once.
    pub fn instant() -> Self {
        Self::symmetric(f32::INFINITY)
    }

    /// `log2 cone_floor`, or minus infinity for a floor of 0.
    pub fn floor_log2(&self) -> f64 {
        if self.cone_floor > 0.0 {
            f64::from(self.cone_floor).log2()
        } else {
            f64::NEG_INFINITY
        }
    }

    /// The adapted level after `dt` seconds toward the meter `metered_log2` from
    /// `adapted_log2`; `dt` 0 returns it unchanged.
    pub fn step(&self, adapted_log2: f64, metered_log2: f64, dt: f64) -> f64 {
        let (a, t) = (adapted_log2, metered_log2);
        if dt <= 0.0 {
            return a;
        }
        let decay = |dt: f64, tau: f32| {
            if tau > 0.0 {
                (-dt / f64::from(tau)).exp()
            } else {
                0.0
            }
        };
        if t >= a {
            return t + (a - t) * decay(dt, self.tau_light);
        }
        let f = self.floor_log2();
        if a > f {
            if t >= f {
                return t + (a - t) * decay(dt, self.tau_cone);
            }
            let to_floor = if self.tau_cone > 0.0 {
                f64::from(self.tau_cone) * ((a - t) / (f - t)).ln()
            } else {
                0.0
            };
            if dt <= to_floor {
                return t + (a - t) * decay(dt, self.tau_cone);
            }
            return t + (f - t) * decay(dt - to_floor, self.tau_rod);
        }
        t + (a - t) * decay(dt, self.tau_rod)
    }

    /// The exposure at the adapted level `adapted_log2`.
    pub fn exposure(&self, adapted_log2: f64) -> f64 {
        let e = f64::from(self.key) * f64::from(self.compensation_ev).exp2() / adapted_log2.exp2();
        e.clamp(f64::from(self.min), f64::from(self.max))
    }
}

/// `ExState` in `exposure.wgsl`, field for field: 32 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ExposureState {
    /// The exposure the next frame's passes use.
    pub current: f32,
    /// The exposure the last frame's passes used.
    pub previous: f32,
    /// The adapted level, log2 of frame luminance.
    pub adapted_log2: f32,
    /// The last metered level, log2 of frame luminance.
    pub metered_log2: f32,
    /// Exposure passes run since the frame was made.
    pub frame: u32,
    /// [`STATE_HELD`].
    pub flags: u32,
    pub _pad: [u32; 2],
}

/// `ExposureState::flags` bit: the last pass metered nothing (another view than the shaded
/// one) and held the adapted and metered levels.
pub const STATE_HELD: u32 = 1;

impl ExposureState {
    /// The state as the frame starts: exposure 1.0, nothing metered.
    pub fn initial() -> Self {
        Self {
            current: 1.0,
            previous: 1.0,
            flags: STATE_HELD,
            ..Self::default()
        }
    }

    /// The state from its 32 bytes, as a blocking read of [`crate::Frame::exposure_state`]
    /// returns them.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        bytemuck::pod_read_unaligned(&bytes[..size_of::<Self>()])
    }

    pub fn readout(&self) -> ExposureReadout {
        ExposureReadout {
            exposure: self.current,
            adapted_log2: self.adapted_log2,
            metered_log2: self.metered_log2,
            frame: self.frame,
            held: self.flags & STATE_HELD != 0,
        }
    }
}

/// The exposure state as the CPU last read it back ([`crate::Frame::exposure_readout`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExposureReadout {
    /// The exposure the frame after the read one uses.
    pub exposure: f32,
    pub adapted_log2: f32,
    pub metered_log2: f32,
    /// Exposure passes run up to the read one.
    pub frame: u32,
    /// The read pass metered nothing and held the levels.
    pub held: bool,
}

/// `ExUniform` in `exposure.wgsl`, field for field.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ExposureUniform {
    pub tiles: [u32; 2],
    pub view_count: u32,
    pub mode: u32,
    pub rects: [[f32; 4]; 2],
    pub manual: f32,
    pub strength: f32,
    pub sigma: f32,
    pub dt: f32,
    /// `key 2^compensation_ev`, the limits, the time constants and `log2 cone_floor` (a large
    /// negative number for no floor).
    pub key: f32,
    pub min: f32,
    pub max: f32,
    pub tau_light: f32,
    pub tau_cone: f32,
    pub tau_rod: f32,
    pub floor_log2: f32,
    pub _pad: f32,
}

/// `ExUniform::mode` bit: the exposure is automatic.
pub(crate) const EX_AUTO: u32 = 1;
/// `ExUniform::mode` bit: the shaded view ran and its tiles hold the meter.
pub(crate) const EX_METER: u32 = 2;
/// `ExUniform::mode` bit: the adapted level jumps to the meter.
pub(crate) const EX_RESET: u32 = 4;

impl ExposureUniform {
    /// The auto fields of `auto`.
    pub fn set_auto(&mut self, auto: &AutoExposure) {
        self.key = auto.key * auto.compensation_ev.exp2();
        self.min = auto.min;
        self.max = auto.max;
        self.tau_light = auto.tau_light;
        self.tau_cone = auto.tau_cone;
        self.tau_rod = auto.tau_rod;
        self.floor_log2 = auto.floor_log2().max(-1e30) as f32;
    }
}

/// Read-back buffers in the ring.
const RING: usize = 3;
/// Bytes of one read-back buffer.
const RING_BYTES: u64 = 256;

/// A ring slot's progress.
enum Phase {
    Idle,
    /// The state was copied in by the encode of this index; not yet asked to map.
    Copied(u64),
    /// Asked to map: 0 pending, 1 mapped, 2 failed.
    Mapping(Arc<AtomicU8>),
}

struct Slot {
    buffer: wgpu::Buffer,
    phase: Phase,
}

struct Ring {
    slots: Vec<Slot>,
    /// Encodes so far.
    encodes: u64,
    latest: Option<ExposureReadout>,
}

/// The exposure pass, its state and its read-back ring.
pub(crate) struct ExposurePass {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    /// The state the pass writes (`STORAGE | COPY_SRC`).
    state: wgpu::Buffer,
    /// The copy of the state every frame pass reads through group 0 (`UNIFORM | COPY_DST`).
    state_uniform: wgpu::Buffer,
    ring: Mutex<Ring>,
}

impl ExposurePass {
    pub fn new(device: &wgpu::Device) -> Self {
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("exposure"),
            entries: &[
                entry(0, wgpu::BufferBindingType::Uniform),
                entry(1, wgpu::BufferBindingType::Storage { read_only: false }),
                entry(2, wgpu::BufferBindingType::Storage { read_only: true }),
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("exposure"),
            source: wgpu::ShaderSource::Wgsl(EXPOSURE_WGSL.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("exposure"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("exposure"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("exposure"),
            compilation_options: Default::default(),
            cache: None,
        });
        let init = |label, usage| {
            wgpu::util::DeviceExt::create_buffer_init(
                device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: bytemuck::bytes_of(&ExposureState::initial()),
                    usage,
                },
            )
        };
        let state = init(
            "exposure state",
            wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
        );
        let state_uniform = init(
            "exposure state uniform",
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("exposure params"),
            size: size_of::<ExposureUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let slots = (0..RING)
            .map(|_| Slot {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("exposure readback"),
                    size: RING_BYTES,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                phase: Phase::Idle,
            })
            .collect();
        Self {
            pipeline,
            layout,
            uniform,
            state,
            state_uniform,
            ring: Mutex::new(Ring {
                slots,
                encodes: 0,
                latest: None,
            }),
        }
    }

    pub fn state(&self) -> &wgpu::Buffer {
        &self.state
    }

    pub fn state_uniform(&self) -> &wgpu::Buffer {
        &self.state_uniform
    }

    /// Bytes the state, its uniform copy, the pass's parameters and the ring occupy.
    pub fn bytes(&self) -> u64 {
        2 * size_of::<ExposureState>() as u64
            + size_of::<ExposureUniform>() as u64
            + RING as u64 * RING_BYTES
    }

    /// Writes the pass's parameters for this frame and records it over `tiles`, then the
    /// state's copies: into the uniform the next frame reads, and into a free ring slot.
    /// Slots copied by an earlier encode are asked to map first: every encode's commands are
    /// submitted before the next encode.
    pub fn encode(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        tiles: &wgpu::Buffer,
        uniform: &ExposureUniform,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(uniform));
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("exposure"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.state.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: tiles.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("exposure"),
                timestamp_writes,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        let bytes = size_of::<ExposureState>() as u64;
        encoder.copy_buffer_to_buffer(&self.state, 0, &self.state_uniform, 0, bytes);
        let mut ring = self
            .ring
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = ring.encodes;
        ring.request_maps(now);
        if let Some(slot) = ring
            .slots
            .iter_mut()
            .find(|s| matches!(s.phase, Phase::Idle))
        {
            encoder.copy_buffer_to_buffer(&self.state, 0, &slot.buffer, 0, bytes);
            slot.phase = Phase::Copied(now);
        }
        ring.encodes += 1;
    }

    /// The newest state a ring slot has finished mapping, after a non-blocking poll; slots
    /// copied before the last encode are asked to map first.
    pub fn readout(&self, device: &wgpu::Device) -> Option<ExposureReadout> {
        let mut ring = self
            .ring
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let last = ring.encodes.saturating_sub(1);
        ring.request_maps(last);
        let _ = device.poll(wgpu::PollType::Poll);
        let mut newest: Option<ExposureReadout> = ring.latest;
        for slot in &mut ring.slots {
            let Phase::Mapping(done) = &slot.phase else {
                continue;
            };
            match done.load(Ordering::Acquire) {
                0 => continue,
                1 => {
                    let r = {
                        let bytes = slot
                            .buffer
                            .get_mapped_range(..size_of::<ExposureState>() as u64)
                            .expect("exposure readback mapped");
                        ExposureState::from_bytes(&bytes).readout()
                    };
                    slot.buffer.unmap();
                    if newest.is_none_or(|n| r.frame.wrapping_sub(n.frame) as i32 > 0) {
                        newest = Some(r);
                    }
                }
                _ => {}
            }
            slot.phase = Phase::Idle;
        }
        ring.latest = newest;
        newest
    }
}

impl Ring {
    /// Asks every slot copied by an encode before index `before` to map.
    fn request_maps(&mut self, before: u64) {
        for slot in &mut self.slots {
            if let Phase::Copied(at) = slot.phase
                && at < before
            {
                let done = Arc::new(AtomicU8::new(0));
                let flag = done.clone();
                slot.buffer.map_async(wgpu::MapMode::Read, .., move |r| {
                    flag.store(if r.is_ok() { 1 } else { 2 }, Ordering::Release);
                });
                slot.phase = Phase::Mapping(done);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exposure pass parses and validates headless, standalone.
    #[test]
    fn exposure_wgsl_validates_through_naga() {
        let module = wgpu::naga::front::wgsl::parse_str(EXPOSURE_WGSL).unwrap_or_else(|e| {
            panic!("exposure.wgsl parses: {}", e.emit_to_string(EXPOSURE_WGSL))
        });
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::default(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("exposure.wgsl validates: {e:?}"));
    }

    /// The state and the uniform are the shader's sizes: 32 bytes and 96.
    #[test]
    fn the_state_and_the_uniform_have_the_shaders_layout() {
        assert_eq!(size_of::<ExposureState>(), 32);
        assert_eq!(size_of::<ExposureUniform>(), 96);
    }

    /// The centre weight is 1 + strength at the centre and falls to 1 far out; the average
    /// weighs 1 everywhere.
    #[test]
    fn the_centre_weight_peaks_at_one_plus_strength() {
        let c = Metering::default();
        assert_eq!(c.weight(0.0), 4.0);
        assert!((c.weight(0.5) - (1.0 + 3.0 * (-0.5f32).exp())).abs() < 1e-6);
        assert!((c.weight(5.0) - 1.0).abs() < 1e-6);
        assert_eq!(Metering::Average.weight(0.0), 1.0);
    }

    /// The reference model's worked numbers (Plan 0040): from `a = -4` toward `t = -8` the
    /// floor `-6` is reached at `ln 2` s, and the rod stage leaves `2 exp(-(t - ln 2) / 4)`;
    /// toward bright the step is `4 exp(-t / 0.15)`; a step of 0 holds; `instant` jumps.
    #[test]
    fn the_reference_step_follows_the_two_stage_curve() {
        let e = AutoExposure::eye();
        let a = |t: f64| e.step(-4.0, -8.0, t);
        assert!((a(std::f64::consts::LN_2) + 6.0).abs() < 1e-12);
        assert!((a(0.5) - (-8.0 + 4.0 * (-0.5f64).exp())).abs() < 1e-12);
        for t in [1.0, 2.0, 8.0] {
            let want = -8.0 + 2.0 * (-(t - std::f64::consts::LN_2) / 4.0).exp();
            assert!((a(t) - want).abs() < 1e-12, "{t}: {} {want}", a(t));
        }
        // Stepping in frames lands on the closed form.
        let mut x = -4.0;
        for _ in 0..600 {
            x = e.step(x, -8.0, 1.0 / 60.0);
        }
        assert!((x - a(10.0)).abs() < 1e-9, "{x} {}", a(10.0));
        assert!(
            (e.step(-8.0, -4.0, 0.15) - (-4.0 - 4.0 * (-1f64).exp())).abs() < 1e-6,
            "tau_light is 0.15 in f32"
        );
        assert_eq!(e.step(-5.3, -8.0, 0.0), -5.3);
        assert_eq!(AutoExposure::instant().step(-5.3, -8.0, 1.0 / 60.0), -8.0);
        assert!((e.exposure(-8.0) - f64::from(0.18f32) * 256.0).abs() < 1e-9);
        assert_eq!(e.exposure(-12.0), 64.0);
        let s = AutoExposure::symmetric(2.0);
        assert!((s.step(-1.0, -3.0, 0.5) - (-3.0 + 2.0 * (-1f64).exp())).abs() < 1e-12);
    }
}
