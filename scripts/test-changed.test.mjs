// Tests for scripts/test-changed.mjs's package resolution, against the workspace's own
// `cargo metadata --no-deps` (read once; no build). Run: node --test scripts/test-changed.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { buildCommand, readMetadata, withDependents } from "./test-changed.mjs";

const metadata = readMetadata();

const pFlags = (args) => args.flatMap((a, i) => (args[i - 1] === "-p" ? [a] : []));

test("a_sim_change_builds_sim_and_its_dependents_only", () => {
  const files = new Set(["crates/vxe-sim/src/body.rs"]);
  assert.deepEqual(withDependents(metadata, ["vxe-sim"]), ["flythrough", "vxe-sim", "vxe-tester"]);

  const tests = buildCommand({ metadata, files, profile: "quick" }).args;
  assert.deepEqual(pFlags(tests), ["flythrough", "vxe-sim", "vxe-tester"]);
  assert.equal(tests.filter((a) => a === "-p").length, 3);
  assert.ok(!tests.includes("--workspace"));
  assert.deepEqual(tests.slice(tests.indexOf("-E"), tests.indexOf("-E") + 2), ["-E", "rdeps(vxe-sim)"]);

  const lint = buildCommand({ metadata, files, clippy: true }).args;
  assert.deepEqual(lint, [
    "clippy", "-p", "flythrough", "-p", "vxe-sim", "-p", "vxe-tester", "--all-targets", "--", "-D", "warnings",
  ]);
});

test("a_root_manifest_change_selects_the_workspace", () => {
  const files = new Set(["Cargo.toml", "crates/vxe-sim/src/body.rs"]);
  const tests = buildCommand({ metadata, files, profile: "quick" }).args;
  assert.ok(tests.includes("--workspace"));
  assert.equal(tests.indexOf("-p"), -1);
  assert.equal(tests.indexOf("-E"), -1);
  const lint = buildCommand({ metadata, files, clippy: true }).args;
  assert.deepEqual(lint, ["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]);
});

test("a_docs_only_change_runs_nothing", () => {
  const files = new Set(["docs/nfr.md", "scripts/test-changed.mjs"]);
  assert.equal(buildCommand({ metadata, files, profile: "quick" }).args, null);
  assert.equal(buildCommand({ metadata, files, clippy: true }).args, null);
});

test("named_gpu_binaries_build_their_owning_packages_only", () => {
  const files = new Set(["crates/vxe-render/src/lib.rs"]);
  const args = buildCommand({ metadata, files, profile: "gpu", binaries: ["gpu_exposure"] }).args;
  // `gpu_exposure` is a binary name two packages share; both are built.
  assert.deepEqual(pFlags(args), ["flythrough", "vxe-render"]);
  assert.deepEqual(args.slice(args.indexOf("-E"), args.indexOf("-E") + 2), ["-E", "binary(=gpu_exposure)"]);
});

test("a_world_change_reaches_every_member_that_depends_on_it", () => {
  const set = withDependents(metadata, ["vxe-world"]);
  for (const p of ["vxe-world", "vxe-sim", "vxe-render", "vxe-net", "vxe-script-wasm", "flythrough", "vxe-tester"]) {
    assert.ok(set.includes(p), p);
  }
  assert.ok(!set.includes("vxe-app"));
});
