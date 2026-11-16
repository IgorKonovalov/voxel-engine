//! The model viewer's and the assets' flags: `--model` and `--clip` (`crate::viewer`), and
//! `--materials` (`crate::materials`).

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
        Flag::value(
            &["--materials"],
            "PATH",
            "the material table file (assets/materials.txt): each scene takes its rows by \
             name; a saved change of colour, roughness, opacity or emission shows within a \
             second, a row added or a class changed waits for a restart",
            |p: &mut Parsing, v| {
                p.args.materials = Some(v.text.into());
                Ok(())
            },
        ),
    ]
}
