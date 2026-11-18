//! The frame: the hit pass ([`Marcher`], `hit.wgsl` over the walk), the light pass (`light.wgsl`), the
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
    ATROUS_WGSL, COMPOSITE_WGSL, GRADIENT_WGSL, LIGHT_WGSL, PROBE_GATHER_WGSL, PROBE_UPDATE_WGSL,
    TEMPORAL_WGSL,
};
use vxe_world::ChunkCoord;

mod apply;
mod baked;
mod compose;
mod dispatch;
mod encode;
mod lights;
mod marcher;
mod output_binds;
mod pipelines;
mod probe_channel;
mod readback_ring;
mod settings;
mod sky;
mod state;
mod targets;
mod timer;
mod upscale;

use self::baked::*;
pub use self::compose::*;
pub(crate) use self::dispatch::*;
pub use self::encode::*;
pub use self::lights::*;
pub use self::marcher::*;
pub(crate) use self::pipelines::*;
use self::probe_channel::*;
use self::readback_ring::*;
pub use self::settings::*;
use self::sky::SkyState;
pub(crate) use self::sky::{SKY_ATMOSPHERE_WGSL, SKY_PASSES, SKY_WGSL};
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
    /// The passes' bind groups, built again only when what they bind changes.
    binds: BindCache,
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
    /// The entities' pool, instances and tree (ADR-0050), group 0's binding 15.
    entities: crate::entities::EntityMirror,
    /// The scale [`Frame::apply_settings`] last applied; [`Frame::settings`] reads it while
    /// the sizes still match it.
    scale: RenderScale,
    /// The `Rebuild` values the caller last built with ([`Frame::built_with`]).
    build: crate::settings::BuildSettings,
    output_hint: Option<[u32; 2]>,
    /// [`Frame::history_restarts`].
    restarts: u64,
    /// The sky, its uniform and LUTs ([`Frame::set_sky`]).
    sky: SkyState,
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
            binds: BindCache::default(),
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
            entities: crate::entities::EntityMirror::new(
                device,
                crate::entities::DEFAULT_ENTITY_POOL_BYTES,
            ),
            scale: RenderScale::One,
            build: crate::settings::BuildSettings::default(),
            output_hint: None,
            restarts: 0,
            sky: SkyState::new(device),
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

    /// Whether the last encode with history found the headlamp's light changed: the lamp's
    /// switch, its parameters or its shadow walk differ from the encode before. Such a frame
    /// sets [`LIGHTING_LIGHTS_CHANGED`], as a changed list does ([`Frame::lights_changed`]).
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
        self.restarts += 1;
        let h = self.history();
        h.valid = false;
        h.encoded = false;
    }

    /// The restarts of the temporal history so far: each [`Frame::reset_history`] (a size, an
    /// upscaler, a jitter or a lighting resolution change makes one) and each drop of the
    /// upscaler's history alone ([`Frame::reset_upscaler_history`], a change of view). An
    /// edit's history kill is not one.
    pub fn history_restarts(&self) -> u64 {
        self.restarts
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

    /// The render-side style word of each material id (`styles[m]` for material `m`, 0 for the
    /// ids past its end; bits: [`SHADOW_AS_BRICK`], [`SHADOW_ABSORB`] with its sigma), from the
    /// next frame on: how the renderer treats the material, leaving the world untouched. Writes
    /// only when the words change.
    pub fn set_material_styles(&mut self, queue: &wgpu::Queue, styles: &[u32]) {
        let len = styles.iter().rposition(|&w| w != 0).map_or(0, |i| i + 1);
        let styles = &styles[..len.min(EMISSION_ROWS)];
        if styles == self.state.styles.as_slice() {
            return;
        }
        let rows = styles.len().max(self.state.styles.len());
        let words: Vec<f32> = (0..rows)
            .map(|i| styles.get(i).map_or(0.0, |&w| w as f32))
            .collect();
        queue.write_buffer(
            &self.state.emission_scale,
            (2 * EMISSION_ROWS * 4) as u64,
            bytemuck::cast_slice(&words),
        );
        self.state.styles = styles.to_vec();
    }

    /// The style words [`Frame::set_material_styles`] last set, trailing zeros dropped.
    pub fn material_styles(&self) -> &[u32] {
        &self.state.styles
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
            self.reset_upscaler_history();
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

    /// Bind groups the frame's passes created so far, FSR 2's and the exposure pass's
    /// included: a pass takes its group back while what it binds is unchanged, so an encode
    /// whose targets, buffers and passes the last two encodes also had creates none.
    pub fn bind_groups_built(&self) -> u64 {
        self.binds.built().0 + self.exposure.binds_built().0
    }

    /// Texture views the frame's bind groups created so far ([`Frame::bind_groups_built`]).
    pub fn views_built(&self) -> u64 {
        self.binds.built().1 + self.exposure.binds_built().1
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

    /// The lighting grid's hit positions the next or last encode writes: [`Frame::hit_positions`]
    /// at the full tier, the half tier's downsampled hits otherwise; the pixels of
    /// [`Frame::irradiance`], [`Frame::accumulated`] and [`Frame::filtered`].
    pub fn light_hit_positions(&self) -> &wgpu::Texture {
        self.targets.lit_pos(self.cur)
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

    /// The light pass's irradiance (rgba16float: the emissive sample in rgb; in a 1 at a fresh
    /// pixel, one whose shadow ray entered the box of an entity with a moved part or whose hit
    /// is a moved part's voxel, else 0; in the steps view the list's shadow rays' counters, the
    /// rays cast in a).
    pub fn irradiance(&self) -> &wgpu::Texture {
        &self.targets.irradiance
    }

    /// The light pass's direct part (rgba16float: the list of lights' terms, each times its
    /// colour, in rgb; their luminance in a; pre-exposed).
    pub fn direct(&self) -> &wgpu::Texture {
        &self.targets.direct
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

    /// The temporal pass's accumulated irradiance (rgba16float: the emissive part in rgb; in a
    /// 1 at a fresh pixel, as [`Frame::irradiance`], else 0).
    pub fn accumulated(&self) -> &wgpu::Texture {
        &self.targets.accum[self.cur]
    }

    /// The a-trous iterations the last encode recorded: [`Lighting::atrous_count`] while the
    /// filter runs, 0 while it is off or the view does not shade.
    pub fn atrous_dispatches(&self) -> u32 {
        self.state
            .atrous_dispatches
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The temporal pass's accumulated direct part (rgba16float: as [`Frame::direct`]).
    pub fn direct_accumulated(&self) -> &wgpu::Texture {
        &self.targets.direct_accum[self.cur]
    }

    /// The light pass's visibility samples (rgba8unorm, ADR-0070): the sky rays' share that
    /// reached the sky in r, and 1 in a where the sky rays were cast this frame, else 0.
    pub fn visibility_samples(&self) -> &wgpu::Texture {
        &self.targets.vis
    }

    /// The temporal pass's visibility pair (rgba16float, ADR-0070): the sky visibility's
    /// running mean in r, the key light's in g (0), the sky's short mean in b, its age in a.
    pub fn visibility_pair(&self) -> &wgpu::Texture {
        &self.targets.pair[self.cur]
    }

    /// The filter's output, the irradiance the composite shades with when the filter runs
    /// (rgba16float: irradiance rgb, variance a): the target the last of
    /// [`Lighting::atrous_iterations`] wrote.
    pub fn filtered(&self) -> &wgpu::Texture {
        &self.targets.filter[self.state.lighting.atrous_count() % 2]
    }

    /// The gradient pass's strata, one texel per 3 x 3 lighting pixels (rgba16float: lambda,
    /// the re-shaded and the last frame's direct luminance, 0); all 0 outside the gradient mode.
    pub fn gradient_strata(&self) -> &wgpu::Texture {
        &self.targets.strata
    }

    /// The temporal pass's moments (rgba16float: luminance, its square, the age in frames).
    pub fn moments(&self) -> &wgpu::Texture {
        &self.targets.moments[self.cur]
    }

    /// The entity mirror the frame draws (ADR-0050): its pool takes models
    /// ([`crate::EntityMirror::upload_model`]).
    pub fn entity_mirror(&self) -> &crate::EntityMirror {
        &self.entities
    }

    pub fn entity_mirror_mut(&mut self) -> &mut crate::EntityMirror {
        &mut self.entities
    }

    /// Builds every pipeline a frame that draws entities uses (the hit pass's and the light
    /// pass's entity variants, and the no-entity light pass), so the first frame that draws
    /// a part builds none: a driver may take seconds over the entity walk. Call it once,
    /// during start-up, after creating the frame; without it each is built on the first frame
    /// that uses it.
    pub fn prepare_entities(&self, device: &wgpu::Device) {
        self.pipes.prepare_entities(device);
    }

    /// Replaces the entity mirror (a pool of another size); the next frames draw no entity
    /// until [`Frame::set_entities`].
    pub fn set_entity_mirror(&mut self, mirror: crate::EntityMirror) {
        self.entities = mirror;
    }

    /// The entities the next frames draw: one instance a part of each entity of `list` as
    /// `set` poses it, within the caps ([`crate::EntityMirror::set_entities`]). Call at most
    /// once a frame (the hit record holds one bit of a slot's generation, so two calls between
    /// frames can hide a slot's new occupant), and every frame while anything moves: the
    /// transforms of the call before are the frame before's. An entity left out of `list`
    /// stays in the tree as a ghost leaf until the next call, so make one more call after the
    /// list last changed ([`crate::EntityMirror::walks_entities`] is true until then).
    pub fn set_entities(
        &mut self,
        queue: &wgpu::Queue,
        set: &vxe_entity::EntitySet,
        list: &[(vxe_entity::EntityId, crate::ModelHandle)],
    ) {
        self.entities.set_entities(queue, set, list);
    }

    /// [`Frame::set_entities`] with each entity's parts placed by a [`crate::DisplayPose`]
    /// (between two ticks, say) in place of a pose in a set
    /// ([`crate::EntityMirror::set_entity_displays`]), under the same call rules.
    pub fn set_entity_displays<'a>(
        &mut self,
        queue: &wgpu::Queue,
        list: impl IntoIterator<
            Item = (
                vxe_entity::EntityId,
                crate::ModelHandle,
                &'a crate::DisplayPose,
            ),
        >,
    ) {
        self.entities.set_entity_displays(queue, list);
    }

    /// Entities the last [`Frame::set_entities`] was given and did not draw.
    pub fn entities_over_cap(&self) -> usize {
        self.entities.entities_over_cap()
    }

    /// Whether any part the last [`Frame::set_entities`] drew moved since the call before: its
    /// transform differs, so its pixels and the pixels whose shadow rays cross its entity's box
    /// are fresh ([`Frame::irradiance`]).
    pub fn entities_moved(&self) -> bool {
        self.entities.entities_moved()
    }

    /// Bytes the frame's targets, its probe volume (once built), FSR 2's resources, the store's
    /// pool (once allocated) and the entity mirror ([`crate::EntityMirror::bytes`]) occupy.
    pub fn frame_bytes(&self) -> u64 {
        self.targets.bytes()
            + self.entities.bytes()
            + self.probe.volume.get().map_or(0, ProbeVolume::bytes)
            + self.exposure.bytes()
            + self.sky.bytes()
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
        self.binds.clear();
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
