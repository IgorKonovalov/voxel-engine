//! The queue modes' and the bake budget's cycles the menu's toggles step through.

/// The missing-light mode after `m` in [`svet_render::MissingLight::ALL`].
pub(crate) fn next_missing_light(m: svet_render::MissingLight) -> svet_render::MissingLight {
    use svet_render::MissingLight;
    let i = MissingLight::ALL.iter().position(|&x| x == m).unwrap_or(0);
    MissingLight::ALL[(i + 1) % MissingLight::ALL.len()]
}

/// The reach mode after `m` in [`svet_render::ReachMode::ALL`].
pub(crate) fn next_reach(m: svet_render::ReachMode) -> svet_render::ReachMode {
    use svet_render::ReachMode;
    let i = ReachMode::ALL.iter().position(|&x| x == m).unwrap_or(0);
    ReachMode::ALL[(i + 1) % ReachMode::ALL.len()]
}

/// The budgets the menu's `bake budget` steps through: the default slice, 150 ms a second and 300 ms a
/// second, each capped at a quarter of the frame.
pub(crate) const BUDGET_CYCLE: [svet_render::Budget; 3] = [
    svet_render::Budget::PerFrame {
        ms: svet_render::DEFAULT_BAKE_SLICE_MS,
    },
    svet_render::Budget::PerSecond {
        ms_per_s: 150.0,
        cap: 0.25,
    },
    svet_render::Budget::PerSecond {
        ms_per_s: 300.0,
        cap: 0.25,
    },
];

/// The budget after `b` in [`BUDGET_CYCLE`]; one not in it goes to the first.
pub(crate) fn next_budget(b: svet_render::Budget) -> svet_render::Budget {
    let i = BUDGET_CYCLE.iter().position(|&x| x == b);
    BUDGET_CYCLE[i.map_or(0, |i| (i + 1) % BUDGET_CYCLE.len())]
}
