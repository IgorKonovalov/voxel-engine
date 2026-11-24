//! The queue modes' cycles the menu's toggles step through.

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
