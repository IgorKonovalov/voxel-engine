//! CPU only: no finding. This comment names software_device, wgpu::Instance and
//! request_adapter, and a comment is not a use.
/* Nor is a block comment: device(), or #[ignore] */
#[test]
fn adds() {
    let device = 2 + 2;
    assert_eq!(device, 4);
}

// SEEDED: ignored, neither "soak" nor inside `mod reading`.
#[test]
#[ignore = "slow"]
fn stray() {}

/// A long series: no finding.
#[test]
#[ignore = "soak"]
fn slow_one() {}

/// A measurement run by hand: no finding. Its braces in a string do not end the module.
mod reading {
    #[test]
    #[ignore = "a reading"]
    fn census() {
        println!("}} {}", "}");
    }

    #[test]
    #[ignore]
    fn sweep() {}
}
