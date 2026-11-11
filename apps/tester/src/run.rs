//! A run of the fly-through as a child process, and the report folder the runs fill.
//!
//! The launcher builds the fly-through's command line from a menu choice ([`fly_args`]), starts
//! it with its stdout and stderr captured line by line into the run's `log.txt`
//! ([`run_child`]), and reads the `run.json` the child wrote there. A run that exits non-zero,
//! is killed, or leaves no readable `run.json` is a failed run, kept with its exit code and the
//! log's last lines; the report still saves.
//!
//! The report folder is `vxe-report-<version>-<os>-<UTC yyyymmdd-hhmmss>/` holding
//! `system.json`, `tester.log` and `runs/NN-<scene>-<mode>/{log.txt, run.json}`; [`ReportDir`]
//! zips it beside itself.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};

use flythrough::path::{PATHS, PathMode, Scene};
use flythrough::report::json::{self, Value};

/// Lines of a failed run's log the launcher shows.
pub const TAIL_LINES: usize = 10;

/// The output resolution a run asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    Hd720,
    Hd1080,
    /// The display's size, as the launcher's window found it.
    Display,
}

impl Output {
    pub const ALL: [Self; 3] = [Self::Hd720, Self::Hd1080, Self::Display];

    pub fn name(self) -> &'static str {
        match self {
            Self::Hd720 => "1280x720",
            Self::Hd1080 => "1920x1080",
            Self::Display => "display",
        }
    }

    /// The size in pixels: the display's for [`Output::Display`], 1280 x 720 when it is not
    /// known.
    pub fn size(self, display: Option<[u32; 2]>) -> [u32; 2] {
        match self {
            Self::Hd720 => [1280, 720],
            Self::Hd1080 => [1920, 1080],
            Self::Display => display.unwrap_or([1280, 720]),
        }
    }
}

/// The internal scale a run renders at (`--internal-scale`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    One,
    OneHalf,
    Two,
}

impl Scale {
    pub const ALL: [Self; 3] = [Self::One, Self::OneHalf, Self::Two];

    pub fn name(self) -> &'static str {
        match self {
            Self::One => "1",
            Self::OneHalf => "1.5",
            Self::Two => "2",
        }
    }
}

/// One run's choice: the scene, how its path plays, and the quality.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunSpec {
    pub scene: Scene,
    pub mode: PathMode,
    pub output: Output,
    pub scale: Scale,
}

/// The fly-through's arguments for `spec`: the scene's flags, `--tester`, its path and mode,
/// the resolution, the internal scale, and `--report run_dir`.
pub fn fly_args(spec: &RunSpec, display: Option<[u32; 2]>, run_dir: &Path) -> Vec<String> {
    let path = PATHS
        .iter()
        .find(|p| p.scene == spec.scene)
        .map_or(spec.scene.name(), |p| p.name);
    let [w, h] = spec.output.size(display);
    let mut a: Vec<String> = spec.scene.flags().iter().map(|&f| f.to_owned()).collect();
    a.extend(
        [
            "--tester",
            "--path",
            path,
            "--path-mode",
            spec.mode.name(),
            "--render",
            &format!("{w}x{h}"),
            "--internal-scale",
            spec.scale.name(),
            "--report",
        ]
        .map(str::to_owned),
    );
    a.push(run_dir.display().to_string());
    a
}

/// The fly-through next to `exe` (the running launcher): `flythrough`, `flythrough.exe` on
/// Windows.
pub fn flythrough_beside(exe: &Path) -> PathBuf {
    exe.with_file_name(format!("flythrough{}", std::env::consts::EXE_SUFFIX))
}

/// How a run ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// Exit 0 with a `run.json`: its wall p50 and p95 in ms and its `ended`.
    Ok { p50: f64, p95: f64, ended: String },
    /// A non-zero exit, a kill, or no readable `run.json`: why, and the log's last lines.
    Failed { why: String, tail: Vec<String> },
}

impl Outcome {
    /// The run's row in the window: `ok: wall p50 6.2 ms, p95 20.7 ms` or `failed: exit 101`.
    pub fn row(&self) -> String {
        match self {
            Outcome::Ok { p50, p95, ended } if ended == "completed" => {
                format!("ok: wall p50 {p50:.1} ms, p95 {p95:.1} ms")
            }
            Outcome::Ok { p50, p95, ended } => {
                format!("{ended}: wall p50 {p50:.1} ms, p95 {p95:.1} ms")
            }
            Outcome::Failed { why, .. } => format!("failed: {why}"),
        }
    }
}

/// Runs `cmd` with its stdout and stderr appended line by line to `run_dir/log.txt` (stderr
/// lines marked `stderr: `), waits for it, then reads `run_dir/run.json`. `Err` only when the
/// folder or the log cannot be made or the child cannot start (a missing fly-through).
pub fn run_child(mut cmd: Command, run_dir: &Path) -> io::Result<Outcome> {
    std::fs::create_dir_all(run_dir)?;
    let log = Arc::new(Mutex::new(File::create(run_dir.join("log.txt"))?));
    let tail = Arc::new(Mutex::new(VecDeque::with_capacity(TAIL_LINES)));
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let pipe = |r: Box<dyn io::Read + Send>, mark: &'static str| {
        let log = Arc::clone(&log);
        let tail = Arc::clone(&tail);
        std::thread::spawn(move || {
            for line in BufReader::new(r).lines() {
                let Ok(line) = line else { break };
                let line = format!("{mark}{line}");
                let mut f = log.lock().unwrap_or_else(PoisonError::into_inner);
                let _ = writeln!(f, "{line}");
                let mut t = tail.lock().unwrap_or_else(PoisonError::into_inner);
                if t.len() == TAIL_LINES {
                    t.pop_front();
                }
                t.push_back(line);
            }
        })
    };
    let readers = [
        child.stdout.take().map(|o| pipe(Box::new(o), "")),
        child.stderr.take().map(|e| pipe(Box::new(e), "stderr: ")),
    ];
    let status = child.wait()?;
    for r in readers.into_iter().flatten() {
        let _ = r.join();
    }
    let tail: Vec<String> = tail
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .cloned()
        .collect();
    if !status.success() {
        let why = status
            .code()
            .map_or_else(|| "killed by a signal".to_owned(), |c| format!("exit {c}"));
        return Ok(Outcome::Failed { why, tail });
    }
    match read_run(&run_dir.join("run.json")) {
        Ok(o) => Ok(o),
        Err(why) => Ok(Outcome::Failed { why, tail }),
    }
}

/// A `run.json`'s wall p50, p95 and `ended`.
fn read_run(path: &Path) -> Result<Outcome, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("no run.json ({e})"))?;
    let v = json::parse(&text).map_err(|e| format!("run.json does not parse: {e}"))?;
    let wall = |k: &str| {
        v.get("wall_ms")
            .and_then(|w| w.get(k))
            .and_then(Value::as_f64)
    };
    Ok(Outcome::Ok {
        p50: wall("p50").unwrap_or(f64::NAN),
        p95: wall("p95").unwrap_or(f64::NAN),
        ended: v
            .get("ended")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned(),
    })
}

/// `secs` since the Unix epoch as `yyyymmdd-hhmmss` in UTC (the civil-from-days algorithm of
/// Howard Hinnant's date library).
pub fn utc_stamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

/// The report folder's name: `vxe-report-<version>-<os>-<stamp>`.
pub fn report_name(version: &str, os: &str, stamp: &str) -> String {
    format!("vxe-report-{version}-{os}-{stamp}")
}

/// A report folder being filled: its root, its `tester.log` and its run count.
pub struct ReportDir {
    pub root: PathBuf,
    log: File,
    runs: u32,
}

impl ReportDir {
    /// Makes `name/` under the first of `bases` it can create it in, with an empty
    /// `tester.log`; `Err` with the last base's error when none is writable.
    pub fn create(bases: &[PathBuf], name: &str) -> io::Result<Self> {
        let mut last = io::Error::new(io::ErrorKind::NotFound, "no base folder");
        for base in bases {
            let root = base.join(name);
            match std::fs::create_dir_all(root.join("runs"))
                .and_then(|()| File::create(root.join("tester.log")))
            {
                Ok(log) => {
                    return Ok(Self { root, log, runs: 0 });
                }
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// Appends `line` to `tester.log`.
    pub fn log(&mut self, line: &str) {
        let _ = writeln!(self.log, "{line}");
        let _ = self.log.flush();
    }

    /// The next run's folder: `runs/NN-<scene>-<mode>`, counted from 01.
    pub fn next_run(&mut self, scene: Scene, mode: PathMode) -> PathBuf {
        self.runs += 1;
        self.root
            .join("runs")
            .join(format!("{:02}-{}-{}", self.runs, scene.name(), mode.name()))
    }

    /// Zips the folder to `<root>.zip` beside it, its entries under the folder's name.
    pub fn zip(&self) -> io::Result<PathBuf> {
        let name = self
            .root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let files = crate::zip::collect(&self.root, &name)?;
        let out = self.root.with_file_name(format!("{name}.zip"));
        let mut f = io::BufWriter::new(File::create(&out)?);
        crate::zip::write_zip(&mut f, &files)?;
        f.flush()?;
        Ok(out)
    }
}
