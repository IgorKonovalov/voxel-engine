#!/usr/bin/env node
// Run a nextest tier narrowed to the crates a change touched, and their dependents.
//
// The per-phase gate (dev skill, lane rules): a phase that edits vxe-sim should not wait for
// the audio or the net tests. This script maps the changed files to workspace packages and
// runs `cargo nextest run --workspace --profile <tier> -E 'rdeps(a) | rdeps(b)'`, which the
// tier's own default-filter still narrows (quick keeps no GPU test, gpu no CPU test). The soak
// tests are `#[ignore = "soak"]`, which no profile can select, so `--profile soak` adds
// `--run-ignored only` and `--profile full` `--run-ignored all`, unless the arguments after
// `--` already name one.
//
// Which change: the working tree against HEAD (unstaged, staged and untracked files), the
// unit a dev agent gates before its phase commit. With no working-tree change the last commit
// (HEAD~1..HEAD) is taken instead. `--since <rev>` adds the commits since the merge base with
// <rev> (`--since main` is a lane's whole branch, for its tip gate).
//
// Which tests: a file under a package's directory selects `rdeps(<package>)`. A file that
// changes what every crate builds or runs (the root Cargo.toml, Cargo.lock, the toolchain,
// the nextest config, the cargo, clippy, rustfmt and deny configs) selects every test. Files
// under docs/, scripts/, .claude/, .github/ and markdown anywhere select nothing. Anything
// else outside a package (root assets/, for one) selects every test: the safe reading.
//
// Which gpu binaries: `apps/flythrough` depends on every crate, so `rdeps` of any render change
// selects every flythrough gpu binary. `--binaries a,b` (gpu tier only) replaces the package
// filter with the named `gpu_*` binaries, `binary(=a) | binary(=b)`, plus every binary whose
// own `tests/gpu_*.rs` file the change touched. A name two packages share (`gpu_exposure`)
// selects both when named; a touched file selects its own package's by `binary_id`. Shared
// support files under `tests/support/` join no binary. The whole gpu tier runs at a plan's tip.
// The soak line the tiers exclude is docs/nfr.md section 7's: 10 s on the dev reference's
// lavapipe, 60 s on CI.
//
// Usage:  node scripts/test-changed.mjs [--profile quick|gpu|gate|soak|full] [--since <rev>]
//                                       [--binaries gpu_a,gpu_b] [--dry-run]
//                                       [-- <more nextest arguments>]
// Exit: nextest's exit code; 0 with a message when nothing changed that a test covers;
//       2 on a usage error (an unknown argument, `--binaries` outside the gpu tier, or a name
//       that is no gpu test binary of the workspace).

import { execFileSync, spawnSync } from "node:child_process";
import { dirname, resolve, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");

const args = process.argv.slice(2);
let profile = "quick";
let since;
let dryRun = false;
let binaries;
const passthrough = [];
for (let i = 0; i < args.length; i++) {
  const a = args[i];
  if (a === "--") {
    passthrough.push(...args.slice(i + 1));
    break;
  } else if (a === "--profile") profile = args[++i];
  else if (a.startsWith("--profile=")) profile = a.slice("--profile=".length);
  else if (a === "--since") since = args[++i];
  else if (a.startsWith("--since=")) since = a.slice("--since=".length);
  else if (a === "--binaries") binaries = args[++i];
  else if (a.startsWith("--binaries=")) binaries = a.slice("--binaries=".length);
  else if (a === "--dry-run") dryRun = true;
  else {
    console.error(`test-changed: unknown argument ${a}`);
    process.exit(2);
  }
}
if (!profile) {
  console.error("test-changed: --profile needs a tier name");
  process.exit(2);
}
if (binaries !== undefined) {
  if (profile !== "gpu") {
    console.error(`test-changed: --binaries narrows the gpu tier only, not --profile ${profile}`);
    process.exit(2);
  }
  binaries = (binaries ?? "").split(",").map((b) => b.trim()).filter(Boolean);
  if (binaries.length === 0) {
    console.error("test-changed: --binaries needs a comma-separated list of gpu_* binary names");
    process.exit(2);
  }
}

const git = (...argv) =>
  execFileSync("git", argv, { cwd: ROOT, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] })
    .split("\n")
    .filter(Boolean);

// Workspace packages, longest directory first so a nested package (apps/flythrough/logic)
// wins over its parent.
const metadata = JSON.parse(
  execFileSync("cargo", ["metadata", "--no-deps", "--format-version", "1"], {
    cwd: ROOT,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    maxBuffer: 64 * 1024 * 1024,
  }),
);
const packages = metadata.packages
  .map((p) => ({ name: p.name, dir: relative(ROOT, dirname(p.manifest_path)).split(sep).join("/") }))
  .sort((a, b) => b.dir.length - a.dir.length);

// The workspace's gpu test binaries: every integration test target named `gpu_*`, keyed by
// its source file.
const gpuTargets = metadata.packages.flatMap((p) =>
  p.targets
    .filter((t) => t.kind.includes("test") && t.name.startsWith("gpu_"))
    .map((t) => ({
      name: t.name,
      id: `${p.name}::${t.name}`,
      src: relative(ROOT, t.src_path).split(sep).join("/"),
    })),
);
if (binaries) {
  const unknown = binaries.filter((b) => !gpuTargets.some((t) => t.name === b));
  if (unknown.length) {
    console.error(`test-changed: --binaries names no gpu test binary of the workspace: ${unknown.join(", ")}`);
    process.exit(2);
  }
}

// The changed files.
let files = new Set([
  ...git("diff", "--name-only", "HEAD"),
  ...git("ls-files", "--others", "--exclude-standard"),
]);
let described = "the working tree against HEAD";
if (since) {
  const base = git("merge-base", since, "HEAD")[0];
  for (const f of git("diff", "--name-only", base, "HEAD")) files.add(f);
  described += ` and the commits since ${base.slice(0, 7)} (merge base with ${since})`;
}
if (files.size === 0) {
  files = new Set(git("diff", "--name-only", "HEAD~1", "HEAD"));
  described = "the last commit (the working tree is clean)";
}

const EVERYTHING = /^(Cargo\.toml|Cargo\.lock|rust-toolchain\.toml|clippy\.toml|rustfmt\.toml|deny\.toml|\.config\/nextest\.toml|\.cargo\/)/;
const NOTHING = /^(docs|scripts|\.claude|\.github)\/|\.md$|^(LICENSE|README)/;

let everything = false;
const selected = new Set();
const ignored = [];
for (const f of files) {
  if (EVERYTHING.test(f)) {
    everything = true;
    continue;
  }
  const pkg = packages.find((p) => f === p.dir || f.startsWith(p.dir + "/"));
  if (pkg) selected.add(pkg.name);
  else if (NOTHING.test(f)) ignored.push(f);
  else everything = true;
}

console.error(`test-changed: ${files.size} changed file(s), ${described}`);

// The gpu binaries the phase names, and those whose own file the change touched.
let binaryExpr;
if (binaries) {
  const terms = new Set(binaries.map((b) => `binary(=${b})`));
  const touched = gpuTargets.filter((t) => files.has(t.src) && !binaries.includes(t.name));
  for (const t of touched) {
    const shared = gpuTargets.filter((u) => u.name === t.name).length > 1;
    terms.add(shared ? `binary_id(=${t.id})` : `binary(=${t.name})`);
  }
  binaryExpr = [...terms].join(" | ");
  const joined = touched.length ? `; joined by the change: ${touched.map((t) => t.id).join(", ")}` : "";
  console.error(`test-changed: binaries ${binaries.join(", ")}${joined}`);
}

if (!binaryExpr && !everything && selected.size === 0) {
  console.error("test-changed: no crate changed, nothing to run");
  process.exit(0);
}

const nextest = ["nextest", "run", "--workspace", "--profile", profile];
if (binaryExpr) {
  console.error(`test-changed: the named gpu binaries -> -E '${binaryExpr}'`);
  nextest.push("-E", binaryExpr);
} else if (everything) {
  console.error("test-changed: a change outside the packages selects every test in the tier");
} else {
  const expr = [...selected].sort().map((p) => `rdeps(${p})`).join(" | ");
  console.error(`test-changed: packages ${[...selected].sort().join(", ")} -> -E '${expr}'`);
  nextest.push("-E", expr);
}
const runIgnored = { soak: "only", full: "all" }[profile];
if (runIgnored && !passthrough.some((a) => a.startsWith("--run-ignored"))) {
  nextest.push("--run-ignored", runIgnored);
}
nextest.push(...passthrough);

console.error(`test-changed: cargo ${nextest.join(" ")}`);
if (dryRun) process.exit(0);
const run = spawnSync("cargo", nextest, { cwd: ROOT, stdio: "inherit" });
process.exit(run.status ?? 1);
