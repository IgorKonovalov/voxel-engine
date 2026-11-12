//! The frame: the hit pass ([`Marcher`], `march.wgsl`), the light pass (`light.wgsl`), the
//! probe volume's passes (`probe_gather.wgsl`, ADR-0023) and the composite (`composite.wgsl`),
//! each a compute dispatch over the internal target (ADR-0018).

use crate::camera::{Camera, View};
use crate::exposure::{
    EX_AUTO, EX_METER, EX_RESET, Exposure, ExposurePass, ExposureReadout, ExposureUniform,
    METER_TILE, Metering,
};
use crate::fsr2::{Fsr2, UpscalerKind};
use crate::gpu_util::*;
use crate::gpu_world::GpuWorld;
use crate::light::LightParams;
use crate::probes::{DEFAULT_PROBE_SPACING, ProbeUniform, ProbeVolume};
use crate::{
    ATROUS_WGSL, COMPOSITE_WGSL, GRADIENT_WGSL, LIGHT_WGSL, MARCH_WGSL, PROBE_GATHER_WGSL,
    PROBE_UPDATE_WGSL, TEMPORAL_WGSL,
};
use vxe_world::ChunkCoord;

mod baked;
mod encode;
mod marcher;
mod pipelines;
mod probe_channel;
mod readback_ring;
mod settings;
mod state;
mod targets;
mod timer;
mod upscale;

use self::baked::*;
pub use self::encode::*;
pub use self::marcher::*;
pub(crate) use self::pipelines::*;
use self::probe_channel::*;
use self::readback_ring::*;
pub use self::settings::*;
pub(crate) use self::state::*;
use self::targets::*;
pub use self::timer::*;
pub use self::upscale::*;

/// One frame of the renderer: the hit pass ([`Marcher`]), the light pass, the temporal pass
/// and the composite, with their targets at one internal resolution. [`Frame::target`] is the
/// `rgba8unorm` image the presenter draws.
///
/// The hit buffer and the history are kept twice, this frame's and the last: [`Frame::encode`]
/// writes one copy and reads the other, and [`Frame::advance`] swaps them. History holds while
/// frames are encoded with `encode` and advanced; [`Frame::render`] draws one frame alone,
/// with no history.
pub struct Frame {
    device: wgpu::Device,
    pipes: Pipelines,
    state: FrameState,
    up: UpscaleState,
    baked: BakeLink,
    probe: ProbeChannel,
    targets: Targets,
    /// The copy of the double-buffered targets the next `encode` writes.
    cur: usize,
    resolution: LightingResolution,
    /// The output size the frame's image is presented at; the targets are at the internal
    /// size.
    output: [u32; 2],
    /// The exposure pass, its state and its read-back ring (ADR-0033).
    exposure: ExposurePass,
    metering: Metering,
    /// The exposure pass's step in seconds ([`Frame::set_dt`]).
    dt: f32,
    /// The next encode's exposure pass jumps the adapted level to the meter.
    reset_exposure: std::sync::atomic::AtomicBool,
    /// A copy of the composite's image at the internal size, taken right after the composite
    /// pass ([`Frame::set_composite_capture`]); `None` unless a test or tool asked for it.
    composite_capture: Option<wgpu::Texture>,
}

impl Frame {
    /// A frame drawing into a `width` × `height` target, presented at that size.
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        Self::with_sizes(device, [width, height], [width, height])
    }

    /// A frame rendering at `internal` pixels for an image presented at `output` pixels.
    pub fn with_sizes(device: &wgpu::Device, internal: [u32; 2], output: [u32; 2]) -> Self {
        let [width, height] = internal;
        Self {
            pipes: Pipelines::new(device),
            targets: Targets::new(
                device,
                width,
                height,
                LightingResolution::default().scale(),
                (internal != output).then_some(output),
                true,
            ),
            baked: BakeLink::new(device),
            state: FrameState::new(device),
            up: UpscaleState::new(device),
            probe: ProbeChannel::new(),
            cur: 0,
            resolution: LightingResolution::default(),
            output,
            device: device.clone(),
            exposure: ExposurePass::new(device),
            metering: Metering::default(),
            dt: 1.0 / 60.0,
            reset_exposure: std::sync::atomic::AtomicBool::new(true),
            composite_capture: None,
        }
    }

    /// Sets the frame's step in seconds (1/60 by default): the time the exposure pass's
    /// adapted level moves over in each encoded frame, and FSR 2's `delta_time`. A negative or
    /// non-finite `dt` is 0, which holds the exposure.
    pub fn set_dt(&mut self, seconds: f32) {
        self.dt = if seconds.is_finite() {
            seconds.max(0.0)
        } else {
            0.0
        };
    }

    pub fn dt(&self) -> f32 {
        self.dt
    }

    /// Makes the next frame's exposure pass that meters jump the adapted level to the meter
    /// (a new frame starts so).
    pub fn reset_exposure(&mut self) {
        self.reset_exposure
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Sets how the exposure pass weights the meter's tiles under a manual exposure
    /// (centre-weighted by default); in auto, [`crate::AutoExposure::metering`] does.
    pub fn set_metering(&mut self, metering: Metering) {
        self.metering = metering;
    }

    pub fn metering(&self) -> Metering {
        self.metering
    }

    /// The exposure state as last read back, without waiting: a non-blocking poll of
    /// `device`, then the newest of the read-back ring's slots that has finished mapping
    /// (two or three frames late), else the last one read; `None` until the first. Each
    /// encode's commands must be submitted before the next encode or this call.
    pub fn exposure_readout(&self, device: &wgpu::Device) -> Option<ExposureReadout> {
        self.exposure.readout(device)
    }

    /// The exposure state buffer the exposure pass writes (32 bytes, `COPY_SRC`:
    /// [`crate::exposure::ExposureState::from_bytes`] reads a copy), for tests and tools.
    pub fn exposure_state(&self) -> &wgpu::Buffer {
        self.exposure.state()
    }

    /// The meter's tiles the composite writes (`COPY_SRC`): per 8 x 8 workgroup of the
    /// internal target, row-major over `ceil(w / 8) x ceil(h / 8)`, the sum of its pixels'
    /// clamped log2 luminance and their count, as two `f32`.
    pub fn exposure_tiles(&self) -> &wgpu::Buffer {
        &self.targets.tiles
    }

    /// Whether the last encode with history reset the lamp's history
    /// ([`LIGHTING_LAMP_CHANGED`]): the lamp's switch, its parameters or its shadow walk
    /// differ from the encode before.
    pub fn lamp_changed(&self) -> bool {
        self.state
            .lamp_changed
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The size the hit pass and the lighting passes render at.
    pub fn internal_size(&self) -> [u32; 2] {
        [self.targets.width, self.targets.height]
    }

    /// The size the frame's image is presented at.
    pub fn output_size(&self) -> [u32; 2] {
        self.output
    }

    /// The copy of the double-buffered targets the last (or next) encode writes.
    pub fn current_copy(&self) -> usize {
        self.cur
    }

    /// The frame index the next `params` seeds its random numbers with.
    pub fn frame_index(&self) -> u32 {
        self.state.frame
    }

    pub fn set_frame_index(&mut self, frame: u32) {
        self.state.frame = frame;
    }

    fn history(&mut self) -> &mut History {
        self.state
            .history
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Ends the frame last encoded: the next frame index, its hit buffer and history become
    /// the previous frame's, and the double-buffered targets swap. Call once per frame, after
    /// [`Frame::encode`].
    pub fn advance(&mut self) {
        self.state.frame = self.state.frame.wrapping_add(1);
        let h = self.history();
        h.prev_views = h.last_views;
        h.prev_offset = h.last_offset;
        h.valid = h.encoded;
        h.encoded = false;
        self.cur ^= 1;
    }

    /// Kills the history of every pixel whose hit lies in the voxels `min ..= max` dilated by
    /// one voxel, in the next encoded frame: an edit's `EditResult::aabb`. More than
    /// [`MAX_EDITS`] boxes in one frame flush all history. With the lamp channel running, the
    /// next encoded frame also restarts every probe within the lamp's reach whose cell the box
    /// touches ([`ProbeVolume::touched`]): it blends at hysteresis 0, its age at 1; boxes past
    /// [`MAX_EDITS`] touch no probe, which then refresh in their slices.
    /// A store holding a piece also keeps the box for [`Frame::rebake_digs`]; under the live
    /// queue the box goes to the queue instead, which re-bakes its rings over its slices.
    pub fn invalidate(&mut self, min: [i32; 3], max: [i32; 3]) {
        if self.baked.store.piece().is_some() && self.baked.queue.is_none() {
            self.baked.dig_boxes.push([min, max]);
        }
        if let Some(q) = self.baked.queue.as_mut() {
            q.invalidate(min, max);
        }
        let h = self.history();
        if h.edits.len() == MAX_EDITS {
            h.overflow = true;
        } else {
            h.edits.push([min, max]);
        }
    }

    /// Drops all history: the next encoded frame starts every pixel afresh.
    pub fn reset_history(&mut self) {
        self.history().valid = false;
    }

    /// Encodes and submits one frame of `camera` over `world` with history, after advancing
    /// from the frame before: what a frame loop does, for tests and tools.
    pub fn step(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        world: &GpuWorld,
        camera: &Camera,
        light: &LightParams,
    ) {
        self.advance();
        let params = self.params(world, camera, light);
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        self.encode(device, queue, &mut enc, world, &params, None);
        queue.submit([enc.finish()]);
    }

    /// Scales material `material`'s emission by `scale` from the next frame on, in the
    /// pixels' light (and the probe rows, see [`Frame::set_emission_scales`]): a visual knob that leaves the world untouched
    /// (an authoritative change is a `Replace` op).
    pub fn set_emission_scale(&mut self, queue: &wgpu::Queue, material: u16, scale: f32) {
        self.set_emission_scales(queue, material, scale, scale);
    }

    /// Scales material `material`'s emission by `pixels` in the light the frame's pixels
    /// sample (the light pass, the composite) and by `probes` in the light the probe rays
    /// sample. An emitter that varies over time passes its current scale and its time
    /// average. The probe rows are kept for a per-source bounce channel; the lamp channel
    /// (ADR-0043) reads none of them, so `probes` changes nothing a frame shows.
    pub fn set_emission_scales(
        &mut self,
        queue: &wgpu::Queue,
        material: u16,
        pixels: f32,
        probes: f32,
    ) {
        let i = usize::from(material);
        for (row, scale) in [(i, pixels), (EMISSION_ROWS + i, probes)] {
            if self.state.emission_cpu[row] != scale {
                self.state.emission_cpu[row] = scale;
                if row == i {
                    self.state.emission_changed_at = Some(self.state.frame);
                }
                queue.write_buffer(
                    &self.state.emission_scale,
                    row as u64 * 4,
                    bytemuck::bytes_of(&scale),
                );
            }
        }
    }

    /// Material `material`'s emission scale in the pixels' light.
    pub fn emission_scale(&self, material: u16) -> f32 {
        self.state.emission_cpu[usize::from(material)]
    }

    /// Material `material`'s emission scale in the probe rays' light.
    pub fn probe_emission_scale(&self, material: u16) -> f32 {
        self.state.emission_cpu[EMISSION_ROWS + usize::from(material)]
    }

    /// The emission-scale buffer: an `f32` per material id for the pixels' light, then one
    /// per material id for the probe rays' (see [`Frame::set_emission_scales`]).
    pub fn emission_scales(&self) -> &wgpu::Buffer {
        &self.state.emission_scale
    }

    /// Sets how LOD-only chunks pick their mip level.
    pub fn set_lod(&mut self, lod: LodParams) {
        self.state.lod = lod;
    }

    pub fn lod(&self) -> LodParams {
        self.state.lod
    }

    /// Sets the surface look: normal mode and view. A change of view drops the
    /// upscaler's history (not the lighting's).
    pub fn set_look(&mut self, look: LookParams) {
        if look.view != self.state.look.view {
            self.history().view_changed = true;
        }
        self.state.look = look;
    }

    pub fn look(&self) -> LookParams {
        self.state.look
    }

    /// Whether the field look walks split ([`FLAG_FIELD_SPLIT`], the default): the cube
    /// walk's pipeline carries each ray to the first brick the field can hit in, and the field
    /// walk goes on from there. Off, the field walk carries every ray from its origin. Either
    /// gives the same hit records; the split walks only when the world's table holds a crisp
    /// row ([`GpuWorld::crisp_rows`]), outside the cracks and steps views.
    pub fn set_field_split(&mut self, on: bool) {
        self.state.field_split = on;
    }

    /// Sets the lighting knobs; a change of [`Lighting::bake_pool_bytes`] rebuilds an allocated
    /// store empty.
    pub fn set_lighting(&mut self, lighting: Lighting) {
        self.state.lighting = lighting;
        self.fit_store(false);
        self.rebuild_probes();
    }

    pub fn lighting(&self) -> Lighting {
        self.state.lighting
    }

    /// The frame's `rgba8unorm` image: the composite's output, or with the upscaler (see
    /// [`Frame::upscaling`]) the upscaled image at the output size. With the upscaler the
    /// composite writes the internal image into its top-left pixels and the upscale pass's
    /// sharpening pass overwrites the whole of it.
    pub fn target(&self) -> &wgpu::Texture {
        &self.targets.output
    }

    /// The hit buffer's position texture the next or last encode writes (rgba32float: world
    /// position in voxels, then `t`, `3e38` on a miss).
    pub fn hit_positions(&self) -> &wgpu::Texture {
        &self.targets.hit_pos[self.cur]
    }

    /// The hit buffer's record texture (rgba16uint: octahedral normal u and v as snorm16
    /// bits, material id, flags: bit 0 hit, bit 1 field-surface hit, bit 2 translucent layers
    /// in front, bits 3..5 the entry face's axis plus one, 0 for none).
    pub fn hit_records(&self) -> &wgpu::Texture {
        &self.targets.hit_info[self.cur]
    }

    /// The translucent solid in front of each hit (rgba16float: its reflected light before
    /// exposure in rgb, the luminance of its transmittance in a; ADR-0047).
    pub fn layers(&self) -> &wgpu::Texture {
        &self.targets.layer
    }

    /// The translucent solid's transmittance per channel in front of each hit (r32uint, rgb9e5;
    /// `bake::rgb9e5_unpack` reads a texel), 1 where none is in front.
    pub fn transmittance(&self) -> &wgpu::Texture {
        &self.targets.transmit
    }

    /// The light pass's irradiance (rgba16float: rgb, then the emissive part's luminance).
    pub fn irradiance(&self) -> &wgpu::Texture {
        &self.targets.irradiance
    }

    /// Turns the composite capture on or off: while on, every encode copies the composite's
    /// image (rgba8unorm, display-encoded, at the internal size) into
    /// [`Frame::composite_capture`] right after the composite pass, before the upscaler
    /// overwrites the target. For tests and tools that read the composite apart from the
    /// upscaled display; it costs one copy a frame and is not counted in
    /// [`Frame::frame_bytes`]. Under FSR 2 the composite writes FSR's linear input instead
    /// ([`Fsr2::colour_in`]), and nothing is captured.
    pub fn set_composite_capture(&mut self, device: &wgpu::Device, on: bool) {
        self.composite_capture =
            on.then(|| composite_capture_texture(device, self.internal_size()));
    }

    /// The composite capture the last encode wrote, or `None` while it is off.
    pub fn composite_capture(&self) -> Option<&wgpu::Texture> {
        self.composite_capture.as_ref()
    }

    /// The temporal pass's accumulated irradiance (rgba16float: the emissive part in rgb, the
    /// lamp's in a).
    pub fn accumulated(&self) -> &wgpu::Texture {
        &self.targets.accum[self.cur]
    }

    /// The filter's output, the irradiance the composite shades with when the filter runs
    /// (rgba16float: irradiance rgb, variance a).
    pub fn filtered(&self) -> &wgpu::Texture {
        &self.targets.filter[ATROUS_ITERATIONS % 2]
    }

    /// The temporal pass's moments (rgba16float: luminance, its square, the age in frames).
    pub fn moments(&self) -> &wgpu::Texture {
        &self.targets.moments[self.cur]
    }

    /// Bytes the frame's targets, its probe volume (once built), FSR 2's resources and the store's
    /// pool (once allocated) occupy.
    pub fn frame_bytes(&self) -> u64 {
        self.targets.bytes()
            + self.probe.volume.get().map_or(0, ProbeVolume::bytes)
            + self.exposure.bytes()
            + self.up.fsr.as_ref().map_or(0, Fsr2::bytes)
            + if self.baked.store.pool().size() > crate::bake::MINIMAL_POOL_BYTES {
                self.baked.store.pool().size()
            } else {
                0
            }
    }

    /// Replaces the targets with ones of `width` × `height`, presented at that size, and
    /// drops the history; the pipelines are kept.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        self.set_sizes(device, [width, height], [width, height]);
    }

    /// Replaces the targets with ones of `internal` pixels for an image presented at
    /// `output` pixels and drops the history; the pipelines are kept.
    pub fn set_sizes(&mut self, device: &wgpu::Device, internal: [u32; 2], output: [u32; 2]) {
        self.output = output;
        let fsr = self.up.fsr.is_some();
        let upscaled = (fsr || self.up.jitter || internal != output).then_some(output);
        self.targets = Targets::new(
            device,
            internal[0],
            internal[1],
            self.resolution.scale(),
            upscaled,
            !fsr,
        );
        if let Some(f) = self.up.fsr.as_mut() {
            f.resize(device, internal, output);
        }
        if self.composite_capture.is_some() {
            self.composite_capture = Some(composite_capture_texture(device, internal));
        }
        self.reset_history();
    }

    /// Runs the lighting passes at `resolution`; the lighting targets are replaced and the
    /// history dropped when it changes.
    pub fn set_resolution(&mut self, device: &wgpu::Device, resolution: LightingResolution) {
        if resolution != self.resolution {
            self.resolution = resolution;
            self.set_sizes(device, self.internal_size(), self.output);
        }
    }

    pub fn resolution(&self) -> LightingResolution {
        self.resolution
    }
}
