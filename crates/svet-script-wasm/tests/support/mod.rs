//! Locating the components `scripts/build-guests.sh` builds.

use std::path::PathBuf;

/// `target/guests/<name>.wasm` under the workspace root. Panics, naming the script, when the
/// component has not been built: a missing component fails the test rather than skipping it.
pub fn guest(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let path = root.join("target/guests").join(format!("{name}.wasm"));
    assert!(
        path.is_file(),
        "{} is missing: run scripts/build-guests.sh first",
        path.display()
    );
    path
}
