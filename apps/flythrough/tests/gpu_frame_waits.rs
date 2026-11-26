//! The frame path's waits on the GPU: a headless fly-through with the stats HUD shown and the
//! bake queue's line due counts every read that waits for the GPU (the pass timers' and the
//! store's counters'), and prints the count on its `frames:` line at exit.

use std::process::Command;

/// The frames the run draws: five of the queue's once-a-second lines, a line every 60
/// headless frames.
const FRAMES: u32 = 300;

/// 300 headless frames of the showcase on the software adapter (`--adapter llvmpipe`; the
/// default adapter where none is named so), 64 x 36, the live queue's pre-roll at its smallest,
/// with the stats HUD shown. Asserted, each a count the code keeps at any pace: the `frames:`
/// line reads `blocking reads 0`; every queue line names the age of the store's counts it read,
/// or not read back yet (the first frame's line prints before the frame turns the count on and
/// so names none); at least one line read them, over 240 frames that never wait, which only a
/// GPU 240 frames behind would miss; without `TIMESTAMP_QUERY` the line says the march timer
/// was not read. Printed and not asserted, because the GPU's pace chooses them: the ages (a
/// three-slot ring, so 3 once the GPU is three copies behind; at most 2 is asserted where the
/// test waits between frames, `gpu_frame`'s timers-and-store test) and, with `TIMESTAMP_QUERY`,
/// how many frames the march timer handed over a set (one lost to every frame whose unwaited
/// poll found no newer set). Before the timers and the store read through rings, the same run
/// read 608 blocking reads (two timer reads a frame and the queue line's).
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
    // The run's line, not the queue step's `frames: queue step ...` tally lines printed
    // before it at exit.
    let run_line = format!("frames: {FRAMES} frames;");
    let frames = stdout
        .lines()
        .find(|l| l.starts_with(&run_line))
        .unwrap_or_else(|| panic!("no `{run_line}` line in\n{stdout}"));
    println!("{frames}");
    assert!(frames.ends_with("; blocking reads 0"), "{frames}");
    let ages: Vec<Option<u32>> = stdout
        .lines()
        .filter(|l| l.starts_with("bake queue: at "))
        .map(|l| {
            // Frame 0's line prints before that frame turns the store's count on, so it
            // names no read; every later line must.
            if l.contains(", frame 0, ") && l.contains("store reads not counted") {
                return None;
            }
            let (_, rest) = l
                .split_once(" pixels (")
                .unwrap_or_else(|| panic!("the line names the read's age: {l}"));
            // `(missing light ...; store ... B, ...; <age>)`: the age is the last item.
            let (inside, _) = rest.split_once(')').expect("closed");
            let age = inside.rsplit("; ").next().expect("an item");
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
    if let Some((_, rest)) = frames.split_once(" over ") {
        let timed: u32 = rest
            .split_once(" timed frames")
            .and_then(|(n, _)| n.parse().ok())
            .unwrap_or_else(|| panic!("a timed count in {frames}"));
        println!("the march timer handed over {timed} sets over {FRAMES} frames");
    } else {
        assert!(frames.contains("march GPU n/a"), "{frames}");
        println!("the adapter has no TIMESTAMP_QUERY: the timers were not read");
    }
}
