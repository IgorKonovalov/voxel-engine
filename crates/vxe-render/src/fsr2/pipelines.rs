//! FSR 2's pipelines and their layouts, and the bind groups and dispatches of one frame.

use super::*;
use crate::frame::{COMMON_WGSL, EDITS_WGSL, HITREC_WGSL, compose};

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

/// Which groups an FSR 2 pass binds: ours read the frame's world at group 0 and the pass's at
/// 2, composed after the prelude and the frame's modules it names ([`compose`]); AMD's read the
/// constants at 0 and the pass's at 1, after FSR 2's common parts ([`fsr2_source`]).
#[derive(Clone, Copy)]
pub(crate) enum Side {
    Ours(&'static [&'static str]),
    Amd,
}

/// One FSR 2 pass: its label, its WGSL, its entry point, whose groups it binds, and its own
/// bindings.
pub(crate) struct FsrPassSpec {
    pub(crate) label: &'static str,
    pub(crate) source: &'static str,
    pub(crate) entry: &'static str,
    pub(crate) side: Side,
    pub(crate) io: &'static [(u32, Io)],
}

impl FsrPassSpec {
    /// The pass's composed source.
    pub(crate) fn composed(&self) -> String {
        match self.side {
            Side::Ours(modules) => {
                let mut all = modules.to_vec();
                all.push(self.source);
                compose(&all)
            }
            Side::Amd => fsr2_source(self.source, true),
        }
    }
}

const R32F: Io = Io::Store(wgpu::TextureFormat::R32Float);
const RG32F: Io = Io::Store(wgpu::TextureFormat::Rg32Float);
const RGBA8: Io = Io::Store(wgpu::TextureFormat::Rgba8Unorm);
const RGBA16F: Io = Io::Store(wgpu::TextureFormat::Rgba16Float);

/// FSR 2's passes in the order [`Fsr2::new`] builds them: our input and tone passes, then AMD's
/// luminance pyramid (its mips and its tail), reconstruct previous depth, depth clip, lock,
/// accumulate and RCAS.
pub(crate) const FSR2_PIPES: [FsrPassSpec; 9] = [
    FsrPassSpec {
        label: "fsr2 input",
        source: FSR2_INPUT_WGSL,
        entry: "fsr2_input",
        side: Side::Ours(&[COMMON_WGSL, HITREC_WGSL, EDITS_WGSL]),
        io: &[
            (0, Io::Float),
            (1, Io::Uint),
            (2, Io::Float),
            (3, Io::Uniform),
            (4, Io::Uniform),
            (5, Io::Uniform),
            (6, R32F),
            (7, RG32F),
            (8, RGBA8),
        ],
    },
    FsrPassSpec {
        label: "fsr2 tone",
        source: FSR2_TONE_WGSL,
        entry: "tone",
        side: Side::Ours(&[COMMON_WGSL]),
        io: &[(0, Io::Float), (1, RGBA8)],
    },
    FsrPassSpec {
        label: "fsr2 luminance pyramid",
        source: FSR2_LUMINANCE_PYRAMID_WGSL,
        entry: "spd_mips",
        side: Side::Amd,
        io: &[(0, Io::Float), (1, R32F), (2, R32F)],
    },
    FsrPassSpec {
        label: "fsr2 luminance pyramid tail",
        source: FSR2_LUMINANCE_PYRAMID_WGSL,
        entry: "spd_tail",
        side: Side::Amd,
        io: &[(3, Io::Float), (4, RG32F), (5, Io::Float)],
    },
    FsrPassSpec {
        label: "fsr2 reconstruct previous depth",
        source: FSR2_RECONSTRUCT_WGSL,
        entry: "reconstruct_and_dilate",
        side: Side::Amd,
        io: &[
            (0, Io::Float),
            (1, Io::Float),
            (2, Io::Float),
            (3, Io::Float),
            (4, Io::Storage),
            (5, R32F),
            (6, RG32F),
            (7, R32F),
        ],
    },
    FsrPassSpec {
        label: "fsr2 depth clip",
        source: FSR2_DEPTH_CLIP_WGSL,
        entry: "depth_clip",
        side: Side::Amd,
        io: &[
            (0, Io::Float),
            (1, Io::Float),
            (2, Io::Float),
            (3, Io::Float),
            (4, Io::Float),
            (5, Io::Float),
            (6, Io::Float),
            (7, Io::Float),
            (8, Io::ReadStorage),
            (9, RGBA16F),
            (10, RGBA8),
        ],
    },
    FsrPassSpec {
        label: "fsr2 lock",
        source: FSR2_LOCK_WGSL,
        entry: "compute_lock",
        side: Side::Amd,
        io: &[(0, Io::Float), (1, Io::Storage), (2, Io::Storage)],
    },
    FsrPassSpec {
        label: "fsr2 accumulate",
        source: FSR2_ACCUMULATE_WGSL,
        entry: "accumulate",
        side: Side::Amd,
        io: &[
            (0, Io::Float),
            (1, Io::Float),
            (2, Io::Float),
            (3, Io::Float),
            (4, Io::Float),
            (5, Io::Float),
            (6, Io::Float),
            (7, Io::Float),
            (8, Io::Storage),
            (9, RG32F),
            (10, RGBA16F),
            (11, RGBA8),
            (12, R32F),
        ],
    },
    FsrPassSpec {
        label: "fsr2 rcas",
        source: FSR2_RCAS_WGSL,
        entry: "rcas",
        side: Side::Amd,
        io: &[(0, Io::Float), (1, Io::Float), (2, RGBA8), (3, Io::Float)],
    },
];

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
        let cb_layout = io_layout(device, "fsr2 constants", &[Io::Uniform]);
        let [
            input,
            tone,
            spd_mips,
            spd_tail,
            reconstruct,
            depth_clip,
            lock,
            accumulate,
            rcas,
        ] = FSR2_PIPES.each_ref().map(|spec| {
            let layout = io_layout_at(device, spec.label, spec.io);
            let groups = match spec.side {
                Side::Ours(_) => vec![Some(world_layout), None, Some(&layout)],
                Side::Amd => vec![Some(&cb_layout), Some(&layout)],
            };
            let pipeline =
                compute_pipeline(device, spec.label, spec.composed(), &groups, spec.entry);
            Pass { layout, pipeline }
        });
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
