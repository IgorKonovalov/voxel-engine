#!/usr/bin/env node
// Asserts that every relative markdown link in the repo resolves: the file exists, and a
// `#fragment` on a markdown target names a heading of that file.
//
// Run at every plan close (architect close ceremony): moving a plan into
// docs/plans/done/ breaks inbound links to its old path and every `../` link
// inside it. A fragment is checked when its target is a markdown file (or the link is a pure
// `#fragment`, which targets the file it is in): it must equal a heading's slug or an HTML
// `id`/`name` anchor of that file. Slugs follow GitHub's rule: the heading's text (a link
// reduced to its label), lowercased, every character that is not a letter, digit, mark,
// `_`, `-` or space dropped, each space a `-`, and a repeated slug suffixed `-1`, `-2`, ...
// in order. Fragments are compared lowercased (GitHub's anchors are lowercase). A fragment on
// any other file (`main.rs#L10`) is not checked. External links (scheme:, //host) are
// skipped, as are fenced code blocks and inline code spans. Exit 0 = all resolve, 1 = broken
// links printed as `file:line -> target`, with the reason for a fragment.
//
// The files are the markdown git tracks plus untracked files it would track, so a new plan is
// checked before it's staged and gitignored files (conductor state, node_modules) never are.
//
// Usage:  node scripts/check-doc-links.mjs [root]
// The optional `root` scans another tree: scripts/fixtures/doc-links is the seeded one, and
// scripts/fixtures/README.md names its expected findings. The seeded trees (the
// subdirectories of scripts/fixtures) are skipped by path on an ordinary run.

import { execFileSync } from 'node:child_process';
import { readFileSync, existsSync, statSync } from 'node:fs';
import { join, dirname, resolve, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

// fileURLToPath, not URL.pathname: the latter yields "/D:/..." on Windows, which is not a
// directory, and git would be spawned with a missing cwd (ENOENT).
const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const root = resolve(process.argv[2] ?? REPO_ROOT);
const SEEDED = resolve(REPO_ROOT, 'scripts', 'fixtures');

/** True for a file inside a seeded tree that the run is not rooted in. */
function inSeeded(abs) {
  if (root === SEEDED || root.startsWith(SEEDED + sep)) return false;
  return abs.startsWith(SEEDED + sep) && dirname(abs) !== SEEDED;
}

function markdownFiles() {
  const out = execFileSync(
    'git',
    ['ls-files', '-z', '--cached', '--others', '--exclude-standard', '--', '*.md'],
    { cwd: root, encoding: 'utf8' },
  );
  // A tracked file deleted from the working tree is still listed by --cached.
  return out
    .split('\0')
    .filter(Boolean)
    .map((path) => join(root, path))
    .filter((file) => existsSync(file) && !inSeeded(file));
}

// A label may hold an image (`[![alt](badge.svg)](#license)`), so the outer link is read too.
const LINK = /\[(?:[^\[\]]|!\[[^\]]*\]\([^)]*\))*\]\(\s*<?([^)\s>]+)>?(?:\s+"[^"]*")?\s*\)|^\s*\[[^\]]+\]:\s*(\S+)/g;
const IMAGE_IN_LABEL = /!\[[^\]]*\]\(\s*<?([^)\s>]+)>?(?:\s+"[^"]*")?\s*\)/g;

/** GitHub's slug of a heading's markdown text (before the repeat suffix). */
function slug(text) {
  return text
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/<[^>]+>/g, '')
    .trim()
    .toLowerCase()
    .replace(/[^\p{L}\p{M}\p{N}_\- ]/gu, '')
    .replace(/ /g, '-');
}

const anchorCache = new Map();
/** The fragments a markdown file answers to: its headings' slugs and its HTML anchors. */
function anchors(file) {
  if (anchorCache.has(file)) return anchorCache.get(file);
  const seen = new Map();
  const set = new Set();
  let inFence = false;
  for (const line of readFileSync(file, 'utf8').split('\n')) {
    if (/^\s*(```|~~~)/.test(line)) {
      inFence = !inFence;
      continue;
    }
    if (inFence) continue;
    const h = /^ {0,3}#{1,6}\s+(.*?)(?:\s+#+)?\s*$/.exec(line);
    if (h) {
      const base = slug(h[1]);
      let s = base;
      let n = seen.get(base) ?? 0;
      while (set.has(s)) s = `${base}-${++n}`;
      seen.set(base, n);
      set.add(s);
    }
    for (const a of line.matchAll(/<a\s[^>]*\b(?:id|name)\s*=\s*"([^"]+)"/gi)) set.add(a[1].toLowerCase());
  }
  anchorCache.set(file, set);
  return set;
}

const broken = [];
let checked = 0;
let fragments = 0;
for (const file of markdownFiles()) {
  let inFence = false;
  readFileSync(file, 'utf8')
    .split('\n')
    .forEach((line, i) => {
      if (/^\s*(```|~~~)/.test(line)) inFence = !inFence;
      if (inFence) return;
      const text = line.replace(/`[^`]*`/g, '');
      const targets = [];
      for (const m of text.matchAll(LINK)) {
        // An image inside a link's label is a link of its own.
        for (const img of m[0].slice(1).matchAll(IMAGE_IN_LABEL)) targets.push(img[1]);
        targets.push(m[1] || m[2]);
      }
      for (const target of targets) {
        if (!target || /^([a-z][a-z0-9+.-]*:|\/\/)/i.test(target)) continue;
        const hash = target.indexOf('#');
        const path = decodeURIComponent(hash < 0 ? target : target.slice(0, hash));
        const fragment = hash < 0 ? '' : decodeURIComponent(target.slice(hash + 1));
        const where = `${relative(root, file)}:${i + 1} -> ${target}`;
        let abs = file;
        if (path) {
          checked++;
          abs = path.startsWith('/') ? join(root, path) : resolve(dirname(file), path);
          if (!existsSync(abs) || (path.endsWith('/') && !statSync(abs).isDirectory())) {
            broken.push(where);
            continue;
          }
        }
        if (!fragment || !abs.endsWith('.md') || statSync(abs).isDirectory()) continue;
        fragments++;
        if (!anchors(abs).has(fragment.toLowerCase())) {
          broken.push(`${where} (no heading or anchor #${fragment} in ${relative(root, abs)})`);
        }
      }
    });
}

if (broken.length) {
  console.error(`check-doc-links: ${broken.length} broken relative link(s):`);
  for (const b of broken) console.error(`  ${b}`);
  console.error(
    'After a `git mv` into docs/plans/done/: inbound `plans/NNNN-...` -> `plans/done/NNNN-...`; ' +
      'outbound links inside the moved plan gain one `../`. A fragment names a heading by ' +
      "GitHub's slug of it; when the heading left, point the link at where its text went.",
  );
  process.exit(1);
}
console.log(`check-doc-links: ${checked} relative link(s) resolve, ${fragments} fragment(s) name a heading.`);
