//! Carries the `gpu_` prefix: GPU, and no finding.
#[test]
fn golden() {
    let _ = software_device(64 << 20);
}
