//! The hit pass (`hit.wgsl`): its pipelines and its output layouts.

use std::sync::OnceLock;

use super::*;

/// The hit pass's three pipelines: `hit.wgsl`'s `main`, the same with `FIELD_SURFACE`, and
/// `field_resume`.
pub(super) struct HitPipes {
    pub(super) pipeline: wgpu::ComputePipeline,
    pub(super) field: wgpu::ComputePipeline,
    pub(super) resume: wgpu::ComputePipeline,
}

/// The hit pass: `hit.wgsl`'s `main`, one primary ray per pixel into the hit buffer, and in
/// the contour look over a mirror with the field ([`NormalMode::Contour`]) its pipeline with
/// `FIELD_SURFACE`, which walks the field surface, or with the split walk
/// ([`FLAG_FIELD_SPLIT`]) the plain pipeline and then `field_resume` over the rays it handed
/// on. Each comes twice: with `ENTITIES` false (`entity.wgsl`), built with the frame and
/// dispatched while no entity part is drawn, and with it true, built on the first frame that
/// draws one.
pub struct Marcher {
    pub(super) bare: HitPipes,
    entities: OnceLock<HitPipes>,
    pub(super) out_layout: wgpu::BindGroupLayout,
    pub(super) resume_layout: wgpu::BindGroupLayout,
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
            entities: OnceLock::new(),
            out_layout,
            resume_layout,
        }
    }

    /// The pipelines for a frame that draws entities (`entities`) or none.
    pub(super) fn pipes(
        &self,
        device: &wgpu::Device,
        world_layout: &wgpu::BindGroupLayout,
        entities: bool,
    ) -> &HitPipes {
        if !entities {
            return &self.bare;
        }
        self.entities.get_or_init(|| {
            Self::build(
                device,
                world_layout,
                &self.out_layout,
                &self.resume_layout,
                true,
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
