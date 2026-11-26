// node --test scripts/shoot-images.test.mjs: the shoot script's parts that need no GPU.

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { test } from "node:test";
import {
  batchOf,
  CELL,
  CELL_LEVEL,
  cellShare,
  diffFraction,
  framesOf,
  MANIFEST,
  parseCli,
  printLines,
  readManifest,
  REQUIRED,
  ROOT,
  validate,
} from "./shoot-images.mjs";

const SCRIPT = path.join(ROOT, "scripts/shoot-images.mjs");

const FIXTURE = {
  shots: {
    still: {
      kind: "still", file: "still.webp", width: 800, quality: 80, source: "flythrough", seed: 3,
      flags: ["--headless", "--settled-start", "--source", "showcase", "--render", "320x180", "--frames", "8"],
      still_at: 7, tolerance: 0.001, cap_bytes: 300000, note: "a fixture",
    },
    "loop-a": {
      kind: "loop", file: "loop-a.webp", width: 400, quality: 70, fps: 10, source: "flythrough",
      flags: ["--headless", "--settled-start", "--source", "cave", "--dig-at", "2", "--frames", "6"],
      still_at: "3..5", tolerance: 0, cap_bytes: 700000, note: "a fixture",
    },
    spin: {
      kind: "loop", file: "spin.webp", width: 480, quality: 70, fps: 25, source: "clip",
      flags: ["--clip", "walk", "--orbit", "24,3,-10"], tolerance: 0, cap_bytes: 700000, note: "a fixture",
    },
  },
  frozen: { "light/old.png": "kept" },
};

test("the committed manifest parses and validates", () => {
  const m = readManifest(path.join(ROOT, MANIFEST));
  assert.deepEqual(validate(m), []);
  for (const [id, e] of Object.entries(m.shots)) {
    for (const f of REQUIRED) assert.ok(e[f] !== undefined, `${id} has ${f}`);
  }
  for (const [file, why] of Object.entries(m.frozen)) {
    assert.ok(typeof why === "string" && why.length > 0, `${file} says why it is frozen`);
  }
});

test("the manifest names every image file under docs/images, as an entry or frozen", () => {
  const m = readManifest(path.join(ROOT, MANIFEST));
  const named = new Set([...Object.values(m.shots).map((e) => e.file), ...Object.keys(m.frozen)]);
  const dir = path.join(ROOT, "docs/images");
  const walk = (d) =>
    fs.readdirSync(d, { withFileTypes: true }).flatMap((x) =>
      x.isDirectory() ? walk(path.join(d, x.name)) : [path.relative(dir, path.join(d, x.name))],
    );
  for (const f of walk(dir).filter((f) => /\.(png|gif|webp|jpe?g|svg)$/.test(f))) {
    assert.ok(named.has(f), `${f} is in the manifest`);
  }
});

test("validate names a missing field, a bad range and a flag the script adds", () => {
  assert.deepEqual(validate(FIXTURE), []);
  const bad = structuredClone(FIXTURE);
  delete bad.shots.still.tolerance;
  bad.shots["loop-a"].still_at = "5";
  bad.shots.spin.flags.push("--out-dir", "x");
  bad.shots.spin.file = "still.webp";
  bad.shots["loop-a"].flags = bad.shots["loop-a"].flags.filter((f) => f !== "--settled-start");
  assert.deepEqual(validate(bad), [
    "still: no `tolerance`",
    "still: tolerance is not a fraction in 0..1",
    "loop-a: a loop's still_at is A..B",
    "loop-a: a flythrough entry runs --settled-start",
    "spin: flags name --out-dir, which the script adds",
    "spin: file still.webp is also still's",
  ]);
});

test("framesOf is the one still or the whole range", () => {
  assert.deepEqual(framesOf(FIXTURE.shots.still), [7]);
  assert.deepEqual(framesOf(FIXTURE.shots["loop-a"]), [3, 4, 5]);
});

test("--print gives the clear, the locked render and the encode, copy-pasteable", () => {
  assert.deepEqual(printLines("still", FIXTURE.shots.still), [
    "# still: from the repository root",
    "rm -rf target/shots/still",
    "flock /home/igor/Work/voxel-engine/.git/svet-perf.lock timeout 300 systemd-run --user --scope --quiet -p MemoryMax=6G -p MemorySwapMax=2G -- target/release/flythrough --headless --settled-start --source showcase --render 320x180 --frames 8 --seed 3 --still-at 7 --still-dir target/shots/still",
    "ffmpeg -v error -y -pattern_type glob -i 'target/shots/still/*.png' -frames:v 1 -vf scale=800:-1:flags=lanczos -c:v libwebp -quality 80 -compression_level 6 docs/images/still.webp",
  ]);
  const loop = printLines("loop-a", FIXTURE.shots["loop-a"]);
  assert.match(loop[2], / --still-at 3,4,5 --still-dir target\/shots\/loop-a$/);
  assert.match(loop[3], /-framerate 10 .* -c:v libwebp_anim -quality 70 -compression_level 6 -loop 0 docs\/images\/loop-a\.webp$/);
  const clip = printLines("spin", FIXTURE.shots.spin);
  assert.match(clip[2], / -- target\/release\/examples\/clip --clip walk --orbit 24,3,-10 --fps 25 --out-dir target\/shots\/spin$/);
});

test("the CLI prints an entry of a manifest file and refuses an unknown one", () => {
  const dir = fs.mkdtempSync(path.join(ROOT, "target", "shoot-test-"));
  try {
    const file = path.join(dir, "shots.json");
    fs.writeFileSync(file, JSON.stringify(FIXTURE));
    const ok = spawnSync("node", [SCRIPT, "--manifest", file, "--print", "still"], { encoding: "utf8" });
    assert.equal(ok.status, 0, ok.stderr);
    assert.equal(ok.stdout, `${printLines("still", FIXTURE.shots.still).join("\n")}\n`);
    const no = spawnSync("node", [SCRIPT, "--manifest", file, "--print", "nope"], { encoding: "utf8" });
    assert.equal(no.status, 2);
    assert.match(no.stderr, /no entry nope/);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("parseCli reads the options and refuses an unknown one", () => {
  assert.deepEqual(parseCli(["--only", "a,b", "--check", "--no-build"]), {
    only: ["a", "b"], print: null, check: true, build: false, manifest: MANIFEST, help: false, batch: null,
  });
  assert.throws(() => parseCli(["--bogus"]), /unknown option --bogus/);
  assert.throws(() => parseCli(["--print"]), /--print needs a value/);
});

test("diffFraction counts pixels with a channel over the level", () => {
  const a = Buffer.from([10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10]);
  const b = Buffer.from([12, 10, 10, 13, 10, 10, 10, 10, 10, 10, 10, 0]);
  assert.deepEqual(diffFraction(a, b), { fraction: 0.5 });
  assert.equal(diffFraction(a, b.subarray(3)).fraction, 1);
});

// A 10 x 10 frame in cells of 8: four cells, the right and bottom ones 2 wide or 2 high.
const W = 10;
const H = 10;
const frame = (v = 100) => Buffer.alloc(W * H * 3, v);
/** Adds `by` to channel `ch` of every pixel in the rectangle [x0, x1) x [y0, y1). */
function move(buf, x0, y0, x1, y1, by, ch = 0) {
  for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) buf[(y * W + x) * 3 + ch] += by;
  return buf;
}

test("the check's cell constants are the reading's pick", () => {
  assert.equal(CELL, 8);
  assert.equal(CELL_LEVEL, 4);
});

test("cellShare: equal buffers read 0", () => {
  assert.deepEqual(cellShare(frame(), frame(), W, H), { share: 0, max: 0, worst: null });
});

test("cellShare: one cell moved by L + 1 reads one over the cell count, by L reads 0", () => {
  const over = cellShare(frame(), move(frame(), 0, 0, 8, 8, CELL_LEVEL + 1), W, H);
  assert.deepEqual(over, { share: 1 / 4, max: CELL_LEVEL + 1, worst: { frame: 0, share: 1 / 4 } });
  const at = cellShare(frame(), move(frame(), 0, 0, 8, 8, CELL_LEVEL, 2), W, H);
  assert.deepEqual(at, { share: 0, max: CELL_LEVEL, worst: null });
});

test("cellShare: an edge cell counts once, by its own pixels' mean", () => {
  assert.equal(cellShare(frame(), move(frame(), 8, 8, 10, 10, CELL_LEVEL + 1), W, H).share, 1 / 4);
  // One pixel of the 2 x 2 corner cell moved by 4 (L + 1): the cell's mean moves by 5.
  assert.equal(cellShare(frame(), move(frame(), 9, 9, 10, 10, 4 * (CELL_LEVEL + 1)), W, H).share, 1 / 4);
  // The same move in a full 8 x 8 cell averages out.
  assert.equal(cellShare(frame(), move(frame(), 0, 0, 1, 1, 4 * (CELL_LEVEL + 1)), W, H).share, 0);
});

test("cellShare: cells of every frame of a loop count, and a size mismatch reads 1", () => {
  const two = Buffer.concat([frame(), frame()]);
  const moved = Buffer.concat([frame(), move(frame(), 0, 0, 8, 8, -(CELL_LEVEL + 1), 1)]);
  assert.equal(cellShare(two, moved, W, H).share, 1 / 8);
  assert.equal(cellShare(frame(), two, W, H).share, 1);
  assert.equal(cellShare(frame(), frame(), 7, 7).share, 1);
});

test("cellShare: two frames, one cell moved by L + 1 in the second: the worst frame is frame 1", () => {
  const two = Buffer.concat([frame(), frame()]);
  const moved = Buffer.concat([frame(), move(frame(), 0, 0, 8, 8, CELL_LEVEL + 1)]);
  assert.deepEqual(cellShare(two, moved, W, H), { share: 1 / 8, max: CELL_LEVEL + 1, worst: { frame: 1, share: 1 / 4 } });
  assert.equal(cellShare(two, Buffer.concat([frame(), frame()]), W, H).worst, null);
});

test("batchOf starts no render once the hold is the limit old, on a faked clock", () => {
  let t = 1000;
  const now = () => t;
  const render = (id) => {
    t += 250;
    return id;
  };
  // Starts at 0, 250 and 500 s; the fourth would start at 750 s, past 600.
  assert.deepEqual(batchOf(["a", "b", "c", "d"], render, now, 600), ["a", "b", "c"]);
  assert.deepEqual(batchOf(["d"], render, now, 600), ["d"]);
  // A render that overruns the limit still ends the batch after itself.
  const slow = () => {
    t += 900;
    return "x";
  };
  assert.deepEqual(batchOf(["x", "y"], slow, now, 600), ["x"]);
});
