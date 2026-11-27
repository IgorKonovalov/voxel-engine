// Tests for scripts/package-tester.sh: an archive packed from stand-in binaries holds the
// fly-through's assets beside them, as apps/flythrough/src/assets.rs resolves them. Packs the
// host's kind, as the release workflow does: on Linux a tar.gz (needs bash, tar and objdump for
// the glibc floor; skipped without objdump), on Windows a zip (Git Bash and 7z, as on the
// release workflow's Windows runner).
// Run: node --test scripts/package-tester.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const assets = join(root, "apps/flythrough/assets");
const windows = process.platform === "win32";
// Git Bash's tools take a Windows path with forward slashes, not with backslashes.
const slashed = (p) => p.replaceAll("\\", "/");

// The archive's file paths, folders left out: tar's listing on Linux, 7z's technical listing
// (a `Path = ` and a `Folder = ` line per entry, after the `----------` line) on Windows.
function listArchive(archive) {
  if (!windows) {
    return execFileSync("tar", ["-tzf", archive], { encoding: "utf8" })
      .split("\n")
      .filter((l) => l && !l.endsWith("/"));
  }
  const text = execFileSync("7z", ["l", "-slt", archive], { encoding: "utf8" }).replaceAll("\r", "");
  const entries = text.slice(text.indexOf("\n----------\n")).split("\n\n");
  return entries
    .filter((e) => /^Folder = -$/m.test(e))
    .map((e) => slashed(e.match(/^Path = (.*)$/m)[1]));
}

// The files a run reads from the assets folder: materials.txt, each entity's <n>/<n>.rig and
// <n>/<n>.vox, each structure's <n>/<n>.vox; read off the source tree, so a new entry joins.
function expectedAssets() {
  const out = ["materials.txt"];
  const dirs = (kind) =>
    readdirSync(join(assets, kind), { withFileTypes: true }).filter((d) => d.isDirectory()).map((d) => d.name);
  for (const n of dirs("entities")) out.push(`entities/${n}/${n}.rig`, `entities/${n}/${n}.vox`);
  for (const n of dirs("structures")) out.push(`structures/${n}/${n}.vox`);
  return out.sort();
}

test("a_packed_archive_ships_the_assets_beside_the_binaries", (t) => {
  if (!windows && spawnSync("objdump", ["--version"]).status !== 0) {
    t.skip("no objdump");
    return;
  }
  const tmp = mkdtempSync(join(tmpdir(), "svet-package-"));
  try {
    const bins = join(tmp, "bin");
    mkdirSync(bins);
    const exe = windows ? ".exe" : "";
    for (const b of ["svet-tester", "flythrough"]) {
      if (windows) {
        // The Windows kind only copies the binaries: any file with the name stands in.
        writeFileSync(join(bins, b + exe), "stand-in\n");
      } else {
        // The Linux kind reads the binaries' glibc symbols: any dynamically linked ELF stands in.
        copyFileSync(existsSync("/usr/bin/true") ? "/usr/bin/true" : "/bin/true", join(bins, b));
      }
    }
    const out = join(tmp, "out");
    const script = slashed(join(root, "scripts/package-tester.sh"));
    execFileSync("bash", [script, windows ? "windows" : "linux", slashed(out)], {
      env: { ...process.env, SVET_BIN_DIR: slashed(bins) },
      encoding: "utf8",
    });
    // The script prints the archive's path in bash's form (/d/... under Git Bash); the one file
    // in the output folder is that archive.
    const written = readdirSync(out);
    assert.equal(written.length, 1, written.join(", "));
    assert.ok(written[0].endsWith(windows ? "-windows-x86_64.zip" : "-linux-x86_64.tar.gz"), written[0]);
    const list = listArchive(join(out, written[0]));
    const top = list[0].split("/")[0];
    const files = list.map((l) => l.slice(top.length + 1));
    for (const f of [`svet-tester${exe}`, `flythrough${exe}`, "README-TESTER.txt", "LICENSE-MIT", "LICENSE-APACHE"]) {
      assert.ok(files.includes(f), `${f} in ${files.join(", ")}`);
    }
    const packed = files.filter((f) => f.startsWith("assets/")).map((f) => f.slice("assets/".length)).sort();
    assert.deepEqual(packed, expectedAssets());
    assert.ok(packed.includes("entities/walker/walker.rig") && packed.includes("structures/chapel/chapel.vox"));
    assert.ok(!files.some((f) => f.endsWith(".voxtext") || f.endsWith("README.md")), files.join(", "));
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
});
