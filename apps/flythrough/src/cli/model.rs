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
        Flag::value(
            &["--vox"],
            "PATH[@x,y,z][*k]",
            "place a MagicaVoxel scene in the streamed cave at the start: its palette as rows \
             (glass, glow, metal), hidden layers left out, mirrored groups mirrored, each voxel \
             k^3 world voxels (1 to 4); its box's minimum corner at x,y,z, or 3 m ahead of the \
             spawn (library: apps/flythrough/assets/structures/)",
            |p: &mut Parsing, v| {
                p.args.vox =
                    Some(crate::structures::VoxArg::parse(v.text).map_err(|e| v.takes(&e))?);
                Ok(())
            },
        ),
        Flag::value(
            &["--vox-clear"],
            "on|off",
            "carve --vox's box (and 1 m on its sides and above) to air before placing it [on]",
            |p: &mut Parsing, v| {
                p.args.vox_clear = v.on_off()?;
                Ok(())
            },
        ),
        Flag::switch(
            &["--vox-op"],
            "place --vox as logged ops (a box carve, then a stamp per placement) on the first \
             tick its box is resident, instead of through the cave's source",
            |p: &mut Parsing| p.args.vox_op = true,
        ),
    ]
}
