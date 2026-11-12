//! FSR 2's resources: its textures and buffers at the render and display sizes.

use super::*;

/// FSR 2's resources at one pair of sizes; replaced with the frame's targets.
pub(super) struct Resources {
    pub(super) render: [u32; 2],
    pub(super) display: [u32; 2],
    /// The composite's linear colour at the render size (rgba32float: the tone pass of the
    /// composite's own arithmetic reproduces its rgba8 bytes only from f32 radiance).
    pub(super) colour: wgpu::Texture,
    /// Our input pass's depth (r32float), motion vectors (rg32float) and masks (rgba8unorm:
    /// reactive, transparency and composition, the edit flag).
    pub(super) depth: wgpu::Texture,
    pub(super) motion: wgpu::Texture,
    pub(super) reactive: wgpu::Texture,
    /// The luminance pyramid's shading-change mip and mip 5 (r32float), and its exposure
    /// (rg32float 1 x 1), this frame's and the last.
    pub(super) luma_mip: wgpu::Texture,
    pub(super) luma_mip5: wgpu::Texture,
    pub(super) auto_exposure: [wgpu::Texture; 2],
    /// The reconstructed previous depth, an atomic `u32` per render pixel.
    pub(super) reconstructed: wgpu::Buffer,
    /// The reconstruct pass's dilated depth (r32float), dilated motion (rg32float, this
    /// frame's and the last) and lock input luma (r32float).
    pub(super) dilated_depth: wgpu::Texture,
    pub(super) dilated_motion: [wgpu::Texture; 2],
    pub(super) lock_luma: wgpu::Texture,
    /// The depth clip's prepared input colour (rgba16float: YCoCg, depth clip) and dilated
    /// reactive masks (rgba8unorm).
    pub(super) prepared: wgpu::Texture,
    pub(super) dilated_reactive: wgpu::Texture,
    /// The exposure (rg32float 1 x 1, x).
    pub(super) exposure: wgpu::Texture,
    /// The internal upscaled colour (rgba16float), lock status (rg32float) and luma history
    /// (rgba8unorm) at the display size, this frame's and the last.
    pub(super) upscaled: [wgpu::Texture; 2],
    pub(super) lock_status: [wgpu::Texture; 2],
    pub(super) luma_history: [wgpu::Texture; 2],
    /// The new locks, an `f32` per display pixel.
    pub(super) new_locks: wgpu::Buffer,
    /// The accumulate pass's history weight (r32float), at the display size with
    /// [`Fsr2::set_debug`], else 1 x 1.
    pub(super) weight: wgpu::Texture,
}

pub(super) fn texture(
    device: &wgpu::Device,
    label: &str,
    format: wgpu::TextureFormat,
    [width, height]: [u32; 2],
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

/// Bytes per texel of the formats the port uses.
pub(super) fn texel_bytes(format: wgpu::TextureFormat) -> u64 {
    use wgpu::TextureFormat as F;
    match format {
        F::Rgba8Unorm | F::R32Float => 4,
        F::Rg32Float | F::Rgba16Float => 8,
        F::Rgba32Float => 16,
        f => unreachable!("no FSR target is {f:?}"),
    }
}

pub(super) fn texture_bytes(t: &wgpu::Texture) -> u64 {
    u64::from(t.width()) * u64::from(t.height()) * texel_bytes(t.format())
}

/// The size of mip `level` of a pyramid whose mip 0 is half of `render` (AMD's scene
/// luminance), at least 1.
pub(super) fn mip_size(render: [u32; 2], level: u32) -> [u32; 2] {
    render.map(|n| ((n / 2) >> level).max(1))
}

impl Resources {
    pub(super) fn new(
        device: &wgpu::Device,
        render: [u32; 2],
        display: [u32; 2],
        debug: bool,
    ) -> Self {
        use wgpu::TextureFormat::{R32Float, Rg32Float, Rgba8Unorm, Rgba16Float, Rgba32Float};
        let pair = |label, format, size| [0, 1].map(|_| texture(device, label, format, size));
        let buffer = |label, size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let px = |s: [u32; 2]| u64::from(s[0]) * u64::from(s[1]);
        Self {
            render,
            display,
            colour: texture(device, "fsr2 colour", Rgba32Float, render),
            depth: texture(device, "fsr2 depth", R32Float, render),
            motion: texture(device, "fsr2 motion", Rg32Float, render),
            reactive: texture(device, "fsr2 reactive", Rgba8Unorm, render),
            luma_mip: texture(
                device,
                "fsr2 shading-change mip",
                R32Float,
                mip_size(render, SHADING_CHANGE_MIP as u32),
            ),
            luma_mip5: texture(device, "fsr2 luma mip 5", R32Float, mip_size(render, 5)),
            auto_exposure: pair("fsr2 auto exposure", Rg32Float, [1, 1]),
            reconstructed: buffer("fsr2 reconstructed depth", 4 * px(render)),
            dilated_depth: texture(device, "fsr2 dilated depth", R32Float, render),
            dilated_motion: pair("fsr2 dilated motion", Rg32Float, render),
            lock_luma: texture(device, "fsr2 lock luma", R32Float, render),
            prepared: texture(device, "fsr2 prepared colour", Rgba16Float, render),
            dilated_reactive: texture(device, "fsr2 dilated reactive", Rgba8Unorm, render),
            exposure: texture(device, "fsr2 exposure", Rg32Float, [1, 1]),
            upscaled: pair("fsr2 upscaled", Rgba16Float, display),
            lock_status: pair("fsr2 lock status", Rg32Float, display),
            luma_history: pair("fsr2 luma history", Rgba8Unorm, display),
            new_locks: buffer("fsr2 new locks", 4 * px(display)),
            weight: texture(
                device,
                "fsr2 history weight",
                R32Float,
                if debug { display } else { [1, 1] },
            ),
        }
    }

    pub(super) fn bytes(&self) -> u64 {
        let singles = [
            &self.colour,
            &self.depth,
            &self.motion,
            &self.reactive,
            &self.luma_mip,
            &self.luma_mip5,
            &self.dilated_depth,
            &self.lock_luma,
            &self.prepared,
            &self.dilated_reactive,
            &self.exposure,
            &self.weight,
        ];
        let pairs = [
            &self.auto_exposure,
            &self.dilated_motion,
            &self.upscaled,
            &self.lock_status,
            &self.luma_history,
        ];
        singles.iter().map(|t| texture_bytes(t)).sum::<u64>()
            + pairs
                .iter()
                .map(|p| texture_bytes(&p[0]) + texture_bytes(&p[1]))
                .sum::<u64>()
            + self.new_locks.size()
            + self.reconstructed.size()
    }
}
