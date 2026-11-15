// Mount actual Rust-generated live and generation-1 quick-share snapshots in
// the shipped viewer. All HTTP is local or intercepted; nothing reaches a gist.
import fs from "node:fs";
import path from "node:path";
import http from "node:http";
import assert from "node:assert/strict";
import { chromium } from "playwright";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = process.argv[2] || path.join(root, "target/artifacts/comment-output-1113");
const dist = path.join(root, "dist");
const original = fs.readFileSync(path.join(dist, "index.html"), "utf8");
const live = fs.readFileSync(path.join(artifacts, "live-included-snapshot.json"));
const omitted = fs.readFileSync(path.join(artifacts, "live-omitted-snapshot.json"));
const envelope = fs.readFileSync(path.join(artifacts, "share-envelope.txt"), "utf8");
const key = Buffer.from("tine-quick-share-fixture-key-32b").toString("base64url");
const gist = "1234567890abcdef1234567890abcdef";
const mime = { ".js": "text/javascript", ".css": "text/css", ".wasm": "application/wasm", ".svg": "image/svg+xml", ".woff2": "font/woff2" };
const server = http.createServer((request, response) => {
  const url = new URL(request.url, "http://localhost");
  if (url.pathname === "/snapshot.json") { response.setHeader("Content-Type", "application/json"); response.end(url.searchParams.has("omitted") ? omitted : live); return; }
  if (url.pathname.endsWith(".html") || url.pathname === "/") {
    const mode = url.searchParams.get("mode");
    const snapshot = mode === "share" ? "quick-share:1" : mode === "omitted" ? "snapshot.json?omitted" : "snapshot.json";
    response.setHeader("Content-Type", "text/html");
    response.end(original.replace("<head>", `<head><meta name="tine-published" content="${snapshot}">`)); return;
  }
  const file = path.resolve(dist, `.${decodeURIComponent(url.pathname)}`);
  if (!file.startsWith(`${dist}${path.sep}`) || !fs.existsSync(file) || !fs.statSync(file).isFile()) { response.writeHead(404); response.end(); return; }
  response.setHeader("Content-Type", mime[path.extname(file)] || "application/octet-stream");
  response.end(fs.readFileSync(file));
});
await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
const browser = await chromium.launch();
const proof = [];
try {
  for (const mode of ["live", "share", "omitted"]) {
    const page = await browser.newPage({ viewport: { width: 1800, height: 1000 } });
    const errors = [];
    page.on("pageerror", error => errors.push(String(error)));
    await page.route("**/*", route => {
      const url = new URL(route.request().url());
      if (url.origin === origin) return route.continue();
      if (mode === "share" && url.hostname === "gist.githubusercontent.com") return route.fulfill({ status: 200, contentType: "text/plain", headers: { "access-control-allow-origin": "*" }, body: envelope });
      return route.abort();
    });
    const hash = mode === "share" ? `#fixture/${gist}/${key}/page/Markdown` : "#/page/Markdown";
    await page.goto(`${origin}/index.html?mode=${mode}${hash}`);
    await page.getByText("Body passage", { exact: true }).first().waitFor();
    assert.match(await page.locator(".page-blocks").innerText(), /Body passage/);
    assert.equal(await page.locator("textarea.block-editor, .cm-editor").count(), 0);
    if (mode === "omitted") {
      assert.equal(await page.locator(".comment-card").count(), 0);
      assert.equal(await page.getByText(/COMMENTMARKER|REPLYMARKER/).count(), 0);
    } else {
      await page.locator(".margin-column .comment-card").first().waitFor();
      assert.match(await page.locator(".margin-column").innerText(), /COMMENTMARKER/);
      assert.match(await page.locator(".margin-column").innerText(), /REPLYMARKER/);
      assert.equal(await page.locator(".page-blocks .comment-card").count(), 0);
    }
    assert.deepEqual(errors, []);
    await page.screenshot({ path: path.join(artifacts, `viewer-${mode}.png`) });
    proof.push({ mode, marginCards: await page.locator(".margin-column .comment-card").count(), bodyIntact: true, errors });
    await page.close();
  }
  fs.writeFileSync(path.join(artifacts, "viewer-proof.json"), JSON.stringify(proof, null, 2));
  console.log(JSON.stringify(proof));
} finally {
  try { await browser.close(); }
  finally { await new Promise(resolve => server.close(resolve)); }
}
