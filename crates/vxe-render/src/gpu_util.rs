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

/// What a cached bind group binds, one per binding: the resource itself, so a replaced
/// texture or buffer misses.
enum Bound {
    Tex(wgpu::Texture),
    Buf(wgpu::Buffer),
}

impl Bound {
    fn is(&self, r: &Res<'_>) -> bool {
        match (self, r) {
            (Bound::Tex(a), Res::Tex(b)) => a == *b,
            (Bound::Buf(a), Res::Buf(b)) => a == *b,
            _ => false,
        }
    }
}

struct Cached {
    layout: wgpu::BindGroupLayout,
    at: Vec<u32>,
    bound: Vec<Bound>,
    group: wgpu::BindGroup,
    /// The [`BindCache::tick`] that last took it.
    used: u64,
}

#[derive(Default)]
struct CacheState {
    entries: Vec<Cached>,
    tick: u64,
    groups: u64,
    views: u64,
}

/// Bind groups kept with the layout and the resources they bind: a pass that binds the same
/// textures and buffers again takes its group back, and a group and its texture views are
/// created only on a miss. A group no [`BindCache::tick`] of the last [`BindCache::KEEP`] took
/// is dropped, so a replaced resource is not held past them, and a group per history parity
/// survives the frame that does not bind it.
#[derive(Default)]
pub(crate) struct BindCache {
    state: std::sync::Mutex<CacheState>,
}

impl BindCache {
    /// Ticks a group outlives its last use.
    pub(crate) const KEEP: u64 = 2;

    fn lock(&self) -> std::sync::MutexGuard<'_, CacheState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Starts a frame's use: drops the groups the last [`BindCache::KEEP`] ticks did not take.
    pub(crate) fn tick(&self) {
        let mut s = self.lock();
        s.tick += 1;
        let oldest = s.tick.saturating_sub(Self::KEEP);
        s.entries.retain(|e| e.used >= oldest);
    }

    /// Drops every group (the resources they bind were replaced).
    pub(crate) fn clear(&self) {
        self.lock().entries.clear();
    }

    /// Bind groups and texture views created so far.
    pub(crate) fn built(&self) -> (u64, u64) {
        let s = self.lock();
        (s.groups, s.views)
    }

    /// [`BindCache::io_at`] at the binding numbers `0, 1, ..` in turn.
    pub(crate) fn io(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        res: &[Res<'_>],
    ) -> wgpu::BindGroup {
        const AT: [u32; 32] = {
            let mut a = [0; 32];
            let mut i = 0;
            while i < 32 {
                a[i] = i as u32;
                i += 1;
            }
            a
        };
        self.io_at(device, layout, &AT[..res.len()], res)
    }

    /// [`bind_io_at`] through the cache.
    pub(crate) fn io_at(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        at: &[u32],
        res: &[Res<'_>],
    ) -> wgpu::BindGroup {
        let mut s = self.lock();
        let tick = s.tick;
        if let Some(e) = s.entries.iter_mut().find(|e| {
            e.layout == *layout
                && e.at == at
                && e.bound.len() == res.len()
                && e.bound.iter().zip(res).all(|(b, r)| b.is(r))
        }) {
            e.used = tick;
            return e.group.clone();
        }
        let group = bind_io_at(device, layout, at, res);
        s.groups += 1;
        s.views += res.iter().filter(|r| matches!(r, Res::Tex(_))).count() as u64;
        s.entries.push(Cached {
            layout: layout.clone(),
            at: at.to_vec(),
            bound: res
                .iter()
                .map(|r| match r {
                    Res::Tex(t) => Bound::Tex((*t).clone()),
                    Res::Buf(b) => Bound::Buf((*b).clone()),
                })
                .collect(),
            group: group.clone(),
            used: tick,
        });
        group
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

/// Compute pipelines [`compute_pipeline_with`] has created in this process (every compute
/// pipeline of the crate goes through it).
static COMPUTE_PIPELINES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The count of compute pipelines this process has created, for a reading or a test that a
/// stretch of frames builds none.
pub fn compute_pipelines_created() -> u64 {
    COMPUTE_PIPELINES.load(std::sync::atomic::Ordering::Relaxed)
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
    COMPUTE_PIPELINES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
