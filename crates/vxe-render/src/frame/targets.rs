//! The frame's targets at the internal resolution, and the copies kept for the next frame.

use super::*;

/// The texture [`Frame::set_composite_capture`] copies the composite into.
pub(super) fn composite_capture_texture(device: &wgpu::Device, size: [u32; 2]) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("composite capture"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// The frame's targets: the hit buffer, the translucent layers and the output at the internal
/// resolution, and the lighting passes' targets on the lighting grid, the same at full
/// resolution and half of it on each edge at half resolution (with its own hit buffer, sampled
/// from the full one). The hit buffers and the history are kept twice, indexed by
/// `Frame::cur`.
pub(super) struct Targets {
    pub(super) width: u32,
    pub(super) height: u32,
    /// Full-resolution pixels per lighting pixel on an edge: 1 or 2.
    pub(super) scale: u32,
    /// The hit buffer: position + t (rgba32float), and octahedral normal, material and flags
    /// (rgba16uint).
    pub(super) hit_pos: [wgpu::Texture; 2],
    pub(super) hit_info: [wgpu::Texture; 2],
    /// The lighting grid's hit buffer at half resolution.
    pub(super) half_pos: Option<[wgpu::Texture; 2]>,
    pub(super) half_info: Option<[wgpu::Texture; 2]>,
    /// The translucent solid in front of each hit: its reflected light (rgba16float), or the
    /// steps view's counters.
    pub(super) layer: wgpu::Texture,
    /// The translucent solid's transmittance per channel, rgb9e5 in r32uint (ADR-0047).
    pub(super) transmit: wgpu::Texture,
    /// The light pass's irradiance (rgba16float), on the lighting grid.
    pub(super) irradiance: wgpu::Texture,
    /// The temporal pass's accumulated irradiance and moments (rgba16float).
    pub(super) accum: [wgpu::Texture; 2],
    pub(super) moments: [wgpu::Texture; 2],
    /// The filter's ping-pong pair (rgba16float: irradiance rgb, variance a).
    pub(super) filter: [wgpu::Texture; 2],
    /// The gradient pass's strata, one texel per 3 x 3 lighting pixels (rgba16float: the
    /// lamp's lambda, the new and old luminances, 0).
    pub(super) strata: wgpu::Texture,
    /// The frame's display-encoded image (rgba8unorm): the composite's output at the
    /// internal size, or with the upscaler at the output size, the composite's image in its
    /// top-left pixels until the sharpening pass overwrites it.
    pub(super) output: wgpu::Texture,
    /// The probe gather's indirect irradiance (rgba16float), on the lighting grid.
    pub(super) indirect: wgpu::Texture,
    /// The upscaler's history at the output size (rgba16float), this frame's and the last.
    pub(super) history: Option<[wgpu::Texture; 2]>,
    /// The exposure meter's tiles: two `f32` per 8 x 8 workgroup of the internal target.
    pub(super) tiles: wgpu::Buffer,
    /// The split walk's hand-off list (a count word and three of padding, then a `vec4<u32>`
    /// per pixel; `march.wgsl`'s `SplitList`), built by the first encode that splits
    /// ([`Targets::split_list`]); until then the hit pass binds `split_stub`, the header and
    /// one ray (the binding's minimum size), which the unsplit walk never writes. The resume
    /// dispatch's workgroup counts (`SplitArgs`).
    pub(super) split_list: std::sync::OnceLock<wgpu::Buffer>,
    pub(super) split_stub: wgpu::Buffer,
    pub(super) split_args: wgpu::Buffer,
}

impl Targets {
    /// The targets at the internal size `width` x `height`, with the output at `upscaled` when
    /// an upscaler runs, and the history of ADR-0024's upscaler with `own_history`.
    pub(super) fn new(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        scale: u32,
        upscaled: Option<[u32; 2]>,
        own_history: bool,
    ) -> Self {
        let sized = |label, format, width, height| {
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
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let (lw, lh) = (width.div_ceil(scale), height.div_ceil(scale));
        let tex = |label, format| sized(label, format, width, height);
        let lit = |label, format| sized(label, format, lw, lh);
        use wgpu::TextureFormat::{R32Uint, Rgba8Unorm, Rgba16Float, Rgba16Uint, Rgba32Float};
        let half = scale > 1;
        Self {
            width,
            height,
            scale,
            hit_pos: [0, 1].map(|_| tex("hit pos", Rgba32Float)),
            hit_info: [0, 1].map(|_| tex("hit info", Rgba16Uint)),
            half_pos: half.then(|| [0, 1].map(|_| lit("half hit pos", Rgba32Float))),
            half_info: half.then(|| [0, 1].map(|_| lit("half hit info", Rgba16Uint))),
            layer: tex("layer", Rgba16Float),
            transmit: tex("transmittance", R32Uint),
            irradiance: lit("irradiance", Rgba16Float),
            accum: [0, 1].map(|_| lit("accumulated irradiance", Rgba16Float)),
            moments: [0, 1].map(|_| lit("moments", Rgba16Float)),
            filter: [0, 1].map(|_| lit("filter", Rgba16Float)),
            strata: sized(
                "gradient strata",
                Rgba16Float,
                lw.div_ceil(GRADIENT_STRATUM),
                lh.div_ceil(GRADIENT_STRATUM),
            ),
            output: match upscaled {
                Some([w, h]) => sized("frame target", Rgba8Unorm, w, h),
                None => tex("frame target", Rgba8Unorm),
            },
            history: upscaled
                .filter(|_| own_history)
                .map(|[w, h]| [0, 1].map(|_| sized("upscale history", Rgba16Float, w, h))),
            indirect: lit("indirect irradiance", Rgba16Float),
            tiles: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("exposure meter tiles"),
                size: 8 * u64::from(width.div_ceil(METER_TILE) * height.div_ceil(METER_TILE)),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            split_list: std::sync::OnceLock::new(),
            split_stub: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("split walk rays (unsplit)"),
                size: 32,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            split_args: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("split walk dispatch"),
                size: 16,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::INDIRECT
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
        }
    }

    /// The split walk's hand-off list at one ray per pixel, built here on the first call.
    pub(super) fn split_list(&self, device: &wgpu::Device) -> &wgpu::Buffer {
        self.split_list.get_or_init(|| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("split walk rays"),
                size: 16 + 16 * u64::from(self.width) * u64::from(self.height),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        })
    }

    /// The meter's tiles on each edge.
    pub(super) fn tile_counts(&self) -> [u32; 2] {
        [self.width, self.height].map(|n| n.div_ceil(METER_TILE))
    }

    /// The lighting grid's size.
    pub(super) fn light_size(&self) -> [u32; 2] {
        [self.irradiance.width(), self.irradiance.height()]
    }

    /// The lighting grid's hit positions and records, copy `i`.
    pub(super) fn lit_pos(&self, i: usize) -> &wgpu::Texture {
        self.half_pos.as_ref().map_or(&self.hit_pos[i], |h| &h[i])
    }

    pub(super) fn lit_info(&self, i: usize) -> &wgpu::Texture {
        self.half_info.as_ref().map_or(&self.hit_info[i], |h| &h[i])
    }

    /// Bytes the targets occupy.
    pub(super) fn bytes(&self) -> u64 {
        let px = u64::from(self.width) * u64::from(self.height);
        let [lw, lh] = self.light_size();
        let lpx = u64::from(lw) * u64::from(lh);
        let half = if self.scale > 1 { 2 * (16 + 8) } else { 0 };
        let strata = u64::from(self.strata.width()) * u64::from(self.strata.height());
        let output = u64::from(self.output.width()) * u64::from(self.output.height());
        let history = self.history.as_ref().map_or(0, |h| {
            2 * 8 * u64::from(h[0].width()) * u64::from(h[0].height())
        });
        px * (2 * (16 + 8) + 8)
            + output * 4
            + history
            + lpx * (half + 8 + 2 * 8 + 2 * 8 + 2 * 8 + 8)
            + strata * 8
            + self.tiles.size()
            + self.split_list.get().map_or(0, wgpu::Buffer::size)
            + self.split_stub.size()
            + self.split_args.size()
    }
}
