//! `--still` and `--mode-text`: a headless run writes its last frame as a PPM whose pixels are
//! not all one colour, and a mode text naming a flag it was given (the window rehearsal's
//! inputs, `scripts/rehearse-window.mjs`); `--still-at` writes the listed frame as a PNG.

use std::process::Command;

/// The closed 12 m room at 64 x 36 on lavapipe for 3 frames under `--look trilinear8`: the still is a
/// `P6` of 64 x 36 pixels with more than one colour in it, and the mode text holds the first
/// frame's banner and the mode rows, which name the look.
#[test]
fn a_headless_run_writes_a_still_and_a_mode_text_naming_its_flags() {
    let dir = std::env::temp_dir().join(format!("flythrough-still-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let (still, text) = (dir.join("run.ppm"), dir.join("run.txt"));
    let out = Command::new(env!("CARGO_BIN_EXE_flythrough"))
        .args([
            "--headless",
            "--adapter",
            "llvmpipe",
            "--room",
            "12",
            "--render",
            "64x36",
            "--look",
            "trilinear8",
            "--frames",
            "3",
        ])
        .arg("--still")
        .arg(&still)
        .arg("--mode-text")
        .arg(&text)
        .output()
        .expect("the fly-through runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() && stderr.contains("no adapter") {
        println!("skipped: no adapter ({})", stderr.trim());
        return;
    }
    assert!(out.status.success(), "stdout:\n{stdout}\nstderr:\n{stderr}");

    let ppm = std::fs::read(&still).expect("the still is written");
    let head = b"P6\n64 36\n255\n";
    assert_eq!(&ppm[..head.len()], head, "a P6 of the output size");
    let px = &ppm[head.len()..];
    assert_eq!(px.len(), 64 * 36 * 3);
    let first = &px[..3];
    let other = px.chunks_exact(3).filter(|p| *p != first).count();
    assert!(other > 0, "the still is all one colour {first:?}");

    let text = std::fs::read_to_string(&text).expect("the mode text is written");
    assert!(text.starts_with("banner (first frame):\n"), "{text}");
    let modes = text
        .split("\nmodes:\n")
        .nth(1)
        .expect("the mode rows follow");
    assert!(modes.contains("look trilinear8"), "{text}");
    println!("{other} of {} pixels differ from the first", px.len() / 3);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `--still-at 1 --still-dir DIR` on the closed 4 m room at 320 x 180 on lavapipe for 2
/// frames: the run prints `still: DIR/room-40-00001.png 320x180 frame 1`, and the file is a PNG
/// of 320 x 180 whose pixels are not all one colour.
#[test]
fn still_at_writes_a_png_of_the_listed_frame() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("flythrough-still-at-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let out = Command::new(env!("CARGO_BIN_EXE_flythrough"))
        .args([
            "--headless",
            "--adapter",
            "llvmpipe",
            "--room",
            "4",
            "--render",
            "320x180",
            "--frames",
            "2",
            "--still-at",
            "1",
            "--still-dir",
        ])
        .arg(&dir)
        .output()
        .expect("the fly-through runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() && stderr.contains("no adapter") {
        println!("skipped: no adapter ({})", stderr.trim());
        return;
    }
    assert!(out.status.success(), "stdout:\n{stdout}\nstderr:\n{stderr}");
    let file = dir.join("room-40-00001.png");
    let line = format!("still: {} 320x180 frame 1", file.display());
    assert!(
        stdout.lines().any(|l| l == line),
        "no `{line}` in:\n{stdout}"
    );
    let written: Vec<_> = std::fs::read_dir(&dir)
        .expect("the still dir is made")
        .map(|e| e.expect("an entry").file_name())
        .collect();
    assert_eq!(written, ["room-40-00001.png"], "one still, frame 1's");

    let decoder = png::Decoder::new(std::io::BufReader::new(
        std::fs::File::open(&file).expect("the still opens"),
    ));
    let mut reader = decoder.read_info().expect("a PNG");
    let mut px = vec![0; reader.output_buffer_size().expect("a size")];
    let info = reader.next_frame(&mut px).expect("its pixels decode");
    assert_eq!((info.width, info.height), (320, 180));
    assert_eq!(info.color_type, png::ColorType::Rgb);
    let px = &px[..info.buffer_size()];
    let first = &px[..3];
    let other = px.chunks_exact(3).filter(|p| *p != first).count();
    assert!(other > 0, "the still is all one colour {first:?}");
    println!("{other} of {} pixels differ from the first", 320 * 180);
    let _ = std::fs::remove_dir_all(&dir);
}
