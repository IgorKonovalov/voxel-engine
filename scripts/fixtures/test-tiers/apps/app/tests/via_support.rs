//! SEEDED: reaches a device only through the support module it declares.
mod support;

#[test]
fn renders_through_support() {
    let _ = support::rig();
}
