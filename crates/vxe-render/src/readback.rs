//! Blocking copies from GPU resources back to the CPU, for tests and tools.

/// Bytes of `buffer` in `offset..offset + size` (both multiples of 4).
pub fn read_buffer(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    buffer: &wgpu::Buffer,
    offset: u64,
    size: u64,
) -> Vec<u8> {
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    enc.copy_buffer_to_buffer(buffer, offset, &staging, 0, size);
    queue.submit([enc.finish()]);
    map_and_copy(device, &staging, |bytes| bytes.to_vec())
}

/// The pixels of an `rgba8unorm` texture, tightly packed, rows top to bottom.
pub fn read_rgba8(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<u8> {
    let (w, h) = (texture.width(), texture.height());
    let row = w * 4;
    let padded =
        row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback rgba8"),
        size: u64::from(padded * h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    enc.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(h),
            },
        },
        texture.size(),
    );
    queue.submit([enc.finish()]);
    map_and_copy(device, &staging, |bytes| {
        bytes
            .chunks(padded as usize)
            .flat_map(|r| &r[..row as usize])
            .copied()
            .collect()
    })
}

/// The pixels of a texture of `texel` bytes per pixel, tightly packed, rows top to bottom.
fn read_texels(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    texel: u32,
) -> Vec<u8> {
    let (w, h) = (texture.width(), texture.height());
    let row = w * texel;
    let padded =
        row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback texels"),
        size: u64::from(padded * h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    enc.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(h),
            },
        },
        texture.size(),
    );
    queue.submit([enc.finish()]);
    map_and_copy(device, &staging, |bytes| {
        bytes
            .chunks(padded as usize)
            .flat_map(|r| &r[..row as usize])
            .copied()
            .collect()
    })
}

/// The pixels of an `rgba32float` texture, rows top to bottom.
pub fn read_rgba32f(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> Vec<[f32; 4]> {
    read_texels(device, queue, texture, 16)
        .chunks(16)
        .map(|p| [0, 1, 2, 3].map(|k| f32::from_le_bytes(p[4 * k..4 * k + 4].try_into().unwrap())))
        .collect()
}

/// The pixels of an `rgba16uint` texture, rows top to bottom.
pub fn read_rgba16u(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> Vec<[u16; 4]> {
    read_texels(device, queue, texture, 8)
        .chunks(8)
        .map(|p| [0, 1, 2, 3].map(|k| u16::from_le_bytes([p[2 * k], p[2 * k + 1]])))
        .collect()
}

/// The pixels of an `rgba16float` texture as `f32`, rows top to bottom.
pub fn read_rgba16f(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> Vec<[f32; 4]> {
    let (w, h) = (texture.width(), texture.height());
    let row = w * 8;
    let padded =
        row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback rgba16f"),
        size: u64::from(padded * h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    enc.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(h),
            },
        },
        texture.size(),
    );
    queue.submit([enc.finish()]);
    map_and_copy(device, &staging, |bytes| {
        bytes
            .chunks(padded as usize)
            .flat_map(|r| r[..row as usize].chunks(8))
            .map(|p| [0, 1, 2, 3].map(|k| f16_to_f32(u16::from_le_bytes([p[2 * k], p[2 * k + 1]]))))
            .collect()
    })
}

/// An IEEE binary16 value as `f32`.
pub fn f16_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from((h >> 10) & 0x1F);
    let frac = f32::from(h & 0x3FF);
    match exp {
        0 => sign * frac * 2f32.powi(-24),
        31 if frac == 0.0 => sign * f32::INFINITY,
        31 => f32::NAN,
        _ => sign * (1.0 + frac / 1024.0) * 2f32.powi(exp - 15),
    }
}

fn map_and_copy<T>(device: &wgpu::Device, staging: &wgpu::Buffer, f: impl FnOnce(&[u8]) -> T) -> T {
    staging.map_async(wgpu::MapMode::Read, .., |r| {
        r.expect("readback buffer maps");
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("device poll for readback");
    let out = f(&staging.get_mapped_range(..).expect("mapped range"));
    staging.unmap();
    out
}

/// The walk cost a frame of the steps view (`ViewMode::Steps`) holds, from its `rgba8unorm`
/// pixels: the histograms of steps, storage loads and pool loads per ray, each clamped at 255
/// as the view clamps them, and each 8 × 8 pixel block's largest step count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepHistogram {
    /// `steps[n]` rays took `n` steps (`n = 255`: 255 or more).
    pub steps: [u64; 256],
    /// `loads[n]` rays issued `n` 32-bit storage loads from the mirror (chunk table, record
    /// bits and entries, masks, pool, palette).
    pub loads: [u64; 256],
    /// `pool_reads[n]` rays loaded `n` pool words, counted among `loads` too.
    pub pool_reads: [u64; 256],
    /// The sum over 8 × 8 pixel blocks (those at the right and bottom edges may be smaller) of
    /// each block's maximum steps, and the number of blocks.
    pub block_max_sum: u64,
    pub blocks: u64,
}

/// Edge in pixels of the blocks `StepHistogram` takes the maximum over: the marcher's
/// workgroup, the rays that run in lockstep.
pub const STEP_BLOCK: usize = 8;

impl StepHistogram {
    /// The histograms of `pixels`, tightly packed RGBA bytes in rows of `width` pixels: red is
    /// steps, green storage loads, blue pool loads.
    pub fn from_rgba8(pixels: &[u8], width: usize) -> Self {
        let mut h = Self {
            steps: [0; 256],
            loads: [0; 256],
            pool_reads: [0; 256],
            block_max_sum: 0,
            blocks: 0,
        };
        for p in pixels.chunks_exact(4) {
            h.steps[usize::from(p[0])] += 1;
            h.loads[usize::from(p[1])] += 1;
            h.pool_reads[usize::from(p[2])] += 1;
        }
        let width = width.max(1);
        let height = pixels.len() / 4 / width;
        let bw = width.div_ceil(STEP_BLOCK);
        let mut max = vec![0u8; bw * height.div_ceil(STEP_BLOCK)];
        for (i, p) in pixels.chunks_exact(4).take(width * height).enumerate() {
            let b = (i / width / STEP_BLOCK) * bw + (i % width) / STEP_BLOCK;
            max[b] = max[b].max(p[0]);
        }
        h.block_max_sum = max.iter().map(|&m| u64::from(m)).sum();
        h.blocks = max.len() as u64;
        h
    }

    pub fn rays(&self) -> u64 {
        self.steps.iter().sum()
    }

    /// Mean steps per ray, a clamped ray counting 255; 0 with no rays.
    pub fn mean_steps(&self) -> f64 {
        mean(&self.steps)
    }

    /// The smallest step count at or above which no more than 1 % of rays lie: the 99th
    /// percentile, 255 when more than 1 % of rays clamp.
    pub fn p99_steps(&self) -> u32 {
        percentile(&self.steps, 0.99)
    }

    /// Mean over 8 × 8 pixel blocks of each block's maximum steps: what a group of rays in
    /// lockstep pays. 0 with no rays.
    pub fn block_max_mean(&self) -> f64 {
        if self.blocks == 0 {
            return 0.0;
        }
        self.block_max_sum as f64 / self.blocks as f64
    }

    /// Mean storage loads per ray, a clamped ray counting 255; 0 with no rays.
    pub fn mean_loads(&self) -> f64 {
        mean(&self.loads)
    }

    pub fn p99_loads(&self) -> u32 {
        percentile(&self.loads, 0.99)
    }

    pub fn mean_pool_reads(&self) -> f64 {
        mean(&self.pool_reads)
    }

    pub fn p99_pool_reads(&self) -> u32 {
        percentile(&self.pool_reads, 0.99)
    }
}

fn mean(h: &[u64; 256]) -> f64 {
    let n: u64 = h.iter().sum();
    if n == 0 {
        return 0.0;
    }
    let s: u64 = h.iter().enumerate().map(|(i, &c)| i as u64 * c).sum();
    s as f64 / n as f64
}

/// The least `v` with at least `q` of the counts at or below it.
fn percentile(h: &[u64; 256], q: f64) -> u32 {
    let n: u64 = h.iter().sum();
    let need = (q * n as f64).ceil() as u64;
    let mut acc = 0;
    for (i, &c) in h.iter().enumerate() {
        acc += c;
        if acc >= need {
            return i as u32;
        }
    }
    255
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_histogram_reads_red_as_steps_green_as_loads_and_blue_as_pool_loads() {
        // 100 rays: 98 of 10 steps, one of 20, one of 255; 7 loads each, one of 40; pool
        // loads 1 each, one of 3.
        let mut px = Vec::new();
        for i in 0..100u32 {
            let steps = match i {
                0 => 255,
                1 => 20,
                _ => 10,
            };
            px.extend_from_slice(&[
                steps,
                if i == 2 { 40 } else { 7 },
                if i == 5 { 3 } else { 1 },
                255,
            ]);
        }
        let h = StepHistogram::from_rgba8(&px, 100);
        assert_eq!(h.rays(), 100);
        assert_eq!(h.steps[10], 98);
        assert_eq!(h.mean_steps(), (98.0 * 10.0 + 20.0 + 255.0) / 100.0);
        assert_eq!(h.p99_steps(), 20);
        assert_eq!(h.loads[7], 99);
        assert_eq!(h.mean_loads(), 7.33);
        assert_eq!(h.p99_loads(), 7);
        assert_eq!(h.mean_pool_reads(), 1.02);
        assert_eq!(h.p99_pool_reads(), 1);
        // One row of 100 pixels: 13 blocks, the first holding the 255 and the 20, the rest 10.
        assert_eq!(h.blocks, 13);
        assert_eq!(h.block_max_mean(), (255.0 + 12.0 * 10.0) / 13.0);
    }

    #[test]
    fn the_block_maximum_is_taken_over_8_by_8_pixel_blocks() {
        // 16 × 10 pixels: four blocks, two of them 8 × 2 at the bottom. Steps are 1 except
        // one pixel of 50 in the top-right block and one of 7 in the bottom-left block.
        let (w, h) = (16, 10);
        let mut px = vec![0u8; w * h * 4];
        for (i, p) in px.chunks_exact_mut(4).enumerate() {
            let (x, y) = (i % w, i / w);
            p[0] = match (x, y) {
                (12, 3) => 50,
                (2, 9) => 7,
                _ => 1,
            };
        }
        let s = StepHistogram::from_rgba8(&px, w);
        assert_eq!(s.blocks, 4);
        assert_eq!(s.block_max_sum, 1 + 50 + 7 + 1);
        assert_eq!(s.block_max_mean(), 59.0 / 4.0);
    }
}
