//! The engine's own upscaler and its sharpen pass, the view jitter, the lamp the upscaler
//! reads back over, and the composite's FSR 2 variant; the frame's upscale setters.

use super::*;

/// The upscale pass, compiled after [`MARCH_WGSL`].
pub const UPSCALE_WGSL: &str = include_str!("../shaders/upscale.wgsl");

/// The sharpening pass (RCAS), compiled after [`MARCH_WGSL`].
pub const SHARPEN_WGSL: &str = include_str!("../shaders/sharpen.wgsl");

/// Phases of the jitter sequence: frame `f` takes phase `f % JITTER_PHASES`.
pub const JITTER_PHASES: u32 = 16;

/// The radical inverse of `i` in base `b`: element `i` of the Halton sequence of that base.
pub(super) fn halton(mut i: u32, b: u32) -> f32 {
    let (mut f, mut r) = (1.0f32, 0.0f32);
    while i > 0 {
        f /= b as f32;
        r += f * (i % b) as f32;
        i /= b;
    }
    r
}

/// The sub-pixel jitter of frame `frame` in a sequence of `phases`, in internal pixels:
/// elements `1 ..= phases` of the Halton (2, 3) sequence by `frame % phases`, centred on zero,
/// each component in `(-0.5, 0.5)`. The upscaler of ADR-0024 takes [`JITTER_PHASES`], FSR 2
/// its own count (`fsr2::jitter_phases`).
pub fn jitter_at(frame: u32, phases: u32) -> [f32; 2] {
    let i = frame % phases.max(1) + 1;
    [halton(i, 2) - 0.5, halton(i, 3) - 0.5]
}

/// The upscale pass's tuning ([`Frame::set_upscaler`]). This frame's colour at an output
/// pixel is reconstructed from the 3 x 3 internal samples around the nearest with the kernel
/// `exp(-2.29 (d / width)^2)`, `d` a sample's distance from the output pixel's centre in
/// output pixels: at width 1, Karis's Gaussian fit of a Blackman-Harris window of width 3.3
/// ("High Quality Temporal Supersampling", 2014). Last frame's history is read at the
/// reprojected position by Catmull-Rom over the 4 x 4 texels around it, held non-negative.
/// The current colour's share of the blend where history is kept is the alpha times the
/// nearest sample's kernel weight over the kernel's peak, divided by the nearest sample's
/// mean kernel weight at the frame's scale (its sample uniform over one internal pixel
/// around the centre), at most 1: a sample far from the output pixel's centre counts for
/// less, and the current colour's mean share stays the alpha. A still pixel (one whose
/// history moved less than 0.01 output pixels) keeps its history unclipped unless the light
/// at its hit changed: the lamp's unshadowed term moved past [`Upscaler::change_rel`] and
/// [`Upscaler::change_abs`] since the lamp of [`Upscaler::change_frames`] frames before, or
/// an emission scale changed within [`EMISSION_CHANGE_FRAMES`]; such a pixel is clipped and
/// blended as a moving one (ADR-0024).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Upscaler {
    /// The reconstruction kernel's width.
    pub width: f32,
    /// The current colour's weight in the blend where the history held still.
    pub alpha: f32,
    /// [`Upscaler::alpha`] where the history moved by 0.01 output pixels or more.
    pub alpha_moving: f32,
    /// The clip box around the 3 x 3 samples' YCoCg mean, in standard deviations (inside
    /// their minimum and maximum).
    pub clip_gamma: f32,
    /// A light change: the lamp's term moved by more than this share of the larger of the
    /// two.
    pub change_rel: f32,
    /// ... and by more than this, in the lamp's irradiance units.
    pub change_abs: f32,
    /// The frames between the lamps compared, 1 to [`LAMP_FRAMES`].
    pub change_frames: u32,
}

/// Frames of lamp parameters the frame keeps for [`Upscaler::change_frames`].
pub(crate) const LAMP_FRAMES: usize = 32;

impl Default for Upscaler {
    /// The Gaussian of width 0.6 over the 3 x 3, the Catmull-Rom fetch and the
    /// nearness-weighted blend, normalised to a mean share of 0.1 still and 0.2 in motion
    /// (ADR-0024).
    fn default() -> Self {
        Self {
            width: 0.6,
            alpha: 0.1,
            alpha_moving: 0.2,
            clip_gamma: 1.0,
            change_rel: 0.1,
            change_abs: 0.01,
            change_frames: 24,
        }
    }
}

/// `UpTuning` in `upscale.wgsl`, field for field.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct UpscaleUniform {
    pub(super) width: f32,
    pub(super) alpha: f32,
    pub(super) alpha_moving: f32,
    pub(super) clip_gamma: f32,
    pub(super) w_mean: f32,
    pub(super) change_rel: f32,
    pub(super) change_abs: f32,
    pub(super) old_on: u32,
    pub(super) old_pos: [f32; 3],
    pub(super) old_intensity: f32,
    pub(super) old_dir: [f32; 3],
    pub(super) old_cos_outer: f32,
    pub(super) old_cos_inner: f32,
    pub(super) _pad: [f32; 3],
}

/// One frame's lamp, as `Params` carries it.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Lamp {
    pub(super) on: bool,
    pub(super) pos: [f32; 3],
    pub(super) intensity: f32,
    pub(super) dir: [f32; 3],
    pub(super) cos_outer: f32,
    pub(super) cos_inner: f32,
}

impl Lamp {
    pub(super) fn of(p: &Params) -> Self {
        Self {
            on: p.lighting & LIGHTING_LAMP != 0,
            pos: p.light_pos,
            intensity: p.light_intensity,
            dir: p.light_dir,
            cos_outer: p.cos_outer,
            cos_inner: p.cos_inner,
        }
    }
}

impl Upscaler {
    /// The reconstruction kernel at offset `(x, y)` output pixels, 1 at the centre
    /// (`up_kernel` in `upscale.wgsl`).
    pub(super) fn kernel(&self, x: f32, y: f32) -> f32 {
        (-2.29 * (x * x + y * y) / (self.width * self.width)).exp()
    }

    /// The nearest sample's mean kernel weight at `scale` output pixels per internal pixel:
    /// its offset uniform over the internal pixel around the output centre, a 32 x 32 grid.
    pub(super) fn mean_weight(&self, scale: f32) -> f32 {
        const N: usize = 32;
        let at = |i: usize| ((i as f32 + 0.5) / N as f32 - 0.5) * scale;
        let mut sum = 0.0;
        for j in 0..N {
            for i in 0..N {
                sum += self.kernel(at(i), at(j));
            }
        }
        sum / (N * N) as f32
    }

    pub(super) fn uniform(&self, scale: f32, old: Lamp) -> UpscaleUniform {
        UpscaleUniform {
            width: self.width,
            alpha: self.alpha,
            alpha_moving: self.alpha_moving,
            clip_gamma: self.clip_gamma,
            w_mean: self.mean_weight(scale),
            change_rel: self.change_rel,
            change_abs: self.change_abs,
            old_on: u32::from(old.on),
            old_pos: old.pos,
            old_intensity: old.intensity,
            old_dir: old.dir,
            old_cos_outer: old.cos_outer,
            old_cos_inner: old.cos_inner,
            _pad: [0.0; 3],
        }
    }
}

/// The source of the composite's variant for FSR 2: `HDR_TARGET` set and the target
/// rgba32float, linear radiance before exposure in the shaded view.
pub fn composite_hdr_source() -> String {
    let flag = "const HDR_TARGET: bool = false;";
    let target = "var target_tex: texture_storage_2d<rgba8unorm, write>;";
    assert!(
        COMPOSITE_WGSL.matches(flag).count() == 1 && COMPOSITE_WGSL.matches(target).count() == 1,
        "composite.wgsl declares its HDR flag and its target once"
    );
    COMPOSITE_WGSL
        .replacen(flag, "const HDR_TARGET: bool = true;", 1)
        .replacen(
            target,
            "var target_tex: texture_storage_2d<rgba32float, write>;",
            1,
        )
}

/// The composite's FSR 2 variant: its layout (the composite's, the target rgba32float) and
/// pipeline.
pub(super) fn composite_hdr_pass(
    device: &wgpu::Device,
    world_layout: &wgpu::BindGroupLayout,
) -> (wgpu::BindGroupLayout, wgpu::ComputePipeline) {
    let layout = io_layout(
        device,
        "composite hdr",
        &[
            Io::Float,
            Io::Uint,
            Io::Float,
            Io::Float,
            Io::Store(wgpu::TextureFormat::Rgba32Float),
            Io::Float,
            Io::Uint,
            Io::Float,
            Io::Storage,
            Io::Uint,
        ],
    );
    let pipeline = compute_pipeline(
        device,
        "composite hdr",
        format!("{MARCH_WGSL}\n{}", composite_hdr_source()),
        &[Some(world_layout), None, Some(&layout)],
        "composite",
    );
    (layout, pipeline)
}

impl Frame {
    /// Jitters every view's rays by [`jitter_at`] of the frame index (off by default: each
    /// ray through its pixel's centre). The upscaler runs when the frame jitters or the
    /// internal size differs from the output size, so turning the jitter on or off at equal
    /// sizes replaces the targets and drops the history.
    pub fn set_jitter(&mut self, device: &wgpu::Device, on: bool) {
        if on != self.jitter {
            self.jitter = on;
            self.set_sizes(device, self.internal_size(), self.output);
        }
    }

    /// Sets the sharpening pass's strength in `0..=1` (clamped): 0 (the default) writes the
    /// upscale pass's output unchanged, 1 is RCAS at full strength (0 stops), and between them
    /// `2 (1 - s)` stops (`con = 2^-stops`). It runs with the upscaler.
    pub fn set_sharpness(&mut self, queue: &wgpu::Queue, sharpness: f32) {
        self.sharpness = sharpness.clamp(0.0, 1.0);
        let con = if self.sharpness > 0.0 {
            (-2.0 * (1.0 - self.sharpness)).exp2()
        } else {
            0.0
        };
        queue.write_buffer(
            &self.sharpen_params,
            0,
            bytemuck::cast_slice(&[con, 0.0, 0.0, 0.0]),
        );
    }

    pub fn sharpness(&self) -> f32 {
        self.sharpness
    }

    /// Sets the upscale pass's reconstruction, history fetch and blend from the next frame on.
    pub fn set_upscaler(&mut self, upscaler: Upscaler) {
        self.upscaler = upscaler;
    }

    pub fn upscaler(&self) -> Upscaler {
        self.upscaler
    }

    /// Whether an upscaler runs: FSR 2, or the frame jitters or renders below its output
    /// size. Without one the composite's image is the frame's, byte for byte.
    pub fn upscaling(&self) -> bool {
        self.fsr.is_some() || self.jitter || self.internal_size() != self.output
    }

    /// Selects the upscaler: `Off` turns the jitter off (at equal sizes no upscaler runs),
    /// `Own` sets the tuning of ADR-0024's upscaler (the jitter as [`Frame::set_jitter`] left
    /// it), and `Fsr2` runs FSR 2 with its own jitter sequence. A change of upscaler replaces
    /// the targets and drops the history; a change of FSR's settings alone keeps them.
    pub fn set_upscaler_kind(&mut self, device: &wgpu::Device, kind: UpscalerKind) {
        let was_fsr = self.fsr.is_some();
        match kind {
            UpscalerKind::Off => self.jitter = false,
            UpscalerKind::Own(u) => self.upscaler = u,
            UpscalerKind::Fsr2(s) => {
                if let Some(f) = self.fsr.as_mut() {
                    f.set_settings(s);
                } else {
                    self.fsr = Some(Fsr2::new(
                        device,
                        &self.world_layout,
                        self.internal_size(),
                        self.output,
                        s,
                    ));
                }
                if self.composite_hdr.is_none() {
                    self.composite_hdr = Some(composite_hdr_pass(device, &self.world_layout));
                }
            }
        }
        if !kind.is_fsr2() {
            self.fsr = None;
        }
        let changed = std::mem::discriminant(&kind) != std::mem::discriminant(&self.kind)
            || was_fsr != self.fsr.is_some();
        self.kind = kind;
        if changed {
            self.set_sizes(device, self.internal_size(), self.output);
        }
    }

    pub fn upscaler_kind(&self) -> UpscalerKind {
        self.kind
    }

    /// FSR 2's context while the frame runs it.
    pub fn fsr2(&self) -> Option<&Fsr2> {
        self.fsr.as_ref()
    }

    pub fn fsr2_mut(&mut self) -> Option<&mut Fsr2> {
        self.fsr.as_mut()
    }

    pub fn jitter(&self) -> bool {
        self.jitter
    }

    /// Drops the upscaler's history alone in the next encoded frame (the lighting's is kept),
    /// as a change of view does.
    pub fn reset_upscaler_history(&mut self) {
        self.history().view_changed = true;
    }

    /// The upscaler's history the next or last encode writes (rgba16float at the output size:
    /// the upscaled image, display-encoded rgb), or `None` without the upscaler.
    pub fn upscale_history(&self) -> Option<&wgpu::Texture> {
        self.targets.history.as_ref().map(|h| &h[self.cur])
    }
}

#[cfg(test)]
mod tests {
    use super::{JITTER_PHASES, jitter_at};

    /// The jitter's sixteen phases are distinct points of `(-0.5, 0.5)^2` that repeat with
    /// period 16 and average to within 1/16 of zero on each axis (-0.029 and -0.037).
    #[test]
    fn the_jitter_cycles_sixteen_distinct_centred_offsets() {
        let phases: Vec<[f32; 2]> = (0..JITTER_PHASES)
            .map(|f| jitter_at(f, JITTER_PHASES))
            .collect();
        for (i, p) in phases.iter().enumerate() {
            assert!(p.iter().all(|c| c.abs() < 0.5), "phase {i}: {p:?}");
            assert_eq!(jitter_at(i as u32 + JITTER_PHASES, JITTER_PHASES), *p);
            assert!(phases[..i].iter().all(|q| q != p), "phase {i} repeats");
        }
        assert_eq!(phases[0], [0.0, 1.0 / 3.0 - 0.5]);
        for k in 0..2 {
            let mean = phases.iter().map(|p| p[k]).sum::<f32>() / JITTER_PHASES as f32;
            assert!(mean.abs() <= 1.0 / 16.0, "axis {k}: mean {mean}");
        }
    }

    /// The upscale pass parses and validates headless, compiled after the hit
    /// pass.
    #[test]
    fn upscale_wgsl_validates_through_naga() {
        let source = format!("{}\n{}", crate::MARCH_WGSL, super::UPSCALE_WGSL);
        let module = wgpu::naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("upscale.wgsl parses: {}", e.emit_to_string(&source)));
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::default(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("upscale.wgsl validates: {e:?}"));
    }

    /// The sharpening pass parses and validates headless, compiled after the hit pass.
    #[test]
    fn sharpen_wgsl_validates_through_naga() {
        let source = format!("{}\n{}", crate::MARCH_WGSL, super::SHARPEN_WGSL);
        let module = wgpu::naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("sharpen.wgsl parses: {}", e.emit_to_string(&source)));
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::default(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("sharpen.wgsl validates: {e:?}"));
    }
}
