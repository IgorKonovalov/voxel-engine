//! The launcher's runs: the fly-through's command line for each menu choice, flag for flag and
//! accepted by the fly-through's own parser, and a run's outcome from a fake child that prints,
//! writes a `run.json` and exits with a chosen code, or panics.

use std::path::{Path, PathBuf};
use std::process::Command;

use flythrough::path::{PathMode, Scene};
use vxe_app::KeyCode;
use vxe_tester::menu::{Command as MenuCommand, Menu};
use vxe_tester::run::{
    Outcome, Output, Profile, ReportDir, RunSpec, TAIL_LINES, child_command, fly_args,
    flythrough_beside, logged_command, read_run, report_name, run_child, utc_stamp,
};

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("vxe-tester-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("scratch folder");
    d
}

/// Each menu choice's flags, exactly: a benchmark adds `--lamp off` after the scene's flags,
/// a tour adds none. The report folder is `R`.
#[test]
fn each_choice_gives_its_exact_flag_list() {
    let r = Path::new("R");
    let table: [(RunSpec, &[&str]); 5] = [
        (
            RunSpec {
                scene: Scene::Cave,
                mode: PathMode::Bench,
                output: Output::Hd720,
                profile: Profile::Floor,
            },
            &[
                "--source",
                "cave",
                "--lamp",
                "off",
                "--tester",
                "--path",
                "cave",
                "--path-mode",
                "bench",
                "--render",
                "1280x720",
                "--profile",
                "floor",
                "--report",
                "R",
            ],
        ),
        (
            RunSpec {
                scene: Scene::Showcase,
                mode: PathMode::Tour,
                output: Output::Hd1080,
                profile: Profile::Portable,
            },
            &[
                "--source",
                "showcase",
                "--tester",
                "--path",
                "showcase",
                "--path-mode",
                "tour",
                "--render",
                "1920x1080",
                "--profile",
                "portable",
                "--report",
                "R",
            ],
        ),
        (
            RunSpec {
                scene: Scene::Castle,
                mode: PathMode::Bench,
                output: Output::Display,
                profile: Profile::Performance,
            },
            &[
                "--castle",
                "--lamp",
                "off",
                "--tester",
                "--path",
                "castle",
                "--path-mode",
                "bench",
                "--render",
                "2560x1440",
                "--profile",
                "performance",
                "--report",
                "R",
            ],
        ),
        (
            RunSpec {
                scene: Scene::Castle,
                mode: PathMode::Bench,
                output: Output::Hd1080,
                profile: Profile::High,
            },
            &[
                "--castle",
                "--lamp",
                "off",
                "--tester",
                "--path",
                "castle",
                "--path-mode",
                "bench",
                "--render",
                "1920x1080",
                "--profile",
                "high",
                "--report",
                "R",
            ],
        ),
        (
            RunSpec {
                scene: Scene::Castle,
                mode: PathMode::Tour,
                output: Output::Hd720,
                profile: Profile::Floor,
            },
            &[
                "--castle",
                "--tester",
                "--path",
                "castle",
                "--path-mode",
                "tour",
                "--render",
                "1280x720",
                "--profile",
                "floor",
                "--report",
                "R",
            ],
        ),
    ];
    for (spec, want) in table {
        assert_eq!(fly_args(&spec, Some([2560, 1440]), r), want, "{spec:?}");
    }
    // Without a known display the display choice falls back to 1280 x 720.
    let spec = RunSpec {
        output: Output::Display,
        ..table[0].0
    };
    assert!(fly_args(&spec, None, r).contains(&"1280x720".to_owned()));
}

/// The menu's keys reach those choices: `Right` on the scene row, `Enter` on the run row.
#[test]
fn the_menu_keys_make_the_run() {
    let mut m = Menu::default();
    for _ in 0..4 {
        assert_eq!(m.key(KeyCode::ArrowUp), None);
    }
    m.key(KeyCode::ArrowRight); // showcase -> castle
    m.key(KeyCode::ArrowDown);
    m.key(KeyCode::Enter); // bench -> tour
    let run = m.key(KeyCode::Digit5);
    assert_eq!(
        run,
        Some(MenuCommand::Run(RunSpec {
            scene: Scene::Castle,
            mode: PathMode::Tour,
            output: Output::Hd720,
            profile: Profile::Floor,
        }))
    );
    assert_eq!(
        m.key(KeyCode::Digit6),
        Some(MenuCommand::RunAll(Output::Hd720, Profile::Floor))
    );
    assert_eq!(m.key(KeyCode::Digit7), Some(MenuCommand::SaveReport));
    assert_eq!(m.key(KeyCode::Digit9), Some(MenuCommand::Quit));
}

/// The fly-through built next to this test's binary (`target/<profile>/flythrough`).
fn flythrough() -> PathBuf {
    let exe = std::env::current_exe().expect("test binary");
    // target/<profile>/deps/run-<hash> -> target/<profile>/flythrough
    let profile = exe
        .parent()
        .and_then(Path::parent)
        .expect("target/<profile>");
    let fly = flythrough_beside(&profile.join("vxe-tester"));
    assert!(
        fly.exists(),
        "{} is missing: build it first (cargo build -p flythrough)",
        fly.display()
    );
    fly
}

/// Every choice's flag list parses in the fly-through (`--check-args`), so a renamed flag fails
/// here, not in a tester's run.
#[test]
fn the_flythrough_accepts_every_choice() {
    let fly = flythrough();
    for scene in Scene::ALL {
        for mode in [PathMode::Bench, PathMode::Tour] {
            for output in Output::ALL {
                for profile in Profile::ALL {
                    let spec = RunSpec {
                        scene,
                        mode,
                        output,
                        profile,
                    };
                    let args = fly_args(&spec, Some([2560, 1440]), Path::new("unused"));
                    let out = Command::new(&fly)
                        .arg("--check-args")
                        .args(&args)
                        .output()
                        .expect("the fly-through runs");
                    assert!(
                        out.status.success(),
                        "{args:?}: {}",
                        String::from_utf8_lossy(&out.stderr)
                    );
                    assert_eq!(
                        String::from_utf8_lossy(&out.stdout).trim(),
                        "flythrough: arguments ok"
                    );
                }
            }
        }
    }
    // A flag the fly-through does not know fails the check.
    let out = Command::new(&fly)
        .args(["--check-args", "--no-such-flag"])
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(2));
}

/// The fake child: this test binary run again with `VXE_FAKE_CHILD` set (`ok`, `exit7`,
/// `panic` or `nojson`) and `VXE_FAKE_DIR` the run folder. Without them it does nothing.
#[test]
fn fake_child() {
    let (Ok(mode), Ok(dir)) = (
        std::env::var("VXE_FAKE_CHILD"),
        std::env::var("VXE_FAKE_DIR"),
    ) else {
        return;
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    println!("fake args: {}", args.join(" "));
    let cwd = std::env::current_dir().expect("current dir");
    println!("fake cwd: {}", cwd.display());
    for i in 0..15 {
        println!("fake line {i}");
    }
    eprintln!("fake warning");
    if mode != "nojson" {
        let json = r#"{"schema": 1, "wall_ms": {"p50": 6.25, "p95": 20.5, "p99": 24.0, "max": 30.0}, "lamp": "off", "surface": {"format": "Bgra8Unorm", "color_space": "Srgb"}, "ended": "completed"}"#;
        std::fs::write(Path::new(&dir).join("run.json"), json).expect("run.json");
    }
    match mode.as_str() {
        "exit7" => std::process::exit(7),
        "panic" => panic!("the fake child panics"),
        _ => std::process::exit(0),
    }
}

fn fake(mode: &str, dir: &Path) -> Command {
    let mut c = Command::new(std::env::current_exe().expect("test binary"));
    c.args(["fake_child", "--exact", "--nocapture", "--test-threads=1"])
        .env("VXE_FAKE_CHILD", mode)
        .env("VXE_FAKE_DIR", dir);
    c
}

#[test]
fn a_run_reads_ok_with_its_p50_or_failed_with_its_exit_and_tail_and_the_report_saves() {
    let base = scratch("runs");
    let name = report_name("0.0.0", "test", &utc_stamp(1_762_867_200));
    assert_eq!(name, "vxe-report-0.0.0-test-20251111-132000");
    let mut report = ReportDir::create(&[base.join("reports")], &name).expect("report folder");
    report.log("tester log line");

    let ok_dir = report.next_run(Scene::Cave, PathMode::Bench);
    assert!(ok_dir.ends_with("runs/01-cave-bench"));
    let ok = run_child(fake("ok", &ok_dir), &ok_dir).expect("runs");
    assert_eq!(
        ok,
        Outcome::Ok {
            p50: 6.25,
            p95: 20.5,
            ended: "completed".to_owned(),
            lamp: Some(false),
            load_ms: None,
            surface: Some("Bgra8Unorm".to_owned()),
        }
    );
    assert_eq!(
        ok.row(),
        "ok: wall p50 6.2 ms, p95 20.5 ms, lamp off, surface Bgra8Unorm"
    );
    // A `run.json` from before the surface field: no surface part.
    let older = Outcome::Ok {
        p50: 6.25,
        p95: 20.5,
        ended: "completed".to_owned(),
        lamp: Some(false),
        load_ms: None,
        surface: None,
    };
    assert_eq!(older.row(), "ok: wall p50 6.2 ms, p95 20.5 ms, lamp off");
    let log = std::fs::read_to_string(ok_dir.join("log.txt")).expect("log.txt");
    assert!(log.contains("fake line 14\n"), "{log}");
    assert!(log.contains("stderr: fake warning\n"), "{log}");

    let exit_dir = report.next_run(Scene::Showcase, PathMode::Bench);
    let failed = run_child(fake("exit7", &exit_dir), &exit_dir).expect("runs");
    let Outcome::Failed { why, tail } = &failed else {
        panic!("exit 7 is a failure: {failed:?}")
    };
    assert_eq!(why, "exit 7");
    assert_eq!(failed.row(), "failed: exit 7");
    assert_eq!(tail.len(), TAIL_LINES);

    let panic_dir = report.next_run(Scene::Castle, PathMode::Bench);
    let panicked = run_child(fake("panic", &panic_dir), &panic_dir).expect("runs");
    assert!(
        matches!(&panicked, Outcome::Failed { why, .. } if why == "exit 101"),
        "{panicked:?}"
    );
    assert_eq!(panicked.row(), "failed: exit 101");
    let Outcome::Failed { tail, .. } = &panicked else {
        unreachable!()
    };
    assert_eq!(tail.len(), TAIL_LINES);
    // The test harness prints its summary after the panic, so the panic is in the log, above
    // the tail.
    let log = std::fs::read_to_string(panic_dir.join("log.txt")).expect("log.txt");
    assert!(
        log.contains("stderr: ") && log.contains("the fake child panics"),
        "{log}"
    );

    let json_dir = report.next_run(Scene::Cave, PathMode::Tour);
    let missing = run_child(fake("nojson", &json_dir), &json_dir).expect("runs");
    assert!(
        matches!(&missing, Outcome::Failed { why, .. } if why.starts_with("no run.json")),
        "{missing:?}"
    );

    let absent = run_child(Command::new(base.join("no-such-flythrough")), &json_dir);
    assert!(
        absent.is_err(),
        "a missing fly-through is an error, not a panic"
    );

    let zip = report.zip().expect("the report saves after failed runs");
    assert_eq!(zip, base.join("reports").join(format!("{name}.zip")));
    let files = vxe_tester::zip::collect(&report.root, &name).expect("collect");
    let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    for want in [
        format!("{name}/tester.log"),
        format!("{name}/runs/01-cave-bench/log.txt"),
        format!("{name}/runs/01-cave-bench/run.json"),
        format!("{name}/runs/02-showcase-bench/log.txt"),
        format!("{name}/runs/03-castle-bench/log.txt"),
    ] {
        assert!(names.contains(&want.as_str()), "{want} not in {names:?}");
    }
    assert!(std::fs::metadata(&zip).expect("zip").len() > 0);
    let _ = std::fs::remove_dir_all(&base);
}

/// No absolute path of the folder a report is made under reaches the saved zip: `tester.log`
/// names executables by file name and folders relative to the report root, the child runs in
/// the report root with a relative `--report` (the fake prints both), and the zip writes the
/// base, standing in for the home folder, `~` in what the child printed of its own accord.
#[test]
fn the_saved_report_holds_no_absolute_path_of_its_base() {
    let base = scratch("noleak");
    let base_str = base.to_str().expect("a UTF-8 temp folder").to_owned();
    let name = report_name("0.0.0", "test", &utc_stamp(1_762_867_200));
    let mut report = ReportDir::create(&[base.join("reports")], &name).expect("report folder");

    // The launcher's system child: logged by file name, its output file relative.
    let tester = base.join("bin").join("vxe-tester");
    let sys_args = ["--system-json", "system.json", "--displays"].map(str::to_owned);
    report.log(&format!("system: {}", logged_command(&tester, &sys_args)));

    let spec = RunSpec {
        scene: Scene::Cave,
        mode: PathMode::Bench,
        output: Output::Hd720,
        profile: Profile::Floor,
    };
    let dir = report.next_run(spec.scene, spec.mode);
    let rel = report.relative(&dir);
    assert_eq!(rel, Path::new("runs").join("01-cave-bench"));
    let args = fly_args(&spec, None, &rel);
    let fly = flythrough_beside(&tester);
    report.log(&format!("run: {}", logged_command(&fly, &args)));

    // The fake child takes the fly-through's arguments after libtest's `--` (as filters that
    // match no other test) and writes its run.json through the relative folder.
    let exe = std::env::current_exe().expect("test binary");
    let mut child_args: Vec<String> = [
        "fake_child",
        "--exact",
        "--nocapture",
        "--test-threads=1",
        "--",
    ]
    .map(str::to_owned)
    .to_vec();
    child_args.extend(args.iter().cloned());
    let mut cmd = child_command(&exe, &child_args, &report.root);
    cmd.env("VXE_FAKE_CHILD", "ok").env("VXE_FAKE_DIR", &rel);
    let outcome = run_child(cmd, &dir).expect("runs");
    assert!(matches!(outcome, Outcome::Ok { .. }), "{outcome:?}");

    let tester_log = std::fs::read_to_string(report.root.join("tester.log")).expect("tester.log");
    let rel_s = rel.display().to_string();
    let fly_name = format!("flythrough{}", std::env::consts::EXE_SUFFIX);
    assert_eq!(
        tester_log,
        format!(
            "system: vxe-tester --system-json system.json --displays\n\
             run: {fly_name} --source cave --lamp off --tester --path cave --path-mode bench --render 1280x720 \
             --profile floor --report {rel_s}\n"
        )
    );
    // The child ran in the report root and was handed the relative folder; its own print of
    // its current directory is the one absolute path, left for the scrub.
    let log = std::fs::read_to_string(dir.join("log.txt")).expect("log.txt");
    assert!(
        log.contains(&format!(
            "fake args: {} {}\n",
            child_args[..5].join(" "),
            args.join(" ")
        )),
        "{log}"
    );
    assert!(
        log.contains(&format!("fake cwd: {}\n", report.root.display())),
        "{log}"
    );

    let zip = report.zip_scrubbing(Some(&base_str)).expect("zip");
    let bytes = std::fs::read(&zip).expect("zip bytes");
    let text = String::from_utf8_lossy(&bytes);
    assert!(!text.contains(&base_str), "the zip names {base_str}");
    let scrubbed_root = format!(
        "fake cwd: ~{}",
        &report.root.display().to_string()[base_str.len()..]
    );
    assert!(
        text.contains(&scrubbed_root),
        "{scrubbed_root} not in the zip"
    );
    // The zip is stored, so its bytes hold every entry's text as written: the three files.
    let files = vxe_tester::zip::collect(&report.root, &name).expect("collect");
    let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            format!("{name}/runs/01-cave-bench/log.txt"),
            format!("{name}/runs/01-cave-bench/run.json"),
            format!("{name}/tester.log"),
        ]
    );
    let _ = std::fs::remove_dir_all(&base);
}

/// A base that cannot be made falls through to the next.
#[test]
fn an_unwritable_base_falls_back_to_the_next() {
    let base = scratch("fallback");
    let file = base.join("a-file");
    std::fs::write(&file, b"x").expect("file");
    let r = ReportDir::create(&[file.join("reports"), base.join("tmp")], "r").expect("fallback");
    assert_eq!(r.root, base.join("tmp").join("r"));
    let _ = std::fs::remove_dir_all(&base);
}

/// `read_run` reads a schema 1 `run.json` with no load time and a schema 2 one with its
/// `load_ms`; the row shows `load 7.4 s` for the second and no load part for the first.
#[test]
fn a_schema_1_and_a_schema_2_run_read_and_only_2_shows_its_load() {
    let dir = scratch("schemas");
    std::fs::create_dir_all(&dir).expect("scratch");
    let one = dir.join("one.json");
    std::fs::write(
        &one,
        r#"{"schema": 1, "wall_ms": {"p50": 6.25, "p95": 20.5}, "lamp": "off", "ended": "completed"}"#,
    )
    .expect("schema 1");
    let two = dir.join("two.json");
    std::fs::write(
        &two,
        r#"{"schema": 2, "wall_ms": {"p50": 6.25, "p95": 20.5}, "lamp": "off", "load_ms": 7412.0, "load_stages": [{"name": "pre-roll", "ms": 6120.0}], "ended": "completed"}"#,
    )
    .expect("schema 2");
    let one = read_run(&one).expect("schema 1 reads");
    assert!(matches!(one, Outcome::Ok { load_ms: None, .. }), "{one:?}");
    assert_eq!(one.row(), "ok: wall p50 6.2 ms, p95 20.5 ms, lamp off");
    let two = read_run(&two).expect("schema 2 reads");
    assert!(
        matches!(two, Outcome::Ok { load_ms: Some(ms), .. } if ms == 7412.0),
        "{two:?}"
    );
    assert_eq!(
        two.row(),
        "ok: wall p50 6.2 ms, p95 20.5 ms, lamp off, load 7.4 s"
    );
}
