//! Where the streamed mirror's new records are packed: `--mirror-pack`.

use crate::cli::parse_args_from;
use crate::hud;
use crate::test_support::argv;
use crate::toggles::TOGGLES;
use svet_render::Packing;

/// `--mirror-pack` takes `inline`, the default, and `workers`, and refuses another word; the
/// modes start at the flag's packing and the HUD's row and the banner's read
/// `mirror pack inline` and `mirror pack workers`; `U` flips the packing back and forth.
#[test]
fn the_mirror_pack_flag_names_both_packings() {
    let pack = |v: &[&str]| parse_args_from(argv(v)).expect("parses").mirror_pack;
    assert_eq!(pack(&[]), Packing::Inline);
    assert_eq!(pack(&["--mirror-pack", "inline"]), Packing::Inline);
    assert_eq!(pack(&["--mirror-pack", "workers"]), Packing::Workers);
    assert!(parse_args_from(argv(&["--mirror-pack", "render"])).is_err());
    assert_eq!(hud::mirror_pack_row(Packing::Inline), "mirror pack inline");
    assert_eq!(
        hud::mirror_pack_row(Packing::Workers),
        "mirror pack workers"
    );
    let mut m = parse_args_from(argv(&["--mirror-pack", "workers"]))
        .unwrap()
        .start_modes(0.0);
    assert_eq!(m.row(hud::HudRow::Mirror), "mirror pack workers");
    let key = TOGGLES
        .iter()
        .find(|t| t.name == "mirror pack")
        .expect("the toggle");
    assert_eq!(
        key.press,
        crate::toggles::Press::Key(svet_app::KeyCode::KeyU, None)
    );
    assert_eq!(
        crate::toggles::press(key, &mut m, 0.0),
        "mirror pack inline"
    );
    assert_eq!(
        crate::toggles::press(key, &mut m, 0.0),
        "mirror pack workers"
    );
}
