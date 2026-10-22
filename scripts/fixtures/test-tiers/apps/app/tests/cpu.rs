//! CPU only: no finding. This comment names software_device, wgpu::Instance and
//! request_adapter, and a comment is not a use.
/* Nor is a block comment: device() */
#[test]
fn adds() {
    let device = 2 + 2;
    assert_eq!(device, 4);
}
