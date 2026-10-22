//! SEEDED: opens an adapter directly and is not in the gpu filter.
#[test]
fn renders() {
    let instance = wgpu::Instance::default();
    let _ = pollster::block_on(instance.request_adapter(&Default::default()));
}
