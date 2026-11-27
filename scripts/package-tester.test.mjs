// Tests for scripts/package-tester.sh: a Linux archive packed from stand-in binaries holds the
// fly-through's assets beside them, as apps/flythrough/src/assets.rs resolves them. Needs bash,
// tar and objdump (the glibc floor); skipped without objdump.
// Run: node --test scripts/package-tester.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const assets = join(root, "apps/flythrough/assets");

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
  if (spawnSync("objdump", ["--version"]).status !== 0) {
    t.skip("no objdump");
    return;
  }
  const tmp = mkdtempSync(join(tmpdir(), "svet-package-"));
  try {
    const bins = join(tmp, "bin");
    mkdirSync(bins);
    // Any dynamically linked ELF stands in for the two binaries: the script copies them and
    // reads their glibc symbols, nothing more.
    const elf = existsSync("/usr/bin/true") ? "/usr/bin/true" : "/bin/true";
    for (const b of ["svet-tester", "flythrough"]) copyFileSync(elf, join(bins, b));
    const out = join(tmp, "out");
    const archive = execFileSync("bash", [join(root, "scripts/package-tester.sh"), "linux", out], {
      env: { ...process.env, SVET_BIN_DIR: bins },
      encoding: "utf8",
    })
      .trim()
      .split("\n")
      .at(-1);
    const list = execFileSync("tar", ["-tzf", archive], { encoding: "utf8" })
      .split("\n")
      .filter((l) => l && !l.endsWith("/"));
    const top = list[0].split("/")[0];
    const files = list.map((l) => l.slice(top.length + 1));
    for (const f of ["svet-tester", "flythrough", "README-TESTER.txt", "LICENSE-MIT", "LICENSE-APACHE"]) {
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
