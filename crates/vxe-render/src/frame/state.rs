//! What the frame's uniforms are built from between encodes: the history the temporal pass
//! reads, the light and lamp change keys, the light table, the edit boxes and the blue noise.

use super::*;

/// What the next frame's temporal pass reads of the past.
#[derive(Default)]
pub(super) struct History {
    /// The previous frame's views, and whether its hit buffer and history are valid.
    pub(super) prev_views: [View; MAX_VIEWS],
    pub(super) valid: bool,
    /// The views of the last `encode` with history, and whether one ran since `advance`.
    pub(super) last_views: [View; MAX_VIEWS],
    pub(super) encoded: bool,
    /// The lighting grid's offset of the previous frame and of the last encode.
    pub(super) prev_offset: u32,
    pub(super) last_offset: u32,
    /// The edit boxes collected for the next `encode`, and whether more than `MAX_EDITS`
    /// arrived.
    pub(super) edits: Vec<[[i32; 3]; 2]>,
    pub(super) overflow: bool,
    /// The view mode changed since the last encode: the upscaler's history holds another
    /// view's image.
    pub(super) view_changed: bool,
    /// The lamps of the last encodes with history, newest last, at most [`LAMP_FRAMES`].
    pub(super) lamps: std::collections::VecDeque<Lamp>,
    /// The static light of the last encode with history (`None` before the first).
    pub(super) light_key: Option<LightKey>,
    /// Encoded frames with a probe update left in the open light-change window.
    pub(super) light_window: u32,
}

/// What a frame's static light depends on, as the probe volume's light-change window compares
/// it between encodes: the lamp's switch and its parameters (not `masks` or `skips`, which
/// leave the walk byte-identical, and not its pose, which follows the camera), the shadow walk
/// the lamp's shadow rays take. Exposure and the tone map come after the light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct LightKey {
    pub(super) lamp: bool,
    pub(super) params: LampKey,
    pub(super) shadow_walk: ShadowWalk,
}

/// The lamp's parameters as a frame's [`Params`] carry them: its camera-relative pose
/// ([`Params::lamp_pose`]), its intensity, its cone's cosines and its shadow switch. `masks`
/// and `skips` leave the walk byte-identical and are not part of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct LampKey {
    pub(super) pose: [f32; 3],
    pub(super) intensity: f32,
    pub(super) cos_inner: f32,
    pub(super) cos_outer: f32,
    pub(super) shadows: bool,
}

impl LampKey {
    pub(super) fn of(p: &Params) -> Self {
        Self {
            pose: p.lamp_pose,
            intensity: p.light_intensity,
            cos_inner: p.cos_inner,
            cos_outer: p.cos_outer,
            shadows: p.flags & FLAG_SHADOWS != 0,
        }
    }
}

impl LightKey {
    /// Whether the lamp's light differs between `self` and `other`: its switch, its
    /// parameters or its shadow rays' walk.
    pub(super) fn lamp_differs(&self, other: &Self) -> bool {
        self.lamp != other.lamp
            || self.params != other.params
            || self.shadow_walk != other.shadow_walk
    }
}

/// Entries the candidate table holds at most.
pub(crate) const LIGHT_TABLE_ENTRIES: usize = 1024;

/// The light pass's candidate table as written past the light-sample reservation
/// (`GpuWorld::light_table_base`): a header slot `count, total (f32 bits), 0, 0`, then per
/// entry `start, len`, the cumulative weight (f32 bits) and a pad.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct LightTableUniform {
    pub(crate) count: u32,
    pub(crate) total: f32,
    pub(crate) _pad: [u32; 2],
    pub(crate) chunks: [[u32; 4]; LIGHT_TABLE_ENTRIES],
}

/// The candidate table for a camera at `eye` over `world`: the chunks holding a range within
/// the light radius of the camera's chunk (squared chunk distance), the nearest
/// [`LIGHT_TABLE_ENTRIES`] of them, in chunk-coordinate order, each weighing its power over
/// `max(d^2, 64^2)`, `d` the eye's distance in voxels to the chunk's box.
pub(super) fn light_table(world: &GpuWorld, eye: [f32; 3]) -> LightTableUniform {
    let r = i64::from(world.light_radius());
    let cam = eye.map(|v| (v / 64.0).floor() as i32);
    let mut near: Vec<(i64, ChunkCoord, u32, u32, f32)> = world
        .light_chunks()
        .map(|(c, start, len, power)| {
            let d2 = c.distance2(ChunkCoord(cam));
            (d2, c, start, len, power)
        })
        .filter(|&(d2, ..)| d2 <= r * r)
        .collect();
    near.sort_unstable_by_key(|&(d2, c, ..)| (d2, c.0));
    near.truncate(LIGHT_TABLE_ENTRIES);
    near.sort_unstable_by_key(|&(_, c, ..)| c.0);
    let mut t = LightTableUniform {
        count: near.len() as u32,
        total: 0.0,
        _pad: [0; 2],
        chunks: [[0; 4]; LIGHT_TABLE_ENTRIES],
    };
    let mut cdf = 0.0f32;
    for (i, &(_, c, start, len, power)) in near.iter().enumerate() {
        let d2: f32 = (0..3)
            .map(|k| {
                let lo = c.0[k] as f32 * 64.0;
                let gap = (lo - eye[k]).max(eye[k] - (lo + 64.0)).max(0.0);
                gap * gap
            })
            .sum();
        cdf += power / d2.max(64.0 * 64.0);
        t.chunks[i] = [start, len, cdf.to_bits(), 0];
    }
    t.total = cdf;
    t
}

/// The temporal pass's edit boxes, field for field with `Edits` in `temporal.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct EditsUniform {
    pub(super) count: u32,
    pub(super) _pad: [u32; 3],
    pub(super) boxes: [[i32; 4]; 2 * MAX_EDITS],
}

/// The 128 x 128 blue-noise ranks (`assets/blue_noise_128.r8`, made by
/// `assets/blue_noise_128.py`: void and cluster, Ulichney 1993), one byte per texel, row-major.
pub(super) const BLUE_NOISE: &[u8; 128 * 128] = include_bytes!("../../assets/blue_noise_128.r8");

/// Material ids the emission-scale buffer covers: every `u16`.
pub(super) const EMISSION_ROWS: usize = 1 << 16;

/// What the frame's uniforms are built from: the parameter and edit-box buffers, the a-trous
/// strides, the LOD, the look, the lighting, the emission scales, the blue noise, the frame
/// index and the history the next encode reads.
pub(super) struct FrameState {
    pub(super) params: wgpu::Buffer,
    pub(super) edits: wgpu::Buffer,
    /// The a-trous iterations' strides, one uniform each.
    pub(super) atrous_steps: [wgpu::Buffer; ATROUS_ITERATIONS],
    pub(super) lod: LodParams,
    pub(super) look: LookParams,
    /// Whether the field look takes the split walk ([`FLAG_FIELD_SPLIT`]); on by default.
    pub(super) field_split: bool,
    pub(super) lighting: Lighting,
    /// The per-material emission scales, 1.0 unless set: an `f32` per material id for the
    /// light the frame's pixels sample, then [`EMISSION_ROWS`] more for the light the probe
    /// rays sample.
    pub(super) emission_scale: wgpu::Buffer,
    pub(super) emission_cpu: Vec<f32>,
    /// The blue-noise texture, uploaded by the first `encode`.
    pub(super) blue_noise: wgpu::Texture,
    pub(super) blue_noise_written: std::sync::atomic::AtomicBool,
    /// The frame index the random numbers are seeded with.
    pub(super) frame: u32,
    /// The frame index of the last emission-scale change.
    pub(super) emission_changed_at: Option<u32>,
    pub(super) history: std::sync::Mutex<History>,
    /// Whether the last encode with history set [`LIGHTING_LAMP_CHANGED`].
    pub(super) lamp_changed: std::sync::atomic::AtomicBool,
}

impl FrameState {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let atrous_steps = [0usize, 1, 2, 3].map(|i| {
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
            params,
            edits,
            atrous_steps,
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
            history: std::sync::Mutex::new(History::default()),
            lamp_changed: std::sync::atomic::AtomicBool::new(false),
        }
    }
}
