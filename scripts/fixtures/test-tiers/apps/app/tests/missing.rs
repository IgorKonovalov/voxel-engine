//! SEEDED: opens an adapter directly and lacks the `gpu_` prefix.
#[test]
fn renders() {
    let instance = wgpu::Instance::default();
    let _ = pollster::block_on(instance.request_adapter(&Default::default()));
}
