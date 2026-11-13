#!/usr/bin/env node
// Assert the file caps of Plan 0045's Decision: no `.rs` or `.wgsl` file under a `src/`
// directory of crates/ or apps/ is over 1200 lines, and none under a `tests/` (its `support/`
// included), `benches/` or `examples/` directory is over 2000.
//
// A size rule written in prose regrows unnoticed: the files this cap was set against had
// grown to 10 226 lines (the fly-through's main.rs) and 6 776 (the bake module) before a
// review named them. So it is a gate. The function cap, 150 lines, is clippy's
// `too_many_lines` (clippy.toml and the workspace lints), not this script's.
//
// A file is classed by the nearest of those directory names on its path: `src/` is the
// source cap, `tests/`, `benches/` or `examples/` the test cap. A file under neither (a
// crate's build.rs) is not measured. Lines are counted as `wc -l` counts them, so a file of
// exactly the cap passes and one more line is a finding.
//
// Usage:  node scripts/check-file-size.mjs [root]
// Exit 0 = every measured file is within its cap; the counts are printed. Exit 1 = one line
// per file over, `<file> -> N lines, over the <kind> cap of C`. The optional `root` scans
// another tree: scripts/fixtures/file-size is the seeded one, and scripts/fixtures/README.md
// names its expected findings. Fixture trees are skipped by path on an ordinary run, and a
// file is read only once git tracks it.

import { readFileSync } from "node:fs";
import { dirname, resolve, join, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { execFileSync } from "node:child_process";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const ROOT = resolve(process.argv[2] ?? REPO_ROOT);
const SEEDED = resolve(REPO_ROOT, "scripts", "fixtures");
const CAPS = { source: 1200, test: 2000 };

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

/** `source` or `test` by the nearest classing directory on `path`, or `null` for neither. */
function kind(path) {
  const dirs = path.split("/").slice(0, -1).reverse();
  for (const d of dirs) {
    if (d === "src") return "source";
    if (d === "tests" || d === "benches" || d === "examples") return "test";
  }
  return null;
}

/** Lines as `wc -l` counts them: the newlines. */
const lines = (path) => readFileSync(join(ROOT, path), "utf8").split("\n").length - 1;

const findings = [];
const measured = { source: 0, test: 0 };
for (const file of trackedFiles()
  .filter((p) => /^(crates|apps)\/.*\.(rs|wgsl)$/.test(p))
  .sort()) {
  const k = kind(file);
  if (!k) continue;
  measured[k]++;
  const n = lines(file);
  if (n > CAPS[k]) findings.push(`${file} -> ${n} lines, over the ${k} cap of ${CAPS[k]}`);
}

if (findings.length) {
  for (const f of findings) console.error(f);
  console.error(`${findings.length} file(s) over their cap`);
  process.exit(1);
}
console.log(
  `file size: ${measured.source} source files within ${CAPS.source} lines, ${measured.test} test, bench and example files within ${CAPS.test}`,
);
