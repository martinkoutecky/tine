#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
export const previewPath = "artifacts/compare-page/compare.preview.html";
export const fixturePath = "scripts/fixtures/compare-page.html";
export const publishedPath = "website/compare.html";
const escape = (value) => String(value).replace(/[&<>"']/g, (c) => ({
  "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
})[c]);
const requireText = (value, context) => {
  if (typeof value !== "string" || !value.trim()) throw new Error(`${context}: expected nonempty text`);
};
const read = (relative, base) => {
  requireText(relative, "evidence path");
  const absolute = path.resolve(base, relative);
  if (!absolute.startsWith(`${base}${path.sep}`)) throw new Error(`Evidence escapes repository: ${relative}`);
  return fs.readFileSync(absolute, "utf8");
};

// Human review establishes that evidence supports the wording. This validates
// its location, not the truth of arbitrary prose. Never fetch URLs during builds.
export function renderComparison({ base = root, data, version } = {}) {
  data ??= JSON.parse(read("website/compare/rows.json", base));
  version ??= JSON.parse(read("package.json", base)).version;
  requireText(version, "version");
  if (data.schemaVersion !== 1) throw new Error("Unsupported comparison schema");
  const expected = ["tine", "logseq", "db", "obsidian"];
  if (JSON.stringify(data.columns.map((c) => c.id)) !== JSON.stringify(expected)) {
    throw new Error("Expected Tine, Logseq file, Logseq DB and Obsidian columns");
  }
  for (const column of data.columns) requireText(column.label, "column label");
  if (!Array.isArray(data.rows) || !data.rows.length) throw new Error("Expected comparison rows");
  const ids = new Set();
  const sections = new Map();
  for (const row of data.rows) {
    for (const field of ["id", "section", "feature"]) requireText(row[field], field);
    if (ids.has(row.id)) throw new Error(`Duplicate comparison id: ${row.id}`);
    ids.add(row.id);
    const cells = data.columns.map(({ id }) => {
      const cell = row[id];
      if (id === "tine") {
        requireText(cell?.text, `${row.id}: Tine text`);
        if (cell.claim === true) {
          requireText(cell.reason, `${row.id}: claim reason`);
          if (cell.evidence) throw new Error(`${row.id}: choose evidence or claim`);
          return `${escape(cell.text)}<br><small>Claim: ${escape(cell.reason)}</small>`;
        }
        if (cell.claim !== undefined) throw new Error(`${row.id}: invalid claim flag`);
        const evidence = cell.evidence;
        const source = read(evidence?.path, base);
        if (evidence.path.startsWith("website/guide/") && evidence.path.endsWith(".html")) {
          requireText(evidence.anchor, `${row.id}: Guide anchor`);
          requireText(evidence.contains, `${row.id}: evidence excerpt`);
          const marker = `id="${evidence.anchor}"`;
          const index = source.indexOf(marker);
          if (index < 0) throw new Error(`${row.id}: missing Guide anchor ${evidence.anchor}`);
          const next = source.indexOf('<li id="', index + marker.length);
          const block = source.slice(index, next < 0 ? undefined : next).replace(/<[^>]*>/g, "");
          if (!block.toLowerCase().includes(evidence.contains.toLowerCase())) {
            throw new Error(`${row.id}: Guide evidence excerpt changed`);
          }
          return `${escape(cell.text)}<br><small><a href="${escape(evidence.path.slice(8))}#${escape(evidence.anchor)}">Guide</a></small>`;
        }
        if (!/(?:^scripts\/(?:e2e-|cli-).*\.mjs$|(?:\.test\.|\/tests\/))/.test(evidence.path)) {
          throw new Error(`${row.id}: expected Guide page or test path`);
        }
        if (evidence.contains && !source.includes(evidence.contains)) throw new Error(`${row.id}: test evidence excerpt changed`);
        return `${escape(cell.text)}<br><small><a href="https://github.com/martinkoutecky/tine/blob/v${encodeURIComponent(version)}/${escape(evidence.path)}">Test</a></small>`;
      }
      // Missing/unverified competitor cells never inherit the old status glyph.
      if (!cell || cell.unknown === true) return "Unknown";
      requireText(cell.text, `${row.id}/${id}: text`);
      const url = new URL(cell.source);
      const primary = id === "obsidian"
        ? ["help.obsidian.md", "obsidian.md"].includes(url.hostname)
          || (url.hostname === "github.com" && url.pathname.startsWith("/obsidianmd/"))
        : ["docs.logseq.com", "logseq.com"].includes(url.hostname)
          || (url.hostname === "github.com" && url.pathname.startsWith("/logseq/"));
      if (url.protocol !== "https:" || url.username || url.password || !primary) throw new Error(`${row.id}/${id}: expected primary HTTPS source`);
      if (!/^\d{4}-\d{2}-\d{2}$/.test(cell.checked ?? "")
        || !Number.isFinite(Date.parse(cell.checked))
        || new Date(cell.checked).toISOString().slice(0, 10) !== cell.checked) {
        throw new Error(`${row.id}/${id}: expected checked date YYYY-MM-DD`);
      }
      return `${escape(cell.text)}<br><small><a href="${escape(cell.source)}">Source</a> · ${cell.checked}</small>`;
    });
    if (!sections.has(row.section)) sections.set(row.section, []);
    sections.get(row.section).push(`          <tr><th scope="row" class="feat">${escape(row.feature)}</th>${cells.map((cell) => `<td class="note">${cell}</td>`).join("")}</tr>`);
  }
  const tables = [...sections].map(([section, rows]) => `      <h2 class="cmp-h">${escape(section)}</h2>
      <div class="tbl-wrap"><table class="cmp">
        <thead><tr><th scope="col">Feature</th>${data.columns.map((c) => `<th scope="col">${escape(c.label)}</th>`).join("")}</tr></thead>
        <tbody>
${rows.join("\n")}
        </tbody>
      </table></div>`).join("\n\n");
  return read("website/compare/template.html", base)
    .replace("{{TABLES}}", tables).replaceAll("{{VERSION}}", escape(version));
}

export function main(args = process.argv.slice(2)) {
  const modes = ["--preview", "--write", "--check", "--update-fixture"];
  if (args.length > 1 || (args.length && !modes.includes(args[0]))) {
    throw new Error(`Usage: node scripts/build-compare-page.mjs [${modes.join(" | ")}]`);
  }
  const mode = args[0] ?? "--preview";
  const html = renderComparison(); // Validate before any write.
  if (mode === "--check") {
    if (read(publishedPath, root) !== html) throw new Error("Published comparison is stale; regenerate at the stable cut with npm run compare:build");
    console.log("Published comparison matches checked-in data and version");
    return;
  }
  const output = mode === "--write" ? publishedPath : mode === "--update-fixture" ? fixturePath : previewPath;
  fs.mkdirSync(path.dirname(path.join(root, output)), { recursive: true });
  // Let the standalone development preview resolve the unchanged website CSS,
  // assets and Guide links. Published output and its fixture use normal URLs.
  fs.writeFileSync(path.join(root, output), mode === "--preview"
    ? html.replace("<head>", '<head>\n<base href="../../website/">') : html);
  console.log(`Comparison generated: ${output}`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
