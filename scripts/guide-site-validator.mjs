import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const deliberateStubTargets = new Set(
  JSON.parse(fs.readFileSync(path.join(root, "docs/guide-deliberate-stubs.json"), "utf8"))
    .map((stub) => stub.file),
);

export function filesUnder(dir, prefix = "") {
  const files = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
    const relative = path.join(prefix, entry.name);
    if (entry.isDirectory()) files.push(...filesUnder(path.join(dir, entry.name), relative));
    else files.push(relative);
  }
  return files;
}

function isDeliberateStubTarget(dir, resolved) {
  const relative = path.relative(dir, resolved);
  return !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative) && deliberateStubTargets.has(relative);
}

export function validateGuideSiteLinks(dir) {
  const failures = [];
  const siteRoot = path.resolve(dir);
  for (const relative of filesUnder(dir).filter((file) => file.endsWith(".html"))) {
    const html = fs.readFileSync(path.join(dir, relative), "utf8");
    for (const match of html.matchAll(/<[^>]+?(?:href|src)="([^"]+)"[^>]*>/g)) {
      const tag = match[0];
      const target = match[1];
      if (/^(?:[a-z]+:|#|\/\/)/i.test(target)) continue;
      const withoutFragment = target.split("#", 1)[0].split("?", 1)[0];
      if (!withoutFragment) continue;
      const resolved = path.resolve(path.dirname(path.join(dir, relative)), decodeURIComponent(withoutFragment));
      const withinSite = resolved.startsWith(`${siteRoot}${path.sep}`);
      if (!withinSite || !fs.existsSync(resolved)) {
        const isRefOrTag = /class="[^"]*\b(?:ref|tag)\b/.test(tag);
        if (isRefOrTag && !fs.existsSync(resolved) && isDeliberateStubTarget(siteRoot, resolved)) continue;
        failures.push(`${relative}: missing local target ${target}`);
      }
    }
  }
  if (failures.length) throw new Error(`Guide link validation failed:\n${failures.join("\n")}`);
}
