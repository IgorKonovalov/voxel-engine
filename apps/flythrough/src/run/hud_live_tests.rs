//! The HUD and the banner follow the run's live state: every render settings row the run
//! holds changes their text when its value changes, or is listed with the reason the HUD does
//! not name it; and every menu entry that changes the run's settings or modes changes the text.

use super::*;
use crate::test_support::{argv, bare_run, carved_rock};
use svet_render::RenderSettings;

/// The rows no flag sets: [`crate::cli::Args::render_settings`] writes a library default or a
/// value another row gives. A run starts with the base's value; those a menu entry changes are
/// walked by `every_entry_that_changes_the_run_moves_the_hud`.
const NO_FLAG: &[(&str, &str)] = &[
    ("lighting.tone_map", "the library's default"),
    ("lighting.candidates", "the library's default"),
    ("lighting.probe_freeze", "the library's default"),
    ("lighting.lamp_reach", "the library's default"),
    ("lighting.probe_move_blend", "the library's default"),
    ("lighting.bake_pool_bytes", "the library's default"),
    (
        "lighting.light_history_far",
        "a menu entry's, off at the start",
    ),
    (
        "lighting.far_light_wide",
        "a menu entry's, off at the start",
    ),
    ("lighting.key_step", "the library's default"),
    ("look.cutaway", "a key's, off at the start"),
    ("lod.r_full", "follows build.residency's full radius"),
    ("lod.forced_level", "none in the fly-through"),
    ("fsr2.reactive_translucent", "FSR 2's default"),
    ("fsr2.reactive_light", "FSR 2's default"),
    ("fsr2.metres_per_voxel", "FSR 2's default"),
    ("field_split", "on in the fly-through"),
    (
        "probe_axis",
        "none in the fly-through: the volume follows the lamp's reach",
    ),
    ("build.record_chunks", "follows build.residency"),
    ("build.light_samples", "the mirror's default"),
    ("build.light_chunk_cap", "the mirror's default"),
    ("build.light_sync_samples", "the mirror's default"),
    ("build.entity_pool_bytes", "the entity pool's default"),
    ("mark_colours", "the library's colours"),
];

/// The rows a flag sets that the HUD and the banner do not name, and why. None is a key's or
/// a menu entry's mode (those show theirs, ADR-0055's rule on toggles).
const UNNAMED: &[(&str, &str)] = &[
    (
        "lighting.light_floor",
        "--light-floor: a list light's reach, a start-up value",
    ),
    (
        "look.translucent_layers",
        "a start-up value, the profile's unless --translucent-layers",
    ),
    (
        "look.tuning.face_mix",
        "a part of --look's preset, read off the face look alone",
    ),
    (
        "metering",
        "--metering: the meter's weights, a start-up value",
    ),
    (
        "bake.drain",
        "a start-up value, the profile's unless --bake-drain",
    ),
    (
        "build.mirror_budget_bytes",
        "a rebuild value; the HUD reads the mirror's own bytes",
    ),
    ("build.light_radius", "--light-radius: a rebuild value"),
    (
        "build.sync_budget_bytes",
        "a rebuild value; the HUD reads the sync's own bytes",
    ),
    (
        "build.sync_budget_time",
        "a rebuild value; the HUD reads the sync's own time",
    ),
    (
        "build.residency",
        "a rebuild value; the HUD reads the streamer's own radii",
    ),
    (
        "build.workers",
        "a rebuild value: generation threads, which cost only time",
    ),
    (
        "output_hint",
        "a rebuild value; the HUD reads the frame's own sizes",
    ),
];

/// The flags of a bare run under the sky (`--level --sky on`), so the banner's sky words show.
pub(super) fn sky_args() -> crate::cli::Args {
    crate::cli::parse_args_from(argv(&["--level", "--sky", "on"])).expect("parses")
}

/// A bare run started from `args`: its modes are the ones `args` starts.
pub(super) fn run_of(args: crate::cli::Args) -> Run {
    let mut run = bare_run(carved_rock());
    run.modes = args.start_modes(0.0);
    run.args = args;
    run
}

/// Writes `s` into the flags [`crate::cli::Args::render_settings`] reads, as the flags that
/// set them would: what a run started with those flags holds.
pub(super) fn hold(a: &mut crate::cli::Args, s: &RenderSettings) {
    use crate::hud::UpscalerMode;
    let l = &s.lighting;
    let base = a.start_lighting();
    let g = &mut a.light;
    g.exposure = match l.exposure {
        svet_render::Exposure::Manual(e) => Some(e),
        svet_render::Exposure::Auto(_) => None,
    };
    g.lamp = Some(l.lamp);
    g.lamp_bounce = l.lamp_bounce;
    g.shadow_walk = l.shadow_walk;
    g.store_read = l.store_read;
    g.light_weights = l.light_weights;
    g.young_filter = l.young_filter;
    g.entity_light = l.entity_light;
    g.filter = l.filter;
    g.atrous_iterations = l.atrous_iterations;
    g.light_floor = l.light_floor;
    g.history = l.history;
    g.probe_rays = l.probe_rays;
    g.probe_slices = l.probe_slices;
    g.resolution = s.lighting_resolution;
    g.probe_spacing = s.probe_spacing;
    g.metering = s.metering;
    g.radius = Some(s.build.light_radius);
    let o = &mut a.outdoor;
    o.sky_rays = Some(l.sky_rays);
    o.sky_history = Some(l.sky_history);
    o.key_history = Some(l.key_history);
    o.light_history = Some(l.light_history);
    o.landing_fade = Some(l.landing_fade);
    o.source_fade = Some(l.source_fade);
    o.dig_hold = Some(l.dig_hold);
    o.penumbra = Some(l.penumbra);
    o.key_far = l.key_far;
    o.key_far_mix = l.key_far_mix;
    o.sky_normal = l.sky_normal;
    o.sky_bounce = l.sky_bounce;
    o.column_wait = l.column_wait;
    o.key_cap = l.key_cap;
    o.sky_down_skip = l.sky_down_skip;
    o.key_at = l.key_at;
    o.key_horizon = l.key_horizon;
    if let svet_render::Exposure::Auto(auto) = l.exposure
        && base.exposure != l.exposure
    {
        a.outdoor.exposure_rate = Some(auto.rate_ev_s);
    }
    let k = &mut a.look;
    k.upscaler = Some(match s.upscaler {
        svet_render::UpscalerChoice::Off => UpscalerMode::Off,
        svet_render::UpscalerChoice::Fsr2 => UpscalerMode::Fsr2,
    });
    if s.upscaler == svet_render::UpscalerChoice::Fsr2 {
        k.internal_scale = s.scale;
    }
    k.sharpen = s.fsr2.sharpness;
    k.jitter_phases = s.fsr2.jitter_phases;
    k.dither = s.fsr2.dither;
    k.translucent_layers = s.look.translucent_layers;
    k.preset.normal_mode = s.look.normal_mode;
    k.preset.tuning = s.look.tuning;
    k.preset.far_normal = s.look.far_normal;
    k.view = s.look.view;
    if s.build.field_surface {
        k.preset.normal_mode = svet_render::NormalMode::Contour;
    }
    a.light_cap = s.light_cap;
    let b = &mut a.bake;
    b.budget = s.bake.budget;
    b.coarse = s.bake.coarse_samples;
    b.near = s.bake.near;
    b.full_first = Some(s.bake.full_first);
    b.reach = s.bake.reach;
    b.edit_ms = s.bake.edit_ms;
    b.dig_reach = s.bake.dig_reach;
    b.tiers = s.bake.tiers;
    b.drain = s.bake.drain;
    b.missing_light = s.missing_light;
    a.mirror_budget = Some(s.build.mirror_budget_bytes);
    a.sync_budget = s.build.sync_budget_bytes;
    a.sync_time = s.build.sync_budget_time;
    a.residency = s.build.residency;
    a.workers = s.build.workers;
    a.output_hint = s.output_hint;
}

/// Every row of the HUD and the banner as the window draws them with the stats HUD shown,
/// the passes timed and a frame sample (all zero) given: the readings rows read no clock. The
/// profile row is left out: it counts the fields edited off the profile (`profile: floor +1`)
/// whatever their values, so it moves for every row and names none.
fn hud_text(run: &mut Run) -> String {
    run.modes.profile.clear();
    run.start = now();
    run.hud.visible = true;
    run.hud.format_us = 0;
    run.hud.pass_ms = Some(vec![0.0; svet_render::FRAME_PASSES.len()]);
    run.fill_hud(Some(crate::setup::GfxSample {
        mirror_bytes: 0,
        pool_live: 0,
        pool_free: 0,
        pressure: 0.0,
        budget_bytes: 0,
        light_samples: 0,
        sizes: None,
    }));
    run.hud.text.clone()
}

/// The rows of `list`.
fn names(list: &[(&'static str, &str)]) -> Vec<&'static str> {
    list.iter().map(|&(n, _)| n).collect()
}

/// For every row of `RenderSettings::FIELDS`, from the settings a run under the sky starts
/// with: the row's test value (`RenderSettings::with_test_value`) written into the flags
/// (`hold`) reads back from the started run's `Run::live_settings` (as the frame clamps it),
/// and the HUD's text differs from the base run's. A `NO_FLAG` row cannot be written (the run
/// starts with the base's value) and an `UNNAMED` row leaves the text as the base's. A row in
/// neither list whose text does not move fails: the HUD prints a constant, or names it
/// nowhere.
#[test]
fn every_settings_row_a_flag_sets_moves_the_hud() {
    let no_flag = names(NO_FLAG);
    let unnamed = names(UNNAMED);
    let base = run_of(sky_args()).live_settings();
    let mut args = sky_args();
    hold(&mut args, &base);
    let mut held = run_of(args);
    assert_eq!(
        held.live_settings().changed_fields(&base),
        Vec::<&str>::new(),
        "hold round-trips the base"
    );
    let text = hud_text(&mut held);
    assert_eq!(hud_text(&mut run_of(sky_args())), text, "the base's text");
    let (mut unheld, mut silent) = (Vec::new(), Vec::new());
    for &(name, _) in RenderSettings::FIELDS {
        let s = base.with_test_value(name).expect("a test value");
        let mut args = sky_args();
        hold(&mut args, &s);
        let mut run = run_of(args);
        // The row itself reads back; a row another follows (`lod.r_full` the residency's)
        // moves with it.
        let held = !run
            .live_settings()
            .changed_fields(&s.clamped())
            .contains(&name);
        if no_flag.contains(&name) {
            assert!(
                !held,
                "{name} is listed as set by no flag, yet a flag sets it"
            );
            continue;
        }
        if !held {
            unheld.push(name);
            continue;
        }
        let moved = hud_text(&mut run) != text;
        if unnamed.contains(&name) {
            assert!(!moved, "{name} is listed unnamed, yet the HUD names it");
        } else if !moved {
            silent.push(name);
        }
    }
    assert!(unheld.is_empty(), "rows no flag sets: {unheld:?}");
    assert!(
        silent.is_empty(),
        "rows the HUD does not follow: {silent:?}"
    );
    for n in no_flag.iter().chain(&unnamed) {
        assert!(RenderSettings::class_of(n).is_some(), "{n} is no row");
    }
}

/// Every menu entry available on a run under the sky, activated as a player does (`Tab`, its
/// words, `Enter`; a number then `Up`, or `Down` at its top): an entry that changes the run's
/// render settings (`Run::live_settings`) changes the HUD's text too, so no key or entry
/// changes the frame while the window names the old value. Reports the rows each changed.
#[test]
fn every_entry_that_changes_the_settings_moves_the_hud() {
    use super::menu_tests::{frame_keys, pressed, select};
    use svet_app::KeyCode;
    let mut silent = Vec::new();
    let mut moved = 0;
    for e in crate::hud::menu::entries() {
        let mut run = run_of(sky_args());
        if e.available(&run.modes, &run.run_view()).is_err() {
            continue;
        }
        select(&mut run, &e);
        let before = run.live_settings();
        let text = hud_text(&mut run);
        // The menu's own keys, typed as a player types them.
        // keys:typed-begin
        frame_keys(&mut run, &pressed(KeyCode::Enter));
        if e.is_number() {
            frame_keys(&mut run, &pressed(KeyCode::ArrowUp));
            if run.live_settings() == before {
                frame_keys(&mut run, &pressed(KeyCode::ArrowDown));
            }
        }
        // keys:typed-end
        let changed = before.changed_fields(&run.live_settings());
        if changed.is_empty() {
            continue;
        }
        moved += 1;
        if hud_text(&mut run) == text {
            silent.push(format!("{}: {changed:?}", e.label));
        }
    }
    assert!(
        silent.is_empty(),
        "entries the HUD does not follow: {silent:?}"
    );
    assert!(moved > 10, "{moved} entries changed the settings");
}
