//! The hit pass (`hit.wgsl`): its pipelines and its output layouts.

use std::sync::OnceLock;

use super::*;

/// The hit pass's three pipelines: `hit.wgsl`'s `main`, the same with `FIELD_SURFACE`, and
/// `field_resume`.
struct HitPipes {
    pipeline: wgpu::ComputePipeline,
    field: wgpu::ComputePipeline,
    resume: wgpu::ComputePipeline,
}

/// The hit pass: `hit.wgsl`'s `main`, one primary ray per pixel into the hit buffer, and in
/// the contour look over a mirror with the field ([`NormalMode::Contour`]) its pipeline with
/// `FIELD_SURFACE`, which walks the field surface, or with the split walk
/// ([`FLAG_FIELD_SPLIT`]) the plain pipeline and then `field_resume` over the rays it handed
/// on. Each comes twice: with `ENTITIES` false (`entity.wgsl`), built with the frame and
/// dispatched while no entity part is drawn, and with it true, taken by
/// [`Marcher::set_entities`] at start-up or else built on the first frame that draws a part (a
/// driver may take seconds over the entity walk). The steps view takes each with `COUNT_STEPS` on
/// ([`Marcher::get`]), which keeps `ENTITIES` at its default.
pub struct Marcher {
    bare: HitPipes,
    entities: OnceLock<HitPipes>,
    pub(super) out_layout: wgpu::BindGroupLayout,
    pub(super) resume_layout: wgpu::BindGroupLayout,
    /// `pipeline`, `field` and `resume` with `COUNT_STEPS` on, each built on its first use.
    counting: [OnceLock<wgpu::ComputePipeline>; 3],
}

/// Which of the hit pass's pipelines a dispatch runs.
#[derive(Clone, Copy)]
pub(super) enum HitPipe {
    Plain = 0,
    Field = 1,
    Resume = 2,
}

impl Marcher {
    /// The hit pass's output layouts: the plain and field dispatches' (`out`) and the resume
    /// dispatch's (`resume`).
    pub(super) fn layouts(device: &wgpu::Device) -> [wgpu::BindGroupLayout; 2] {
        use wgpu::TextureFormat::{R32Uint, Rgba16Float, Rgba16Uint, Rgba32Float};
        let out_layout = io_layout(
            device,
            "hit out",
            &[
                Io::Store(Rgba32Float),
                Io::Store(Rgba16Uint),
                Io::Store(Rgba16Float),
                Io::Storage,
                Io::Storage,
                Io::Store(R32Uint),
                Io::ReadStorage,
            ],
        );
        // The resume dispatch reads its workgroup count from the split arguments, so they are
        // not bound to it.
        let resume_layout = io_layout(
            device,
            "hit resume",
            &[
                Io::Store(Rgba32Float),
                Io::Store(Rgba16Uint),
                Io::Store(Rgba16Float),
                Io::Storage,
            ],
        );
        [out_layout, resume_layout]
    }

    /// The hit pass over `layouts` ([`Marcher::layouts`]) with its `ENTITIES`-false set
    /// `bare`, in [`HitPipe`] order ([`Marcher::build_one`] builds each).
    pub(super) fn from_parts(
        layouts: [wgpu::BindGroupLayout; 2],
        bare: [wgpu::ComputePipeline; 3],
    ) -> Self {
        let [out_layout, resume_layout] = layouts;
        Self {
            bare: HitPipes::from(bare),
            entities: OnceLock::new(),
            out_layout,
            resume_layout,
            counting: std::array::from_fn(|_| OnceLock::new()),
        }
    }

    /// Whether the `ENTITIES`-true set is built.
    pub(super) fn entities_built(&self) -> bool {
        self.entities.get().is_some()
    }

    /// Takes `set` (in [`HitPipe`] order, each from [`Marcher::build_one`] with `entities`)
    /// as the `ENTITIES`-true set unless one is built.
    pub(super) fn set_entities(&self, set: [wgpu::ComputePipeline; 3]) {
        let _ = self.entities.set(HitPipes::from(set));
    }

    /// Pipeline `which` for a frame that draws entities (`entities`) or none; with `count` its
    /// `COUNT_STEPS` variant (the steps view, `ENTITIES` at its default), built over
    /// `world_layout` on its first use.
    pub(super) fn get(
        &self,
        device: &wgpu::Device,
        world_layout: &wgpu::BindGroupLayout,
        which: HitPipe,
        count: bool,
        entities: bool,
    ) -> &wgpu::ComputePipeline {
        if count {
            return self.counting(device, world_layout, which);
        }
        let pipes = if entities {
            self.entity_pipes(device, world_layout)
        } else {
            &self.bare
        };
        match which {
            HitPipe::Plain => &pipes.pipeline,
            HitPipe::Field => &pipes.field,
            HitPipe::Resume => &pipes.resume,
        }
    }

    /// The `ENTITIES`-true set, built over `world_layout` on its first use.
    fn entity_pipes(
        &self,
        device: &wgpu::Device,
        world_layout: &wgpu::BindGroupLayout,
    ) -> &HitPipes {
        self.entities.get_or_init(|| {
            let layouts = [&self.out_layout, &self.resume_layout];
            HitPipes::from(
                HitPipe::ALL
                    .map(|which| Self::build_one(device, world_layout, layouts, which, true)),
            )
        })
    }

    fn counting(
        &self,
        device: &wgpu::Device,
        world_layout: &wgpu::BindGroupLayout,
        which: HitPipe,
    ) -> &wgpu::ComputePipeline {
        self.counting[which as usize].get_or_init(|| {
            let (label, out, entry, field): (_, _, _, &[(&str, f64)]) = match which {
                HitPipe::Plain => ("hit counting", &self.out_layout, "main", &[]),
                HitPipe::Field => (
                    "hit field counting",
                    &self.out_layout,
                    "main",
                    &[("FIELD_SURFACE", 1.0)],
                ),
                HitPipe::Resume => (
                    "hit field resume counting",
                    &self.resume_layout,
                    "field_resume",
                    &[],
                ),
            };
            let mut constants = vec![("COUNT_STEPS", 1.0)];
            constants.extend_from_slice(field);
            compute_pipeline_with(
                device,
                label,
                compose(HIT_MODULES),
                &[Some(world_layout), Some(out)],
                entry,
                &constants,
            )
        })
    }

    /// Pipeline `which` of the hit pass over `world_layout` and the output layouts
    /// `[out, resume]`, with `ENTITIES` as `entities`. Safe to call from several threads at
    /// once: each call builds its own pipeline.
    pub(super) fn build_one(
        device: &wgpu::Device,
        world_layout: &wgpu::BindGroupLayout,
        [out_layout, resume_layout]: [&wgpu::BindGroupLayout; 2],
        which: HitPipe,
        entities: bool,
    ) -> wgpu::ComputePipeline {
        let on = if entities { 1.0 } else { 0.0 };
        let (out, entry, field): (_, _, &[(&str, f64)]) = match which {
            HitPipe::Plain => (out_layout, "main", &[]),
            HitPipe::Field => (out_layout, "main", &[("FIELD_SURFACE", 1.0)]),
            HitPipe::Resume => (resume_layout, "field_resume", &[]),
        };
        let mut constants = field.to_vec();
        constants.push(("ENTITIES", on));
        compute_pipeline_with(
            device,
            which.label(),
            compose(HIT_MODULES),
            &[Some(world_layout), Some(out)],
            entry,
            &constants,
        )
    }
}

impl From<[wgpu::ComputePipeline; 3]> for HitPipes {
    fn from([pipeline, field, resume]: [wgpu::ComputePipeline; 3]) -> Self {
        Self {
            pipeline,
            field,
            resume,
        }
    }
}

impl HitPipe {
    /// The three, in index order.
    pub(super) const ALL: [Self; 3] = [Self::Plain, Self::Field, Self::Resume];

    /// The pipeline's label.
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Plain => "hit",
            Self::Field => "hit field",
            Self::Resume => "hit field resume",
        }
    }

    /// The `ENTITIES`-true pipeline's name in the start-up build's times (its pipeline label
    /// is [`HitPipe::label`]'s).
    pub(super) fn entity_label(self) -> &'static str {
        match self {
            Self::Plain => "hit entities",
            Self::Field => "hit field entities",
            Self::Resume => "hit field resume entities",
        }
    }
}
