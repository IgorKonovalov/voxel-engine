//! AMD FidelityFX Super Resolution 2.2.1 ported to WGSL (ADR-0034): FSR 2's upscaling passes
//! under `shaders/fsr2/`, fed from the hit buffer by an input pass of ours. [`Fsr2`] holds one
//! frame's FSR resources and pipelines; [`crate::Frame`] runs it when its upscaler is
//! [`UpscalerKind::Fsr2`].
//!
//! The port takes the portable WGSL forms throughout: the FP32 path, no wave operations, no
//! image atomics, core storage formats (each of AMD's 16- and 8-bit formats widened to one at
//! least as precise), and four-tap loads for AMD's bilinear samples.

use crate::frame::{Dispatch, Dispatches, Params, Upscaler, WORKGROUP};
use crate::gpu_util::*;

mod pipelines;
mod prepare;
mod resources;
mod settings;

pub(crate) use self::pipelines::*;
pub(crate) use self::prepare::*;
use self::resources::*;
pub use self::settings::*;

/// The parts of FSR 2 every FSR pass is compiled after; carries AMD's notice.
pub(crate) const FSR2_COMMON_WGSL: &str = include_str!("../shaders/fsr2/common.wgsl");

/// FSR 2's accumulate pass, compiled after [`FSR2_COMMON_WGSL`].
pub const FSR2_ACCUMULATE_WGSL: &str = include_str!("../shaders/fsr2/accumulate.wgsl");

/// Our input pass (depth, motion vectors, reactive mask), compiled after ``common.wgsl`, `hitrec.wgsl` and `edits.wgsl``.
pub const FSR2_INPUT_WGSL: &str = include_str!("../shaders/fsr2/input.wgsl");

/// Our tone pass (exposure, tone map, display encoding), compiled after ``common.wgsl``.
pub const FSR2_TONE_WGSL: &str = include_str!("../shaders/fsr2/tone.wgsl");

/// FSR 2's luminance pyramid (two entry points), compiled after [`FSR2_COMMON_WGSL`].
pub const FSR2_LUMINANCE_PYRAMID_WGSL: &str =
    include_str!("../shaders/fsr2/luminance_pyramid.wgsl");

/// FSR 2's reconstruct-previous-depth pass, compiled after [`FSR2_COMMON_WGSL`].
pub const FSR2_RECONSTRUCT_WGSL: &str = concat!(
    include_str!("../shaders/fsr2/motion.wgsl"),
    include_str!("../shaders/fsr2/reconstruct_previous_depth.wgsl"),
);
/// FSR 2's depth-clip pass with the motion read it shares with the reconstruct pass, compiled
/// after [`FSR2_COMMON_WGSL`].
pub const FSR2_DEPTH_CLIP_WGSL: &str = concat!(
    include_str!("../shaders/fsr2/motion.wgsl"),
    include_str!("../shaders/fsr2/depth_clip.wgsl"),
);

/// FSR 2's lock pass, compiled after [`FSR2_COMMON_WGSL`].
pub const FSR2_LOCK_WGSL: &str = include_str!("../shaders/fsr2/lock.wgsl");

/// FSR 2's RCAS pass ending in our tone pass's arithmetic, compiled after
/// [`FSR2_COMMON_WGSL`].
pub const FSR2_RCAS_WGSL: &str = include_str!("../shaders/fsr2/rcas.wgsl");

/// The source of an FSR pass: `HDR` (FFX_FSR2_OPTION_HDR_COLOR_INPUT), the common part, then
/// the pass.
pub fn fsr2_source(pass: &str, hdr: bool) -> String {
    format!("const HDR: bool = {hdr};\n{FSR2_COMMON_WGSL}\n{pass}")
}

/// FSR 2's frame passes in encode order, each its own slot of [`crate::FRAME_PASSES`] after
/// the composite, upscale and sharpen slots and before the exposure slot.
pub const FSR2_PASSES: [&str; 9] = [
    "fsr2 input",
    "fsr2 spd",
    "fsr2 spd tail",
    "fsr2 reconstruct",
    "fsr2 depth clip",
    "fsr2 lock",
    "fsr2 accumulate",
    "fsr2 rcas",
    "fsr2 tone",
];

/// The near plane FSR's depth is taken against, in voxels: below any hit's view depth that
/// matters (a hit nearer than it still gets a finite depth above 1).
pub(crate) const NEAR: f32 = 0.01;

/// One frame's FSR 2: its pipelines and resources ([`crate::Frame`] holds it in FSR mode).
pub struct Fsr2 {
    cb_layout: wgpu::BindGroupLayout,
    input: Pass,
    spd_mips: Pass,
    spd_tail: Pass,
    reconstruct: Pass,
    depth_clip: Pass,
    lock: Pass,
    accumulate: Pass,
    rcas: Pass,
    tone: Pass,
    /// The FSR passes the last [`Fsr2Binds::passes`] dispatched, by name ([`Fsr2::dispatched`]).
    dispatched: std::sync::Mutex<Vec<&'static str>>,
    cb: wgpu::Buffer,
    input_tuning: wgpu::Buffer,
    res: Resources,
    settings: Fsr2Settings,
    /// The tone pass reads the composite's colour and FSR's passes are skipped
    /// ([`Fsr2::set_bypass`]).
    bypass: bool,
    /// The accumulate pass writes its history weight ([`Fsr2::set_debug`]).
    debug: bool,
    /// With `debug`, the accumulate pass writes its stage code instead
    /// ([`Fsr2::set_debug_stages`]).
    stages: bool,
    /// The pre-exposure of the last encoded frame (`previousFramePreExposure`).
    pre_exposure: std::sync::Mutex<f32>,
}

impl Fsr2 {
    /// Replaces the resources with ones for `render`-sized frames shown at `display`.
    pub(crate) fn resize(&mut self, device: &wgpu::Device, render: [u32; 2], display: [u32; 2]) {
        if self.res.render != render || self.res.display != display {
            self.res = Resources::new(device, render, display, self.debug);
        }
    }

    pub(crate) fn set_settings(&mut self, settings: Fsr2Settings) {
        self.settings = settings;
    }

    pub fn settings(&self) -> Fsr2Settings {
        self.settings
    }

    /// For tests at equal sizes: the tone pass reads the composite's linear colour directly,
    /// FSR's passes are not dispatched and the rays are not jittered.
    pub fn set_bypass(&mut self, bypass: bool) {
        self.bypass = bypass;
    }

    pub fn bypassed(&self) -> bool {
        self.bypass
    }

    /// The FSR 2 passes ([`FSR2_PASSES`] names) the last encoded frame dispatched.
    pub fn dispatched(&self) -> Vec<&'static str> {
        self.dispatched
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Whether the next frame dispatches RCAS: sharpening above 0 and the upscaler not
    /// bypassed; RCAS then writes the frame's image and the tone pass is not dispatched.
    /// Otherwise the tone pass reads the accumulate pass's output (or, bypassed, the
    /// composite's colour) directly.
    pub fn sharpens(&self) -> bool {
        self.settings.sharpness > 0.0 && !self.bypass
    }

    /// For tests: the accumulate pass writes its history weight ([`Fsr2::history_weight`])
    /// from the next frame on; the resources are replaced and the history dropped by the
    /// frame's next size change, so call it before [`crate::Frame::set_sizes`].
    pub fn set_debug(&mut self, device: &wgpu::Device, debug: bool) {
        if debug != self.debug {
            self.debug = debug;
            self.res = Resources::new(device, self.res.render, self.res.display, debug);
        }
    }

    /// For readings: with [`Fsr2::set_debug`] on, the accumulate pass writes each pixel's stage
    /// code ([`Fsr2Stage`]) to [`Fsr2::history_weight`]'s texture instead of the weight.
    pub fn set_debug_stages(&mut self, on: bool) {
        self.stages = on;
    }

    /// Bytes FSR's resources occupy.
    pub fn bytes(&self) -> u64 {
        self.res.bytes()
    }

    /// The internal colour the composite writes in FSR mode.
    pub(crate) fn colour(&self) -> &wgpu::Texture {
        &self.res.colour
    }

    /// The input pass's device depth (r32float, render size).
    pub fn depth(&self) -> &wgpu::Texture {
        &self.res.depth
    }

    /// The input pass's motion vectors (rg32float, render size, UV to the previous frame).
    pub fn motion(&self) -> &wgpu::Texture {
        &self.res.motion
    }

    /// The input pass's masks (rgba8unorm, render size: x reactive, y the
    /// transparency-and-composition mask, z the edit flag).
    pub fn reactive(&self) -> &wgpu::Texture {
        &self.res.reactive
    }

    /// The composite's linear colour FSR reads (rgba32float, render size).
    pub fn colour_in(&self) -> &wgpu::Texture {
        &self.res.colour
    }

    /// The exposure texel FSR reads (rg32float 1 x 1, x).
    pub fn exposure(&self) -> &wgpu::Texture {
        &self.res.exposure
    }

    /// The depth clip's prepared colour (rgba16float, render size: rgb, the depth clip in a).
    pub fn prepared(&self) -> &wgpu::Texture {
        &self.res.prepared
    }

    /// The depth clip's dilated reactive masks (rgba8unorm, render size: the dilated reactive
    /// mask in x, the accumulation mask in y).
    pub fn dilated_reactive(&self) -> &wgpu::Texture {
        &self.res.dilated_reactive
    }

    /// The reconstruct pass's dilated motion copy `i` (rg32float, render size, UV).
    pub fn dilated_motion(&self, i: usize) -> &wgpu::Texture {
        &self.res.dilated_motion[i]
    }

    /// The internal upscaled colour copy `i` (rgba16float, display size).
    pub fn upscaled(&self, i: usize) -> &wgpu::Texture {
        &self.res.upscaled[i]
    }

    /// The lock status copy `i` (rg32float, display size: lifetime, temporal luma).
    pub fn lock_status(&self, i: usize) -> &wgpu::Texture {
        &self.res.lock_status[i]
    }

    /// The accumulate pass's history weight of the last encoded frame (r32float, display
    /// size with [`Fsr2::set_debug`]).
    pub fn history_weight(&self) -> &wgpu::Texture {
        &self.res.weight
    }
}

/// One display pixel's history in the accumulate pass, decoded from the stage code it writes
/// under [`Fsr2::set_debug_stages`] (accumulate.wgsl's `stage_code`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Fsr2Stage {
    /// No history: the pixel reprojects off screen, or the frame resets.
    pub new_sample: bool,
    /// A lock reprojected from the frame before.
    pub was_locked: bool,
    /// That lock released this frame (the luminance change, the reactive mask or the depth
    /// clip set its lifetime to 0).
    pub released: bool,
    /// The history lay outside the scaled rectification box and was clamped toward it.
    pub clamped: bool,
    /// How far the clamp went, 0 to 1 (`1 - contribution` of AMD's `RectifyHistory`), in
    /// 1/255 steps.
    pub clamp: f32,
    /// The depth clip factor, 0 to 1, in 1/255 steps.
    pub depth_clip: f32,
    /// The history's share of the blend, 0 to 1, in 1/15 steps.
    pub history_weight: f32,
}

impl Fsr2Stage {
    /// The stage code `v` decoded; `None` for a value that is no code (not an integer under
    /// 2^24).
    pub fn decode(v: f32) -> Option<Self> {
        if !(0.0..16_777_216.0).contains(&v) || v.fract() != 0.0 {
            return None;
        }
        let c = v as u32;
        let byte = |shift: u32| ((c >> shift) & 0xFF) as f32 / 255.0;
        Some(Self {
            new_sample: c & 1 != 0,
            was_locked: c & 2 != 0,
            released: c & 4 != 0,
            clamped: c & 8 != 0,
            clamp: byte(4),
            depth_clip: byte(12),
            history_weight: ((c >> 20) & 0xF) as f32 / 15.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stage code's fields come back from the bit layout `stage_code` writes: flags in
    /// bits 0..3, the clamp in 4..11, the depth clip in 12..19, the weight in 20..23; a
    /// fraction or a value past 2^24 is no code.
    #[test]
    fn a_stage_code_decodes_its_fields() {
        let code = (1 | 2 | 8) | (128 << 4) | (255 << 12) | (15 << 20);
        let s = Fsr2Stage::decode(code as f32).unwrap();
        assert_eq!(
            s,
            Fsr2Stage {
                new_sample: true,
                was_locked: true,
                released: false,
                clamped: true,
                clamp: 128.0 / 255.0,
                depth_clip: 1.0,
                history_weight: 1.0,
            }
        );
        assert_eq!(Fsr2Stage::decode(4.0).map(|s| s.released), Some(true));
        assert_eq!(Fsr2Stage::decode(0.5), None);
        assert_eq!(Fsr2Stage::decode(16_777_216.0), None);
    }

    /// FSR's passes are the frame's last slots but the exposure pass, in their order.
    #[test]
    fn fsrs_passes_close_the_frame_passes() {
        let n = crate::FRAME_PASSES.len() - 1 - FSR2_PASSES.len();
        assert_eq!(crate::FRAME_PASSES[n..n + FSR2_PASSES.len()], FSR2_PASSES);
        assert_eq!(crate::FRAME_PASSES.last(), Some(&"exposure"));
        assert_eq!(crate::FRAME_PASSES[n - 2..n], ["upscale", "sharpen"]);
    }
}
