# Architecture assets

Reusable buildings and structures as MagicaVoxel `.vox` files, for testbeds, showcases and
stamping tests. Nothing here is engine code; a consumer loads a file through `vxe-assets`.

The `.vox` files are not committed (they are tens of MB each). Regenerate them:

- `castle.vox`: the generated gothic castle and dungeon of Plan 0042,
  `cargo run --release -p flythrough -- --castle --castle-out assets/architecture/castle.vox`.
- `castle-phase1.vox`: the same castle as Plan 0042 Phase 1 produced it, kept locally for
  comparison only; it is not regenerable from the current tree.
