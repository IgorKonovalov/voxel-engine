// Bite checks for the PreToolUse deny-hooks. Run: `node --test ".claude/hooks/*.test.mjs"`
//
// Each case pipes a real hook payload through the hook as a child process, the
// same way Claude Code invokes it, so a broken require/JSON path fails here too.
// Edit with the Write/Edit tools: the attribution literals below would trip the
// attribution hook if this file were written through the Bash tool.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));

function run(hook, command) {
  const r = spawnSync('node', [join(here, hook)], {
    input: JSON.stringify({ tool_name: 'Bash', tool_input: { command } }),
    encoding: 'utf8',
  });
  assert.equal(r.status, 0, r.stderr);
  const out = JSON.parse(r.stdout);
  return out.hookSpecificOutput?.permissionDecision === 'deny' ? 'deny' : 'allow';
}

const cases = {
  'block-broad-git-add.js': {
    deny: [
      'git add -A',
      'git add .',
      'git add --all',
      'git add :/',
      'cargo test && git add . && git commit -m x',
    ],
    allow: [
      'git add crates/vxe-world/src/lib.rs',
      'git add ./Cargo.toml docs/plans/0001-x.md',
      'git status',
    ],
  },
  'block-attribution-trailers.js': {
    deny: [
      'git commit -m "feat: x" -m "Co-Authored-By: Claude <noreply@anthropic.com>"',
      "git commit -F - <<'EOF'\nfeat: x\n\nCo-Authored-By: Claude Fable <noreply@anthropic.com>\nEOF",
      'gh pr create --title x --body "Generated with [Claude Code](https://claude.com/claude-code)"',
      'git tag -a v1.0.0 -m "Claude-Session: abc"',
    ],
    allow: [
      'git commit -m "feat(world): add brick map"',
      'git log --grep=Co-Authored-By',
      'grep -n co-authored-by .claude/hooks/block-attribution-trailers.js',
    ],
  },
  'block-push-and-history-rewrite.js': {
    deny: [
      'git push --force',
      'git push -f origin main',
      'git push --force-with-lease origin main',
      'git push origin --delete feature',
      'git push origin :feature',
      'git commit --amend --no-edit',
      'git reset --hard HEAD~1',
      'git rebase main',
      'cargo test && git rebase -i HEAD~3',
      'bash -c "git push --force"',
    ],
    allow: [
      'git push',
      'git push origin main',
      'git push -u origin main',
      'cargo test && git push',
      'bash -c "git push"',
      'git commit -m "docs: mention that git rebase is denied"',
      'git stash push',
      'git log origin/main..HEAD',
      'git reset crates/vxe-world/src/lib.rs',
    ],
  },
  'block-compositor-window-close.js': {
    deny: [
      // The two commands that closed the VS Code window on 2026-10-16.
      `hyprctl dispatch "hl.dsp.window.close('address:0x55a08c4018f0')"`,
      `hyprctl dispatch 'hl.dsp.window.close({ title = "^nonexistentwindow$" })'`,
      "hyprctl dispatch closewindow 'title:^flythrough$'",
      'hyprctl dispatch killactive',
      'hyprctl dispatch forcekillactive',
      'hyprctl dispatch exit',
      'hyprctl -i 0 dispatch killactive',
      'hyprctl kill',
      'sleep 5; hyprctl dispatch closewindow address:0x1',
      'bash -c "hyprctl dispatch killactive"',
      'wlrctl toplevel close title:flythrough',
      'swaymsg kill',
      'xdotool windowclose 12345',
      'wmctrl -ic 0x1',
    ],
    allow: [
      'hyprctl clients -j',
      'hyprctl activewindow',
      'hyprctl dispatch togglespecialworkspace vxe',
      'hyprctl dispatch focuswindow address:0x1',
      `hyprctl dispatch "hl.dsp.window.move({ workspace = '1', window = 'address:0x1' })"`,
      'hyprctl dispatch movetoworkspacesilent "special:vxe,title:^flythrough$"',
      'hyprctl monitors | grep dpms',
      'grep -n closewindow .claude/hooks/block-compositor-window-close.js',
      'pkill -f target/release/flythrough',
      'kill -INT 12345',
      './target/release/flythrough --frames 120',
    ],
  },
};

for (const [hook, { deny, allow }] of Object.entries(cases)) {
  for (const cmd of deny) {
    test(`${hook} denies: ${cmd.split('\n')[0]}`, () => assert.equal(run(hook, cmd), 'deny'));
  }
  for (const cmd of allow) {
    test(`${hook} allows: ${cmd.split('\n')[0]}`, () => assert.equal(run(hook, cmd), 'allow'));
  }
}
