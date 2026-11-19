//! The light-stability rules' forms (`support/rules.rs`) on constructed traces and shots, on
//! the CPU: each form counts or classes what its rule names and nothing else.

#[path = "support/rules.rs"]
mod rules;

use rules::{F, T_FADE, T_RETURN, T_STILL, TraceClass, trace_class, trace_classes};

/// A grey rgba8 shot of `n` pixels at `v`.
fn grey(n: usize, v: u8) -> Vec<u8> {
    (0..n).flat_map(|_| [v, v, v, 255]).collect()
}

/// A trace stepping from 100 to 140 linearly over `ramp` frames from the event: the 8 frames
/// before, the event's `F + 8` frames, the 8 after.
fn ramp(ramp: usize) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let before = vec![100u8; 8];
    let event = (0..F + 8)
        .map(|k| {
            let t = ((k + 1) as f32 / ramp as f32).min(1.0);
            (100.0 + 40.0 * t).round() as u8
        })
        .collect();
    (before, event, vec![140u8; 8])
}

#[test]
fn a_linear_fade_inside_f_is_a_fade() {
    let (b, e, a) = ramp(F);
    assert_eq!(trace_class(&b, &e, &a, T_FADE, F), TraceClass::Fade);
}

#[test]
fn a_flash_above_the_band_is_a_flash() {
    let (b, mut e, a) = ramp(F);
    e[F + 2] = 140 + T_FADE + 1;
    assert_eq!(trace_class(&b, &e, &a, T_FADE, F), TraceClass::Flash);
    // On the band's edge it is not.
    let (b, mut e, a) = ramp(F);
    e[F + 2] = 140 + T_FADE;
    assert_eq!(trace_class(&b, &e, &a, T_FADE, F), TraceClass::Fade);
}

#[test]
fn a_dip_below_the_band_is_a_dip() {
    let (b, mut e, a) = ramp(F);
    e[0] = 100 - T_FADE - 1;
    assert_eq!(trace_class(&b, &e, &a, T_FADE, F), TraceClass::Dip);
}

#[test]
fn reaching_new_at_frame_f_plus_one_is_slow() {
    // Frame index F + 1 is the first within T_FADE of new: 140 - 5 until then.
    let before = vec![100u8; 8];
    let event: Vec<u8> = (0..F + 8).map(|k| if k <= F { 135 } else { 140 }).collect();
    let after = vec![140u8; 8];
    assert_eq!(
        trace_class(&before, &event, &after, T_FADE, F),
        TraceClass::Slow
    );
    // The same trace reaching new at frame F is a fade.
    let event: Vec<u8> = (0..F + 8).map(|k| if k < F { 135 } else { 140 }).collect();
    assert_eq!(
        trace_class(&before, &event, &after, T_FADE, F),
        TraceClass::Fade
    );
}

#[test]
fn leaving_new_after_reaching_it_is_an_overshoot() {
    let (b, mut e, a) = ramp(4);
    // Back towards old by T_FADE + 1 after reaching new: inside the band, not a flash or dip.
    e[10] = 140 - T_FADE - 1;
    assert_eq!(trace_class(&b, &e, &a, T_FADE, F), TraceClass::Overshoot);
}

#[test]
fn a_still_pixel_is_a_fade() {
    assert_eq!(
        trace_class(&[90; 8], &[90; F + 8], &[90; 8], T_FADE, F),
        TraceClass::Fade
    );
}

#[test]
fn rule_1_counts_the_pixels_over_t_still_inside_the_region() {
    let a = grey(4, 100);
    let mut b = a.clone();
    b[0] = 100 + T_STILL + 1; // pixel 0, red channel
    b[4 + 1] = 100 + T_STILL; // pixel 1, on the line
    b[8 + 2] = 100 - T_STILL - 2; // pixel 2, blue channel
    assert_eq!(rules::still_count(&a, &b, T_STILL, None), 2);
    let region = [false, true, true, true];
    assert_eq!(rules::still_count(&a, &b, T_STILL, Some(&region)), 1);
}

#[test]
fn rule_2_counts_a_return_frame_nine_levels_off() {
    let settled = grey(3, 50);
    let mut returned = settled.clone();
    returned[0] = 50 + 9;
    returned[4] = 50 + T_RETURN;
    assert_eq!(rules::return_count(&settled, &returned, T_RETURN), 1);
}

#[test]
fn rule_4_counts_a_pixel_outside_the_region_moving_five() {
    let s0 = grey(3, 70);
    let mut s1 = s0.clone();
    s1[0] = 75; // pixel 0 outside, moves 5
    s1[4] = 90; // pixel 1 inside, moves 20: the event's own
    s1[8] = 74; // pixel 2 outside, moves 4: on the line
    let s2 = s0.clone();
    let inside = [false, true, false];
    assert_eq!(rules::outside_count(&[&s0, &s1, &s2], T_STILL, &inside), 1);
}

#[test]
fn rule_3_over_a_region_counts_each_class() {
    let n = 3;
    let len = rules::MEAN_FRAMES + F + 16;
    let shots: Vec<Vec<u8>> = (0..len)
        .map(|k| {
            let mut s = grey(n, 100);
            // Pixel 1 flashes on the event's frame; pixel 2 steps to 160 at once (a fade
            // of one frame is inside the band: its trace reaches new at frame 0).
            if k == rules::MEAN_FRAMES {
                s[4] = 200;
            }
            if k >= rules::MEAN_FRAMES {
                s[8] = 160;
            }
            s
        })
        .collect();
    let refs: Vec<&[u8]> = shots.iter().map(Vec::as_slice).collect();
    let c = trace_classes(&refs, &[true, true, true], T_FADE, F);
    assert_eq!(c[TraceClass::Fade.index()], 2);
    assert_eq!(c[TraceClass::Flash.index()], 1);
}
