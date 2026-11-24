//! The frame settings' tests: the scale, the sky term's defaults and the key light's bits.

use super::RenderScale;

#[test]
fn the_internal_size_rounds_the_output_over_the_scale() {
    assert_eq!(RenderScale::One.internal([2530, 1378]), [2530, 1378]);
    assert_eq!(RenderScale::OneHalf.internal([2530, 1378]), [1687, 919]);
    assert_eq!(RenderScale::Two.internal([385, 1]), [193, 1]);
    for s in RenderScale::ALL {
        assert_eq!(RenderScale::from_name(s.name()), Some(s));
    }
}

/// Plan 0086 Phase 17: the sky term reads the smoothed normal by default, with no bounce.
#[test]
fn the_sky_term_defaults_to_the_smoothed_normal() {
    let l = super::Lighting::default();
    assert_eq!(
        (l.sky_normal, l.sky_bounce),
        (super::SkyNormal::Smoothed, 0.0)
    );
}

/// Plan 0095 Phases 2 and 4: `key_far ray` sets no key bit whatever the mix, so its lighting
/// bits are the frame's before the channel (Phase 2's, to the bit); `baked` sets the far
/// estimate's bit, `column` the horizon's beside it, and `step` its own beside either; every
/// value round-trips through its name.
#[test]
fn the_key_far_ray_sets_no_key_bit() {
    use super::{
        KeyFar, KeyFarMix, LIGHTING_KEY_BAKED, LIGHTING_KEY_COLUMN, LIGHTING_KEY_STEP, Lighting,
    };
    let keys = LIGHTING_KEY_BAKED | LIGHTING_KEY_STEP | LIGHTING_KEY_COLUMN;
    let at = |key_far, key_far_mix| {
        Lighting {
            key_far,
            key_far_mix,
            ..Lighting::default()
        }
        .flags()
    };
    let ray = at(KeyFar::Ray, KeyFarMix::Blend);
    assert_eq!(ray & keys, 0);
    assert_eq!(at(KeyFar::Ray, KeyFarMix::Step), ray);
    assert_eq!(
        at(KeyFar::Baked, KeyFarMix::Blend),
        ray | LIGHTING_KEY_BAKED
    );
    assert_eq!(
        at(KeyFar::Baked, KeyFarMix::Step),
        ray | LIGHTING_KEY_BAKED | LIGHTING_KEY_STEP
    );
    assert_eq!(
        at(KeyFar::Column, KeyFarMix::Blend),
        ray | LIGHTING_KEY_BAKED | LIGHTING_KEY_COLUMN
    );
    assert_eq!(at(KeyFar::Column, KeyFarMix::Step), ray | keys);
    assert_eq!(
        (Lighting::default().key_far, Lighting::default().key_far_mix),
        (KeyFar::Column, KeyFarMix::Step)
    );
    assert_eq!(KeyFar::ALL.map(KeyFar::name), ["baked", "column", "ray"]);
    for k in KeyFar::ALL {
        assert_eq!(KeyFar::from_name(k.name()), Some(k));
    }
}

/// Plan 0086 Phase 25 (the 86d window's state): the key light's visibility mean holds 256
/// frames by default and the sky's 64, each its own cap.
#[test]
fn the_key_mean_holds_256_frames_by_default() {
    let l = super::Lighting::default();
    assert_eq!((l.key_history, l.sky_history), (256, 64));
    assert_eq!(super::DEFAULT_KEY_HISTORY, 256);
}
