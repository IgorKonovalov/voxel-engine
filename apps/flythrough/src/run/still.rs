//! `--still` and `--mode-text`: a headless run writes the banner its first frame composes, and
//! at the end of `--frames` its last frame's output and the mode and stats rows, for a window
//! rehearsal (`scripts/rehearse-window.mjs`) to read before the owner opens the window.

use crate::run::Run;
use std::fmt::Write as _;
use std::path::Path;
use svet_app::Gpu;

impl Run {
    /// The first frame's text: the banner the frame composes, then the mode banner as it shows
    /// at its start (a headless run's first frame may come after the window's 3 s).
    pub(crate) fn start_text(&self) -> String {
        let mut banner = String::new();
        self.compose_banner(&mut banner, &self.args.queue_words());
        let mut modes = String::new();
        self.modes.write_banner(self.modes.banner_at, &mut modes);
        format!(
            "banner (first frame):\n{}\n\nbanner (at its start):\n{}\n",
            banner.trim_matches('\n'),
            modes.trim_matches('\n'),
        )
    }

    /// The exit's text: the mode rows the HUD shows and the stats HUD's rows.
    pub(crate) fn exit_text(&self) -> String {
        let mut rows = String::new();
        self.modes.write_rows(&mut rows);
        let mut stats = self.hud.stats.clone();
        stats.world.age_frames = 0;
        let mut out = String::new();
        let _ = write!(out, "\nmodes:\n{rows}\nhud:\n{stats}\n");
        out
    }

    /// At the first frame under `--mode-text`: writes [`Run::start_text`] to its path.
    pub(crate) fn write_start_text(&self) {
        if let Some(path) = &self.args.still.text {
            report(path, std::fs::write(path, self.start_text()));
        }
    }

    /// At the end of `--frames`: appends [`Run::exit_text`] to `--mode-text`'s file and writes
    /// the frame's output to `--still`'s as a PPM. Call after the exit readings refresh the
    /// stats HUD.
    pub(crate) fn write_still(&self, gpu: &Gpu) {
        if let Some(path) = &self.args.still.text {
            let appended = std::fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(path)
                .and_then(|mut f| std::io::Write::write_all(&mut f, self.exit_text().as_bytes()));
            report(path, appended);
        }
        if let (Some(path), Some(gfx)) = (&self.args.still.image, &self.gfx) {
            let target = gfx.frame.target();
            let px = svet_render::debug::read_rgba8(&gpu.device, &gpu.queue, target);
            report(
                path,
                std::fs::write(path, ppm(&px, target.width(), target.height())),
            );
        }
    }
}

/// Prints where a file went, or why it did not.
fn report(path: &Path, r: std::io::Result<()>) {
    match r {
        Ok(()) => println!("still: wrote {}", path.display()),
        Err(e) => eprintln!("still: cannot write {}: {e}", path.display()),
    }
}

/// `rgba` (tightly packed `width` x `height`, rows top to bottom) as a binary PPM: the `P6`
/// header and the RGB bytes, alpha dropped.
pub(crate) fn ppm(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut out = format!("P6\n{width} {height}\n255\n").into_bytes();
    out.reserve(rgba.len() / 4 * 3);
    for px in rgba.chunks_exact(4) {
        out.extend_from_slice(&px[..3]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::hud_live_tests::run_of;
    use crate::test_support::argv;

    /// The PPM is the `P6` header and the pixels' RGB bytes in order, alpha dropped.
    #[test]
    fn a_ppm_is_the_header_and_the_rgb_bytes() {
        let rgba = [1, 2, 3, 255, 4, 5, 6, 0, 7, 8, 9, 9, 10, 11, 12, 1];
        let p = ppm(&rgba, 2, 2);
        let head = b"P6\n2 2\n255\n";
        assert_eq!(&p[..head.len()], head);
        assert_eq!(&p[head.len()..], &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
    }

    /// `--still` and `--mode-text` take a path each and need `--headless` and `--frames`;
    /// `--help` lists both.
    #[test]
    fn the_still_flags_parse_only_in_a_headless_run_with_frames() {
        let parse = |v: &[&str]| crate::cli::parse_args_from(argv(v));
        let a = parse(&[
            "--headless",
            "--frames",
            "3",
            "--still",
            "a.ppm",
            "--mode-text",
            "a.txt",
        ])
        .expect("parses");
        assert_eq!(a.still.image.as_deref(), Some(Path::new("a.ppm")));
        assert_eq!(a.still.text.as_deref(), Some(Path::new("a.txt")));
        assert!(!parse(&[]).expect("parses").still.any());
        for bad in [
            &["--still", "a.ppm"][..],
            &["--headless", "--mode-text", "a.txt"],
            &["--frames", "3", "--still", "a.ppm"],
        ] {
            let e = parse(bad).err().expect("refused");
            assert!(e.contains("need --headless and --frames"), "{bad:?}: {e}");
        }
        assert!(parse(&["--headless", "--frames", "3", "--still"]).is_err());
        let help = crate::cli::help();
        assert!(help.contains("--still PATH") && help.contains("--mode-text PATH"));
    }

    /// The start text names a flag the run was given: the mode banner at its start carries
    /// the lighting line, and `--key-far ray` shows there; the exit text carries the mode
    /// rows, the same words in the HUD's lighting row.
    #[test]
    fn the_mode_text_names_a_flag_the_run_was_given() {
        let args = |extra: &[&str]| {
            let mut v = vec!["--level", "--sky", "on"];
            v.extend_from_slice(extra);
            crate::cli::parse_args_from(argv(&v)).expect("parses")
        };
        let given = run_of(args(&["--key-far", "ray"]));
        let start = given.start_text();
        assert!(start.starts_with("banner (first frame):\n"), "{start}");
        assert!(start.contains("key far ray"), "{start}");
        assert!(given.exit_text().contains("key far ray"));
        let plain = run_of(args(&[]));
        assert!(!plain.start_text().contains("key far ray"));
        assert!(plain.exit_text().contains("\nmodes:\n"));
    }
}
