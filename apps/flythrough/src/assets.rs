//! Where the fly-through finds its assets folder (the material file, the entities library,
//! the structures). A packaged build reads `assets/` beside its executable: the tester's
//! archive ships the folder there (`scripts/package-tester.sh`). A dev build has no such
//! folder beside it (`target/<profile>/` holds none), so it reads the source tree's
//! `apps/flythrough/assets`, a path fixed at build time, which on another machine names the
//! build machine's checkout. The folder beside the executable wins when it exists; nothing
//! else is probed.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The source tree's assets folder, as the binary was built.
pub const SOURCE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets");

/// The assets folder this run reads ([`root_from`] over the running executable's folder),
/// resolved once.
pub fn root() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let exe = std::env::current_exe().ok();
        root_from(exe.as_deref().and_then(Path::parent), Path::new(SOURCE))
    })
}

/// `exe_dir/assets` when it is a folder, else `source`.
pub fn root_from(exe_dir: Option<&Path>, source: &Path) -> PathBuf {
    exe_dir
        .map(|d| d.join("assets"))
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| source.to_path_buf())
}

/// `rel` under the assets folder this run reads.
pub fn path(rel: &str) -> PathBuf {
    root().join(rel)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An executable's folder with an `assets/` beside it resolves there; one without it, or
    /// no executable folder at all, resolves to the source path; a file named `assets` is not
    /// a folder and is passed over. A dev build's test binary (under `target/`, no `assets/`
    /// beside it) resolves to the source tree.
    #[test]
    fn the_assets_beside_the_executable_win_over_the_source_tree() {
        let tmp = std::env::temp_dir().join(format!("svet-assets-root-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let packed = tmp.join("packed");
        let bare = tmp.join("bare");
        let file = tmp.join("file");
        std::fs::create_dir_all(packed.join("assets")).expect("packed");
        std::fs::create_dir_all(&bare).expect("bare");
        std::fs::create_dir_all(&file).expect("file dir");
        std::fs::write(file.join("assets"), "not a folder").expect("file");
        let source = Path::new("/build/machine/apps/flythrough/assets");
        assert_eq!(root_from(Some(&packed), source), packed.join("assets"));
        assert_eq!(root_from(Some(&bare), source), source);
        assert_eq!(root_from(Some(&file), source), source);
        assert_eq!(root_from(None, source), source);
        assert_eq!(root(), Path::new(SOURCE));
        assert!(path("materials.txt").is_file());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
