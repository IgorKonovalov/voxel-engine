#!/usr/bin/env node
// Run a nextest tier, or clippy, narrowed to the crates a change touched and their dependents.
//
// The per-phase gate (dev skill, lane rules): a phase that edits vxe-sim should not wait for
// the audio or the net tests, nor build them. This script maps the changed files to workspace
// packages, closes the set over the workspace packages that depend on them (normal, dev, build
// and optional dependencies alike), and runs
// `cargo nextest run -p <each> --profile <tier> -E 'rdeps(a) | rdeps(b)'`. The `-p` flags keep
// cargo from building any other package's test targets (`wasmtime` and the guests among them);
// the `rdeps` filter and the tier's own default-filter still pick the tests (quick keeps no GPU
// test, gpu no CPU test). The soak tests are `#[ignore = "soak"]`, which no profile can select,
// so `--profile soak` adds `--run-ignored only` and `--profile full` `--run-ignored all`,
// unless the arguments after `--` already name one.
//
// `--clippy` runs `cargo clippy -p <each> --all-targets -- -D warnings` over the same package
// set instead of a test tier. A lint that fires only in a package outside the set is the tip
// gate's and CI's to catch: both keep `--workspace`.
//
// A narrowed set unifies features over fewer packages than `--workspace` does, so its first
// build may compile a dependency once more with a smaller feature set; cargo keeps both.
//
// Which change: the working tree against HEAD (unstaged, staged and untracked files), the
// unit a dev agent gates before its phase commit. With no working-tree change the last commit
// (HEAD~1..HEAD) is taken instead. `--since <rev>` adds the commits since the merge base with
// <rev> (`--since main` is a lane's whole branch, for its tip gate).
//
// Which packages: a file under a package's directory selects that package. A file that changes
// what every crate builds or runs (the root Cargo.toml, Cargo.lock, the toolchain, the nextest
// config, the cargo, clippy, rustfmt and deny configs) selects every package, `--workspace`.
// Files under docs/, scripts/, .claude/, .github/ and markdown anywhere select nothing.
// Anything else outside a package (root assets/, for one) selects every package: the safe
// reading.
//
// Which gpu binaries: `apps/flythrough` depends on every crate, so `rdeps` of any render change
// selects every flythrough gpu binary. `--binaries a,b` (gpu tier only) replaces the package
// filter with the named `gpu_*` binaries, `binary(=a) | binary(=b)`, plus every binary whose
// own `tests/gpu_*.rs` file the change touched, and builds only the packages that own them. A
// name two packages share (`gpu_exposure`) selects both when named; a touched file selects its
// own package's by `binary_id`. Shared support files under `tests/support/` join no binary.
// The whole gpu tier runs at a plan's tip. The soak line the tiers exclude is docs/nfr.md
// section 7's: 10 s on the dev reference's lavapipe, 60 s on CI.
//
// Usage:  node scripts/test-changed.mjs [--profile quick|gpu|gate|soak|full] [--since <rev>]
//                                       [--binaries gpu_a,gpu_b] [--clippy] [--dry-run]
//                                       [-- <more nextest or clippy arguments>]
// Exit: nextest's or clippy's exit code; 0 with a message when nothing changed that a crate
//       covers; 2 on a usage error (an unknown argument, a flag missing its value,
//       `--binaries` outside the gpu tier or with `--clippy`, or a name that is no gpu test
//       binary of the workspace).
// Tests: `node --test scripts/test-changed.test.mjs`.

import { execFileSync, spawnSync } from "node:child_process";
import { dirname, resolve, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

export const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");

const EVERYTHING = /^(Cargo\.toml|Cargo\.lock|rust-toolchain\.toml|clippy\.toml|rustfmt\.toml|deny\.toml|\.config\/nextest\.toml|\.cargo\/)/;
const NOTHING = /^(docs|scripts|\.claude|\.github)\/|\.md$|^(LICENSE|README)/;

// `cargo metadata --no-deps`: the workspace's packages with their declared dependencies, which
// name a path dependency on another member by its package name.
export function readMetadata(root = ROOT) {
  return JSON.parse(
    execFileSync("cargo", ["metadata", "--no-deps", "--format-version", "1"], {
      cwd: root,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
      maxBuffer: 64 * 1024 * 1024,
    }),
  );
}

// Workspace packages, longest directory first so a nested package (apps/flythrough/logic)
// wins over its parent.
function packageDirs(metadata, root) {
  return metadata.packages
    .map((p) => ({ name: p.name, dir: relative(root, dirname(p.manifest_path)).split(sep).join("/") }))
    .sort((a, b) => b.dir.length - a.dir.length);
}

// The changed files mapped to packages: `everything` when one of them reaches every crate.
export function selectPackages(metadata, files, root = ROOT) {
  const packages = packageDirs(metadata, root);
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
  return { everything, selected, ignored };
}

// The named packages and every workspace package that depends on one of them, directly or
// through another member, by any dependency kind. Sorted by name.
export function withDependents(metadata, names) {
  const members = new Set(metadata.packages.map((p) => p.name));
  const dependents = new Map([...members].map((m) => [m, new Set()]));
  for (const p of metadata.packages) {
    for (const d of p.dependencies) {
      if (members.has(d.name) && d.name !== p.name) dependents.get(d.name).add(p.name);
    }
  }
  const closed = new Set(names);
  const queue = [...names];
  while (queue.length) {
    for (const up of dependents.get(queue.pop()) ?? []) {
      if (!closed.has(up)) {
        closed.add(up);
        queue.push(up);
      }
    }
  }
  return [...closed].sort();
}

// The workspace's gpu test binaries: every integration test target named `gpu_*`, keyed by
// its source file.
export function gpuTargets(metadata, root = ROOT) {
  return metadata.packages.flatMap((p) =>
    p.targets
      .filter((t) => t.kind.includes("test") && t.name.startsWith("gpu_"))
      .map((t) => ({
        name: t.name,
        pkg: p.name,
        id: `${p.name}::${t.name}`,
        src: relative(root, t.src_path).split(sep).join("/"),
      })),
  );
}

// The cargo arguments for a change. Returns { args, notes } or { args: null, notes } when
// nothing changed that a crate covers. `files` is a Set of repo-relative paths.
export function buildCommand({ metadata, files, profile = "quick", binaries, clippy = false, passthrough = [], root = ROOT }) {
  const notes = [];
  const { everything, selected } = selectPackages(metadata, files, root);

  let binaryExpr;
  let binaryPackages;
  if (binaries) {
    const targets = gpuTargets(metadata, root);
    const terms = new Set(binaries.map((b) => `binary(=${b})`));
    const owners = new Set(targets.filter((t) => binaries.includes(t.name)).map((t) => t.pkg));
    const touched = targets.filter((t) => files.has(t.src) && !binaries.includes(t.name));
    for (const t of touched) {
      const shared = targets.filter((u) => u.name === t.name).length > 1;
      terms.add(shared ? `binary_id(=${t.id})` : `binary(=${t.name})`);
      owners.add(t.pkg);
    }
    binaryExpr = [...terms].join(" | ");
    binaryPackages = [...owners].sort();
    const joined = touched.length ? `; joined by the change: ${touched.map((t) => t.id).join(", ")}` : "";
    notes.push(`binaries ${binaries.join(", ")}${joined}`);
  }

  if (!binaryExpr && !everything && selected.size === 0) {
    notes.push("no crate changed, nothing to run");
    return { args: null, notes };
  }

  let packageArgs;
  if (binaryPackages) {
    packageArgs = binaryPackages.flatMap((p) => ["-p", p]);
  } else if (everything) {
    packageArgs = ["--workspace"];
    notes.push("a change outside the packages selects every package");
  } else {
    const set = withDependents(metadata, [...selected]);
    packageArgs = set.flatMap((p) => ["-p", p]);
    notes.push(`packages ${[...selected].sort().join(", ")}; with their dependents ${set.join(", ")}`);
  }

  if (clippy) {
    return { args: ["clippy", ...packageArgs, "--all-targets", ...passthrough, "--", "-D", "warnings"], notes };
  }

  const args = ["nextest", "run", ...packageArgs, "--profile", profile];
  if (binaryExpr) {
    notes.push(`the named gpu binaries -> -E '${binaryExpr}'`);
    args.push("-E", binaryExpr);
  } else if (!everything) {
    const expr = [...selected].sort().map((p) => `rdeps(${p})`).join(" | ");
    notes.push(`tests -> -E '${expr}'`);
    args.push("-E", expr);
  }
  const runIgnored = { soak: "only", full: "all" }[profile];
  if (runIgnored && !passthrough.some((a) => a.startsWith("--run-ignored"))) {
    args.push("--run-ignored", runIgnored);
  }
  args.push(...passthrough);
  return { args, notes };
}

function main(argv) {
  let profile = "quick";
  let since;
  let dryRun = false;
  let clippy = false;
  let binaries;
  const passthrough = [];
  // The value after a flag; a flag that ends the arguments is a usage error.
  const value = (i) => {
    if (i + 1 >= argv.length) {
      console.error(`test-changed: ${argv[i]} needs a value`);
      process.exit(2);
    }
    return argv[i + 1];
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--") {
      passthrough.push(...argv.slice(i + 1));
      break;
    } else if (a === "--profile") profile = value(i++);
    else if (a.startsWith("--profile=")) profile = a.slice("--profile=".length);
    else if (a === "--since") since = value(i++);
    else if (a.startsWith("--since=")) since = a.slice("--since=".length);
    else if (a === "--binaries") binaries = value(i++);
    else if (a.startsWith("--binaries=")) binaries = a.slice("--binaries=".length);
    else if (a === "--clippy") clippy = true;
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
    if (profile !== "gpu" || clippy) {
      console.error(`test-changed: --binaries narrows the gpu tier only, not ${clippy ? "--clippy" : `--profile ${profile}`}`);
      process.exit(2);
    }
    binaries = (binaries ?? "").split(",").map((b) => b.trim()).filter(Boolean);
    if (binaries.length === 0) {
      console.error("test-changed: --binaries needs a comma-separated list of gpu_* binary names");
      process.exit(2);
    }
  }

  const git = (...gitArgs) =>
    execFileSync("git", gitArgs, { cwd: ROOT, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] })
      .split("\n")
      .filter(Boolean);

  const metadata = readMetadata();
  if (binaries) {
    const targets = gpuTargets(metadata);
    const unknown = binaries.filter((b) => !targets.some((t) => t.name === b));
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
  console.error(`test-changed: ${files.size} changed file(s), ${described}`);

  const { args, notes } = buildCommand({ metadata, files, profile, binaries, clippy, passthrough });
  for (const n of notes) console.error(`test-changed: ${n}`);
  if (!args) process.exit(0);
  console.error(`test-changed: cargo ${args.join(" ")}`);
  if (dryRun) process.exit(0);
  const run = spawnSync("cargo", args, { cwd: ROOT, stdio: "inherit" });
  process.exit(run.status ?? 1);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2));
}
