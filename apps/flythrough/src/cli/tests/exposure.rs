//! The auto exposure's compensation: the showcase's low key and `--exposure-comp`.

use crate::cli::{SHOWCASE_EXPOSURE_COMP, parse_args_from};
use crate::hud;
use crate::test_support::argv;
use vxe_render::Exposure;

/// `--exposure-comp`: the showcase's auto exposure starts at `SHOWCASE_EXPOSURE_COMP` (-1.5
/// EV) and every other source at 0; the flag sets it on any source, refusing values past 4 EV;
/// the banner names a compensation that is not 0 (`exposure comp -1.5 EV`) and none at 0;
/// the exposure key's symmetric preset keeps it.
#[test]
fn the_exposure_compensation_is_the_showcases_low_key() {
    let auto = |v: &[&str]| match parse_args_from(argv(v)).unwrap().start_lighting().exposure {
        Exposure::Auto(a) => a.compensation_ev,
        Exposure::Manual(_) => panic!("auto"),
    };
    assert_eq!(SHOWCASE_EXPOSURE_COMP, -1.5);
    assert_eq!(auto(&["--source", "showcase", "--exposure", "auto"]), -1.5);
    assert_eq!(auto(&[]), 0.0);
    assert_eq!(auto(&["--source", "outdoor"]), 0.0);
    let zero = [
        "--source",
        "showcase",
        "--exposure",
        "auto",
        "--exposure-comp",
        "0",
    ];
    assert_eq!(auto(&zero), 0.0);
    assert_eq!(auto(&["--exposure-comp", "-2"]), -2.0);
    assert_eq!(
        parse_args_from(argv(&["--exposure-comp", "5"]))
            .err()
            .as_deref(),
        Some("--exposure-comp takes -4 to 4, not 5")
    );
    let line = |v: &[&str]| parse_args_from(argv(v)).unwrap().start_modes(0.0).line();
    let show = line(&["--source", "showcase", "--exposure", "auto"]);
    assert!(show.contains("exposure comp -1.5 EV"), "{show}");
    assert!(!line(&[]).contains("exposure comp"));
    // The showcase starts at its shot's manual exposure, which no compensation moves.
    assert!(!line(&["--source", "showcase"]).contains("exposure comp"));
    let mut m = parse_args_from(argv(&["--exposure-comp", "-1"]))
        .unwrap()
        .start_modes(0.0);
    m.press(hud::ModeKey::Exposure, 0.0);
    assert!(m.exposure_mode().starts_with("auto symmetric"));
    assert!(m.line().contains("exposure comp -1.0 EV"), "{}", m.line());
}
