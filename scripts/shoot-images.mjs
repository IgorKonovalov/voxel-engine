#!/usr/bin/env node
// Re-renders the images under docs/images/ that docs/images/shots.json describes, headless,
// in batches inside holds of the GPU lock, and encodes each with ffmpeg after its hold: a
// still as lossy WebP (libwebp), a loop as animated WebP (libwebp_anim), both scaled to the
// entry's width with lanczos.
//
//   node scripts/shoot-images.mjs [--only ID,..] [--print ID [--candidate N]] [--check]
//                                 [--candidates] [--no-build]
//
// An entry's `source` is `flythrough` (the testbed binary with the entry's `flags`, plus
// `--seed`, `--still-at` and `--still-dir`; a loop's `still_at` is a frame range `A..B`) or
// `clip` (the fly-through's `clip` example with the entry's `flags`, plus `--fps` and
// `--out-dir`). The frames land under target/shots/<id>/. Every command is derived from the
// entry by `commandsOf`, so `--print` and the shoot cannot disagree. `--check` renders and
// encodes into target/shots/check/ instead, decodes the new and the committed file to raw RGB
// with ffmpeg, cuts every frame into CELL x CELL cells and prints the share of cells whose
// largest channel mean differs by more than CELL_LEVEL, against the entry's `tolerance`; it
// exits 1 when one is over. An entry marked `unjudged` (its noise set broke its ceiling, so its
// `tolerance` is null) is rendered and its share printed, marked unjudged, and never counts
// towards the exit code. Encoder noise and a one-pixel move average out inside a cell; a
// changed scene, pass or hour moves whole cells. The `frozen` map names images that are kept
// and never re-rendered.
//
// Candidates: an entry may carry `candidates`, 2 to 9 flag sets (a list of flags, or
// `{ "flags": [..], "still_at": .. }`). Candidate N is the entry with its flags merged over the
// entry's own by `mergeFlags` (a flag the entry has takes the candidate's value; a new one is
// appended), so it differs from the entry only in what it names. `--candidates` renders every
// candidate into target/shots/candidates/<id>/<N>/, keeps its tile as <id>/<N>.png (a still's
// frame, a loop's middle frame; a loop also as a 2-second <id>/<N>.webp), and tiles each entry's
// candidates, labelled with their number and flags, into <id>.png (3 columns of 640 px) with a
// legend <id>.txt of one line per tile: the number, the flags and the bare render command.
// index.html beside them shows every sheet's tiles with their commands. `--print ID --candidate
// N` prints the commands that make tile N alone.
//
// The lock: the script takes it once per batch (it runs itself under `flock` with the hidden
// `--batch ID,..`), renders entry after entry inside the hold and starts no render once the
// hold is HOLD_S old; a render's timeout is TIMEOUT_S, so no hold passes HOLD_S + TIMEOUT_S.
// The live-runner check runs before every hold.

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { LOCK, liveRunner } from "./rehearse-window.mjs";

export const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
export const MANIFEST = "docs/images/shots.json";
export const IMAGES = "docs/images";
export const SHOTS = "target/shots";
export const BINARIES = { flythrough: "target/release/flythrough", clip: "target/release/examples/clip" };
/** A pixel differs when one of its channels differs by more than this many levels of 255
 * (`diffFraction`, the PNG reading; `--check` reads cells). */
export const LEVEL = 2;
/** `--check`'s cell side in pixels of the decoded file. */
export const CELL = 8;
/** `--check`'s cell difference: a cell differs when one channel's mean moves by more than this. */
export const CELL_LEVEL = 4;
/** No render starts once a hold of the lock is this old (seconds). */
export const HOLD_S = 600;
/** One render's timeout (seconds). */
export const TIMEOUT_S = 300;
const MEMORY = "6G";
/** Where `--candidates` writes, and its sheets' shape. */
export const CANDIDATES = `${SHOTS}/candidates`;
export const TILE_W = 640;
export const COLUMNS = 3;
/** A loop candidate's length in seconds at the entry's fps. */
export const CANDIDATE_LOOP_S = 2;

/** The fields every entry carries, and the ones its kind or source adds. */
export const REQUIRED = ["kind", "file", "source", "flags", "width", "quality", "tolerance", "cap_bytes", "note"];
const KINDS = { still: ["still_at"], loop: ["fps"] };
const SOURCES = ["flythrough", "clip"];
/** Flags the script adds itself; an entry naming one would run twice or fight it. */
const OWN = {
  flythrough: ["--still-at", "--still-dir", "--still", "--mode-text"],
  clip: ["--fps", "--out-dir"],
};

/** The manifest's problems, one line each; empty when it is well formed. */
export function validate(manifest) {
  const errors = [];
  if (!manifest || typeof manifest.shots !== "object") return ["no `shots` object"];
  if (manifest.frozen !== undefined && typeof manifest.frozen !== "object") errors.push("`frozen` is not an object");
  const files = new Map();
  for (const [id, e] of Object.entries(manifest.shots)) {
    if (!/^[a-z0-9-]+$/.test(id)) errors.push(`${id}: an id is lowercase letters, digits and hyphens`);
    for (const f of [...REQUIRED, ...(KINDS[e.kind] ?? [])]) {
      if (e[f] === undefined) errors.push(`${id}: no \`${f}\``);
    }
    if (!KINDS[e.kind]) errors.push(`${id}: kind ${e.kind} is not still or loop`);
    if (!SOURCES.includes(e.source)) errors.push(`${id}: source ${e.source} is not flythrough or clip`);
    if (!Array.isArray(e.flags) || e.flags.some((f) => typeof f !== "string")) errors.push(`${id}: flags is not a list of strings`);
    else for (const f of OWN[e.source] ?? []) if (e.flags.includes(f)) errors.push(`${id}: flags name ${f}, which the script adds`);
    if (typeof e.file === "string") {
      if (!/^[a-z0-9-]+\.webp$/.test(e.file)) errors.push(`${id}: file ${e.file} is not <name>.webp`);
      if (files.has(e.file)) errors.push(`${id}: file ${e.file} is also ${files.get(e.file)}'s`);
      files.set(e.file, id);
      if (manifest.frozen?.[e.file] !== undefined) errors.push(`${id}: file ${e.file} is also frozen`);
    }
    // A null tolerance is a draft's (no noise set read yet; `--check` fails it) or an unjudged
    // entry's (its set broke its ceiling; `--check` prints its share and does not count it).
    const unjudged = e.unjudged !== undefined;
    if (unjudged && !(typeof e.unjudged === "string" && e.unjudged.length > 0)) {
      errors.push(`${id}: unjudged is not a line (the largest pair, the smallest wrong share, the date)`);
    }
    if (unjudged && e.tolerance !== null) errors.push(`${id}: an unjudged entry's tolerance is null`);
    else if (!((e.draft === true || unjudged) && e.tolerance === null) && !(typeof e.tolerance === "number" && e.tolerance >= 0 && e.tolerance <= 1)) {
      errors.push(`${id}: tolerance is not a fraction in 0..1${e.draft === true ? " or null" : ""}`);
    }
    if (e.candidates !== undefined) errors.push(...validateCandidates(id, e));
    if (!(Number.isInteger(e.width) && e.width > 0)) errors.push(`${id}: width is not a positive integer`);
    if (!(Number.isInteger(e.quality) && e.quality >= 0 && e.quality <= 100)) errors.push(`${id}: quality is not 0..100`);
    if (!(Number.isInteger(e.cap_bytes) && e.cap_bytes > 0)) errors.push(`${id}: cap_bytes is not a positive integer`);
    if (e.source === "flythrough") {
      if (e.kind === "still" && !Number.isInteger(e.still_at)) errors.push(`${id}: a still's still_at is one frame`);
      if (e.kind === "loop" && !/^\d+\.\.\d+$/.test(String(e.still_at))) errors.push(`${id}: a loop's still_at is A..B`);
      if (!e.flags?.includes("--headless")) errors.push(`${id}: a flythrough entry runs --headless`);
      if (!e.flags?.includes("--settled-start")) errors.push(`${id}: a flythrough entry runs --settled-start`);
    }
    if (e.source === "clip" && e.kind !== "loop") errors.push(`${id}: a clip entry is a loop`);
  }
  return errors;
}

/** An entry's `candidates` problems, one line each. */
function validateCandidates(id, e) {
  const errors = [];
  const c = e.candidates;
  if (!Array.isArray(c) || c.length < 2 || c.length > 9) return [`${id}: candidates is not a list of 2 to 9`];
  c.forEach((cand, i) => {
    const n = i + 1;
    const flags = Array.isArray(cand) ? cand : cand?.flags;
    if (!Array.isArray(flags) || flags.some((f) => typeof f !== "string")) {
      errors.push(`${id}: candidate ${n} is not a list of flags or { flags, still_at }`);
      return;
    }
    if (flags.length > 0 && !flags[0].startsWith("--")) errors.push(`${id}: candidate ${n} starts with a value, not a flag`);
    for (const f of OWN[e.source] ?? []) if (flags.includes(f)) errors.push(`${id}: candidate ${n} names ${f}, which the script adds`);
    if (!Array.isArray(cand) && cand.still_at !== undefined) {
      if (e.source !== "flythrough") errors.push(`${id}: candidate ${n}: still_at is a flythrough entry's`);
      else if (e.kind === "still" && !Number.isInteger(cand.still_at)) errors.push(`${id}: candidate ${n}: a still's still_at is one frame`);
      else if (e.kind === "loop" && !/^\d+\.\.\d+$/.test(String(cand.still_at))) errors.push(`${id}: candidate ${n}: a loop's still_at is A..B`);
    }
  });
  return errors;
}

/**
 * `over`'s flags merged over `base`: a flag `base` has takes `over`'s value in its place, a flag
 * it lacks is appended (with its value), a bare flag it has stays once. A token after a flag is
 * its value unless it starts with `--`.
 */
export function mergeFlags(base, over) {
  const out = [...base];
  const takesValue = (list, i) => i + 1 < list.length && !list[i + 1].startsWith("--");
  for (let i = 0; i < over.length; i++) {
    const f = over[i];
    const j = out.indexOf(f);
    if (takesValue(over, i)) {
      const v = over[++i];
      if (j < 0) out.push(f, v);
      else if (takesValue(out, j)) out[j + 1] = v;
      else out.splice(j + 1, 0, v);
    } else if (j < 0) out.push(f);
  }
  return out;
}

/**
 * Candidate `n` (from 1) of an entry, as an entry of its own: the candidate's flags merged over
 * the entry's, its `still_at` if it names one. A loop's frames are cut to its first
 * CANDIDATE_LOOP_S seconds at the entry's fps.
 */
export function candidateOf(entry, n) {
  const cand = entry.candidates?.[n - 1];
  if (cand === undefined) throw new Error(`no candidate ${n}: the entry has ${entry.candidates?.length ?? 0}`);
  const flags = Array.isArray(cand) ? cand : cand.flags;
  const c = { ...entry, flags: mergeFlags(entry.flags, flags) };
  delete c.candidates;
  if (!Array.isArray(cand) && cand.still_at !== undefined) c.still_at = cand.still_at;
  if (c.kind === "loop" && c.source === "flythrough") {
    const [a, b] = String(c.still_at).split("..").map(Number);
    c.still_at = `${a}..${Math.min(b, a + CANDIDATE_LOOP_S * c.fps - 1)}`;
  }
  return c;
}

/**
 * The commands that make tile `n` of entry `id`: `clear` and `render` as `commandsOf`'s into
 * `<id>/<n>/`, `tile` writes `<id>/<n>.png` (a still's frame, a loop's middle frame), and for a
 * loop `encode` writes the 2-second `<id>/<n>.webp`. `bare` is the render without the lock,
 * the timeout and the scope: the command a person runs.
 */
export function candidateCommands(id, entry, n, wrapOpts = {}) {
  const c = candidateOf(entry, n);
  const dir = `${CANDIDATES}/${id}/${n}`;
  const cmds = commandsOf(id, c, { dir, out: `${dir}.webp`, ...wrapOpts });
  const loopFrames = c.kind === "loop" ? CANDIDATE_LOOP_S * c.fps : 1;
  const middle = c.kind === "loop" ? Math.floor(loopFrames / 2) : 0;
  const input = ["-pattern_type", "glob", "-i", `${dir}/*.png`];
  const tile = ["ffmpeg", "-v", "error", "-y", ...input, "-vf", `select=eq(n\\,${middle})`, "-frames:v", "1", `${dir}.png`];
  const encode = c.kind === "loop" ? [...cmds.encode.slice(0, -1), "-frames:v", String(loopFrames), `${dir}.webp`] : null;
  return { dir, tilePng: `${dir}.png`, clear: cmds.clear, render: cmds.render, bare: cmds.bare, tile, encode, entry: c };
}

/** The frames a flythrough entry writes: its one still, or every frame of its range. */
export function framesOf(entry) {
  if (entry.kind === "still") return [entry.still_at];
  const [a, b] = String(entry.still_at).split("..").map(Number);
  if (!(b >= a)) throw new Error(`still_at ${entry.still_at}: B is before A`);
  return Array.from({ length: b - a + 1 }, (_, i) => a + i);
}

/** The memory-capped scope with a timeout, inside the GPU lock unless `lock` is null (a batch
 * already holds it). */
export function wrap(argv, { lock = LOCK, memory = MEMORY, timeout = TIMEOUT_S } = {}) {
  return [
    ...(lock ? ["flock", lock] : []), "timeout", String(timeout),
    "systemd-run", "--user", "--scope", "--quiet", "-p", `MemoryMax=${memory}`, "-p", "MemorySwapMax=2G", "--",
    ...argv,
  ];
}

/**
 * The commands that make entry `id`, run from the repository root: `clear` empties its frame
 * directory, `render` writes the frames (wrapped), `encode` makes `out` from them.
 */
export function commandsOf(id, entry, { check = false, dir: dirAt, out: outAt, ...wrapOpts } = {}) {
  const dir = dirAt ?? (check ? `${SHOTS}/check/${id}` : `${SHOTS}/${id}`);
  const out = outAt ?? (check ? `${SHOTS}/check/${entry.file}` : `${IMAGES}/${entry.file}`);
  let argv;
  if (entry.source === "flythrough") {
    argv = [BINARIES.flythrough, ...entry.flags];
    if (entry.seed !== undefined && !entry.flags.includes("--seed")) argv.push("--seed", String(entry.seed));
    argv.push("--still-at", framesOf(entry).join(","), "--still-dir", dir);
  } else {
    argv = [BINARIES.clip, ...entry.flags];
    if (entry.seed !== undefined && !entry.flags.includes("--seed")) argv.push("--seed", String(entry.seed));
    argv.push("--fps", String(entry.fps), "--out-dir", dir);
  }
  const scale = `scale=${entry.width}:-1:flags=lanczos`;
  const input = ["-pattern_type", "glob", "-i", `${dir}/*.png`];
  const encode =
    entry.kind === "still"
      ? ["ffmpeg", "-v", "error", "-y", ...input, "-frames:v", "1", "-vf", scale,
          "-c:v", "libwebp", "-quality", String(entry.quality), "-compression_level", "6", out]
      : ["ffmpeg", "-v", "error", "-y", "-framerate", String(entry.fps), ...input, "-vf", scale,
          "-c:v", "libwebp_anim", "-quality", String(entry.quality), "-compression_level", "6", "-loop", "0", out];
  return { dir, out, clear: ["rm", "-rf", dir], render: wrap(argv, wrapOpts), bare: argv, encode };
}

const quote = (w) => (/^[A-Za-z0-9_./,:=@%+-]+$/.test(w) ? w : `'${w.replaceAll("'", "'\\''")}'`);
export const shell = (argv) => argv.map(quote).join(" ");

/** `--print ID`'s lines: the three commands, copy-pasteable from the repository root. */
export function printLines(id, entry, candidate = null) {
  if (candidate !== null) {
    const c = candidateCommands(id, entry, candidate);
    return [`# ${id} candidate ${candidate}: from the repository root`, shell(c.clear), `mkdir -p ${c.dir}`,
      shell(c.render), shell(c.tile), ...(c.encode ? [shell(c.encode)] : [])];
  }
  const c = commandsOf(id, entry);
  return [`# ${id}: from the repository root`, shell(c.clear), shell(c.render), shell(c.encode)];
}

/** A file decoded by ffmpeg to raw RGB, every frame of a loop in order. */
export function decodeRgb(file) {
  const r = spawnSync(
    "ffmpeg",
    ["-v", "error", "-i", file, "-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"],
    { maxBuffer: 1 << 30 },
  );
  if (r.status !== 0) throw new Error(`ffmpeg could not decode ${file}: ${r.stderr}`);
  return r.stdout;
}

/** A decoded file's frame size, by ffprobe. */
export function probeSize(file) {
  const r = spawnSync(
    "ffprobe",
    ["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height", "-of", "csv=p=0", file],
    { encoding: "utf8" },
  );
  const [w, h] = (r.stdout ?? "").trim().split(",").map(Number);
  if (r.status !== 0 || !(w > 0 && h > 0)) throw new Error(`ffprobe could not size ${file}: ${r.stderr}`);
  return { width: w, height: h };
}

/**
 * The share of cells of two RGB buffers (frames of `width` x `height`, every frame in order)
 * whose largest channel mean differs by more than `level`. Cells are `cell` x `cell` from the
 * top left; the right and bottom ones are smaller, and each counts once. `max` is the largest
 * cell difference. `worst` is the frame with the largest share of its own cells over `level`
 * (the first of equals) and that share, or null when no cell is over.
 */
export function cellShare(a, b, width, height, { cell = CELL, level = CELL_LEVEL } = {}) {
  const fb = width * height * 3;
  if (a.length !== b.length) return { share: 1, max: 255, note: `sizes differ: ${a.length} and ${b.length} bytes` };
  if (fb === 0 || a.length % fb !== 0) return { share: 1, max: 255, note: `${a.length} bytes are not frames of ${width} x ${height}` };
  let cells = 0;
  let over = 0;
  let max = 0;
  let worst = null;
  for (let f = 0; f < a.length / fb; f++) {
    const cells0 = cells;
    const over0 = over;
    for (let y0 = 0; y0 < height; y0 += cell) {
      for (let x0 = 0; x0 < width; x0 += cell) {
        const y1 = Math.min(y0 + cell, height);
        const x1 = Math.min(x0 + cell, width);
        const d = [0, 0, 0];
        for (let y = y0; y < y1; y++) {
          for (let x = x0, i = f * fb + (y * width + x0) * 3; x < x1; x++, i += 3) {
            d[0] += a[i] - b[i];
            d[1] += a[i + 1] - b[i + 1];
            d[2] += a[i + 2] - b[i + 2];
          }
        }
        const diff = Math.max(...d.map(Math.abs)) / ((y1 - y0) * (x1 - x0));
        cells++;
        if (diff > level) over++;
        if (diff > max) max = diff;
      }
    }
    const share = (over - over0) / (cells - cells0);
    if (over > over0 && (worst === null || share > worst.share)) worst = { frame: f, share };
  }
  return { share: over / cells, max, worst };
}

/**
 * `--check`'s judgment of one entry from `d`, the cell share of its new file against the
 * committed one (`cellShare`'s result; null when there is nothing to read). `fails` counts
 * towards exit 1. A draft with no tolerance fails unread. An unjudged entry (its noise set broke
 * its ceiling, so it has no tolerance) prints its share marked unjudged and never counts, however
 * far apart its files read.
 */
export function verdict(entry, d) {
  if (entry.unjudged === undefined && entry.tolerance === null) {
    return { fails: true, unjudged: false, text: "; no tolerance (a draft whose noise set is not read)" };
  }
  let text = `; cell share ${d.share.toFixed(6)} (C ${CELL}, L ${CELL_LEVEL}, largest ${d.max.toFixed(2)}) `;
  let fails = false;
  if (entry.unjudged !== undefined) text += `UNJUDGED, not counted (${entry.unjudged})`;
  else {
    fails = d.share > entry.tolerance;
    text += `tolerance ${entry.tolerance} ${fails ? "OVER" : "ok"}`;
  }
  if (d.note) text += ` (${d.note})`;
  // A loop's worst frame is a reading beside the share, not judged itself.
  if (entry.kind === "loop" && !d.note) {
    text += d.worst ? `; worst frame ${d.worst.frame} at ${d.worst.share.toFixed(6)} (unjudged)` : "; worst frame none";
  }
  return { fails, unjudged: entry.unjudged !== undefined, text };
}

/**
 * One hold's renders: `render` each unit in order, starting none once `now()` (seconds) is
 * `limit` past the hold's start. Returns what the renders returned; the units after them wait
 * for the next hold.
 */
export function batchOf(units, render, now, limit = HOLD_S) {
  const start = now();
  const done = [];
  for (const u of units) {
    if (now() - start >= limit) break;
    done.push(render(u));
  }
  return done;
}

/** The share of pixels of two RGB buffers whose largest channel difference is over `level`. */
export function diffFraction(a, b, level = LEVEL) {
  if (a.length !== b.length) return { fraction: 1, note: `sizes differ: ${a.length} and ${b.length} bytes` };
  let over = 0;
  for (let i = 0; i < a.length; i += 3) {
    const d = Math.max(Math.abs(a[i] - b[i]), Math.abs(a[i + 1] - b[i + 1]), Math.abs(a[i + 2] - b[i + 2]));
    if (d > level) over++;
  }
  return { fraction: over / (a.length / 3) };
}

export function readManifest(file) {
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

const USAGE = `usage: node scripts/shoot-images.mjs [--only ID,..] [--print ID] [--check] [--no-build] [--manifest FILE]
  --only ID,..   shoot only these entries
  --print ID     print the entry's commands (copy-pasteable from the repository root) and exit
  --check        render and encode into ${SHOTS}/check/, print each entry's cell share (cells of
                 ${CELL} x ${CELL}, a channel mean more than ${CELL_LEVEL} apart) against the
                 committed file and its tolerance; exit 1 when one is over (an unjudged
                 entry's share is printed and not counted)
  --candidates   render every candidate of the entries that have them into ${CANDIDATES}/ and
                 tile each entry's into <id>.png with a legend <id>.txt; writes index.html
  --candidate N  with --print: the commands that make candidate N's tile alone
  --no-build     use the release binaries as already built
  --manifest F   read F instead of ${MANIFEST}`;

export function parseCli(argv) {
  const o = {
    only: null, print: null, check: false, build: true, manifest: MANIFEST, help: false, batch: null,
    candidates: false, candidate: null,
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const val = () => {
      if (i + 1 >= argv.length) throw new Error(`${a} needs a value`);
      return argv[++i];
    };
    if (a === "--only") o.only = val().split(",").filter(Boolean);
    else if (a === "--print") o.print = val();
    else if (a === "--check") o.check = true;
    else if (a === "--candidates") o.candidates = true;
    else if (a === "--candidate") {
      const v = val();
      if (!/^[1-9]$/.test(v)) throw new Error(`--candidate ${v} is not 1 to 9`);
      o.candidate = Number(v);
    }
    else if (a === "--no-build") o.build = false;
    else if (a === "--manifest") o.manifest = val();
    else if (a === "--batch") o.batch = val().split(",").filter(Boolean);
    else if (a === "-h" || a === "--help") o.help = true;
    else throw new Error(`unknown option ${a}`);
  }
  if (o.candidate !== null && o.print === null) throw new Error("--candidate goes with --print ID");
  if (o.candidates && o.check) throw new Error("--candidates and --check are two runs");
  return o;
}

function run(argv, opts = {}) {
  return spawnSync(argv[0], argv.slice(1), { cwd: ROOT, encoding: "utf8", maxBuffer: 1 << 28, ...opts });
}

const BATCH_TAG = "shoot-batch ";

/** A render unit is an entry's id, or `id:N` for its candidate N. */
export function unitCommands(unit, manifest, opts = {}) {
  const [id, n] = unit.split(":");
  return n === undefined ? commandsOf(id, manifest.shots[id], opts) : candidateCommands(id, manifest.shots[id], Number(n), opts);
}

/** The units `--candidates` renders: every candidate of every listed entry that has them. */
export function candidateUnits(ids, manifest) {
  return ids.flatMap((id) => (manifest.shots[id].candidates ?? []).map((_, i) => `${id}:${i + 1}`));
}

/** Words of `text` broken into lines of at most `width` characters. */
export function wrapText(text, width) {
  const lines = [];
  let line = "";
  for (const w of text.split(" ")) {
    if (line && line.length + 1 + w.length > width) {
      lines.push(line);
      line = w;
    } else line = line ? `${line} ${w}` : w;
  }
  if (line) lines.push(line);
  return lines;
}

/** The candidate's flags as the legend and the label print them. */
export function candidateLabel(entry, n) {
  const cand = entry.candidates[n - 1];
  const flags = Array.isArray(cand) ? cand : cand.flags;
  const at = !Array.isArray(cand) && cand.still_at !== undefined ? ` (frame ${cand.still_at})` : "";
  return (flags.length ? flags.join(" ") : "(the entry's flags)") + at;
}

const LABEL_H = 64;
const FONT_SIZE = 16;

function fontFile() {
  const r = spawnSync("fc-match", ["-f", "%{file}", "monospace"], { encoding: "utf8" });
  return r.status === 0 && r.stdout ? r.stdout : null;
}

/**
 * After the renders: one labelled tile per candidate (a failed one is a dark tile that says
 * so, so numbers and lines stay aligned), the sheet `<id>.png` tiled by ffmpeg and the legend
 * `<id>.txt`. Returns the sheet's path, its tile count and the legend's line count.
 */
function makeSheet(id, entry, failures) {
  const base = path.join(ROOT, CANDIDATES, id);
  const tiles = path.join(base, "tiles");
  fs.rmSync(tiles, { recursive: true, force: true });
  fs.mkdirSync(tiles, { recursive: true });
  const font = fontFile();
  const fontArg = font ? `fontfile=${font.replaceAll(":", "\\:")}:` : "";
  const k = entry.candidates.length;
  let size = null;
  for (let n = 1; n <= k; n++) {
    const png = path.join(ROOT, candidateCommands(id, entry, n).tilePng);
    if (fs.existsSync(png) && !failures.has(n)) {
      size = probeSize(png);
      break;
    }
  }
  const tileH = size ? Math.round((TILE_W * size.height) / size.width / 2) * 2 : 360;
  const legend = [];
  for (let n = 1; n <= k; n++) {
    const c = candidateCommands(id, entry, n);
    const png = path.join(ROOT, c.tilePng);
    const failed = failures.get(n) ?? (fs.existsSync(png) ? null : "not rendered");
    const label = candidateLabel(entry, n);
    legend.push(`${n}  ${label}${failed ? `  [FAILED: ${failed}]` : ""}  ::  ${shell(c.bare)}`);
    const textFile = path.join(tiles, `${n}.txt`);
    fs.writeFileSync(textFile, [`${id} ${n}${failed ? "  FAILED" : ""}`, ...wrapText(label, 60)].slice(0, 3).join("\n"));
    const draw = `drawtext=${fontArg}textfile=${textFile}:x=8:y=h-${LABEL_H}+6:fontsize=${FONT_SIZE}:fontcolor=white:line_spacing=3:expansion=none`;
    const out = path.join(tiles, `${String(n).padStart(2, "0")}.png`);
    const args = failed
      ? ["-f", "lavfi", "-i", `color=c=0x3a1010:s=${TILE_W}x${tileH}`, "-vf", `pad=iw:ih+${LABEL_H}:0:0:color=0x202020,${draw}`]
      : ["-i", png, "-vf", `scale=${TILE_W}:${tileH}:flags=lanczos,pad=iw:ih+${LABEL_H}:0:0:color=0x202020,${draw}`];
    const r = run(["ffmpeg", "-v", "error", "-y", ...args, "-frames:v", "1", out]);
    if (r.status !== 0) throw new Error(`ffmpeg could not label ${id} tile ${n}: ${r.stderr}`);
  }
  const rows = Math.ceil(k / COLUMNS);
  const sheet = path.join(ROOT, CANDIDATES, `${id}.png`);
  const r = run(["ffmpeg", "-v", "error", "-y", "-pattern_type", "glob", "-i", `${tiles}/*.png`,
    "-vf", `tile=${COLUMNS}x${rows}:padding=4:color=black`, "-frames:v", "1", sheet]);
  if (r.status !== 0) throw new Error(`ffmpeg could not tile ${id}: ${r.stderr}`);
  fs.writeFileSync(path.join(ROOT, CANDIDATES, `${id}.txt`), legend.join("\n") + "\n");
  const tileCount = fs.readdirSync(tiles).filter((f) => f.endsWith(".png")).length;
  return { sheet: path.relative(ROOT, sheet), tiles: tileCount, lines: legend.length };
}

const esc = (s) => String(s).replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;");

/** index.html: every entry with candidates on disk, each tile with its number, flags and command. */
function writeIndex(manifest) {
  const parts = [];
  for (const [id, entry] of Object.entries(manifest.shots)) {
    if (!entry.candidates || !fs.existsSync(path.join(ROOT, CANDIDATES, `${id}.txt`))) continue;
    const cells = entry.candidates.map((_, i) => {
      const n = i + 1;
      const c = candidateCommands(id, entry, n);
      const rel = (p) => path.relative(CANDIDATES, p);
      const media = entry.kind === "loop" && fs.existsSync(path.join(ROOT, `${c.dir}.webp`))
        ? `<a href="${rel(`${c.dir}.webp`)}"><img src="${rel(`${c.dir}.webp`)}" alt="${id} ${n}"></a>`
        : `<a href="${rel(c.tilePng)}"><img src="${rel(c.tilePng)}" alt="${id} ${n}"></a>`;
      return `<figure>${media}<figcaption><b>${id} ${n}</b> ${esc(candidateLabel(entry, n))}` +
        `<code>${esc(shell(c.bare))}</code><code>node scripts/shoot-images.mjs --print ${id} --candidate ${n}</code></figcaption></figure>`;
    });
    parts.push(`<section id="${id}"><h2>${id} <small>${entry.kind}</small></h2>` +
      `<p>${esc(entry.shows ?? "")}</p><div class="grid">${cells.join("")}</div></section>`);
  }
  const html = `<!doctype html><meta charset="utf-8"><title>Shot candidates</title>
<style>body{font:14px/1.4 sans-serif;background:#181818;color:#ddd;margin:16px}
.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(480px,1fr));gap:12px}
img{width:100%;display:block}figure{margin:0}code{display:block;font-size:11px;color:#9c9;word-break:break-all;margin-top:4px}
a{color:#8af}h2 small{color:#888;font-weight:normal}</style>
<h1>Shot candidates</h1><p>${Object.keys(manifest.shots).filter((id) => manifest.shots[id].candidates).map((id) => `<a href="#${id}">${id}</a>`).join(" &middot; ")}</p>
${parts.join("\n")}\n`;
  fs.writeFileSync(path.join(ROOT, CANDIDATES, "index.html"), html);
  return `${CANDIDATES}/index.html`;
}

/** Inside a hold (`--batch`): render the entries in order until the hold is HOLD_S old, and
 * print one line for the parent: the hold's start and end, its load and each render. */
function renderBatch(o, manifest) {
  const clock = () => Date.now() / 1000;
  const start = clock();
  const load = os.loadavg()[0];
  const results = batchOf(o.batch, (id) => {
    const c = unitCommands(id, manifest, { check: o.check, lock: null });
    run(c.clear);
    fs.mkdirSync(path.join(ROOT, c.dir), { recursive: true });
    const t0 = clock();
    const r = run(c.render);
    fs.writeFileSync(path.join(ROOT, `${c.dir}.log`), `$ ${shell(c.render)}\n${r.stdout ?? ""}${r.stderr ?? ""}`);
    return { id, status: r.status ?? String(r.signal), secs: clock() - t0 };
  }, clock);
  console.log(BATCH_TAG + JSON.stringify({ start, end: clock(), load, results }));
  return 0;
}

/** After the hold: a candidate's tile and, for a loop, its 2-second WebP; the failure, or null. */
function finishCandidate(id, entry, n, r) {
  const c = candidateCommands(id, entry, n);
  fs.rmSync(path.join(ROOT, c.tilePng), { force: true });
  if (r.status !== 0) return `render exit ${r.status}, see ${c.dir}.log`;
  const pngs = fs.existsSync(path.join(ROOT, c.dir)) ? fs.readdirSync(path.join(ROOT, c.dir)).filter((f) => f.endsWith(".png")) : [];
  const want = c.entry.source === "flythrough" ? framesOf(c.entry).length : null;
  if (pngs.length === 0 || (want !== null && pngs.length !== want)) return `${pngs.length} frames written, ${want ?? "some"} expected, see ${c.dir}.log`;
  const t = run(c.tile);
  if (t.status !== 0 || !fs.existsSync(path.join(ROOT, c.tilePng))) return `tile exit ${t.status}: ${t.stderr}`;
  if (c.encode) {
    const e = run(c.encode);
    if (e.status !== 0) return `encode exit ${e.status}: ${e.stderr}`;
  }
  return null;
}

/** After the hold: check the frames a render wrote and encode them. */
function encodeOne(id, entry, check) {
  const c = commandsOf(id, entry, { check });
  const log = path.relative(ROOT, path.join(ROOT, `${c.dir}.log`));
  const pngs = fs.existsSync(path.join(ROOT, c.dir)) ? fs.readdirSync(path.join(ROOT, c.dir)).filter((f) => f.endsWith(".png")) : [];
  const want = entry.source === "flythrough" ? framesOf(entry).length : null;
  if (pngs.length === 0 || (want !== null && pngs.length !== want)) {
    return { failure: `${pngs.length} frames written, ${want ?? "some"} expected, see ${log}` };
  }
  fs.mkdirSync(path.dirname(path.join(ROOT, c.out)), { recursive: true });
  const e = run(c.encode);
  if (e.status !== 0) return { failure: `encode exit ${e.status}: ${e.stderr}` };
  return { out: c.out, frames: pngs.length, bytes: fs.statSync(path.join(ROOT, c.out)).size };
}

function main() {
  let o;
  try {
    o = parseCli(process.argv.slice(2));
  } catch (e) {
    console.error(`${e.message}\n${USAGE}`);
    return 2;
  }
  if (o.help) {
    console.log(USAGE);
    return 0;
  }
  let manifest;
  try {
    manifest = readManifest(path.resolve(ROOT, o.manifest));
  } catch (e) {
    console.error(`${o.manifest}: ${e.message}`);
    return 2;
  }
  const errors = validate(manifest);
  if (errors.length) {
    for (const l of errors) console.error(`${o.manifest}: ${l}`);
    return 2;
  }
  const ids = o.batch ?? o.only ?? Object.keys(manifest.shots);
  for (const unit of [...ids, ...(o.print ? [o.print] : [])]) {
    const id = unit.split(":")[0];
    if (!manifest.shots[id]) {
      console.error(`no entry ${id} in ${o.manifest}`);
      return 2;
    }
  }
  if (o.print) {
    const entry = manifest.shots[o.print];
    if (o.candidate !== null && !(o.candidate <= (entry.candidates?.length ?? 0))) {
      console.error(`${o.print} has ${entry.candidates?.length ?? 0} candidates, not ${o.candidate}`);
      return 2;
    }
    console.log(printLines(o.print, entry, o.candidate).join("\n"));
    return 0;
  }
  if (o.batch) return renderBatch(o, manifest);
  const t0 = Date.now();
  if (o.build) {
    const b = run(
      ["systemd-run", "--user", "--scope", "--quiet", "-p", `MemoryMax=${MEMORY}`, "-p", "MemorySwapMax=2G", "--",
        "cargo", "build", "--release", "-q", "-p", "flythrough", "--bin", "flythrough", "--example", "clip"],
      { stdio: "inherit" },
    );
    if (b.status !== 0) {
      console.error(`the release build failed (exit ${b.status})`);
      return 1;
    }
    console.log(`built in ${((Date.now() - t0) / 1000).toFixed(0)} s`);
  }
  let failed = 0;
  /** `--check`'s unjudged entries: rendered and read, never counted. */
  const unjudgedIds = [];
  let waited = 0;
  let held = 0;
  const units = o.candidates ? candidateUnits(ids, manifest) : ids;
  if (units.length === 0) {
    console.error(o.candidates ? "no listed entry has candidates" : "no entry to shoot");
    return 2;
  }
  /** Per entry, the candidates that failed and why. */
  const candidateFailures = new Map();
  let todo = [...units];
  for (let hold = 1; todo.length > 0; hold++) {
    const up = liveRunner();
    if (up) {
      console.error(`a live-session runner is up (${up}); no hold is taken while it runs; not shot: ${todo.join(",")}`);
      return 1;
    }
    const asked = Date.now() / 1000;
    const loadAsked = os.loadavg()[0];
    const b = run(["flock", LOCK, process.execPath, fileURLToPath(import.meta.url), "--batch", todo.join(","),
      "--manifest", o.manifest, ...(o.check ? ["--check"] : [])]);
    const line = (b.stdout ?? "").split("\n").find((l) => l.startsWith(BATCH_TAG));
    if (b.status !== 0 || !line) {
      console.error(`hold ${hold}: the batch exited ${b.status ?? b.signal}: ${b.stderr}`);
      return 1;
    }
    const h = JSON.parse(line.slice(BATCH_TAG.length));
    if (h.results.length === 0) {
      console.error(`hold ${hold}: the batch rendered nothing`);
      return 1;
    }
    waited += h.start - asked;
    held += h.end - h.start;
    console.log(
      `hold ${hold}: one flock, waited ${(h.start - asked).toFixed(0)} s (load1 ${loadAsked.toFixed(2)} asked, ` +
        `${h.load.toFixed(2)} at the hold), held ${(h.end - h.start).toFixed(0)} s, ` +
        h.results.map((r) => `${r.id} ${r.secs.toFixed(1)} s`).join(", "),
    );
    for (const r of h.results) {
      if (o.candidates) {
        const [id, n] = r.id.split(":");
        const fails = candidateFailures.get(id) ?? new Map();
        candidateFailures.set(id, fails);
        const why = finishCandidate(id, manifest.shots[id], Number(n), r);
        if (why) {
          fails.set(Number(n), why);
          console.error(`${r.id}: ${why}`);
          failed++;
        } else console.log(`${r.id}: tile ${candidateCommands(id, manifest.shots[id], Number(n)).tilePng}, render ${r.secs.toFixed(1)} s`);
        continue;
      }
      const entry = manifest.shots[r.id];
      if (r.status !== 0) {
        console.error(`${r.id}: render exit ${r.status}, see ${commandsOf(r.id, entry, { check: o.check }).dir}.log`);
        failed++;
        continue;
      }
      const e = encodeOne(r.id, entry, o.check);
      if (e.failure) {
        console.error(`${r.id}: ${e.failure}`);
        failed++;
        continue;
      }
      const cap = e.bytes > entry.cap_bytes ? ` OVER its cap of ${entry.cap_bytes}` : "";
      let out = `${r.id}: ${e.out} ${e.bytes} bytes${cap}, ${e.frames} frame(s), render ${r.secs.toFixed(1)} s`;
      if (o.check) {
        const committed = path.join(ROOT, IMAGES, entry.file);
        if (!fs.existsSync(committed)) {
          out += `; no committed ${IMAGES}/${entry.file}`;
          failed++;
        } else {
          let d = null;
          if (entry.tolerance !== null || entry.unjudged !== undefined) {
            const { width, height } = probeSize(committed);
            d = cellShare(decodeRgb(committed), decodeRgb(path.join(ROOT, e.out)), width, height);
          }
          const v = verdict(entry, d);
          if (v.fails) failed++;
          if (v.unjudged) unjudgedIds.push(r.id);
          out += v.text;
        }
      }
      console.log(out);
    }
    todo = todo.slice(h.results.length);
  }
  if (o.candidates) {
    for (const [id, fails] of candidateFailures) {
      const s = makeSheet(id, manifest.shots[id], fails);
      console.log(`sheet ${s.sheet}: ${s.tiles} tiles, legend ${s.sheet.replace(/\.png$/, ".txt")} ${s.lines} lines`);
    }
    console.log(`index ${writeIndex(manifest)}`);
  }
  const judged = units.length - unjudgedIds.length;
  console.log(
    `${judged - failed} of ${judged} ${o.check ? "within tolerance" : o.candidates ? "candidates rendered" : "shot"}` +
      `${unjudgedIds.length ? ` (${unjudgedIds.length} unjudged, not counted: ${unjudgedIds.join(", ")})` : ""}, ` +
      `${((Date.now() - t0) / 1000).toFixed(0)} s in all: ${waited.toFixed(0)} s waiting for the lock, ${held.toFixed(0)} s held`,
  );
  return failed ? 1 : 0;
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  process.exitCode = main();
}
