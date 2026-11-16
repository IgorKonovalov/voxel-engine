//! The model viewer's flags (`crate::viewer`): `--model` and `--clip`.

use super::flags::Parsing;
use super::table::Flag;

/// The viewer's rows.
pub(super) fn model_flags() -> Vec<Flag<Parsing>> {
    vec![
        Flag::value(
            &["--model"],
            "PATH",
            "view a .rig model and its .vox: ` places it, Left/Right cycle its clips (fading \
             between them), Down pauses; a saved change reloads, a refused one shows in red \
             until fixed",
            |p: &mut Parsing, v| {
                p.args.model = Some(v.text.into());
                Ok(())
            },
        ),
        Flag::value(
            &["--clip"],
            "NAME",
            "the --model clip to play first (the rig's first)",
            |p: &mut Parsing, v| {
                p.args.clip = Some(v.string());
                Ok(())
            },
        ),
    ]
}
