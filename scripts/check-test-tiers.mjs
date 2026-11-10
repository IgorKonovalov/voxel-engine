#!/usr/bin/env node
// Assert that every test binary that reaches a GPU device carries the `gpu_` prefix, which the
// tiers in .config/nextest.toml select it by.
//
// The test tiers split the suite into `quick` (CPU only, no perf lock) and `gpu` by one mark,
// the binary's name: `binary(/^gpu_/)` is the gpu tier, `not binary(/^gpu_/)` the quick tier.
// A test file that opens a device without the prefix lands in the quick tier: it runs on
// lavapipe without the perf lock, on every dev phase of every lane. Nothing in nextest notices.
// This gate does.
//
// A test file is GPU when it, or a `support` module it declares (`mod support;`), names a
// device entry point. Two more kinds of test binary are scanned, by their `#[cfg(test)]` items
// alone (their other code runs as the program, not under the test harness): an example a
// manifest marks `test = true`, whose name must carry the prefix too, and a binary crate's
// `src/main.rs` with the modules it declares, whose binary is named after its package and so
// cannot carry it: a device there is always a finding. The entry points:
//
//   software_device    the tests' shared helper that opens the software adapter
//   wgpu::Instance     opening an instance directly
//   request_adapter    asking an instance for an adapter
//   fn device(         a crate-local `device()` helper, or a call to one: `device()`
//   --headless         spawning a headless render of an app binary (it opens a device); in a
//                      binary's or an example's own test items, only beside `Command::new`
//
// A file may carry the prefix without opening a device (a test the gpu tier keeps for another
// reason, such as a shader compile check); only the converse is a finding.
//
// The profiles are read too: the `lavapipe` override's filter must be `binary(/^gpu_/)`, the
// `quick` profile's default-filter `not binary(/^gpu_/) and not (<soak>)` and the `gpu`
// profile's `binary(/^gpu_/) and not (<soak>)`, where <soak> is another default-profile
// override's filter, the same in both; `gate` must be `not (<soak>)` and `soak` `(<soak>)`; a
// `ci-windows` profile, when there is one, must be the `quick` profile's filter.
//
// Usage:  node scripts/check-test-tiers.mjs [root]
// Exit 0 = every device-opening test binary carries the prefix. Exit 1 = one line per finding:
//   <file> -> opens a GPU device but its test binary `<id>` lacks the gpu_ prefix: rename it ...
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
const PREFIX = "gpu_";
const GPU = "binary(/^gpu_/)";

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

/** As `reachesDevice`, for a binary's or an example's own test items: there `--headless` is
 * also an argument its parser is tested on, so it counts only beside a spawned process. */
const itemsReachDevice = (src) =>
  ENTRY_POINTS.slice(0, -1).some((re) => re.test(src)) ||
  (ENTRY_POINTS.at(-1).test(src) && /\bCommand::new\b/.test(src));

/** The `#[cfg(test)]` items of comment-free source `src`: each attribute's item through its
 * matching close brace, or through its `;` for a `mod name;` declaration. */
function testItems(src) {
  const out = [];
  const attr = /#\[cfg\(test\)\]/g;
  let m;
  while ((m = attr.exec(src))) {
    let i = m.index + m[0].length;
    let depth = 0;
    for (; i < src.length; i++) {
      const ch = src[i];
      if (ch === "{") depth++;
      else if (ch === "}" && --depth === 0) break;
      else if (ch === ";" && depth === 0) break;
    }
    out.push(src.slice(m.index, i + 1));
    attr.lastIndex = i + 1;
  }
  return out.join("\n");
}

/** The package name in a Cargo.toml, its `[[test]]` path -> name map and the `[[example]]`
 * paths marked `test = true`, path -> name. */
function manifest(dir) {
  const text = readFileSync(join(ROOT, dir, "Cargo.toml"), "utf8");
  const name = /^\[package\][\s\S]*?^name\s*=\s*"([^"]+)"/m.exec(text)?.[1];
  const targets = (kind, dirName, keep) => {
    const map = new Map();
    for (const block of text.split(new RegExp(`^\\[\\[${kind}\\]\\]\\s*$`, "m")).slice(1)) {
      const body = block.split(/^\[/m)[0];
      const tname = /^name\s*=\s*"([^"]+)"/m.exec(body)?.[1];
      const tpath = /^path\s*=\s*"([^"]+)"/m.exec(body)?.[1] ?? `${dirName}/${tname}.rs`;
      if (tname && keep(body)) map.set(tpath, tname);
    }
    return map;
  };
  const tests = targets("test", "tests", () => true);
  const examples = targets("example", "examples", (body) => /^test\s*=\s*true\s*$/m.test(body));
  return { name, tests, examples };
}

/** The `lavapipe` override's filter, every default-profile override filter, and each profile's default-filter. */
function configFilters(text) {
  let lavapipe;
  const overrides = [];
  const defaults = new Map();
  for (const block of text.split(/^(?=\[)/m)) {
    const header = /^\[\[?([^\]]+)\]\]?/.exec(block)?.[1];
    if (header === "profile.default.overrides") {
      const f = /^filter\s*=\s*'([^']*)'/m.exec(block)?.[1];
      if (f !== undefined) overrides.push(f);
      if (/test-group\s*=\s*['"]lavapipe['"]/.test(block)) lavapipe = f;
    }
    const prof = /^profile\.([^.]+)$/.exec(header ?? "")?.[1];
    const df = /^default-filter\s*=\s*'([^']*)'/m.exec(block)?.[1];
    if (prof && df) defaults.set(prof, df);
  }
  return { lavapipe, defaults, overrides };
}

if (!existsSync(join(ROOT, CONFIG))) {
  console.error(`${CONFIG} -> missing under ${ROOT}`);
  process.exit(1);
}
const { lavapipe, defaults, overrides } = configFilters(readFileSync(join(ROOT, CONFIG), "utf8"));
if (lavapipe !== GPU) {
  findings.push(`${CONFIG} -> the lavapipe override's filter is not '${GPU}'`);
}

// The quick and gpu profiles: the prefix, then `and not (<soak>)`, the soak list being another
// default-profile override's filter; then the gate and soak profiles, and CI's Windows arm.
const tails = [];
for (const [profile, prefix] of [
  ["quick", `not ${GPU}`],
  ["gpu", GPU],
]) {
  const df = defaults.get(profile) ?? "";
  const soak = df.startsWith(prefix) ? /^ and not \((.*)\)$/.exec(df.slice(prefix.length))?.[1] : undefined;
  if (soak === undefined || !overrides.includes(soak)) {
    findings.push(
      `${CONFIG} -> [profile.${profile}] default-filter is not \`${prefix} and not (<soak>)\`, the soak override's filter copied verbatim`,
    );
  } else tails.push(soak);
}
if (tails.length === 2 && tails[0] !== tails[1]) {
  findings.push(`${CONFIG} -> the quick and gpu profiles exclude different soak lists`);
}
if (tails.length === 2 && tails[0] === tails[1]) {
  for (const [profile, expected, shown] of [
    ["gate", `not (${tails[0]})`, "not (<soak>)"],
    ["soak", `(${tails[0]})`, "(<soak>)"],
  ]) {
    if (defaults.get(profile) !== expected) {
      findings.push(
        `${CONFIG} -> [profile.${profile}] default-filter is not ${shown}: copy the soak override's filter into it verbatim`,
      );
    }
  }
}
const win = defaults.get("ci-windows");
if (win !== undefined && win !== defaults.get("quick")) {
  findings.push(`${CONFIG} -> [profile.ci-windows] default-filter is not the quick profile's: copy it verbatim`);
}

// Every integration-test file: tests/*.rs of a crate or app.
const files = trackedFiles();
const testFiles = files.filter((p) => /^(crates|apps)\/[^/]+\/tests\/[^/]+\.rs$/.test(p));
const manifests = new Map();
const manifestOf = (dir) => {
  if (!manifests.has(dir)) manifests.set(dir, manifest(dir));
  return manifests.get(dir);
};
for (const file of testFiles.sort()) {
  const dir = file.split("/").slice(0, 2).join("/");
  const { name: pkg, tests } = manifestOf(dir);
  const rel = file.slice(dir.length + 1);
  const bin = tests.get(rel) ?? rel.slice("tests/".length, -".rs".length);
  if (bin.startsWith(PREFIX)) continue;
  let gpuFile = reachesDevice(code(file));
  if (!gpuFile && /^\s*(pub\s+)?mod\s+support\s*;/m.test(code(file))) {
    for (const s of [`${dir}/tests/support/mod.rs`, `${dir}/tests/support.rs`]) {
      if (files.includes(s) && reachesDevice(code(s))) gpuFile = true;
    }
  }
  if (gpuFile) {
    findings.push(
      `${file} -> opens a GPU device but its test binary \`${pkg}::${bin}\` lacks the ${PREFIX} prefix: rename it to tests/${PREFIX}${bin}.rs`,
    );
  }
}
// Test binaries scanned by their `#[cfg(test)]` items: examples marked `test = true`, and a
// binary crate's `src/main.rs` with the modules it declares.
let units = 0;
for (const file of files.filter((p) => /^(crates|apps)\/[^/]+\/Cargo\.toml$/.test(p)).sort()) {
  const dir = file.split("/").slice(0, 2).join("/");
  const { name: pkg, examples } = manifestOf(dir);
  for (const [rel, ex] of examples) {
    const src = `${dir}/${rel}`;
    if (!files.includes(src)) continue;
    units++;
    if (!ex.startsWith(PREFIX) && itemsReachDevice(testItems(code(src)))) {
      findings.push(
        `${src} -> its tests open a GPU device but the example \`${pkg}::example/${ex}\` lacks the ${PREFIX} prefix: rename it to ${PREFIX}${ex}`,
      );
    }
  }
  const main = `${dir}/src/main.rs`;
  if (!files.includes(main)) continue;
  units++;
  const mods = [...code(main).matchAll(/^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([a-z_][a-z0-9_]*)\s*;/gm)]
    .map((m) => [`${dir}/src/${m[1]}.rs`, `${dir}/src/${m[1]}/mod.rs`].find((f) => files.includes(f)))
    .filter(Boolean);
  for (const src of [main, ...mods]) {
    if (itemsReachDevice(testItems(code(src)))) {
      findings.push(
        `${src} -> its tests open a GPU device inside the binary \`${pkg}::bin/${pkg}\`, which cannot carry the ${PREFIX} prefix: move them to a tests/${PREFIX}*.rs file`,
      );
    }
  }
}

if (findings.length) {
  for (const f of findings) console.error(f);
  console.error(`${findings.length} finding(s) across ${testFiles.length + units} test binaries`);
  process.exit(1);
}
const gpuCount = testFiles.filter((f) => f.split("/").pop().startsWith(PREFIX)).length;
console.log(
  `test tiers: ${testFiles.length} test files (${gpuCount} ${PREFIX}) and ${units} example and binary test modules, every device behind the prefix`,
);
