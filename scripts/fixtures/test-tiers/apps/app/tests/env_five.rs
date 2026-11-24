//! Seeded for check-test-tiers.mjs: five distinct `SVET_*` names, one over the cap of 4 (a
//! finding). A name in a comment, "SVET_IN_A_COMMENT", is not counted.

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

#[test]
fn five_switches() {
    let _ = [
        env("SVET_ONE"),
        env("SVET_TWO"),
        env("SVET_THREE"),
        env("SVET_FOUR"),
        env("SVET_FIVE"),
        env("SVET_ONE"),
    ];
}
