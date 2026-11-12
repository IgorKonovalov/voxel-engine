//! The settle rig's readings (`tests/support/settle.rs`): how fast the light settles after a
//! fly-in, a lamp toggle, a dig and a fast yaw, the dots a turn, a flight or a dig leaves, the
//! world-keyed flicker, the probes inside rock, the lamp channel's reach, step and freeze, and
//! the frame's cost in flight, on the level's ore veins at `stillness`'s poses, on the
//! fly-through's default look (`LookPreset::DEFAULT`, FSR 2 at 1.5).
//!
//! `cargo run --release -p flythrough --example settle -- --read NAME[,NAME...] [FLAGS]`.
//! Each reading prints lines and asserts nothing; `--help` lists the readings and the flags.
//! Every reading runs on the adapter `--adapter` names (unset: lavapipe). The filters narrow
//! the runs, each a comma list: `--poses` (patch, vein), `--events` (flyin, flyin-pinned,
//! toggle, dig, dig100, yaw10, yaw30, yaw118, yaw180), `--lamp` and `--indirect` (on, off),
//! `--auto` (pinned, auto), `--upscaler` (up, s1, bridge) and `--k-runs` (yaw, flight).
//! `settle_readings` takes each run's `N_step` from the composite's `sigma_view` in a
//! `wander_readings` log on the same adapter (`--wander-log`); `--dry` prints the `N_step` of
//! each run's first seed and runs no more.

#[path = "../tests/support/mod.rs"]
mod support;

use flythrough::cli::table::{Flag, Section, Table, Val};
use support::settle::*;
use support::stillness::{Pose, VEIN_POSE, VeinLevel, vein_level_on};
use support::{SyncNow, adapter_device, stillness};
use vxe_render::glam::Vec3;
use vxe_render::{Camera, Exposure, Frame, LightParams, Lighting};

/// The flags as read; a list left unset takes each reading's own default.
struct Args {
    read: Vec<String>,
    adapter: String,
    poses: Option<Vec<String>>,
    events: Option<Vec<String>>,
    lamp: Option<Vec<String>>,
    indirect: Option<Vec<String>>,
    auto: Option<Vec<String>>,
    upscaler: Option<Vec<String>>,
    k_runs: Option<Vec<String>>,
    wander_log: Option<String>,
    dry: bool,
    long_frames: usize,
    probe_moves: Option<Vec<String>>,
    probe_move: Option<f32>,
    still_settle: usize,
    probe_freeze: Option<Vec<u32>>,
    filmstrip: Option<String>,
    help: bool,
}

impl Default for Args {
    fn default() -> Self {
        Args {
            read: Vec::new(),
            adapter: String::new(),
            poses: None,
            events: None,
            lamp: None,
            indirect: None,
            auto: None,
            upscaler: None,
            k_runs: None,
            wander_log: None,
            dry: false,
            long_frames: 8192,
            probe_moves: None,
            probe_move: None,
            still_settle: 1200,
            probe_freeze: None,
            filmstrip: None,
            help: false,
        }
    }
}

/// A comma list; an empty value is no list (the reading's default).
fn comma(v: &Val<'_>) -> Option<Vec<String>> {
    (!v.text.is_empty()).then(|| v.text.split(',').map(str::to_owned).collect())
}

/// A row setting one of the comma-list filters.
fn list_flag(
    names: &'static [&'static str],
    value: &'static str,
    help: &'static str,
    field: fn(&mut Args) -> &mut Option<Vec<String>>,
) -> Flag<Args> {
    Flag::value(names, value, help, move |a: &mut Args, v| {
        *field(a) = comma(v);
        Ok(())
    })
}

fn filter_flags() -> Vec<Flag<Args>> {
    vec![
        list_flag(
            &["--poses"],
            "LIST",
            "the poses: patch, vein (and spawn for strip_flight_dot_readings); default all",
            |a| &mut a.poses,
        ),
        list_flag(
            &["--events"],
            "LIST",
            "the events: flyin, flyin-pinned, toggle, dig, dig100, yaw10, yaw30, yaw118, yaw180; default the reading's",
            |a| &mut a.events,
        ),
        list_flag(
            &["--lamp"],
            "LIST",
            "the lamp states: on, off; default both",
            |a| &mut a.lamp,
        ),
        list_flag(
            &["--indirect"],
            "LIST",
            "the indirect states: on, off; default both (flight_cost_readings: on)",
            |a| &mut a.indirect,
        ),
        list_flag(
            &["--auto"],
            "LIST",
            "the exposures: pinned, auto; default both",
            |a| &mut a.auto,
        ),
        list_flag(
            &["--upscaler"],
            "LIST",
            "the upscalers: up (FSR 2 at 1.5), s1 (scale 1), bridge (ours over FACE); default up",
            |a| &mut a.upscaler,
        ),
        list_flag(
            &["--k-runs"],
            "LIST",
            "the flicker runs: yaw, flight; default both",
            |a| &mut a.k_runs,
        ),
        Flag::value(
            &["--probe-freeze"],
            "LIST",
            "the lamp channel's freeze counts to run each setting at; default the renderer's",
            |a: &mut Args, v| {
                a.probe_freeze = match comma(v) {
                    None => None,
                    Some(l) => Some(
                        l.iter()
                            .map(|n| n.parse().map_err(|_| v.takes("freeze counts")))
                            .collect::<Result<_, _>>()?,
                    ),
                };
                Ok(())
            },
        ),
        Flag::value(
            &["--probe-move"],
            "H",
            "the lamp channel's moving-lamp blend h_move for every run; default the renderer's",
            |a: &mut Args, v| {
                a.probe_move = Some(v.parse()?);
                Ok(())
            },
        ),
    ]
}

fn run_flags() -> Vec<Flag<Args>> {
    vec![
        Flag::value(
            &["--wander-log"],
            "PATH",
            "a wander_readings log on this adapter: settle_readings' N_step comes from it",
            |a: &mut Args, v| {
                a.wander_log = Some(v.string());
                Ok(())
            },
        ),
        Flag::switch(
            &["--dry"],
            "settle_readings: print each pinned run's N_step from its first seed, run no more",
            |a: &mut Args| a.dry = true,
        ),
        Flag::value(
            &["--long-frames"],
            "N",
            "long_series' frames after the event (8192)",
            |a: &mut Args, v| {
                a.long_frames = v.parse()?;
                Ok(())
            },
        ),
        list_flag(
            &["--probe-moves"],
            "LIST",
            "lamp_step_readings' h_move values (0.5,0.8,0.9)",
            |a| &mut a.probe_moves,
        ),
        Flag::value(
            &["--still-settle"],
            "N",
            "still_lamp_composite_readings' frames before the read (1200)",
            |a: &mut Args, v| {
                a.still_settle = v.parse()?;
                Ok(())
            },
        ),
        Flag::value(
            &["--filmstrip"],
            "TAG",
            "the settle runs write contact sheets under live/filmstrip/TAG/",
            |a: &mut Args, v| {
                a.filmstrip = (!v.text.is_empty()).then(|| v.string());
                Ok(())
            },
        ),
    ]
}

fn table() -> Table<Args> {
    Table {
        usage: "settle --read NAME[,NAME...] [FLAGS]",
        sections: vec![
            Section {
                title: "Readings",
                flags: vec![
                    Flag::value(
                        &["--read"],
                        "LIST",
                        "the readings to run, in order (listed below)",
                        |a: &mut Args, v| {
                            a.read = comma(v).unwrap_or_default();
                            Ok(())
                        },
                    ),
                    Flag::value(
                        &["--adapter"],
                        "NAME",
                        "the first Vulkan adapter whose name contains NAME (rtx, radeon); unset or fallback: lavapipe",
                        |a: &mut Args, v| {
                            a.adapter = v.string();
                            Ok(())
                        },
                    ),
                    Flag::switch(
                        &["--help", "-h"],
                        "print this help and exit",
                        |a: &mut Args| a.help = true,
                    ),
                ],
            },
            Section {
                title: "Filters",
                flags: filter_flags(),
            },
            Section {
                title: "Runs",
                flags: run_flags(),
            },
        ],
    }
}

/// A reading's name, its one line for `--help` and its run.
type Reading = (&'static str, &'static str, fn(&Args));

/// The readings `--read` names.
const READINGS: [Reading; 17] = [
    (
        "dot_readings",
        "the dots after the fast yaws, the slow yaw and the two digs (with their settle)",
        dot_readings,
    ),
    (
        "eye_lamp_off_readings",
        "the eye's settled exposure, 600 frames lamp on then 900 off, at each pose",
        eye_lamp_off_readings,
    ),
    (
        "eye_readings",
        "the eye's settled exposure at each pose and the spawn after 1800 frames lamp on",
        eye_readings,
    ),
    (
        "flicker_readings",
        "the world-keyed flicker K at the yaw and the flight, each pose and setting",
        flicker_readings,
    ),
    (
        "flight_cost_readings",
        "the frame's per-pass cost at 1080p from 720p in the steady flight (timestamps)",
        flight_cost_readings,
    ),
    (
        "frozen_share_readings",
        "the volume's and the view's frozen probes every 64 frames to 1024, at rest",
        frozen_share_readings,
    ),
    (
        "lamp_reach_readings",
        "the lamp channel's light by band of distance from the lamp, over the direct D",
        lamp_reach_readings,
    ),
    (
        "lamp_step_readings",
        "the gather after a 2-voxel lamp step at each h_move: frames to 90 %, overshoot",
        lamp_step_readings,
    ),
    (
        "long_series",
        "the gather's and the composite's mean log2 every 128 frames after an event",
        long_series,
    ),
    (
        "path_flight_dot_readings",
        "the dots along the rig's steady flight",
        path_flight_dot_readings,
    ),
    (
        "probe_census_readings",
        "the share of traced probes inside rock at frames 8 .. 1024 at rest",
        probe_census_readings,
    ),
    (
        "reading_adapter",
        "the adapter --adapter picks",
        reading_adapter,
    ),
    (
        "settle_readings",
        "the settle of every part after each event, pose and setting (needs --wander-log)",
        settle_readings,
    ),
    (
        "still_lamp_composite_readings",
        "the output's moved pixels per frame with the lamp still, bounce on and off",
        still_lamp_composite_readings,
    ),
    (
        "strip_flight_dot_readings",
        "the strip tool's flight's dots, each pose and the spawn",
        strip_flight_dot_readings,
    ),
    (
        "strip_path_readings",
        "the strip flight's camera every 12th frame: air or rock (CPU only)",
        strip_path_readings,
    ),
    (
        "wander_readings",
        "the still-camera wander of every part, and the gather's share of the variance",
        wander_readings,
    ),
];

fn help() -> String {
    let mut out = table().help();
    out.push_str("\nReadings (--read):\n");
    let width = READINGS.iter().map(|r| r.0.len()).max().unwrap_or(0);
    for (name, line, _) in READINGS {
        out.push_str(&format!("  {name:width$}  {line}\n"));
    }
    out
}

fn main() {
    let mut args = Args::default();
    if let Err(e) = table().parse_into(std::env::args().skip(1), &mut args) {
        eprintln!("{e}");
        std::process::exit(2);
    }
    if args.help {
        print!("{}", help());
        return;
    }
    if args.read.is_empty() {
        eprintln!("name a reading with --read (see --help)");
        std::process::exit(2);
    }
    let mut runs = Vec::new();
    for name in &args.read {
        let Some(&(_, _, run)) = READINGS.iter().find(|r| r.0 == name) else {
            eprintln!("unknown reading {name} (see --help)");
            std::process::exit(2);
        };
        runs.push(run);
    }
    for run in runs {
        run(&args);
    }
}

// ---- The filters ----

fn list(given: &Option<Vec<String>>, all: &[&str]) -> Vec<String> {
    match given {
        Some(v) => v.clone(),
        None => all.iter().map(|s| (*s).to_owned()).collect(),
    }
}

fn on_off(given: &Option<Vec<String>>) -> Vec<bool> {
    list(given, &["on", "off"])
        .iter()
        .map(|s| s == "on")
        .collect()
}

impl Args {
    fn poses(&self) -> Vec<Pose> {
        let want = list(&self.poses, &["patch", "vein"]);
        POSES
            .into_iter()
            .filter(|p| want.iter().any(|w| w == p.name))
            .collect()
    }

    /// The freeze counts the settings run at: `--probe-freeze`'s, or the renderer's default.
    fn freezes(&self) -> Vec<Option<u32>> {
        match &self.probe_freeze {
            Some(v) => v.iter().map(|&n| Some(n)).collect(),
            None => vec![None],
        }
    }

    fn settings(&self) -> Vec<Setting> {
        let auto: Vec<bool> = list(&self.auto, &["pinned", "auto"])
            .iter()
            .map(|s| s == "auto")
            .collect();
        let up = list(&self.upscaler, &["up"]);
        let mut out = Vec::new();
        for probe_freeze in self.freezes() {
            for up in &up {
                for &lamp in &on_off(&self.lamp) {
                    for &indirect in &on_off(&self.indirect) {
                        for &a in &auto {
                            out.push(Setting {
                                lamp,
                                indirect,
                                auto: a,
                                upscaler: up != "s1",
                                bridge: up == "bridge",
                                probe_freeze,
                                probe_move: self.probe_move,
                            });
                        }
                    }
                }
            }
        }
        out
    }

    fn events(&self, all: &[&str]) -> Vec<Event> {
        list(&self.events, all)
            .iter()
            .map(|e| match e.as_str() {
                "flyin" => Event::FlyIn { pinned: false },
                "flyin-pinned" => Event::FlyIn { pinned: true },
                "toggle" => Event::Toggle,
                "dig" => Event::Dig,
                "dig100" => Event::DigFar,
                "yaw10" => Event::Yaw {
                    frames: 10,
                    degrees: 90,
                },
                "yaw30" => Event::Yaw {
                    frames: 30,
                    degrees: 90,
                },
                "yaw118" => Event::Yaw {
                    frames: 118,
                    degrees: 90,
                },
                "yaw180" => Event::Yaw {
                    frames: 20,
                    degrees: 180,
                },
                other => panic!("unknown event {other}"),
            })
            .collect()
    }

    fn k_runs(&self) -> Vec<KRun> {
        list(&self.k_runs, &["yaw", "flight"])
            .iter()
            .map(|r| match r.as_str() {
                "yaw" => KRun::Yaw,
                "flight" => KRun::Path,
                other => panic!("unknown flicker run {other}"),
            })
            .collect()
    }

    /// The rig's setting for the lamp channel's readings: the lamp and its bounce on, the
    /// pose's pinned exposure, scale 1 with no upscaler.
    fn channel_setting(&self) -> Setting {
        Setting {
            lamp: true,
            indirect: true,
            auto: false,
            upscaler: false,
            bridge: false,
            probe_freeze: None,
            probe_move: self.probe_move,
        }
    }

    /// The default look's setting: lamp and indirect on, FSR 2 at 1.5, `auto` as given.
    fn default_setting(&self, auto: bool) -> Setting {
        Setting {
            lamp: true,
            indirect: true,
            auto,
            upscaler: true,
            bridge: false,
            probe_freeze: None,
            probe_move: self.probe_move,
        }
    }
}

fn reading_level(args: &Args) -> Option<VeinLevel> {
    vein_level_on(|pool| adapter_device(&args.adapter, "--adapter ", pool))
}

/// The poses and the spawn at the eye's settled exposure: the eye's value at each against the
/// pose's constant, lamp on.
fn eye_at(args: &Args, level: &VeinLevel, pose: &Pose) -> f32 {
    eye_exposure(level, pose, args.default_setting(true), 1800)
}

// ---- The settle and its dots ----

/// The composite's `sigma_view` per (pose, lamp, indirect, upscaler) from a `wander_readings`
/// log (`--wander-log`), on the same adapter: the step settle's `N_step` comes from it.
fn wander_sigmas(args: &Args, adapter: &str) -> Vec<(String, f32)> {
    let path = args
        .wander_log
        .as_ref()
        .expect("--wander-log names a wander_readings log on this adapter");
    let text = std::fs::read_to_string(path).expect("the wander log reads");
    let head = format!("reading[{adapter}]: wander ");
    text.lines()
        .filter_map(|l| {
            let rest = l.strip_prefix(&head)?;
            let (name, v) = rest.split_once(" composite: sigma_view ")?;
            let v: f32 = v.split_whitespace().next()?.parse().ok()?;
            Some((name.replace(" pinned", ""), v))
        })
        .collect()
}

fn sigma_of(sigmas: &[(String, f32)], pose: &Pose, lamp: bool, setting: Setting) -> f32 {
    let on = |b: bool| if b { "on" } else { "off" };
    let key = format!(
        "{} lamp {} indirect {} {}{}",
        pose.name,
        on(lamp),
        on(setting.indirect),
        setting.upscaler_name(),
        setting.freeze_words()
    );
    sigmas
        .iter()
        .find(|(k, _)| *k == key)
        .unwrap_or_else(|| panic!("no wander reading for {key} in --wander-log"))
        .1
}

/// What every run of one settle reading shares.
struct SettleCtx {
    adapter: String,
    sigmas: Vec<(String, f32)>,
    tag: Option<String>,
    dry: bool,
    seeded: bool,
}

/// The settle runs of `all` events (narrowed by the filters). `seeded`: each pinned run
/// repeats over `N_step` seeds and its Auto run over the same; otherwise one seed (the dots'
/// reading, with no wander log needed).
fn run_settle(args: &Args, all: &[&str], seeded: bool) {
    let Some(mut level) = reading_level(args) else {
        return;
    };
    let adapter = level.adapter.clone();
    let sigmas = if seeded {
        wander_sigmas(args, &adapter)
    } else {
        Vec::new()
    };
    let ctx = SettleCtx {
        adapter,
        sigmas,
        tag: args.filmstrip.clone(),
        // `--dry`: print each pinned run's `N_step` from its first seed, run no more.
        dry: args.dry,
        seeded,
    };
    for pose in args.poses() {
        for event in args.events(all) {
            let mut pinned_strip: Option<Vec<Vec<u8>>> = None;
            for setting in args.settings() {
                settle_one(&ctx, &mut level, &pose, event, setting, &mut pinned_strip);
            }
        }
    }
}

/// One settle run (with its seeds) and its lines; a pinned run's strip is kept in
/// `pinned_strip` for the Auto run's sheet after it.
fn settle_one(
    ctx: &SettleCtx,
    level: &mut VeinLevel,
    pose: &Pose,
    event: Event,
    setting: Setting,
    pinned_strip: &mut Option<Vec<Vec<u8>>>,
) {
    let adapter = &ctx.adapter;
    #[allow(
        clippy::disallowed_methods,
        reason = "a reading's frames a second, not simulation"
    )]
    let t0 = std::time::Instant::now();
    let r = settle_run(level, pose, setting, event, 0, ctx.tag.as_deref());
    if event.is_dig() {
        reset_world(level);
    }
    let name = format!("{} {} {}", pose.name, event.name(), setting.name());
    let mut sum = SeedSum::new();
    sum.add(&r);
    let lamp_after = if event == Event::Toggle {
        !setting.lamp
    } else {
        setting.lamp
    };
    // The Auto run is one seed: it reads the exposure's own settle, which the eye's smooth
    // adaptation carries with little seed noise.
    let n = if !ctx.seeded || setting.auto {
        Some(1)
    } else {
        let (parts, _) = sum.settles();
        let d = parts[0].1.d;
        let sigma = sigma_of(&ctx.sigmas, pose, lamp_after, setting);
        let n = n_step(sigma, d);
        println!(
            "reading[{adapter}]: n_step {name}: composite single-seed D {d:+.4}, sigma_view {sigma:.4}, (30 sigma / |D|)^2 {:.2}, N_step {}",
            (30.0 * sigma / d.abs().max(1e-9)).powi(2),
            n.map_or("under the wander".into(), |n| n.to_string())
        );
        n
    };
    let runs = if ctx.dry { 1 } else { n.unwrap_or(1) };
    for k in 1..runs {
        let rk = settle_run(level, pose, setting, event, k * SEED_STRIDE, None);
        if event.is_dig() {
            reset_world(level);
        }
        sum.add(&rk);
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a reading's frames a second, not simulation"
    )]
    let secs = t0.elapsed().as_secs_f64();
    print_settle(adapter, &name, &sum, n.is_none(), setting.auto, secs);
    if event == Event::Toggle {
        let open = r.window.iter().take_while(|&&o| o).count();
        let later = r.window.iter().skip(open).filter(|&&o| o).count();
        println!(
            "reading[{adapter}]: window {name}: open frames 1 to {open}, {later} more open later in 1 to 64"
        );
    }
    if let Some(d) = &r.dots {
        println!("reading[{adapter}]: dots {name}: {}", d.line());
    }
    if setting.auto {
        let e: Vec<String> = r.strip_exposure.iter().map(|e| format!("{e:.3}")).collect();
        println!(
            "reading[{adapter}]: auto exposure {name} at frames {STRIP_FRAMES:?}: {}",
            e.join(" ")
        );
        if let (Some(tag), Some(p)) = (&ctx.tag, &*pinned_strip) {
            let size = if setting.upscaler {
                OUTPUT
            } else {
                stillness::SIZE
            };
            auto_sheet(
                tag,
                &format!(
                    "{}-{}-lamp{}-ind{}-auto-vs-pinned",
                    pose.name,
                    event.name().replace(' ', "-"),
                    u8::from(setting.lamp),
                    u8::from(setting.indirect)
                ),
                size,
                p,
                &r.strip,
            );
        }
    } else {
        *pinned_strip = Some(r.strip);
    }
}

/// A settle run's step, per-tile and curve lines over its seeds.
fn print_settle(adapter: &str, name: &str, sum: &SeedSum, under: bool, auto: bool, secs: f64) {
    let (steps, exposure) = sum.settles();
    let under = if under {
        " (under the wander: one seed)"
    } else {
        ""
    };
    let parts: Vec<String> = steps
        .iter()
        .map(|(p, st, _)| format!("{} {}", p.name(), st.cell()))
        .collect();
    println!(
        "reading[{adapter}]: step {name}, {} seeds{under}: {}; exposure {}; {:.0} s",
        sum.n,
        parts.join(", "),
        exposure.cell(),
        secs
    );
    let tiles: Vec<String> = steps
        .iter()
        .map(|(p, _, s)| format!("{} {}", p.name(), s.cell()))
        .collect();
    println!(
        "reading[{adapter}]: per-tile {name}, {} seeds: {}; {} counted tiles",
        sum.n,
        tiles.join(", "),
        steps[0].2.counted
    );
    for (p, st, s) in &steps {
        println!(
            "reading[{adapter}]: curve {name} {}: M every 8 frames {}",
            p.name(),
            st.curve_line()
        );
        println!(
            "reading[{adapter}]: tile-curve {name} {}: settled % {}",
            p.name(),
            s.curve_line()
        );
    }
    if auto {
        println!(
            "reading[{adapter}]: curve {name} exposure: log2 every 8 frames {}",
            exposure.curve_line()
        );
    }
}

/// The dots after the fast yaws, the slow yaw (90 degrees over 118 frames) and the two digs
/// (with their settle).
fn dot_readings(args: &Args) {
    run_settle(
        args,
        &["yaw10", "yaw30", "yaw118", "yaw180", "dig", "dig100"],
        false,
    );
}

/// The settle of every part after each event, pose and setting.
fn settle_readings(args: &Args) {
    run_settle(args, &["toggle", "dig"], true);
}

/// A long series after an event: the gather's and the composite's mean tile log2 over the
/// full tiles every 128 frames to `--long-frames` (default 8192), still camera, at each pose
/// and setting the filters pick (events toggle or dig).
fn long_series(args: &Args) {
    let Some(mut level) = reading_level(args) else {
        return;
    };
    let adapter = level.adapter.clone();
    for pose in args.poses() {
        for event in args.events(&["toggle"]) {
            for setting in args.settings().into_iter().filter(|s| !s.auto) {
                let rows = long_run(&mut level, &pose, setting, event, args.long_frames);
                if event.is_dig() {
                    reset_world(&mut level);
                }
                let line: Vec<String> = rows
                    .iter()
                    .map(|(f, g, c, e)| format!("{f}:{g:.3}/{c:.3}/{e:.3}"))
                    .collect();
                println!(
                    "reading[{adapter}]: long {} {} {}: frame:gather/composite/emissive mean log2 {}",
                    pose.name,
                    event.name(),
                    setting.name(),
                    line.join(" ")
                );
            }
        }
    }
}

/// The dots along the rig's steady flight, whose path has air around every step.
fn path_flight_dot_readings(args: &Args) {
    let Some(level) = reading_level(args) else {
        return;
    };
    let adapter = level.adapter.clone();
    for pose in args.poses() {
        for setting in args.settings().into_iter().filter(|s| !s.auto) {
            let d = path_dots(&level, &pose, setting);
            println!(
                "reading[{adapter}]: path-flight dots {} {}: {}",
                pose.name,
                setting.name(),
                d.line()
            );
        }
    }
}

/// The strip tool's flight's dots, each pose and the spawn (at the eye's settled exposure).
fn strip_flight_dot_readings(args: &Args) {
    let Some(level) = reading_level(args) else {
        return;
    };
    let adapter = level.adapter.clone();
    let spawn = spawn_pose(eye_at(args, &level, &spawn_pose(1.0)));
    let all = [POSES[0], VEIN_POSE, spawn];
    let want = list(&args.poses, &["patch", "vein", "spawn"]);
    for pose in all.iter().filter(|p| want.iter().any(|w| w == p.name)) {
        for setting in args.settings().into_iter().filter(|s| !s.auto) {
            let d = strip_dots(&level, pose, setting);
            println!(
                "reading[{adapter}]: strip-flight dots {} {}: {}",
                pose.name,
                setting.name(),
                d.line()
            );
        }
    }
}

/// The strip flight's camera at each 12th frame: whether it is in air. CPU only.
fn strip_path_readings(_: &Args) {
    let source = flythrough::cave::CaveSource::new(
        1,
        vxe_world::ChunkExtent::new([0, 0, 0], flythrough::cave::LEVEL_CHUNKS),
    );
    let world = vxe_world::World::generate(
        source.extent,
        &source,
        &flythrough::with_emissive_ore(&flythrough::cave_materials()),
    );
    for pose in [POSES[0], VEIN_POSE, spawn_pose(1.0)] {
        let mut camera = pose.camera();
        let mut line = Vec::new();
        for f in 0..STRIP_FRAMES_FLOWN {
            let (fwd, _, _) = camera.basis();
            camera.position += fwd * STRIP_SPEED / 60.0;
            camera.yaw += STRIP_YAW.to_radians() / 60.0;
            let v = camera.position.to_array().map(|c| c.floor() as i32);
            let air = world.material_at(v) == Some(vxe_world::AIR);
            if f % 12 == 0 {
                line.push(if air { "air" } else { "rock" });
            }
        }
        println!(
            "reading: strip path {} every 12 frames: {}",
            pose.name,
            line.join(" ")
        );
    }
}

// ---- The eye, the flicker and the wander ----

/// The eye's settled exposure in the constants' own state: 600 frames lamp on, then 900 off.
fn eye_lamp_off_readings(args: &Args) {
    let Some(level) = reading_level(args) else {
        return;
    };
    let adapter = level.adapter.clone();
    for pose in POSES {
        println!(
            "reading[{adapter}]: eye {} 600 on, 900 off: {:.4} against the constant {:.4}",
            pose.name,
            eye_after_lamp_off(&level, &pose, args.probe_move),
            pose.exposure
        );
    }
}

/// The eye's settled exposure at each pose and the spawn, 1800 frames lamp on, against the
/// pose's constant.
fn eye_readings(args: &Args) {
    let Some(level) = reading_level(args) else {
        return;
    };
    let adapter = level.adapter.clone();
    for pose in POSES {
        println!(
            "reading[{adapter}]: eye {}: {:.4} after 1800 frames against the constant {:.4}",
            pose.name,
            eye_at(args, &level, &pose),
            pose.exposure
        );
    }
    let spawn = spawn_pose(1.0);
    println!(
        "reading[{adapter}]: eye spawn: {:.4} after 1800 frames",
        eye_at(args, &level, &spawn)
    );
}

/// The world-keyed flicker `K` at the yaw and the flight, each pose and setting: over frames
/// `SHIMMER_FROM ..` of the run and over the frames before them.
fn flicker_readings(args: &Args) {
    let Some(level) = reading_level(args) else {
        return;
    };
    let adapter = level.adapter.clone();
    for pose in args.poses() {
        for setting in args.settings().into_iter().filter(|s| !s.auto) {
            for run in args.k_runs() {
                let (late, early) = flicker_run(&level, &pose, setting, run);
                println!(
                    "reading[{adapter}]: flicker {} {} {}: from {SHIMMER_FROM} {}; before {SHIMMER_FROM} {}",
                    pose.name,
                    run.name(),
                    setting.name(),
                    late.cell(),
                    early.cell()
                );
            }
        }
    }
}

/// The still-camera wander (run (vi)) of every part at each pose and setting, pinned exposure,
/// and the gather's share of the composite's and the display's view-mean variance where both
/// indirect states are read: `(sigma_on^2 - sigma_off^2) / sigma_on^2`, which assumes the
/// gather's wander independent of the direct parts'.
fn wander_readings(args: &Args) {
    let Some(level) = reading_level(args) else {
        return;
    };
    let adapter = level.adapter.clone();
    for pose in args.poses() {
        let mut by: Vec<(Setting, Wanders)> = Vec::new();
        for setting in args.settings().into_iter().filter(|s| !s.auto) {
            let (w, fps) = wander_run(&level, &pose, setting, 0);
            let name = format!("{} {}", pose.name, setting.name());
            println!("reading[{adapter}]: wander {name}: {fps:.1} frames a second");
            for (p, w) in &w {
                println!(
                    "reading[{adapter}]: wander {name} {}: {}",
                    p.name(),
                    w.map_or("no counted tiles".into(), |w| w.line())
                );
            }
            by.push((setting, w));
        }
        for (s_on, w_on) in by.iter().filter(|(s, _)| s.indirect) {
            let Some((_, w_off)) = by
                .iter()
                .find(|(s, _)| !s.indirect && s.lamp == s_on.lamp && s.upscaler == s_on.upscaler)
            else {
                continue;
            };
            for part in [Part::Composite, Part::Display] {
                let get = |w: &[(Part, Option<Wander>)]| {
                    w.iter().find(|(p, _)| *p == part).and_then(|(_, w)| *w)
                };
                if let (Some(on), Some(off)) = (get(w_on), get(w_off)) {
                    let share =
                        (on.sigma_view.powi(2) - off.sigma_view.powi(2)) / on.sigma_view.powi(2);
                    println!(
                        "reading[{adapter}]: wander {} {}: gather's share of the {} view-mean variance {share:.3}",
                        pose.name,
                        s_on.name().replace("indirect on ", ""),
                        part.name()
                    );
                }
            }
        }
    }
}

// ---- The probes and the lamp channel ----

/// The frame's per-pass cost at 1080p from 720p in the steady flight, each pose, lamp on,
/// indirect as `--indirect` lists (default on), at each of `--probe-freeze`'s counts.
fn flight_cost_readings(args: &Args) {
    let Some(level) = reading_level(args) else {
        return;
    };
    let adapter = level.adapter.clone();
    let indirect: Vec<bool> = list(&args.indirect, &["on"])
        .iter()
        .map(|s| s == "on")
        .collect();
    for pose in args.poses() {
        for (freeze, indirect) in args
            .freezes()
            .into_iter()
            .flat_map(|f| indirect.iter().map(move |&i| (f, i)))
        {
            let lighting = Lighting {
                lamp: true,
                lamp_bounce: indirect,
                exposure: Exposure::Manual(pose.exposure),
                probe_freeze: freeze.unwrap_or(vxe_render::DEFAULT_PROBE_FREEZE),
                ..Lighting::default()
            };
            let Some(ms) = flight_cost(&level, &pose, lighting, KRun::Path, 120) else {
                println!("reading[{adapter}]: no TIMESTAMP_QUERY");
                return;
            };
            let line: Vec<String> = ms
                .iter()
                .filter(|(_, v)| *v > 0.0)
                .map(|(n, v)| format!("{n} {v:.3}"))
                .collect();
            let total: f64 = ms.iter().map(|(_, v)| v).sum();
            println!(
                "reading[{adapter}]: flight cost {} indirect {}{} 1920x1080 from 1280x720: total {total:.3} ms ({})",
                pose.name,
                if indirect { "on" } else { "off" },
                freeze.map_or(String::new(), |f| format!(" probe-freeze {f}")),
                line.join(", ")
            );
        }
    }
}

/// A's frozen share at rest: the volume's and the view's traced, inside and frozen probes every
/// 64 frames to 1024, at each of `--probe-freeze`'s counts, and the first frame all of the
/// view's traced probes outside rock read frozen.
fn frozen_share_readings(args: &Args) {
    let Some(level) = reading_level(args) else {
        return;
    };
    let adapter = level.adapter.clone();
    for pose in args.poses() {
        for setting in args.settings().into_iter().filter(|s| !s.auto) {
            let freeze = setting
                .probe_freeze
                .unwrap_or(vxe_render::DEFAULT_PROBE_FREEZE);
            let mut frame = make_frame(&level, &pose, setting);
            let light = LightParams::headlamp();
            let cam = pose.camera();
            let mut curve = Vec::new();
            let mut first = None;
            for f in 1..=1024usize {
                frame.step(&level.device, &level.queue, &level.gpu, &cam, &light);
                if f % 8 == 0 {
                    let c = freeze_census(&level, &frame, &cam, freeze);
                    if first.is_none()
                        && c.in_view[0] > 0
                        && c.in_view[2] + c.in_view[1] == c.in_view[0]
                    {
                        first = Some(f);
                    }
                    if f % 64 == 0 {
                        curve.push(format!(
                            "{f}:{:.3}/{:.3}",
                            f64::from(c.all[2]) / f64::from((c.all[0] - c.all[1]).max(1)),
                            f64::from(c.in_view[2])
                                / f64::from((c.in_view[0] - c.in_view[1]).max(1)),
                        ));
                    }
                }
            }
            println!(
                "reading[{adapter}]: frozen {} {}: view frozen at {first:?}; frame:share of volume/view {}",
                pose.name,
                setting.name(),
                curve.join(" ")
            );
        }
    }
}

/// Air over chunks `[0, 2)^3`, a rock block at x, y in `[40, 64)`, z in `[40, 64)`, and an ore
/// plate over it at z `[80, 84)`: the block's corner faces the camera, its faces lit from above.
fn ore_box() -> (vxe_world::World, Camera) {
    use vxe_world::*;
    let table = flythrough::with_emissive_ore(&flythrough::cave_materials());
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [2, 2, 2]),
        &UniformSource(AIR),
        &table,
    );
    let fill = |min: [i64; 3], max: [i64; 3], m| {
        world
            .apply(&EditOp::box_(
                Fixed3(min.map(|v| v * FIXED_ONE)),
                Fixed3(max.map(|v| v * FIXED_ONE)),
                Mode::Fill(m),
            ))
            .expect("the box is resident");
    };
    fill([40, 40, 40], [64, 64, 64], flythrough::cave::ROCK);
    fill([20, 20, 80], [90, 90, 84], flythrough::cave::ORE);
    // At the block's lowest corner on x and y, above its top: three faces in view.
    let camera = Camera {
        position: [30.5, 28.5, 70.5].into(),
        yaw: 45f32.to_radians(),
        pitch: (-35f32).to_radians(),
        vfov: 70f32.to_radians(),
    };
    (world, camera)
}

/// Bands of `REACH_BAND` voxels the reach reading sorts its probes and hits into.
const REACH_BAND: f32 = 16.0;
const REACH_BANDS: usize = 16;

/// The lamp channel's reach, read: under the lamp (on, no emissive light, the channel over the
/// whole 32-probe volume with no reach cut, its freeze 64), still for 600 frames at the
/// `ore_box` camera and at the level's spawn facing +x, the direct lamp irradiance `D` at the
/// wall the lamp lights at arm's length (the light pass's lamp part over the central 8 x 8
/// lighting pixels), then by 16-voxel band of the lamp-to-probe distance the brightest interior
/// texel of any traced probe outside rock, and by band of the lamp-to-hit distance the
/// brightest gather over the hit pixels, each over `D`. `probes::LAMP_REACH` is the band edge
/// past which every probe band reads under 0.01, rounded up to whole spacings.
fn lamp_reach_readings(args: &Args) {
    let Some(level) = reading_level(args) else {
        return;
    };
    let (device, queue) = (&level.device, &level.queue);
    let (box_world, box_camera) = ore_box();
    let mut box_gpu = vxe_render::GpuWorld::with_config(
        device,
        vxe_render::MirrorConfig {
            budget_bytes: 64 << 20,
            ..vxe_render::MirrorConfig::new(vxe_render::Window::Fixed(box_world.extent()))
        },
    );
    for _ in 0..2 {
        box_gpu
            .sync_now(&box_world, device, queue)
            .expect("the mirror fits");
    }
    let spawn = spawn_pose(VEIN_POSE.exposure).camera();
    for (name, gpu, camera) in [
        ("ore_box", &box_gpu, box_camera),
        ("spawn", &level.gpu, spawn),
    ] {
        lamp_reach_at(&level, name, gpu, camera);
    }
}

/// [`lamp_reach_readings`] at one camera over `gpu`.
fn lamp_reach_at(level: &VeinLevel, name: &str, gpu: &vxe_render::GpuWorld, camera: Camera) {
    let adapter = &level.adapter;
    let (device, queue) = (&level.device, &level.queue);
    let light = LightParams::headlamp();
    let size = [160u32, 90];
    let mut frame = Frame::new(device, size[0], size[1]);
    frame.set_resolution(device, vxe_render::LightingResolution::Full);
    frame.set_lighting(Lighting {
        lamp: true,
        candidates: 0,
        lamp_bounce: true,
        lamp_reach: 0.0,
        exposure: Exposure::Manual(1.0),
        ..Lighting::default()
    });
    frame.set_dt(1.0 / 60.0);
    for _ in 0..600 {
        frame.step(device, queue, gpu, &camera, &light);
    }
    let params = frame.params(gpu, &camera, &light);
    let lamp = Vec3::from(params.light_pos);
    let irr = vxe_render::debug::read_float(device, queue, frame.irradiance());
    let (cx, cy) = (size[0] / 2, size[1] / 2);
    let mut d_sum = 0.0f64;
    for y in cy - 4..cy + 4 {
        for x in cx - 4..cx + 4 {
            // The light pass's lamp part is its alpha (the emissive part is its rgb).
            d_sum += f64::from(irr[(y * size[0] + x) as usize][3]);
        }
    }
    let d = d_sum / 64.0;
    let v = frame.probes();
    let origin = v.origin_for(camera.position.to_array());
    let mut probe_band = [0.0f64; REACH_BANDS];
    let mut probe_count = [0u32; REACH_BANDS];
    for s in 0..v.count() {
        let w = v.slot_probe(origin, s);
        let p = v.read_probe(device, queue, s);
        if p.inside() || p.age() == 0 {
            continue;
        }
        let r = (Vec3::from(v.probe_position(w)) - lamp).length();
        let b = ((r / REACH_BAND) as usize).min(REACH_BANDS - 1);
        let hi = p
            .interior_irradiance()
            .iter()
            .map(|t| f64::from(t[0]))
            .fold(0.0, f64::max);
        probe_band[b] = probe_band[b].max(hi);
        probe_count[b] += 1;
    }
    let pos = vxe_render::debug::read_float(device, queue, frame.hit_positions());
    let info = vxe_render::debug::read_rgba16u(device, queue, frame.hit_records());
    let gather = vxe_render::debug::read_float(device, queue, frame.indirect_irradiance());
    let mut hit_band = [0.0f64; REACH_BANDS];
    for ((p, i), g) in pos.iter().zip(&info).zip(&gather) {
        if i[3] & 1 == 0 {
            continue;
        }
        let r = (Vec3::new(p[0], p[1], p[2]) - lamp).length();
        let b = ((r / REACH_BAND) as usize).min(REACH_BANDS - 1);
        hit_band[b] = hit_band[b].max(f64::from(g[0]));
    }
    let under = (0..REACH_BANDS)
        .find(|&b| probe_band[b..].iter().all(|&e| e < 0.01 * d))
        .map(|b| b as f32 * REACH_BAND);
    let fmt = |v: &[f64]| {
        v.iter()
            .map(|e| format!("{:.4}", e / d))
            .collect::<Vec<_>>()
            .join(" ")
    };
    println!(
        "reading[{adapter}]: lamp reach {name}: D {d:.4} at the central pixels; probe bands of {REACH_BAND} voxels (brightest texel / D) {}; probes per band {probe_count:?}; gather bands (brightest hit / D) {}; every probe band under 0.01 from {under:?} voxels",
        fmt(&probe_band),
        fmt(&hit_band)
    );
}

/// The gather's mean over the hit pixels.
fn mean_gather(level: &VeinLevel, frame: &Frame) -> f64 {
    let (d, q) = (&level.device, &level.queue);
    let g = vxe_render::debug::read_float(d, q, frame.indirect_irradiance());
    let info = vxe_render::debug::read_rgba16u(d, q, frame.hit_records());
    let (mut sum, mut n) = (0.0f64, 0u32);
    for (g, i) in g.iter().zip(&info) {
        if i[3] & 1 != 0 {
            sum += f64::from(g[0] + g[1] + g[2]) / 3.0;
            n += 1;
        }
    }
    sum / f64::from(n.max(1))
}

/// The lamp step at each `h_move` (`--probe-moves`, default 0.5, 0.8 and 0.9): at each pose
/// (`--poses`) with the lamp channel on, 600 still frames (every probe in reach frozen), then
/// the camera (and the lamp with it) stepped 2 voxels to its right in one frame and held, for
/// 600 frames (a step of the lamp's offset instead is a change of its parameters, which
/// restarts the channel): the gather's mean over the hit pixels each frame, the frames until it
/// has moved 90 % of the way from its value before the step to its value at the end, and the
/// largest overshoot past the end value as a share of the step.
fn lamp_step_readings(args: &Args) {
    let Some(level) = reading_level(args) else {
        return;
    };
    let moves: Vec<f32> = list(&args.probe_moves, &["0.5", "0.8", "0.9"])
        .iter()
        .map(|s| s.parse().expect("an h_move"))
        .collect();
    for pose in args.poses() {
        for &h in &moves {
            lamp_step_at(args, &level, &pose, h);
        }
    }
}

/// [`lamp_step_readings`] at one pose and `h_move`.
fn lamp_step_at(args: &Args, level: &VeinLevel, pose: &Pose, h: f32) {
    let adapter = &level.adapter;
    let mut frame = make_frame(level, pose, args.channel_setting());
    frame.set_lighting(Lighting {
        probe_move_blend: h,
        ..frame.lighting()
    });
    let mut cam = pose.camera();
    let light = LightParams::headlamp();
    for _ in 0..600 {
        frame.step(&level.device, &level.queue, &level.gpu, &cam, &light);
    }
    let before = mean_gather(level, &frame);
    let freeze = frame.lighting().probe_freeze;
    let frozen = |frame: &Frame, cam: &Camera| freeze_census(level, frame, cam, freeze).all[2];
    let mut census = vec![(0, frozen(&frame, &cam))];
    let (_, right, _) = cam.basis();
    cam.position += right * 2.0;
    let mut series = Vec::new();
    for f in 1..=600 {
        frame.step(&level.device, &level.queue, &level.gpu, &cam, &light);
        series.push(mean_gather(level, &frame));
        if [1, 8, 16, 64, 600].contains(&f) {
            census.push((f, frozen(&frame, &cam)));
        }
    }
    let end = *series.last().expect("frames ran");
    let step = end - before;
    let to90 = series
        .iter()
        .position(|&v| (v - before) / step >= 0.9)
        .map(|i| i + 1);
    let over = series
        .iter()
        .map(|&v| (v - end) / step)
        .fold(0.0f64, f64::max);
    println!(
        "reading[{adapter}]: lamp step {} h_move {h}: gather mean {before:.5} -> {end:.5} ({:+.1} %); frames to 90 % {to90:?}; overshoot {:.3} of the step; frozen probes at frames {census:?}; frames 1..=24 {:?}",
        pose.name,
        step / before * 100.0,
        over,
        series[..24]
            .iter()
            .map(|v| format!("{v:.5}"))
            .collect::<Vec<_>>()
    );
}

/// The share of traced probes inside rock at frames 8 .. 1024 at rest, indirect on.
fn probe_census_readings(args: &Args) {
    let Some(level) = reading_level(args) else {
        return;
    };
    let adapter = level.adapter.clone();
    for pose in args.poses() {
        let mut frame = make_frame(&level, &pose, args.default_setting(false));
        let light = LightParams::headlamp();
        let cam = pose.camera();
        let mut done = 0;
        for at in [8usize, 64, 256, 1024] {
            for _ in done..at {
                frame.step(&level.device, &level.queue, &level.gpu, &cam, &light);
            }
            done = at;
            let (traced, inside) = probe_census(&level, &frame);
            println!(
                "reading[{adapter}]: probes {} frame {at}: {traced} traced, {inside} inside rock ({:.4} of traced)",
                pose.name,
                f64::from(inside) / f64::from(traced.max(1))
            );
        }
    }
}

/// Prints the adapter `--adapter` picks.
fn reading_adapter(args: &Args) {
    let Some((_, _, name)) = adapter_device(&args.adapter, "--adapter ", 64 << 20) else {
        return;
    };
    println!("reading adapter: {name}");
}

/// The still image through the channel: at each pose with the lamp on and still and its bounce
/// on and off (scale 1, pinned exposure), after `--still-settle` frames (default 1200: the
/// freeze's 64 visits at 8 slices with room), the output's pixels that differ from the last
/// frame's over the next 64 frames, per frame, and the probe rays dispatched then.
fn still_lamp_composite_readings(args: &Args) {
    let Some(level) = reading_level(args) else {
        return;
    };
    for (pose, bounce) in args
        .poses()
        .into_iter()
        .flat_map(|p| [true, false].map(move |b| (p, b)))
    {
        still_lamp_at(args, &level, &pose, bounce);
    }
}

/// [`still_lamp_composite_readings`] at one pose and bounce state.
fn still_lamp_at(args: &Args, level: &VeinLevel, pose: &Pose, bounce: bool) {
    let adapter = &level.adapter;
    let settle = args.still_settle;
    let setting = Setting {
        indirect: bounce,
        ..args.channel_setting()
    };
    let mut frame = make_frame(level, pose, setting);
    let cam = pose.camera();
    let light = LightParams::headlamp();
    for _ in 0..settle {
        frame.step(&level.device, &level.queue, &level.gpu, &cam, &light);
    }
    let read =
        |frame: &Frame| vxe_render::debug::read_rgba8(&level.device, &level.queue, frame.target());
    let mut last = read(&frame);
    let mut moved = Vec::new();
    let mut rays = 0u64;
    let mut step_max = 0u8;
    assert_eq!(
        frame.internal_size(),
        frame.output_size(),
        "scale 1: a hit pixel is an output pixel"
    );
    for _ in 0..64 {
        frame.step(&level.device, &level.queue, &level.gpu, &cam, &light);
        rays += frame.traced_probe_rays();
        let now = read(&frame);
        let changed: Vec<usize> = now
            .chunks(4)
            .zip(last.chunks(4))
            .enumerate()
            .filter(|(_, (a, b))| a[..3] != b[..3])
            .map(|(i, (a, b))| {
                for c in 0..3 {
                    step_max = step_max.max(a[c].abs_diff(b[c]));
                }
                i
            })
            .collect();
        moved.push(changed.len());
        last = now;
    }
    println!(
        "reading[{adapter}]: still lamp {} bounce {bounce}: after {settle} frames, pixels moved per frame over 64 {moved:?} (of {}); probe rays dispatched over them {rays} (a frozen probe returns at once); largest channel step {step_max}",
        pose.name,
        last.len() / 4
    );
}
