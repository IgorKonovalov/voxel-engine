//! Helpers the passes share: a pass's group layout and bind group from a list of its
//! bindings, texture views and layout entries, and compute pipelines from a WGSL source.

/// A group-2 binding of a frame pass.
#[derive(Clone, Copy)]
pub(crate) enum Io {
    /// A `texture_2d<f32>` read with `textureLoad`.
    Float,
    /// A `texture_2d<u32>`.
    Uint,
    /// A write-only storage texture of this format.
    Store(wgpu::TextureFormat),
    /// A uniform buffer.
    Uniform,
    /// A read-write storage buffer.
    Storage,
    /// A read-only storage buffer.
    ReadStorage,
}

pub(crate) fn io_layout(device: &wgpu::Device, label: &str, io: &[Io]) -> wgpu::BindGroupLayout {
    let at: Vec<(u32, Io)> = io.iter().enumerate().map(|(i, &k)| (i as u32, k)).collect();
    io_layout_at(device, label, &at)
}

/// A group-2 layout of `io` at explicit binding numbers.
pub(crate) fn io_layout_at(
    device: &wgpu::Device,
    label: &str,
    io: &[(u32, Io)],
) -> wgpu::BindGroupLayout {
    let entries: Vec<wgpu::BindGroupLayoutEntry> = io
        .iter()
        .map(|&(b, k)| match k {
            Io::Float => sampled_texture(b, FLOAT),
            Io::Uint => sampled_texture(b, UINT),
            Io::Store(f) => storage_texture(b, f),
            Io::Uniform | Io::Storage | Io::ReadStorage => wgpu::BindGroupLayoutEntry {
                binding: b,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: if matches!(k, Io::Uniform) {
                        wgpu::BufferBindingType::Uniform
                    } else {
                        wgpu::BufferBindingType::Storage {
                            read_only: matches!(k, Io::ReadStorage),
                        }
                    },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        })
        .collect();
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &entries,
    })
}

/// One binding of a pass's group-2 bind group.
pub(crate) enum Res<'a> {
    Tex(&'a wgpu::Texture),
    Buf(&'a wgpu::Buffer),
}

pub(crate) fn bind_io(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    res: &[Res<'_>],
) -> wgpu::BindGroup {
    let at: Vec<u32> = (0..res.len() as u32).collect();
    bind_io_at(device, layout, &at, res)
}

/// A group-2 bind group of `res` at the binding numbers `at`.
pub(crate) fn bind_io_at(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    at: &[u32],
    res: &[Res<'_>],
) -> wgpu::BindGroup {
    let views: Vec<Option<wgpu::TextureView>> = res
        .iter()
        .map(|r| match r {
            Res::Tex(t) => Some(view(t)),
            Res::Buf(_) => None,
        })
        .collect();
    let entries: Vec<wgpu::BindGroupEntry<'_>> = res
        .iter()
        .zip(&views)
        .zip(at)
        .map(|((r, v), &binding)| wgpu::BindGroupEntry {
            binding,
            resource: match (r, v) {
                (Res::Buf(b), _) => b.as_entire_binding(),
                (_, Some(v)) => wgpu::BindingResource::TextureView(v),
                (Res::Tex(_), None) => unreachable!("a texture binding has its view"),
            },
        })
        .collect();
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout,
        entries: &entries,
    })
}

pub(crate) fn buffer_entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

pub(crate) fn view(t: &wgpu::Texture) -> wgpu::TextureView {
    t.create_view(&wgpu::TextureViewDescriptor::default())
}

pub(crate) fn storage_texture(
    binding: u32,
    format: wgpu::TextureFormat,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

pub(crate) fn sampled_texture(
    binding: u32,
    sample_type: wgpu::TextureSampleType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

pub(crate) const FLOAT: wgpu::TextureSampleType =
    wgpu::TextureSampleType::Float { filterable: false };

pub(crate) const UINT: wgpu::TextureSampleType = wgpu::TextureSampleType::Uint;

pub(crate) fn compute_pipeline(
    device: &wgpu::Device,
    label: &str,
    source: String,
    layouts: &[Option<&wgpu::BindGroupLayout>],
    entry: &str,
) -> wgpu::ComputePipeline {
    compute_pipeline_with(device, label, source, layouts, entry, &[])
}

/// [`compute_pipeline`] with the pipeline-overridable constants `constants`.
pub(crate) fn compute_pipeline_with(
    device: &wgpu::Device,
    label: &str,
    source: String,
    layouts: &[Option<&wgpu::BindGroupLayout>],
    entry: &str,
    constants: &[(&str, f64)],
) -> wgpu::ComputePipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: layouts,
        immediate_size: 0,
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        module: &module,
        entry_point: Some(entry),
        compilation_options: wgpu::PipelineCompilationOptions {
            constants,
            ..Default::default()
        },
        cache: None,
    })
}
