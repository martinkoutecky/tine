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
  const pluginRoots = ["plugin-sdk", "community-plugins"];
  const roots = ["src", "src-tauri/src", "crates", "fixtures", ...pluginRoots];
  const individual = ["Cargo.toml", "Cargo.lock", "package.json", "package-lock.json", "src-tauri/tauri.conf.json"];
  // These files attest to the signed native test journey; they are not inputs
  // to the shipped application or to the production behavior audited by the
  // v0.6.0 lanes. Keep the exemption explicit so real revocation fixtures and
  // runtime/plugin sources still invalidate the audit fingerprint.
  const auditEvidenceOnly = new Set([
    "fixtures/plugin-revocation/README.md",
    "fixtures/plugin-revocation/fixture.json",
    "fixtures/plugin-revocation/revoked-index.json.sig",
  ]);
  const files = [];
  const walk = (relative) => {
    const absolute = path.join(root, relative);
    if (!fs.existsSync(absolute)) return;
    const stat = fs.lstatSync(absolute);
    if (stat.isDirectory()) {
      // Cargo build output is local evidence, never audited source. This applies
      // both to community plugins and to Rust-backed fixtures; otherwise a
      // warmed developer checkout hashes differently from a clean CI checkout.
      if (path.basename(relative) === "target") return;
      for (const name of fs.readdirSync(absolute).sort()) walk(path.join(relative, name));
    } else if (stat.isFile()) {
      const normalized = relative.replaceAll(path.sep, "/");
      if (!auditEvidenceOnly.has(normalized)) files.push(normalized);
    }
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
