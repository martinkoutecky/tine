#!/usr/bin/env node
// Real CLI export + Chromium: read-only reference hydration and publication privacy.
// First build: npm run build && cargo build -p tine --features custom-protocol
import assert from "node:assert/strict";
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { chromium } from "playwright";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = path.join(root, "target/artifacts/export-embed-kids");
fs.mkdirSync(artifacts, { recursive: true });
const run = fs.mkdtempSync(path.join(artifacts, "probe-"));
const graph = path.join(run, "graph");
for (const dir of ["graph/pages", "graph/journals", "graph/logseq", "output", "home", "data", "config", "cache", "state", "runtime"])
  fs.mkdirSync(path.join(run, dir), { recursive: true });
const ids = Object.fromEntries(["source", "folded", "nested", "private", "host", "foldHost", "nestedHost", "privateHost", "ref", "query", "pageHost"]
  .map((name, i) => [name, `58900000-0000-4000-8000-${String(i + 1).padStart(12, "0")}`]));
const writePage = (name, text) => fs.writeFileSync(path.join(graph, `pages/${name}.md`), text);
writePage("A", `public:: true

- TODO Published source X [[Target]]
  id:: ${ids.source}
  - Published child Y
    - Published grandchild Z
- Published folded source
  id:: ${ids.folded}
  collapsed:: true
  - Folded child Y
    - Folded grandchild Z
- {{embed ((${ids.source}))}}
  id:: ${ids.nested}
`);
writePage("B", `public:: true

- {{embed ((${ids.source}))}}
  id:: ${ids.host}
- {{embed ((${ids.folded}))}}
  id:: ${ids.foldHost}
- {{embed ((${ids.nested}))}}
  id:: ${ids.nestedHost}
- {{embed ((${ids.private}))}}
  id:: ${ids.privateHost}
- Reference ((${ids.source}))
  id:: ${ids.ref}
- {{query (task TODO)}}
  id:: ${ids.query}
`);
writePage("PageEmbed", `public:: true\n\n- {{embed [[A]]}}\n  id:: ${ids.pageHost}\n`);
writePage("Target", "public:: true\n\n- Linked reference destination\n");
writePage("Private", `public:: false\n\n- PRIVATE CONTENT SENTINEL\n  id:: ${ids.private}\n  - PRIVATE CHILD SENTINEL\n`);
const app = path.resolve(process.env.TINE_APP || path.join(root, "target/debug/tine"));
const env = { ...process.env, HOME: path.join(run, "home"), XDG_DATA_HOME: path.join(run, "data"),
  XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"),
  XDG_STATE_HOME: path.join(run, "state"), XDG_RUNTIME_DIR: path.join(run, "runtime") };
delete env.TINE_GRAPH;
// Replay an earlier fixture export to compare bundles without rebuilding the CLI.
let output = process.env.TINE_EXPORTED_SITE && path.resolve(process.env.TINE_EXPORTED_SITE);
if (!output) {
  const exported = spawnSync(app, ["export", "live", graph, "--output", path.join(run, "output"), "--name", "Embed fixture", "--home", "B"],
    { cwd: root, env, encoding: "utf8" });
  fs.writeFileSync(path.join(run, "export.log"), `${exported.stdout}\n${exported.stderr}`);
  assert.equal(exported.status, 0, `CLI export failed: ${exported.stderr}`);
  output = exported.stdout.match(/Published \d+ pages to (.+)/)?.[1]?.trim();
  assert.ok(output, `Missing export receipt: ${exported.stdout}`);
}
const snapshotBytes = fs.readFileSync(path.join(output, "app/snapshot.json"));
const snapshot = JSON.parse(snapshotBytes);
assert.deepEqual(snapshot.pages.map(page => page.name).sort(), ["A", "B", "PageEmbed", "Target"]);
assert.ok(!snapshotBytes.includes("PRIVATE CONTENT SENTINEL"));
assert.ok(!snapshotBytes.includes("PRIVATE CHILD SENTINEL"));
const source = snapshot.pages.find(page => page.name === "A").blocks.find(block => block.raw.includes("Published source X"));
assert.equal(source.children[0].children[0].raw, "Published grandchild Z");
// Exported DTO keys are runtime locators, distinct from authored id:: values.
const runtimeIds = new Map(snapshot.pages.flatMap(page => page.blocks.map(block => [block.raw.match(/\nid:: (\S+)/)?.[1], block.id])));
const staticB = fs.readFileSync(path.join(output, "b.html"), "utf8");
assert.ok(staticB.includes("Published child Y") && staticB.includes("Published grandchild Z"));
assert.ok(!staticB.includes("PRIVATE CONTENT SENTINEL"));

const mime = { ".html": "text/html", ".js": "text/javascript", ".json": "application/json", ".css": "text/css", ".wasm": "application/wasm", ".svg": "image/svg+xml" };
const server = http.createServer((request, response) => {
  const url = new URL(request.url, "http://127.0.0.1");
  const file = path.resolve(output, `.${decodeURIComponent(url.pathname)}${url.pathname.endsWith("/") ? "index.html" : ""}`);
  if (!file.startsWith(`${output}${path.sep}`)) { response.writeHead(403).end(); return; }
  try { response.writeHead(200, { "Content-Type": mime[path.extname(file)] || "application/octet-stream" }); response.end(fs.readFileSync(file)); }
  catch { response.writeHead(404).end(); }
});
await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
  binarySha256: process.env.TINE_EXPORTED_SITE ? null : createHash("sha256").update(fs.readFileSync(app)).digest("hex"),
  exportedSite: output,
  snapshotSha256: createHash("sha256").update(snapshotBytes).digest("hex"),
  journey: "tine export live → published block/page/nested embeds, folds, preview, query and Linked References",
  expected: "Public source descendants render and expand without publishing private content", checks: [], errors: [], outcome: "pending" };
let browser;
let page;
try {
  browser = await chromium.launch({ env, args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  page = await browser.newPage({ viewport: { width: 1280, height: 1100 } });
  // This proof must stay local even if an export accidentally adds a remote dependency.
  await page.route("**/*", route => new URL(route.request().url()).origin === origin ? route.continue() : route.abort());
  page.on("pageerror", error => proof.errors.push(String(error)));
  const open = async name => {
    await page.goto(`${origin}/app/#/page/${name}`);
    await page.locator(".page-title").filter({ hasText: name }).first().waitFor();
  };
  const host = id => {
    assert.ok(runtimeIds.has(id), `Fixture host not exported: ${id}`);
    return page.locator(`.ls-block[data-block-id="${runtimeIds.get(id)}"]`).first();
  };
  // Sticky ancestor labels duplicate row text in hidden presentation elements.
  const visibleText = (scope, text) => scope.getByText(text, { exact: true }).filter({ visible: true });
  const contains = async (scope, text) => {
    await visibleText(scope, text).first().waitFor({ state: "visible", timeout: 8000 });
  };
  await open("B");
  await contains(host(ids.host), "Published child Y");
  await contains(host(ids.host), "Published grandchild Z");
  proof.checks.push("block embed children + grandchildren (source page not visited)");
  const folded = host(ids.foldHost);
  await contains(folded, "Published folded source");
  assert.equal(await visibleText(folded, "Folded child Y").count(), 0);
  await folded.locator(".collapse-toggle").first().click();
  await contains(folded, "Folded child Y");
  await contains(folded, "Folded grandchild Z");
  await folded.locator(".collapse-toggle").first().click();
  assert.equal(await visibleText(folded, "Folded child Y").count(), 0);
  proof.checks.push("authored collapsed source expands and collapses");
  await contains(host(ids.nestedHost), "Published grandchild Z");
  proof.checks.push("nested block embed");
  assert.ok(!(await page.locator("body").innerText()).includes("PRIVATE CONTENT SENTINEL"));
  proof.checks.push("private source absent from snapshot, static HTML and app");
  const query = host(ids.query);
  await contains(query, "Published child Y");
  // Query disclosure may fold the next level. Open it when needed.
  if (!(await visibleText(query, "Published grandchild Z").first().isVisible()))
    await query.locator(".collapse-toggle").last().click();
  await contains(query, "Published grandchild Z");
  proof.checks.push("baked query descendants");
  await host(ids.ref).locator(".block-ref").hover();
  const peek = page.locator(".peek-popup");
  await contains(peek, "Published grandchild Z");
  proof.checks.push("block reference hover preview");
  await page.screenshot({ path: path.join(run, "block-embeds.png"), fullPage: true });
  await open("PageEmbed");
  await contains(host(ids.pageHost), "Published child Y");
  await contains(host(ids.pageHost), "Published grandchild Z");
  proof.checks.push("page embed");
  await page.screenshot({ path: path.join(run, "page-embed.png"), fullPage: true });
  await open("Target");
  const references = page.locator(".linked-references");
  await contains(references, "Published child Y");
  proof.checks.push("Linked References descendants");
  assert.deepEqual(proof.errors, []);
  assert.deepEqual(fs.readFileSync(path.join(output, "app/snapshot.json")), snapshotBytes);
  proof.outcome = "pass";
  console.log(`PASS: ${proof.checks.join("; ")}\nArtifacts: ${run}`);
} catch (error) {
  proof.outcome = "fail";
  proof.observed = String(error);
  proof.classification = "ambiguous"; // Classify using failure.png and observed before reporting.
  if (page) {
    await page.screenshot({ path: path.join(run, "failure.png"), fullPage: true }).catch(() => {});
    fs.writeFileSync(path.join(run, "failure.html"), await page.content());
  }
  throw error;
} finally {
  fs.writeFileSync(path.join(run, "proof.json"), `${JSON.stringify(proof, null, 2)}\n`);
  await browser?.close();
  await new Promise(resolve => server.close(resolve));
}
