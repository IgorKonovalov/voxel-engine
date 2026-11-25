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

/// Plan 0100 Phase 1: `key_at` is `hit` by default, which sets no `lighting2` bit, so the
/// default's second word of switches is the frame's before the setting (`sky_down_skip` alone);
/// `face` and `face-smooth` each set their own bit, and nothing else of either word moves; every
/// value round-trips through its name and `next` cycles the three; the withdrawn `centre` (Plan
/// 0100 Phase 9) is no name.
#[test]
fn the_key_at_hit_sets_no_bit() {
    use super::{
        KeyAt, LIGHTING2_KEY_AT_FACE, LIGHTING2_KEY_AT_SMOOTH, LIGHTING2_SKY_DOWN_SKIP, Lighting,
    };
    let d = Lighting::default();
    assert_eq!(d.key_at, KeyAt::Hit);
    assert_eq!(d.flags2(), 0);
    let skip = Lighting {
        sky_down_skip: true,
        ..d
    };
    assert_eq!(skip.flags2(), LIGHTING2_SKY_DOWN_SKIP);
    let at = |key_at| Lighting { key_at, ..d };
    assert_eq!(at(KeyAt::Face).flags2(), LIGHTING2_KEY_AT_FACE);
    assert_eq!(at(KeyAt::FaceSmooth).flags2(), LIGHTING2_KEY_AT_SMOOTH);
    for k in KeyAt::ALL {
        assert_eq!(at(k).flags(), d.flags(), "{}", k.name());
        assert_eq!(KeyAt::from_name(k.name()), Some(k));
    }
    assert_eq!(KeyAt::ALL.map(KeyAt::name), ["hit", "face", "face-smooth"]);
    assert_eq!(KeyAt::from_name("centre"), None);
    assert_eq!(
        KeyAt::ALL.map(KeyAt::next),
        [KeyAt::Face, KeyAt::FaceSmooth, KeyAt::Hit]
    );
}

/// Plan 0100 Phase 5b: `key_horizon` is `march` by default, which sets no bit; `walk` sets
/// LIGHTING2_KEY_WALK alone and leaves the first word; the names round-trip and `next` cycles.
#[test]
fn the_key_horizon_march_sets_no_bit() {
    use super::{KeyHorizon, LIGHTING2_KEY_WALK, Lighting};
    let d = Lighting::default();
    assert_eq!(d.key_horizon, KeyHorizon::March);
    assert_eq!(d.flags2(), 0);
    let walk = Lighting {
        key_horizon: KeyHorizon::Walk,
        ..d
    };
    assert_eq!(walk.flags2(), LIGHTING2_KEY_WALK);
    assert_eq!(walk.flags(), d.flags());
    assert_eq!(KeyHorizon::ALL.map(KeyHorizon::name), ["march", "walk"]);
    for k in KeyHorizon::ALL {
        assert_eq!(KeyHorizon::from_name(k.name()), Some(k));
    }
    assert_eq!(
        KeyHorizon::ALL.map(KeyHorizon::next),
        [KeyHorizon::Walk, KeyHorizon::March]
    );
}

/// Plan 0100 Phase 6: `key_cover` is `off` by default, which sets no bit; `ray` and `record` set
/// their bit alone, and none touches the first word; the names round-trip and `next` cycles; the
/// withdrawn `ray-record` (Plan 0100 Phase 9) is no name.
#[test]
fn the_key_cover_off_sets_no_bit() {
    use super::{KeyCover, LIGHTING2_KEY_COVER_RAY, LIGHTING2_KEY_COVER_RECORD, Lighting};
    let d = Lighting::default();
    assert_eq!(d.key_cover, KeyCover::Off);
    assert_eq!(d.flags2(), 0);
    let bits = KeyCover::ALL.map(|key_cover| {
        let l = Lighting { key_cover, ..d };
        assert_eq!(l.flags(), d.flags());
        l.flags2()
    });
    assert_eq!(
        bits,
        [0, LIGHTING2_KEY_COVER_RAY, LIGHTING2_KEY_COVER_RECORD]
    );
    assert_eq!(KeyCover::ALL.map(KeyCover::name), ["off", "ray", "record"]);
    assert_eq!(KeyCover::from_name("ray-record"), None);
    for k in KeyCover::ALL {
        assert_eq!(KeyCover::from_name(k.name()), Some(k));
    }
    assert_eq!(
        KeyCover::ALL.map(KeyCover::next),
        [KeyCover::Ray, KeyCover::Record, KeyCover::Off]
    );
}
