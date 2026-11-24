//! The present blit on the software adapter (lavapipe): a source holding every 8-bit value
//! reaches an `Rgba8Unorm` and an `Rgba8UnormSrgb` target as the same bytes, so the two surface
//! encodings show the marcher's display-encoded target alike.

use svet_render::{Presenter, wgpu};

/// One row of 256 pixels: pixel `i` is grey `i`, alpha 255.
const W: u32 = 256;

fn texture(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("present test"),
        size: wgpu::Extent3d {
            width: W,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

/// The grey ramp blitted onto a target of `format`, read back as stored bytes.
fn blit_ramp(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Vec<u8> {
    let source = texture(
        device,
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    );
    let ramp: Vec<u8> = (0..=255u8).flat_map(|i| [i, i, i, 255]).collect();
    queue.write_texture(
        source.as_image_copy(),
        &ramp,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(W * 4),
            rows_per_image: None,
        },
        source.size(),
    );
    let target = texture(
        device,
        format,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let presenter = Presenter::new(device, format);
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    presenter.encode(device, &mut enc, &source, &view);
    queue.submit([enc.finish()]);
    svet_render::debug::read_rgba8(device, queue, &target)
}

/// Pixel `i` of both targets stores byte `i` in every colour channel: `fs_main` copies, and
/// `fs_srgb`'s exact decode undoes the target's sRGB encode.
#[test]
fn both_surface_encodings_store_every_byte_of_the_source() {
    let Some((device, queue)) =
        svet_render::test_support::software_device(0, wgpu::Features::empty())
    else {
        return;
    };
    for format in [
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Rgba8UnormSrgb,
    ] {
        let img = blit_ramp(&device, &queue, format);
        for (i, p) in img.chunks(4).enumerate() {
            assert_eq!(p[..3], [i as u8; 3], "{format:?}: pixel {i}");
        }
    }
}
