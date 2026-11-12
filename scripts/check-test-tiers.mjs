#!/usr/bin/env node
// Assert the marks the test tiers in .config/nextest.toml select by: every test binary that
// reaches a GPU device carries the `gpu_` prefix, and every `#[ignore]` is a soak mark or a
// reading.
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
// Every `#[ignore]` in a tracked `.rs` file under crates/ or apps/ (comments and string
// literals blanked) must be `#[ignore = "soak"]`, a long series only the soak tier runs, or sit
// inside a `mod reading` block, a measurement run by hand that soak and full leave out by
// `test(/(^|::)reading::/)`. Any other ignored test would run in the soak tier unasked.
//
// A file under a crate's or app's `tests/` (its `support/` included) names at most
// ENV_CAP distinct `VXE_*` environment variables, counted as string literals outside comments,
// so a wrapper (`env`, `env_or`, `on_off`, `list`) does not hide one. A reading with more
// switches than that is a tool: an example whose flags print under `--help` (Plan 0045).
//
// The profiles are read too: the `lavapipe` override's filter must be `binary(/^gpu_/)`, the
// `quick` and `ci-windows` profiles' default-filter `not binary(/^gpu_/)`, the `gpu` profile's
// `binary(/^gpu_/)`, `gate` none, and `soak` and `full` `not test(/(^|::)reading::/)`.
//
// Usage:  node scripts/check-test-tiers.mjs [root]
// Exit 0 = every device-opening test binary carries the prefix. Exit 1 = one line per finding:
//   <file> -> opens a GPU device but its test binary `<id>` lacks the gpu_ prefix: rename it ...
//   <file>:<line> -> an #[ignore] that is neither "soak" nor inside `mod reading`: ...
//   <file> -> names N VXE_* environment variables, over the cap of 4: ...
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
const NOT_READING = "not test(/(^|::)reading::/)";
const ENV_CAP = 4;

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

/** Rust source with comments and string and char literals turned to spaces, offsets and
 * newlines kept, so braces and attributes are read from code alone. */
function blanked(text) {
  const out = text.split("");
  const blank = (from, to) => {
    for (let k = from; k < to && k < out.length; k++) if (out[k] !== "\n") out[k] = " ";
  };
  let i = 0;
  while (i < text.length) {
    const two = text.slice(i, i + 2);
    const raw = /^r(#*)"/.exec(text.slice(i, i + 12));
    if (two === "//") {
      const e = text.indexOf("\n", i);
      const end = e < 0 ? text.length : e;
      blank(i, end);
      i = end;
    } else if (two === "/*") {
      const e = text.indexOf("*/", i + 2);
      const end = e < 0 ? text.length : e + 2;
      blank(i, end);
      i = end;
    } else if (raw && !/\w/.test(text[i - 1] ?? "")) {
      const close = `"${raw[1]}`;
      const e = text.indexOf(close, i + raw[0].length);
      const end = e < 0 ? text.length : e + close.length;
      blank(i, end);
      i = end;
    } else if (text[i] === '"') {
      let j = i + 1;
      while (j < text.length && text[j] !== '"') j += text[j] === "\\" ? 2 : 1;
      blank(i, j + 1);
      i = j + 1;
    } else if (/^'(\\.|[^\\'])'/.test(text.slice(i, i + 4))) {
      const len = /^'(\\.|[^\\'])'/.exec(text.slice(i, i + 4))[0].length;
      blank(i, i + len);
      i += len;
    } else i++;
  }
  return out.join("");
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

/** The `lavapipe` override's filter and each profile's default-filter. */
function configFilters(text) {
  let lavapipe;
  const defaults = new Map();
  for (const block of text.split(/^(?=\[)/m)) {
    const header = /^\[\[?([^\]]+)\]\]?/.exec(block)?.[1];
    if (header === "profile.default.overrides") {
      const f = /^filter\s*=\s*'([^']*)'/m.exec(block)?.[1];
      if (/test-group\s*=\s*['"]lavapipe['"]/.test(block)) lavapipe = f;
    }
    const prof = /^profile\.([^.]+)$/.exec(header ?? "")?.[1];
    const df = /^default-filter\s*=\s*'([^']*)'/m.exec(block)?.[1];
    if (prof && df) defaults.set(prof, df);
  }
  return { lavapipe, defaults };
}

if (!existsSync(join(ROOT, CONFIG))) {
  console.error(`${CONFIG} -> missing under ${ROOT}`);
  process.exit(1);
}
const { lavapipe, defaults } = configFilters(readFileSync(join(ROOT, CONFIG), "utf8"));
if (lavapipe !== GPU) {
  findings.push(`${CONFIG} -> the lavapipe override's filter is not '${GPU}'`);
}

// Each profile's default-filter, `undefined` for none; the readings' filter in soak and full.
for (const [profile, expected] of [
  ["quick", `not ${GPU}`],
  ["gpu", GPU],
  ["gate", undefined],
  ["soak", NOT_READING],
  ["full", NOT_READING],
  ["ci-windows", `not ${GPU}`],
]) {
  if (defaults.get(profile) !== expected) {
    const want = expected === undefined ? "no default-filter" : `default-filter '${expected}'`;
    findings.push(`${CONFIG} -> [profile.${profile}] does not have ${want}`);
  }
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

// Every `#[ignore]`: `"soak"`, or inside a `mod reading` block.
let ignores = 0;
for (const file of files.filter((p) => /^(crates|apps)\/.*\.rs$/.test(p)).sort()) {
  const text = readFileSync(join(ROOT, file), "utf8");
  const src = blanked(text);
  const readings = [];
  for (const m of src.matchAll(/\bmod\s+reading\s*\{/g)) {
    let depth = 0;
    let i = m.index + m[0].length - 1;
    for (; i < src.length; i++) {
      if (src[i] === "{") depth++;
      else if (src[i] === "}" && --depth === 0) break;
    }
    readings.push([m.index, i]);
  }
  for (const m of src.matchAll(/#\[\s*ignore\b[^\]]*\]/g)) {
    ignores++;
    const soak = /^#\[\s*ignore\s*=\s*"soak"\s*\]$/.test(text.slice(m.index, m.index + m[0].length));
    const inReading = readings.some(([s, e]) => m.index > s && m.index < e);
    if (!soak && !inReading) {
      const line = src.slice(0, m.index).split("\n").length;
      findings.push(
        `${file}:${line} -> an #[ignore] that is neither "soak" nor inside \`mod reading\`: mark a long series #[ignore = "soak"], or move a reading into the file's \`mod reading\``,
      );
    }
  }
}

// Every file under a `tests/` directory: at most ENV_CAP distinct `VXE_*` names.
let envFiles = 0;
for (const file of files.filter((p) => /^(crates|apps)\/[^/]+\/tests\/.*\.rs$/.test(p)).sort()) {
  const names = new Set([...code(file).matchAll(/"(VXE_[A-Z0-9_]+)"/g)].map((m) => m[1]));
  if (names.size > 0) envFiles++;
  if (names.size > ENV_CAP) {
    findings.push(
      `${file} -> names ${names.size} VXE_* environment variables, over the cap of ${ENV_CAP}: a reading with more switches is an example with flags (${[...names].sort().join(", ")})`,
    );
  }
}

if (findings.length) {
  for (const f of findings) console.error(f);
  console.error(`${findings.length} finding(s) across ${testFiles.length + units} test binaries`);
  process.exit(1);
}
const gpuCount = testFiles.filter((f) => f.split("/").pop().startsWith(PREFIX)).length;
console.log(
  `test tiers: ${testFiles.length} test files (${gpuCount} ${PREFIX}) and ${units} example and binary test modules, every device behind the prefix; ${ignores} #[ignore]s, each soak or a reading; ${envFiles} files naming VXE_* variables, each at most ${ENV_CAP}`,
);
