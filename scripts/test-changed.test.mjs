// Tests for scripts/test-changed.mjs's package resolution, against the workspace's own
// `cargo metadata --no-deps` (read once; no build). Run: node --test scripts/test-changed.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { buildCommand, readMetadata, soleCommand, soleTests, withDependents } from "./test-changed.mjs";

const metadata = readMetadata();

const pFlags = (args) => args.flatMap((a, i) => (args[i - 1] === "-p" ? [a] : []));

test("a_sim_change_builds_sim_and_its_dependents_only", () => {
  const files = new Set(["crates/svet-sim/src/body.rs"]);
  assert.deepEqual(withDependents(metadata, ["svet-sim"]), ["flythrough", "svet-sim", "vxe-tester"]);

  const tests = buildCommand({ metadata, files, profile: "quick" }).args;
  assert.deepEqual(pFlags(tests), ["flythrough", "svet-sim", "vxe-tester"]);
  assert.equal(tests.filter((a) => a === "-p").length, 3);
  assert.ok(!tests.includes("--workspace"));
  assert.deepEqual(tests.slice(tests.indexOf("-E"), tests.indexOf("-E") + 2), ["-E", "rdeps(svet-sim)"]);

  const lint = buildCommand({ metadata, files, clippy: true }).args;
  assert.deepEqual(lint, [
    "clippy", "-p", "flythrough", "-p", "svet-sim", "-p", "vxe-tester", "--all-targets", "--", "-D", "warnings",
  ]);
});

test("a_root_manifest_change_selects_the_workspace", () => {
  const files = new Set(["Cargo.toml", "crates/svet-sim/src/body.rs"]);
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
  const files = new Set(["crates/svet-render/src/lib.rs"]);
  const args = buildCommand({ metadata, files, profile: "gpu", binaries: ["gpu_exposure"] }).args;
  // `gpu_exposure` is a binary name two packages share; both are built.
  assert.deepEqual(pFlags(args), ["flythrough", "svet-render"]);
  assert.deepEqual(args.slice(args.indexOf("-E"), args.indexOf("-E") + 2), ["-E", "binary(=gpu_exposure)"]);
});

const sole = [
  { pkg: "flythrough", name: "a_gpu_one", gpu: true },
  { pkg: "flythrough", name: "a_cpu_one", gpu: false },
  { pkg: "svet-sim", name: "a_sim_one", gpu: false },
  { pkg: "svet-audio", name: "an_audio_one", gpu: false },
];
const ids = (r) => r?.tests.map((t) => `${t.pkg}::${t.name}`) ?? null;

test("a_change_runs_the_sole_tests_its_tier_keeps_in_reach", () => {
  const files = new Set(["crates/svet-sim/src/body.rs"]);
  // svet-sim reaches flythrough, not svet-audio.
  assert.deepEqual(ids(soleCommand({ metadata, files, profile: "quick", sole })), ["flythrough::a_cpu_one", "svet-sim::a_sim_one"]);
  assert.deepEqual(ids(soleCommand({ metadata, files, profile: "gpu", sole })), ["flythrough::a_gpu_one"]);
  const gate = soleCommand({ metadata, files, profile: "gate", sole });
  assert.deepEqual(ids(gate), ["flythrough::a_gpu_one", "flythrough::a_cpu_one", "svet-sim::a_sim_one"]);
  assert.deepEqual(pFlags(gate.args), ["flythrough", "svet-sim"]);
  assert.deepEqual(gate.args.slice(gate.args.indexOf("--run-ignored"), gate.args.indexOf("--run-ignored") + 2), ["--run-ignored", "only"]);
  assert.ok(gate.args[gate.args.indexOf("-E") + 1].includes("(package(=svet-sim) & test(/(^|::)a_sim_one$/))"));
  // Soak and full run every ignored test already; a docs change reaches no package.
  assert.equal(soleCommand({ metadata, files, profile: "soak", sole }), null);
  assert.equal(soleCommand({ metadata, files: new Set(["docs/nfr.md"]), profile: "gate", sole }), null);
  // A root change reaches every package.
  assert.equal(soleCommand({ metadata, files: new Set(["Cargo.lock"]), profile: "quick", sole }).tests.length, 3);
});

test("the_workspace_sole_marks_name_a_package_a_test_and_a_mechanism", () => {
  for (const t of soleTests(metadata)) {
    assert.ok(t.pkg && /^[a-z_][a-z0-9_]*$/.test(t.name) && t.mechanism.length > 0, JSON.stringify(t));
  }
});

test("a_world_change_reaches_every_member_that_depends_on_it", () => {
  const set = withDependents(metadata, ["svet-world"]);
  for (const p of ["svet-world", "svet-sim", "svet-render", "svet-net", "svet-script-wasm", "flythrough", "vxe-tester"]) {
    assert.ok(set.includes(p), p);
  }
  assert.ok(!set.includes("svet-app"));
});
