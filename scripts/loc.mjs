#!/usr/bin/env node
// Print the repository's size by area, so "how big is it, and where" is a number read at a
// close review rather than a guess.
//
// Over the tracked files at HEAD's working tree: per crate or app, Rust code lines, Rust test
// lines, Rust comment lines and WGSL code lines (blank lines not counted); markdown lines per
// top-level area; then the largest .rs and .wgsl files. A Rust line counts as a test when its
// file is under tests/, benches/ or a support/ of them, or when it sits inside a
// `#[cfg(test)]` item of a source file (found by brace depth, so a brace inside a string can
// misplace the end; the totals are a reading, not an exact count). Doc comments count as
// comments.
//
// Usage:  node scripts/loc.mjs [--top N] [--min-lines N]
//   --top N        how many of the largest files to list (default 15)
//   --min-lines N  list only files at or over N lines (default 0)
// Exit 0 always; this is a reading, not a gate. The file-size gate is a separate script.

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, resolve, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
let top = 15;
let minLines = 0;
for (let i = 0; i < args.length; i++) {
  if (args[i] === "--top") top = Number(args[++i]);
  else if (args[i] === "--min-lines") minLines = Number(args[++i]);
  else {
    console.error(`loc: unknown argument ${args[i]}`);
    process.exit(2);
  }
}

const files = execFileSync("git", ["ls-files", "-z"], { cwd: ROOT, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 })
  .split("\0")
  .filter(Boolean)
  // Worktrees under .claude/worktrees are other checkouts, never this tree's files.
  .filter((p) => !p.startsWith(".claude/worktrees/"));

// crates/<name>, apps/<name>, apps/<name>/logic, <crate>/guests; else the top-level entry.
function area(p) {
  const parts = p.split("/");
  if ((parts[0] === "crates" || parts[0] === "apps") && parts.length > 2) {
    let a = `${parts[0]}/${parts[1]}`;
    if (parts.includes("guests")) return `${a}/guests`;
    if (parts[2] === "logic") a += "/logic";
    return a;
  }
  return parts[0];
}

const stats = new Map();
const bump = (a, k, n = 1) => {
  if (!stats.has(a)) stats.set(a, {});
  const s = stats.get(a);
  s[k] = (s[k] ?? 0) + n;
};
const sizes = [];

for (const p of files) {
  const ext = p.includes(".") ? p.slice(p.lastIndexOf(".") + 1) : "";
  if (!["rs", "wgsl", "md"].includes(ext)) continue;
  let text;
  try {
    text = readFileSync(join(ROOT, p), "utf8");
  } catch {
    continue; // deleted in the working tree
  }
  const lines = text.split("\n");
  if (text.endsWith("\n")) lines.pop();
  const a = area(p);

  if (ext === "md") {
    bump(a, "md", lines.length);
    continue;
  }
  sizes.push([lines.length, p]);

  if (ext === "wgsl") {
    for (const l of lines) {
      const s = l.trim();
      if (s && !s.startsWith("//")) bump(a, "wgsl");
    }
    continue;
  }

  const testFile = /(^|\/)(tests|benches)\//.test(p);
  let inTest = false;
  let pending = false;
  let depth = 0;
  for (const l of lines) {
    const s = l.trim();
    if (!testFile && !inTest && s.startsWith("#[cfg(test)]")) pending = true;
    if (pending && s.includes("{")) {
      inTest = true;
      pending = false;
      depth = 0;
    } else if (pending && s.endsWith(";")) {
      pending = false; // `#[cfg(test)] mod tests;` or a single gated item
    }
    const test = testFile || inTest || pending;
    if (!s) bump(a, "blank");
    else if (s.startsWith("//")) bump(a, "comment");
    else bump(a, test ? "test" : "code");
    if (inTest) {
      for (const c of l) depth += c === "{" ? 1 : c === "}" ? -1 : 0;
      if (depth <= 0) inTest = false;
    }
  }
}

const cols = [
  ["Rust code", "code"],
  ["Rust test", "test"],
  ["comment", "comment"],
  ["WGSL code", "wgsl"],
  ["markdown", "md"],
];
const fmt = (n) => (n ?? 0).toLocaleString("en-US").padStart(11);
const width = Math.max(28, ...[...stats.keys()].map((k) => k.length + 2));
console.log("area".padEnd(width) + cols.map(([h]) => h.padStart(11)).join(""));
const total = {};
const shown = [...stats.entries()]
  .filter(([, s]) => cols.some(([, k]) => s[k]))
  .sort(([x], [y]) => x.localeCompare(y));
for (const [a, s] of shown) {
  console.log(a.padEnd(width) + cols.map(([, k]) => fmt(s[k])).join(""));
  for (const [, k] of cols) total[k] = (total[k] ?? 0) + (s[k] ?? 0);
}
console.log("total".padEnd(width) + cols.map(([, k]) => fmt(total[k])).join(""));
const functional = (total.code ?? 0) + (total.wgsl ?? 0);
const ratio = functional ? ((total.test ?? 0) / (total.code || 1)).toFixed(2) : "-";
console.log(`functional (Rust code + WGSL code): ${functional.toLocaleString("en-US")}; Rust test : code = ${ratio}`);

console.log(`\nlargest .rs and .wgsl files${minLines ? ` at or over ${minLines} lines` : ""}:`);
sizes
  .filter(([n]) => n >= minLines)
  .sort((x, y) => y[0] - x[0])
  .slice(0, top)
  .forEach(([n, p]) => console.log(`${String(n).padStart(7)}  ${p}`));
