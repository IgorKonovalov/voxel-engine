//! The modes' tests: the banner's and the HUD's words for each mode and key.

use super::*;
use crate::toggles::Press;

/// The `lights` row and the banner name the testbed's lights: the count with the headlamp
/// counted while it is on, carry, the lights in flight and those over the cap; `1` on a
/// full list is refused with `FULL_LINE`, which names Plan 0058.
/// `3` names the picked colour in the banner and the `lights` row, and the newest light
/// takes it; `4` removes the newest and the banner names the count.
#[test]
fn the_banner_and_hud_name_the_colour_and_the_removal() {
    use crate::toggles::{KEY_REMOVE, TOGGLES};
    let lamp_off = vxe_render::Lighting {
        lamp: false,
        ..vxe_render::Lighting::default()
    };
    let mut m = Modes::new(lamp_off, vxe_render::LightingResolution::Half, 0.0);
    for _ in 0..3 {
        m.lights
            .drop_at(false, [0.0; 3], [1.0, 0.0, 0.0], None)
            .unwrap();
    }
    let colour = TOGGLES.iter().find(|t| t.name == "colour").unwrap();
    assert_eq!(colour.press, Press::Menu);
    (colour.flip)(&mut m, 10.0);
    assert!(m.banner(10.0).unwrap().lines().any(|l| l == "colour warm"));
    (colour.flip)(&mut m, 20.0);
    assert!(m.banner(20.0).unwrap().lines().any(|l| l == "colour red"));
    assert!(m.row(HudRow::Lights).ends_with("colour red"));
    assert_eq!(m.lights.lights[2].light.colour, [1.0, 0.0, 0.0]);
    assert_eq!(m.lights.lights[1].light.colour, [1.0; 3]);
    m.lights
        .drop_at(false, [0.0; 3], [1.0, 0.0, 0.0], None)
        .unwrap();
    m.lights
        .fire(false, [0.0; 3], [1.0, 0.0, 0.0], None)
        .unwrap();
    assert_eq!(m.lights.lights[3].light.colour, [1.0, 0.0, 0.0]);
    assert_eq!(m.lights.lights[4].light.colour, [1.0, 0.0, 0.0]);
    // `4` on the three: `run::keys` removes the newest and shows the count.
    assert_eq!(KEY_REMOVE, vxe_app::KeyCode::Digit4);
    m.lights.lights.truncate(3);
    m.lights.remove_newest();
    m.show_lights(m.lights.count_words(false), 30.0);
    assert!(m.banner(30.0).unwrap().lines().any(|l| l == "lights 2/16"));
}

/// `5` flips the newest light's bounce flag: the banner reads `bounce on`, then `bounce
/// off`, and the `lights` row marks each flagged light `*`.
#[test]
fn the_banner_and_hud_name_the_bounce_flag() {
    use crate::toggles::TOGGLES;
    let lamp_off = vxe_render::Lighting {
        lamp: false,
        ..vxe_render::Lighting::default()
    };
    let mut m = Modes::new(lamp_off, vxe_render::LightingResolution::Half, 0.0);
    for _ in 0..2 {
        m.lights
            .drop_at(false, [0.0; 3], [1.0, 0.0, 0.0], None)
            .unwrap();
    }
    let bounce = TOGGLES.iter().find(|t| t.name == "bounce").unwrap();
    assert_eq!(bounce.press, Press::Menu);
    (bounce.flip)(&mut m, 10.0);
    assert!(m.banner(10.0).unwrap().lines().any(|l| l == "bounce on"));
    assert_eq!(
        m.row(HudRow::Lights),
        "lights 2/16 bounce * over 0 colour white"
    );
    assert_eq!(
        m.lights.lights[1].light.flags & vxe_render::Light::BOUNCE,
        vxe_render::Light::BOUNCE
    );
    (bounce.flip)(&mut m, 20.0);
    assert!(m.banner(20.0).unwrap().lines().any(|l| l == "bounce off"));
    assert_eq!(m.row(HudRow::Lights), "lights 2/16 over 0 colour white");
}

/// `F4` steps the a-trous iterations 4, 2, 1 and back, in the lighting row's `filter <n>`
/// and the banner; with the filter off the row reads `filter off`.
#[test]
fn f4_steps_the_filters_iterations() {
    use crate::toggles::TOGGLES;
    let mut m = Modes::new(
        vxe_render::Lighting::default(),
        vxe_render::LightingResolution::Half,
        0.0,
    );
    let f4 = TOGGLES
        .iter()
        .find(|t| t.name == "filter iterations")
        .unwrap();
    assert_eq!(f4.row, HudRow::Lighting);
    assert_eq!((f4.get)(&m), "filter 4");
    for (i, want) in ["filter 2", "filter 1", "filter 4"].into_iter().enumerate() {
        let now = 10.0 * (i + 1) as f64;
        (f4.flip)(&mut m, now);
        assert_eq!((f4.get)(&m), want);
        assert!(m.line().contains(&format!(", {want}, ")), "{}", m.line());
        assert!(m.banner(now).unwrap().contains(want));
    }
    m.lighting.filter = false;
    assert_eq!((f4.get)(&m), "filter off");
}

#[test]
fn the_banner_and_hud_name_the_lights() {
    use crate::lights::{FULL_LINE, TestLights};
    use crate::toggles::TOGGLES;
    let lamp_off = vxe_render::Lighting {
        lamp: false,
        ..vxe_render::Lighting::default()
    };
    let half = vxe_render::LightingResolution::Half;
    let mut m = Modes::new(lamp_off, half, 0.0);
    for _ in 0..3 {
        m.lights
            .drop_at(false, [0.0; 3], [1.0, 0.0, 0.0], None)
            .unwrap();
    }
    let carry = TOGGLES.iter().find(|t| t.name == "carry").unwrap();
    (carry.flip)(&mut m, 10.0);
    assert_eq!(
        m.row(HudRow::Lights),
        "lights 3/16 carry over 0 colour white"
    );
    assert!(m.banner(10.0).unwrap().lines().any(|l| l == "carry on"));
    m.lighting.lamp = true;
    assert!(m.row(HudRow::Lights).starts_with("lights 4/16"));
    m.lighting.lamp = false;
    (carry.flip)(&mut m, 20.0);
    assert!(m.banner(20.0).unwrap().lines().any(|l| l == "carry off"));
    // The drop key's banner, as `run::keys` shows it.
    m.show_lights(m.lights.count_words(false), 30.0);
    assert!(m.banner(30.0).unwrap().lines().any(|l| l == "lights 3/16"));
    // Two lights, one in flight.
    let mut f = Modes::new(lamp_off, half, 0.0);
    f.lights
        .drop_at(false, [0.0; 3], [1.0, 0.0, 0.0], None)
        .unwrap();
    f.lights
        .fire(false, [0.0; 3], [1.0, 0.0, 0.0], None)
        .unwrap();
    assert_eq!(
        f.row(HudRow::Lights),
        "lights 2/16 fly 1 over 0 colour white"
    );
    f.show_lights(format!("fire, {}", f.lights.count_words(false)), 40.0);
    let b = f.banner(40.0).unwrap();
    assert!(b.contains("fire") && b.contains("lights 2/16"), "{b}");
    // A full list refuses a drop and a shot with `FULL_LINE`.
    let mut full = TestLights::default();
    for _ in 0..16 {
        full.drop_at(false, [0.0; 3], [1.0, 0.0, 0.0], None)
            .unwrap();
    }
    assert_eq!(
        full.drop_at(false, [0.0; 3], [1.0, 0.0, 0.0], None),
        Err(FULL_LINE)
    );
    assert_eq!(
        full.fire(false, [0.0; 3], [1.0, 0.0, 0.0], None),
        Err(FULL_LINE)
    );
    assert!(FULL_LINE.contains("0058"));
    // Over the cap: sixteen lights with the lamp switched on.
    let mut over = Modes::new(lamp_off, half, 0.0).with_lights(full);
    over.lighting.lamp = true;
    assert_eq!(over.row(HudRow::Lights), "lights 16/16 over 1 colour white");
    // `--lights 3` parses.
    let a = crate::cli::parse_args_from(["--lights", "3"].map(String::from)).unwrap();
    assert_eq!(a.light.lights, 3);
}

/// `parts` joined by `, ` into rows of at most [`BANNER_COLS`] characters where they fit,
/// row by row: the reference [`wrap_from`] is held to.
fn wrap(parts: &[&str]) -> Vec<String> {
    let mut rows = Vec::new();
    let mut row = String::new();
    for part in parts {
        if !row.is_empty() && row.len() + 2 + part.len() > BANNER_COLS {
            rows.push(std::mem::take(&mut row));
        }
        if !row.is_empty() {
            row.push_str(", ");
        }
        row.push_str(part);
    }
    if !row.is_empty() {
        rows.push(row);
    }
    rows
}

/// Wrapping in place after a prefix gives the reference's rows, for parts that fill a row
/// exactly, overrun it by one, a part longer than a row, and one part alone.
#[test]
fn wrapping_in_place_matches_the_row_by_row_wrap() {
    let exact = "x".repeat(BANNER_COLS - 4);
    let long = "y".repeat(BANNER_COLS + 5);
    let cases: [Vec<&str>; 5] = [
        vec![
            "lighting half",
            "shadow walk coarse",
            "history edits",
            "lamp on",
        ],
        vec![&exact, "ab", "c"],
        vec![&exact, "abc", "d"],
        vec!["a", &long, "b", "c"],
        vec!["alone"],
    ];
    for parts in cases {
        let mut out = "prefix\n".to_owned();
        let from = out.len();
        out.push_str(&parts.join(", "));
        wrap_from(&mut out, from);
        assert_eq!(out[from..], wrap(&parts).join("\n"), "{parts:?}");
        assert_eq!(&out[..from], "prefix\n");
    }
}

/// Each of `F3`, `F5` to `F8`, `M`, `Comma`, `Semicolon`, `Delete` and `Insert` changes the banner's text to the new mode; the banner is
/// empty 3 s after the last change and shows again on the next one.
#[test]
fn the_banner_names_each_change_and_fades_after_three_seconds() {
    let mut m = Modes::new(
        vxe_render::Lighting::default(),
        vxe_render::LightingResolution::default(),
        0.0,
    );
    let start = m.banner(0.0).expect("the banner shows at the start");
    assert_eq!(
        start,
        "lighting half, store read face\nweights normal, shadow walk coarse\nhistory edits, lamp on\nlamp bounce off at 64 rays 1/8, filter 4\nyoung luma, entity light world\nore pulse off, exposure manual +0.00 EV"
    );
    assert!(start.lines().all(|l| l.len() <= BANNER_COLS));
    let mut t = 1000.0;
    let mut last = start;
    for (key, word) in [
        (ModeKey::ShadowWalk, "shadow walk full"),
        (ModeKey::History, "history gradient"),
        (ModeKey::Resolution, "lighting full"),
        (ModeKey::Lamp, "lamp off"),
        (ModeKey::LampBounce, "lamp bounce on"),
        (ModeKey::Pulse, "ore pulse on"),
        (ModeKey::StoreRead, "store read normal"),
        (ModeKey::LightWeights, "weights face"),
        (ModeKey::StoreRead, "store read face"),
        (ModeKey::LightWeights, "weights normal"),
        (ModeKey::YoungFilter, "young edges"),
        (ModeKey::YoungFilter, "young luma"),
        (ModeKey::EntityLight, "entity light own"),
        (ModeKey::EntityLight, "entity light world"),
    ] {
        m.press(key, t);
        let now = m.banner(t).expect("a change shows the banner");
        assert_ne!(now, last, "{key:?} changes the text");
        assert!(now.contains(word), "{key:?}: {now}");
        last = now;
        t += 500.0;
    }
    let changed = t - 500.0;
    assert!(m.banner(changed + BANNER_MS - 1.0).is_some());
    assert_eq!(
        m.banner(changed + BANNER_MS),
        None,
        "empty 3 s after the last change"
    );
    // `Q` from the default auto eye: auto symmetric, then manual holding the read-back's
    // exposure, then auto eye again, each with its EV.
    let mut q = Modes::new(
        vxe_render::Lighting {
            exposure: vxe_render::Exposure::Auto(vxe_render::AutoExposure::eye()),
            ..vxe_render::Lighting::default()
        },
        vxe_render::LightingResolution::default(),
        0.0,
    );
    q.set_readout(Some(vxe_render::ExposureReadout {
        exposure: 11.52,
        adapted_log2: -6.0,
        metered_log2: -6.0,
        frame: 9,
        held: false,
        ceiling: 16.0,
        sky_share: 0.0,
        sky_floor: None,
        highlight: None,
    }));
    assert!(
        q.banner(0.0)
            .unwrap()
            .contains("exposure auto eye +3.53 EV")
    );
    for (i, want) in [
        "exposure auto symmetric +3.53 EV",
        "exposure manual +3.53 EV",
        "exposure auto eye +3.53 EV",
    ]
    .into_iter()
    .enumerate()
    {
        let at = 100.0 * (i + 1) as f64;
        q.press(ModeKey::Exposure, at);
        let b = q.banner(at).expect("a change shows the banner");
        assert!(b.contains(want), "{b}");
        assert!(b.lines().all(|l| l.len() <= BANNER_COLS), "{b}");
    }
    assert_eq!(
        q.exposure_row().to_string(),
        "exposure auto eye +3.53 EV  scene -6.0 EV  settles +3.53 EV  ceiling +4.00 EV"
    );
    m.press(ModeKey::Lamp, changed + 10_000.0);
    let again = m
        .banner(changed + 10_000.0)
        .expect("the next change shows it again");
    assert!(again.contains("lamp on"), "{again}");
}

/// `N` names each preset in turn on the banner's first row, then the far normal.
#[test]
fn the_banner_names_each_look_preset() {
    use crate::LookPreset;
    let mut m = Modes::new(
        vxe_render::Lighting::default(),
        vxe_render::LightingResolution::default(),
        0.0,
    );
    let lighting = m.banner(0.0).expect("the banner shows at the start");
    let mut preset = LookPreset::FACE;
    let mut t = 5000.0;
    let mut names = Vec::new();
    for _ in 0..LookPreset::ALL.len() {
        preset = preset.next(true);
        m.show_look(&preset, [0.0; 2], t);
        let b = m.banner(t).expect("a press shows the banner");
        assert_eq!(
            b,
            format!("look {}, far normal off\n{lighting}", preset.name)
        );
        assert!(b.lines().count() <= BANNER_ROWS && b.lines().all(|l| l.len() <= BANNER_COLS));
        assert_eq!(m.banner(t + BANNER_MS), None);
        names.push(preset.name);
        t += BANNER_MS;
    }
    assert_eq!(names, ["field", "trilinear8", "gradient27", "face"]);
    assert_eq!(
        LookPreset::FACE.next(false).name,
        "trilinear8",
        "without the field's band slots N skips the field"
    );
    assert_eq!(LookPreset::GRADIENT27.next(false), LookPreset::FACE);
    let first = m.banner(t - BANNER_MS).unwrap();
    assert!(first.starts_with("look face, far normal off\n"), "{first}");
    m.show_look(&LookPreset::FACE, [2.0, 6.0], t);
    assert!(
        m.banner(t)
            .unwrap()
            .starts_with("look face, far normal 2..6\n"),
        "the knob's setting shows"
    );
    m.show_look(&LookPreset::GRADIENT27, [2.0, 6.0], t);
    assert!(
        m.banner(t)
            .unwrap()
            .starts_with("look gradient27, far normal off\n"),
        "a mode that does not read the far normal does not name it"
    );
}

/// `F9`'s view shows on the banner after the look's words, `view cracks`, from the press
/// for `BANNER_MS`; the shaded view adds nothing.
#[test]
fn the_banner_names_the_view() {
    use vxe_render::ViewMode;
    let mut m = Modes::new(
        vxe_render::Lighting::default(),
        vxe_render::LightingResolution::default(),
        0.0,
    );
    let lighting = m.banner(0.0).expect("the banner shows at the start");
    m.show_look(&crate::LookPreset::FIELD, [0.0; 2], 0.0);
    m.show_view(ViewMode::Cracks, 5000.0);
    assert_eq!(
        m.banner(5000.0).as_deref(),
        Some(
            format!(
                "{}\n{lighting}",
                wrap(&["look field", "far normal off", "view cracks"]).join("\n")
            )
            .as_str()
        )
    );
    assert_eq!(m.banner(5000.0 + BANNER_MS), None);
    m.show_view(ViewMode::Shaded, 9000.0);
    assert_eq!(
        m.banner(9000.0).as_deref(),
        Some(format!("look field, far normal off\n{lighting}").as_str())
    );
}

/// The sky anchor: with a sky set the banner names `sky anchor N` for an `N` other than the
/// default 4 (and nothing for 4) and `sky anchor floor X` for a floor share other than the
/// default (`off` for 0), and the exposure row the ceiling the read-back carries with its `N`,
/// the read-back's sky share and the floor's share (`floor -` while absent); `Q`'s symmetric
/// preset keeps the anchor and the floor's share; with no read-back the row names the model's
/// `max`.
#[test]
fn the_banner_and_row_name_the_sky_anchor() {
    let auto = vxe_render::AutoExposure {
        max: 4.0,
        sky_anchor_ev: Some(4.0),
        ..vxe_render::AutoExposure::eye()
    };
    let mut m = Modes::new(
        vxe_render::Lighting {
            exposure: vxe_render::Exposure::Auto(auto),
            ..vxe_render::Lighting::default()
        },
        vxe_render::LightingResolution::default(),
        0.0,
    );
    m.set_auto(Adaptation::Eye, auto);
    m.sky = true;
    assert!(!m.line().contains("sky anchor"), "{}", m.line());
    let mut two = m.clone();
    two.set_auto(
        Adaptation::Eye,
        vxe_render::AutoExposure {
            sky_anchor_ev: Some(2.0),
            ..auto
        },
    );
    assert!(two.line().contains("sky anchor 2, "), "{}", two.line());
    assert!(!m.line().contains("sky anchor floor"), "{}", m.line());
    for (floor, word) in [
        (0.25, "sky anchor floor 0.25, "),
        (0.0, "sky anchor floor off, "),
    ] {
        let mut f = m.clone();
        f.set_auto(
            Adaptation::Eye,
            vxe_render::AutoExposure {
                sky_anchor_floor: floor,
                ..auto
            },
        );
        assert!(f.line().contains(word), "{}", f.line());
    }
    assert_eq!(m.exposure_row().ceiling, Some((2.0, Some(4.0))));
    m.set_readout(Some(vxe_render::ExposureReadout {
        exposure: 0.01,
        adapted_log2: 0.0,
        metered_log2: 0.0,
        frame: 3,
        held: false,
        ceiling: 0.0625,
        sky_share: 0.23,
        sky_floor: Some(0.12),
        highlight: None,
    }));
    let row = m.exposure_row();
    assert_eq!(row.ceiling, Some((-4.0, Some(4.0))));
    assert_eq!(row.sky_share, Some(0.23));
    assert_eq!(
        row.to_string(),
        "exposure auto eye -6.64 EV  scene +0.0 EV  settles -4.00 EV  ceiling -4.00 EV (anchor 4, \
         sky 23 %, floor 12 %)"
    );
    m.set_readout(Some(vxe_render::ExposureReadout {
        exposure: 0.01,
        adapted_log2: 0.0,
        metered_log2: 0.0,
        frame: 4,
        held: false,
        ceiling: 0.0625,
        sky_share: 0.23,
        sky_floor: None,
        highlight: None,
    }));
    assert!(
        m.exposure_row()
            .to_string()
            .ends_with("(anchor 4, sky 23 %, floor -)"),
        "{}",
        m.exposure_row()
    );
    m.press(ModeKey::Exposure, 0.0);
    let vxe_render::Exposure::Auto(sym) = m.lighting.exposure else {
        panic!("Q from auto eye is auto symmetric");
    };
    assert_eq!((sym.sky_anchor_ev, sym.max), (Some(4.0), 4.0));
    assert_eq!(
        (sym.sky_anchor_floor, sym.sky_around_tau),
        (auto.sky_anchor_floor, auto.sky_around_tau)
    );
    m.sky = false;
    assert!(!m.line().contains("sky anchor"));
}
