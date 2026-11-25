#!/usr/bin/env node
// The window rehearsal: before the owner's live session opens, render each of its runs headless
// with the run's own flags, save a still and the mode text beside the runner, and write one sheet
// an agent reads, so an obvious fault (a white hole at a dig, two arms that draw the same picture,
// a banner that does not name a flag) never reaches the window.
//
//   node scripts/rehearse-window.mjs --adapter NAME --out DIR (--runner live/sessionN.sh | --list FILE)
//        [--bin PATH] [--frames N] [--render WxH] [--before RUN=BEFORE].. [--dig RUN=F,..]..
//        [--floor SHARE] [--tolerance LEVELS] [--named-ok FLAG,..] [--timeout S] [--memory SIZE]
//
// The runs come from a runner script (each `run "label" "hint" "$BIN" flags..` line, read by
// running the script with LIVE_LIST set, which turns the live-runner lib's `run` into a recorder;
// a run's name is its label's first word, `1.` -> `1`) or from a list file, one run a line:
// `name flags.. [before=NAME] [dig=F,..]`, `#` comments, single or double quotes group words.
// `--before` and `--dig` add the same to runner runs.
//
// Each run: `flock <perf lock> timeout S systemd-run --scope (memory cap) -- BIN <the run's flags>
// --headless --adapter NAME --frames N [--render WxH unless the run sets one] [--dig-at F,..]
// --still <out>/<run>.ppm --mode-text <out>/<run>.txt`, never windowed, the lock taken per run.
// It refuses to start, and stops between runs, while a live-session runner is up.
//
// Writes <out>/<run>.png, <out>/<run>.txt, <out>/<run>.log and <out>/index.md: per run its command
// line, the still, the facts (all one colour, the share of pure black, clipped white and near
// white, the mean level; against its `before` run the differing pixels' count and share) and the mode text.
// A loud line: a run that wrote no still, a still all one colour, two arms whose pictures differ
// in under the floor's share of pixels (the same picture), a mode text that does not name a
// render flag the run passes, or a run that digs whose dig carved nothing (its `carved:` row).
// Exit 1 when any loud line fires, 2 on a usage error.

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import zlib from "node:zlib";

export const LOCK = "/home/igor/Work/voxel-engine/.git/svet-perf.lock";
export const RUNNER_PATTERN = "^bash .*live/session[0-9]+[a-z]?\\.sh";

// Flags whose sections are not render modes: the run, the session, the world, the model viewer,
// audio, loading and diagnostics. Their flags are never asked of the mode text.
const PLUMBING_SECTIONS = new Set([
  "Run",
  "Session",
  "World",
  "Model viewer and assets",
  "Audio",
  "Loading",
  "Diagnostics",
]);

// Flags in the mode sections that set the world, the clock or a reading, not a mode the banner
// or the HUD's mode rows name.
const NOT_MODES = new Set([
  "--spawn",
  "--trees",
  "--leaf-fill",
  "--sun",
  "--time",
  "--day-length",
  "--sun-radius",
  "--moon-radius",
  "--switch-at",
  "--spawn-hash",
  "--dump-black",
  "--band-census",
  "--lamp-toggle",
  "--entities",
  "--entities-walk",
  "--fade-reading",
  "--queue-census",
  "--queue-rule-reading",
  "--read-missing",
  "--settings",
]);

// Flags the rehearsal sets itself; a run's own copy is dropped.
const OWN_FLAGS = new Map([
  ["--headless", 0],
  ["--adapter", 1],
  ["--frames", 1],
  ["--still", 1],
  ["--mode-text", 1],
]);

// ---------------------------------------------------------------------------- parsing the runs

/** Splits a line into words: whitespace separates, single and double quotes group. */
export function tokenize(line) {
  const out = [];
  let word = null;
  let quote = null;
  for (const ch of line) {
    if (quote) {
      if (ch === quote) quote = null;
      else word += ch;
    } else if (ch === "'" || ch === '"') {
      quote = ch;
      word ??= "";
    } else if (/\s/.test(ch)) {
      if (word !== null) out.push(word);
      word = null;
    } else {
      word = (word ?? "") + ch;
    }
  }
  if (quote) throw new Error(`unclosed ${quote} in: ${line}`);
  if (word !== null) out.push(word);
  return out;
}

/** A run's name from a runner's label: its first word, without a trailing `.` or `:`. */
export function nameOf(label) {
  const first = (label.trim().split(/\s+/)[0] ?? "").replace(/[.:]+$/, "");
  return first.replace(/[^A-Za-z0-9_-]/g, "");
}

/** Gives each run a unique name: a missing or repeated one becomes `run<N>` (1-based). */
function uniqueNames(runs) {
  const seen = new Set();
  runs.forEach((r, i) => {
    if (!r.name || seen.has(r.name)) r.name = `run${i + 1}`;
    seen.add(r.name);
  });
  return runs;
}

/** The runs of a list file: `name flags.. [before=NAME] [dig=F,..]` a line. */
export function parseList(text) {
  const runs = [];
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const [name, ...words] = tokenize(line);
    const run = { name, label: name, program: null, args: [], before: null, dig: null };
    for (const w of words) {
      if (w.startsWith("before=")) run.before = w.slice(7);
      else if (w.startsWith("dig=")) run.dig = w.slice(4);
      else run.args.push(w);
    }
    runs.push(run);
  }
  return uniqueNames(runs);
}

/**
 * The runs the live-runner lib's recorder wrote (`LIVE_LIST`): per run, NUL-terminated, its
 * label, the count of words after the hint, then those words (the program, then its flags).
 */
export function parseRecords(buf) {
  const parts = buf.toString("utf8").split("\0");
  parts.pop();
  const runs = [];
  for (let i = 0; i < parts.length; ) {
    const label = parts[i];
    const n = Number(parts[i + 1]);
    if (!Number.isInteger(n) || i + 2 + n > parts.length) {
      throw new Error(`a broken record at ${i}: ${JSON.stringify(parts.slice(i, i + 3))}`);
    }
    const words = parts.slice(i + 2, i + 2 + n);
    const program = words[0] && !words[0].startsWith("--") ? words.shift() : null;
    runs.push({ name: nameOf(label), label, program, args: words, before: null, dig: null });
    i += 2 + n;
  }
  return uniqueNames(runs);
}

/** The runs of a runner script: runs it with `LIVE_LIST` set, so its `run` lines only record. */
export function readRunner(runner) {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "rehearse-"));
  const list = path.join(tmp, "runs");
  try {
    fs.writeFileSync(list, "");
    const r = spawnSync("bash", [runner], {
      env: { ...process.env, LIVE_LIST: list },
      stdio: ["ignore", "pipe", "pipe"],
      encoding: "utf8",
    });
    if (r.status !== 0) {
      throw new Error(`${runner} in list mode exited ${r.status}: ${(r.stderr || r.stdout).trim()}`);
    }
    return parseRecords(fs.readFileSync(list));
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
}

/** Applies `--before RUN=BEFORE` and `--dig RUN=F,..` pairs to the runs; throws on an unknown run. */
export function applyPairs(runs, key, pairs) {
  for (const p of pairs) {
    const [name, value] = p.split("=");
    const run = runs.find((r) => r.name === name);
    if (!run || !value) throw new Error(`--${key} ${p}: no run named ${name}, or no value`);
    run[key] = value;
  }
}

// ---------------------------------------------------------------------------- images

/** A channel level at or over which a pixel with all three counts as near white. */
export const NEAR_WHITE = 240;

/** A binary PPM (`P6`, maxval 255) as `{ width, height, data }`, `data` the RGB bytes. */
export function readPpm(buf) {
  let at = 0;
  const field = () => {
    while (at < buf.length) {
      const c = buf[at];
      if (c === 0x23) while (at < buf.length && buf[at] !== 0x0a) at++;
      else if (c === 0x20 || c === 0x0a || c === 0x0d || c === 0x09) at++;
      else break;
    }
    const from = at;
    while (at < buf.length && ![0x20, 0x0a, 0x0d, 0x09].includes(buf[at])) at++;
    return buf.subarray(from, at).toString("ascii");
  };
  if (field() !== "P6") throw new Error("not a binary PPM (P6)");
  const width = Number(field());
  const height = Number(field());
  if (Number(field()) !== 255) throw new Error("a PPM with a maxval other than 255");
  at++;
  const data = buf.subarray(at, at + width * height * 3);
  if (data.length !== width * height * 3) throw new Error("a short PPM");
  return { width, height, data };
}

function chunk(type, body) {
  const head = Buffer.alloc(8);
  head.writeUInt32BE(body.length, 0);
  head.write(type, 4, "ascii");
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(zlib.crc32(Buffer.concat([head.subarray(4), body])) >>> 0, 0);
  return Buffer.concat([head, body, crc]);
}

/** An RGB image as a PNG: 8-bit truecolour, no filter, one zlib stream. */
export function encodePng({ width, height, data }) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8;
  ihdr[9] = 2;
  const row = width * 3;
  const raw = Buffer.alloc((row + 1) * height);
  for (let y = 0; y < height; y++) {
    data.copy(raw, y * (row + 1) + 1, y * row, (y + 1) * row);
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", zlib.deflateSync(raw)),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

/**
 * A still's facts: all one colour, the share of pure black, of clipped white and of near white
 * (every channel at least `NEAR_WHITE`), the mean level (0..1).
 */
export function imageFacts({ width, height, data }) {
  const n = width * height;
  let black = 0;
  let white = 0;
  let near = 0;
  let sum = 0;
  let oneColour = true;
  for (let i = 0; i < n * 3; i += 3) {
    const [r, g, b] = [data[i], data[i + 1], data[i + 2]];
    if (r === 0 && g === 0 && b === 0) black++;
    if (r === 255 && g === 255 && b === 255) white++;
    if (r >= NEAR_WHITE && g >= NEAR_WHITE && b >= NEAR_WHITE) near++;
    sum += r + g + b;
    if (oneColour && (r !== data[0] || g !== data[1] || b !== data[2])) oneColour = false;
  }
  return { pixels: n, oneColour, black: black / n, white: white / n, nearWhite: near / n, mean: sum / (n * 3 * 255) };
}

/** The pixels of `a` and `b` (the same size) whose largest channel difference is over `tolerance`. */
export function diffFacts(a, b, tolerance) {
  if (a.width !== b.width || a.height !== b.height) return null;
  const n = a.width * a.height;
  let count = 0;
  for (let i = 0; i < n * 3; i += 3) {
    const d = Math.max(
      Math.abs(a.data[i] - b.data[i]),
      Math.abs(a.data[i + 1] - b.data[i + 1]),
      Math.abs(a.data[i + 2] - b.data[i + 2]),
    );
    if (d > tolerance) count++;
  }
  return { count, share: count / n };
}

// ---------------------------------------------------------------------------- the mode text

/** The flags of `--help`'s mode sections: every flag outside `PLUMBING_SECTIONS` and `NOT_MODES`. */
export function modeFlags(help) {
  const out = new Set();
  let section = "";
  for (const line of help.split("\n")) {
    const head = /^(\S.*):$/.exec(line);
    if (head) section = head[1];
    const flag = /^\s{2}(--[a-z0-9-]+)/.exec(line);
    if (flag && !PLUMBING_SECTIONS.has(section) && !NOT_MODES.has(flag[1])) out.add(flag[1]);
  }
  return out;
}

/**
 * The mode flags in `args` the mode text does not name: a flag is named when the text holds its
 * words (`--key-far` -> `key far`) and, when its value is a single word, that word too.
 */
export function unnamedFlags(args, text, checked) {
  const lower = text.toLowerCase();
  const missing = [];
  for (let i = 0; i < args.length; i++) {
    const flag = args[i];
    if (!flag.startsWith("--") || !checked.has(flag)) continue;
    const value = args[i + 1] && !args[i + 1].startsWith("--") ? args[i + 1] : null;
    const words = flag.slice(2).replaceAll("-", " ");
    const valueNamed = !value || !/^[a-z][a-z0-9_]*$/i.test(value) || lower.includes(value.toLowerCase());
    if (!lower.includes(words) || !valueNamed) missing.push(value ? `${flag} ${value}` : flag);
  }
  return missing;
}

/** The last `carved:` line of a mode text (the stats HUD's edit row), or null. */
export function carvedLine(text) {
  const lines = text.split("\n").filter((l) => l.startsWith("carved: "));
  return lines.at(-1) ?? null;
}

/** Whether a run digs: a `dig=` of the rehearsal's, or its own `--dig-at`. */
export const digs = (run) => !!run.dig || run.args.includes("--dig-at");

// ---------------------------------------------------------------------------- the sheet

const pct = (x) => `${(x * 100).toFixed(x < 0.01 && x > 0 ? 3 : 1)} %`;

/** The loud lines of one rehearsal's results. */
export function loudLines(results, floor) {
  const loud = [];
  const byName = new Map(results.map((r) => [r.name, r]));
  for (const r of results) {
    if (!r.image) {
      loud.push(`${r.name}: no still (${r.failure ?? "the run wrote none"})`);
      continue;
    }
    if (r.facts.oneColour) loud.push(`${r.name}: the still is all one colour`);
    if (r.unnamed?.length) loud.push(`${r.name}: the mode text does not name ${r.unnamed.join(", ")}`);
    if (digs(r) && !/^carved: (?!nothing)/.test(r.carved ?? "")) {
      loud.push(`${r.name}: the scripted dig carved nothing (${r.carved ?? "no carved line"})`);
    }
    if (r.before) {
      const b = byName.get(r.before);
      if (!b?.image) loud.push(`${r.name}: its before run ${r.before} has no still`);
      else if (!r.diff) loud.push(`${r.name}: a different size from its before run ${r.before}`);
      else if (r.diff.share < floor) {
        loud.push(
          `${r.name}: the same picture as ${r.before} (${r.diff.count} pixels, ${pct(r.diff.share)}, under the floor ${pct(floor)})`,
        );
      }
    }
  }
  return loud;
}

/** The sheet's markdown: the header, the loud lines, then per run its command, still, facts and mode text. */
export function sheet(meta, results, loud) {
  const out = [`# Window rehearsal`, ""];
  for (const [k, v] of Object.entries(meta)) out.push(`- ${k}: ${v}`);
  out.push("", "## Loud lines", "");
  if (loud.length) for (const l of loud) out.push(`- **LOUD** ${l}`);
  else out.push("none");
  for (const r of results) {
    out.push("", `## ${r.name}`, "", r.label !== r.name ? `${r.label}\n` : "", "```", r.command, "```", "");
    if (r.image) {
      out.push(`![${r.name}](${r.name}.png)`, "");
      const f = r.facts;
      out.push(
        `- ${f.oneColour ? "**all one colour**" : "not one colour"}; black ${pct(f.black)}; clipped white ${pct(f.white)}; near white ${pct(f.nearWhite)}; mean ${f.mean.toFixed(3)}; ${r.image.width} x ${r.image.height}`,
      );
      if (r.before) {
        out.push(
          r.diff
            ? `- against ${r.before}: ${r.diff.count} pixels differ (${pct(r.diff.share)})`
            : `- against ${r.before}: no comparison`,
        );
      }
      if (r.unnamed?.length) out.push(`- the mode text does not name: ${r.unnamed.join(", ")}`);
      if (digs(r)) out.push(`- the dig: ${r.carved ?? "no carved line"}`);
    } else {
      out.push(`- no still: ${r.failure ?? "the run wrote none"} (see ${r.name}.log)`);
    }
    if (r.text) {
      const shown = r.text.split("\nhud:\n")[0].trim();
      out.push("", "```text", shown, "```", `(the stats rows: ${r.name}.txt)`);
    }
  }
  return `${out.join("\n")}\n`;
}

// ---------------------------------------------------------------------------- running

/** The PIDs of this process and its ancestors. */
function ancestors() {
  const out = new Set();
  let pid = process.pid;
  while (pid > 1 && !out.has(pid)) {
    out.add(pid);
    try {
      const stat = fs.readFileSync(`/proc/${pid}/stat`, "utf8");
      pid = Number(stat.slice(stat.lastIndexOf(")") + 2).split(" ")[1]);
    } catch {
      break;
    }
  }
  return out;
}

/** A live-session runner that is up, other than one this process descends from, or null. */
export function liveRunner() {
  const r = spawnSync("pgrep", ["-af", RUNNER_PATTERN], { encoding: "utf8" });
  const mine = ancestors();
  const lines = (r.stdout || "").split("\n").filter(Boolean);
  return lines.find((l) => !mine.has(Number(l.split(" ")[0]))) ?? null;
}

/** The binary's flags for one run: its own minus the rehearsal's, then the rehearsal's. */
export function commandOf(run, opts, out) {
  const args = [];
  for (let i = 0; i < run.args.length; i++) {
    const a = run.args[i];
    if (OWN_FLAGS.has(a)) {
      i += OWN_FLAGS.get(a);
      continue;
    }
    args.push(a);
  }
  args.push("--headless", "--adapter", opts.adapter, "--frames", String(opts.frames));
  if (!args.includes("--render")) args.push("--render", opts.render);
  if (run.dig) {
    if (args.includes("--dig-at")) throw new Error(`${run.name}: dig= with the run's own --dig-at`);
    args.push("--dig-at", run.dig);
  }
  args.push("--still", path.join(out, `${run.name}.ppm`), "--mode-text", path.join(out, `${run.name}.txt`));
  return args;
}

const quote = (w) => (/^[A-Za-z0-9_./,:=@%+-]+$/.test(w) ? w : `'${w.replaceAll("'", "'\\''")}'`);

function rehearseOne(run, opts) {
  const bin = opts.bin ?? run.program;
  if (!bin) return { ...run, failure: "no binary (give --bin)" };
  const args = commandOf(run, opts, opts.out);
  const command = [bin, ...args].map(quote).join(" ");
  for (const ext of ["ppm", "txt"]) fs.rmSync(path.join(opts.out, `${run.name}.${ext}`), { force: true });
  const wrapped = [
    ...(opts.lock ? ["flock", opts.lock] : []),
    "timeout",
    String(opts.timeout),
    "systemd-run",
    "--user",
    "--scope",
    "--quiet",
    "-p",
    `MemoryMax=${opts.memory}`,
    "-p",
    "MemorySwapMax=1G",
    "--",
    bin,
    ...args,
  ];
  const t0 = Date.now();
  const r = spawnSync(wrapped[0], wrapped.slice(1), { encoding: "utf8", maxBuffer: 1 << 28 });
  const secs = ((Date.now() - t0) / 1000).toFixed(1);
  fs.writeFileSync(path.join(opts.out, `${run.name}.log`), `$ ${command}\n${r.stdout ?? ""}${r.stderr ?? ""}`);
  const result = { ...run, command, secs };
  const ppm = path.join(opts.out, `${run.name}.ppm`);
  const txt = path.join(opts.out, `${run.name}.txt`);
  if (fs.existsSync(txt)) result.text = fs.readFileSync(txt, "utf8");
  if (r.status !== 0) result.failure = `exit ${r.status ?? r.signal}`;
  if (fs.existsSync(ppm)) {
    result.image = readPpm(fs.readFileSync(ppm));
    fs.writeFileSync(path.join(opts.out, `${run.name}.png`), encodePng(result.image));
    fs.rmSync(ppm);
    result.facts = imageFacts(result.image);
  }
  if (result.text !== undefined) {
    result.unnamed = unnamedFlags(run.args, result.text, opts.checked);
    result.carved = carvedLine(result.text);
  }
  return result;
}

const USAGE = `usage: node scripts/rehearse-window.mjs --adapter NAME --out DIR (--runner FILE | --list FILE)
  [--bin PATH] [--frames N (60)] [--render WxH (640x360)] [--before RUN=BEFORE].. [--dig RUN=F,..]..
  [--floor SHARE (0.001)] [--tolerance LEVELS (2)] [--named-ok FLAG,..] [--timeout S (600)]
  [--memory SIZE (6G)] [--lock PATH (the perf lock)]`;

export function parseCli(argv) {
  const opts = {
    frames: 60,
    render: "640x360",
    floor: 0.001,
    tolerance: 2,
    timeout: 600,
    memory: "6G",
    lock: LOCK,
    before: [],
    dig: [],
    namedOk: [],
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const v = () => {
      if (i + 1 >= argv.length) throw new Error(`${a} takes a value`);
      return argv[++i];
    };
    switch (a) {
      case "--bin": opts.bin = v(); break;
      case "--adapter": opts.adapter = v(); break;
      case "--out": opts.out = v(); break;
      case "--runner": opts.runner = v(); break;
      case "--list": opts.list = v(); break;
      case "--frames": opts.frames = Number(v()); break;
      case "--render": opts.render = v(); break;
      case "--before": opts.before.push(v()); break;
      case "--dig": opts.dig.push(v()); break;
      case "--floor": opts.floor = Number(v()); break;
      case "--tolerance": opts.tolerance = Number(v()); break;
      case "--named-ok": opts.namedOk.push(...v().split(",")); break;
      case "--timeout": opts.timeout = Number(v()); break;
      case "--memory": opts.memory = v(); break;
      case "--lock": opts.lock = v(); break;
      case "-h":
      case "--help": opts.help = true; break;
      default: throw new Error(`unknown argument ${a}`);
    }
  }
  if (opts.help) return opts;
  if (!opts.adapter || !opts.out) throw new Error("--adapter and --out are required");
  if (!!opts.runner === !!opts.list) throw new Error("give one of --runner and --list");
  if (opts.list && !opts.bin) throw new Error("--list needs --bin");
  if (!(opts.frames >= 1) || !/^\d+x\d+$/.test(opts.render)) throw new Error("bad --frames or --render");
  return opts;
}

function main() {
  let opts;
  try {
    opts = parseCli(process.argv.slice(2));
  } catch (e) {
    console.error(`${e.message}\n${USAGE}`);
    return 2;
  }
  if (opts.help) {
    console.log(USAGE);
    return 0;
  }
  const up = liveRunner();
  if (up) {
    console.error(`a live-session runner is up (${up}); the rehearsal takes no GPU work while it runs`);
    return 2;
  }
  let runs;
  try {
    runs = opts.runner ? readRunner(opts.runner) : parseList(fs.readFileSync(opts.list, "utf8"));
    applyPairs(runs, "before", opts.before);
    applyPairs(runs, "dig", opts.dig);
    for (const r of runs) {
      if (r.before && !runs.some((o) => o.name === r.before)) throw new Error(`${r.name}: no run named ${r.before}`);
    }
  } catch (e) {
    console.error(e.message);
    return 2;
  }
  fs.mkdirSync(opts.out, { recursive: true });
  const bin = opts.bin ?? runs.find((r) => r.program)?.program;
  const help = spawnSync(bin, ["--help"], { encoding: "utf8" });
  if (help.status !== 0) {
    console.error(`${bin} --help failed: ${help.stderr}`);
    return 2;
  }
  opts.checked = modeFlags(help.stdout);
  for (const f of opts.namedOk) opts.checked.delete(f);

  const results = [];
  for (const run of runs) {
    const up = liveRunner();
    if (up) {
      console.error(`stopped before ${run.name}: a live-session runner came up (${up})`);
      return 2;
    }
    const r = rehearseOne(run, opts);
    console.log(`${run.name}: ${r.failure ?? "ok"} (${r.secs ?? 0} s with the lock wait)`);
    results.push(r);
  }
  const byName = new Map(results.map((r) => [r.name, r]));
  for (const r of results) {
    const b = r.before && byName.get(r.before);
    if (r.image && b?.image) r.diff = diffFacts(r.image, b.image, opts.tolerance);
  }
  const loud = loudLines(results, opts.floor);
  const meta = {
    binary: `${bin} ${fs.existsSync(`${bin}.commit`) ? fs.readFileSync(`${bin}.commit`, "utf8").trim() : ""}`.trim(),
    adapter: opts.adapter,
    frames: opts.frames,
    runs: opts.runner ?? opts.list,
    "same-picture floor": `${pct(opts.floor)} of the pixels, a pixel differing by over ${opts.tolerance} levels`,
    when: `${new Date().toISOString()}, load ${os.loadavg()[0].toFixed(2)}`,
  };
  fs.writeFileSync(path.join(opts.out, "index.md"), sheet(meta, results, loud));
  for (const l of loud) console.log(`LOUD ${l}`);
  console.log(`sheet: ${path.join(opts.out, "index.md")}`);
  return loud.length ? 1 : 0;
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  process.exitCode = main();
}
