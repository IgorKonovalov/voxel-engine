//! The auto exposure's compensation: `--exposure-comp`.

use crate::cli::parse_args_from;
use crate::hud;
use crate::test_support::argv;
use svet_render::Exposure;

/// `--exposure-comp`: every source's auto exposure starts at 0 EV; the flag sets it on any
/// source, refusing values past 4 EV; the banner names a compensation that is not 0
/// (`exposure comp -1.5 EV`) and none at 0; the exposure key's symmetric preset keeps it.
#[test]
fn the_exposure_compensation_is_a_flag_at_zero_by_default() {
    let auto = |v: &[&str]| match parse_args_from(argv(v)).unwrap().start_lighting().exposure {
        Exposure::Auto(a) => a.compensation_ev,
        Exposure::Manual(_) => panic!("auto"),
    };
    assert_eq!(auto(&["--source", "showcase", "--exposure", "auto"]), 0.0);
    assert_eq!(auto(&[]), 0.0);
    assert_eq!(auto(&["--source", "outdoor"]), 0.0);
    let low = [
        "--source",
        "showcase",
        "--exposure",
        "auto",
        "--exposure-comp",
        "-1.5",
    ];
    assert_eq!(auto(&low), -1.5);
    assert_eq!(auto(&["--exposure-comp", "-2"]), -2.0);
    assert_eq!(
        parse_args_from(argv(&["--exposure-comp", "5"]))
            .err()
            .as_deref(),
        Some("--exposure-comp takes -4 to 4, not 5")
    );
    let line = |v: &[&str]| parse_args_from(argv(v)).unwrap().start_modes(0.0).line();
    let show = line(&low);
    assert!(show.contains("exposure comp -1.5 EV"), "{show}");
    assert!(!line(&["--source", "showcase", "--exposure", "auto"]).contains("exposure comp"));
    assert!(!line(&[]).contains("exposure comp"));
    // The showcase starts at its shot's manual exposure, which no compensation moves.
    assert!(!line(&["--source", "showcase", "--exposure-comp", "-1"]).contains("exposure comp"));
    let mut m = parse_args_from(argv(&["--exposure-comp", "-1"]))
        .unwrap()
        .start_modes(0.0);
    m.press(hud::ModeKey::Exposure, 0.0);
    assert!(m.exposure_mode().starts_with("auto symmetric"));
    assert!(m.line().contains("exposure comp -1.0 EV"), "{}", m.line());
}
