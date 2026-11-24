//! The WIT package the host serves is `svet:logic@0.1.0`. A component built against another
//! package is refused at load, before any tick, with an error that names the one served.

use std::path::Path;

use svet_net::LogicError;
use svet_script_wasm::{WasmConfig, WasmLogic};

/// The `greedy` guest as `scripts/build-guests.sh` built it against `vxe:logic@0.1.0`, the
/// package under the engine's earlier prefix (ADR-0071). It imports the `host` interface; a
/// component that imports none names no package and links either way.
const PRE_RENAME: &str = "tests/fixtures/pre-rename-greedy.wasm";

#[test]
fn a_component_of_the_old_package_is_refused_naming_svet_logic() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(PRE_RENAME);
    let err = WasmLogic::load(&path, &WasmConfig::default())
        .expect_err("a vxe:logic component must not link against the svet:logic host");
    let LogicError::Load(message) = err else {
        panic!("expected a load error, got {err:?}");
    };
    assert!(
        message.contains("svet:logic/host@0.1.0"),
        "the error names the served package: {message}"
    );
    assert!(
        message.contains("vxe:logic/host@0.1.0"),
        "the error names the component's import: {message}"
    );
}
