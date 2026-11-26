#!/usr/bin/env node
// No gate test passes by a clock: a test that sleeps, polls with a timeout, compares a duration
// with a constant, runs the bake thread without the blocking wait, or steps a queue on a device
// with timestamps reads the machine's pace, not the code (Plan 0106). Every such site is a
// finding unless `scripts/test-clocks-allow.txt` names it with a reason.
//
// Test code is a tracked `.rs` file under a crate's or app's `tests/`, a file under `src/`
// whose path has a component naming `tests` (`hud_tests.rs`, `split_tests/thread.rs`), a file
// a test file declares (`mod fade;`) or a `#[cfg(test)] mod name;` declares, and every
// `#[cfg(test)]` item elsewhere. Comments and string literals are blanked first, so a doc line
// or a message naming a token is not a use. Exempt: the inside of a `mod reading { .. }` block
// (a measurement run by hand may time anything), and a helper every call of which sits in one.
// No tier mark is exempt: a test marked `#[ignore = "soak"]` (the weekly tier) or `sole:` is
// read as a gate test is, and so is a helper only such a test calls.
//
// The rules, each site reported with its enclosing function (the test, or the helper it calls):
//
//   1  `sleep(`, `yield_now(`, `recv_timeout(`, `wait_timeout*(`, `park_timeout(`
//   2  a comparison (`<`, `<=`, `>`, `>=`) with a literal or a constant inside an `assert!`
//      family macro, or in an `if` whose block panics, where one side reads a duration:
//      `.elapsed()`, `duration_since`, `as_secs*`, `as_millis`, `as_micros`, `as_nanos`, or a
//      name the file bound to one (`let ms = t.elapsed()..`, `fn seconds_since(..) -> f64`)
//   3  `set_bake_thread(.., true)`, or a `"--bake-thread", "on"` argument pair in a file that
//      spawns (under `tests/`, or naming `Command::new`), in a file whose code never names
//      `wait_bake_step`: the bake thread then runs at its own pace beside frames the test counts
//   4  `TIMESTAMP_QUERY` in a file that steps a queue (`.step(`, `step_with(`, `BakeQueue`):
//      with timestamps the bake step's sample cap adapts to measured time, so a step count is
//      no longer causal
//
// What it cannot see (the review's): a give-up loop that counts frames with no sleep, a poll
// with no wait, a bound hidden behind a struct field or a function returning a non-float, an
// implicit bound inside the code under test (the HUD banner's `BANNER_MS` after `now()`).
//
// The allow-list: one row a site, `file | fn | token | reason`, `#` comments and blank lines
// ignored. A row covers every site of its token in that function of that file. A row whose
// site is gone is a finding, so the list cannot outlive what it excuses.
//
// Usage:  node scripts/check-test-clocks.mjs [root]
//         node scripts/check-test-clocks.mjs --self-test
// Exit 0 = every site is allow-listed and every row has a site. Exit 1 = one line a finding:
//   <file>:<line> -> rule N: `<token>` in `<fn>`: <what it is>
//   scripts/test-clocks-allow.txt:<line> -> a row whose site is gone: <row>
// The optional `root` scans another tree with that tree's own allow-list;
// scripts/fixtures/test-clocks is the seeded one, and scripts/fixtures/README.md names its
// expected findings. `--self-test` asserts them, and a floor on the repository's scan.

import { readFileSync, existsSync } from "node:fs";
import { dirname, resolve, join, sep, posix } from "node:path";
import { fileURLToPath } from "node:url";
import { execFileSync } from "node:child_process";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SEEDED = resolve(REPO_ROOT, "scripts", "fixtures");
const FIXTURE = resolve(SEEDED, "test-clocks");
const ALLOW = "scripts/test-clocks-allow.txt";

const RULE1 = /\b(sleep|yield_now|recv_timeout|wait_timeout\w*|park_timeout)\s*\(/g;
const DURATION = /\.elapsed\s*\(\s*\)|\bduration_since\b|\bas_secs(?:_f32|_f64)?\b|\bas_millis\b|\bas_micros\b|\bas_nanos\b/;
const COMPARE = /\s(<=|>=|<|>)\s/;
const CONSTANT = /(?:^|[^\w.])(\d[\d_]*(?:\.\d+)?(?:e-?\d+)?(?:_?[a-z]\w*)?|[A-Z][A-Z0-9_]{2,}|Duration::\w+)\b/;
const ASSERT = /\b(?:debug_)?assert(?:_eq|_ne)?!\s*\(/g;
const IF = /\bif\s+/g;
const STEPS_A_QUEUE = /\.step\s*\(|\bstep_with\s*\(|\bBakeQueue\b/;

/** Tracked files under `root`, relative to it, skipping seeded trees that are not the root. */
function trackedFiles(root) {
  const out = execFileSync("git", ["ls-files", "-z"], {
    cwd: root,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "ignore"],
    maxBuffer: 64 * 1024 * 1024,
  });
  const inSeeded = (abs) =>
    !(root === SEEDED || root.startsWith(SEEDED + sep)) && abs.startsWith(SEEDED + sep);
  return out.split("\0").filter((p) => p && !inSeeded(resolve(root, p)));
}

/** Rust source with comments (and, with `strings`, string and char literals) turned to spaces,
 * offsets and newlines kept. Block comments nest, as in Rust. */
function blanked(text, strings = true) {
  const out = text.split("");
  const blank = (from, to) => {
    for (let k = from; k < to && k < out.length; k++) if (out[k] !== "\n") out[k] = " ";
  };
  let i = 0;
  while (i < text.length) {
    const two = text.slice(i, i + 2);
    const raw = /^b?r(#*)"/.exec(text.slice(i, i + 12));
    if (two === "//") {
      const e = text.indexOf("\n", i);
      const end = e < 0 ? text.length : e;
      blank(i, end);
      i = end;
    } else if (two === "/*") {
      let depth = 0;
      let j = i;
      for (; j < text.length; j++) {
        if (text.startsWith("/*", j)) (depth++, j++);
        else if (text.startsWith("*/", j) && --depth === 0) break;
        else if (text.startsWith("*/", j)) j++;
      }
      const end = Math.min(text.length, j + 2);
      blank(i, end);
      i = end;
    } else if (raw && !/\w/.test(text[i - 1] ?? "")) {
      const close = `"${raw[1]}`;
      const e = text.indexOf(close, i + raw[0].length);
      const end = e < 0 ? text.length : e + close.length;
      if (strings) blank(i, end);
      i = end;
    } else if (text[i] === '"') {
      let j = i + 1;
      while (j < text.length && text[j] !== '"') j += text[j] === "\\" ? 2 : 1;
      if (strings) blank(i, j + 1);
      i = j + 1;
    } else if (/^'(\\.|[^\\'])'/.test(text.slice(i, i + 4))) {
      const len = /^'(\\.|[^\\'])'/.exec(text.slice(i, i + 4))[0].length;
      if (strings) blank(i, i + len);
      i += len;
    } else i++;
  }
  return out.join("");
}

/** The index of the bracket closing the one opened at `open` (`(`, `{` or `[`). */
function closing(src, open) {
  const pairs = { "(": ")", "{": "}", "[": "]" };
  const o = src[open];
  const c = pairs[o];
  let depth = 0;
  for (let i = open; i < src.length; i++) {
    if (src[i] === o) depth++;
    else if (src[i] === c && --depth === 0) return i;
  }
  return src.length;
}

/** Each `fn`'s name, its attribute span's start and its body `[open, close]`. */
function functions(src) {
  const out = [];
  for (const m of src.matchAll(/\bfn\s+([A-Za-z_]\w*)/g)) {
    let i = m.index + m[0].length;
    // The body's `{` is the first at bracket depth 0 after the signature; a `;` first is a
    // declaration with no body.
    let depth = 0;
    for (; i < src.length; i++) {
      const ch = src[i];
      if (ch === "(" || ch === "[" || ch === "<") depth++;
      else if ((ch === ")" || ch === "]" || (ch === ">" && src[i - 1] !== "-")) && depth > 0) depth--;
      else if (depth === 0 && (ch === "{" || ch === ";")) break;
    }
    if (src[i] !== "{") continue;
    // The attributes: back to the previous `}`, `;` or `{`.
    let a = m.index;
    while (a > 0 && !"};{".includes(src[a - 1])) a--;
    out.push({ name: m[1], attrs: a, open: i, close: closing(src, i), sig: src.slice(m.index, i) });
  }
  return out;
}

/** `[start, end]` spans of each `#[cfg(test)]` item: through its matching brace, or its `;`. */
function cfgTestItems(src) {
  const out = [];
  for (const m of src.matchAll(/#\[cfg\(test\)\]/g)) {
    let i = m.index + m[0].length;
    for (; i < src.length; i++) {
      if (src[i] === "{") {
        i = closing(src, i);
        break;
      }
      if (src[i] === ";") break;
    }
    out.push([m.index, i]);
  }
  return out;
}

/** `[start, end]` spans of each `mod reading { .. }`. */
function readingMods(src) {
  return [...src.matchAll(/\bmod\s+reading\s*\{/g)].map((m) => {
    const open = m.index + m[0].length - 1;
    return [m.index, closing(src, open)];
  });
}

/** The module files a file declares with `mod name;` (a `#[path]` honoured), all, or only those
 * under `#[cfg(test)]`. `src` keeps string literals (the path is one) and blanks comments. */
function declared(file, src, files, onlyCfgTest) {
  const base = posix.basename(file);
  const dir = posix.dirname(file);
  const root =
    /^(mod|lib|main)\.rs$/.test(base) || /(^|\/)(tests|examples|benches|src\/bin)\/[^/]+\.rs$/.test(file);
  const childDir = root ? dir : `${dir}/${base.slice(0, -3)}`;
  const out = [];
  const re =
    /((?:#\[[^\]]*\]\s*)*)(?:pub(?:\([^)]*\))?\s+)?\bmod\s+([a-z_][a-z0-9_]*)\s*;/g;
  for (const m of src.matchAll(re)) {
    if (onlyCfgTest && !/#\[cfg\(test\)\]/.test(m[1])) continue;
    const path = /#\[path\s*=\s*"([^"]+)"\]/.exec(m[1])?.[1];
    const f = path
      ? [posix.normalize(`${dir}/${path}`)].find((p) => files.has(p))
      : [`${childDir}/${m[2]}.rs`, `${childDir}/${m[2]}/mod.rs`].find((p) => files.has(p));
    if (f) out.push(f);
  }
  return out;
}

/** Every file `file` reaches through `mod name;` declarations, itself included. */
function reached(file, decl) {
  const seen = new Set([file]);
  const todo = [file];
  while (todo.length) for (const d of decl.get(todo.pop()) ?? []) if (!seen.has(d)) (seen.add(d), todo.push(d));
  return seen;
}

/** `s` split at its commas outside brackets. */
function topLevel(s) {
  const out = [];
  let depth = 0;
  let from = 0;
  for (let i = 0; i < s.length; i++) {
    if ("([{".includes(s[i])) depth++;
    else if (")]}".includes(s[i])) depth--;
    else if (s[i] === "," && depth === 0) (out.push(s.slice(from, i)), (from = i + 1));
  }
  if (s.slice(from).trim()) out.push(s.slice(from));
  return out;
}

const lineOf = (src, at) => src.slice(0, at).split("\n").length;
const inside = (spans, at) => spans.some(([s, e]) => at > s && at < e);

/** The scan of `root`: every site, its rule, token, function and line, plus counts. */
function scan(root) {
  const all = trackedFiles(root).filter((p) => /^(crates|apps)\/.*\.rs$/.test(p));
  const files = new Set(all);
  const read = new Map(all.map((f) => [f, readFileSync(join(root, f), "utf8")]));
  const code = new Map([...read].map(([f, t]) => [f, blanked(t)]));
  const quoted = new Map([...read].map(([f, t]) => [f, blanked(t, false)]));
  const decl = new Map(all.map((f) => [f, declared(f, quoted.get(f), files, false)]));

  // Whole files of test code: by path, then every file a `#[cfg(test)] mod name;` declares,
  // then every file a test file declares, to a fixed point.
  const whole = new Set(
    all.filter((f) => /^(crates|apps)\/[^/]+\/tests\//.test(f) || /\/src\/(.*\/)?[^/]*tests[^/]*(\/|$)/.test(f)),
  );
  for (const f of all) for (const d of declared(f, quoted.get(f), files, true)) whole.add(d);
  for (let grew = true; grew; ) {
    grew = false;
    for (const f of [...whole]) {
      for (const d of decl.get(f)) {
        if (!whole.has(d)) (whole.add(d), (grew = true));
      }
    }
  }

  // Functions anywhere in the tree that return a number read from a clock (`seconds_since`,
  // `ms_since`): a test calls them across files, so their names bind everywhere.
  const clockFns = new Set();
  for (const f of all) {
    const src = code.get(f);
    for (const fn of functions(src)) {
      if (/->\s*(f32|f64|u64|u128|Duration)\s*$/.test(fn.sig.trim()) && DURATION.test(src.slice(fn.open, fn.close))) {
        clockFns.add(fn.name);
      }
    }
  }

  const sites = [];
  let testFiles = 0;
  for (const file of all.sort()) {
    const text = read.get(file);
    const src = code.get(file);
    const withStrings = quoted.get(file);
    const isWhole = whole.has(file);
    const cfgItems = isWhole ? [] : cfgTestItems(src);
    if (!isWhole && cfgItems.length === 0) continue;
    testFiles++;
    const fns = functions(src);
    // Exempt spans: `mod reading` blocks, then every helper whose calls in this file all sit in
    // exempt spans (at least one call), to a fixed point. A helper called from another file is
    // not followed: it stays reported.
    const spans = [...readingMods(src)];
    const exempt = (at) => inside(spans, at);
    for (let grew = true; grew; ) {
      grew = false;
      for (const f of fns) {
        if (exempt(f.open + 1) || /#\[\s*test\s*\]/.test(src.slice(f.attrs, f.open))) continue;
        const calls = [...src.matchAll(new RegExp(`(?<![\\w.])${f.name}\\s*\\(`, "g"))]
          .map((m) => m.index)
          .filter((at) => !/\bfn\s+$/.test(src.slice(Math.max(0, at - 8), at)));
        if (calls.length && calls.every(exempt)) (spans.push([f.attrs, f.close]), (grew = true));
      }
    }
    const isTest = (at) => isWhole || inside(cfgItems, at);
    const fnOf = (at) => fns.filter((f) => at > f.open && at < f.close).sort((a, b) => b.open - a.open)[0];
    const fnAt = (at) => fnOf(at)?.name ?? "(file)";
    const add = (rule, token, at, what) => {
      if (!isTest(at) || exempt(at)) return;
      sites.push({ file, line: lineOf(src, at), rule, token, fn: fnAt(at), what });
    };

    // Rule 1.
    for (const m of src.matchAll(RULE1)) {
      if (/\bfn\s+$/.test(src.slice(Math.max(0, m.index - 8), m.index))) continue;
      add(1, m[1], m.index, "a wait paced by the wall clock");
    }

    // Rule 2: the names bound to a duration, to a fixed point. A function returning a number
    // binds its name for the whole file; a `let` binds its name inside its own function, from
    // the `let` on.
    const boundFns = new Set(clockFns);
    const lets = []; // { name, from, to }
    const named = (s, at) =>
      [...boundFns].find((n) => new RegExp(`\\b${n}\\s*\\(`).test(s)) ??
      lets.find((l) => at > l.from && at < l.to && new RegExp(`(?<![\\w.])${l.name}\\b`).test(s))?.name;
    const reads = (s, at) => DURATION.test(s) || named(s, at) !== undefined;
    // Each `let`'s (name, value) pairs: `let x = v;`, or a flat tuple `let (a, b) = (u, v);`
    // paired by position (any other value binds every name of the tuple to the whole value).
    const letSites = [];
    for (const m of src.matchAll(/\blet\s+(?:mut\s+)?([a-z_]\w*|\([^()=;]*\))\s*(?::[^=;]+)?=([^;]*);/g)) {
      if (!m[1].startsWith("(")) {
        letSites.push({ at: m.index, name: m[1], value: m[2] });
        continue;
      }
      const names = topLevel(m[1].slice(1, -1)).map((n) => n.replace(/^\s*(ref\s+)?(mut\s+)?/, "").trim());
      const v = m[2].trim();
      const values = v.startsWith("(") && closing(v, 0) === v.length - 1 ? topLevel(v.slice(1, -1)) : [];
      names.forEach((name, k) => {
        if (/^[a-z_]\w*$/.test(name) && name !== "_") {
          letSites.push({ at: m.index, name, value: values.length === names.length ? values[k] : v });
        }
      });
    }
    for (let grew = true; grew; ) {
      grew = false;
      for (const s of letSites) {
        if (lets.some((l) => l.from === s.at && l.name === s.name) || !reads(s.value, s.at)) continue;
        lets.push({ name: s.name, from: s.at, to: fnOf(s.at)?.close ?? src.length });
        grew = true;
      }
      for (const f of fns) {
        if (boundFns.has(f.name) || !/->\s*(f32|f64|u64|u128|Duration)\s*$/.test(f.sig.trim())) continue;
        if (reads(src.slice(f.open, f.close), f.open + 1)) (boundFns.add(f.name), (grew = true));
      }
    }
    const durationToken = (s, at) =>
      DURATION.exec(s)?.[0].replace(/^\./, "").replace(/\s*\(\s*\)$/, "") ?? named(s, at);
    // A comparison with a literal or a constant; zero alone is no bound (a duration is never
    // below it), so a condition whose only constants are zero is not one.
    const compared = (cond, at) => {
      if (!COMPARE.test(cond)) return undefined;
      const consts = [...cond.matchAll(new RegExp(CONSTANT.source, "g"))].map((m) => m[1]);
      if (!consts.some((c) => !/^0+(\.0*)?(_?[a-z]\w*)?$/.test(c))) return undefined;
      return durationToken(cond, at);
    };
    for (const m of src.matchAll(ASSERT)) {
      const open = m.index + m[0].length - 1;
      const args = src.slice(open + 1, closing(src, open));
      // The condition: up to the first comma at depth 0.
      let depth = 0;
      let k = 0;
      for (; k < args.length; k++) {
        if ("([{".includes(args[k])) depth++;
        else if (")]}".includes(args[k])) depth--;
        else if (args[k] === "," && depth === 0) break;
      }
      const token = compared(args.slice(0, k), m.index);
      if (token) add(2, token, m.index, "a duration compared with a constant in an assertion");
    }
    for (const m of src.matchAll(IF)) {
      const brace = src.indexOf("{", m.index);
      if (brace < 0) continue;
      const cond = src.slice(m.index + m[0].length, brace);
      const body = src.slice(brace, closing(src, brace));
      const token = compared(cond, m.index);
      if (token && /\b(panic|unreachable)!/.test(body)) {
        add(2, token, m.index, "a duration compared with a constant before a panic");
      }
    }

    // Rule 3.
    if (![...reached(file, decl)].some((f) => /\bwait_bake_step\b/.test(code.get(f)))) {
      for (const m of src.matchAll(/\bset_bake_thread\s*\(/g)) {
        const open = m.index + m[0].length - 1;
        if (/\btrue\b/.test(src.slice(open, closing(src, open)))) {
          add(3, "set_bake_thread", m.index, "the bake thread on in a file that never names `wait_bake_step`");
        }
      }
      const spawns = /^(crates|apps)\/[^/]+\/tests\//.test(file) || /\bCommand::new\b/.test(src);
      if (spawns) {
        for (const m of withStrings.matchAll(/"--bake-thread"\s*,\s*"on"/g)) {
          add(3, "--bake-thread on", m.index, "a child with the bake thread on in a file that never names `wait_bake_step`");
        }
      }
    }

    // Rule 4.
    if (STEPS_A_QUEUE.test(src)) {
      for (const m of src.matchAll(/\bTIMESTAMP_QUERY\b/g)) {
        add(4, "TIMESTAMP_QUERY", m.index, "timestamps on a device in a file that steps a queue");
      }
    }
  }
  return { sites, testFiles };
}

/** The allow-list's rows: file, fn, token, reason and the row's line. */
function allowRows(root) {
  const path = join(root, ALLOW);
  if (!existsSync(path)) return { rows: [], errors: [] };
  const rows = [];
  const errors = [];
  readFileSync(path, "utf8")
    .split("\n")
    .forEach((raw, i) => {
      const line = raw.trim();
      if (!line || line.startsWith("#")) return;
      // The reason is everything after the third `|`, so it may hold one.
      const parts = line.split("|");
      const cells = [...parts.slice(0, 3), parts.slice(3).join("|")].map((c) => (c ?? "").trim());
      if (parts.length < 4 || cells.some((c) => !c)) {
        errors.push(`${ALLOW}:${i + 1} -> a row is \`file | fn | token | reason\`, all four filled: ${line}`);
        return;
      }
      rows.push({ file: cells[0], fn: cells[1], token: cells[2], reason: cells[3], line: i + 1 });
    });
  return { rows, errors };
}

/** The findings of `root` against its own allow-list. */
function check(root) {
  const { sites, testFiles } = scan(root);
  const { rows, errors } = allowRows(root);
  const used = new Set();
  const findings = [...errors];
  for (const s of sites) {
    const row = rows.find((r) => r.file === s.file && r.fn === s.fn && r.token === s.token);
    if (row) used.add(row);
    else findings.push(`${s.file}:${s.line} -> rule ${s.rule}: \`${s.token}\` in \`${s.fn}\`: ${s.what}`);
  }
  for (const r of rows) {
    if (!used.has(r)) {
      findings.push(`${ALLOW}:${r.line} -> a row whose site is gone: ${r.file} | ${r.fn} | ${r.token}`);
    }
  }
  return { findings, sites, rows, used: used.size, testFiles };
}

// --- self-test ---------------------------------------------------------------
//
// The fixture's findings are pinned line for line: a rule whose pattern stopped matching shows
// as a missing line, not as a quiet exit 0. The repository half is a floor: a scan that found
// no test file, or matched none of the allow-list's rows, measures nothing.
const EXPECTED = [
  "crates/seeded/tests/seeded.rs:14 -> rule 1: `sleep` in `paced_by_a_sleep`",
  "crates/seeded/tests/seeded.rs:21 -> rule 2: `elapsed` in `bound_by_a_constant`",
  "crates/seeded/tests/seeded.rs:27 -> rule 2: `ms` in `bound_through_a_name`",
  "crates/seeded/tests/seeded.rs:56 -> rule 1: `sleep` in `a_long_series`",
  "crates/seeded/tests/seeded.rs:60 -> rule 1: `yield_now` in `paced_helper`",
  "crates/seeded/src/lib.rs:30 -> rule 2: `seconds_since` in `panics_past_a_bound`",
  "crates/seeded/tests/threaded.rs:9 -> rule 3: `set_bake_thread` in `thread_on_and_counted`",
  "crates/seeded/tests/timed.rs:9 -> rule 4: `TIMESTAMP_QUERY` in `timed_device`",
  "scripts/test-clocks-allow.txt:5 -> a row whose site is gone: crates/seeded/tests/seeded.rs | removed_long_ago | sleep",
];
const REPO_TEST_FILE_FLOOR = 100;
const REPO_ROW_FLOOR = 5;

function selfTest() {
  const results = [];
  const record = (label, ok, detail) => results.push({ label, ok, detail });
  const fixture = check(FIXTURE);
  const got = fixture.findings.map((f) => f.replace(/^(.* in `[^`]*`):.*$/, "$1"));
  for (const want of EXPECTED) {
    record(`fixture: ${want.replace(/ -> .*/, "")}`, got.includes(want), want.replace(/^.* -> /, ""));
  }
  const extra = got.filter((g) => !EXPECTED.includes(g));
  record(`fixture: nothing beyond the ${EXPECTED.length} seeded`, extra.length === 0, extra.join("; ") || "none");
  record(
    "fixture: one of the allow-list's two rows covers a site",
    fixture.rows.length === 2 && fixture.used === 1,
    `${fixture.used} of ${fixture.rows.length} rows used`,
  );
  const repo = check(REPO_ROOT);
  record(
    `non-vacuity: the repository scans at least ${REPO_TEST_FILE_FLOOR} files of test code`,
    repo.testFiles >= REPO_TEST_FILE_FLOOR,
    `${repo.testFiles} files`,
  );
  record(
    `non-vacuity: at least ${REPO_ROW_FLOOR} allow-list rows match a site`,
    repo.used >= REPO_ROW_FLOOR,
    `${repo.used} of ${repo.rows.length} rows, ${repo.sites.length} sites`,
  );
  const width = Math.max(...results.map((r) => r.label.length));
  for (const r of results) {
    console.log(`${r.label}${" ".repeat(width - r.label.length + 1)} ${r.ok ? "OK" : "FAILED"} (${r.detail})`);
  }
  const passed = results.filter((r) => r.ok).length;
  console.log(`self-test: ${passed}/${results.length}`);
  process.exit(passed === results.length ? 0 : 1);
}

// --- main --------------------------------------------------------------------

const args = process.argv.slice(2);
if (args.includes("--self-test")) selfTest();
const ROOT = resolve(args.find((a) => !a.startsWith("--")) ?? REPO_ROOT);
const { findings, sites, rows, testFiles } = check(ROOT);
if (findings.length) {
  for (const f of findings) console.error(f);
  console.error(`${findings.length} finding(s) across ${testFiles} files of test code`);
  process.exit(1);
}
console.log(
  `test clocks: ${testFiles} files of test code, ${sites.length} clock sites, each allow-listed by one of ${rows.length} rows`,
);
