//! A text overlay drawn over a finished frame (ADR-0009): rows of ASCII bytes laid out as a
//! grid of 8 x 8 glyph cells at an integer scale, and a strip of bars for recent samples (such
//! as frame times), alpha-blended onto the target by one fullscreen fragment pass. The
//! overlay knows nothing of what the rows or samples say; the application supplies them.
//!
//! The cell grid is laid out at its cap, [`GRID_COLS`] x [`GRID_ROWS`] cells, with a fixed row
//! stride of [`GRID_COLS`]; the part a target shows is `ceil(width / (8 scale))` x
//! `ceil(height / (8 scale))` cells from the top-left, and the rest is clipped, never wrapped.

use crate::font8x8::{FONT, printable};

/// The source of the overlay shader, for headless validation.
pub const HUD_WGSL: &str = include_str!("shaders/hud.wgsl");

/// The most columns of cells the overlay holds; the cell buffer's row stride.
pub const GRID_COLS: u32 = 256;
/// The most rows of cells the overlay holds.
pub const GRID_ROWS: u32 = 128;
/// Bytes of the cell buffer: one `u32` per cell at the cap (128 KiB).
pub const CELL_BYTES: u64 = (GRID_COLS * GRID_ROWS * 4) as u64;
/// The most samples the strip holds.
pub const MAX_SAMPLES: usize = 256;
/// The strip's height in glyph pixels: `64 scale` target pixels.
pub const STRIP_PX: u32 = 64;
/// The frame time the strip's red line marks when its ceiling is above it: one 60 Hz frame.
pub const LINE_MS: f32 = 1000.0 / 60.0;
/// `Params::line_px` for no line.
const NO_LINE: u32 = u32::MAX;

/// The colour of a row of text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HudColour {
    #[default]
    White = 0,
    Yellow = 1,
    Red = 2,
}

/// The uniform block of `hud.wgsl`, field for field.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    scale: u32,
    sample_count: u32,
    ceiling_ms: f32,
    line_px: u32,
}

/// The overlay's GPU state: its pipeline, the cell grid and the strip's samples, written by
/// the `set_*` calls and drawn by [`HudOverlay::encode`] over whatever the target holds.
/// After the first call of each, the `set_*` calls and `encode` reuse every buffer they
/// write through; [`HudOverlay::allocations`] counts the overlay's own CPU allocations.
pub struct HudOverlay {
    pipeline: wgpu::RenderPipeline,
    bind: wgpu::BindGroup,
    params: wgpu::Buffer,
    cells: wgpu::Buffer,
    samples: wgpu::Buffer,
    /// Target pixels per glyph pixel along each axis: a cell covers `8 scale` x `8 scale`.
    scale: u32,
    /// Rows of cells the last `set_text` wrote; the next one clears any it does not reuse.
    rows: u32,
    /// The CPU copy of the rows being written, reused between calls.
    staging: Vec<u32>,
    /// Times `staging` grew its allocation.
    allocations: u32,
}

impl HudOverlay {
    /// An overlay for targets of `format`, each glyph pixel drawn as `scale` x `scale` target
    /// pixels. `scale` is at least 1.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, scale: u32) -> Self {
        let scale = scale.max(1);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hud.wgsl"),
            source: wgpu::ShaderSource::Wgsl(HUD_WGSL.into()),
        });
        let storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hud"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
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
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hud"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hud"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let buffer = |label, size, usage, contents: Option<&[u8]>| {
            let b = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: contents.is_some(),
            });
            if let Some(bytes) = contents {
                b.slice(..)
                    .get_mapped_range_mut()
                    .expect("mapped at creation")
                    .copy_from_slice(bytes);
                b.unmap();
            }
            b
        };
        let params = buffer(
            "hud params",
            size_of::<Params>() as u64,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            Some(bytemuck::bytes_of(&Params {
                scale,
                sample_count: 0,
                ceiling_ms: 1.0,
                line_px: NO_LINE,
            })),
        );
        let cells = buffer(
            "hud cells",
            CELL_BYTES,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            None,
        );
        let samples = buffer(
            "hud samples",
            (MAX_SAMPLES * size_of::<f32>()) as u64,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            None,
        );
        let font = buffer(
            "hud font",
            size_of_val(&FONT) as u64,
            wgpu::BufferUsages::STORAGE,
            Some(FONT.as_flattened()),
        );
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hud"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: cells.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: font.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: samples.as_entire_binding(),
                },
            ],
        });
        Self {
            pipeline,
            bind,
            params,
            cells,
            samples,
            scale,
            rows: 0,
            staging: Vec::new(),
            allocations: 0,
        }
    }

    /// Target pixels per glyph pixel along each axis, as given to [`HudOverlay::new`].
    pub fn scale(&self) -> u32 {
        self.scale
    }

    /// The CPU allocations the overlay has made: each time its staging copy of the rows grew.
    /// Rows that fit the cells already staged allocate nothing.
    pub fn allocations(&self) -> u32 {
        self.allocations
    }

    /// Replaces the text with `rows`, all white, from the top-left cell.
    pub fn set_text(&mut self, queue: &wgpu::Queue, rows: &[&[u8]]) {
        self.write_rows(queue, rows.iter().map(|&r| (r, HudColour::White)));
    }

    /// Replaces the text with the lines of `text`, line `i` in `colours[i]` (white past the
    /// end of `colours`), as [`Self::set_rows`] does, without a slice of rows to build.
    pub fn set_lines(&mut self, queue: &wgpu::Queue, text: &str, colours: &[HudColour]) {
        let colour = |i| colours.get(i).copied().unwrap_or_default();
        let rows = text
            .lines()
            .enumerate()
            .map(|(i, l)| (l.as_bytes(), colour(i)));
        self.write_rows(queue, Counted(text.lines().count(), rows));
    }

    /// Replaces the text with `rows`, each in its colour, from the top-left cell. A row past
    /// [`GRID_ROWS`] or a byte past [`GRID_COLS`] is dropped; a byte outside 32 to 126 draws
    /// as `?`.
    pub fn set_rows(&mut self, queue: &wgpu::Queue, rows: &[(&[u8], HudColour)]) {
        self.write_rows(queue, rows.iter().copied());
    }

    fn write_rows<'a>(
        &mut self,
        queue: &wgpu::Queue,
        rows: impl ExactSizeIterator<Item = (&'a [u8], HudColour)>,
    ) {
        let n = (rows.len() as u32).min(GRID_ROWS);
        // Rows the previous text used and this one does not are written blank.
        let written = n.max(self.rows);
        let capacity = self.staging.capacity();
        self.staging.clear();
        self.staging.resize((written * GRID_COLS) as usize, 0);
        self.allocations += u32::from(self.staging.capacity() != capacity);
        for (cells, (bytes, colour)) in self.staging.chunks_mut(GRID_COLS as usize).zip(rows) {
            for (cell, &b) in cells.iter_mut().zip(bytes) {
                *cell = u32::from(printable(b)) | (colour as u32) << 8;
            }
        }
        if written > 0 {
            queue.write_buffer(&self.cells, 0, bytemuck::cast_slice(&self.staging));
        }
        self.rows = n;
    }

    /// Replaces the strip with `samples`, oldest first; past [`MAX_SAMPLES`] only the last
    /// ones are kept. A sample of `ceiling_ms` fills the strip's height and larger ones clamp;
    /// when `ceiling_ms` is above [`LINE_MS`] a one-pixel red line marks that height. An empty
    /// slice or a `ceiling_ms` that is not above zero hides the strip.
    pub fn set_samples(&mut self, queue: &wgpu::Queue, samples: &[f32], ceiling_ms: f32) {
        let samples = &samples[samples.len().saturating_sub(MAX_SAMPLES)..];
        let shown = ceiling_ms > 0.0 && ceiling_ms.is_finite();
        let strip_px = STRIP_PX * self.scale;
        let params = Params {
            scale: self.scale,
            sample_count: if shown { samples.len() as u32 } else { 0 },
            ceiling_ms: if shown { ceiling_ms } else { 1.0 },
            line_px: if shown && ceiling_ms > LINE_MS {
                ((LINE_MS / ceiling_ms * strip_px as f32).round() as u32).min(strip_px - 1)
            } else {
                NO_LINE
            },
        };
        if !samples.is_empty() {
            queue.write_buffer(&self.samples, 0, bytemuck::cast_slice(samples));
        }
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
    }

    /// Records the overlay onto `view`, a `width` x `height` target, into `encoder`; what the
    /// target holds stays under it.
    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        self.encode_timed(encoder, view, width, height, None);
    }

    /// [`Self::encode`], writing the pass's start and end timestamps when `timestamps` is
    /// given.
    pub fn encode_timed(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
        timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("hud"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            timestamp_writes: timestamps,
            ..Default::default()
        });
        if width == 0 || height == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind, &[]);
        // The first instance is the target height, read by the shader as `instance_index`.
        pass.draw(0..3, height..height + 1);
    }
}

/// An iterator of known length: `.0` items of `.1`.
struct Counted<I>(usize, I);

impl<I: Iterator> Iterator for Counted<I> {
    type Item = I::Item;

    fn next(&mut self) -> Option<I::Item> {
        let item = self.1.next()?;
        self.0 -= 1;
        Some(item)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.0, Some(self.0))
    }
}

impl<I: Iterator> ExactSizeIterator for Counted<I> {}

#[cfg(test)]
mod tests {
    use super::HUD_WGSL;

    #[test]
    fn hud_wgsl_validates_through_naga() {
        let module = wgpu::naga::front::wgsl::parse_str(HUD_WGSL).expect("hud.wgsl parses");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::default(),
        )
        .validate(&module)
        .expect("hud.wgsl validates");
    }
}
