//! Teardown: a streaming world with workers busy, edits applied and chunks persisted is
//! dropped while jobs are queued and running, and the drop returns promptly; and the
//! headless fly-through exits through every traced teardown stage within a bound. The
//! fly-through's window close and Escape both end in this teardown; a worker that cannot
//! stop, or a join or GPU wait that never returns, hangs the process at exit.
#![allow(
    clippy::disallowed_methods,
    reason = "the tests bound teardown by wall-clock time"
)]

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use flythrough::cave::CaveSource;
use flythrough::{RESIDENCY, cave_materials};
use vxe_world::*;

const DROP_BOUND: Duration = Duration::from_secs(5);

/// The fly-through's streamer: the cave at seed 1, residency 16/24/1, one worker per core
/// but one, no spill directory.
fn streaming_world(seed: u64) -> (World, Streamer, VoxelCoord) {
    let source = CaveSource::unbounded(seed);
    let spawn = source.spawn();
    let workers = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .saturating_sub(1)
        .max(1);
    let world = World::new(&cave_materials());
    let streamer = Streamer::new(
        Arc::new(source),
        StreamConfig {
            residency: RESIDENCY,
            workers,
            persist: PersistConfig::default(),
        },
    );
    (world, streamer, spawn)
}

/// Flies `steps` chunks along +x from the spawn, one `update` and `poll` per chunk, carving
/// a 1 m sphere at the focus each step; returns without waiting for the workers, so jobs
/// are queued and running when it does.
fn fly_and_dig(world: &World, streamer: &mut Streamer, spawn: VoxelCoord, steps: i32) {
    let mut carved = 0u64;
    for i in 0..steps {
        let focus = VoxelCoord([spawn.0[0] + i * CHUNK_EDGE, spawn.0[1], spawn.0[2]]);
        streamer.update(world, &[focus]);
        let ev = streamer.poll(world);
        assert!(ev.errors.is_empty(), "stream errors: {:?}", ev.errors);
        let op = EditOp::sphere(centre_of_voxel(focus), 10 * FIXED_ONE, Mode::Carve);
        if let Ok(r) = world.apply(&op) {
            carved += r
                .changed_from
                .iter()
                .map(|&(_, n)| u64::from(n))
                .sum::<u64>();
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let st = streamer.stats();
    assert!(
        st.in_flight > 0 || st.queued > 0,
        "the flight should leave work in flight: {st:?}"
    );
    println!("flew {steps} chunks: {st:?}, carved {carved} voxels");
}

/// Drops `value` on another thread and returns how long it took, or `None` if the drop had
/// not returned within `bound`. The hung thread is left behind: the test process exits
/// without joining it.
fn timed_drop<T: Send + 'static>(value: T, bound: Duration) -> Option<Duration> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let t = Instant::now();
        drop(value);
        let _ = tx.send(t.elapsed());
    });
    rx.recv_timeout(bound).ok()
}

#[test]
fn dropping_the_streamer_mid_flight_returns_within_bound() {
    let (world, mut streamer, spawn) = streaming_world(1);
    fly_and_dig(&world, &mut streamer, spawn, 24);
    let took = timed_drop((world, streamer), DROP_BOUND)
        .unwrap_or_else(|| panic!("dropping the world and streamer took over {DROP_BOUND:?}"));
    println!("dropped in {took:?}");
}

/// The streamer dropped before the world, as a struct declared `streamer` then `world`
/// would: the workers' results still reference the store's chunks.
#[test]
fn dropping_the_streamer_before_the_world_returns_within_bound() {
    let (world, mut streamer, spawn) = streaming_world(1);
    fly_and_dig(&world, &mut streamer, spawn, 24);
    let took = timed_drop(streamer, DROP_BOUND)
        .unwrap_or_else(|| panic!("dropping the streamer took over {DROP_BOUND:?}"));
    println!("streamer dropped in {took:?}");
    let took = timed_drop(world, DROP_BOUND)
        .unwrap_or_else(|| panic!("dropping the world took over {DROP_BOUND:?}"));
    println!("world dropped in {took:?}");
}

/// The fly-through's stderr teardown trace for `--headless --frames 3`, or `None` with a
/// printed skip when the machine has no adapter. The run is killed past `RUN_BOUND`.
fn headless_run_trace() -> Option<String> {
    let (status, err) = headless_run(&[])?;
    assert!(
        status.success(),
        "the fly-through failed: {status}; stderr:\n{err}"
    );
    Some(err)
}

/// The exit status and stderr of `--headless --frames 3` at 64x36 with `extra` arguments, or
/// `None` with a printed skip when the machine has no adapter. The run is killed past
/// `RUN_BOUND`.
fn headless_run(extra: &[&str]) -> Option<(std::process::ExitStatus, String)> {
    const RUN_BOUND: Duration = Duration::from_secs(120);
    // `--look plain`: the teardown trace needs no contour attributes, and a plain mirror
    // builds no attribute pass. On DX12's software adapter (WARP) the attribute pass's
    // initial fill alone runs past `RUN_BOUND`, with its shader compiled by DXC (Plan 0046).
    let mut child = Command::new(env!("CARGO_BIN_EXE_flythrough"))
        .args([
            "--headless",
            "--frames",
            "3",
            "--render",
            "64x36",
            "--look",
            "plain",
        ])
        .args(extra)
        .env("RUST_LOG", "flythrough=info")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the fly-through starts");
    let mut stderr = child.stderr.take().expect("stderr is piped");
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    let t = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("the fly-through can be waited on") {
            break status;
        }
        if t.elapsed() > RUN_BOUND {
            let _ = child.kill();
            let _ = child.wait();
            let err = reader.join().unwrap_or_default();
            panic!("the headless fly-through ran over {RUN_BOUND:?}; stderr:\n{err}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let err = reader.join().expect("stderr is read");
    if !status.success() && err.contains("no adapter") {
        if std::env::var("VXE_REQUIRE_SOFTWARE_ADAPTER").as_deref() == Ok("1") {
            panic!("VXE_REQUIRE_SOFTWARE_ADAPTER=1 but the fly-through found no adapter: {err}");
        }
        println!("skipped: no adapter ({})", err.trim());
        return None;
    }
    Some((status, err))
}

/// The `teardown: ` lines of a trace, without the prefix.
fn teardown_lines(err: &str) -> Vec<&str> {
    err.lines()
        .filter_map(|l| l.split_once("teardown: ").map(|(_, rest)| rest))
        .collect()
}

/// Asserts that `lines` start, in order, with `expected`.
fn assert_stages(lines: &[&str], expected: &[&str]) {
    assert_eq!(lines.len(), expected.len(), "teardown trace: {lines:#?}");
    for (line, want) in lines.iter().zip(expected) {
        assert!(line.starts_with(want), "expected {want:?}, got {line:?}");
    }
}

/// A replay log that does not read ends the session before any world exists: the app, the
/// queue, the device and the adapter are dropped through their traced stages, and then the
/// read error is printed once and the process exits with status 1.
#[test]
fn a_replay_log_that_does_not_read_exits_1_after_the_teardown() {
    let missing = std::env::temp_dir().join("vxe-flythrough-no-such-replay.vxlog");
    assert!(!missing.exists(), "{} exists", missing.display());
    let path = missing.to_str().expect("a UTF-8 temp path");
    let Some((status, err)) = headless_run(&["--level", "--replay", path]) else {
        return;
    };
    assert_eq!(status.code(), Some(1), "stderr:\n{err}");
    let printed = format!("flythrough: --replay {path}: ");
    assert_eq!(err.matches(&printed).count(), 1, "stderr:\n{err}");
    assert_stages(
        &teardown_lines(&err),
        &[
            "the app asked to exit",
            "dropping the app",
            "app dropped in",
            "dropping the queue",
            "queue dropped in",
            "dropping the device",
            "device dropped in",
            "dropping the adapter",
            "adapter dropped in",
            "done, exiting the process",
        ],
    );
}

/// A level whose mirror budget (1 MiB) cannot hold its pool fails its first sync: the session
/// ends through the run's traced stages (the GPU mirror and passes, the world) and the
/// device's, then prints the sync error once and exits with status 1.
#[test]
fn a_mirror_that_cannot_sync_exits_1_after_the_teardown() {
    let Some((status, err)) = headless_run(&["--level", "--mirror-budget", "1"]) else {
        return;
    };
    assert_eq!(status.code(), Some(1), "stderr:\n{err}");
    assert_eq!(
        err.matches("mirror sync failed").count(),
        1,
        "stderr:\n{err}"
    );
    assert_stages(
        &teardown_lines(&err),
        &[
            "the app asked to exit",
            "dropping the app",
            "dropping the GPU mirror and passes",
            "GPU mirror and passes dropped in",
            "dropping the world",
            "world dropped in",
            "app dropped in",
            "dropping the queue",
            "queue dropped in",
            "dropping the device",
            "device dropped in",
            "dropping the adapter",
            "adapter dropped in",
            "done, exiting the process",
        ],
    );
}

/// `--frames` ends the run by returning `Flow::Exit` from the frame, the path Escape takes;
/// the window close joins the same teardown after the event loop returns. Every stage is
/// traced, in the windowed shell's order (the app with its streamer and GPU objects before
/// the queue and device), and the stages together take under `DROP_BOUND`.
#[test]
fn the_headless_flythrough_exits_through_every_teardown_stage_within_bound() {
    let Some(err) = headless_run_trace() else {
        return;
    };
    let lines = teardown_lines(&err);
    println!("{}", lines.join("\n"));
    let expected = [
        "the app asked to exit",
        "dropping the app",
        "dropping the streamer",
        "streamer dropped in",
        "dropping the GPU mirror and passes",
        "GPU mirror and passes dropped in",
        "dropping the world",
        "world dropped in",
        "app dropped in",
        "dropping the queue",
        "queue dropped in",
        "dropping the device",
        "device dropped in",
        "dropping the adapter",
        "adapter dropped in",
        "done, exiting the process",
    ];
    assert_stages(&lines, &expected);
    // The app's stage contains the streamer's and the mirror's, so it is not summed again.
    let ms = |stage: &str| -> f64 {
        lines
            .iter()
            .find_map(|l| l.strip_prefix(stage))
            .and_then(|rest| rest.trim_end_matches(" ms").parse().ok())
            .unwrap_or_else(|| panic!("no duration for {stage:?}"))
    };
    let total: f64 = [
        "app dropped in ",
        "queue dropped in ",
        "device dropped in ",
        "adapter dropped in ",
    ]
    .into_iter()
    .map(ms)
    .sum();
    assert!(
        total < DROP_BOUND.as_secs_f64() * 1e3,
        "teardown took {total:.1} ms, over {DROP_BOUND:?}"
    );
}
