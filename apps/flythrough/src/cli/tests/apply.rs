//! `--apply-mib`: the bake thread's apply cap.

use crate::cli::parse_args_from;
use crate::test_support::argv;

/// `--apply-mib N` sets the MiB a frame applies of the bake thread's lists, 1 to 16; the
/// default is `svet_render::DEFAULT_APPLY_CAP` (2 MiB); 0, 17 and a missing value are refused.
/// The banner's row names it.
#[test]
fn the_apply_mib_flag_parses_one_to_sixteen() {
    let a = parse_args_from(argv(&[])).expect("no flags parse");
    assert_eq!(a.bake.apply_mib, 2);
    assert_eq!(a.bake.apply_mib << 20, svet_render::DEFAULT_APPLY_CAP);
    assert_eq!(
        crate::hud::bake_apply_row(a.bake.apply_mib),
        "bake apply 2 MiB a frame"
    );
    let a = parse_args_from(argv(&["--apply-mib", "1"])).expect("--apply-mib 1 parses");
    assert_eq!(a.bake.apply_mib, 1);
    let a = parse_args_from(argv(&["--apply-mib", "16"])).expect("--apply-mib 16 parses");
    assert_eq!(a.bake.apply_mib, 16);
    for bad in [
        &["--apply-mib", "0"][..],
        &["--apply-mib", "17"],
        &["--apply-mib"],
    ] {
        assert!(parse_args_from(argv(bad)).is_err(), "{bad:?}");
    }
}
