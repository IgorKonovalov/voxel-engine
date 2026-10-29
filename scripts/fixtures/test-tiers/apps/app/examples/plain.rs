//! Not a test target (no `test = true`): the program opens a device, no finding.
fn main() {
    let instance = wgpu::Instance::default();
    let _ = pollster::block_on(instance.request_adapter(&Default::default()));
}
