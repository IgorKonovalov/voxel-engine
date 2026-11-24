//! Seeded for check-test-tiers.mjs: four distinct `SVET_*` names, the cap (passes); the first
//! is named twice, and "SVET_FIFTH" in this comment is not counted.

#[test]
fn four_switches() {
    for name in ["SVET_ONE", "SVET_TWO", "SVET_THREE", "SVET_FOUR", "SVET_ONE"] {
        let _ = std::env::var(name);
    }
}
