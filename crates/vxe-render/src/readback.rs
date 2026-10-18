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
/// pixels: the histogram of steps per ray, and of pool reads per ray, each clamped at 255 as
/// the view clamps them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepHistogram {
    /// `steps[n]` rays took `n` steps (`n = 255`: 255 or more).
    pub steps: [u64; 256],
    /// `pool_reads[n]` rays read the pool `n` times.
    pub pool_reads: [u64; 256],
}

impl StepHistogram {
    /// The histograms of `pixels`, tightly packed RGBA bytes: red is steps, green pool reads.
    pub fn from_rgba8(pixels: &[u8]) -> Self {
        let mut h = Self {
            steps: [0; 256],
            pool_reads: [0; 256],
        };
        for p in pixels.chunks_exact(4) {
            h.steps[usize::from(p[0])] += 1;
            h.pool_reads[usize::from(p[1])] += 1;
        }
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
    fn the_histogram_reads_red_as_steps_and_green_as_pool_reads() {
        // 100 rays: 98 of 10 steps, one of 20, one of 255; pool reads 1 each, one of 3.
        let mut px = Vec::new();
        for i in 0..100u32 {
            let steps = match i {
                0 => 255,
                1 => 20,
                _ => 10,
            };
            px.extend_from_slice(&[steps, if i == 5 { 3 } else { 1 }, 0, 255]);
        }
        let h = StepHistogram::from_rgba8(&px);
        assert_eq!(h.rays(), 100);
        assert_eq!(h.steps[10], 98);
        assert_eq!(h.mean_steps(), (98.0 * 10.0 + 20.0 + 255.0) / 100.0);
        assert_eq!(h.p99_steps(), 20);
        assert_eq!(h.mean_pool_reads(), 1.02);
        assert_eq!(h.p99_pool_reads(), 1);
    }
}
