//! The outdoor source's bars that open a device: the in-process source switch leaves nothing
//! of the outdoor world behind. Headless runs of the fly-through binary on lavapipe.
#![allow(
    clippy::disallowed_methods,
    reason = "the runs are bounded by wall-clock time"
)]

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The stdout of a headless run on lavapipe at 320 x 180 with the live queue's pre-roll at
/// its smallest, the residency at 6 and 8 chunks (the spawn's block of 2 inside it) and `extra` arguments, or `None` with a printed skip when there is no such
/// adapter. Panics past `bound` or on a failed exit, with both streams.
fn run(extra: &[&str], bound: Duration) -> Option<String> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_flythrough"))
        .args([
            "--headless",
            "--adapter",
            "llvmpipe",
            "--render",
            "320x180",
            "--near",
            "0",
            "--coarse",
            "0",
            "--residency",
            "6,8",
        ])
        .args(extra)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the fly-through starts");
    let t = Instant::now();
    while child.try_wait().expect("waits").is_none() {
        if t.elapsed() > bound {
            let _ = child.kill();
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let out = child.wait_with_output().expect("the run's output");
    let (stdout, stderr) = (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    );
    if !out.status.success() && stderr.contains("no adapter") {
        if std::env::var("VXE_REQUIRE_SOFTWARE_ADAPTER").as_deref() == Ok("1") {
            panic!("VXE_REQUIRE_SOFTWARE_ADAPTER=1 but no adapter: {stderr}");
        }
        println!("skipped: no adapter ({})", stderr.trim());
        return None;
    }
    assert!(
        out.status.success() && t.elapsed() <= bound,
        "the run failed or ran over {bound:?}; stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    Some(stdout)
}

/// The last line of `out` that starts with `prefix`.
fn last_line<'a>(out: &'a str, prefix: &str) -> &'a str {
    out.lines()
        .rev()
        .find(|l| l.starts_with(prefix))
        .unwrap_or_else(|| panic!("no `{prefix}` line in:\n{out}"))
}

/// A run that starts on `--source outdoor` and switches at frame `at` (the outdoor source's
/// next is the cave) ends, at frame `frames`, with the voxel hash of the chunks within 2 of
/// the cave's spawn chunk equal to a fresh `--source cave` run's, none of their voxels of the
/// outdoor rows' ids, and the new server's op log empty at the switch.
fn switch_leaves_nothing_behind(at: u32, frames: u32) {
    let bound = Duration::from_secs(300);
    let (frames, at) = (frames.to_string(), at.to_string());
    let common = ["--frames", frames.as_str(), "--spawn-hash"];
    let Some(switched) = run(
        &[&common[..], &["--source", "outdoor", "--switch-at", &at]].concat(),
        bound,
    ) else {
        return;
    };
    let fresh = run(&[&common[..], &["--source", "cave"]].concat(), bound)
        .expect("the adapter of the first run");
    // Frames 0 to `at` were drawn on the outdoor source.
    let drawn = at.parse::<u32>().expect("a frame") + 1;
    assert_eq!(
        last_line(&switched, "switch: "),
        format!("switch: source outdoor to cave after {drawn} frames, op log 0")
    );
    let hash = last_line(&switched, "spawn hash");
    assert!(hash.starts_with("spawn hash 0x"), "{hash}");
    assert!(hash.ends_with(", 0 voxels of ids 5 and 6"), "{hash}");
    assert_eq!(hash, last_line(&fresh, "spawn hash"));
}

/// The plan's run: the switch at frame 200 of 600.
#[test]
#[ignore = "soak"]
fn a_source_switch_leaves_nothing_behind() {
    switch_leaves_nothing_behind(200, 600);
}

/// The smaller variant: the switch at frame 20 of 60. Three world loads (outdoor, the
/// switched cave and a fresh cave) dominate its time, over the soak line on lavapipe.
#[test]
#[ignore = "soak"]
fn a_source_switch_leaves_nothing_behind_in_60_frames() {
    switch_leaves_nothing_behind(20, 60);
}
