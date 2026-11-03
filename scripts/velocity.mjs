#!/usr/bin/env node
// Print the repository's velocity for the last N days, so a slowdown is a number read at
// every close review rather than a feeling noticed three weeks late.
//
// Per day, over every branch and without merge commits: commits, lines added and removed
// under crates/ and apps/ (code) and under docs/ and .claude/ (docs), the active span from
// the first to the last commit, the median gap between consecutive commits, and the plans
// closed (files first added under docs/plans/done/). The last line sums the window and the
// window before it, so "half of last week" is a comparison, not a guess.
//
// Usage:  node scripts/velocity.mjs [--days N] [--until YYYY-MM-DD]
// Exit 0 always; this is a reading, not a gate.

import { execFileSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
let days = 14;
let until = new Date();
for (let i = 0; i < args.length; i++) {
  if (args[i] === "--days") days = Number(args[++i]);
  else if (args[i] === "--until") until = new Date(`${args[++i]}T23:59:59`);
  else {
    console.error(`velocity: unknown argument ${args[i]}`);
    process.exit(2);
  }
}

const git = (...argv) =>
  execFileSync("git", argv, { cwd: ROOT, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"], maxBuffer: 256 * 1024 * 1024 });

const dayOf = (unix) => new Date(unix * 1000).toISOString().slice(0, 10);
const since = new Date(until.getTime() - 2 * days * 86400 * 1000);
since.setHours(0, 0, 0, 0);
const sinceArg = `--since=${since.toISOString()}`;
const untilArg = `--until=${until.toISOString()}`;

// One record per commit: time, then numstat lines until the next record.
const log = git("log", "--all", "--no-merges", "--numstat", "--format=@%at", sinceArg, untilArg);
const perDay = new Map();
const day = (d) => {
  if (!perDay.has(d)) perDay.set(d, { commits: 0, times: [], codeAdd: 0, codeDel: 0, docAdd: 0, docDel: 0, closed: 0 });
  return perDay.get(d);
};
let current;
for (const line of log.split("\n")) {
  if (line.startsWith("@")) {
    const t = Number(line.slice(1));
    current = day(dayOf(t));
    current.commits++;
    current.times.push(t);
  } else if (current && /^\d+\t\d+\t/.test(line)) {
    const [a, d, path] = line.split("\t");
    if (/^(crates|apps)\//.test(path)) {
      current.codeAdd += Number(a);
      current.codeDel += Number(d);
    } else if (/^(docs|\.claude)\//.test(path)) {
      current.docAdd += Number(a);
      current.docDel += Number(d);
    }
  }
}
for (const line of git("log", "--all", "--diff-filter=A", "--format=@%at", "--name-only", sinceArg, untilArg, "--", "docs/plans/done/").split("\n")) {
  if (line.startsWith("@")) current = day(dayOf(Number(line.slice(1))));
  else if (current && /^docs\/plans\/done\/\d{4}-.*\.md$/.test(line)) current.closed++;
}

const median = (xs) => {
  if (!xs.length) return 0;
  const s = [...xs].sort((a, b) => a - b);
  return s[Math.floor(s.length / 2)];
};

const rows = [...perDay.entries()].sort(([a], [b]) => (a < b ? -1 : 1));
console.log("day         commits  code +/-        docs +/-        active h  median gap  closed");
for (const [d, r] of rows) {
  const times = r.times.sort((a, b) => a - b);
  const gaps = times.slice(1).map((t, i) => (t - times[i]) / 60);
  const active = times.length > 1 ? (times.at(-1) - times[0]) / 3600 : 0;
  console.log(
    `${d}  ${String(r.commits).padStart(7)}  ${`${r.codeAdd}+/${r.codeDel}-`.padEnd(15)} ${`${r.docAdd}+/${r.docDel}-`.padEnd(15)} ${active.toFixed(1).padStart(8)}  ${median(gaps).toFixed(0).padStart(7)} min  ${String(r.closed).padStart(6)}`,
  );
}

const cut = new Date(until.getTime() - days * 86400 * 1000).toISOString().slice(0, 10);
const sum = (keep) => {
  const out = { commits: 0, code: 0, docs: 0, closed: 0, days: 0 };
  for (const [d, r] of rows) {
    if (!keep(d)) continue;
    out.commits += r.commits;
    out.code += r.codeAdd + r.codeDel;
    out.docs += r.docAdd + r.docDel;
    out.closed += r.closed;
    out.days++;
  }
  return out;
};
const last = sum((d) => d >= cut);
const before = sum((d) => d < cut);
const fmt = (w, name) => `${name}: ${w.commits} commits over ${w.days} active day(s), ${w.code} code lines, ${w.docs} doc lines, ${w.closed} plan(s) closed`;
console.log("");
console.log(fmt(before, `the ${days} days before ${cut}`));
console.log(fmt(last, `the last ${days} days`));
if (before.commits && last.commits) {
  const ratio = (last.commits / Math.max(last.days, 1)) / (before.commits / Math.max(before.days, 1));
  console.log(`commits per active day: ${ratio.toFixed(2)}x the window before`);
}
