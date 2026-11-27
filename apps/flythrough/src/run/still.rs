//! `--still` and `--mode-text`: a headless run writes the banner its first frame composes, and
//! at the end of `--frames` its last frame's output and the mode and stats rows, for a window
//! rehearsal (`scripts/rehearse-window.mjs`) to read before the owner opens the window.
//! `--still-at`: the listed frames as PNGs, the frame's output or the overlay over it.

use crate::cli::{Args, RunMode};
use crate::run::Run;
use crate::run::hud_feed::Overlay;
use std::fmt::Write as _;
use std::path::Path;
use svet_app::Gpu;
use svet_render::wgpu;

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

/// `--still-at`'s file stem for the run's scene: `showcase-hall`, `outdoor-15-00` (the
/// start's clock), `castle-nave`, `level-vein` under `--pose`, else the source's name.
pub(crate) fn still_stem(args: &Args) -> String {
    if args.castle {
        return format!("castle-{}", args.castle_shot.name());
    }
    match args.mode {
        RunMode::Showcase => format!("showcase-{}", args.shot.name),
        RunMode::Outdoor => {
            let m = args.outdoor.clock.minutes.floor() as u32 % 1440;
            format!("outdoor-{:02}-{:02}", m / 60, m % 60)
        }
        RunMode::Level => args
            .pose
            .map_or_else(|| "level".to_owned(), |p| format!("level-{}", p.name)),
        RunMode::Cave => "cave".to_owned(),
        RunMode::Room(edge) => format!("room-{edge}"),
        RunMode::ThinRig => "thin-rig".to_owned(),
        RunMode::SeamRig => "seam-rig".to_owned(),
    }
}

impl Run {
    /// After drawn frame `frame`, when `--still-at` lists it: writes its still ([`write_png_at`]),
    /// under the stats HUD the overlay over the output as drawn ([`compose`]), else the frame's
    /// output alone.
    pub(crate) fn write_still_at(&self, gpu: &Gpu, frame: u32) {
        let Some(gfx) = self
            .gfx
            .as_ref()
            .filter(|_| self.args.still.at.contains(&frame))
        else {
            return;
        };
        let hud = Overlay::of(self.args.tester, self.menu.is_open(), self.hud.visible);
        let output = gfx.frame.target();
        let composed =
            (hud == Overlay::Stats).then(|| compose(gpu, &gfx.presenter, &gfx.hud, output));
        write_png_at(gpu, &self.args, frame, output, composed.as_ref());
    }
}

/// A texture the presenter and the overlay draw into, in a submit of their own after the
/// frame's: the surface is a render target the app cannot read back, so the still is the same
/// two passes, over the same output and overlay rows, drawn again into a texture it can.
pub(crate) fn compose(
    gpu: &Gpu,
    presenter: &svet_render::Presenter,
    hud: &svet_render::HudOverlay,
    output: &wgpu::Texture,
) -> wgpu::Texture {
    let [w, h] = gpu.size;
    let tex = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("still composed"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: gpu.surface_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    let mut enc = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    presenter.encode(&gpu.device, &mut enc, output, &view);
    hud.encode_timed(&mut enc, &view, w, h, None);
    gpu.queue.submit([enc.finish()]);
    tex
}

/// Writes frame `frame`'s still as `DIR/<stem>-<frame:05>.png`: `composed` when the overlay
/// was drawn into one ([`compose`]), else the frame's output; prints the `still:`
/// line or why it was not written.
fn write_png_at(
    gpu: &Gpu,
    args: &Args,
    frame: u32,
    output: &wgpu::Texture,
    composed: Option<&wgpu::Texture>,
) {
    let path = args
        .still
        .dir()
        .join(format!("{}-{frame:05}.png", still_stem(args)));
    let (tex, bgra) = match composed {
        Some(t) => {
            use wgpu::TextureFormat as F;
            match t.format() {
                F::Rgba8Unorm | F::Rgba8UnormSrgb => (t, false),
                F::Bgra8Unorm | F::Bgra8UnormSrgb => (t, true),
                f => {
                    eprintln!(
                        "still: cannot write {}: a {f:?} surface is not read back",
                        path.display()
                    );
                    return;
                }
            }
        }
        None => (output, false),
    };
    let (w, h) = (tex.width(), tex.height());
    let rgba = svet_render::debug::read_rgba8(&gpu.device, &gpu.queue, tex);
    match write_png(&path, &rgb(&rgba, bgra), w, h) {
        Ok(()) => println!("still: {} {w}x{h} frame {frame}", path.display()),
        Err(e) => eprintln!("still: cannot write {}: {e}", path.display()),
    }
}

/// `rgba` (or BGRA when `bgra`) as tightly packed RGB, alpha dropped.
pub(crate) fn rgb(rgba: &[u8], bgra: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len() / 4 * 3);
    for px in rgba.chunks_exact(4) {
        if bgra {
            out.extend_from_slice(&[px[2], px[1], px[0]]);
        } else {
            out.extend_from_slice(&px[..3]);
        }
    }
    out
}

/// `rgb` (`width` x `height`, rows top to bottom) as an 8-bit RGB PNG at `path`, its directory
/// made if missing.
fn write_png(path: &Path, rgb: &[u8], width: u32, height: u32) -> Result<(), String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut wr| wr.write_image_data(rgb))
        .map_err(|e| format!("png: {e}"))
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

    /// The still's RGB keeps red, green and blue in order from RGBA and swaps them from BGRA,
    /// alpha dropped either way.
    #[test]
    fn rgb_drops_alpha_and_unswaps_bgra() {
        let px = [1, 2, 3, 255, 4, 5, 6, 0];
        assert_eq!(rgb(&px, false), vec![1, 2, 3, 4, 5, 6]);
        assert_eq!(rgb(&px, true), vec![3, 2, 1, 6, 5, 4]);
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
