//! The shell's defaults a user sees before an app overrides them.

/// An app that keeps the default config gets a window titled with the engine's name.
#[test]
fn the_default_window_title_is_the_engines_name() {
    assert_eq!(svet_app::ShellConfig::default().title, "svet");
}
