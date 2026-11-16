//! The loading screen (ADR-0059): start-up stepped from the frame loop behind loading frames
//! bakes what the start-up run in one call bakes, and the loading frames are not world frames.
//! Headless fly-through runs on the software adapter (`--adapter llvmpipe`), skipped where the
//! machine has none.

use std::process::Command;

/// The run's stdout and stderr: headless on lavapipe, a 12 m room (about 26 chunks) at 64 x 36, `--frames 3`,
/// with `extra` arguments; `None` with a printed skip without the adapter.
fn room_run(extra: &[&str]) -> Option<(String, String)> {
    let out = Command::new(env!("CARGO_BIN_EXE_flythrough"))
        .args([
            "--headless",
            "--adapter",
            "llvmpipe",
            "--room",
            "12",
            "--frames",
            "3",
            "--render",
            "64x36",
        ])
        .args(extra)
        .env("RUST_LOG", "flythrough=info")
        .output()
        .expect("the fly-through runs");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if !out.status.success() && stderr.contains("no adapter") {
        if std::env::var("VXE_REQUIRE_SOFTWARE_ADAPTER").as_deref() == Ok("1") {
            panic!("VXE_REQUIRE_SOFTWARE_ADAPTER=1 but the fly-through found no adapter: {stderr}");
        }
        println!("skipped: no adapter ({})", stderr.trim());
        return None;
    }
    assert!(
        out.status.success(),
        "the fly-through failed: {}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        out.status
    );
    Some((stdout, stderr))
}

/// The `pre-roll:` line's steps, chunks baked and samples.
fn pre_roll(stdout: &str) -> (u64, u64, u64) {
    let line = stdout
        .lines()
        .find_map(|l| l.strip_prefix("pre-roll: "))
        .unwrap_or_else(|| panic!("no pre-roll: line in\n{stdout}"));
    let n: Vec<u64> = line
        .split(", ")
        .take(3)
        .map(|part| {
            part.split(' ')
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(|| panic!("a count in {part:?} of {line:?}"))
        })
        .collect();
    (n[0], n[1], n[2])
}

/// The world frames the run submitted: its `diag: frame N: submitted` log lines.
fn world_frames(stderr: &str) -> usize {
    stderr
        .lines()
        .filter(|l| l.contains("diag: frame ") && l.contains(": submitted"))
        .count()
}

/// A 12 m room headless with `--loading-screen on` and a 0.05 ms slice, so the pre-roll steps
/// once a loading frame, against the same run with the screen off (the pre-roll run in one
/// call): the same steps, the same chunks baked and the same samples, at least 3 loading
/// frames on, and exactly 3 world frames in both.
#[test]
fn a_sliced_pre_roll_bakes_what_an_unsliced_one_bakes() {
    let Some((off_out, off_err)) = room_run(&["--loading-screen", "off"]) else {
        return;
    };
    let Some((on_out, on_err)) = room_run(&["--loading-screen", "on", "--load-slice", "0.05"])
    else {
        return;
    };
    let off = pre_roll(&off_out);
    let on = pre_roll(&on_out);
    println!("pre-roll off {off:?}, on {on:?}");
    assert!(off.1 > 0 && off.2 > 0, "the pre-roll baked: {off:?}");
    assert_eq!(on, off, "steps, chunks baked and samples, on against off");
    let loading = on_out
        .lines()
        .find_map(|l| l.strip_prefix("loading: pre-roll "))
        .unwrap_or_else(|| panic!("no loading: line in\n{on_out}"));
    let frames: u32 = loading
        .split(", ")
        .nth(1)
        .and_then(|p| p.split(' ').next())
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| panic!("a frame count in {loading:?}"));
    println!("loading: pre-roll {loading}");
    assert!(frames >= 3, "at least 3 loading frames: {loading}");
    assert!(
        !off_out.contains("loading: pre-roll"),
        "the screen off draws no loading frame"
    );
    assert_eq!(world_frames(&off_err), 3, "3 world frames off");
    assert_eq!(world_frames(&on_err), 3, "3 world frames on");
}
