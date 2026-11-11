//! The tester build's launcher (ADR-0054): a small window with a menu that starts the release
//! fly-through as a child process per scene, logs each run and packs the report folder, which a
//! tester sends back by hand. Its parts live here so the tests reach them; `main.rs` is the
//! binary `vxe-tester`.

pub mod system;
