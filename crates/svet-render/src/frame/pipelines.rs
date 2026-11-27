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
/// pipeline-overridable constants, its bind groups from index 0, whether its source keeps the
/// walk's default-off arms ([`compose_arms`]), and whether its pipeline is built on its first
/// use rather than with the frame.
pub(crate) struct PassSpec {
    pub(crate) label: &'static str,
    pub(crate) modules: &'static [&'static str],
    pub(crate) entry: &'static str,
    pub(crate) constants: &'static [(&'static str, f64)],
    pub(crate) groups: &'static [Group],
    pub(crate) arms: bool,
    pub(crate) lazy: bool,
}

impl PassSpec {
    /// The pass's source: the prelude and its modules ([`compose`], or [`compose_arms`] for a
    /// pass that keeps the walk's default-off arms).
    pub(crate) fn source(&self) -> String {
        if self.arms {
            compose_arms(self.modules)
        } else {
            compose(self.modules)
        }
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
pub(super) const P_COMPOSITE: usize = 7;
pub(super) const P_GATHER: usize = 8;
pub(super) const P_PROBE_TRACE: usize = 9;
pub(super) const P_PROBE_BLEND: usize = 10;
pub(super) const P_LIGHT_BARE: usize = 11;
pub(super) const P_LIGHT_STEPS: usize = 12;
pub(super) const P_LIGHT_ARMS: usize = 13;
pub(super) const P_LIGHT_BARE_ARMS: usize = 14;

/// The probe update's modules: its trace and blend share one source.
const PROBE_UPDATE: &[&str] = &[
    COMMON_WGSL,
    MIRROR_WGSL,
    WALK_WGSL,
    HITREC_WGSL,
    ENTITY_WGSL,
    LIGHT_WGSL,
    EMISSIVE_WGSL,
    PROBE_GATHER_WGSL,
    PROBE_UPDATE_WGSL,
];

/// The light pass's modules, in each of its variants: the sky's join it alone of the passes
/// that compile `light.wgsl`, so the bake, the probe and the gradient passes take no sky term.
const LIGHT_MODULES: &[&str] = &[
    COMMON_WGSL,
    MIRROR_WGSL,
    WALK_WGSL,
    HITREC_WGSL,
    ENTITY_WGSL,
    LIGHT_WGSL,
    EMISSIVE_WGSL,
    SKY_ATMOSPHERE_WGSL,
    SKY_WGSL,
    SKY_LIGHT_WGSL,
    KEY_HORIZON_WGSL,
    SHADE_WGSL,
];

/// Every frame pass but the hit pass ([`Marcher`]) and the composite's FSR 2 variant
/// ([`composite_hdr_source`]), indexed by the `P_*` constants.
pub(crate) const PASSES: [PassSpec; 15] = [
    PassSpec {
        label: "temporal",
        modules: &[
            COMMON_WGSL,
            MIRROR_WGSL,
            WALK_WGSL,
            HITREC_WGSL,
            ENTITY_WGSL,
            EDITS_WGSL,
            SKY_ATMOSPHERE_WGSL,
            SKY_WGSL,
            PAIR_WGSL,
            KEY_HORIZON_WGSL,
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
                Io::Float,
                Io::Float,
                F16,
                Io::ReadStorage,
                Io::Float,
                Io::Float,
                F16,
            ]),
        ],
        arms: false,
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
            ENTITY_WGSL,
            EDITS_WGSL,
            SKY_ATMOSPHERE_WGSL,
            SKY_WGSL,
            PAIR_WGSL,
            KEY_HORIZON_WGSL,
            TEMPORAL_WGSL,
        ],
        entry: "temporal",
        constants: &[("COUNT_STORE", 1.0)],
        groups: &[W, E, Group::SharedWith(P_TEMPORAL)],
        arms: false,
        lazy: true,
    },
    // The light pass (`shade`) that walks the entities, built by
    // [`Pipelines::prepare_entities`] or else on the first encode that runs it.
    PassSpec {
        label: "light",
        modules: LIGHT_MODULES,
        entry: "shade",
        constants: &[("ENTITY_SHADOWS", 1.0)],
        groups: &[
            W,
            E,
            Group::Own(&[
                Io::Float,
                Io::Uint,
                F16,
                F16,
                Io::ReadStorage,
                Io::Store(wgpu::TextureFormat::Rgba8Unorm),
            ]),
        ],
        arms: false,
        lazy: true,
    },
    PassSpec {
        label: "gradient",
        modules: &[
            COMMON_WGSL,
            MIRROR_WGSL,
            WALK_WGSL,
            HITREC_WGSL,
            ENTITY_WGSL,
            LIGHT_WGSL,
            EMISSIVE_WGSL,
            GRADIENT_WGSL,
        ],
        entry: "gradient",
        constants: &[("ENTITY_SHADOWS", 1.0)],
        groups: &[W, E, E, Group::Own(&[Io::Float, Io::Uint, Io::Float, F16])],
        arms: false,
        lazy: false,
    },
    PassSpec {
        label: "downsample",
        modules: &[
            COMMON_WGSL,
            HITREC_WGSL,
            SKY_ATMOSPHERE_WGSL,
            SKY_WGSL,
            COMPOSITE_WGSL,
        ],
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
        arms: false,
        lazy: false,
    },
    PassSpec {
        label: "variance",
        modules: &[
            COMMON_WGSL,
            HITREC_WGSL,
            SKY_ATMOSPHERE_WGSL,
            SKY_WGSL,
            ATROUS_WGSL,
        ],
        entry: "variance",
        constants: &[],
        groups: &[
            W,
            E,
            Group::OwnAt(&[
                (0, Io::Float),
                (1, Io::Uint),
                (2, Io::Float),
                (3, Io::Float),
                (4, F16),
                (9, Io::Float),
                (10, Io::Float),
                (11, Io::Float),
                (12, Io::ReadStorage),
            ]),
        ],
        arms: false,
        lazy: false,
    },
    PassSpec {
        label: "atrous",
        modules: &[
            COMMON_WGSL,
            HITREC_WGSL,
            SKY_ATMOSPHERE_WGSL,
            SKY_WGSL,
            ATROUS_WGSL,
        ],
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
                (10, Io::Float),
            ]),
        ],
        arms: false,
        lazy: false,
    },
    PassSpec {
        label: "composite",
        modules: &[
            COMMON_WGSL,
            HITREC_WGSL,
            SKY_ATMOSPHERE_WGSL,
            SKY_WGSL,
            COMPOSITE_WGSL,
        ],
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
                Io::Float,
                Io::Float,
                Io::Float,
                Io::ReadStorage,
                Io::ReadStorage,
            ]),
        ],
        arms: false,
        lazy: false,
    },
    PassSpec {
        label: "gather",
        modules: &[COMMON_WGSL, HITREC_WGSL, PROBE_GATHER_WGSL],
        entry: "gather",
        constants: &[],
        groups: &[W, E, Group::Own(&[Io::Float, Io::Uint, F16]), Group::Probes],
        arms: false,
        lazy: false,
    },
    // The probe update's two dispatches (`probe_update.wgsl`).
    PassSpec {
        label: "probe_trace",
        modules: PROBE_UPDATE,
        entry: "probe_trace",
        constants: &[],
        groups: &[W, E, E, Group::Probes],
        arms: false,
        lazy: false,
    },
    PassSpec {
        label: "probe_blend",
        modules: PROBE_UPDATE,
        entry: "probe_blend",
        constants: &[],
        groups: &[W, E, E, Group::Probes],
        arms: false,
        lazy: false,
    },
    // The light pass with `ENTITIES` false (`entity.wgsl`): the frame's light pass while no
    // entity part is drawn ([`Frame::light_pipeline`]), built on its first use or by
    // [`Pipelines::prepare_entities`].
    PassSpec {
        label: "light (no entities)",
        modules: LIGHT_MODULES,
        entry: "shade",
        constants: &[("ENTITIES", 0.0)],
        groups: &[W, E, Group::SharedWith(P_LIGHT)],
        arms: false,
        lazy: true,
    },
    // The light pass with `COUNT_STEPS` on, for the steps view, built on its first use.
    PassSpec {
        label: "light counting",
        modules: LIGHT_MODULES,
        entry: "shade",
        constants: &[("COUNT_STEPS", 1.0)],
        groups: &[W, E, Group::SharedWith(P_LIGHT)],
        arms: true,
        lazy: true,
    },
    // The light pass with the walk's default-off arm (the absorbing step: [`compose_arms`]),
    // built when it is picked ([`Frame::light_pipeline`]).
    PassSpec {
        label: "light (arms)",
        modules: LIGHT_MODULES,
        entry: "shade",
        constants: &[("ENTITY_SHADOWS", 1.0)],
        groups: &[W, E, Group::SharedWith(P_LIGHT)],
        arms: true,
        lazy: true,
    },
    PassSpec {
        label: "light (no entities, arms)",
        modules: LIGHT_MODULES,
        entry: "shade",
        constants: &[("ENTITIES", 0.0)],
        groups: &[W, E, Group::SharedWith(P_LIGHT)],
        arms: true,
        lazy: true,
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
    /// The threads a start-up build ([`Pipelines::new`], [`Pipelines::prepare_entities`],
    /// [`Pipelines::prepare_light_arms`]) creates its pipelines on.
    pub(super) threads: usize,
    /// Each pipeline a start-up build created, by label, with its wall time in ms, in the
    /// order of the build's jobs.
    times: std::sync::Mutex<Vec<(&'static str, f64)>>,
}

/// The most threads a start-up build takes.
pub(super) const MAX_BUILD_THREADS: usize = 8;

/// One pipeline of a start-up build.
#[derive(Clone, Copy)]
enum Job {
    /// The hit pass's pipeline, with `ENTITIES` true or false.
    Hit(HitPipe, bool),
    /// Pass `p` of [`PASSES`].
    Pass(usize),
}

impl Job {
    fn label(self) -> &'static str {
        match self {
            Self::Hit(which, false) => which.label(),
            Self::Hit(which, true) => which.entity_label(),
            Self::Pass(p) => PASSES[p].label,
        }
    }
}

impl Pipelines {
    /// The layouts and every pipeline but the lazy passes', created on `threads` scoped
    /// threads (clamped to 1..=[`MAX_BUILD_THREADS`]; 1 builds on the calling thread).
    pub(super) fn new(device: &wgpu::Device, threads: usize) -> Self {
        let threads = threads.clamp(1, MAX_BUILD_THREADS);
        let world_layout = world_layout(device);
        let hit_layouts = Marcher::layouts(device);
        let layouts = PASSES.each_ref().map(|spec| {
            spec.groups.iter().find_map(|g| match *g {
                Group::Own(io) => Some(io_layout(device, spec.label, io)),
                Group::OwnAt(io) => Some(io_layout_at(device, spec.label, io)),
                _ => None,
            })
        });
        let probe_layout = probe_layout(device);
        let built: [OnceLock<wgpu::ComputePipeline>; PASSES.len()] =
            std::array::from_fn(|_| OnceLock::new());
        let bare: [OnceLock<wgpu::ComputePipeline>; 3] = std::array::from_fn(|_| OnceLock::new());
        let jobs: Vec<Job> = HitPipe::ALL
            .into_iter()
            .map(|which| Job::Hit(which, false))
            .chain(
                (0..PASSES.len())
                    .filter(|&p| !PASSES[p].lazy)
                    .map(Job::Pass),
            )
            .collect();
        let ms = run_jobs(threads, jobs.len(), |j| match jobs[j] {
            Job::Hit(which, entities) => {
                let [out, resume] = &hit_layouts;
                let pipe =
                    Marcher::build_one(device, &world_layout, [out, resume], which, entities);
                let _ = bare[which as usize].set(pipe);
            }
            Job::Pass(p) => {
                built[p]
                    .get_or_init(|| build_pass(device, p, &world_layout, &probe_layout, &layouts));
            }
        });
        let bare = bare.map(|b| b.into_inner().expect("every hit job ran"));
        Self {
            world_layout,
            hit: Marcher::from_parts(hit_layouts, bare),
            probe_layout,
            layouts,
            built,
            threads,
            times: std::sync::Mutex::new(labelled(&jobs, &ms)),
        }
    }

    /// Builds the hit pass's `ENTITIES`-true set and both light pipelines (`P_LIGHT` and
    /// `P_LIGHT_BARE`) unless they are built, on [`Pipelines::threads`] threads, so no later
    /// frame, with entities or without, builds a pipeline outside the steps view and the
    /// store counters.
    pub(super) fn prepare_entities(&self, device: &wgpu::Device) {
        let set: [OnceLock<wgpu::ComputePipeline>; 3] = std::array::from_fn(|_| OnceLock::new());
        let hits = if self.hit.entities_built() {
            Vec::new()
        } else {
            HitPipe::ALL.map(|which| Job::Hit(which, true)).to_vec()
        };
        let jobs: Vec<Job> = hits
            .into_iter()
            .chain(self.unbuilt([P_LIGHT, P_LIGHT_BARE]))
            .collect();
        let ms = run_jobs(self.threads, jobs.len(), |j| match jobs[j] {
            Job::Hit(which, entities) => {
                let layouts = [&self.hit.out_layout, &self.hit.resume_layout];
                let pipe = Marcher::build_one(device, &self.world_layout, layouts, which, entities);
                let _ = set[which as usize].set(pipe);
            }
            Job::Pass(p) => {
                self.pipeline(device, p);
            }
        });
        if let [Some(a), Some(b), Some(c)] = set.map(OnceLock::into_inner) {
            self.hit.set_entities([a, b, c]);
        }
        self.note(&jobs, &ms);
    }

    /// Builds the light pass's variants with the default-off arms compiled in
    /// (`P_LIGHT_ARMS` and `P_LIGHT_BARE_ARMS`) unless they are built, on
    /// [`Pipelines::threads`] threads.
    pub(super) fn prepare_light_arms(&self, device: &wgpu::Device) {
        let jobs: Vec<Job> = self.unbuilt([P_LIGHT_ARMS, P_LIGHT_BARE_ARMS]).collect();
        let ms = run_jobs(self.threads, jobs.len(), |j| {
            if let Job::Pass(p) = jobs[j] {
                self.pipeline(device, p);
            }
        });
        self.note(&jobs, &ms);
    }

    /// The passes of `passes` whose pipelines are not built, as jobs.
    fn unbuilt<const N: usize>(&self, passes: [usize; N]) -> impl Iterator<Item = Job> + '_ {
        passes
            .into_iter()
            .filter(|&p| self.built[p].get().is_none())
            .map(Job::Pass)
    }

    /// Appends the jobs' times to [`Pipelines::times`].
    fn note(&self, jobs: &[Job], ms: &[f64]) {
        self.times
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend(labelled(jobs, ms));
    }

    /// Every pipeline the start-up builds have created so far, by label, with its wall time
    /// in ms, in build order.
    pub(super) fn build_times(&self) -> Vec<(&'static str, f64)> {
        self.times
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Pass `p`'s own group layout, or the layout it shares.
    pub(super) fn layout(&self, p: usize) -> &wgpu::BindGroupLayout {
        pass_layout(&self.layouts, p)
    }

    /// Pass `p`'s pipeline, built on its first use.
    pub(super) fn pipeline(&self, device: &wgpu::Device, p: usize) -> &wgpu::ComputePipeline {
        self.built[p].get_or_init(|| {
            build_pass(
                device,
                p,
                &self.world_layout,
                &self.probe_layout,
                &self.layouts,
            )
        })
    }
}

/// Each job's label beside its time.
fn labelled(jobs: &[Job], ms: &[f64]) -> Vec<(&'static str, f64)> {
    jobs.iter()
        .map(|j| j.label())
        .zip(ms.iter().copied())
        .collect()
}

/// Pass `p`'s own group layout of `layouts`, or the layout it shares.
fn pass_layout(
    layouts: &[Option<wgpu::BindGroupLayout>; PASSES.len()],
    p: usize,
) -> &wgpu::BindGroupLayout {
    let at = PASSES[p]
        .groups
        .iter()
        .find_map(|g| match *g {
            Group::SharedWith(q) => Some(q),
            _ => None,
        })
        .unwrap_or(p);
    layouts[at]
        .as_ref()
        .expect("PASSES gives every pass whose layout is read a group of its own")
}

/// Pass `p`'s pipeline over the world's, the probe volume's and the passes' layouts.
fn build_pass(
    device: &wgpu::Device,
    p: usize,
    world_layout: &wgpu::BindGroupLayout,
    probe_layout: &wgpu::BindGroupLayout,
    layouts: &[Option<wgpu::BindGroupLayout>; PASSES.len()],
) -> wgpu::ComputePipeline {
    let spec = &PASSES[p];
    let groups: Vec<Option<&wgpu::BindGroupLayout>> = spec
        .groups
        .iter()
        .map(|g| match g {
            Group::World => Some(world_layout),
            Group::Empty => None,
            Group::Own(_) | Group::OwnAt(_) | Group::SharedWith(_) => Some(pass_layout(layouts, p)),
            Group::Probes => Some(probe_layout),
        })
        .collect();
    compute_pipeline_with(
        device,
        spec.label,
        spec.source(),
        &groups,
        spec.entry,
        spec.constants,
    )
}

/// Runs `job(i)` once for every `i` in `0..count`: in index order on the calling thread when
/// `threads` is 1 (or there is one job), else on `min(threads, count)` scoped threads, each
/// taking the next unclaimed index until none is left. Returns each job's wall time in ms, by
/// index. The threads end with the call; a job's panic resumes on the caller.
fn run_jobs(threads: usize, count: usize, job: impl Fn(usize) + Sync) -> Vec<f64> {
    let ms: Vec<OnceLock<f64>> = (0..count).map(|_| OnceLock::new()).collect();
    let timed = |i: usize| {
        let t = stopwatch();
        job(i);
        let _ = ms[i].set(ms_since(t));
    };
    if threads <= 1 || count <= 1 {
        (0..count).for_each(timed);
    } else {
        let next = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|s| {
            for w in 0..threads.min(count) {
                std::thread::Builder::new()
                    .name(format!("pipelines {w}"))
                    .spawn_scoped(s, || {
                        loop {
                            let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            if i >= count {
                                break;
                            }
                            timed(i);
                        }
                    })
                    .expect("a pipeline build thread starts");
            }
        });
    }
    ms.into_iter()
        .map(|m| m.into_inner().unwrap_or(0.0))
        .collect()
}

#[allow(
    clippy::disallowed_methods,
    reason = "a start-up build reports each pipeline's wall time, a reading; nothing branches on it"
)]
fn stopwatch() -> std::time::Instant {
    std::time::Instant::now()
}

#[allow(
    clippy::disallowed_methods,
    reason = "a start-up build reports each pipeline's wall time, a reading; nothing branches on it"
)]
fn ms_since(t: std::time::Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

impl Frame {
    /// Group 0 of the frame's shaders over `world`.
    pub(super) fn world_bind(&self, device: &wgpu::Device, world: &GpuWorld) -> wgpu::BindGroup {
        self.binds.io_at(
            device,
            &self.pipes.world_layout,
            &[
                0, 1, 7, 8, 9, 2, 3, 4, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            ],
            &[
                Res::Buf(&self.state.params),
                Res::Buf(world.pool_shard(0)),
                Res::Buf(world.pool_shard(1)),
                Res::Buf(world.pool_shard(2)),
                Res::Buf(world.pool_shard(3)),
                Res::Buf(world.records()),
                Res::Buf(world.table()),
                Res::Buf(world.materials()),
                Res::Buf(world.light_buffer()),
                Res::Buf(&self.state.emission_scale),
                Res::Tex(&self.state.blue_noise),
                Res::Buf(self.exposure.state_uniform()),
                Res::Buf(&self.state.lights.buffer),
                Res::Buf(self.entities.buffer()),
                Res::Buf(&self.sky.uniform),
                Res::Tex(self.sky.bound()[0]),
                Res::Tex(self.sky.bound()[1]),
                Res::Tex(world.column_map()),
            ],
        )
    }
}

/// The world's group 0, as `common.wgsl` declares it: the frame's parameters (0), the mirror's
/// pools, records, table and materials, the light samples and the emission scales, the blue
/// noise (12), the exposure state (13), the list of lights (14), the entity mirror (15), and the
/// sky's uniform (16) and LUTs (17, 18) as `sky.wgsl` declares them, and the mirror's column map
/// (19).
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
            storage(15),
            sampled_texture(12, FLOAT),
            sampled_texture(17, FLOAT),
            sampled_texture(18, FLOAT),
            sampled_texture(19, wgpu::TextureSampleType::Sint),
            wgpu::BindGroupLayoutEntry {
                binding: 16,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
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
            wgpu::BindGroupLayoutEntry {
                binding: 14,
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
