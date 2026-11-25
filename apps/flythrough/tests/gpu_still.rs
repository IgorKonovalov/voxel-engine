//! `--still` and `--mode-text`: a headless run writes its last frame as a PPM whose pixels are
//! not all one colour, and a mode text naming a flag it was given (the window rehearsal's
//! inputs, `scripts/rehearse-window.mjs`).

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
