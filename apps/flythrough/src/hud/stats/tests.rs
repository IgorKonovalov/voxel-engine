//! The stats HUD's tests: its rows, its cadence and its samples.

use super::*;

/// The `pace` row names the mode and shows the window's median wall time beside the
/// frame's CPU and GPU, under the cpu row, for both modes; without a mode it is hidden.
#[test]
fn a_shown_hud_names_the_pace() {
    let (mut out, mut colours) = (String::new(), Vec::new());
    let mut s = HudStats::fixed();
    s.write_rows(&mut out, &mut colours);
    assert!(!out.contains("pace "), "{out}");
    for (pace, want) in [
        ("free", "pace free  wall 16.50 / cpu 4.12 / gpu 1.75 ms"),
        (
            "compositor",
            "pace compositor  wall 16.50 / cpu 4.12 / gpu 1.75 ms",
        ),
    ] {
        s.pace = Some(pace);
        s.write_rows(&mut out, &mut colours);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[1], "cpu 4.125 ms", "{out}");
        assert_eq!(lines[2], want, "{out}");
        assert_eq!(colours[2], HudColour::White);
        assert_eq!(lines.len(), colours.len());
    }
    s.march_ms = None;
    s.write_rows(&mut out, &mut colours);
    assert!(
        out.lines()
            .any(|l| l == "pace compositor  wall 16.50 / cpu 4.12 / gpu n/a ms"),
        "{out}"
    );
}

fn fed(ms: impl IntoIterator<Item = f32>) -> FrameStats {
    let mut s = FrameStats::new();
    for v in ms {
        s.record(v / 1000.0);
    }
    s
}

#[test]
fn equal_deltas_read_as_mean_median_and_p99() {
    let s = fed([5.0; RING]);
    let f = s.frame_ms();
    assert!((f.mean - 5.0).abs() < 1e-3, "mean {}", f.mean);
    assert!((f.p50 - 5.0).abs() < 1e-4, "p50 {}", f.p50);
    assert!((f.p99 - 5.0).abs() < 1e-4, "p99 {}", f.p99);
    assert!((s.fps() - 200.0).abs() < 0.1, "fps {}", s.fps());
}

/// One spike in a full window: index `round(0.99 x 239) = 237` of the sorted window is
/// still a 1 ms sample, so the spike shows in the max and not in the p99.
#[test]
fn one_spike_among_239_reads_in_the_max_not_the_p99() {
    let s = fed(std::iter::once(100.0).chain([1.0; RING - 1]));
    let f = s.frame_ms();
    assert!((f.p50 - 1.0).abs() < 1e-4, "p50 {}", f.p50);
    assert!((f.p99 - 1.0).abs() < 1e-4, "p99 {}", f.p99);
    assert!((f.max - 100.0).abs() < 1e-3, "max {}", f.max);
}

/// Three spikes in a full window sort to indices 237 to 239, so the p99 index
/// `round(0.99 x 239) = 237` holds a spike; the median index `round(0.5 x 239) = 120`
/// still holds 1 ms. Two spikes would leave index 237 at 1 ms.
#[test]
fn three_spikes_among_237_read_in_the_p99() {
    let s = fed([100.0; 3].into_iter().chain([1.0; RING - 3]));
    let f = s.frame_ms();
    assert!((f.p99 - 100.0).abs() < 1e-3, "p99 {}", f.p99);
    assert!((f.p50 - 1.0).abs() < 1e-4, "p50 {}", f.p50);
    assert!((f.max - 100.0).abs() < 1e-3, "max {}", f.max);
}

#[test]
fn a_known_sequence_reads_nearest_rank() {
    let s = fed((1..=100).map(|v| v as f32));
    assert_eq!(s.frames_total(), 100);
    assert!((s.fps() - 19.801_98).abs() < 1e-2, "fps {}", s.fps());
    assert!((s.frame_ms_avg() - 50.5).abs() < 1e-3);
    // Sorted 1..=100: nearest rank ceil(0.99 x 100) = 99 holds 99 ms.
    assert!((s.frame_ms_p99() - 99.0).abs() < 1e-3);
}

#[test]
fn an_empty_window_reads_zero() {
    let s = FrameStats::new();
    assert_eq!(s.fps(), 0.0);
    assert_eq!(s.frame_ms(), FrameMs::default());
    assert_eq!(s.samples().count(), 0);
}

#[test]
fn samples_are_chronological_after_wrap() {
    let mut s = FrameStats::new();
    for i in 0..(RING + 10) {
        s.record(i as f32);
    }
    let got: Vec<f32> = s.samples().collect();
    assert_eq!(got.len(), RING);
    assert_eq!(got.first().copied(), Some(10.0));
    assert_eq!(got.last().copied(), Some((RING + 9) as f32));
    assert!(got.windows(2).all(|w| w[1] == w[0] + 1.0));
}

/// The cadence driving a stub counter: no call while hidden, one on turning visible, then
/// one every `every`th visible frame.
#[test]
fn the_world_is_sampled_on_toggle_and_every_nth_visible_frame() {
    for every in [60, 240] {
        let mut cadence = Cadence::new(every);
        let mut calls = 0u32;
        let mut frame = |visible: bool, calls: &mut u32| {
            if cadence.due(visible) {
                *calls += 1;
            }
        };
        for _ in 0..300 {
            frame(false, &mut calls);
        }
        assert_eq!(calls, 0, "every {every}: hidden frames sample nothing");
        frame(true, &mut calls);
        assert_eq!(calls, 1, "every {every}: turning visible samples once");
        for _ in 0..120 {
            frame(true, &mut calls);
        }
        assert_eq!(calls, 1 + 120 / every, "every {every}: 120 visible frames");
    }
}

#[test]
fn hiding_and_showing_again_samples_on_the_toggle() {
    let mut c = Cadence::new(60);
    assert!(c.due(true));
    assert!(!c.due(true));
    assert_eq!(c.age(), 1);
    assert!(!c.due(false));
    assert!(c.due(true));
    assert_eq!(c.age(), 0);
}

#[test]
fn cpu_bytes_counts_full_brick_voxels_and_chunks_and_the_pool() {
    assert_eq!(cpu_bytes_estimate(0, 1, 0), 1024);
    let old = 3 * 1024 + 2 * size_of::<Chunk>() as u64;
    assert_eq!(cpu_bytes_estimate(2, 3, 0), old);
    assert_eq!(cpu_bytes_estimate(2, 3, 4096), old + 4_194_304);
}

#[test]
fn an_edit_line_reads_like_the_printed_one() {
    let e = EditLine {
        verb: "carved",
        counts: vec![("rock".to_owned(), 1234), ("ore".to_owned(), 12)],
        chunks: 3,
        ms: 0.214,
    };
    assert_eq!(
        e.to_string(),
        "carved: rock 1234, ore 12 (3 chunks, 0.21 ms)"
    );
    let none = EditLine {
        verb: "filled",
        ..EditLine::default()
    };
    assert_eq!(none.to_string(), "filled: nothing (0 chunks, 0.00 ms)");
}

#[test]
fn a_row_over_its_line_is_yellow_and_one_per_line() {
    let stats = HudStats {
        frame_ms: FrameMs {
            mean: 10.0,
            p50: 10.0,
            p99: 20.0,
            max: 20.0,
        },
        march_ms: Some(1.0),
        upload_ms: 2.0,
        ..HudStats::default()
    };
    let (mut out, mut colours) = (String::new(), Vec::new());
    stats.write_rows(&mut out, &mut colours);
    assert_eq!(out.lines().count(), colours.len());
    assert_eq!(colours[0], HudColour::Yellow, "frame p99 over 16.67 ms");
    assert_eq!(colours[2], HudColour::White, "march under 3 ms");
    assert_eq!(colours[3], HudColour::Yellow, "upload over 1 ms");
    assert!(out.lines().any(|l| l == "cpu bytes 0 (estimate)"));
    assert_eq!(stats.to_string(), out.trim_end());
    let spike = HudStats {
        frame_ms: FrameMs {
            max: 40.0,
            ..FrameMs::default()
        },
        ..HudStats::default()
    };
    spike.write_rows(&mut out, &mut colours);
    assert_eq!(colours[0], HudColour::Red, "a frame over 33.3 ms");
}

/// The steps row appears only with a steps sample, before the format row, and reads the
/// sample's five figures.
#[test]
fn the_steps_row_prints_the_last_frame_s_sample() {
    let stats = HudStats {
        steps: Some(StepsSample {
            mean_steps: 15.55,
            p99_steps: 84,
            block_max_mean: 31.2,
            mean_loads: 12.4,
            mean_pool_reads: 2.45,
        }),
        ..HudStats::default()
    };
    let lines = stats.to_string();
    let lines: Vec<&str> = lines.lines().collect();
    assert_eq!(
        lines[lines.len() - 2],
        "steps per ray mean 15.55 p99 84 block-max mean 31.20; loads mean 12.40, pool 2.45"
    );
    assert!(lines[lines.len() - 1].starts_with("format "));
    assert!(
        !HudStats::default()
            .to_string()
            .lines()
            .any(|l| l.starts_with("steps per ray")),
        "no row without a sample"
    );
}

/// The cracks row appears only with a cracks sample, before the format row, and reads the
/// leaks' share in percent of the hit pixels.
#[test]
fn the_cracks_row_prints_the_last_frame_s_shares() {
    let stats = HudStats {
        cracks: Some(vxe_render::CrackShares {
            hits: 2000,
            leaks: 6,
        }),
        ..HudStats::default()
    };
    let lines = stats.to_string();
    let lines: Vec<&str> = lines.lines().collect();
    assert_eq!(lines[lines.len() - 2], "cracks leaks 0.30 %");
    assert!(lines[lines.len() - 1].starts_with("format "));
    assert!(
        !HudStats::default()
            .to_string()
            .lines()
            .any(|l| l.starts_with("cracks ")),
        "no row without a sample"
    );
}

/// The cutaway row follows the camera row while the view is on, and is absent otherwise.
#[test]
fn the_cutaway_row_shows_while_the_view_is_on() {
    let stats = HudStats {
        cutaway: Some("auto"),
        ..HudStats::default()
    };
    let text = stats.to_string();
    let lines: Vec<&str> = text.lines().collect();
    let i = lines
        .iter()
        .position(|l| l.starts_with("fly at "))
        .expect("the camera row");
    assert_eq!(lines[i + 1], "cutaway (auto)");
    assert!(
        !HudStats::default()
            .to_string()
            .lines()
            .any(|l| l.starts_with("cutaway")),
        "no row while the view is off"
    );
}

/// The pressure row follows the mirror row when there is a reading: yellow past 0.9, and
/// with the full radius while streaming.
#[test]
fn a_pressure_reading_adds_its_row() {
    let (mut out, mut colours) = (String::new(), Vec::new());
    let mut s = HudStats::fixed();
    s.pressure = Some(Pressure {
        pressure: 0.921_875,
        budget_bytes: 3 << 30,
        r_full: Some([15, 16]),
    });
    s.write_rows(&mut out, &mut colours);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 13);
    assert_eq!(
        lines[5],
        "mirror pressure 0.922 of 3221225472 B, r_full 15 of 16"
    );
    assert_eq!(colours[5], HudColour::Yellow);
    s.pressure = Some(Pressure {
        pressure: 0.25,
        budget_bytes: 1 << 20,
        r_full: None,
    });
    s.write_rows(&mut out, &mut colours);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[5], "mirror pressure 0.250 of 1048576 B");
    assert_eq!(colours[5], HudColour::White);
}

/// The golden's fixture: twelve rows, the frame and upload rows yellow, the rest white.
#[test]
fn fixed_stats_format_to_their_rows() {
    let (mut out, mut colours) = (String::new(), Vec::new());
    HudStats::fixed().write_rows(&mut out, &mut colours);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 12);
    assert_eq!(
        lines[0],
        "fps 58.8  frame mean 17.00 p50 16.50 p99 24.25 max 31.50 ms (240 frames)"
    );
    assert_eq!(lines[2], "gpu march 1.750 ms  hud 0.038 ms");
    assert_eq!(lines[3], "upload 1310720 B, 3 chunks, 1.500 ms");
    assert_eq!(
        lines[4],
        "mirror 268435456 B live, gpu pool 245760 live / 16384 free slots"
    );
    assert_eq!(lines[9], "carved: rock 4169 (2 chunks, 0.25 ms)");
    let yellow: Vec<usize> = (0..colours.len())
        .filter(|&i| colours[i] != HudColour::White)
        .collect();
    assert_eq!(yellow, [0, 3]);
    assert!(colours.iter().all(|&c| c != HudColour::Red));
}

/// The queue row counts the chunks in the view cone beyond the shell still waiting, blank
/// and coarse, from the queue's counters; the shell and the chunks out of view are not
/// counted.
#[test]
fn the_queue_row_counts_the_view() {
    let mut st = vxe_render::QueueStats::default();
    st.view_waiting[vxe_render::internals::VIEW_SHELL] = [3, 1];
    st.view_waiting[vxe_render::internals::VIEW_IN] = [7, 5];
    st.view_waiting[vxe_render::internals::VIEW_OUT] = [40, 9];
    assert_eq!(
        queue_row(&st, 1.254, ""),
        "in view: 12 waiting (5 coarse), withdrawn 0, step 1.25 ms"
    );
    let (mut out, mut colours) = (String::new(), Vec::new());
    let mut s = HudStats::fixed();
    s.write_rows(&mut out, &mut colours);
    assert!(!out.contains("in view:"), "{out}");
    s.queue = Some(queue_row(&st, 1.254, ""));
    s.write_rows(&mut out, &mut colours);
    assert!(
        out.lines()
            .any(|l| l == "in view: 12 waiting (5 coarse), withdrawn 0, step 1.25 ms"),
        "{out}"
    );
    assert_eq!(out.lines().count(), colours.len());
}

/// The lighting row shows the store's chunks, megabytes and missing pixels under the store,
/// and the missing-light mode with its fallback and still-black pixels.
#[test]
fn the_store_row_reads_its_counts() {
    let (mut out, mut colours) = (String::new(), Vec::new());
    let mut s = HudStats::fixed();
    s.lighting = Some(LightingRow {
        passes: vec![("light", 0.5)],
        light_samples: 7,
        live_probes: None,
        store: Some(vxe_render::StoreStats {
            chunks: 12,
            bytes: 3_400_000,
            missing: 56,
            brick_reads: 0,
            coarse_reads: 0,
            coarse_chunks: 0,
            fallback: 40,
            still_black: 16,
            ..Default::default()
        }),
        missing_light: "chain",
        store_note: ", queue depth 4, coarse 2, jobs 9, slice 1.25 ms".to_owned(),
        modes: "light baked".to_owned(),
        upscale: None,
    });
    s.write_rows(&mut out, &mut colours);
    assert!(
        out.contains(
            "store 12 3.4 MB missing 56 (light chain: fallback 40 black 16), queue depth 4, coarse 2, jobs 9, slice 1.25 ms; light baked"
        ),
        "{out}"
    );
}

/// The lighting rows follow the march row: every pass's time, yellow when light,
/// temporal and the probe update and gather pass the 6.0 ms line or the filter the 3.0 ms
/// one, then the light samples, the live probes and the modes.
#[test]
fn the_lighting_rows_follow_the_march_row() {
    let (mut out, mut colours) = (String::new(), Vec::new());
    let mut s = HudStats::fixed();
    s.lighting = Some(LightingRow {
        passes: vec![
            ("hit", 1.25),
            ("light", 4.5),
            ("temporal", 0.5),
            ("atrous", 2.5),
            ("probes", 0.25),
            ("gather", 0.5),
        ],
        light_samples: 18_522,
        live_probes: Some(9_876),
        store: Some(vxe_render::StoreStats {
            chunks: 806,
            bytes: 15_400_000,
            missing: 3,
            brick_reads: 59_724,
            ..Default::default()
        }),
        store_note: String::new(),
        missing_light: "",
        modes: "lighting full".to_owned(),
        upscale: None,
    });
    s.write_rows(&mut out, &mut colours);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 14);
    assert_eq!(
        lines[3],
        "gpu hit 1.25 light 4.50 temporal 0.50 atrous 2.50 probes 0.25 gather 0.50 ms"
    );
    assert_eq!(
        lines[4],
        "lights 18522 samples, 9876 probes live, store 806 15.4 MB missing 3 brick level 59724; lighting full"
    );
    assert_eq!(colours[3], HudColour::White);
    s.lighting.as_mut().unwrap().passes[5].1 = 1.25;
    s.write_rows(&mut out, &mut colours);
    assert_eq!(
        colours[3],
        HudColour::Yellow,
        "light, temporal, probes and gather over 6.0 ms"
    );
    s.lighting.as_mut().unwrap().passes[5].1 = 0.5;
    s.lighting.as_mut().unwrap().live_probes = None;
    s.lighting.as_mut().unwrap().store = None;
    s.write_rows(&mut out, &mut colours);
    assert_eq!(
        out.lines().nth(4),
        Some("lights 18522 samples; lighting full")
    );
    s.lighting.as_mut().unwrap().passes[1].1 = 6.0;
    s.write_rows(&mut out, &mut colours);
    assert_eq!(
        colours[3],
        HudColour::Yellow,
        "light + temporal over 6.0 ms"
    );
}

/// While the upscaler runs a row follows the light samples: the upscale and sharpen
/// passes' times and the two sizes, yellow when they and the composite pass 1.5 ms.
#[test]
fn the_upscale_row_follows_the_lighting_rows() {
    let (mut out, mut colours) = (String::new(), Vec::new());
    let mut s = HudStats::fixed();
    s.lighting = Some(LightingRow {
        passes: vec![("composite", 0.2), ("upscale", 0.5), ("sharpen", 0.12)],
        light_samples: 0,
        live_probes: None,
        store: None,
        store_note: String::new(),
        missing_light: "",
        modes: "lighting half".to_owned(),
        upscale: Some([[1280, 720], [1920, 1080]]),
    });
    s.write_rows(&mut out, &mut colours);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 15);
    assert_eq!(
        lines[5],
        "upscale 0.50 sharpen 0.12 ms, 1280x720 to 1920x1080"
    );
    assert_eq!(colours[5], HudColour::White);
    s.lighting.as_mut().unwrap().passes[1].1 = 1.3;
    s.write_rows(&mut out, &mut colours);
    assert_eq!(
        colours[5],
        HudColour::Yellow,
        "0.2 + 1.3 + 0.12 over 1.5 ms"
    );
}

/// The exposure row follows the march row's block (here, with no lighting rows, the march
/// row) and reads the mode, the exposure and the metered scene in EV; in auto, the
/// settled exposure too.
#[test]
fn the_exposure_row_names_the_mode_and_the_scene() {
    let (mut out, mut colours) = (String::new(), Vec::new());
    let mut s = HudStats::fixed();
    s.exposure = Some(ExposureRow {
        mode: "manual".to_owned(),
        ev: 0.0,
        scene_ev: Some(-7.94),
        target_ev: None,
    });
    s.write_rows(&mut out, &mut colours);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[3], "exposure manual +0.00 EV  scene -7.9 EV");
    assert_eq!(colours[3], HudColour::White);
    s.exposure = Some(ExposureRow {
        mode: "auto eye".to_owned(),
        ev: 2.411,
        scene_ev: None,
        target_ev: Some(5.526),
    });
    s.write_rows(&mut out, &mut colours);
    assert_eq!(
        out.lines().nth(3),
        Some("exposure auto eye +2.41 EV  scene n/a  settles +5.53 EV")
    );
}

/// Under FSR 2 the upscale row reads its passes' total and the gpu row leaves them out;
/// the total counts towards the 1.5 ms line.
#[test]
fn the_upscale_row_sums_fsr2s_passes() {
    let (mut out, mut colours) = (String::new(), Vec::new());
    let mut s = HudStats::fixed();
    s.lighting = Some(LightingRow {
        passes: vec![
            ("composite", 0.2),
            ("upscale", 0.0),
            ("sharpen", 0.0),
            ("fsr2 input", 0.1),
            ("fsr2 accumulate", 0.4),
            ("fsr2 tone", 0.05),
        ],
        light_samples: 0,
        live_probes: None,
        store: None,
        store_note: String::new(),
        missing_light: "",
        modes: "lighting half".to_owned(),
        upscale: Some([[1280, 720], [1920, 1080]]),
    });
    s.write_rows(&mut out, &mut colours);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[5], "fsr2 0.55 ms, 1280x720 to 1920x1080");
    assert_eq!(colours[5], HudColour::White);
    assert!(!out.contains("fsr2 input"), "{out}");
    s.lighting.as_mut().unwrap().passes[4].1 = 1.4;
    s.write_rows(&mut out, &mut colours);
    assert_eq!(colours[5], HudColour::Yellow, "0.2 + 1.55 over 1.5 ms");
}

/// A pad row sits before the format row, and a run without a pad source shows none.
#[test]
fn the_pad_row_shows_before_the_format_row() {
    let (mut out, mut colours) = (String::new(), Vec::new());
    let mut s = HudStats::fixed();
    s.pad = Some(PadRow {
        name: "PS5 Controller".to_owned(),
        power: Some(PadPower::Charging(5)),
    });
    s.write_rows(&mut out, &mut colours);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 13);
    assert_eq!(lines[11], "pad PS5 Controller, charging 5%");
    assert_eq!(lines[12], "format 6 us");
    assert_eq!(colours[11], HudColour::White);
}

#[test]
fn rss_reads_on_linux() {
    if cfg!(target_os = "linux") {
        let page = page_bytes();
        assert!(page.is_power_of_two() && page >= 4096, "page {page}");
        assert!(rss_bytes(page).is_some_and(|b| b > 0));
    }
}
