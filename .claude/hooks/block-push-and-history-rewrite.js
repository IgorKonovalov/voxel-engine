#!/usr/bin/env node
// PreToolUse hook: deny every form of history rewrite, local or remote: `git reset --hard`,
// `git rebase`, `git commit --amend`, `git filter-branch`, and a forced or deleting `git push`.
// A plain `git push` is allowed (the owner's call, 2026-10-16). See CLAUDE.md -> "Commit
// hygiene". Wired up in .claude/settings.json under hooks.PreToolUse with matcher
// "Bash|PowerShell".
//
// What is judged is the COMMAND POSITION of each simple command, not a substring: the line is
// split on shell separators outside quotes, heredoc and PowerShell here-string bodies are dropped
// first, and a `bash -c "..."` / `powershell -Command "..."` wrapper is unwrapped and judged too.
// So a commit message that mentions `git rebase`, `git stash push` and `git log origin/main` all
// pass. The bite checks live in .claude/hooks/hooks.test.mjs.

const { readFileSync } = require("fs");

function dropBodies(cmd) {
  return cmd
    .replace(/@'[\s\S]*?\n'@/g, "''")
    .replace(/@"[\s\S]*?\n"@/g, '""')
    .replace(/<<-?\s*(['"]?)(\w+)\1([^\n]*)\n[\s\S]*?\n[ \t]*\2[ \t]*(?=\n|$)/g, "$3");
}

// Splits on && || ; | & and newlines that sit outside single or double quotes.
function splitOutsideQuotes(cmd) {
  const out = [];
  let cur = "";
  let quote = null;
  for (let i = 0; i < cmd.length; i++) {
    const c = cmd[i];
    if (quote) {
      cur += c;
      if (c === quote) quote = null;
      continue;
    }
    if (c === "'" || c === '"') {
      quote = c;
      cur += c;
      continue;
    }
    if (c === ";" || c === "\n" || c === "|" || c === "&") {
      // A redirection (`2>&1`, `&>`) is not a separator.
      if (c === "&" && (cmd[i - 1] === ">" || cmd[i + 1] === ">")) {
        cur += c;
        continue;
      }
      // A PowerShell call operator (`& git push`) is a prefix, not a separator.
      if (c === "&" && cur.trim() === "" && cmd[i + 1] !== "&") continue;
      out.push(cur);
      cur = "";
      if ((c === "|" || c === "&") && cmd[i + 1] === c) i++;
      continue;
    }
    cur += c;
  }
  out.push(cur);
  return out;
}

const WRAPPERS = [
  /^(?:bash|sh|zsh|dash)(?:\.exe)?\s+(?:-\w+\s+)*-\w*c\s+(["'])([\s\S]*)\1\s*$/i,
  /^(?:powershell|pwsh)(?:\.exe)?\s+(?:-\w+\s+)*-(?:Command|c)\s+(["'])([\s\S]*)\1\s*$/i,
  /^cmd(?:\.exe)?\s+\/[cC]\s+(["'])([\s\S]*)\1\s*$/,
];

function simpleCommands(cmd, depth = 0) {
  const result = [];
  for (const raw of splitOutsideQuotes(dropBodies(cmd))) {
    let seg = raw.trim().replace(/^[({!\s]+/, "");
    seg = seg.replace(/^(?:\w+=\S*\s+)+/, "").replace(/^(?:time|exec|command)\s+/, "");
    if (!seg) continue;
    const wrapped = depth < 3 && WRAPPERS.map((re) => seg.match(re)).find(Boolean);
    if (wrapped) result.push(...simpleCommands(wrapped[2], depth + 1));
    else result.push(seg);
  }
  return result;
}

const GIT =
  /^git(?:\.exe)?((?:\s+(?:(?:-C|-c|--git-dir|--work-tree|--namespace)(?:=|\s+)(?:"[^"]*"|'[^']*'|\S+)|--no-pager|-P|--bare))*)\s+([\w-]+)(.*)$/s;

function offending(seg) {
  const m = seg.match(GIT);
  if (!m) return null;
  const sub = m[2];
  const rest = m[3].replace(/"[^"]*"|'[^']*'/g, '""');
  if (sub === "rebase" || sub === "filter-branch") return `git ${sub}`;
  if (sub === "push") {
    // A forced push rewrites the remote; a deleting push (`--delete`, `-d`, or an empty-source
    // refspec `:branch`) removes it. Both are denied; a plain push is not.
    if (/(^|\s)--force(?:-with-lease|-if-includes)?(=|\s|$)/.test(rest)) return "git push --force";
    if (/(^|\s)-[a-zA-Z]*f[a-zA-Z]*(\s|$)/.test(rest)) return "git push -f";
    if (/(^|\s)(?:--delete|-[a-zA-Z]*d[a-zA-Z]*)(\s|$)/.test(rest)) return "git push --delete";
    if (/(^|\s):\S/.test(rest)) return "git push :refspec";
    return null;
  }
  if (sub === "reset" && /(^|\s)--hard(\s|$)/.test(rest)) return "git reset --hard";
  if (sub === "commit" && /(^|\s)--amend(\s|=|$)/.test(rest)) return "git commit --amend";
  return null;
}

module.exports = { simpleCommands, offending };

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
          `Blocked "${hit}". History is never rewritten in this repo, locally or on the remote: ` +
          `no reset --hard, no rebase, no commit --amend, no filter-branch, no forced or ` +
          `deleting push. Make a new commit instead of amending, and push it plainly. ` +
          `See CLAUDE.md -> "Commit hygiene".`,
      },
    }),
  );
  process.exit(0);
}
