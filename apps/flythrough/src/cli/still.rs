//! `--still PATH` and `--mode-text PATH`: a headless run's last frame and the text the window
//! would show for its flags, written at the end of `--frames N` (a window rehearsal,
//! `scripts/rehearse-window.mjs`).

use super::flags::Parsing;
use super::table::Flag;
use std::path::PathBuf;

/// Where a headless run writes its last frame and its mode text, at the end of `--frames`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct StillArgs {
    /// `--still PATH`: the frame's output (no HUD overlay) as a binary PPM (`P6`).
    pub(crate) image: Option<PathBuf>,
    /// `--mode-text PATH`: the banner the run's first frame composes, the mode banner as it
    /// shows at the start, the mode rows and the stats HUD's rows at the exit.
    pub(crate) text: Option<PathBuf>,
}

impl StillArgs {
    /// Whether either file is asked for.
    pub(crate) fn any(&self) -> bool {
        self.image.is_some() || self.text.is_some()
    }

    /// Both need a headless run that ends by `--frames`.
    pub(crate) fn check(&self, headless: bool, frames: Option<u32>) -> Result<(), String> {
        if self.any() && !(headless && frames.is_some()) {
            return Err(
                "--still and --mode-text write at the end of a headless run: they \
                        need --headless and --frames"
                    .to_owned(),
            );
        }
        Ok(())
    }
}

/// The two flags, for the Run section.
pub(super) fn still_flags() -> Vec<Flag<Parsing>> {
    vec![
        Flag::value(
            &["--still"],
            "PATH",
            "with --headless and --frames: write the last frame's output (no HUD) to PATH as a binary PPM",
            |p: &mut Parsing, v| {
                p.args.still.image = Some(PathBuf::from(v.text));
                Ok(())
            },
        ),
        Flag::value(
            &["--mode-text"],
            "PATH",
            "with --headless and --frames: write the first frame's banner, the mode rows and the stats HUD's rows to PATH",
            |p: &mut Parsing, v| {
                p.args.still.text = Some(PathBuf::from(v.text));
                Ok(())
            },
        ),
    ]
}
