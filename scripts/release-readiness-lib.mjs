import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";

export const dispositionStatuses = new Set(["update", "current", "not-applicable", "consult"]);

export function releaseSection(changelog, version) {
  const escaped = version.replaceAll(".", "\\.");
  const match = changelog.match(new RegExp(`^## \\[${escaped}\\] - \\d{4}-\\d{2}-\\d{2}\\n([\\s\\S]*?)(?=^## \\[|\\Z)`, "m"));
  return match?.[1] ?? null;
}

export function changelogItems(section) {
  const items = [];
  let heading = null;
  let current = null;
  for (const line of section.split("\n")) {
    const h = line.match(/^### (Added|Changed|Fixed)$/);
    if (h) {
      if (current) items.push(current);
      current = null;
      heading = h[1];
      continue;
    }
    if (/^- /.test(line) && heading) {
      if (current) items.push(current);
      current = { section: heading, text: line.slice(2).trim() };
      continue;
    }
    if (current && /^  \S/.test(line)) current.text += ` ${line.trim()}`;
  }
  if (current) items.push(current);
  return items;
}

export function normalizeItemText(text) {
  return text.replace(/\s+/g, " ").trim();
}

export function validateDisposition(owner, value, problems) {
  if (!value || !dispositionStatuses.has(value.status)) {
    problems.push(`${owner}: invalid disposition`);
    return;
  }
  if (typeof value.reason !== "string" || value.reason.length < 3) problems.push(`${owner}: missing reason`);
  if (!Array.isArray(value.refs)) problems.push(`${owner}: refs must be an array`);
  if (value.status === "update" && value.refs?.length === 0) problems.push(`${owner}: update must reference changed files`);
}

export function auditableSourceFingerprint(root) {
  const roots = ["src", "src-tauri/src", "crates", "fixtures"];
  const individual = ["Cargo.toml", "Cargo.lock", "package.json", "package-lock.json", "src-tauri/tauri.conf.json"];
  const files = [];
  const walk = (relative) => {
    const absolute = path.join(root, relative);
    if (!fs.existsSync(absolute)) return;
    const stat = fs.lstatSync(absolute);
    if (stat.isDirectory()) {
      for (const name of fs.readdirSync(absolute).sort()) walk(path.join(relative, name));
    } else if (stat.isFile()) files.push(relative.replaceAll(path.sep, "/"));
  };
  for (const relative of roots) walk(relative);
  for (const relative of individual) walk(relative);
  const hash = crypto.createHash("sha256");
  for (const relative of [...new Set(files)].sort()) {
    hash.update(relative);
    hash.update("\0");
    hash.update(fs.readFileSync(path.join(root, relative)));
    hash.update("\0");
  }
  return hash.digest("hex");
}
