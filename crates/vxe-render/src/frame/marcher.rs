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
/// on. Each comes twice, both built with the frame: with `ENTITIES` false (`entity.wgsl`),
/// dispatched while no entity part is drawn, and with it true, so the first frame that draws
/// a part builds nothing (a driver may take seconds over the entity walk). The steps view takes each with `COUNT_STEPS` on ([`Marcher::get`]), which keeps
/// `ENTITIES` at its default.
pub struct Marcher {
    bare: HitPipes,
    entities: HitPipes,
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
    pub(super) fn new(device: &wgpu::Device, world_layout: &wgpu::BindGroupLayout) -> Self {
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
        Self {
            bare: Self::build(device, world_layout, &out_layout, &resume_layout, false),
            entities: Self::build(device, world_layout, &out_layout, &resume_layout, true),
            out_layout,
            resume_layout,
            counting: std::array::from_fn(|_| OnceLock::new()),
        }
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
        let pipes = if entities { &self.entities } else { &self.bare };
        match which {
            HitPipe::Plain => &pipes.pipeline,
            HitPipe::Field => &pipes.field,
            HitPipe::Resume => &pipes.resume,
        }
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

    fn build(
        device: &wgpu::Device,
        world_layout: &wgpu::BindGroupLayout,
        out_layout: &wgpu::BindGroupLayout,
        resume_layout: &wgpu::BindGroupLayout,
        entities: bool,
    ) -> HitPipes {
        let on = if entities { 1.0 } else { 0.0 };
        let layouts = [Some(world_layout), Some(out_layout)];
        let pipeline = compute_pipeline_with(
            device,
            "hit",
            compose(HIT_MODULES),
            &layouts,
            "main",
            &[("ENTITIES", on)],
        );
        let field = compute_pipeline_with(
            device,
            "hit field",
            compose(HIT_MODULES),
            &layouts,
            "main",
            &[("FIELD_SURFACE", 1.0), ("ENTITIES", on)],
        );
        let resume = compute_pipeline_with(
            device,
            "hit field resume",
            compose(HIT_MODULES),
            &[Some(world_layout), Some(resume_layout)],
            "field_resume",
            &[("ENTITIES", on)],
        );
        HitPipes {
            pipeline,
            field,
            resume,
        }
    }
}
