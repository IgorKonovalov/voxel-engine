#!/usr/bin/env node
// Rank the tests of a nextest run by wall time, and name the ones over the soak line.
//
// nextest prints one `PASS [ 12.345s] (n/total) <binary> <test>` line per test when run with
// `--status-level pass` (the lane rules' form, tee'd to a log). This script reads such a log,
// from a local run or a CI job log (`gh run view <id> --log`), and prints the slowest tests,
// the serial total, and every test over the line, which docs/nfr.md section 7 sends to the
// soak tier by the mark `#[ignore = "soak"]` (.config/nextest.toml). The line is 10 s on the
// dev reference's lavapipe, read quiet, and 60 s on CI: the default is the local one, and a CI
// log is read with `--over 60`. ANSI colour codes are tolerated, as the raw escape or in the
// caret form a CI log viewer writes.
//
// Usage:  node scripts/test-times.mjs <log> [--over <seconds>] [--top <n>]
// Exit 0 = no test over the line (default 10 s). Exit 1 = at least one, each printed as
//   <seconds> <binary> <test> -> over the <line> s line: mark it #[ignore = "soak"]

import { readFileSync } from "node:fs";

const args = process.argv.slice(2);
let log;
let over = 10;
let top = 25;
for (let i = 0; i < args.length; i++) {
  const a = args[i];
  if (a === "--over") over = Number(args[++i]);
  else if (a === "--top") top = Number(args[++i]);
  else if (!log) log = a;
  else {
    console.error(`test-times: unexpected argument ${a}`);
    process.exit(2);
  }
}
if (!log || !Number.isFinite(over) || !Number.isFinite(top)) {
  console.error("usage: node scripts/test-times.mjs <log> [--over <seconds>] [--top <n>]");
  process.exit(2);
}

const ANSI = /(?:\x1b|\^\[)\[[0-9;]*m/g;
const LINE = /\b(PASS|FAIL|TIMEOUT|LEAK)\b\s*\[\s*([0-9.]+)s\]\s*(?:\(\s*\d+\/\d+\)\s*)?(\S+)\s+(\S.*?)\s*$/;

const rows = [];
for (const raw of readFileSync(log, "utf8").split("\n")) {
  const m = LINE.exec(raw.replace(ANSI, ""));
  if (m) rows.push({ status: m[1], secs: Number(m[2]), binary: m[3], test: m[4] });
}
if (rows.length === 0) {
  console.error(`test-times: no per-test lines in ${log}; run nextest with --status-level pass`);
  process.exit(2);
}
rows.sort((a, b) => b.secs - a.secs);

const total = rows.reduce((s, r) => s + r.secs, 0);
const slow = rows.filter((r) => r.secs > over);
console.log(`${rows.length} tests, ${(total / 60).toFixed(1)} min serial; ${slow.length} over ${over} s (${(slow.reduce((s, r) => s + r.secs, 0) / 60).toFixed(1)} min of it)`);
console.log(`top ${Math.min(top, rows.length)}:`);
for (const r of rows.slice(0, top)) {
  console.log(`${r.secs.toFixed(1).padStart(8)}  ${r.status.padEnd(7)} ${r.binary}  ${r.test}`);
}
if (slow.length) {
  console.log("");
  for (const r of slow) {
    console.log(`${r.secs.toFixed(1)} ${r.binary} ${r.test} -> over the ${over} s line: mark it #[ignore = "soak"]`);
  }
  process.exit(1);
}
