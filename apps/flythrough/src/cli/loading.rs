//! The loading screen's flags (ADR-0059): whether start-up steps behind loading frames, and the
//! diagnostics that read it.

use super::flags::Parsing;
use super::table::{Flag, Section};

/// The wall time a loading frame spends on start-up steps before it draws, in ms.
pub(crate) const LOAD_SLICE_MS: f64 = 25.0;

/// The loading frame's fade over the first world frames, in ms.
pub(crate) const LOADING_FADE_MS: f64 = 250.0;

/// `--loading-screen` and its diagnostics.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LoadingArgs {
    /// `--loading-screen on|off` as given; `None` is on in a window and off headless.
    pub(crate) screen: Option<bool>,
    /// `--load-slice MS`: the wall time a loading frame spends on steps.
    pub(crate) slice_ms: f64,
    /// `--loading-fade MS`: the loading frame fades out over the first world frames; 0 cuts.
    pub(crate) fade_ms: f64,
    /// `--quit-at-loading-frame N`: acts as `Esc` at loading frame `N`, for a test.
    pub(crate) quit_at: Option<u32>,
    /// `--pre-roll-stepped`: the pre-roll waits on the device before each step and has no
    /// box and no cap, so its outcome does not depend on the machine's pace. Headless only.
    pub(crate) pre_roll_stepped: bool,
}

impl Default for LoadingArgs {
    fn default() -> Self {
        Self {
            screen: None,
            slice_ms: LOAD_SLICE_MS,
            fade_ms: LOADING_FADE_MS,
            quit_at: None,
            pre_roll_stepped: false,
        }
    }
}

impl LoadingArgs {
    /// Whether start-up steps behind loading frames: as given, else in a window only.
    pub(crate) fn screen_on(&self, headless: bool) -> bool {
        self.screen.unwrap_or(!headless)
    }

    /// `--pre-roll-stepped` holds the start-up for the device at every pre-roll step, a
    /// headless reading's mode: refused in a window.
    pub(crate) fn check(&self, headless: bool) -> Result<(), String> {
        if self.pre_roll_stepped && !headless {
            return Err(
                "--pre-roll-stepped steps a headless run's pre-roll by count; \
                        it needs --headless"
                    .to_owned(),
            );
        }
        Ok(())
    }
}

/// The help's Loading and Diagnostics sections.
pub(crate) fn sections() -> [Section<Parsing>; 2] {
    [
        Section {
            title: "Loading",
            flags: loading_flags(),
        },
        Section {
            title: "Diagnostics",
            flags: diagnostic_flags(),
        },
    ]
}

/// The loading screen's rows.
pub(crate) fn loading_flags() -> Vec<Flag<Parsing>> {
    vec![
        Flag::value(
            &["--loading-screen"],
            "on|off",
            "step the start-up and the in-process rebuilds behind loading frames [on in a window, off headless]",
            |p: &mut Parsing, v| {
                p.args.loading.screen = Some(v.on_off()?);
                Ok(())
            },
        ),
        Flag::value(
            &["--loading-fade"],
            "MS",
            "fade the loading frame out over the first world frames; 0 cuts [250]",
            |p: &mut Parsing, v| {
                p.args.loading.fade_ms = v.parse_if(
                    |ms: &f64| *ms >= 0.0 && ms.is_finite(),
                    "a time of 0 ms or more",
                )?;
                Ok(())
            },
        ),
    ]
}

/// The diagnostics' rows: flags for tests and readings.
pub(crate) fn diagnostic_flags() -> Vec<Flag<Parsing>> {
    vec![
        Flag::value(
            &["--load-slice"],
            "MS",
            "the start-up step time a loading frame spends before it draws [25]",
            |p: &mut Parsing, v| {
                p.args.loading.slice_ms =
                    v.parse_if(|ms: &f64| *ms > 0.0 && ms.is_finite(), "a time above 0 ms")?;
                Ok(())
            },
        ),
        Flag::value(
            &["--quit-at-loading-frame"],
            "N",
            "act as Esc at loading frame N (a test's)",
            |p: &mut Parsing, v| {
                p.args.loading.quit_at = Some(v.parse()?);
                Ok(())
            },
        ),
        Flag::switch(
            &["--pre-roll-stepped"],
            "with --headless: wait on the device before every light pre-roll step and end the pre-roll by its chunks only (no 5 s box, no 120 s cap), so its outcome does not depend on the machine's pace",
            |p: &mut Parsing| p.args.loading.pre_roll_stepped = true,
        ),
    ]
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

    /// `--loading-fade` takes 0 ms or more; the default is 250.
    #[test]
    fn the_loading_fade_takes_zero_or_more() {
        let a = parse_args_from(argv(&[])).expect("no flags parse");
        assert_eq!(a.loading.fade_ms, 250.0);
        let a = parse_args_from(argv(&["--loading-fade", "0"])).expect("0 parses");
        assert_eq!(a.loading.fade_ms, 0.0);
        assert!(parse_args_from(argv(&["--loading-fade", "-1"])).is_err());
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

    /// `--pre-roll-stepped` is off by default, parses with `--headless` and is refused
    /// without it.
    #[test]
    fn the_stepped_pre_roll_is_headless_only_and_off_by_default() {
        let a = parse_args_from(argv(&["--headless"])).expect("no flags parse");
        assert!(!a.loading.pre_roll_stepped);
        let a = parse_args_from(argv(&["--headless", "--pre-roll-stepped"]))
            .expect("the flag parses headless");
        assert!(a.loading.pre_roll_stepped);
        let Err(e) = parse_args_from(argv(&["--pre-roll-stepped"])) else {
            panic!("--pre-roll-stepped parsed in a window");
        };
        assert!(e.contains("--headless"), "{e}");
    }
}
