//! The model viewer's and the entities library's flags (`crate::viewer`,
//! `crate::entities::library`): `--model`, `--clip`, `--library-dir` and `--library`.

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
            &["--library-dir"],
            "DIR",
            "the entities library: every DIR/<name>/<name>.rig is an entry (the fly-through's \
             assets/entities)",
            |p: &mut Parsing, v| {
                p.args.library_dir = Some(v.text.into());
                Ok(())
            },
        ),
    ]
}

/// `--library`'s row: print the library's entries and exit, beside `--keys`.
pub(super) fn library_flag() -> Flag<Parsing> {
    Flag::switch(
        &["--library"],
        "print the entities library (each entry's parts, size, pool bytes and clips, or its \
         error), and exit",
        |p: &mut Parsing| p.args.library = true,
    )
}
