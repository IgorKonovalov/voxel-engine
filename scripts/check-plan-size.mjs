#!/usr/bin/env node
// Assert that a plan's `## Implementation log` stays a log, not a readings dump.
//
// Readings, attempts and resume notes belong in docs/plans/logs/NNNN.md; the plan body keeps
// one row per phase and the close block. Nothing in a markdown file stops the log from
// growing back (Plan 0054's reached a thousand lines, and every fresh agent read it), so this
// gate does: for every plan numbered 0055 or higher, under docs/plans/ or docs/plans/done/,
// the `## Implementation log` section (to the next `## ` heading or the end of file) is at
// most LOG_CAP lines and holds no `### Resume notes` heading.
//
// Usage:  node scripts/check-plan-size.mjs [root]
// Exit 0 = every plan in scope is within the cap. Exit 1 = one line per finding:
//   <file> -> Implementation log is N lines, cap LOG_CAP: move readings to docs/plans/logs/
//   <file>:<line> -> resume notes in the plan body: they go to docs/plans/logs/NNNN.md
// The optional `root` scans another tree; scripts/fixtures/plan-size is the seeded one, and
// scripts/fixtures/README.md names its expected findings. Fixture trees are skipped by path
// on an ordinary run.

import { readFileSync } from "node:fs";
import { dirname, resolve, sep, basename } from "node:path";
import { fileURLToPath } from "node:url";
import { execFileSync } from "node:child_process";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const ROOT = resolve(process.argv[2] ?? REPO_ROOT);
const SEEDED = resolve(REPO_ROOT, "scripts", "fixtures");
const LOG_CAP = 150;
const FIRST_PLAN = 55;

const files = execFileSync("git", ["ls-files", "-z"], {
  cwd: ROOT,
  encoding: "utf8",
  stdio: ["ignore", "pipe", "ignore"],
  maxBuffer: 64 * 1024 * 1024,
})
  .split("\0")
  .filter((p) => p && /^docs\/plans\/(done\/)?\d{4}-[^/]+\.md$/.test(p))
  .filter((p) => {
    const abs = resolve(ROOT, p);
    const rootIsSeeded = ROOT === SEEDED || ROOT.startsWith(SEEDED + sep);
    return rootIsSeeded || !abs.startsWith(SEEDED + sep);
  })
  .filter((p) => Number(basename(p).slice(0, 4)) >= FIRST_PLAN)
  .sort();

const findings = [];
for (const file of files) {
  const lines = readFileSync(resolve(ROOT, file), "utf8").split("\n");
  const start = lines.findIndex((l) => /^## Implementation log\s*$/.test(l));
  if (start < 0) continue;
  let end = lines.findIndex((l, i) => i > start && /^## /.test(l));
  if (end < 0) end = lines.length;
  const length = end - start;
  if (length > LOG_CAP) {
    findings.push(`${file} -> Implementation log is ${length} lines, cap ${LOG_CAP}: move readings to docs/plans/logs/`);
  }
  for (let i = start; i < end; i++) {
    if (/^###\s+Resume notes\b/.test(lines[i])) {
      findings.push(`${file}:${i + 1} -> resume notes in the plan body: they go to docs/plans/logs/NNNN.md`);
    }
  }
}

if (findings.length) {
  for (const f of findings) console.error(f);
  console.error(`${findings.length} finding(s) across ${files.length} plan(s)`);
  process.exit(1);
}
console.log(`plan size: ${files.length} plan(s) from ${String(FIRST_PLAN).padStart(4, "0")} on, every Implementation log within ${LOG_CAP} lines, no resume notes in a plan body`);
