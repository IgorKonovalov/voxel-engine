//! Voxel world: storage, materials, edit operations, spatial queries and streaming.
//!
//! This crate is headless and deterministic. It links no GPU API, reads no wall clock and
//! owns no unseeded randomness. The same edit operation applied to the same world produces
//! identical voxels and identical edit results on every platform (CLAUDE.md, ADR-0001).
//!
//! The first real contents land with Plan 0001 (`docs/plans/`).
