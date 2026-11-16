#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

// These files retain release/evidence history or the unchanged app identity.
export function historicalFile(file) {
  return /(^|\/)(CHANGELOG[^/]*|RECEIPT[^/]*)$/.test(file)
    || /^(docs\/(adr|releases|notes)\/|artifacts\/|evidence\/|archive\/)/.test(file)
    || /(^|\/)\d{4}-\d{2}-\d{2}[^/]*(\/|$)/.test(file)
    || ["docs/app-identity.md", "tests/regressions/non-ui.json",
      "tests/ui-regressions/catalog.json", "scripts/check-branch-name-refs.test.mjs"].includes(file);
}

// Boundaries preserve origin/og-topic, tine-og, parity paths and branch prefixes.
const rules = [
  /\b(?:origin|refs\/heads)\/og(?![\w/-])/g,
  /--(?:base|line)(?:=|\s+)["']?og(?![\w/-])/g,
  /\bdefault\s*:\s*["']?og(?![\w/-])/g,
  /\bbranches(?:-ignore)?\s*:\s*(?:\[[^\]]*?(?<![\w/-])["']?og(?![\w/-])|\n(?:[ \t]*(?:#[^\n]*)?\n|[ \t]+-\s*[^\n]*\n)*?[ \t]+-\s*["']?og(?![\w/-]))/g,
  /\b(?:fetch|push|pull|merge|switch|checkout|rebase)\s+(?:-[\w-]+\s+)*(?:origin\s+)?["']?og(?![\w/-])/g,
  /["'](?:origin|--base|--line)["']\s*,\s*["']og["']/g,
  /["']--(?:base|line)["'][^\n]*?\bdefault\s*=\s*["']og["']/g,
  /\b(?:base|target|line|DEFAULT_BRANCH|LINE_BRANCHES)\s*(?::[^=\n]+)?(?:==|!=|=|:)\s*(?:\(\s*)?["']og["']/g,
];

export function legacyBranchReferences(text) {
  const hits = new Map();
  for (const rule of rules) {
    for (const match of text.matchAll(rule)) {
      const line = text.slice(0, match.index).split("\n").length;
      hits.set(line, text.split("\n")[line - 1].trim());
    }
  }
  return [...hits].map(([line, text]) => ({ line, text }));
}

export function checkFiles(root, files, allow = historicalFile) {
  const failures = [];
  for (const file of [...new Set(files)].sort()) {
    if (allow(file)) continue;
    const absolute = path.join(root, file);
    // A deleted tracked file or a submodule has no source text to inspect.
    if (!fs.existsSync(absolute) || !fs.statSync(absolute).isFile()) continue;
    const text = fs.readFileSync(absolute, "utf8");
    if (text.includes("\0")) continue;
    for (const hit of legacyBranchReferences(text)) failures.push(`${file}:${hit.line}: ${hit.text}`);
  }
  return failures;
}

export function report(failures, label) {
  if (failures.length) {
    console.error(`Old development branch references in ${label}:\n${failures.join("\n")}`);
    return 1;
  }
  console.log(`Branch name guard passed (${label}).`);
  return 0;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
  const files = execFileSync("git", ["ls-files", "--cached", "--others", "--exclude-standard", "-z"], { cwd: root })
    .toString().split("\0").filter(Boolean);
  process.exitCode = report(checkFiles(root, files), "repository");
}
