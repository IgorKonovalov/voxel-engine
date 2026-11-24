use super::*;

/// The exposure pass parses and validates headless, after the prelude.
#[test]
fn exposure_wgsl_validates_through_naga() {
    let source = exposure_source();
    let module = wgpu::naga::front::wgsl::parse_str(&source)
        .unwrap_or_else(|e| panic!("exposure.wgsl parses: {}", e.emit_to_string(&source)));
    wgpu::naga::valid::Validator::new(
        wgpu::naga::valid::ValidationFlags::all(),
        wgpu::naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .unwrap_or_else(|e| panic!("exposure.wgsl validates: {e:?}"));
}

/// The state, the uniform and the highlight are the shader's sizes: 32 bytes, 128 and 16;
/// a meter tile is 16.
#[test]
fn the_state_and_the_uniform_have_the_shaders_layout() {
    assert_eq!(size_of::<ExposureState>(), 32);
    assert_eq!(size_of::<ExposureUniform>(), 128);
    assert_eq!(size_of::<ExposureHighlight>(), 16);
    assert_eq!(METER_TILE_BYTES, 16);
}

/// The centre weight is 1 + strength at the centre and falls to 1 far out; the average
/// weighs 1 everywhere.
#[test]
fn the_centre_weight_peaks_at_one_plus_strength() {
    let c = Metering::default();
    assert_eq!(c.weight(0.0), 4.0);
    assert!((c.weight(0.5) - (1.0 + 3.0 * (-0.5f32).exp())).abs() < 1e-6);
    assert!((c.weight(5.0) - 1.0).abs() < 1e-6);
    assert_eq!(Metering::Average.weight(0.0), 1.0);
}

/// The reference model's worked numbers (Plan 0040): from `a = -4` toward `t = -8` the
/// floor `-6` is reached at `2 ln 2` s, and the rod stage leaves `2 exp(-(t - 2 ln 2) / 10)`;
/// toward bright the step is `4 exp(-t / 0.15)`; a step of 0 holds; `instant` jumps.
#[test]
fn the_reference_step_follows_the_two_stage_curve() {
    let e = AutoExposure {
        rate_ev_s: 0.0,
        ..AutoExposure::eye()
    };
    let a = |t: f64| e.step(-4.0, -8.0, t);
    let floor_at = 2.0 * std::f64::consts::LN_2;
    assert!((a(floor_at) + 6.0).abs() < 1e-12);
    assert!((a(0.5) - (-8.0 + 4.0 * (-0.25f64).exp())).abs() < 1e-12);
    for t in [2.0, 4.0, 16.0] {
        let want = -8.0 + 2.0 * (-(t - floor_at) / 10.0).exp();
        assert!((a(t) - want).abs() < 1e-12, "{t}: {} {want}", a(t));
    }
    // Stepping in frames lands on the closed form.
    let mut x = -4.0;
    for _ in 0..600 {
        x = e.step(x, -8.0, 1.0 / 60.0);
    }
    assert!((x - a(10.0)).abs() < 1e-9, "{x} {}", a(10.0));
    assert!(
        (e.step(-8.0, -4.0, 0.15) - (-4.0 - 4.0 * (-1f64).exp())).abs() < 1e-6,
        "tau_light is 0.15 in f32"
    );
    assert_eq!(e.step(-5.3, -8.0, 0.0), -5.3);
    assert_eq!(AutoExposure::instant().step(-5.3, -8.0, 1.0 / 60.0), -8.0);
    assert!((e.exposure(-6.0) - f64::from(0.18f32) * 64.0).abs() < 1e-9);
    assert_eq!(e.exposure(-8.0), 16.0);
    assert_eq!(e.exposure(-12.0), 16.0);
    let s = AutoExposure::symmetric(2.0);
    assert!((s.step(-1.0, -3.0, 0.5) - (-3.0 + 2.0 * (-1f64).exp())).abs() < 1e-12);
}

/// The frame after a step of `d` EV toward a brighter meter by which the capped eye is within
/// 1/64 EV of its target, from the model's constants: with `r` the per-frame cap and `tau`
/// ADR-0033's light constant in frames, the cap binds while the gap exceeds `tau r`, then the
/// exponential tail decays from there, plus 2 frames of rounding.
pub(crate) fn landing_frame(d: f64, r: f64, tau: f64) -> u32 {
    ((d - tau * r) / r).ceil() as u32 + (tau * (64.0 * tau * r).ln()).ceil() as u32 + 2
}

/// ADR-0076 rule 5 (Plan 0094 Phase 4, the rate of Phase 7a): at the default rate and `dt`
/// 1/60 a 2 EV step toward a brighter meter moves the adapted level by at most the rate's
/// frame step (1/16 EV at 3.75 EV/s) within 1/256, monotonically, the first step being the
/// cap, and is within 1/64 EV of the target by [`landing_frame`] (the frame it reaches printed);
/// a long `dt` steps no further than a 15 fps frame's; with the rate 0 the step is
/// ADR-0033's; a still meter holds the level.
#[test]
fn the_rate_caps_a_step() {
    let e = AutoExposure::eye();
    assert_eq!(e.rate_ev_s, DEFAULT_EXPOSURE_RATE_EV_S);
    let dt = 1.0 / 60.0;
    let r_f = f64::from(DEFAULT_EXPOSURE_RATE_EV_S) * dt;
    let landing = landing_frame(2.0, r_f, f64::from(e.tau_light) / dt);
    let (a0, t) = (-4.0, -2.0);
    let mut a = a0;
    let mut trace = vec![a];
    let mut reached = None;
    for k in 1..=160 {
        let next = e.step(a, t, dt);
        assert!(
            next >= a && next <= t,
            "monotonic toward the target: {a} -> {next}"
        );
        assert!((next - a) <= r_f + 1.0 / 256.0, "{a} -> {next}");
        a = next;
        trace.push(a);
        if reached.is_none() && (a - t).abs() <= 1.0 / 64.0 {
            reached = Some(k);
        }
    }
    println!(
        "reading: a 2 EV step at {DEFAULT_EXPOSURE_RATE_EV_S} EV/s is within 1/64 EV at frame \
         {reached:?}, the derived landing frame {landing}"
    );
    assert!(
        reached.is_some_and(|k| k <= landing),
        "{reached:?} {landing}"
    );
    assert!((trace[1] - (a0 + r_f)).abs() <= 1.0 / 256.0, "{trace:?}");
    let long = e.step(a0, t, 1.0);
    assert!((long - a0 - f64::from(DEFAULT_EXPOSURE_RATE_EV_S * MAX_RATE_DT)).abs() < 1e-6);
    let free = AutoExposure {
        rate_ev_s: 0.0,
        ..e
    };
    assert!(
        free.step(a0, t, dt) > a0 + r_f,
        "ADR-0033's bright step is faster"
    );
    assert_eq!(e.step(-3.0, -3.0, dt), -3.0);
    assert!((AutoExposure::eye().step(-3.0, -3.0 + 1.0 / 128.0, dt) + 3.0).abs() <= 1.0 / 64.0);
}

/// Plan 0086 Phase 17: the auto exposure's default anchors its ceiling 4 EV over the
/// sunlit ground, and with no sky that ceiling is `max`.
#[test]
fn the_default_anchor_is_4() {
    assert_eq!(AutoExposure::default().sky_anchor_ev, Some(4.0));
    assert_eq!(
        AutoExposure::eye().sky_anchor_ev,
        Some(DEFAULT_SKY_ANCHOR_EV)
    );
    assert_eq!(
        AutoExposure::default().ceiling(None),
        AutoExposure::default().max
    );
}

/// Plan 0086 Phase 12 bar 1: the anchored ceiling follows the sky. Under `Sky::uniform(L)`
/// with no key light `E_sky(+z)` is `pi L`, so at `N` 0 the ceiling is `key pi / (0.5 pi
/// L)`; doubling `L` halves it, `N` 2 quadruples it; with no sky the ceiling is `max`, and
/// the pass's uniform is byte for byte the one without the anchor, so the state it writes
/// is too.
#[test]
fn the_ceiling_follows_the_sky() {
    use std::f32::consts::PI;
    let at = |n: f32| AutoExposure {
        sky_anchor_ev: Some(n),
        ..AutoExposure::eye()
    };
    let ground = |l: f32| Some(ground_irradiance([PI * l; 3], 0.0));
    for l in [0.5f32, 1.0, 37.0, 2400.0] {
        let want = 0.18 * PI / (0.5 * PI * l);
        let c0 = at(0.0).ceiling(ground(l));
        assert!((c0 - want).abs() <= 1e-6 * want, "{l}: {c0} {want}");
        let half = at(0.0).ceiling(ground(2.0 * l));
        assert!((half - 0.5 * c0).abs() <= 1e-6 * c0, "{l}: {half} {c0}");
        let four = at(2.0).ceiling(ground(l));
        assert!((four - 4.0 * c0).abs() <= 1e-6 * c0, "{l}: {four} {c0}");
    }
    // The key light's horizontal share adds to the sky's.
    assert_eq!(ground_irradiance([1.0, 1.0, 1.0], 2.0), 3.0);
    // No sky, or no anchor: `max`.
    let none = AutoExposure {
        sky_anchor_ev: None,
        ..AutoExposure::eye()
    };
    assert_eq!(at(2.0).ceiling(None), none.max);
    assert_eq!(at(2.0).ceiling(Some(0.0)), none.max);
    assert_eq!(none.ceiling(ground(1.0)), none.max);
    // A ground so dim the anchored exposure is over `max` (as at night): the anchor never
    // lifts the ceiling, it is `max` exactly, and the target is `max` at every share.
    let dim = ground(1e-3);
    let open = 4.0 * 0.18 * PI / (0.5 * PI * 1e-3);
    assert!(open > at(2.0).max, "{open}");
    assert_eq!(at(2.0).ceiling(dim), at(2.0).max);
    for k in 0..=64 {
        let sky = k as f32 / 64.0;
        assert_eq!(
            at(2.0).ceiling_target(dim, sky, None),
            at(2.0).max,
            "share {sky}"
        );
    }
    let uniform = |a: &AutoExposure, sky: bool| {
        let mut u = ExposureUniform::default();
        u.set_auto(a, sky);
        bytemuck::bytes_of(&u).to_vec()
    };
    assert_eq!(uniform(&at(2.0), false), uniform(&none, false));
    assert_eq!(uniform(&none, true), uniform(&none, false));
    assert_ne!(uniform(&at(2.0), true), uniform(&none, true));
    // The settled exposure clamps under the ceiling in force.
    let e = AutoExposure::eye();
    assert_eq!(e.exposure_under(-12.0, 0.25), 0.25);
    assert_eq!(e.exposure_under(-12.0, 1e-9), f64::from(e.min));
}

/// Plan 0094 Phase 7b: the ceiling's target is the anchored one at a sky share of `F` and
/// over, `max` with no sky in view and log2-linear between; where the anchored exposure is
/// over `max` (the night) the anchored ceiling and the target are `max` at any share; a
/// share `F` of 0, no anchor or no sky keep today's ceiling.
#[test]
fn the_ceiling_target_follows_the_sky_in_view() {
    use std::f32::consts::PI;
    // The lift off: the release's top is `max` (the before; the lift's own test below).
    let e = AutoExposure {
        max: 4.0,
        sky_anchor_lift: None,
        ..AutoExposure::eye()
    };
    let f = e.sky_anchor_share;
    assert_eq!(f, DEFAULT_SKY_ANCHOR_SHARE);
    // Noon: a bright ground, the anchored ceiling far under `max`.
    let day = Some(ground_irradiance([PI * 2400.0; 3], 0.0));
    let a = e.ceiling(day);
    assert!(a < e.max, "{a}");
    let near = |x: f32, y: f32| (x.log2() - y.log2()).abs() < 1e-5;
    assert_eq!(e.ceiling_target(day, 1.0, None), a);
    assert_eq!(e.ceiling_target(day, f, None), a);
    assert!(near(e.ceiling_target(day, 0.0, None), e.max));
    assert!(near(
        e.ceiling_target(day, 0.5 * f, None),
        (a * e.max).sqrt()
    ));
    let mut last = e.ceiling_target(day, 0.0, None);
    for k in 1..=32 {
        let c = e.ceiling_target(day, f * k as f32 / 16.0, None);
        assert!(c <= last, "monotonic in the share: {k}");
        last = c;
    }
    // Night: the anchored exposure over `max`; the ceiling and its target are `max`.
    let night = Some(ground_irradiance([PI * 1e-3; 3], 0.0));
    assert_eq!(e.ceiling(night), e.max);
    for sky in [0.0, 0.5 * f, f, 1.0] {
        assert_eq!(e.ceiling_target(night, sky, None), e.max);
    }
    // The before arm, no anchor, no sky.
    let before = AutoExposure {
        sky_anchor_share: 0.0,
        ..e
    };
    assert_eq!(before.ceiling_target(day, 0.0, None), a);
    let none = AutoExposure {
        sky_anchor_ev: None,
        ..e
    };
    assert_eq!(none.ceiling_target(day, 0.0, None), e.max);
    assert_eq!(e.ceiling_target(None, 0.0, None), e.max);
    // The uniform carries the share under the anchor alone.
    let share = |a: &AutoExposure, sky: bool| {
        let mut u = ExposureUniform::default();
        u.set_auto(a, sky);
        u.sky_share
    };
    assert_eq!(share(&e, true), f);
    assert_eq!(share(&e, false), 0.0);
    assert_eq!(share(&none, true), 0.0);
}

/// Plan 0097 Phase 2 bar 1 (ADR-0081): the target holds by the larger of the view's share
/// over `F` and the floor's over `F_floor`. At a day ground: a floor share of `F_floor`
/// alone, or a view share of `F` alone, is `c_a` exactly; neither is `max`; half of
/// `F_floor` is `sqrt(c_a max)`; half of each equals half of the view's alone. With
/// `F_floor` 0 every pair reads as the floor absent (Phase 7b's target); at night `max`
/// at every pair. The uniform carries the term under its bit alone.
#[test]
fn the_ceiling_target_follows_the_floor_too() {
    use std::f32::consts::PI;
    // The lift off: the release's top is `max` (the before; the lift's own test below).
    let e = AutoExposure {
        max: 4.0,
        sky_anchor_lift: None,
        ..AutoExposure::eye()
    };
    let (f, ff) = (e.sky_anchor_share, e.sky_anchor_floor);
    assert_eq!(ff, DEFAULT_SKY_ANCHOR_FLOOR);
    let day = Some(ground_irradiance([PI * 2400.0; 3], 0.0));
    let a = e.ceiling(day);
    assert!(a < e.max, "{a}");
    let ev = |x: f32, y: f32| (f64::from(x).log2() - f64::from(y).log2()).abs();
    assert_eq!(e.ceiling_target(day, 0.0, Some(ff)), a);
    assert_eq!(e.ceiling_target(day, 0.0, Some(1.0)), a);
    assert_eq!(e.ceiling_target(day, f, Some(0.0)), a);
    assert!(ev(e.ceiling_target(day, 0.0, Some(0.0)), e.max) <= 1.0 / 256.0);
    assert!(ev(e.ceiling_target(day, 0.0, None), e.max) <= 1.0 / 256.0);
    let half = e.ceiling_target(day, 0.0, Some(ff / 2.0));
    assert!(ev(half, (a * e.max).sqrt()) <= 1.0 / 256.0, "{half}");
    assert_eq!(
        e.ceiling_target(day, f / 2.0, Some(ff / 2.0)),
        e.ceiling_target(day, f / 2.0, Some(0.0))
    );
    assert_eq!(
        e.ceiling_target(day, f / 2.0, Some(0.0)),
        e.ceiling_target(day, f / 2.0, None)
    );
    // The larger share decides.
    assert_eq!(
        e.ceiling_target(day, f / 4.0, Some(ff / 2.0)),
        e.ceiling_target(day, 0.0, Some(ff / 2.0))
    );
    // `F_floor` 0: the floor's share is ignored, Phase 7b's target at every pair.
    let before = AutoExposure {
        sky_anchor_floor: 0.0,
        ..e
    };
    for sky in [0.0, f / 2.0, f, 1.0] {
        for floor in [None, Some(0.0), Some(ff / 2.0), Some(ff), Some(1.0)] {
            assert_eq!(
                before.ceiling_target(day, sky, floor),
                e.ceiling_target(day, sky, None),
                "sky {sky} floor {floor:?}"
            );
        }
    }
    // Night: `max` at every pair.
    let night = Some(ground_irradiance([PI * 1e-3; 3], 0.0));
    for sky in [0.0, f / 2.0, f, 1.0] {
        for floor in [None, Some(0.0), Some(ff / 2.0), Some(ff), Some(1.0)] {
            assert_eq!(e.ceiling_target(night, sky, floor), e.max);
        }
    }
    // The uniform: the bit and the normalised share, clear when absent or off.
    let around = |a: &AutoExposure, floor: Option<f32>| {
        let mut u = ExposureUniform::default();
        u.set_auto(a, true);
        u.set_around(a, floor);
        (u.mode & EX_AROUND, u.sky_around)
    };
    assert_eq!(around(&e, Some(ff / 2.0)), (EX_AROUND, 0.5));
    assert_eq!(around(&e, Some(0.9)), (EX_AROUND, 1.0));
    assert_eq!(around(&e, None), (0, 0.0));
    assert_eq!(around(&before, Some(0.9)), (0, 0.0));
}

/// Plan 0097 Phase 3b bar 1 (ADR-0081, amended): the release is bounded by the lift. At a day
/// ground whose anchored ceiling `c_a` is 6 EV under `max`, with `L` 4: no share is `c_a 2^4`,
/// half of `F_floor` is `c_a 2^2` (the mix's midpoint), `F` or `F_floor` is `c_a` exactly;
/// `L` 8 (over the gap) clamps to `max`; `L` 0 is `c_a` at every pair; at night `max` at
/// every pair. The uniform carries `2^L` under the anchor alone.
#[test]
fn the_release_is_bounded_by_the_lift() {
    let e = AutoExposure::eye();
    assert_eq!(e.sky_anchor_lift, Some(DEFAULT_SKY_ANCHOR_LIFT));
    assert_eq!(DEFAULT_SKY_ANCHOR_LIFT, 4.0);
    let (f, ff) = (e.sky_anchor_share, e.sky_anchor_floor);
    // The ground that puts `c_a` at `max / 64`: `c_a = 2^N key / (0.5 g / pi)`.
    let c_want = e.max / 64.0;
    let g = DEFAULT_SKY_ANCHOR_EV.exp2() * e.key * 2.0 * std::f32::consts::PI / c_want;
    let day = Some(g);
    let a = e.ceiling(day);
    let ev = |x: f32, y: f32| (f64::from(x).log2() - f64::from(y).log2()).abs();
    assert!(ev(a, c_want) <= 1e-5, "{a} {c_want}");
    let lift = |l: Option<f32>| AutoExposure {
        sky_anchor_lift: l,
        ..e
    };
    let l4 = lift(Some(4.0));
    let top = l4.ceiling_target(day, 0.0, None);
    assert!(ev(top, a * 16.0) <= 1.0 / 256.0, "{top} {}", a * 16.0);
    let mid = l4.ceiling_target(day, 0.0, Some(ff / 2.0));
    assert!(ev(mid, a * 4.0) <= 1.0 / 256.0, "{mid} {}", a * 4.0);
    assert_eq!(l4.ceiling_target(day, f, Some(0.0)), a);
    assert_eq!(l4.ceiling_target(day, 0.0, Some(ff)), a);
    // `L` 8, over the 6 EV gap: `c_top` clamps to `max`.
    let open = lift(Some(8.0)).ceiling_target(day, 0.0, None);
    assert!(ev(open, e.max) <= 1.0 / 256.0, "{open}");
    // `L` 0: the hard anchor at every pair.
    let pairs = [
        (0.0, None),
        (0.0, Some(0.0)),
        (f / 2.0, None),
        (0.0, Some(ff / 2.0)),
        (f, Some(0.0)),
        (0.0, Some(ff)),
        (1.0, Some(1.0)),
    ];
    for (sky, floor) in pairs {
        assert_eq!(
            lift(Some(0.0)).ceiling_target(day, sky, floor),
            a,
            "sky {sky} floor {floor:?}"
        );
    }
    // Night (`c_a >= max`): `max` at every pair with `L` 4.
    let night = Some(ground_irradiance([std::f32::consts::PI * 1e-3; 3], 0.0));
    assert_eq!(l4.ceiling(night), e.max);
    for (sky, floor) in pairs {
        assert_eq!(l4.ceiling_target(night, sky, floor), e.max);
    }
    // The uniform: `2^L` under the anchor, 0 without the anchor or the lift.
    let word = |a: &AutoExposure, sky: bool| {
        let mut u = ExposureUniform::default();
        u.set_auto(a, sky);
        u.lift
    };
    assert_eq!(word(&l4, true), 16.0);
    assert_eq!(word(&l4, false), 0.0);
    assert_eq!(word(&lift(None), true), 0.0);
    assert_eq!(word(&lift(Some(0.0)), true), 1.0);
}

/// Plan 0094 Phase 7b bar 4's model (ADR-0076 rule 5 on the ceiling): from the anchored
/// ceiling toward `max` and back the ceiling in force moves by at most the rate's frame
/// step, monotonically, and lands on its target; at rate 0, from no ceiling, with no anchor
/// or with the share 0 it jumps.
#[test]
fn the_ceiling_steps_at_the_rate() {
    use std::f32::consts::PI;
    let e = AutoExposure {
        max: 4.0,
        ..AutoExposure::eye()
    };
    let dt = 1.0 / 60.0;
    let r_f = f64::from(e.rate_ev_s) * dt;
    let day = Some(ground_irradiance([PI * 2400.0; 3], 0.0));
    let a = e.ceiling(day);
    let gap = f64::from(e.max.log2() - a.log2());
    let frames = (gap / r_f).ceil() as usize;
    let mut c = a;
    let mut trace = vec![c.log2()];
    for target in [e.max, a] {
        for _ in 0..frames + 2 {
            let next = e.step_ceiling(day, c, target, dt);
            let d = f64::from(next.log2() - c.log2());
            assert!(d.abs() <= r_f + 1.0 / 256.0, "{c} -> {next}");
            assert!(
                (target >= c && next >= c) || (target <= c && next <= c),
                "monotonic: {c} -> {next} toward {target}"
            );
            c = next;
            trace.push(c.log2());
        }
        assert!((c.log2() - target.log2()).abs() < 1e-4, "{trace:?}");
    }
    println!("reading: the ceiling's trace in EV {trace:?}");
    let jump = |m: &AutoExposure, g: Option<f32>, from: f32| m.step_ceiling(g, from, e.max, dt);
    let free = AutoExposure {
        rate_ev_s: 0.0,
        ..e
    };
    assert_eq!(jump(&free, day, a), e.max);
    assert_eq!(jump(&e, day, 0.0), e.max);
    assert_eq!(jump(&e, None, a), e.max);
    let before = AutoExposure {
        sky_anchor_share: 0.0,
        ..e
    };
    assert_eq!(jump(&before, day, a), e.max);
}
