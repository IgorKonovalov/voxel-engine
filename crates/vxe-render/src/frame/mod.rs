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
    world_layout: wgpu::BindGroupLayout,
    hit: Marcher,
    light_layout: wgpu::BindGroupLayout,
    /// The light pass alone, built on first use (`light_layout` in group 2).
    light: std::sync::OnceLock<wgpu::ComputePipeline>,
    temporal_layout: wgpu::BindGroupLayout,
    /// The store (minimal until the first bake or live queue, [`Lighting::bake_pool_bytes`]) and
    /// the bake's pipeline, built on the first bake.
    store: crate::bake::Store,
    bake: Option<crate::bake::BakePipeline>,
    /// The edit boxes since the last dig re-bake ([`Frame::rebake_digs`]), every one, while the
    /// store holds a piece.
    dig_boxes: Vec<[[i32; 3]; 2]>,
    /// The live queue ([`Frame::start_bake_queue`]), when the store fills a frame at a time.
    bake_queue: Option<crate::bake::BakeQueue>,
    /// The live queue's world group and the buffers it binds ([`Frame::queue_world_bind`]).
    queue_world_bind: Option<(Vec<wgpu::Buffer>, wgpu::BindGroup)>,
    /// The temporal pass, its `COUNT_STORE` off: it writes no counter.
    temporal: wgpu::ComputePipeline,
    /// The temporal pass with `COUNT_STORE` on, built on the first encode that counts
    /// ([`Frame::set_count_store`]).
    temporal_counting: std::sync::OnceLock<wgpu::ComputePipeline>,
    /// Whether the temporal pass counts the store's reads ([`Frame::set_count_store`]).
    count_store: bool,
    /// The store's five read counters (`counters` in `temporal.wgsl`): missing, brick reads,
    /// coarse reads, fallback, still black.
    store_counters: wgpu::Buffer,
    gradient_layout: wgpu::BindGroupLayout,
    gradient: wgpu::ComputePipeline,
    variance_layout: wgpu::BindGroupLayout,
    variance: wgpu::ComputePipeline,
    atrous_layout: wgpu::BindGroupLayout,
    atrous: wgpu::ComputePipeline,
    /// The a-trous iterations' strides, one uniform each.
    steps: [wgpu::Buffer; ATROUS_ITERATIONS],
    composite_layout: wgpu::BindGroupLayout,
    composite: wgpu::ComputePipeline,
    params: wgpu::Buffer,
    edits: wgpu::Buffer,
    targets: Targets,
    lod: LodParams,
    look: LookParams,
    /// Whether the field look takes the split walk ([`FLAG_FIELD_SPLIT`]); on by default.
    field_split: bool,
    lighting: Lighting,
    /// The per-material emission scales, 1.0 unless set: an `f32` per material id for the
    /// light the frame's pixels sample, then [`EMISSION_ROWS`] more for the light the probe
    /// rays sample.
    emission_scale: wgpu::Buffer,
    emission_cpu: Vec<f32>,
    /// The blue-noise texture, uploaded by the first `encode`.
    blue_noise: wgpu::Texture,
    blue_noise_written: std::sync::atomic::AtomicBool,
    /// The frame index the random numbers are seeded with.
    frame: u32,
    /// The frame index of the last emission-scale change.
    emission_changed_at: Option<u32>,
    /// Whether the last encode ran the probe update (the lamp channel).
    probe_channel_ran: std::sync::atomic::AtomicBool,
    /// The probe trace, blend and gather dispatches the last encode recorded.
    probe_dispatches: std::sync::atomic::AtomicU32,
    /// The copy of the double-buffered targets the next `encode` writes.
    cur: usize,
    resolution: LightingResolution,
    downsample_layout: wgpu::BindGroupLayout,
    downsample: wgpu::ComputePipeline,
    upscale_layout: wgpu::BindGroupLayout,
    upscale: wgpu::ComputePipeline,
    sharpen_layout: wgpu::BindGroupLayout,
    sharpen: wgpu::ComputePipeline,
    /// The sharpening pass's `con` (`Sharpen` in `sharpen.wgsl`).
    sharpen_params: wgpu::Buffer,
    sharpness: f32,
    /// The upscale pass's tuning (`UpTuning` in `upscale.wgsl`).
    upscale_params: wgpu::Buffer,
    upscaler: Upscaler,
    history: std::sync::Mutex<History>,
    /// The probe volume, built on first use (an encode with the lamp channel running,
    /// [`Frame::probes`] or [`Frame::probes_mut`]), at `probe_spacing`; its passes' group-3
    /// layout; the gather's group-2 layout. The device builds the volume.
    probes: std::sync::OnceLock<ProbeVolume>,
    probe_spacing: u32,
    /// The probe volume's side, if fixed ([`Frame::set_probe_axis`]).
    probe_axis_fixed: Option<u32>,
    device: wgpu::Device,
    probe_layout: wgpu::BindGroupLayout,
    gather_layout: wgpu::BindGroupLayout,
    gather: wgpu::ComputePipeline,
    /// The probe update's two dispatches (`probe_update.wgsl`).
    probe_trace: wgpu::ComputePipeline,
    probe_blend: wgpu::ComputePipeline,
    /// Probe rays the last encode dispatched: update-list entries times rays per probe.
    traced_probe_rays: std::sync::atomic::AtomicU64,
    /// Whether the last encode's probe update blended inside a light-change window.
    probe_light_window: std::sync::atomic::AtomicBool,
    /// Whether the last encode with history set [`LIGHTING_LAMP_CHANGED`].
    lamp_changed: std::sync::atomic::AtomicBool,
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
    /// Blocking reads of the store's counters ([`Frame::read_store_stats`]) so far.
    store_blocking_reads: std::sync::atomic::AtomicU64,
    /// The store's counters, copied out by [`Frame::copy_store_stats`].
    store_ring: ReadbackRing,
    /// The views' rays are jittered by [`jitter_at`] of the frame index.
    jitter: bool,
    /// The upscaler the frame runs; with FSR 2, its context and the composite's variant
    /// that stores linear radiance (`HDR_TARGET` in composite.wgsl), built with it.
    kind: UpscalerKind,
    fsr: Option<Fsr2>,
    composite_hdr: Option<(wgpu::BindGroupLayout, wgpu::ComputePipeline)>,
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
        let storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let world_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("world"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                storage(1),
                storage(2),
                storage(3),
                storage(4),
                storage(7),
                storage(8),
                storage(9),
                storage(10),
                storage(11),
                sampled_texture(12, FLOAT),
                wgpu::BindGroupLayoutEntry {
                    binding: 13,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let hit = Marcher::new(device, &world_layout);
        let f16 = Io::Store(wgpu::TextureFormat::Rgba16Float);
        let pass = |label: &str, source: &str, entry: &str, io: &[Io]| {
            let layout = io_layout(device, label, io);
            let pipeline = compute_pipeline(
                device,
                label,
                format!("{MARCH_WGSL}\n{source}"),
                &[Some(&world_layout), None, Some(&layout)],
                entry,
            );
            (layout, pipeline)
        };
        // The light pass's pipeline (`shade`) is built on the first encode that runs it.
        let light_layout = io_layout(device, "light", &[Io::Float, Io::Uint, f16]);
        let targets = Targets::new(
            device,
            width,
            height,
            LightingResolution::default().scale(),
            (internal != output).then_some(output),
            true,
        );
        let (temporal_layout, temporal) = pass(
            "temporal",
            TEMPORAL_WGSL,
            "temporal",
            &[
                Io::Float,
                Io::Uint,
                Io::Float,
                Io::Uint,
                Io::Float,
                Io::Float,
                Io::Float,
                f16,
                f16,
                Io::Uniform,
                Io::Float,
                Io::ReadStorage,
                Io::Storage,
            ],
        );
        let store_counters = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("store counters"),
            size: STORE_STATS_BYTES,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let gradient_layout = io_layout(device, "gradient", &[Io::Float, Io::Uint, Io::Float, f16]);
        let gradient = compute_pipeline(
            device,
            "gradient",
            format!("{MARCH_WGSL}\n{LIGHT_WGSL}\n{GRADIENT_WGSL}"),
            &[Some(&world_layout), None, None, Some(&gradient_layout)],
            "gradient",
        );
        let downsample_layout = io_layout(
            device,
            "downsample",
            &[
                Io::Float,
                Io::Uint,
                Io::Store(wgpu::TextureFormat::Rgba32Float),
                Io::Store(wgpu::TextureFormat::Rgba16Uint),
            ],
        );
        let downsample = compute_pipeline(
            device,
            "downsample",
            format!("{MARCH_WGSL}\n{COMPOSITE_WGSL}"),
            &[Some(&world_layout), None, None, Some(&downsample_layout)],
            "downsample",
        );
        let (variance_layout, variance) = pass(
            "variance",
            ATROUS_WGSL,
            "variance",
            &[Io::Float, Io::Uint, Io::Float, Io::Float, f16],
        );
        let atrous_layout = io_layout_at(
            device,
            "atrous",
            &[
                (0, Io::Float),
                (1, Io::Uint),
                (2, Io::Float),
                (5, f16),
                (6, Io::Uniform),
                (7, Io::Float),
                (8, Io::Float),
            ],
        );
        let atrous = compute_pipeline(
            device,
            "atrous",
            format!("{MARCH_WGSL}\n{ATROUS_WGSL}"),
            &[Some(&world_layout), None, Some(&atrous_layout)],
            "atrous",
        );
        let steps = [0usize, 1, 2, 3].map(|i| {
            wgpu::util::DeviceExt::create_buffer_init(
                device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("atrous step"),
                    contents: bytemuck::cast_slice(&[
                        1u32 << i,
                        u32::from(i + 1 == ATROUS_ITERATIONS),
                        0,
                        0,
                    ]),
                    usage: wgpu::BufferUsages::UNIFORM,
                },
            )
        });
        let (upscale_layout, upscale) = pass(
            "upscale",
            UPSCALE_WGSL,
            "upscale",
            &[
                Io::Float,
                Io::Uint,
                Io::Float,
                Io::Float,
                Io::Uint,
                Io::Float,
                f16,
                Io::Uniform,
                Io::Uniform,
            ],
        );
        let sharpen_layout = io_layout(
            device,
            "sharpen",
            &[
                Io::Float,
                Io::Store(wgpu::TextureFormat::Rgba8Unorm),
                Io::Uniform,
            ],
        );
        let sharpen = compute_pipeline(
            device,
            "sharpen",
            format!("{MARCH_WGSL}\n{SHARPEN_WGSL}"),
            &[Some(&world_layout), None, None, Some(&sharpen_layout)],
            "sharpen_main",
        );
        let (composite_layout, composite) = pass(
            "composite",
            COMPOSITE_WGSL,
            "composite",
            &[
                Io::Float,
                Io::Uint,
                Io::Float,
                Io::Float,
                Io::Store(wgpu::TextureFormat::Rgba8Unorm),
                Io::Float,
                Io::Uint,
                Io::Float,
                Io::Storage,
                Io::Uint,
            ],
        );
        let probe_storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let probe_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("probes"),
            entries: &[
                probe_storage(0),
                probe_storage(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let gather_layout = io_layout(device, "gather", &[Io::Float, Io::Uint, f16]);
        let gather = compute_pipeline(
            device,
            "gather",
            format!("{MARCH_WGSL}\n{PROBE_GATHER_WGSL}"),
            &[
                Some(&world_layout),
                None,
                Some(&gather_layout),
                Some(&probe_layout),
            ],
            "gather",
        );
        let update_source =
            format!("{MARCH_WGSL}\n{LIGHT_WGSL}\n{PROBE_GATHER_WGSL}\n{PROBE_UPDATE_WGSL}");
        let probe_pass = |entry: &str| {
            compute_pipeline(
                device,
                entry,
                update_source.clone(),
                &[Some(&world_layout), None, None, Some(&probe_layout)],
                entry,
            )
        };
        let probe_trace = probe_pass("probe_trace");
        let probe_blend = probe_pass("probe_blend");
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame params"),
            size: size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let edits = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("edit boxes"),
            size: size_of::<EditsUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            world_layout,
            hit,
            light_layout,
            light: std::sync::OnceLock::new(),
            temporal_layout,
            store: crate::bake::Store::new(device, crate::bake::MINIMAL_POOL_BYTES),
            bake: None,
            dig_boxes: Vec::new(),
            bake_queue: None,
            queue_world_bind: None,
            temporal,
            temporal_counting: std::sync::OnceLock::new(),
            count_store: false,
            store_counters,
            gradient_layout,
            gradient,
            variance_layout,
            variance,
            atrous_layout,
            atrous,
            steps,
            composite_layout,
            composite,
            params,
            edits,
            targets,
            resolution: LightingResolution::default(),
            lod: LodParams::default(),
            look: LookParams::default(),
            field_split: true,
            lighting: Lighting::default(),
            emission_scale: wgpu::util::DeviceExt::create_buffer_init(
                device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("emission scale"),
                    contents: bytemuck::cast_slice(&vec![1.0f32; 2 * EMISSION_ROWS]),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                },
            ),
            emission_cpu: vec![1.0; 2 * EMISSION_ROWS],
            blue_noise: device.create_texture(&wgpu::TextureDescriptor {
                label: Some("blue noise"),
                size: wgpu::Extent3d {
                    width: 128,
                    height: 128,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            }),
            blue_noise_written: std::sync::atomic::AtomicBool::new(false),
            frame: 0,
            emission_changed_at: None,
            probe_channel_ran: std::sync::atomic::AtomicBool::new(false),
            probe_dispatches: std::sync::atomic::AtomicU32::new(0),
            cur: 0,
            downsample_layout,
            downsample,
            upscale_layout,
            upscale,
            sharpen_layout,
            sharpen,
            sharpen_params: wgpu::util::DeviceExt::create_buffer_init(
                device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("sharpen"),
                    contents: bytemuck::cast_slice(&[0.0f32; 4]),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                },
            ),
            sharpness: 0.0,
            upscale_params: wgpu::util::DeviceExt::create_buffer_init(
                device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("upscale tuning"),
                    contents: bytemuck::bytes_of(
                        &Upscaler::default().uniform(1.0, Lamp::default()),
                    ),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                },
            ),
            upscaler: Upscaler::default(),
            history: std::sync::Mutex::new(History::default()),
            output,
            jitter: false,
            kind: UpscalerKind::Off,
            fsr: None,
            composite_hdr: None,
            composite_capture: None,
            probes: std::sync::OnceLock::new(),
            probe_spacing: DEFAULT_PROBE_SPACING,
            probe_axis_fixed: None,
            device: device.clone(),
            probe_layout,
            gather_layout,
            gather,
            probe_trace,
            probe_blend,
            traced_probe_rays: std::sync::atomic::AtomicU64::new(0),
            probe_light_window: std::sync::atomic::AtomicBool::new(false),
            lamp_changed: std::sync::atomic::AtomicBool::new(false),
            exposure: ExposurePass::new(device),
            metering: Metering::default(),
            dt: 1.0 / 60.0,
            reset_exposure: std::sync::atomic::AtomicBool::new(true),
            store_blocking_reads: std::sync::atomic::AtomicU64::new(0),
            store_ring: ReadbackRing::new(device, "store stats readback", STORE_STATS_BYTES),
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
        self.lamp_changed.load(std::sync::atomic::Ordering::Relaxed)
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
        self.frame
    }

    pub fn set_frame_index(&mut self, frame: u32) {
        self.frame = frame;
    }

    fn history(&mut self) -> &mut History {
        self.history
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Ends the frame last encoded: the next frame index, its hit buffer and history become
    /// the previous frame's, and the double-buffered targets swap. Call once per frame, after
    /// [`Frame::encode`].
    pub fn advance(&mut self) {
        self.frame = self.frame.wrapping_add(1);
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
        if self.store.piece().is_some() && self.bake_queue.is_none() {
            self.dig_boxes.push([min, max]);
        }
        if let Some(q) = self.bake_queue.as_mut() {
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
            if self.emission_cpu[row] != scale {
                self.emission_cpu[row] = scale;
                if row == i {
                    self.emission_changed_at = Some(self.frame);
                }
                queue.write_buffer(
                    &self.emission_scale,
                    row as u64 * 4,
                    bytemuck::bytes_of(&scale),
                );
            }
        }
    }

    /// Material `material`'s emission scale in the pixels' light.
    pub fn emission_scale(&self, material: u16) -> f32 {
        self.emission_cpu[usize::from(material)]
    }

    /// Material `material`'s emission scale in the probe rays' light.
    pub fn probe_emission_scale(&self, material: u16) -> f32 {
        self.emission_cpu[EMISSION_ROWS + usize::from(material)]
    }

    /// The emission-scale buffer: an `f32` per material id for the pixels' light, then one
    /// per material id for the probe rays' (see [`Frame::set_emission_scales`]).
    pub fn emission_scales(&self) -> &wgpu::Buffer {
        &self.emission_scale
    }

    /// Sets how LOD-only chunks pick their mip level.
    pub fn set_lod(&mut self, lod: LodParams) {
        self.lod = lod;
    }

    pub fn lod(&self) -> LodParams {
        self.lod
    }

    /// Sets the surface look: normal mode and view. A change of view drops the
    /// upscaler's history (not the lighting's).
    pub fn set_look(&mut self, look: LookParams) {
        if look.view != self.look.view {
            self.history().view_changed = true;
        }
        self.look = look;
    }

    pub fn look(&self) -> LookParams {
        self.look
    }

    /// Whether the field look walks split ([`FLAG_FIELD_SPLIT`], the default): the cube
    /// walk's pipeline carries each ray to the first brick the field can hit in, and the field
    /// walk goes on from there. Off, the field walk carries every ray from its origin. Either
    /// gives the same hit records; the split walks only when the world's table holds a crisp
    /// row ([`GpuWorld::crisp_rows`]), outside the cracks and steps views.
    pub fn set_field_split(&mut self, on: bool) {
        self.field_split = on;
    }

    /// Sets the lighting knobs; a change of [`Lighting::bake_pool_bytes`] rebuilds an allocated
    /// store empty.
    pub fn set_lighting(&mut self, lighting: Lighting) {
        self.lighting = lighting;
        self.fit_store(false);
        self.rebuild_probes();
    }

    pub fn lighting(&self) -> Lighting {
        self.lighting
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
            + self.probes.get().map_or(0, ProbeVolume::bytes)
            + self.exposure.bytes()
            + self.fsr.as_ref().map_or(0, Fsr2::bytes)
            + if self.store.pool().size() > crate::bake::MINIMAL_POOL_BYTES {
                self.store.pool().size()
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
        let fsr = self.fsr.is_some();
        let upscaled = (fsr || self.jitter || internal != output).then_some(output);
        self.targets = Targets::new(
            device,
            internal[0],
            internal[1],
            self.resolution.scale(),
            upscaled,
            !fsr,
        );
        if let Some(f) = self.fsr.as_mut() {
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
