//! The mark colours as render settings (ADR-0074, ADR-0055), on the CPU.

use vxe_render::{ApplyClass, MARK_COLOURS, Profile, RenderSettings};

/// Plan 0077 Phase 6 done-when 4: the first and the second mark colours are one `Live` field
/// of the settings, the blue `[0.35, 0.65, 1.0]` and the orange `[1.0, 0.55, 0.15]`, the same
/// in every profile, and the settings' table prints them.
#[test]
fn the_mark_colours_are_settings() {
    assert_eq!(MARK_COLOURS, [[0.35, 0.65, 1.0], [1.0, 0.55, 0.15]]);
    assert_eq!(
        RenderSettings::class_of("mark_colours"),
        Some(ApplyClass::Live)
    );
    for p in Profile::ALL {
        let s = RenderSettings::profile(p);
        assert_eq!(s.mark_colours, MARK_COLOURS, "{p:?}");
        let row = s
            .table()
            .into_iter()
            .find(|(n, _, _)| *n == "mark_colours")
            .expect("a row");
        assert_eq!(row.2, "[[0.35, 0.65, 1.0], [1.0, 0.55, 0.15]]", "{p:?}");
    }
    assert!(
        RenderSettings::PROFILE_PICKS
            .iter()
            .all(|(n, _)| *n != "mark_colours"),
        "no profile picks its own"
    );
}
