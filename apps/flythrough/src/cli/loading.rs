//! The loading screen's flags (ADR-0059): whether start-up steps behind loading frames, and the
//! diagnostics that read it.

use super::flags::Parsing;
use super::table::Flag;

/// The wall time a loading frame spends on start-up steps before it draws, in ms.
pub(crate) const LOAD_SLICE_MS: f64 = 25.0;

/// `--loading-screen` and its diagnostics.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LoadingArgs {
    /// `--loading-screen on|off` as given; `None` is on in a window and off headless.
    pub(crate) screen: Option<bool>,
    /// `--load-slice MS`: the wall time a loading frame spends on steps.
    pub(crate) slice_ms: f64,
}

impl Default for LoadingArgs {
    fn default() -> Self {
        Self {
            screen: None,
            slice_ms: LOAD_SLICE_MS,
        }
    }
}

impl LoadingArgs {
    /// Whether start-up steps behind loading frames: as given, else in a window only.
    pub(crate) fn screen_on(&self, headless: bool) -> bool {
        self.screen.unwrap_or(!headless)
    }
}

/// The loading screen's rows.
pub(crate) fn loading_flags() -> Vec<Flag<Parsing>> {
    vec![Flag::value(
        &["--loading-screen"],
        "on|off",
        "step the start-up behind loading frames [on in a window, off headless]",
        |p: &mut Parsing, v| {
            p.args.loading.screen = Some(v.on_off()?);
            Ok(())
        },
    )]
}

/// The diagnostics' rows: flags for tests and readings.
pub(crate) fn diagnostic_flags() -> Vec<Flag<Parsing>> {
    vec![Flag::value(
        &["--load-slice"],
        "MS",
        "the start-up step time a loading frame spends before it draws [25]",
        |p: &mut Parsing, v| {
            p.args.loading.slice_ms =
                v.parse_if(|ms: &f64| *ms > 0.0 && ms.is_finite(), "a time above 0 ms")?;
            Ok(())
        },
    )]
}

#[cfg(test)]
mod tests {
    use crate::cli::parse_args_from;
    use crate::test_support::argv;

    /// The screen is on in a window and off headless unless given; `--loading-screen` takes
    /// `on` or `off` and nothing else.
    #[test]
    fn the_loading_screen_is_on_in_a_window_and_off_headless() {
        let a = parse_args_from(argv(&[])).expect("no flags parse");
        assert!(a.loading.screen_on(false));
        assert!(!a.loading.screen_on(true));
        let a = parse_args_from(argv(&["--loading-screen", "on"])).expect("on parses");
        assert!(a.loading.screen_on(true));
        let a = parse_args_from(argv(&["--loading-screen", "off"])).expect("off parses");
        assert!(!a.loading.screen_on(false));
        assert!(parse_args_from(argv(&["--loading-screen", "maybe"])).is_err());
    }

    /// `--load-slice` takes a time above 0 ms; the default is 25.
    #[test]
    fn the_load_slice_takes_a_positive_time() {
        let a = parse_args_from(argv(&[])).expect("no flags parse");
        assert_eq!(a.loading.slice_ms, 25.0);
        let a = parse_args_from(argv(&["--load-slice", "0.5"])).expect("0.5 parses");
        assert_eq!(a.loading.slice_ms, 0.5);
        assert!(parse_args_from(argv(&["--load-slice", "0"])).is_err());
        assert!(parse_args_from(argv(&["--load-slice", "inf"])).is_err());
    }
}
