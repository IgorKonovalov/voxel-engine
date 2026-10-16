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
