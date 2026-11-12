//! FSR 2's pipelines and their layouts, and the bind groups and dispatches of one frame.

use super::*;

/// One FSR pass: its group-1 layout (group 2 for ours) and pipeline.
pub(super) struct Pass {
    pub(super) layout: wgpu::BindGroupLayout,
    pub(super) pipeline: wgpu::ComputePipeline,
}

/// The bind groups of one encoded frame.
pub(crate) struct Fsr2Binds {
    pub(super) cb: wgpu::BindGroup,
    pub(super) input: wgpu::BindGroup,
    pub(super) spd_mips: wgpu::BindGroup,
    pub(super) spd_tail: wgpu::BindGroup,
    pub(super) reconstruct: wgpu::BindGroup,
    pub(super) depth_clip: wgpu::BindGroup,
    pub(super) lock: wgpu::BindGroup,
    pub(super) accumulate: wgpu::BindGroup,
    /// With sharpening on: RCAS and its workgroups.
    pub(super) rcas: Option<(wgpu::BindGroup, [u32; 2])>,
    pub(super) tone: wgpu::BindGroup,
    pub(super) bypass: bool,
    pub(super) render_groups: [u32; 2],
    pub(super) display_groups: [u32; 2],
    pub(super) spd_groups: [u32; 2],
}

impl Fsr2Binds {
    /// The dispatches of FSR 2's frame passes ([`FSR2_PASSES`], one slot each), after the
    /// composite, in AMD's order: our input pass, the luminance pyramid (its mips, then its
    /// tail), reconstruct previous depth, depth clip, lock, accumulate, then RCAS (which tone
    /// maps into the frame's image) when sharpening is on, else our tone pass. Bypassed, the
    /// tone pass alone runs. The names of the dispatched slots go to [`Fsr2::dispatched`].
    pub(crate) fn passes<'a>(
        &'a self,
        fsr: &'a Fsr2,
        world: &'a wgpu::BindGroup,
    ) -> [Vec<Dispatch<'a>>; FSR2_PASSES.len()] {
        let fsr_pass = |p: &'a Pass, b: &'a wgpu::BindGroup, workgroups| {
            vec![Dispatch {
                pipeline: &p.pipeline,
                groups: [Some(&self.cb), Some(b), None, None],
                workgroups,
                indirect: None,
            }]
        };
        let tone = vec![Dispatch {
            pipeline: &fsr.tone.pipeline,
            groups: [Some(world), None, Some(&self.tone), None],
            workgroups: self.display_groups,
            indirect: None,
        }];
        let record = |passes: &[Vec<Dispatch<'a>>; FSR2_PASSES.len()]| {
            *fsr.dispatched
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = FSR2_PASSES
                .iter()
                .zip(passes)
                .filter(|(_, d)| !d.is_empty())
                .map(|(n, _)| *n)
                .collect();
        };
        if self.bypass {
            let mut passes: [Vec<Dispatch<'a>>; FSR2_PASSES.len()] = Default::default();
            passes[FSR2_PASSES.len() - 1] = tone;
            record(&passes);
            return passes;
        }
        let (rcas, tone) = match &self.rcas {
            Some((b, workgroups)) => (fsr_pass(&fsr.rcas, b, *workgroups), Vec::new()),
            None => (Vec::new(), tone),
        };
        let passes = [
            vec![Dispatch {
                pipeline: &fsr.input.pipeline,
                groups: [Some(world), None, Some(&self.input), None],
                workgroups: self.render_groups,
                indirect: None,
            }],
            fsr_pass(&fsr.spd_mips, &self.spd_mips, self.spd_groups),
            fsr_pass(&fsr.spd_tail, &self.spd_tail, [1, 1]),
            fsr_pass(&fsr.reconstruct, &self.reconstruct, self.render_groups),
            fsr_pass(&fsr.depth_clip, &self.depth_clip, self.render_groups),
            fsr_pass(&fsr.lock, &self.lock, self.render_groups),
            fsr_pass(&fsr.accumulate, &self.accumulate, self.display_groups),
            rcas,
            tone,
        ];
        record(&passes);
        passes
    }
}

pub(super) fn layout(
    device: &wgpu::Device,
    label: &str,
    entries: &[wgpu::BindGroupLayoutEntry],
) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries,
    })
}

pub(super) fn entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty,
        count: None,
    }
}

pub(super) fn tex_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
    )
}

pub(super) fn uint_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Uint,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
    )
}

pub(super) fn store_entry(binding: u32, format: wgpu::TextureFormat) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
    )
}

pub(super) fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
    )
}

pub(super) fn rw_buffer_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
    )
}

/// A binding of a bind group: a texture's whole view or a buffer.
pub(super) enum Bind<'a> {
    Tex(&'a wgpu::Texture),
    Buf(&'a wgpu::Buffer),
}

pub(super) fn bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    res: &[Bind<'_>],
) -> wgpu::BindGroup {
    let at: Vec<(u32, &Bind<'_>)> = res.iter().enumerate().map(|(i, r)| (i as u32, r)).collect();
    bind_refs(device, layout, &at)
}

/// A bind group of `res` at explicit binding numbers.
pub(super) fn bind_at(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    res: &[(u32, Bind<'_>)],
) -> wgpu::BindGroup {
    let at: Vec<(u32, &Bind<'_>)> = res.iter().map(|(b, r)| (*b, r)).collect();
    bind_refs(device, layout, &at)
}

pub(super) fn bind_refs(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    res: &[(u32, &Bind<'_>)],
) -> wgpu::BindGroup {
    let views: Vec<Option<wgpu::TextureView>> = res
        .iter()
        .map(|(_, r)| match r {
            Bind::Tex(t) => Some(t.create_view(&wgpu::TextureViewDescriptor::default())),
            Bind::Buf(_) => None,
        })
        .collect();
    let entries: Vec<wgpu::BindGroupEntry<'_>> = res
        .iter()
        .zip(&views)
        .map(|((binding, r), v)| wgpu::BindGroupEntry {
            binding: *binding,
            resource: match (r, v) {
                (Bind::Buf(b), _) => b.as_entire_binding(),
                (_, Some(v)) => wgpu::BindingResource::TextureView(v),
                (Bind::Tex(_), None) => unreachable!("a texture binding has its view"),
            },
        })
        .collect();
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout,
        entries: &entries,
    })
}

pub(super) fn pipeline(
    device: &wgpu::Device,
    label: &str,
    source: String,
    layouts: &[Option<&wgpu::BindGroupLayout>],
    entry: &str,
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
        compilation_options: Default::default(),
        cache: None,
    })
}

pub(super) fn ro_buffer_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
    )
}

impl Fsr2 {
    /// FSR 2's pipelines and its resources for `render`-sized frames shown at `display`;
    /// `world_layout` is the frame's group 0, which our input and tone passes read.
    pub(crate) fn new(
        device: &wgpu::Device,
        world_layout: &wgpu::BindGroupLayout,
        render: [u32; 2],
        display: [u32; 2],
        settings: Fsr2Settings,
    ) -> Self {
        use wgpu::TextureFormat::{R32Float, Rg32Float, Rgba8Unorm, Rgba16Float};
        let cb_layout = layout(device, "fsr2 constants", &[uniform_entry(0)]);
        // Ours: group 0 is the frame's world, group 2 the pass's.
        let ours = |label: &str,
                    source: &str,
                    entry_point: &str,
                    entries: &[wgpu::BindGroupLayoutEntry]| {
            let l = layout(device, label, entries);
            let p = pipeline(
                device,
                label,
                format!("{}\n{source}", crate::MARCH_WGSL),
                &[Some(world_layout), None, Some(&l)],
                entry_point,
            );
            Pass {
                layout: l,
                pipeline: p,
            }
        };
        // AMD's: group 0 is the constants, group 1 the pass's.
        let amd = |label: &str,
                   source: &str,
                   entry_point: &str,
                   entries: &[wgpu::BindGroupLayoutEntry]| {
            let l = layout(device, label, entries);
            let p = pipeline(
                device,
                label,
                fsr2_source(source, true),
                &[Some(&cb_layout), Some(&l)],
                entry_point,
            );
            Pass {
                layout: l,
                pipeline: p,
            }
        };
        let input = ours(
            "fsr2 input",
            FSR2_INPUT_WGSL,
            "fsr2_input",
            &[
                tex_entry(0),
                uint_entry(1),
                tex_entry(2),
                uniform_entry(3),
                uniform_entry(4),
                uniform_entry(5),
                store_entry(6, R32Float),
                store_entry(7, Rg32Float),
                store_entry(8, Rgba8Unorm),
            ],
        );
        let tone = ours(
            "fsr2 tone",
            FSR2_TONE_WGSL,
            "tone",
            &[tex_entry(0), store_entry(1, Rgba8Unorm)],
        );
        let spd_mips = amd(
            "fsr2 luminance pyramid",
            FSR2_LUMINANCE_PYRAMID_WGSL,
            "spd_mips",
            &[
                tex_entry(0),
                store_entry(1, R32Float),
                store_entry(2, R32Float),
            ],
        );
        let spd_tail = amd(
            "fsr2 luminance pyramid tail",
            FSR2_LUMINANCE_PYRAMID_WGSL,
            "spd_tail",
            &[tex_entry(3), store_entry(4, Rg32Float), tex_entry(5)],
        );
        let reconstruct = amd(
            "fsr2 reconstruct previous depth",
            FSR2_RECONSTRUCT_WGSL,
            "reconstruct_and_dilate",
            &[
                tex_entry(0),
                tex_entry(1),
                tex_entry(2),
                tex_entry(3),
                rw_buffer_entry(4),
                store_entry(5, R32Float),
                store_entry(6, Rg32Float),
                store_entry(7, R32Float),
            ],
        );
        let depth_clip = amd(
            "fsr2 depth clip",
            FSR2_DEPTH_CLIP_WGSL,
            "depth_clip",
            &[
                tex_entry(0),
                tex_entry(1),
                tex_entry(2),
                tex_entry(3),
                tex_entry(4),
                tex_entry(5),
                tex_entry(6),
                tex_entry(7),
                ro_buffer_entry(8),
                store_entry(9, Rgba16Float),
                store_entry(10, Rgba8Unorm),
            ],
        );
        let lock = amd(
            "fsr2 lock",
            FSR2_LOCK_WGSL,
            "compute_lock",
            &[tex_entry(0), rw_buffer_entry(1), rw_buffer_entry(2)],
        );
        let accumulate = amd(
            "fsr2 accumulate",
            FSR2_ACCUMULATE_WGSL,
            "accumulate",
            &[
                tex_entry(0),
                tex_entry(1),
                tex_entry(2),
                tex_entry(3),
                tex_entry(4),
                tex_entry(5),
                tex_entry(6),
                tex_entry(7),
                rw_buffer_entry(8),
                store_entry(9, Rg32Float),
                store_entry(10, Rgba16Float),
                store_entry(11, Rgba8Unorm),
                store_entry(12, R32Float),
            ],
        );
        let rcas = amd(
            "fsr2 rcas",
            FSR2_RCAS_WGSL,
            "rcas",
            &[
                tex_entry(0),
                tex_entry(1),
                store_entry(2, Rgba8Unorm),
                tex_entry(3),
            ],
        );
        let buffer = |label, size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        Self {
            cb_layout,
            input,
            spd_mips,
            spd_tail,
            reconstruct,
            depth_clip,
            lock,
            accumulate,
            rcas,
            tone,
            dispatched: std::sync::Mutex::new(Vec::new()),
            cb: buffer("fsr2 constants", size_of::<Fsr2Constants>() as u64),
            input_tuning: buffer("fsr2 input tuning", size_of::<InputUniform>() as u64),
            res: Resources::new(device, render, display, false),
            settings,
            bypass: false,
            debug: false,
            pre_exposure: std::sync::Mutex::new(1.0),
        }
    }
}
