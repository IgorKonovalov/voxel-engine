//! The frame's passes: one table ([`PASSES`]) names each pass's modules, entry point,
//! constants and bind groups, and [`Pipelines`] builds their layouts and pipelines from it.

use std::sync::OnceLock;

use super::*;

/// A bind group of a pass, at its index in [`PassSpec::groups`].
#[derive(Clone, Copy)]
pub(crate) enum Group {
    /// The world's group: the frame's parameters, the mirror and the lights.
    World,
    /// An index the pass does not bind.
    Empty,
    /// The pass's own bindings, numbered from 0.
    Own(&'static [Io]),
    /// The pass's own bindings at explicit binding numbers.
    OwnAt(&'static [(u32, Io)]),
    /// The layout of the pass at that index of [`PASSES`].
    SharedWith(usize),
    /// The probe volume's buffers and uniform.
    Probes,
}

/// One frame pass: its label, the WGSL modules its source joins in order, its entry point, its
/// pipeline-overridable constants, its bind groups from index 0, and whether its pipeline is
/// built on its first use rather than with the frame.
pub(crate) struct PassSpec {
    pub(crate) label: &'static str,
    pub(crate) modules: &'static [&'static str],
    pub(crate) entry: &'static str,
    pub(crate) constants: &'static [(&'static str, f64)],
    pub(crate) groups: &'static [Group],
    pub(crate) lazy: bool,
}

impl PassSpec {
    /// The pass's source: the prelude and its modules ([`compose`]).
    pub(crate) fn source(&self) -> String {
        compose(self.modules)
    }
}

const F16: Io = Io::Store(wgpu::TextureFormat::Rgba16Float);
const W: Group = Group::World;
const E: Group = Group::Empty;

pub(super) const P_TEMPORAL: usize = 0;
pub(super) const P_TEMPORAL_COUNTING: usize = 1;
pub(super) const P_LIGHT: usize = 2;
pub(super) const P_GRADIENT: usize = 3;
pub(super) const P_DOWNSAMPLE: usize = 4;
pub(super) const P_VARIANCE: usize = 5;
pub(super) const P_ATROUS: usize = 6;
pub(super) const P_UPSCALE: usize = 7;
pub(super) const P_SHARPEN: usize = 8;
pub(super) const P_COMPOSITE: usize = 9;
pub(super) const P_GATHER: usize = 10;
pub(super) const P_PROBE_TRACE: usize = 11;
pub(super) const P_PROBE_BLEND: usize = 12;

/// The probe update's modules: its trace and blend share one source.
const PROBE_UPDATE: &[&str] = &[
    COMMON_WGSL,
    MIRROR_WGSL,
    WALK_WGSL,
    HITREC_WGSL,
    LIGHT_WGSL,
    EMISSIVE_WGSL,
    PROBE_GATHER_WGSL,
    PROBE_UPDATE_WGSL,
];

/// Every frame pass but the hit pass ([`Marcher`]) and the composite's FSR 2 variant
/// ([`composite_hdr_source`]), indexed by the `P_*` constants.
pub(crate) const PASSES: [PassSpec; 13] = [
    PassSpec {
        label: "temporal",
        modules: &[
            COMMON_WGSL,
            MIRROR_WGSL,
            WALK_WGSL,
            HITREC_WGSL,
            EDITS_WGSL,
            TEMPORAL_WGSL,
        ],
        entry: "temporal",
        constants: &[],
        groups: &[
            W,
            E,
            Group::Own(&[
                Io::Float,
                Io::Uint,
                Io::Float,
                Io::Uint,
                Io::Float,
                Io::Float,
                Io::Float,
                F16,
                F16,
                Io::Uniform,
                Io::Float,
                Io::ReadStorage,
                Io::Storage,
            ]),
        ],
        lazy: false,
    },
    // The temporal pass with `COUNT_STORE` on, built on the first encode that counts
    // ([`Frame::set_count_store`]).
    PassSpec {
        label: "temporal counting",
        modules: &[
            COMMON_WGSL,
            MIRROR_WGSL,
            WALK_WGSL,
            HITREC_WGSL,
            EDITS_WGSL,
            TEMPORAL_WGSL,
        ],
        entry: "temporal",
        constants: &[("COUNT_STORE", 1.0)],
        groups: &[W, E, Group::SharedWith(P_TEMPORAL)],
        lazy: true,
    },
    // The light pass (`shade`), built on the first encode that runs it.
    PassSpec {
        label: "light",
        modules: &[
            COMMON_WGSL,
            MIRROR_WGSL,
            WALK_WGSL,
            HITREC_WGSL,
            LIGHT_WGSL,
            EMISSIVE_WGSL,
        ],
        entry: "shade",
        constants: &[],
        groups: &[W, E, Group::Own(&[Io::Float, Io::Uint, F16])],
        lazy: true,
    },
    PassSpec {
        label: "gradient",
        modules: &[
            COMMON_WGSL,
            MIRROR_WGSL,
            WALK_WGSL,
            HITREC_WGSL,
            LIGHT_WGSL,
            EMISSIVE_WGSL,
            GRADIENT_WGSL,
        ],
        entry: "gradient",
        constants: &[],
        groups: &[W, E, E, Group::Own(&[Io::Float, Io::Uint, Io::Float, F16])],
        lazy: false,
    },
    PassSpec {
        label: "downsample",
        modules: &[COMMON_WGSL, HITREC_WGSL, COMPOSITE_WGSL],
        entry: "downsample",
        constants: &[],
        groups: &[
            W,
            E,
            E,
            Group::Own(&[
                Io::Float,
                Io::Uint,
                Io::Store(wgpu::TextureFormat::Rgba32Float),
                Io::Store(wgpu::TextureFormat::Rgba16Uint),
            ]),
        ],
        lazy: false,
    },
    PassSpec {
        label: "variance",
        modules: &[COMMON_WGSL, HITREC_WGSL, ATROUS_WGSL],
        entry: "variance",
        constants: &[],
        groups: &[
            W,
            E,
            Group::Own(&[Io::Float, Io::Uint, Io::Float, Io::Float, F16]),
        ],
        lazy: false,
    },
    PassSpec {
        label: "atrous",
        modules: &[COMMON_WGSL, HITREC_WGSL, ATROUS_WGSL],
        entry: "atrous",
        constants: &[],
        groups: &[
            W,
            E,
            Group::OwnAt(&[
                (0, Io::Float),
                (1, Io::Uint),
                (2, Io::Float),
                (5, F16),
                (6, Io::Uniform),
                (7, Io::Float),
                (8, Io::Float),
            ]),
        ],
        lazy: false,
    },
    PassSpec {
        label: "upscale",
        modules: &[COMMON_WGSL, HITREC_WGSL, EDITS_WGSL, UPSCALE_WGSL],
        entry: "upscale",
        constants: &[],
        groups: &[
            W,
            E,
            Group::Own(&[
                Io::Float,
                Io::Uint,
                Io::Float,
                Io::Float,
                Io::Uint,
                Io::Float,
                F16,
                Io::Uniform,
                Io::Uniform,
            ]),
        ],
        lazy: false,
    },
    PassSpec {
        label: "sharpen",
        modules: &[COMMON_WGSL, SHARPEN_WGSL],
        entry: "sharpen_main",
        constants: &[],
        groups: &[
            W,
            E,
            E,
            Group::Own(&[
                Io::Float,
                Io::Store(wgpu::TextureFormat::Rgba8Unorm),
                Io::Uniform,
            ]),
        ],
        lazy: false,
    },
    PassSpec {
        label: "composite",
        modules: &[COMMON_WGSL, HITREC_WGSL, COMPOSITE_WGSL],
        entry: "composite",
        constants: &[],
        groups: &[
            W,
            E,
            Group::Own(&[
                Io::Float,
                Io::Uint,
                Io::Float,
                Io::Float,
                Io::Store(wgpu::TextureFormat::Rgba8Unorm),
                Io::Float,
                Io::Uint,
                Io::Float,
                Io::Storage,
                Io::Uint,
            ]),
        ],
        lazy: false,
    },
    PassSpec {
        label: "gather",
        modules: &[COMMON_WGSL, HITREC_WGSL, PROBE_GATHER_WGSL],
        entry: "gather",
        constants: &[],
        groups: &[W, E, Group::Own(&[Io::Float, Io::Uint, F16]), Group::Probes],
        lazy: false,
    },
    // The probe update's two dispatches (`probe_update.wgsl`).
    PassSpec {
        label: "probe_trace",
        modules: PROBE_UPDATE,
        entry: "probe_trace",
        constants: &[],
        groups: &[W, E, E, Group::Probes],
        lazy: false,
    },
    PassSpec {
        label: "probe_blend",
        modules: PROBE_UPDATE,
        entry: "probe_blend",
        constants: &[],
        groups: &[W, E, E, Group::Probes],
        lazy: false,
    },
];

/// Every frame pass's layout and pipeline, built from [`PASSES`], with the world's and the
/// probe volume's group layouts and the hit pass.
pub(super) struct Pipelines {
    pub(super) world_layout: wgpu::BindGroupLayout,
    pub(super) hit: Marcher,
    /// The probe volume's group-3 layout, which the probe passes and the gather bind.
    pub(super) probe_layout: wgpu::BindGroupLayout,
    /// Each pass's own group layout, `None` for a pass with no group of its own.
    layouts: [Option<wgpu::BindGroupLayout>; PASSES.len()],
    /// Each pass's pipeline, built with the frame unless the pass is lazy.
    built: [OnceLock<wgpu::ComputePipeline>; PASSES.len()],
}

impl Pipelines {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let world_layout = world_layout(device);
        let hit = Marcher::new(device, &world_layout);
        let layouts = PASSES.each_ref().map(|spec| {
            spec.groups.iter().find_map(|g| match *g {
                Group::Own(io) => Some(io_layout(device, spec.label, io)),
                Group::OwnAt(io) => Some(io_layout_at(device, spec.label, io)),
                _ => None,
            })
        });
        let pipes = Self {
            world_layout,
            hit,
            probe_layout: probe_layout(device),
            layouts,
            built: std::array::from_fn(|_| OnceLock::new()),
        };
        for (p, spec) in PASSES.iter().enumerate() {
            if !spec.lazy {
                pipes.pipeline(device, p);
            }
        }
        pipes
    }

    /// Pass `p`'s own group layout, or the layout it shares.
    pub(super) fn layout(&self, p: usize) -> &wgpu::BindGroupLayout {
        let at = PASSES[p]
            .groups
            .iter()
            .find_map(|g| match *g {
                Group::SharedWith(q) => Some(q),
                _ => None,
            })
            .unwrap_or(p);
        self.layouts[at]
            .as_ref()
            .expect("PASSES gives every pass whose layout is read a group of its own")
    }

    /// Pass `p`'s pipeline, built on its first use.
    pub(super) fn pipeline(&self, device: &wgpu::Device, p: usize) -> &wgpu::ComputePipeline {
        self.built[p].get_or_init(|| {
            let spec = &PASSES[p];
            let layouts: Vec<Option<&wgpu::BindGroupLayout>> = spec
                .groups
                .iter()
                .map(|g| match g {
                    Group::World => Some(&self.world_layout),
                    Group::Empty => None,
                    Group::Own(_) | Group::OwnAt(_) | Group::SharedWith(_) => Some(self.layout(p)),
                    Group::Probes => Some(&self.probe_layout),
                })
                .collect();
            compute_pipeline_with(
                device,
                spec.label,
                spec.source(),
                &layouts,
                spec.entry,
                spec.constants,
            )
        })
    }
}

/// The world's group 0, as `common.wgsl` declares it: the frame's parameters (0), the mirror's
/// pools, records, table and materials, the light samples and the emission scales, the blue
/// noise (12) and the exposure state (13).
fn world_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let storage = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("world"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
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
            storage(4),
            storage(7),
            storage(8),
            storage(9),
            storage(10),
            storage(11),
            sampled_texture(12, FLOAT),
            wgpu::BindGroupLayoutEntry {
                binding: 13,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    })
}

/// The probe volume's group 3: its data and scratch buffers (read-write) and its uniform.
fn probe_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let probe_storage = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("probes"),
        entries: &[
            probe_storage(0),
            probe_storage(1),
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    })
}
