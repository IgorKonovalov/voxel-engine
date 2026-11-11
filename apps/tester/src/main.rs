//! `vxe-tester`: the tester build's launcher (ADR-0054).
//!
//! With no arguments it opens a small window with the menu (`vxe_tester::menu`): the scene,
//! benchmark or tour, the output and the internal scale; run, run all benchmarks, save report,
//! open report folder, quit. Each run starts the fly-through found next to this executable as a
//! child process (`vxe_tester::run`) and the window shows its state, then its wall p50 and p95,
//! or its exit code and the log's last lines. The first run or save makes the report folder
//! under `reports/` beside the executable (the OS's temp directory when that is not writable),
//! writes its `system.json` and logs every command line and exit code to its `tester.log`;
//! "save report" zips it beside itself. The window names both paths.
//!
//! `vxe-tester --auto` does "run all benchmarks", then "save report", then quits, with the
//! menu's defaults: an unattended reading. `vxe-tester --system-json PATH` writes the machine's
//! `system.json` (`vxe_tester::system`) to `PATH` without a window, prints it and exits 0;
//! `--displays` after it also asks the window system for the displays. A file that cannot be
//! written exits 1.

use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};

use flythrough::path::{PathMode, Scene};
use flythrough::report::json::{self, Value};
use vxe_app::{App, Flow, Gpu, Input, KeyCode, Pacing, ShellConfig, wgpu};
use vxe_render::{HudColour, HudOverlay};
use vxe_tester::menu::{Command, Menu};
use vxe_tester::run::{
    Outcome, Output, ReportDir, RunSpec, Scale, fly_args, flythrough_beside, report_name,
    run_child, utc_stamp,
};
use vxe_tester::system::{SystemInfo, adapters, displays};

/// Glyph pixels per target pixel of the menu's text.
const TEXT_SCALE: u32 = 2;
/// Frames a second the launcher draws at: enough for a menu, little next to a benchmark.
const LAUNCHER_HZ: u32 = 10;
/// Status lines the window keeps.
const STATUS_LINES: usize = 14;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("error")).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    match refs.as_slice() {
        ["--system-json", path] => system_json(path, false),
        ["--system-json", path, "--displays"] => system_json(path, true),
        [] => launcher(false),
        ["--auto"] => launcher(true),
        _ => {
            eprintln!("vxe-tester: usage: vxe-tester [--auto] | --system-json PATH [--displays]");
            std::process::exit(2);
        }
    }
}

fn system_json(path: &str, with_displays: bool) {
    let shown = if with_displays { displays() } else { None };
    let json = SystemInfo::collect(adapters(), shown).to_json();
    print!("{json}");
    if let Err(e) = std::fs::write(path, &json) {
        eprintln!("vxe-tester: --system-json {path}: {e}");
        std::process::exit(1);
    }
}

/// What the window asks the worker to do.
enum Job {
    Run(RunSpec),
    RunAll(Output, Scale),
    Save,
}

/// What the window shows of the worker's state.
#[derive(Default)]
struct Status {
    /// The run in progress, in words.
    busy: Option<String>,
    /// One line per finished run, then messages, newest last.
    lines: Vec<String>,
    /// The report folder once made, and the last zip saved.
    folder: Option<PathBuf>,
    zip: Option<PathBuf>,
    /// Jobs finished, for `--auto`.
    done: u32,
    /// The launcher window's adapter, for `tester.log`.
    window_adapter: Option<String>,
}

impl Status {
    fn say(&mut self, line: String) {
        self.lines.push(line);
        let n = self.lines.len();
        if n > STATUS_LINES {
            self.lines.drain(..n - STATUS_LINES);
        }
    }
}

fn lock(s: &Mutex<Status>) -> std::sync::MutexGuard<'_, Status> {
    s.lock().unwrap_or_else(PoisonError::into_inner)
}

#[allow(
    clippy::disallowed_methods,
    reason = "the report folder's name carries the UTC time it was made"
)]
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The worker: owns the report folder and runs the jobs one at a time.
struct Worker {
    exe: PathBuf,
    report: Option<ReportDir>,
    display: Option<[u32; 2]>,
    status: Arc<Mutex<Status>>,
}

impl Worker {
    fn serve(mut self, jobs: Receiver<Job>) {
        for job in jobs {
            match job {
                Job::Run(spec) => self.run(spec),
                Job::RunAll(output, scale) => {
                    for scene in Scene::ALL {
                        self.run(RunSpec {
                            scene,
                            mode: PathMode::Bench,
                            output,
                            scale,
                        });
                    }
                }
                Job::Save => self.save(),
            }
            let mut s = lock(&self.status);
            s.busy = None;
            s.done += 1;
        }
    }

    /// The report folder, made on first use with its `system.json`.
    fn report(&mut self) -> Option<&mut ReportDir> {
        if self.report.is_none() {
            let name = report_name(
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS,
                &utc_stamp(now_secs()),
            );
            let mut bases = Vec::new();
            if let Some(dir) = self.exe.parent() {
                bases.push(dir.join("reports"));
            }
            bases.push(std::env::temp_dir().join("vxe-reports"));
            match ReportDir::create(&bases, &name) {
                Ok(mut r) => {
                    r.log(&format!(
                        "vxe-tester {} ({})",
                        env!("CARGO_PKG_VERSION"),
                        flythrough::report::COMMIT
                    ));
                    let adapter = lock(&self.status).window_adapter.clone();
                    r.log(&format!(
                        "window adapter (low power): {}",
                        adapter.as_deref().unwrap_or("unknown")
                    ));
                    self.display = write_system(&self.exe, &mut r);
                    let mut s = lock(&self.status);
                    s.folder = Some(r.root.clone());
                    s.say(format!("report folder: {}", r.root.display()));
                    self.report = Some(r);
                }
                Err(e) => lock(&self.status).say(format!("no report folder: {e}")),
            }
        }
        self.report.as_mut()
    }

    fn run(&mut self, spec: RunSpec) {
        let fly = flythrough_beside(&self.exe);
        let words = format!(
            "{} {}",
            spec.scene.name(),
            match spec.mode {
                PathMode::Bench => "benchmark",
                PathMode::Tour => "tour",
            }
        );
        if !fly.exists() {
            lock(&self.status).say(format!("no fly-through at {}", fly.display()));
            return;
        }
        let status = Arc::clone(&self.status);
        let display = self.display;
        let Some(report) = self.report() else { return };
        let dir = report.next_run(spec.scene, spec.mode);
        let args = fly_args(&spec, display, &dir);
        report.log(&format!("run: {} {}", fly.display(), args.join(" ")));
        lock(&status).busy = Some(format!("running the {words}: Esc in its window ends it"));
        let mut cmd = Process::new(&fly);
        cmd.args(&args);
        let outcome = run_child(cmd, &dir);
        let line = match &outcome {
            Ok(o) => format!("{words}: {}", o.row()),
            Err(e) => format!("{words}: did not start: {e}"),
        };
        report.log(&format!("exit: {line}"));
        let mut s = lock(&status);
        s.say(line);
        if let Ok(Outcome::Failed { tail, .. }) = &outcome {
            for t in tail {
                s.say(format!("  {t}"));
            }
        }
    }

    fn save(&mut self) {
        let status = Arc::clone(&self.status);
        lock(&status).busy = Some("saving the report".to_owned());
        let Some(report) = self.report() else { return };
        report.log("save: zip");
        let saved = report.zip();
        let mut s = lock(&status);
        match saved {
            Ok(zip) => {
                s.say(format!("saved {}", zip.display()));
                s.zip = Some(zip);
            }
            Err(e) => s.say(format!("save failed: {e}")),
        }
    }
}

/// Writes the report's `system.json` through a child of this executable (its display probe
/// needs an event loop of its own), logs the command and its exit, and returns the first
/// display's size.
fn write_system(exe: &Path, report: &mut ReportDir) -> Option<[u32; 2]> {
    let path = report.root.join("system.json");
    report.log(&format!(
        "system: {} --system-json {} --displays",
        exe.display(),
        path.display()
    ));
    let status = Process::new(exe)
        .arg("--system-json")
        .arg(&path)
        .arg("--displays")
        .stdout(std::process::Stdio::null())
        .status();
    report.log(&format!("system: exit {status:?}"));
    let v = json::parse(&std::fs::read_to_string(&path).ok()?).ok()?;
    let size = v
        .get("displays")?
        .as_array()?
        .first()?
        .get("size")?
        .as_array()?;
    let n = |i: usize| size.get(i).and_then(Value::as_f64).map(|v| v as u32);
    Some([n(0)?, n(1)?])
}

/// The launcher's window.
struct Launcher {
    menu: Menu,
    status: Arc<Mutex<Status>>,
    jobs: Sender<Job>,
    hud: Option<HudOverlay>,
    /// `--auto`: the jobs it sent; it quits once they are done.
    auto: Option<u32>,
    text: String,
    colours: Vec<HudColour>,
}

const MENU_KEYS: [KeyCode; 16] = [
    KeyCode::ArrowUp,
    KeyCode::ArrowDown,
    KeyCode::ArrowLeft,
    KeyCode::ArrowRight,
    KeyCode::Enter,
    KeyCode::NumpadEnter,
    KeyCode::Escape,
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
];

impl Launcher {
    /// Acts on a menu command; `Exit` for a quit while idle.
    fn command(&mut self, c: Command) -> Flow {
        let busy = lock(&self.status).busy.is_some();
        let job = match c {
            Command::Quit if busy => {
                lock(&self.status).say("a run is going: Esc in its window ends it".to_owned());
                return Flow::Continue;
            }
            Command::Quit => return Flow::Exit,
            Command::OpenFolder => {
                let folder = lock(&self.status).folder.clone();
                match folder {
                    Some(f) => open_folder(&f),
                    None => lock(&self.status).say("no report folder yet".to_owned()),
                }
                return Flow::Continue;
            }
            _ if busy => {
                lock(&self.status).say("busy: wait for the run to end".to_owned());
                return Flow::Continue;
            }
            Command::Run(spec) => Job::Run(spec),
            Command::RunAll(output, scale) => Job::RunAll(output, scale),
            Command::SaveReport => Job::Save,
        };
        lock(&self.status).busy = Some("starting".to_owned());
        let _ = self.jobs.send(job);
        Flow::Continue
    }

    fn write_text(&mut self) {
        self.text.clear();
        self.colours.clear();
        self.text.push_str(&format!(
            "vxe-tester {}: arrows and Enter, or a number\n\n",
            env!("CARGO_PKG_VERSION")
        ));
        self.colours.extend([HudColour::White, HudColour::White]);
        self.menu.write(&mut self.text, &mut self.colours);
        let s = lock(&self.status);
        self.text.push('\n');
        self.colours.push(HudColour::White);
        if let Some(b) = &s.busy {
            self.text.push_str(b);
            self.text.push('\n');
            self.colours.push(HudColour::Yellow);
        }
        for l in &s.lines {
            self.text.push_str(l);
            self.text.push('\n');
            self.colours
                .push(if l.contains("failed") || l.starts_with("  ") {
                    HudColour::Red
                } else {
                    HudColour::White
                });
        }
    }
}

fn open_folder(path: &Path) {
    let opener = if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    if let Err(e) = Process::new(opener).arg(path).spawn() {
        eprintln!("vxe-tester: {opener} {}: {e}", path.display());
    }
}

impl App for Launcher {
    fn init(&mut self, gpu: &Gpu) {
        let info = gpu.adapter.get_info();
        let adapter = format!("{} ({})", info.name, vxe_app::backend_label(info.backend));
        println!("vxe-tester: window on {adapter}");
        lock(&self.status).window_adapter = Some(adapter);
        self.hud = Some(HudOverlay::new(&gpu.device, gpu.surface_format, TEXT_SCALE));
        if self.auto.is_some() {
            let _ = self.jobs.send(Job::RunAll(Output::Hd720, Scale::OneHalf));
            let _ = self.jobs.send(Job::Save);
            lock(&self.status).busy = Some("starting".to_owned());
        }
    }

    fn frame(&mut self, gpu: &Gpu, input: &Input, _dt: f32, surface: &wgpu::TextureView) -> Flow {
        if self.auto.is_none() {
            for k in MENU_KEYS {
                if input.pressed(k)
                    && let Some(c) = self.menu.key(k)
                    && self.command(c) == Flow::Exit
                {
                    return Flow::Exit;
                }
            }
        } else if let Some(n) = self.auto {
            let s = lock(&self.status);
            if s.done >= n && s.busy.is_none() {
                return Flow::Exit;
            }
        }
        self.write_text();
        let mut enc = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let _clear = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("launcher clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: surface,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.02,
                            g: 0.02,
                            b: 0.03,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }
        if let Some(hud) = self.hud.as_mut() {
            hud.set_lines(&gpu.queue, &self.text, &self.colours);
            hud.encode(&mut enc, surface, gpu.size[0], gpu.size[1]);
        }
        gpu.queue.submit([enc.finish()]);
        Flow::Continue
    }
}

fn launcher(auto: bool) {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("vxe-tester"));
    let status = Arc::new(Mutex::new(Status::default()));
    let (jobs, rx) = channel();
    let worker = Worker {
        exe,
        report: None,
        display: None,
        status: Arc::clone(&status),
    };
    let handle = std::thread::spawn(move || worker.serve(rx));
    let app = Launcher {
        menu: Menu::default(),
        status: Arc::clone(&status),
        jobs,
        hud: None,
        auto: auto.then_some(2),
        text: String::new(),
        colours: Vec::new(),
    };
    // The launcher draws a menu: the low-power adapter, vsync and ten frames a second keep it
    // off the GPU and the cores the runs measure.
    let config = ShellConfig {
        title: "vxe-tester".to_owned(),
        size: [960, 600],
        present_mode: wgpu::PresentMode::AutoVsync,
        power_preference: wgpu::PowerPreference::LowPower,
        pacing: Pacing::Cap { hz: LAUNCHER_HZ },
        ..ShellConfig::default()
    };
    let result = vxe_app::run(config, app);
    // The window is gone; a run still going ends with its own window.
    drop(handle);
    let s = lock(&status);
    for l in &s.lines {
        println!("{l}");
    }
    if let Err(e) = result {
        eprintln!("vxe-tester: {e}");
        std::process::exit(1);
    }
}
