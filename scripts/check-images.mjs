#!/usr/bin/env node
// The image gate: the files under docs/images/ against the shot manifest
// (docs/images/shots.json) and the markdown that shows them. Static: it reads names, byte
// sizes and markdown, and renders or decodes nothing (the rendered comparison is
// `node scripts/shoot-images.mjs --check`, run locally on the adapter each entry names).
//
// The rules, each finding one line `<path> -> <reason>`:
// - every image file under docs/images/ (png, jpg, jpeg, gif, webp, svg) is a manifest
//   entry's `file` or a key of its `frozen` map;
// - every entry has `kind` `still` or `loop`; a non-draft entry's file exists and at least
//   one markdown file references it; a `frozen` file exists;
// - every image reference in markdown (`![alt](path)` or `<img src=...>`) carries non-empty
//   alt text (external images included: a badge's alt is what a screen reader says);
// - an entry's file, when it exists, is at or under its cap: 300 000 bytes for a still,
//   700 000 for a loop, or the entry's `cap_bytes` when that is lower (a `cap_bytes` above
//   the kind's cap is itself a finding: the manifest may lower a cap, never raise it);
// - the images README.md references (repository files; drafts excluded) total at or under
//   5 000 000 bytes, or the manifest's top-level `readme_cap_bytes` when that is lower.
// An entry with `"draft": true` need not exist nor be referenced; if its file exists its cap
// holds, and it counts in no README total. The drafts' count is printed. Frozen files carry
// no cap (they are figures nothing re-renders).
//
// The markdown is what git tracks plus untracked files it would track, as check-doc-links
// reads it; fenced code blocks and inline code spans are skipped.
//
// Usage:  node scripts/check-images.mjs [root]
//         node scripts/check-images.mjs --self-test
// Exit 0 = no finding; a summary line is printed. Exit 1 = the findings on stderr, then the
// count line `N finding(s)`. The optional `root` scans another tree:
// scripts/fixtures/images is the seeded one, and scripts/fixtures/README.md names its
// expected findings. The seeded trees (the subdirectories of scripts/fixtures) are skipped by
// path on an ordinary run. `--self-test` runs the seeded tree and the repository in one
// process and asserts the seeded tree's exact findings and draft count, and the repository's
// clean result: exit 0 from a gate whose detectors stopped matching would otherwise look the
// same as a clean tree.

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, statSync } from "node:fs";
import { dirname, join, posix, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SEEDED = resolve(REPO_ROOT, "scripts", "fixtures");
const FIXTURE = resolve(SEEDED, "images");
const IMAGES_DIR = "docs/images";
const MANIFEST = `${IMAGES_DIR}/shots.json`;
const IMAGE_EXT = /\.(png|jpe?g|gif|webp|svg)$/i;
const CAPS = { still: 300_000, loop: 700_000 };
const README_CAP = 5_000_000;

/** Repository-relative files under `root` (tracked, plus untracked ones git would track). */
function files(root) {
  const out = execFileSync("git", ["ls-files", "-z", "--cached", "--others", "--exclude-standard"], {
    cwd: root,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "ignore"],
    maxBuffer: 64 * 1024 * 1024,
  });
  const rooted = root === SEEDED || root.startsWith(SEEDED + sep);
  return out
    .split("\0")
    .filter(Boolean)
    .filter((p) => {
      const abs = resolve(root, p);
      if (!existsSync(abs)) return false; // a tracked file deleted from the working tree
      return rooted || !(abs.startsWith(SEEDED + sep) && dirname(abs) !== SEEDED);
    })
    .map((p) => p.split(sep).join("/"));
}

/** Image references in one markdown text: `{ line, alt, target }`, code skipped. */
function imageRefs(text) {
  const refs = [];
  let inFence = false;
  text.split("\n").forEach((raw, i) => {
    if (/^\s*(```|~~~)/.test(raw)) {
      inFence = !inFence;
      return;
    }
    if (inFence) return;
    const line = raw.replace(/`[^`]*`/g, "");
    for (const m of line.matchAll(/!\[([^\]]*)\]\(\s*<?([^)\s>]+)>?(?:\s+"[^"]*")?\s*\)/g)) {
      refs.push({ line: i + 1, alt: m[1], target: m[2] });
    }
    for (const m of line.matchAll(/<img\b[^>]*>/gi)) {
      const src = /\bsrc\s*=\s*["']([^"']+)["']/i.exec(m[0]);
      const alt = /\balt\s*=\s*["']([^"']*)["']/i.exec(m[0]);
      if (src) refs.push({ line: i + 1, alt: alt ? alt[1] : "", target: src[1] });
    }
  });
  return refs;
}

/** The repository path a reference from `mdPath` names, or null for an external one. */
function resolveRef(mdPath, target) {
  if (/^([a-z][a-z0-9+.-]*:|\/\/)/i.test(target)) return null;
  const path = decodeURIComponent(target.split(/[?#]/)[0]);
  if (!path) return null;
  return path.startsWith("/")
    ? posix.normalize(path.slice(1))
    : posix.normalize(posix.join(posix.dirname(mdPath), path));
}

/** Runs every rule over `root`: `{ findings, drafts, summary }`. */
function check(root) {
  const findings = [];
  const all = files(root);
  const manifestAbs = join(root, MANIFEST);
  if (!existsSync(manifestAbs)) {
    return { findings: [`${MANIFEST} -> missing`], drafts: 0, summary: "" };
  }
  let manifest;
  try {
    manifest = JSON.parse(readFileSync(manifestAbs, "utf8"));
  } catch (e) {
    return { findings: [`${MANIFEST} -> does not parse: ${e.message}`], drafts: 0, summary: "" };
  }
  const shots = manifest.shots ?? {};
  const frozen = manifest.frozen ?? {};
  const size = (p) => statSync(join(root, p)).size;
  const exists = (p) => existsSync(join(root, p));

  // Every markdown file's image references, resolved to repository paths.
  const refs = [];
  for (const md of all.filter((p) => p.endsWith(".md"))) {
    for (const r of imageRefs(readFileSync(join(root, md), "utf8"))) {
      refs.push({ ...r, md, path: resolveRef(md, r.target) });
    }
  }
  for (const r of refs) {
    if (!r.alt.trim()) findings.push(`${r.md}:${r.line} -> image ${r.target} has empty alt text`);
  }
  const referenced = new Set(refs.map((r) => r.path).filter(Boolean));

  // The manifest's names, by repository path.
  const named = new Map();
  for (const [id, e] of Object.entries(shots)) named.set(`${IMAGES_DIR}/${e.file}`, id);
  for (const f of Object.keys(frozen)) named.set(`${IMAGES_DIR}/${f}`, null);

  for (const p of all.filter((p) => p.startsWith(`${IMAGES_DIR}/`) && IMAGE_EXT.test(p))) {
    if (!named.has(p)) findings.push(`${p} -> an image the manifest neither names nor freezes`);
  }
  for (const f of Object.keys(frozen)) {
    const p = `${IMAGES_DIR}/${f}`;
    if (!exists(p)) findings.push(`${p} -> frozen in the manifest but missing`);
  }

  let drafts = 0;
  const draftPaths = new Set();
  for (const [id, e] of Object.entries(shots)) {
    const p = `${IMAGES_DIR}/${e.file}`;
    if (e.draft === true) {
      drafts++;
      draftPaths.add(p);
    }
    const kindCap = CAPS[e.kind];
    if (kindCap === undefined) {
      findings.push(`${p} -> entry ${id}: kind ${JSON.stringify(e.kind)} is neither still nor loop`);
      continue;
    }
    if (e.cap_bytes !== undefined && e.cap_bytes > kindCap) {
      findings.push(`${p} -> entry ${id}: cap_bytes ${e.cap_bytes} raises the ${e.kind} cap of ${kindCap}`);
    }
    const cap = Math.min(kindCap, e.cap_bytes ?? kindCap);
    if (!exists(p)) {
      if (!e.draft) findings.push(`${p} -> entry ${id}'s file is missing`);
      continue;
    }
    const n = size(p);
    if (n > cap) findings.push(`${p} -> ${n} bytes, over the ${e.kind} cap of ${cap}`);
    if (!e.draft && !referenced.has(p)) findings.push(`${p} -> entry ${id}: no markdown references it`);
  }

  const readmeCap = Math.min(README_CAP, manifest.readme_cap_bytes ?? README_CAP);
  const readmeImages = new Set(
    refs
      .filter((r) => r.md === "README.md" && r.path && exists(r.path) && !draftPaths.has(r.path))
      .map((r) => r.path),
  );
  let readmeBytes = 0;
  for (const p of readmeImages) readmeBytes += size(p);
  if (readmeBytes > readmeCap) {
    findings.push(
      `README.md -> its ${readmeImages.size} image(s) total ${readmeBytes} bytes, over the cap of ${readmeCap}`,
    );
  }
  const summary =
    `images: ${Object.keys(shots).length} entries (${drafts} draft), ${Object.keys(frozen).length} frozen, ` +
    `${refs.length} image references with alt text, README.md's images ${readmeBytes} of ${readmeCap} bytes`;
  return { findings, drafts, summary };
}

// The seeded tree's findings, in the order the gate reports them; scripts/fixtures/README.md
// lists the same lines.
const EXPECTED = [
  "docs/page.md:5 -> image images/big.webp has empty alt text",
  "docs/images/stray.png -> an image the manifest neither names nor freezes",
  "docs/images/big.webp -> 301000 bytes, over the still cap of 300000",
  "docs/images/orphan.webp -> entry orphan: no markdown references it",
  "README.md -> its 2 image(s) total 2400 bytes, over the cap of 2000",
];
const EXPECTED_DRAFTS = 1;

function selfTest() {
  const results = [];
  const fixture = check(FIXTURE);
  const same =
    fixture.findings.length === EXPECTED.length && fixture.findings.every((f, i) => f === EXPECTED[i]);
  results.push([`the seeded tree reports exactly the ${EXPECTED.length} expected findings`, same]);
  results.push([`the seeded tree counts ${EXPECTED_DRAFTS} draft`, fixture.drafts === EXPECTED_DRAFTS]);
  const repo = check(REPO_ROOT);
  results.push(["the repository reports no finding", repo.findings.length === 0]);
  for (const [name, ok] of results) console.log(`${ok ? "ok  " : "FAIL"} ${name}`);
  if (!same) {
    console.log("seeded tree reported:");
    for (const f of fixture.findings) console.log(`  ${f}`);
  }
  const passed = results.filter(([, ok]) => ok).length;
  console.log(`self-test: ${passed}/${results.length}`);
  process.exit(passed === results.length ? 0 : 1);
}

const argv = process.argv.slice(2);
if (argv.includes("--self-test")) selfTest();
const root = resolve(argv.find((a) => !a.startsWith("--")) ?? REPO_ROOT);
const { findings, summary } = check(root);
if (findings.length) {
  for (const f of findings) console.error(f);
  console.error(`${findings.length} finding(s)`);
  process.exit(1);
}
console.log(root === REPO_ROOT ? summary : `${summary} (root ${relative(REPO_ROOT, root)})`);
