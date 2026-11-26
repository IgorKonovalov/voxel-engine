//! `--still PATH` and `--mode-text PATH`: a headless run's last frame and the text the window
//! would show for its flags, written at the end of `--frames N` (a window rehearsal,
//! `scripts/rehearse-window.mjs`). `--still-at F,..` and `--still-dir DIR`: the frames written
//! as PNGs, headless or in a window. `--eye X,Y,Z,YAW,PITCH`: the camera's start on any source.

use super::flags::Parsing;
use super::table::Flag;
use std::path::PathBuf;

/// The still directory when `--still-at` is given without `--still-dir`.
pub(crate) const DEFAULT_STILL_DIR: &str = "target/stills";

/// Where a headless run writes its last frame and its mode text, at the end of `--frames`, and
/// the frames a run writes as PNGs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct StillArgs {
    /// `--still PATH`: the frame's output (no HUD overlay) as a binary PPM (`P6`).
    pub(crate) image: Option<PathBuf>,
    /// `--mode-text PATH`: the banner the run's first frame composes, the mode banner as it
    /// shows at the start, the mode rows and the stats HUD's rows at the exit.
    pub(crate) text: Option<PathBuf>,
    /// `--still-at F,..`: the drawn frames (counted from 0, as `--dig-at` counts) whose image
    /// is written as a PNG.
    pub(crate) at: Vec<u32>,
    /// `--still-dir DIR`: where `--still-at`'s PNGs go; unset, [`DEFAULT_STILL_DIR`].
    pub(crate) dir: Option<PathBuf>,
    /// `--settled-start`: frame 0 waits for full residency and an idle light queue
    /// (`run::settle`); headless only.
    pub(crate) settled: bool,
}

impl StillArgs {
    /// Whether either file is asked for.
    pub(crate) fn any(&self) -> bool {
        self.image.is_some() || self.text.is_some()
    }

    /// The directory `--still-at`'s PNGs go to.
    pub(crate) fn dir(&self) -> PathBuf {
        self.dir
            .clone()
            .unwrap_or_else(|| PathBuf::from(DEFAULT_STILL_DIR))
    }

    /// `--still` and `--mode-text` need a headless run that ends by `--frames`; a `--still-at`
    /// frame is one the run draws (under `--frames N`, before frame `N`); `--still-dir` places
    /// `--still-at`'s files and needs it; `--settled-start` holds a headless run's camera
    /// before frame 0, so it needs `--headless`.
    pub(crate) fn check(&self, headless: bool, frames: Option<u32>) -> Result<(), String> {
        if self.any() && !(headless && frames.is_some()) {
            return Err(
                "--still and --mode-text write at the end of a headless run: they \
                        need --headless and --frames"
                    .to_owned(),
            );
        }
        if let Some(n) = frames
            && let Some(&f) = self.at.iter().find(|&&f| f >= n)
        {
            return Err(format!(
                "--still-at {f} is past the run's last frame: --frames {n} draws frames 0 to {}",
                n.saturating_sub(1)
            ));
        }
        if self.settled && !headless {
            return Err(
                "--settled-start waits before a headless run's frame 0: it needs --headless"
                    .to_owned(),
            );
        }
        if self.dir.is_some() && self.at.is_empty() {
            return Err("--still-dir places --still-at's files; it needs --still-at".to_owned());
        }
        Ok(())
    }
}

/// `--eye`'s value: five finite numbers, the pitch within 89 degrees.
fn parse_eye(text: &str) -> Result<[f32; 5], &'static str> {
    let n: Vec<f32> = text
        .split(',')
        .map(|s| s.trim().parse::<f32>())
        .collect::<Result<_, _>>()
        .map_err(|_| "five numbers X,Y,Z,YAW,PITCH")?;
    let eye: [f32; 5] = n.try_into().map_err(|_| "five numbers X,Y,Z,YAW,PITCH")?;
    if eye.iter().any(|f| !f.is_finite()) || eye[4].abs() > 89.0 {
        return Err("finite numbers, the pitch within 89 degrees");
    }
    Ok(eye)
}

/// The still and camera-pose flags, for the Run section.
pub(super) fn still_flags() -> Vec<Flag<Parsing>> {
    vec![
        Flag::value(
            &["--eye"],
            "X,Y,Z,YAW,PITCH",
            "start the camera at this point in voxels (as the HUD reads it), facing YAW degrees \
                 (counter-clockwise from +x, 90 north) and PITCH degrees up, on any source, in \
                 place of its spawn, shot, castle shot or pose; a streamed world centres on it. \
                 Not with --path, which drives the camera",
            |p: &mut Parsing, v| {
                p.args.eye = Some(parse_eye(v.text).map_err(|e| v.takes(e))?);
                Ok(())
            },
        ),
        Flag::value(
            &["--still-at"],
            "F,..",
            "write each listed drawn frame (from 0) as a PNG, DIR/<scene>-<F:05>.png: the frame's \
                 output, or with --hud the output under the overlay as drawn; headless or in a \
                 window; under --frames N each F is under N",
            |p: &mut Parsing, v| {
                p.args.still.at = super::flags::frames(v)?;
                Ok(())
            },
        ),
        Flag::value(
            &["--still-dir"],
            "DIR",
            "where --still-at's PNGs go, made if missing [target/stills]",
            |p: &mut Parsing, v| {
                p.args.still.dir = Some(PathBuf::from(v.text));
                Ok(())
            },
        ),
        Flag::switch(
            &["--settled-start"],
            "with --headless: before frame 0 wait, ended by state and never by wall time, for \
                 full residency and then a light queue with no job and nothing to refine; prints \
                 `settled: ...` with the counts and steps, and exits non-zero past its step cap",
            |p: &mut Parsing| p.args.still.settled = true,
        ),
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
