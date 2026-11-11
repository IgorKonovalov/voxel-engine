//! The frame path's waits on the GPU: a headless fly-through with the stats HUD shown and the
//! bake queue's line due counts every read that waits for the GPU (the pass timers' and the
//! store's counters'), and prints the count on its `frames:` line at exit.

use std::process::Command;

/// The frames the run draws: five of the queue's once-a-second lines, a line every 60
/// headless frames.
const FRAMES: u32 = 300;

/// 300 headless frames of the showcase on the software adapter (`--adapter llvmpipe`; the
/// default adapter where none is named so), 64 x 36, the live queue's pre-roll at its smallest,
/// with the stats HUD shown: the `frames:` line reads `blocking reads 0`, every queue line names
/// the age of the store's counts it read (at most 2 frames, or not read back yet on the first
/// frame) and at least one read them, and with `TIMESTAMP_QUERY` the march timer handed over a
/// set on all but at most two frames. Before the timers and the store read through rings, the
/// same run read 608 (two timer reads a frame and the queue line's).
#[test]
#[ignore = "soak"]
fn a_shown_hud_and_the_queue_line_wait_on_the_gpu_nowhere() {
    let out = Command::new(env!("CARGO_BIN_EXE_flythrough"))
        .args([
            "--headless",
            "--adapter",
            "llvmpipe",
            "--source",
            "showcase",
            "--hud",
            "--frames",
            &FRAMES.to_string(),
            "--render",
            "64x36",
            "--near",
            "0",
            "--coarse",
            "0",
        ])
        .output()
        .expect("the fly-through runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() && stderr.contains("no adapter") {
        println!("skipped: no adapter ({})", stderr.trim());
        return;
    }
    assert!(out.status.success(), "stdout:\n{stdout}\nstderr:\n{stderr}");
    let frames = stdout
        .lines()
        .find(|l| l.starts_with("frames: "))
        .unwrap_or_else(|| panic!("no frames: line in\n{stdout}"));
    println!("{frames}");
    assert!(
        frames.starts_with(&format!("frames: {FRAMES} frames;")),
        "{frames}"
    );
    assert!(frames.ends_with("; blocking reads 0"), "{frames}");
    let ages: Vec<Option<u32>> = stdout
        .lines()
        .filter(|l| l.starts_with("bake queue: at "))
        .map(|l| {
            let (_, rest) = l
                .split_once(" pixels (")
                .expect("the line names the read's age");
            let (age, _) = rest.split_once(')').expect("closed");
            if age == "not read back yet" {
                None
            } else {
                let n = age
                    .strip_prefix("read ")
                    .and_then(|a| a.strip_suffix(" frames old"))
                    .unwrap_or_else(|| panic!("an age: {age}"));
                Some(n.parse().expect("an age in frames"))
            }
        })
        .collect();
    println!("queue lines' read ages: {ages:?}");
    assert!(
        ages.iter().any(Option::is_some),
        "a line read the store's counts"
    );
    assert!(ages.iter().flatten().all(|&a| a <= 2), "{ages:?}");
    if let Some((_, rest)) = frames.split_once(" over ") {
        let timed: u32 = rest
            .split_once(" timed frames")
            .and_then(|(n, _)| n.parse().ok())
            .unwrap_or_else(|| panic!("a timed count in {frames}"));
        assert!(
            timed + 2 >= FRAMES,
            "the march timer handed over {timed} sets"
        );
    } else {
        assert!(frames.contains("march GPU n/a"), "{frames}");
        println!("the adapter has no TIMESTAMP_QUERY: the timers were not read");
    }
}
