#!/usr/bin/env node
// Assert that the gpu filter in .config/nextest.toml covers every test file that reaches a
// GPU device, and names nothing else.
//
// The test tiers (Plan 0043) split the suite into `quick` (CPU only, no perf lock), `gpu` and
// a slow list. `quick` is `not (GPU) and not (SLOW)`, so a GPU test file the gpu filter does
// not match lands in the quick tier: it runs on lavapipe without the perf lock, on every dev
// phase of every lane. Nothing in nextest notices. This gate does.
//
// A test file is GPU when it, or a `support` module it declares (`mod support;`), names a
// device entry point:
//
//   software_device    the tests' shared helper that opens the software adapter
//   wgpu::Instance     opening an instance directly
//   request_adapter    asking an instance for an adapter
//   fn device(         a crate-local `device()` helper, or a call to one: `device()`
//   --headless         spawning a headless render of an app binary (it opens a device)
//
// The gpu filter is read from its one canonical copy, the filter of the
// `[[profile.default.overrides]]` entry whose `test-group` is `lavapipe`. Its entries are
// `binary_id(<package>::<test>)` for one integration-test binary and
// `(package(<package>) - kind(lib))` for every integration-test binary of a package. The
// `quick` profile's default-filter must start with `not (<gpu>)` and the `gpu` profile's with
// `(<gpu>)`, verbatim, and both must end with `and not (<slow>)`, where <slow> is another
// default-profile override's filter, the same in both. A list edited in one place and not the
// others is a finding too.
//
// Usage:  node scripts/check-test-tiers.mjs [root]
// Exit 0 = the filter and the GPU files agree. Exit 1 = one line per finding:
//   <file> -> not in the gpu filter: add `binary_id(<id>)` to the lavapipe override's filter
//             and to the quick and gpu default-filters in .config/nextest.toml
//   <entry> -> matches no test file: ...
// The optional `root` scans another tree; scripts/fixtures/test-tiers is the seeded one, and
// scripts/fixtures/README.md names its expected findings. Fixture trees are skipped by path
// on an ordinary run.

import { readFileSync, existsSync } from "node:fs";
import { dirname, resolve, join, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { execFileSync } from "node:child_process";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const ROOT = resolve(process.argv[2] ?? REPO_ROOT);
const SEEDED = resolve(REPO_ROOT, "scripts", "fixtures");
const CONFIG = ".config/nextest.toml";

const ENTRY_POINTS = [
  /\bsoftware_device\b/,
  /\bwgpu::Instance\b/,
  /\brequest_adapter\b/,
  /\bfn device\s*\(|\bdevice\(\)/,
  /"--headless"/,
];

const findings = [];

/** Tracked files under the root, relative to it, skipping seeded trees that are not the root. */
function trackedFiles() {
  const out = execFileSync("git", ["ls-files", "-z"], {
    cwd: ROOT,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "ignore"],
    maxBuffer: 64 * 1024 * 1024,
  });
  const inSeeded = (abs) =>
    !(ROOT === SEEDED || ROOT.startsWith(SEEDED + sep)) && abs.startsWith(SEEDED + sep);
  return out.split("\0").filter((p) => p && !inSeeded(resolve(ROOT, p)));
}

/** Rust source with line and block comments removed, so a doc comment naming a device is not a use. */
function code(path) {
  return readFileSync(join(ROOT, path), "utf8")
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/\/\/.*$/gm, "");
}

const reachesDevice = (src) => ENTRY_POINTS.some((re) => re.test(src));

/** The package name in a Cargo.toml, and its `[[test]]` path -> name map. */
function manifest(dir) {
  const text = readFileSync(join(ROOT, dir, "Cargo.toml"), "utf8");
  const name = /^\[package\][\s\S]*?^name\s*=\s*"([^"]+)"/m.exec(text)?.[1];
  const tests = new Map();
  for (const block of text.split(/^\[\[test\]\]\s*$/m).slice(1)) {
    const body = block.split(/^\[/m)[0];
    const tname = /^name\s*=\s*"([^"]+)"/m.exec(body)?.[1];
    const tpath = /^path\s*=\s*"([^"]+)"/m.exec(body)?.[1] ?? `tests/${tname}.rs`;
    if (tname) tests.set(tpath, tname);
  }
  return { name, tests };
}

/** The gpu filter (the `lavapipe` override's), every default-profile override filter, and each profile's default-filter. */
function configFilters(text) {
  const blocks = text.split(/^(?=\[)/m);
  let gpu;
  const overrides = [];
  const defaults = new Map();
  let profile;
  for (const block of blocks) {
    const header = /^\[\[?([^\]]+)\]\]?/.exec(block)?.[1];
    if (header === "profile.default.overrides" && /test-group\s*=\s*'lavapipe'|test-group\s*=\s*"lavapipe"/.test(block)) {
      gpu = /^filter\s*=\s*'([^']*)'/m.exec(block)?.[1];
    }
    if (header === "profile.default.overrides") {
      const f = /^filter\s*=\s*'([^']*)'/m.exec(block)?.[1];
      if (f !== undefined) overrides.push(f);
    }
    const prof = /^profile\.([^.]+)$/.exec(header ?? "")?.[1];
    if (prof) profile = prof;
    const df = /^default-filter\s*=\s*'([^']*)'/m.exec(block)?.[1];
    if (prof && df) defaults.set(profile, df);
  }
  return { gpu, defaults, overrides };
}

if (!existsSync(join(ROOT, CONFIG))) {
  console.error(`${CONFIG} -> missing under ${ROOT}`);
  process.exit(1);
}
const { gpu, defaults, overrides: overrideFilters } = configFilters(readFileSync(join(ROOT, CONFIG), "utf8"));
if (!gpu) {
  console.error(`${CONFIG} -> no [[profile.default.overrides]] with test-group = 'lavapipe'`);
  process.exit(1);
}

// The filter's entries: one binary, or a package's integration tests.
const binaries = new Set([...gpu.matchAll(/binary_id\(([^)]+)\)/g)].map((m) => m[1]));
const packages = new Set(
  [...gpu.matchAll(/\(package\(([^)]+)\)\s*-\s*kind\(lib\)\)/g)].map((m) => m[1]),
);
const leftover = gpu
  .replace(/binary_id\([^)]+\)/g, "")
  .replace(/\(package\([^)]+\)\s*-\s*kind\(lib\)\)/g, "")
  .replace(/[\s|]/g, "");
if (leftover) {
  findings.push(`${CONFIG} -> the gpu filter has a term this gate cannot read: ${leftover}`);
}

// The copies in the quick and gpu profiles: `not (<gpu>) and not (<slow>)` and
// `(<gpu>) and not (<slow>)`, the slow list being another default-profile override's filter.
const copies = [
  ["quick", `not (${gpu})`, "not (<gpu>)"],
  ["gpu", `(${gpu})`, "(<gpu>)"],
];
const tails = [];
for (const [profile, prefix, shown] of copies) {
  const df = defaults.get(profile);
  if (!df || !df.startsWith(prefix)) {
    findings.push(
      `${CONFIG} -> [profile.${profile}] default-filter does not start with ${shown}: copy the lavapipe override's filter into it verbatim`,
    );
    continue;
  }
  const slow = /^ and not \((.*)\)$/.exec(df.slice(prefix.length))?.[1];
  if (slow === undefined || !overrideFilters.includes(slow)) {
    findings.push(
      `${CONFIG} -> [profile.${profile}] default-filter does not end with \`and not (<slow>)\`, the slow override's filter copied verbatim`,
    );
  } else tails.push(slow);
}
if (tails.length === 2 && tails[0] !== tails[1]) {
  findings.push(`${CONFIG} -> the quick and gpu profiles exclude different slow lists`);
}

// Every integration-test file: tests/*.rs of a crate or app.
const files = trackedFiles();
const testFiles = files.filter((p) => /^(crates|apps)\/[^/]+\/tests\/[^/]+\.rs$/.test(p));
const seenBinaries = new Set();
const seenPackages = new Set();
const manifests = new Map();
for (const file of testFiles.sort()) {
  const dir = file.split("/").slice(0, 2).join("/");
  if (!manifests.has(dir)) manifests.set(dir, manifest(dir));
  const { name: pkg, tests } = manifests.get(dir);
  const rel = file.slice(dir.length + 1);
  const bin = tests.get(rel) ?? rel.slice("tests/".length, -".rs".length);
  const id = `${pkg}::${bin}`;
  if (packages.has(pkg)) seenPackages.add(pkg);
  if (binaries.has(id)) seenBinaries.add(id);

  const src = code(file);
  let gpuFile = reachesDevice(src);
  if (!gpuFile && /^\s*(pub\s+)?mod\s+support\s*;/m.test(src)) {
    for (const s of [`${dir}/tests/support/mod.rs`, `${dir}/tests/support.rs`]) {
      if (files.includes(s) && reachesDevice(code(s))) gpuFile = true;
    }
  }
  if (gpuFile && !packages.has(pkg) && !binaries.has(id)) {
    findings.push(
      `${file} -> not in the gpu filter: add \`binary_id(${id})\` to the lavapipe override's filter and to the quick and gpu default-filters in ${CONFIG}`,
    );
  }
}
for (const id of [...binaries].sort()) {
  if (!seenBinaries.has(id)) {
    findings.push(`binary_id(${id}) -> matches no test file: remove it from the gpu filter in ${CONFIG}`);
  }
}
for (const pkg of [...packages].sort()) {
  if (!seenPackages.has(pkg)) {
    findings.push(`package(${pkg}) -> matches no test file: remove it from the gpu filter in ${CONFIG}`);
  }
}

if (findings.length) {
  for (const f of findings) console.error(f);
  console.error(`${findings.length} finding(s) across ${testFiles.length} test files`);
  process.exit(1);
}
console.log(
  `test tiers: ${testFiles.length} test files, gpu filter ${binaries.size} binaries + ${packages.size} package(s), all agree`,
);
