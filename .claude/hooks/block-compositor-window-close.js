#!/usr/bin/env node
// PreToolUse hook: deny closing or killing windows through the compositor (`hyprctl dispatch
// closewindow|killactive|exit|hl.dsp.window.close(...)`, `hyprctl kill`, and the wlroots, sway
// and X11 equivalents). Wired up in .claude/settings.json under hooks.PreToolUse with matcher
// "Bash|PowerShell".
//
// Why: on 2026-10-16 two dev agents checking the fly-through's close path asked Hyprland 0.56
// to close the fly-through window by address and by title. Both times the compositor closed
// the focused window instead: the VS Code window hosting the orchestrator and every agent.
// A window-close check belongs to the human phase; an agent ends its own windowed run through
// the process (`--frames N`, or a signal to the pid it spawned).
//
// Judged per simple command, the same way as the other hooks (shared splitter). Reading or
// moving windows through the compositor stays allowed: `hyprctl clients`, `focuswindow`,
// `movetoworkspace`, `togglespecialworkspace`.

const { readFileSync } = require("fs");
const { simpleCommands } = require("./block-push-and-history-rewrite.js");

// Each entry: the command at command position, and the pattern that makes it a close or kill.
// The token boundary lets `hl.dsp.window.close(` and `"closewindow"` match while
// `focuswindow`, `closeall` style names and `hyprctl clients` do not.
const TOKEN = (words) =>
  new RegExp(`(?:^|[\\s"'(.,=])(?:${words})(?=$|[\\s"'().,{}])`, "i");

const RULES = [
  { cmd: /^hyprctl(?:\.exe)?(?:\s|$)/i, bad: TOKEN("closewindow|killactive|forcekillactive|close|kill|exit") },
  { cmd: /^wlrctl(?:\s|$)/i, bad: TOKEN("close|kill") },
  { cmd: /^(?:swaymsg|i3-msg)(?:\s|$)/i, bad: TOKEN("kill|exit") },
  { cmd: /^xdotool(?:\s|$)/i, bad: TOKEN("windowclose|windowkill|windowquit") },
  { cmd: /^wmctrl(?:\s|$)/i, bad: /(^|\s)-i?c(\s|$)/ },
];

function offending(seg) {
  for (const { cmd, bad } of RULES) {
    const m = seg.match(cmd);
    if (!m) continue;
    const rest = seg.slice(m[0].length);
    if (bad.test(rest)) return `${seg.split(/\s+/)[0]} ${rest.match(bad)[0].replace(/^[^\w-]+/, "")}`;
  }
  return null;
}

module.exports = { offending };

if (require.main === module) {
  const input = JSON.parse(readFileSync(0, "utf8") || "{}");
  const cmd = (input.tool_input && input.tool_input.command) || "";
  const hit = simpleCommands(cmd).map(offending).find(Boolean);
  if (!hit) {
    process.stdout.write("{}");
    process.exit(0);
  }
  process.stdout.write(
    JSON.stringify({
      hookSpecificOutput: {
        hookEventName: "PreToolUse",
        permissionDecision: "deny",
        permissionDecisionReason:
          `Blocked "${hit}". Agents never close or kill windows through the compositor: on ` +
          `2026-10-16 Hyprland closed the focused VS Code window instead of the fly-through ` +
          `twice, killing every session. End your own windowed run through the process ` +
          `(--frames N, or a signal to the pid you spawned). A window-close check is a human ` +
          `phase step: ask the owner.`,
      },
    }),
  );
  process.exit(0);
}
