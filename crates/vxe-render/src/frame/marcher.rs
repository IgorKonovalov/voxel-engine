//! The hit pass (`hit.wgsl`): its pipelines and its output layouts.

use super::*;

/// The hit pass: `hit.wgsl`'s `main`, one primary ray per pixel into the hit buffer, and in
/// the contour look over a mirror with the field ([`NormalMode::Contour`]) its pipeline with
/// `FIELD_SURFACE`, which walks the field surface, or with the split walk
/// ([`FLAG_FIELD_SPLIT`]) the plain pipeline and then `field_resume` over the rays it handed
/// on.
pub struct Marcher {
    pub(super) pipeline: wgpu::ComputePipeline,
    pub(super) field: wgpu::ComputePipeline,
    pub(super) resume: wgpu::ComputePipeline,
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
        let layouts = [Some(world_layout), Some(&out_layout)];
        let pipeline = compute_pipeline(device, "hit", compose(HIT_MODULES), &layouts, "main");
        let field = compute_pipeline_with(
            device,
            "hit field",
            compose(HIT_MODULES),
            &layouts,
            "main",
            &[("FIELD_SURFACE", 1.0)],
        );
        let resume = compute_pipeline(
            device,
            "hit field resume",
            compose(HIT_MODULES),
            &[Some(world_layout), Some(&resume_layout)],
            "field_resume",
        );
        Self {
            pipeline,
            field,
            resume,
            out_layout,
            resume_layout,
        }
    }
}
