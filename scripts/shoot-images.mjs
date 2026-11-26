#!/usr/bin/env node
// Re-renders the images under docs/images/ that docs/images/shots.json describes, headless,
// in batches inside holds of the GPU lock, and encodes each with ffmpeg after its hold: a
// still as lossy WebP (libwebp), a loop as animated WebP (libwebp_anim), both scaled to the
// entry's width with lanczos.
//
//   node scripts/shoot-images.mjs [--only ID,..] [--print ID] [--check] [--no-build]
//
// An entry's `source` is `flythrough` (the testbed binary with the entry's `flags`, plus
// `--seed`, `--still-at` and `--still-dir`; a loop's `still_at` is a frame range `A..B`) or
// `clip` (the fly-through's `clip` example with the entry's `flags`, plus `--fps` and
// `--out-dir`). The frames land under target/shots/<id>/. Every command is derived from the
// entry by `commandsOf`, so `--print` and the shoot cannot disagree. `--check` renders and
// encodes into target/shots/check/ instead, decodes the new and the committed file to raw RGB
// with ffmpeg, cuts every frame into CELL x CELL cells and prints the share of cells whose
// largest channel mean differs by more than CELL_LEVEL, against the entry's `tolerance`; it
// exits 1 when one is over. Encoder noise and a one-pixel move average out inside a cell; a
// changed scene, pass or hour moves whole cells. The `frozen` map names images that are kept
// and never re-rendered.
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
    if (!(e.tolerance >= 0 && e.tolerance <= 1)) errors.push(`${id}: tolerance is not a fraction in 0..1`);
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
export function commandsOf(id, entry, { check = false, ...wrapOpts } = {}) {
  const dir = check ? `${SHOTS}/check/${id}` : `${SHOTS}/${id}`;
  const out = check ? `${SHOTS}/check/${entry.file}` : `${IMAGES}/${entry.file}`;
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
  return { dir, out, clear: ["rm", "-rf", dir], render: wrap(argv, wrapOpts), encode };
}

const quote = (w) => (/^[A-Za-z0-9_./,:=@%+-]+$/.test(w) ? w : `'${w.replaceAll("'", "'\\''")}'`);
export const shell = (argv) => argv.map(quote).join(" ");

/** `--print ID`'s lines: the three commands, copy-pasteable from the repository root. */
export function printLines(id, entry) {
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
                 committed file and its tolerance; exit 1 when one is over
  --no-build     use the release binaries as already built
  --manifest F   read F instead of ${MANIFEST}`;

export function parseCli(argv) {
  const o = { only: null, print: null, check: false, build: true, manifest: MANIFEST, help: false, batch: null };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const val = () => {
      if (i + 1 >= argv.length) throw new Error(`${a} needs a value`);
      return argv[++i];
    };
    if (a === "--only") o.only = val().split(",").filter(Boolean);
    else if (a === "--print") o.print = val();
    else if (a === "--check") o.check = true;
    else if (a === "--no-build") o.build = false;
    else if (a === "--manifest") o.manifest = val();
    else if (a === "--batch") o.batch = val().split(",").filter(Boolean);
    else if (a === "-h" || a === "--help") o.help = true;
    else throw new Error(`unknown option ${a}`);
  }
  return o;
}

function run(argv, opts = {}) {
  return spawnSync(argv[0], argv.slice(1), { cwd: ROOT, encoding: "utf8", maxBuffer: 1 << 28, ...opts });
}

const BATCH_TAG = "shoot-batch ";

/** Inside a hold (`--batch`): render the entries in order until the hold is HOLD_S old, and
 * print one line for the parent: the hold's start and end, its load and each render. */
function renderBatch(o, manifest) {
  const clock = () => Date.now() / 1000;
  const start = clock();
  const load = os.loadavg()[0];
  const results = batchOf(o.batch, (id) => {
    const c = commandsOf(id, manifest.shots[id], { check: o.check, lock: null });
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
  for (const id of [...ids, ...(o.print ? [o.print] : [])]) {
    if (!manifest.shots[id]) {
      console.error(`no entry ${id} in ${o.manifest}`);
      return 2;
    }
  }
  if (o.print) {
    console.log(printLines(o.print, manifest.shots[o.print]).join("\n"));
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
  let waited = 0;
  let held = 0;
  let todo = [...ids];
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
          const { width, height } = probeSize(committed);
          const d = cellShare(decodeRgb(committed), decodeRgb(path.join(ROOT, e.out)), width, height);
          const over = d.share > entry.tolerance;
          if (over) failed++;
          out += `; cell share ${d.share.toFixed(6)} (C ${CELL}, L ${CELL_LEVEL}, largest ${d.max.toFixed(2)}) ` +
            `tolerance ${entry.tolerance} ${over ? "OVER" : "ok"}${d.note ? ` (${d.note})` : ""}`;
          // A loop's worst frame is a reading beside the judged share, not judged itself.
          if (entry.kind === "loop" && !d.note) {
            out += d.worst ? `; worst frame ${d.worst.frame} at ${d.worst.share.toFixed(6)} (unjudged)` : "; worst frame none";
          }
        }
      }
      console.log(out);
    }
    todo = todo.slice(h.results.length);
  }
  console.log(
    `${ids.length - failed} of ${ids.length} ${o.check ? "within tolerance" : "shot"}, ` +
      `${((Date.now() - t0) / 1000).toFixed(0)} s in all: ${waited.toFixed(0)} s waiting for the lock, ${held.toFixed(0)} s held`,
  );
  return failed ? 1 : 0;
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  process.exitCode = main();
}
