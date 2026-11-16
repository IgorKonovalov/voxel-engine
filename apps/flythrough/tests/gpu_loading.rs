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
/// frames on in the pre-roll's stages, none off, and exactly 3 world frames in both.
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
    let lighting = loading_frames(&on_err)
        .iter()
        .filter(|(stage, _)| stage.starts_with("lighting"))
        .count();
    println!("{lighting} loading frames stepped the pre-roll");
    assert!(lighting >= 3, "at least 3 loading frames in the pre-roll");
    assert!(
        loading_frames(&off_err).is_empty(),
        "the screen off draws no loading frame"
    );
    assert_eq!(world_frames(&off_err), 3, "3 world frames off");
    assert_eq!(world_frames(&on_err), 3, "3 world frames on");
}

/// Each loading frame's `(stage, fraction)` from its `diag: loading frame N: <stage> <f>` line.
fn loading_frames(stderr: &str) -> Vec<(String, f32)> {
    stderr
        .lines()
        .filter_map(|l| l.split_once("diag: loading frame ").map(|(_, r)| r))
        .map(|r| {
            let (_, rest) = r.split_once(": ").expect("a frame number");
            let rest = rest.rsplit_once(" at ").map_or(rest, |(a, _)| a);
            let (stage, f) = rest.rsplit_once(' ').expect("a stage and a fraction");
            (stage.to_owned(), f.parse().expect("a fraction"))
        })
        .collect()
}

/// The streamed cave headless on lavapipe at the default residency, the pre-roll's near ring
/// at 0 and a coarse pass of 16, the screen on with a 0.05 ms slice (one step a loading
/// frame): the loading frames name the pipelines' build, then the near residency, then
/// the near chunks' light, then the coarse pass, each at least once and in that order; their fractions never decrease; the
/// last reads 1. About 21 s on lavapipe here (2026-11-16), so it runs in the soak tier.
#[test]
#[ignore = "soak"]
fn the_streamed_load_reports_its_stages_in_order_and_ends_at_one() {
    let out = Command::new(env!("CARGO_BIN_EXE_flythrough"))
        .args([
            "--headless",
            "--adapter",
            "llvmpipe",
            "--frames",
            "3",
            "--render",
            "64x36",
            "--near",
            "0",
            "--coarse",
            "16",
            "--loading-screen",
            "on",
            "--load-slice",
            "0.05",
        ])
        .env("RUST_LOG", "flythrough=info")
        .output()
        .expect("the fly-through runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() && stderr.contains("no adapter") {
        println!("skipped: no adapter ({})", stderr.trim());
        return;
    }
    assert!(out.status.success(), "stdout:\n{stdout}\nstderr:\n{stderr}");
    let frames = loading_frames(&stderr);
    let mut order: Vec<&str> = Vec::new();
    for (stage, _) in &frames {
        if order.last() != Some(&stage.as_str()) {
            order.push(stage);
        }
    }
    println!("{} loading frames, stages {order:?}", frames.len());
    let want = [
        "building the pipelines",
        "streaming the near chunks",
        "lighting the near chunks",
        "lighting the coarse pass",
    ];
    assert_eq!(&order[..4], want, "the stages in order");
    for w in frames.windows(2) {
        assert!(w[1].1 >= w[0].1, "the bar moved back: {w:?}");
    }
    assert_eq!(
        frames.last().map(|f| f.1),
        Some(1.0),
        "the last frame reads 1"
    );
}

/// `frames`, `warmup_frames` and `warmup_s` of a headless showcase benchmark of `--frames
/// 130` at 64 x 36 on lavapipe, its report written to a scratch folder, with `screen`.
fn bench_counts(screen: &str) -> Option<(f64, f64, f64)> {
    let dir = std::env::temp_dir().join(format!(
        "vxe-gpu-loading-{}-bench-{screen}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let out = Command::new(env!("CARGO_BIN_EXE_flythrough"))
        .args([
            "--headless",
            "--adapter",
            "llvmpipe",
            "--source",
            "showcase",
            "--path",
            "showcase",
            "--path-mode",
            "bench",
            "--frames",
            "130",
            "--render",
            "64x36",
            "--near",
            "0",
            "--coarse",
            "0",
            "--loading-screen",
            screen,
            "--report",
        ])
        .arg(&dir)
        .output()
        .expect("the fly-through runs");
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() && stderr.contains("no adapter") {
        println!("skipped: no adapter ({})", stderr.trim());
        return None;
    }
    assert!(out.status.success(), "stderr:\n{stderr}");
    let text = std::fs::read_to_string(dir.join("run.json")).expect("run.json");
    let num = |key: &str| {
        text.lines()
            .find_map(|l| l.trim().strip_prefix(&format!("\"{key}\": ")))
            .and_then(|v| v.trim_end_matches(',').parse::<f64>().ok())
            .unwrap_or_else(|| panic!("no {key} in\n{text}"))
    };
    Some((num("frames"), num("warmup_frames"), num("warmup_s")))
}

/// A benchmark's warm-up starts at the first world frame with the screen on: 120 warm-up
/// frames over 2 s of the path's 1/60 s headless clock from frame 0 (the screen off reads the
/// same, 9 measured and 120 warm-up frames at 2.0 s, 2026-11-16), and the measured and warm-up
/// frames add up to the 129 world frames after frame 0, so no loading frame counts.
#[test]
fn a_benchmark_warms_up_from_the_first_world_frame() {
    let Some(on) = bench_counts("on") else {
        return;
    };
    println!("on {on:?}");
    assert_eq!(
        on,
        (9.0, 120.0, 2.0),
        "frames, warm-up frames and warm-up seconds"
    );
    assert_eq!(on.0 + on.1, 129.0, "world frames only");
}
