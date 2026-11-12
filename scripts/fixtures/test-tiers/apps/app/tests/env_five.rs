//! Seeded for check-test-tiers.mjs: five distinct `VXE_*` names, one over the cap of 4 (a
//! finding). A name in a comment, "VXE_IN_A_COMMENT", is not counted.

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

#[test]
fn five_switches() {
    let _ = [
        env("VXE_ONE"),
        env("VXE_TWO"),
        env("VXE_THREE"),
        env("VXE_FOUR"),
        env("VXE_FIVE"),
        env("VXE_ONE"),
    ];
}
