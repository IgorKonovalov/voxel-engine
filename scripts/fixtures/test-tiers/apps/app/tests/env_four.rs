//! Seeded for check-test-tiers.mjs: four distinct `VXE_*` names, the cap (passes); the first
//! is named twice, and "VXE_FIFTH" in this comment is not counted.

#[test]
fn four_switches() {
    for name in ["VXE_ONE", "VXE_TWO", "VXE_THREE", "VXE_FOUR", "VXE_ONE"] {
        let _ = std::env::var(name);
    }
}
