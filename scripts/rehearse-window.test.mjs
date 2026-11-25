// node --test scripts/rehearse-window.test.mjs: the window rehearsal's parts that need no GPU.

import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import zlib from "node:zlib";
import {
  applyPairs,
  carvedLine,
  commandOf,
  diffFacts,
  encodePng,
  imageFacts,
  loudLines,
  modeFlags,
  nameOf,
  parseCli,
  parseList,
  parseRecords,
  readPpm,
  readRunner,
  sheet,
  tokenize,
  unnamedFlags,
} from "./rehearse-window.mjs";

const LIB = "/home/igor/Work/voxel-engine/.claude/skills/orchestrate/references/live-runner-lib.sh";

/** A `w` x `h` RGB image filled by `fill(x, y)` -> [r, g, b]. */
function image(w, h, fill) {
  const data = Buffer.alloc(w * h * 3);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) data.set(fill(x, y), (y * w + x) * 3);
  }
  return { width: w, height: h, data };
}

test("tokenize splits on whitespace and groups quoted words", () => {
  assert.deepEqual(tokenize(`a --x "b c" 'd "e"' f`), ["a", "--x", "b c", 'd "e"', "f"]);
  assert.deepEqual(tokenize(`  ""  `), [""]);
  assert.throws(() => tokenize(`a "b`), /unclosed/);
});

test("a list gives each line a run with its flags, before and dig", () => {
  const runs = parseList(
    [
      "# a comment",
      "",
      "base --source outdoor --time 12:00",
      `ray --source outdoor --key-far ray before=base dig=30,40`,
      "base --hud",
    ].join("\n"),
  );
  assert.equal(runs.length, 3);
  assert.deepEqual(runs[0], {
    name: "base",
    label: "base",
    program: null,
    args: ["--source", "outdoor", "--time", "12:00"],
    before: null,
    dig: null,
  });
  assert.equal(runs[1].before, "base");
  assert.equal(runs[1].dig, "30,40");
  assert.deepEqual(runs[1].args, ["--source", "outdoor", "--key-far", "ray"]);
  assert.equal(runs[2].name, "run3", "a repeated name is renamed");
});

test("a run's name is its label's first word", () => {
  assert.equal(nameOf("1. Noon slope"), "1");
  assert.equal(nameOf("B2. Forest 10,180: smooth"), "B2");
  assert.equal(nameOf("A: x"), "A");
  assert.equal(nameOf("  "), "");
});

test("the recorder's records parse: label, count, program, flags", () => {
  const rec = (label, ...words) => `${label}\0${words.length}\0${words.map((w) => `${w}\0`).join("")}`;
  const buf = Buffer.from(rec("1. Base", "live/bin/f", "--hud", "--sun", "10,180") + rec("2. Ray", "live/bin/f", "--key-far", "ray") + rec("x", "--a"));
  const runs = parseRecords(buf);
  assert.deepEqual(
    runs.map((r) => [r.name, r.program, r.args]),
    [
      ["1", "live/bin/f", ["--hud", "--sun", "10,180"]],
      ["2", "live/bin/f", ["--key-far", "ray"]],
      ["x", null, ["--a"]],
    ],
  );
  assert.throws(() => parseRecords(Buffer.from("a\0" + "9\0" + "b\0")), /broken record/);
});

// The lib lives in the local harness repo, not the public one: CI skips this test.
test("a runner script read through the lib's list mode gives its runs, with variables expanded", { skip: !fs.existsSync(LIB) && "no live-runner lib here" }, () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "rehearse-test-"));
  try {
    const bin = path.join(dir, "flythrough");
    fs.writeFileSync(bin, "#!/bin/sh\nexit 0\n", { mode: 0o755 });
    const runner = path.join(dir, "rehearse-fixture.sh");
    fs.writeFileSync(
      runner,
      [
        "#!/bin/bash",
        "set -u",
        `cd ${dir}`,
        `BIN=${bin}  LOG=${dir}/fixture.log  NAME=F`,
        'NOON="--source outdoor --time 12:00 --hud"',
        `source ${LIB}`,
        'run "1. Noon base" "look at the slope" "$BIN" $NOON',
        'note "Run 1: what"',
        'run "2. Noon ray" "the ray arm\\n  second line" "$BIN" $NOON --key-far ray --label "two words"',
        "live_end",
        "echo never",
      ].join("\n"),
    );
    const runs = readRunner(runner);
    assert.deepEqual(
      runs.map((r) => [r.name, r.label, r.program, r.args]),
      [
        ["1", "1. Noon base", bin, ["--source", "outdoor", "--time", "12:00", "--hud"]],
        ["2", "2. Noon ray", bin, ["--source", "outdoor", "--time", "12:00", "--hud", "--key-far", "ray", "--label", "two words"]],
      ],
    );
    assert.ok(!fs.existsSync(path.join(dir, "fixture.log")), "list mode writes no session log");
    applyPairs(runs, "before", ["2=1"]);
    applyPairs(runs, "dig", ["1=30"]);
    assert.equal(runs[1].before, "1");
    assert.equal(runs[0].dig, "30");
    assert.throws(() => applyPairs(runs, "before", ["3=1"]), /no run named 3/);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("a run's command keeps its flags, drops the rehearsal's own and adds them", () => {
  const opts = { adapter: "llvmpipe", frames: 30, render: "320x180" };
  const run = { name: "a", args: ["--hud", "--adapter", "RTX", "--frames", "9", "--headless", "--sun", "10,180"], dig: "5" };
  assert.deepEqual(commandOf(run, opts, "out"), [
    "--hud", "--sun", "10,180",
    "--headless", "--adapter", "llvmpipe", "--frames", "30", "--render", "320x180",
    "--dig-at", "5", "--still", "out/a.ppm", "--mode-text", "out/a.txt",
  ]);
  const own = commandOf({ name: "b", args: ["--render", "64x36"], dig: null }, opts, "o");
  assert.equal(own.filter((w) => w === "--render").length, 1);
  assert.equal(own[own.indexOf("--render") + 1], "64x36");
  assert.throws(() => commandOf({ name: "c", args: ["--dig-at", "3"], dig: "4" }, opts, "o"), /dig=/);
});

test("the CLI wants an adapter, an output and one source of runs", () => {
  const ok = parseCli(["--adapter", "RTX", "--out", "o", "--runner", "r.sh", "--before", "2=1", "--named-ok", "--a,--b"]);
  assert.equal(ok.frames, 60);
  assert.deepEqual(ok.before, ["2=1"]);
  assert.deepEqual(ok.namedOk, ["--a", "--b"]);
  assert.throws(() => parseCli(["--out", "o", "--runner", "r"]), /required/);
  assert.throws(() => parseCli(["--adapter", "x", "--out", "o"]), /one of/);
  assert.throws(() => parseCli(["--adapter", "x", "--out", "o", "--list", "l"]), /--bin/);
  assert.throws(() => parseCli(["--adapter", "x", "--out", "o", "--runner", "r", "--render", "big"]), /bad/);
  assert.throws(() => parseCli(["--bogus"]), /unknown/);
});

test("a PPM reads back, and a PNG round-trips through node:zlib", () => {
  const img = image(5, 3, (x, y) => [x * 50, y * 100, (x + y) % 2 ? 255 : 0]);
  const ppm = Buffer.concat([Buffer.from("P6\n# made by a test\n5 3\n255\n"), img.data]);
  const back = readPpm(ppm);
  assert.equal(back.width, 5);
  assert.equal(back.height, 3);
  assert.deepEqual(Buffer.from(back.data), img.data);
  assert.throws(() => readPpm(Buffer.from("P3\n1 1\n255\n0 0 0")), /P6/);
  assert.throws(() => readPpm(Buffer.from("P6\n2 2\n255\n\0\0\0")), /short/);

  const png = encodePng(img);
  assert.deepEqual([...png.subarray(0, 8)], [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  // Walk the chunks, checking each CRC, and inflate the IDAT back to the rows.
  let at = 8;
  const chunks = {};
  while (at < png.length) {
    const len = png.readUInt32BE(at);
    const type = png.subarray(at + 4, at + 8).toString("ascii");
    const body = png.subarray(at + 8, at + 8 + len);
    const crc = png.readUInt32BE(at + 8 + len);
    assert.equal(crc, zlib.crc32(png.subarray(at + 4, at + 8 + len)) >>> 0, `${type} crc`);
    chunks[type] = body;
    at += 12 + len;
  }
  assert.deepEqual(Object.keys(chunks), ["IHDR", "IDAT", "IEND"]);
  assert.equal(chunks.IHDR.readUInt32BE(0), 5);
  assert.equal(chunks.IHDR.readUInt32BE(4), 3);
  assert.deepEqual([...chunks.IHDR.subarray(8)], [8, 2, 0, 0, 0]);
  const raw = zlib.inflateSync(chunks.IDAT);
  assert.equal(raw.length, 3 * (1 + 15));
  for (let y = 0; y < 3; y++) {
    assert.equal(raw[y * 16], 0, "filter none");
    assert.deepEqual(raw.subarray(y * 16 + 1, (y + 1) * 16), img.data.subarray(y * 15, (y + 1) * 15));
  }
});

test("the image facts: one colour, black and clipped white shares, mean level", () => {
  const grey = imageFacts(image(4, 4, () => [51, 51, 51]));
  assert.equal(grey.oneColour, true);
  assert.equal(grey.black, 0);
  assert.equal(grey.white, 0);
  assert.ok(Math.abs(grey.mean - 0.2) < 1e-9);
  // A quarter black, a quarter clipped white, the rest mid grey.
  const mixed = imageFacts(image(4, 4, (x) => (x === 0 ? [0, 0, 0] : x === 1 ? [255, 255, 255] : [255, 0, 255])));
  assert.equal(mixed.oneColour, false);
  assert.equal(mixed.black, 0.25);
  assert.equal(mixed.white, 0.25);
  assert.ok(Math.abs(mixed.mean - (0 + 3 + 2 * 2) / 12) < 1e-9);
  assert.equal(mixed.nearWhite, 0.25);
  const nearWhite = imageFacts(image(2, 1, (x) => (x ? [255, 255, 254] : [240, 250, 239])));
  assert.equal(nearWhite.white, 0, "a channel under 255 is not clipped white");
  assert.equal(nearWhite.nearWhite, 0.5, "every channel at 240 or over is near white");
});

test("the difference against a before image counts pixels over the tolerance", () => {
  const a = image(10, 10, () => [100, 100, 100]);
  const b = image(10, 10, (x, y) => (y === 0 ? [100, 103, 100] : x === 0 ? [102, 100, 100] : [100, 100, 100]));
  assert.deepEqual(diffFacts(a, b, 2), { count: 10, share: 0.1 });
  assert.deepEqual(diffFacts(a, b, 0), { count: 19, share: 0.19 });
  assert.equal(diffFacts(a, image(5, 5, () => [0, 0, 0]), 2), null);
});

const HELP = `flythrough [FLAGS]

Run:
  --frames N        exit after N frames
Session:
  --headless        render offscreen
Outdoors and the sky:
  --sun EL,AZ       the sun
  --key-far baked|column|ray   the key far
Look:
  --look PRESET     the look
  --far-normal A,B  the far normal
Lighting:
  --lamp-bounce, --probes on|off   the lamp bounce
`;

test("the mode flags are the help's mode sections, minus the world's", () => {
  assert.deepEqual([...modeFlags(HELP)].sort(), ["--far-normal", "--key-far", "--lamp-bounce", "--look"]);
});

test("a flag is named when the mode text holds its words and its one-word value", () => {
  const checked = modeFlags(HELP);
  const text = "banner (first frame):\nkey far column, look face\nmodes:\nfar normal 8..12\n";
  assert.deepEqual(unnamedFlags(["--key-far", "column", "--far-normal", "8,12", "--frames", "3"], text, checked), []);
  assert.deepEqual(unnamedFlags(["--key-far", "ray"], text, checked), ["--key-far ray"]);
  assert.deepEqual(unnamedFlags(["--look", "field", "--sun", "10,180"], text, checked), ["--look field"]);
  assert.deepEqual(unnamedFlags(["--lamp-bounce", "off"], text, checked), ["--lamp-bounce off"]);
});

test("the sheet's loud lines: no still, one colour, the same picture, an unnamed flag", () => {
  const img = image(2, 2, (x) => [x * 200, 0, 0]);
  const flat = image(2, 2, () => [9, 9, 9]);
  const ok = (name, extra = {}) => ({ name, label: name, command: `f ${name}`, args: [], image: img, facts: imageFacts(img), unnamed: [], text: "modes:\nx\nhud:\nSTATSROW", ...extra });
  const results = [
    ok("base"),
    ok("same", { before: "base", diff: { count: 0, share: 0 } }),
    ok("differs", { before: "base", diff: { count: 2, share: 0.5 } }),
    ok("flat", { image: flat, facts: imageFacts(flat) }),
    ok("quiet", { unnamed: ["--key-far ray"] }),
    { name: "broken", label: "broken", command: "f broken", failure: "exit 101", args: [] },
    ok("orphan", { before: "broken" }),
    ok("dug", { dig: "5", carved: "carved: rock 4169 (2 chunks, 0.25 ms)" }),
    ok("missed", { args: ["--dig-at", "5"], carved: "carved: nothing (0 chunks, 0.02 ms)" }),
  ];
  const loud = loudLines(results, 0.001);
  assert.deepEqual(loud, [
    "same: the same picture as base (0 pixels, 0.0 %, under the floor 0.100 %)",
    "flat: the still is all one colour",
    "quiet: the mode text does not name --key-far ray",
    "broken: no still (exit 101)",
    "orphan: its before run broken has no still",
    "missed: the scripted dig carved nothing (carved: nothing (0 chunks, 0.02 ms))",
  ]);
  const md = sheet({ adapter: "llvmpipe" }, results, loud);
  assert.match(md, /^# Window rehearsal\n\n- adapter: llvmpipe\n/);
  assert.match(md, /- \*\*LOUD\*\* same: the same picture as base/);
  assert.match(md, /## differs\n[\s\S]*!\[differs\]\(differs.png\)[\s\S]*- against base: 2 pixels differ \(50.0 %\)/);
  assert.match(md, /## dug\n[\s\S]*- the dig: carved: rock 4169/);
  assert.match(md, /## broken\n[\s\S]*- no still: exit 101 \(see broken.log\)/);
  assert.ok(!md.includes("STATSROW"), "the stats rows stay in the .txt");
  assert.match(sheet({}, [ok("base")], []), /## Loud lines\n\nnone\n/);
});

test("the script's file runs as a module without starting a rehearsal", () => {
  const here = path.dirname(fileURLToPath(import.meta.url));
  assert.ok(fs.existsSync(path.join(here, "rehearse-window.mjs")));
});

test("the dig's line is the mode text's last carved row", () => {
  assert.equal(carvedLine("a\ncarved: nothing (0 chunks, 0.01 ms)\nb\ncarved: rock 9 (1 chunks, 0.1 ms)\n"), "carved: rock 9 (1 chunks, 0.1 ms)");
  assert.equal(carvedLine("no edit\n"), null);
});
