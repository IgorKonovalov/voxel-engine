//! The view jitter, the light-change thresholds FSR 2's input pass reads, and the composite's
//! FSR 2 variant; the frame's upscaler setters.

use super::*;

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
/// each component in `(-0.5, 0.5)`. FSR 2 takes its own count (`fsr2::jitter_phases`).
pub fn jitter_at(frame: u32, phases: u32) -> [f32; 2] {
    let i = frame % phases.max(1) + 1;
    [halton(i, 2) - 0.5, halton(i, 3) - 0.5]
}

/// The light-change test FSR 2's input pass applies to a still pixel (`light_changed` in
/// `fsr2/input.wgsl`): the list of lights' summed unshadowed term at the hit moved by more
/// than [`CHANGE_REL`] of the larger of the two and by more than [`CHANGE_ABS`] since the
/// list of [`CHANGE_FRAMES`] frames before (ADR-0044), or the sky's key light or scale moved
/// past the same thresholds since the frame before.
pub const CHANGE_REL: f32 = 0.1;

/// ... and by more than this, in the lamp's irradiance units ([`CHANGE_REL`]).
pub const CHANGE_ABS: f32 = 0.01;

/// The frames between the light lists [`CHANGE_REL`] compares, 1 to [`LAMP_FRAMES`].
pub(crate) const CHANGE_FRAMES: usize = 24;

/// Frames of light lists the frame keeps for [`CHANGE_FRAMES`].
pub(crate) const LAMP_FRAMES: usize = 32;

/// `LightChange` in `fsr2/input.wgsl`, field for field: [`CHANGE_REL`] and [`CHANGE_ABS`].
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct LightChangeUniform {
    pub(crate) change_rel: f32,
    pub(crate) change_abs: f32,
    pub(crate) _pad: [f32; 2],
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

/// [`composite_hdr_source`] composed as the composite is: the prelude, `common.wgsl`,
/// `hitrec.wgsl` and the sky's modules first.
pub(crate) fn composite_hdr_compose() -> String {
    compose(&[
        COMMON_WGSL,
        HITREC_WGSL,
        SKY_ATMOSPHERE_WGSL,
        SKY_WGSL,
        &composite_hdr_source(),
    ])
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
            Io::Float,
            Io::Float,
            Io::Float,
            Io::ReadStorage,
            Io::ReadStorage,
        ],
    );
    let pipeline = compute_pipeline(
        device,
        "composite hdr",
        composite_hdr_compose(),
        &[Some(world_layout), None, Some(&layout)],
        "composite",
    );
    (layout, pipeline)
}

impl Frame {
    /// Whether FSR 2 runs. Without it the frame renders at its output size and the
    /// composite's image is the frame's, byte for byte.
    pub fn upscaling(&self) -> bool {
        self.up.fsr.is_some()
    }

    /// Selects the upscaler: `Off` renders at the output size (the internal size
    /// [`Frame::set_sizes`] asked for is kept for FSR 2), and `Fsr2` runs FSR 2 at that
    /// internal size with its own jitter sequence. A change of upscaler replaces the targets
    /// and drops the history; a change of FSR's settings alone keeps them.
    pub fn set_upscaler_kind(&mut self, device: &wgpu::Device, kind: UpscalerKind) {
        let was_fsr = self.up.fsr.is_some();
        match kind {
            UpscalerKind::Off => self.up.fsr = None,
            UpscalerKind::Fsr2(s) => {
                self.up.fsr2 = s;
                if let Some(f) = self.up.fsr.as_mut() {
                    f.set_settings(s);
                } else {
                    self.up.fsr = Some(Fsr2::new(
                        device,
                        &self.pipes.world_layout,
                        self.up.requested,
                        self.output,
                        s,
                    ));
                }
                if self.up.composite_hdr.is_none() {
                    self.up.composite_hdr =
                        Some(composite_hdr_pass(device, &self.pipes.world_layout));
                }
            }
        }
        let changed = std::mem::discriminant(&kind) != std::mem::discriminant(&self.up.kind)
            || was_fsr != self.up.fsr.is_some();
        self.up.kind = kind;
        if changed {
            self.set_sizes(device, self.up.requested, self.output);
        }
    }

    pub fn upscaler_kind(&self) -> UpscalerKind {
        self.up.kind
    }

    /// FSR 2's context while the frame runs it.
    pub fn fsr2(&self) -> Option<&Fsr2> {
        self.up.fsr.as_ref()
    }

    pub fn fsr2_mut(&mut self) -> Option<&mut Fsr2> {
        self.up.fsr.as_mut()
    }

    /// Drops the upscaler's history alone in the next encoded frame (the lighting's is kept),
    /// as a change of view does.
    pub fn reset_upscaler_history(&mut self) {
        self.restarts += 1;
        self.history().view_changed = true;
    }
}

/// The upscaler's state: the light-change thresholds FSR 2's input pass reads, the internal
/// size asked for, the upscaler the frame runs and, with FSR 2, its context and the
/// composite's variant.
pub(super) struct UpscaleState {
    /// [`LightChangeUniform`], written once.
    pub(super) light_change: wgpu::Buffer,
    /// The internal size [`Frame::set_sizes`] last asked for: FSR 2 renders at it, and
    /// without FSR 2 the frame renders at its output size.
    pub(super) requested: [u32; 2],
    /// The upscaler the frame runs; with FSR 2, its context and the composite's variant
    /// that stores linear radiance (`HDR_TARGET` in composite.wgsl), built with it.
    pub(super) kind: UpscalerKind,
    /// FSR 2's settings as last selected, kept while no upscaler runs.
    pub(super) fsr2: crate::Fsr2Settings,
    pub(super) fsr: Option<Fsr2>,
    pub(super) composite_hdr: Option<(wgpu::BindGroupLayout, wgpu::ComputePipeline)>,
}

impl UpscaleState {
    pub(super) fn new(device: &wgpu::Device, requested: [u32; 2]) -> Self {
        Self {
            light_change: wgpu::util::DeviceExt::create_buffer_init(
                device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("light change"),
                    contents: bytemuck::bytes_of(&LightChangeUniform {
                        change_rel: CHANGE_REL,
                        change_abs: CHANGE_ABS,
                        _pad: [0.0; 2],
                    }),
                    usage: wgpu::BufferUsages::UNIFORM,
                },
            ),
            requested,
            kind: UpscalerKind::Off,
            fsr2: crate::Fsr2Settings::default(),
            fsr: None,
            composite_hdr: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::jitter_at;

    /// The phase count the jitter test cycles through.
    const TEST_PHASES: u32 = 16;

    /// The jitter's first sixteen phases are distinct points of `(-0.5, 0.5)^2` that repeat
    /// with period 16 and average to within 1/16 of zero on each axis (-0.029 and -0.037).
    #[test]
    fn the_jitter_cycles_sixteen_distinct_centred_offsets() {
        let phases: Vec<[f32; 2]> = (0..TEST_PHASES)
            .map(|f| jitter_at(f, TEST_PHASES))
            .collect();
        for (i, p) in phases.iter().enumerate() {
            assert!(p.iter().all(|c| c.abs() < 0.5), "phase {i}: {p:?}");
            assert_eq!(jitter_at(i as u32 + TEST_PHASES, TEST_PHASES), *p);
            assert!(phases[..i].iter().all(|q| q != p), "phase {i} repeats");
        }
        assert_eq!(phases[0], [0.0, 1.0 / 3.0 - 0.5]);
        for k in 0..2 {
            let mean = phases.iter().map(|p| p[k]).sum::<f32>() / TEST_PHASES as f32;
            assert!(mean.abs() <= 1.0 / 16.0, "axis {k}: mean {mean}");
        }
    }
}
