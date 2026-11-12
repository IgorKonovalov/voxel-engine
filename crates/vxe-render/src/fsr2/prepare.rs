//! FSR 2's per-frame work: its constants, the exposure seam, and the bind groups over the
//! frame's targets.

use super::*;

/// `cbFSR2` of AMD's passes plus the luminance pyramid's and RCAS's constants:
/// `Fsr2Constants` in `common.wgsl`, field for field.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Fsr2Constants {
    pub render_size: [i32; 2],
    pub max_render_size: [i32; 2],
    pub display_size: [i32; 2],
    pub input_color_size: [i32; 2],
    pub luma_mip_size: [i32; 2],
    pub luma_mip_level: i32,
    pub frame_index: i32,
    pub device_to_view: [f32; 4],
    pub jitter: [f32; 2],
    pub motion_vector_scale: [f32; 2],
    pub downscale: [f32; 2],
    pub motion_vector_jitter_cancellation: [f32; 2],
    pub pre_exposure: f32,
    pub previous_pre_exposure: f32,
    pub tan_half_fov: f32,
    pub jitter_sequence_length: f32,
    pub delta_time: f32,
    pub dynamic_res_change: f32,
    pub view_space_to_meters: f32,
    pub spd_mips: u32,
    pub spd_work_groups: u32,
    pub rcas_con: f32,
    pub debug: u32,
    /// The tone arithmetic RCAS applies: bit 0 the identity tone map, bit 1 the shaded view,
    /// bit 2 the static dither.
    pub tone: u32,
}

/// The shading-change mip of the luminance pyramid (`FFX_FSR2_SHADING_CHANGE_MIP_LEVEL`).
pub(crate) const SHADING_CHANGE_MIP: i32 = 4;

/// `Fsr2InputTuning` in `input.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct InputUniform {
    pub(super) near: f32,
    pub(super) reactive_translucent: f32,
    pub(super) reactive_light: f32,
    pub(super) _pad: f32,
}

/// The frame's targets FSR reads and writes: this frame's hit buffer and translucent layers,
/// the edit boxes and the upscale pass's tuning uniforms (the older lamp), the frame's rgba8
/// image, and the copy of the double-buffered resources this frame writes (the other is last
/// frame's).
pub(crate) struct FrameInputs<'a> {
    pub hit_pos: &'a wgpu::Texture,
    pub hit_info: &'a wgpu::Texture,
    pub layer: &'a wgpu::Texture,
    pub edits: &'a wgpu::Buffer,
    pub lamp: &'a wgpu::Buffer,
    pub output: &'a wgpu::Texture,
    /// The frame's blue noise (`blue_noise` in `march.wgsl`), RCAS's dither.
    pub blue_noise: &'a wgpu::Texture,
    pub cur: usize,
    /// The frame's step in seconds ([`crate::Frame::set_dt`]).
    pub dt: f32,
}

impl Fsr2 {
    /// FSR's constants for a frame of `params` (its first view), `reset` when history is
    /// dropped.
    pub(super) fn constants(&self, params: &Params, reset: bool, dt: f32) -> Fsr2Constants {
        let render = self.res.render.map(|n| n as i32);
        let display = self.res.display.map(|n| n as i32);
        let v = &params.views[0];
        let tan_h = (v.tan_left + v.tan_right) * 0.5;
        let tan_v = (v.tan_up + v.tan_down) * 0.5;
        let mut prev = self
            .pre_exposure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // The manual exposure; in auto, `copy_exposure` replaces it (and `previous_pre_exposure`
        // in both) on the GPU from the exposure state.
        let pre_exposure = params.exposure;
        let [w, h] = self.res.render;
        let c = Fsr2Constants {
            render_size: render,
            max_render_size: render,
            display_size: display,
            input_color_size: render,
            luma_mip_size: render.map(|n| (n as f32 / (2 << SHADING_CHANGE_MIP) as f32) as i32),
            luma_mip_level: SHADING_CHANGE_MIP,
            frame_index: if reset { 0 } else { 1 },
            // setupDeviceDepthToViewSpaceDepthParams for inverted, infinite depth.
            device_to_view: [-f32::EPSILON, NEAR, tan_h, tan_v],
            // The sample of render pixel p lies at p + 0.5 - jitter (see `jitter_offset`).
            jitter: [-v.jitter[0], -v.jitter[1]],
            motion_vector_scale: [1.0, 1.0],
            downscale: [
                render[0] as f32 / display[0] as f32,
                render[1] as f32 / display[1] as f32,
            ],
            motion_vector_jitter_cancellation: [0.0; 2],
            pre_exposure,
            previous_pre_exposure: if reset { pre_exposure } else { *prev },
            tan_half_fov: tan_h,
            jitter_sequence_length: self.settings.phases(w, self.res.display[0]) as f32,
            delta_time: dt,
            dynamic_res_change: 0.0,
            view_space_to_meters: self.settings.metres_per_voxel,
            // SpdSetup over the whole render rectangle.
            spd_mips: ((w.max(h) as f32).log2().floor() as u32).min(12),
            spd_work_groups: w.div_ceil(64) * h.div_ceil(64),
            // FsrRcasCon of `-2 sharpness + 2` stops.
            rcas_con: (-(2.0 - 2.0 * self.settings.sharpness.clamp(0.0, 1.0))).exp2(),
            debug: u32::from(self.debug),
            tone: u32::from(params.lighting & crate::frame::LIGHTING_IDENTITY_TONE_MAP != 0)
                | u32::from((params.flags >> crate::frame::VIEW_SHIFT) & 7 == 0) << 1
                | u32::from(params.dither != 0) << 2,
        };
        *prev = pre_exposure;
        c
    }

    /// The pre-exposure seam (ADR-0033): the composite's colour reaches FSR pre-exposed by the
    /// frame's exposure, so FSR's `pre_exposure` and its exposure texel are that exposure and
    /// `previous_pre_exposure` the last frame's, and the accumulate pass rescales its history by
    /// their ratio. `state` is the exposure state (`current` at byte 0, `previous` at 4) as this
    /// frame's passes read it; the copies go after [`Fsr2::prepare`]'s writes and before FSR's
    /// passes. In manual (`auto` false) `pre_exposure` and the texel stay the CPU's value. On a
    /// `reset` the previous pre-exposure is this frame's.
    pub(crate) fn copy_exposure(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        state: &wgpu::Buffer,
        auto: bool,
        reset: bool,
    ) {
        let pre = std::mem::offset_of!(Fsr2Constants, pre_exposure) as u64;
        let previous = std::mem::offset_of!(Fsr2Constants, previous_pre_exposure) as u64;
        if auto {
            encoder.copy_buffer_to_buffer(state, 0, &self.cb, pre, 4);
            encoder.copy_buffer_to_texture(
                wgpu::TexelCopyBufferInfo {
                    buffer: state,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: None,
                        rows_per_image: None,
                    },
                },
                self.res.exposure.as_image_copy(),
                self.res.exposure.size(),
            );
        }
        match (reset, auto) {
            (false, _) => encoder.copy_buffer_to_buffer(state, 4, &self.cb, previous, 4),
            (true, true) => encoder.copy_buffer_to_buffer(state, 0, &self.cb, previous, 4),
            (true, false) => {}
        }
    }

    /// Writes this frame's constants, the input pass's tuning and the exposure texel (the
    /// frame's manual exposure, replaced in auto by `copy_exposure`).
    fn write_frame(&self, queue: &wgpu::Queue, params: &Params, dt: f32) {
        let reset = params.history & 2 == 0;
        let c = self.constants(params, reset, dt);
        queue.write_buffer(&self.cb, 0, bytemuck::bytes_of(&c));
        queue.write_buffer(
            &self.input_tuning,
            0,
            bytemuck::bytes_of(&InputUniform {
                near: NEAR,
                reactive_translucent: self.settings.reactive_translucent,
                reactive_light: self.settings.reactive_light,
                _pad: 0.0,
            }),
        );
        let r = &self.res;
        // The exposure texel: the frame's manual exposure, replaced in auto by `copy_exposure`.
        queue.write_texture(
            r.exposure.as_image_copy(),
            bytemuck::cast_slice(&[params.exposure, 0.0]),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: None,
                rows_per_image: None,
            },
            r.exposure.size(),
        );
    }

    /// Writes this frame's constants and builds its bind groups from the frame's targets.
    pub(crate) fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        params: &Params,
        f: &FrameInputs<'_>,
    ) -> Fsr2Binds {
        self.write_frame(queue, params, f.dt);
        let r = &self.res;
        let (w, q) = (f.cur, f.cur ^ 1);
        use Res::{Buf, Tex};
        let cb = bind_io(device, &self.cb_layout, &[Buf(&self.cb)]);
        let input = bind_io(
            device,
            &self.input.layout,
            &[
                Tex(f.hit_pos),
                Tex(f.hit_info),
                Tex(f.layer),
                Buf(&self.input_tuning),
                Buf(f.edits),
                Buf(f.lamp),
                Tex(&r.depth),
                Tex(&r.motion),
                Tex(&r.reactive),
            ],
        );
        let spd_mips = bind_io(
            device,
            &self.spd_mips.layout,
            &[Tex(&r.colour), Tex(&r.luma_mip), Tex(&r.luma_mip5)],
        );
        let spd_tail = bind_io_at(
            device,
            &self.spd_tail.layout,
            &[3, 4, 5],
            &[
                Tex(&r.luma_mip5),
                Tex(&r.auto_exposure[w]),
                Tex(&r.auto_exposure[q]),
            ],
        );
        let reconstruct = bind_io(
            device,
            &self.reconstruct.layout,
            &[
                Tex(&r.depth),
                Tex(&r.motion),
                Tex(&r.colour),
                Tex(&r.exposure),
                Buf(&r.reconstructed),
                Tex(&r.dilated_depth),
                Tex(&r.dilated_motion[w]),
                Tex(&r.lock_luma),
            ],
        );
        let depth_clip = bind_io(
            device,
            &self.depth_clip.layout,
            &[
                Tex(&r.depth),
                Tex(&r.motion),
                Tex(&r.colour),
                Tex(&r.reactive),
                Tex(&r.exposure),
                Tex(&r.dilated_depth),
                Tex(&r.dilated_motion[w]),
                Tex(&r.dilated_motion[q]),
                Buf(&r.reconstructed),
                Tex(&r.prepared),
                Tex(&r.dilated_reactive),
            ],
        );
        let lock = bind_io(
            device,
            &self.lock.layout,
            &[Tex(&r.lock_luma), Buf(&r.new_locks), Buf(&r.reconstructed)],
        );
        let accumulate = bind_io(
            device,
            &self.accumulate.layout,
            &[
                Tex(&r.exposure),
                Tex(&r.dilated_reactive),
                Tex(&r.dilated_motion[w]),
                Tex(&r.upscaled[q]),
                Tex(&r.lock_status[q]),
                Tex(&r.prepared),
                Tex(&r.luma_mip),
                Tex(&r.luma_history[q]),
                Buf(&r.new_locks),
                Tex(&r.lock_status[w]),
                Tex(&r.upscaled[w]),
                Tex(&r.luma_history[w]),
                Tex(&r.weight),
            ],
        );
        // RCAS sharpens the accumulate pass's output and tone maps it into the frame's image.
        let sharpening = self.sharpens();
        let rcas = sharpening.then(|| {
            (
                bind_io(
                    device,
                    &self.rcas.layout,
                    &[
                        Tex(&r.upscaled[w]),
                        Tex(&r.exposure),
                        Tex(f.output),
                        Tex(f.blue_noise),
                    ],
                ),
                r.display.map(|n| n.div_ceil(16)),
            )
        });
        let tone_in = if self.bypass {
            &r.colour
        } else {
            &r.upscaled[w]
        };
        let tone = bind_io(device, &self.tone.layout, &[Tex(tone_in), Tex(f.output)]);
        let [rw, rh] = r.render;
        Fsr2Binds {
            cb,
            input,
            spd_mips,
            spd_tail,
            reconstruct,
            depth_clip,
            lock,
            accumulate,
            rcas,
            tone,
            bypass: self.bypass,
            render_groups: r.render.map(|n| n.div_ceil(WORKGROUP)),
            display_groups: r.display.map(|n| n.div_ceil(WORKGROUP)),
            spd_groups: [rw.div_ceil(64), rh.div_ceil(64)],
        }
    }
}
