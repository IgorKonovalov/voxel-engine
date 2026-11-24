//! A camera path run end to end: the fly-through headless on the software adapter (`--adapter
//! llvmpipe`) with a tester benchmark's flags writes its `run.json`. Skipped where the machine
//! has no software adapter.

use std::process::Command;

use flythrough::path::{PathMode, Scene};
use flythrough::report::json::{self, Value};

/// The outdoor benchmark's flags as the tester passes them (the scene's, the mode's,
/// `--tester`, the path), headless on lavapipe at 64 x 36 for 130 frames with the residency cut
/// to 6,8 chunks, its report in a scratch folder: the run exits 0, and `run.json` names the
/// outdoor scene and path, the benchmark mode and the lamp off, carries the `streaming` block,
/// and its measured and warm-up frames add up to the 129 world frames after frame 0.
#[test]
#[ignore = "soak"]
fn a_path_run_writes_its_report() {
    let dir = std::env::temp_dir().join(format!("svet-gpu-path-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let scene = Scene::Outdoor;
    let mode = PathMode::Bench;
    let out = Command::new(env!("CARGO_BIN_EXE_flythrough"))
        .args(scene.flags())
        .args(mode.flags())
        .args([
            "--tester",
            "--path",
            scene.name(),
            "--path-mode",
            mode.name(),
        ])
        .args([
            "--headless",
            "--adapter",
            "llvmpipe",
            "--frames",
            "130",
            "--render",
            "64x36",
            "--residency",
            "6,8",
            "--report",
        ])
        .arg(&dir)
        .output()
        .expect("the fly-through runs");
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() && stderr.contains("no adapter") {
        println!("skipped: no adapter ({})", stderr.trim());
        return;
    }
    assert!(out.status.success(), "{}: stderr:\n{stderr}", out.status);
    let text = std::fs::read_to_string(dir.join("run.json")).expect("run.json");
    let v = json::parse(&text).expect("run.json parses");
    let word = |key: &str| v.get(key).and_then(Value::as_str);
    let num = |key: &str| {
        v.get(key)
            .and_then(Value::as_f64)
            .unwrap_or_else(|| panic!("no {key} in\n{text}"))
    };
    assert_eq!(word("scene"), Some("outdoor"), "{text}");
    assert_eq!(word("path"), Some("outdoor"), "{text}");
    assert_eq!(word("mode"), Some("bench"), "{text}");
    assert_eq!(word("lamp"), Some("off"), "{text}");
    assert!(
        matches!(v.get("streaming"), Some(Value::Obj(_))),
        "streaming is an object in\n{text}"
    );
    assert_eq!(
        num("frames") + num("warmup_frames"),
        129.0,
        "world frames only: {text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
