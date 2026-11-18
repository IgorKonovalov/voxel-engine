//! The sky's GPU side (ADR-0026): its uniform, its LUTs and the passes that build them, and
//! [`Frame::set_sky`]. With no sky ever set the LUTs are 1 x 1 placeholders, no LUT pipeline
//! is built and no LUT pass runs.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use super::*;
use crate::sky::{Sky, SkyKey, SkyUniform};

pub(crate) const SKY_ATMOSPHERE_WGSL: &str = include_str!("../shaders/sky_atmosphere.wgsl");
pub(crate) const SKY_WGSL: &str = include_str!("../shaders/sky.wgsl");
const SKY_TRANSMITTANCE_WGSL: &str = include_str!("../shaders/sky_transmittance.wgsl");
const SKY_MULTISCATTER_WGSL: &str = include_str!("../shaders/sky_multiscatter.wgsl");
const SKY_VIEW_WGSL: &str = include_str!("../shaders/sky_view.wgsl");
const SKY_IRRADIANCE_WGSL: &str = include_str!("../shaders/sky_irradiance.wgsl");

/// Bytes of `E_sky` (ADR-0070): six `vec4<f32>`, the sky's irradiance about each axis.
pub(crate) const SKY_E_BYTES: u64 = 6 * 16;

/// The LUTs' sizes, as `sky_atmosphere.wgsl` declares them.
pub(crate) const TRANSMITTANCE_SIZE: [u32; 2] = [256, 64];
pub(crate) const MULTISCATTER_SIZE: [u32; 2] = [32, 32];
pub(crate) const SKY_VIEW_SIZE: [u32; 2] = [192, 108];

const F16: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// The LUT passes and the irradiance reduction after them: label, modules after the prelude,
/// entry point.
pub(crate) const SKY_PASSES: [(&str, &[&str], &str); 4] = [
    (
        "sky transmittance",
        &[SKY_ATMOSPHERE_WGSL, SKY_TRANSMITTANCE_WGSL],
        "sky_transmittance",
    ),
    (
        "sky multiscatter",
        &[SKY_ATMOSPHERE_WGSL, SKY_MULTISCATTER_WGSL],
        "sky_multiscatter",
    ),
    (
        "sky view",
        &[SKY_ATMOSPHERE_WGSL, SKY_VIEW_WGSL],
        "sky_view",
    ),
    (
        "sky irradiance",
        &[SKY_ATMOSPHERE_WGSL, SKY_IRRADIANCE_WGSL],
        "sky_irradiance",
    ),
];

/// The sky-view LUT's format: a night sky's radiance is 1 / 400 000 of a day's, under
/// rgba16float's normal range at the day's scale.
const F32: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;

fn lut(device: &wgpu::Device, label: &str, size: [u32; 2]) -> wgpu::Texture {
    lut_of(device, label, size, F16)
}

fn lut_of(
    device: &wgpu::Device,
    label: &str,
    size: [u32; 2],
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// The LUTs at their sizes, allocated at the first [`Frame::set_sky`] with a sky.
pub(super) struct SkyLuts {
    pub(super) transmittance: wgpu::Texture,
    pub(super) multiscatter: wgpu::Texture,
    pub(super) view: wgpu::Texture,
}

impl SkyLuts {
    fn bytes(&self) -> u64 {
        [&self.transmittance, &self.multiscatter, &self.view]
            .iter()
            .map(|t| {
                u64::from(t.format().block_copy_size(None).unwrap_or(8))
                    * u64::from(t.width())
                    * u64::from(t.height())
            })
            .sum()
    }
}

/// The LUT passes' layouts and pipelines.
struct SkyPipes {
    layouts: Vec<wgpu::BindGroupLayout>,
    pipes: Vec<wgpu::ComputePipeline>,
}

/// The frame's sky: what was set, the uniform, the LUTs and their passes.
pub(super) struct SkyState {
    pub(super) sky: Option<Sky>,
    pub(super) uniform: wgpu::Buffer,
    /// `E_sky` ([`SKY_E_BYTES`]), written by the irradiance pass after the sky view's each
    /// frame a sky is set; the temporal pass reads it (`sky_e`), the exposure pass its `+z`
    /// (the anchored ceiling, with the key light's horizontal irradiance in its w). Zero
    /// before the first sky.
    pub(super) irradiance: wgpu::Buffer,
    pub(super) luts: Option<SkyLuts>,
    placeholder: wgpu::Texture,
    /// The atmosphere changed since the transmittance and multiple-scattering LUTs were built.
    dirty: AtomicBool,
    /// The sky-view pass adds the multiple-scattering term ([`Frame::set_sky_multiscatter`]).
    multiscatter: bool,
    pipes: OnceLock<SkyPipes>,
    /// The change rule's keys: the last encode with history's, and the one before it, which
    /// the uniform's `prev_key_*` and `prev_sky_scale` hold.
    keys: Mutex<(Option<SkyKey>, Option<SkyKey>)>,
    /// The last encode with history raised [`LIGHTING_SKY_CHANGED`].
    changed: AtomicBool,
}

impl SkyState {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        Self {
            sky: None,
            uniform: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("sky uniform"),
                size: size_of::<SkyUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            irradiance: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("sky irradiance"),
                size: SKY_E_BYTES,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            luts: None,
            placeholder: lut(device, "sky lut placeholder", [1, 1]),
            dirty: AtomicBool::new(false),
            multiscatter: true,
            pipes: OnceLock::new(),
            keys: Mutex::new((None, None)),
            changed: AtomicBool::new(false),
        }
    }

    /// The bytes of the LUTs and the uniform, 0 while no sky was ever set.
    pub(super) fn bytes(&self) -> u64 {
        self.luts.as_ref().map_or(0, |l| {
            l.bytes() + size_of::<SkyUniform>() as u64 + SKY_E_BYTES
        })
    }

    /// The textures group 0 binds at 17 and 18: the LUTs, or the placeholder.
    pub(super) fn bound(&self) -> [&wgpu::Texture; 2] {
        match &self.luts {
            Some(l) => [&l.transmittance, &l.view],
            None => [&self.placeholder, &self.placeholder],
        }
    }

    fn pipes(&self, device: &wgpu::Device) -> &SkyPipes {
        self.pipes.get_or_init(|| {
            let ios: [&[Io]; 4] = [
                &[Io::Uniform, Io::Store(F16)],
                &[Io::Uniform, Io::Float, Io::Store(F16)],
                &[Io::Uniform, Io::Float, Io::Float, Io::Store(F32)],
                &[Io::Uniform, Io::Float, Io::Storage, Io::Float],
            ];
            let layouts: Vec<_> = SKY_PASSES
                .iter()
                .zip(ios)
                .map(|((label, ..), io)| io_layout(device, label, io))
                .collect();
            let pipes = SKY_PASSES
                .iter()
                .zip(&layouts)
                .map(|((label, modules, entry), layout)| {
                    compute_pipeline(device, label, compose(modules), &[Some(layout)], entry)
                })
                .collect();
            SkyPipes { layouts, pipes }
        })
    }

    /// The change rule at an encode with history: whether the sky jumped since the last one
    /// (`SkyKey::jumped_from`), the keys moved on a frame.
    pub(super) fn step(&self) -> bool {
        let now = SkyKey::of(self.sky.as_ref());
        let mut k = self
            .keys
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let jumped = k.0.is_some_and(|old| now.jumped_from(&old));
        *k = (Some(now), k.0);
        self.changed.store(jumped, Ordering::Relaxed);
        jumped
    }

    /// Whether the sky adds to the direct part at `sky_rays` sky rays: a sky is set and
    /// casts sky rays, or its key light is up.
    pub(super) fn lights_direct(&self, sky_rays: u32) -> bool {
        self.sky
            .as_ref()
            .is_some_and(|s| sky_rays > 0 || s.key_light().is_some())
    }

    /// Writes the uniform for a camera at world z `camera_z`, with the sky rays' count from
    /// `state`, and returns the LUT
    /// passes' bind groups, `None` with no sky.
    pub(super) fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        binds: &BindCache,
        camera_z: f32,
        state: &FrameState,
    ) -> Option<[wgpu::BindGroup; 4]> {
        let prev = self
            .keys
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .1;
        let now = SkyKey::of(self.sky.as_ref());
        let u = SkyUniform::of(self.sky.as_ref(), camera_z, self.multiscatter).with_keys(
            &now,
            prev.as_ref(),
            state.lighting.sky_rays.min(MAX_SKY_RAYS),
            state.lighting.sky_history.clamp(1, MAX_SKY_HISTORY),
        );
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
        let (_, luts) = (self.sky.as_ref()?, self.luts.as_ref()?);
        let p = self.pipes(device);
        Some([
            binds.io(
                device,
                &p.layouts[0],
                &[Res::Buf(&self.uniform), Res::Tex(&luts.transmittance)],
            ),
            binds.io(
                device,
                &p.layouts[1],
                &[
                    Res::Buf(&self.uniform),
                    Res::Tex(&luts.transmittance),
                    Res::Tex(&luts.multiscatter),
                ],
            ),
            binds.io(
                device,
                &p.layouts[2],
                &[
                    Res::Buf(&self.uniform),
                    Res::Tex(&luts.transmittance),
                    Res::Tex(&luts.multiscatter),
                    Res::Tex(&luts.view),
                ],
            ),
            binds.io(
                device,
                &p.layouts[3],
                &[
                    Res::Buf(&self.uniform),
                    Res::Tex(&luts.view),
                    Res::Buf(&self.irradiance),
                    Res::Tex(&luts.transmittance),
                ],
            ),
        ])
    }

    /// The LUT passes' dispatches over `groups` ([`SkyState::prepare`]): the transmittance and
    /// multiple-scattering LUTs' when the atmosphere changed (the flag then cleared), the sky
    /// view's and the irradiance reduction's (one workgroup a direction) every frame; empty
    /// with no sky.
    pub(super) fn dispatches<'a>(
        &'a self,
        device: &wgpu::Device,
        groups: Option<&'a [wgpu::BindGroup; 4]>,
    ) -> [Dispatches<'a>; 4] {
        let mut out = [Dispatches::default(); 4];
        let Some(groups) = groups else {
            return out;
        };
        let p = self.pipes(device);
        let sizes = [
            TRANSMITTANCE_SIZE,
            MULTISCATTER_SIZE,
            SKY_VIEW_SIZE,
            [6 * WORKGROUP, WORKGROUP],
        ];
        let rebuild = self.dirty.swap(false, Ordering::Relaxed);
        for (i, slot) in out.iter_mut().enumerate() {
            if i < 2 && !rebuild {
                continue;
            }
            slot.push(Dispatch {
                pipeline: &p.pipes[i],
                groups: [Some(&groups[i]), None, None, None],
                workgroups: sizes[i].map(|n| n.div_ceil(WORKGROUP)),
                indirect: None,
            });
        }
        out
    }
}

impl Frame {
    /// Sets the sky the frame draws on a miss and lights with (ADR-0026), or none. With `None`
    /// (the default) the frame renders exactly as without a sky. The first sky allocates the
    /// LUTs; an atmosphere that differs from the last rebuilds the transmittance LUT on the
    /// next encode.
    pub fn set_sky(&mut self, sky: Option<Sky>) {
        let s = &mut self.sky;
        if let Some(new) = &sky {
            if s.luts.is_none() {
                s.luts = Some(SkyLuts {
                    transmittance: lut(&self.device, "sky transmittance", TRANSMITTANCE_SIZE),
                    multiscatter: lut(&self.device, "sky multiscatter", MULTISCATTER_SIZE),
                    view: lut_of(&self.device, "sky view", SKY_VIEW_SIZE, F32),
                });
                s.dirty.store(true, Ordering::Relaxed);
            }
            if s.sky.is_none_or(|old| old.atmosphere != new.atmosphere) {
                s.dirty.store(true, Ordering::Relaxed);
            }
        }
        s.sky = sky;
    }

    /// Whether the sky-view LUT adds the multiple-scattering term (on by default); a test
    /// hook that reads single scattering alone.
    pub fn set_sky_multiscatter(&mut self, on: bool) {
        self.sky.multiscatter = on;
    }

    /// Whether the last encode with history raised the sky-change bit: the sky jumped (its
    /// key light turned by more than its angular radius, switched body or changed
    /// illuminance, its scale changed, or it was set or cleared). A drift raises nothing.
    pub fn sky_changed(&self) -> bool {
        self.sky.changed.load(Ordering::Relaxed)
    }

    /// `E_sky` (ADR-0070): six `vec4<f32>`, the sky's cosine-weighted irradiance about each
    /// axis in face order (2 k toward -k, 2 k + 1 toward +k) in xyz, written by the last encode
    /// with a sky set; zero before the first. `+z`'s w is the key light's irradiance on a
    /// horizontal face at the base altitude, in luminance (the exposure's anchored ceiling).
    pub fn sky_irradiance(&self) -> &wgpu::Buffer {
        &self.sky.irradiance
    }

    /// The sky [`Frame::set_sky`] last set.
    pub fn sky(&self) -> Option<&Sky> {
        self.sky.sky.as_ref()
    }

    /// The transmittance LUT (rgba16float, 256 x 64, by `transmittance_uv` in
    /// `sky_atmosphere.wgsl`), the sky-view LUT (rgba32float, 192 x 108, radiance times
    /// the sky's view scale) and the multiple-scattering LUT (rgba16float, 32 x 32), `None`
    /// before the first sky.
    pub fn sky_luts(&self) -> Option<[&wgpu::Texture; 3]> {
        self.sky
            .luts
            .as_ref()
            .map(|l| [&l.transmittance, &l.view, &l.multiscatter])
    }
}
