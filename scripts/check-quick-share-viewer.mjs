#!/usr/bin/env node
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import http from "node:http";
import { webcrypto } from "node:crypto";
import { gzipSync } from "node:zlib";
import { chromium } from "playwright";
import { fileURLToPath } from "node:url";
import { sha256, validateViewer, fixtureFingerprint } from "./quick-share-viewer-lib.mjs";

const root = path.resolve(import.meta.dirname, "..");
const mime = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".json": "application/json", ".svg": "image/svg+xml", ".woff2": "font/woff2" };
async function sealDocument(document, link) {
  const keyBytes = Buffer.from(new URL(link).hash.split("/")[2], "base64url");
  const header = Buffer.concat([Buffer.from("TSH1"), Buffer.from([1]), Buffer.alloc(16, 7)]);
  const nonce = webcrypto.getRandomValues(new Uint8Array(12));
  const base = await webcrypto.subtle.importKey("raw", keyBytes, "HKDF", false, ["deriveKey"]);
  const key = await webcrypto.subtle.deriveKey({ name: "HKDF", hash: "SHA-256", salt: header.subarray(5), info: Buffer.from("tine-share/1/enc") }, base, { name: "AES-GCM", length: 256 }, false, ["encrypt"]);
  const cipher = await webcrypto.subtle.encrypt({ name: "AES-GCM", iv: nonce, additionalData: header }, key, gzipSync(JSON.stringify(document)));
  return Buffer.concat([header, Buffer.from(nonce), Buffer.from(cipher)]).toString("base64url");
}
export async function serveViewer(directory) {
  const server = http.createServer((req, res) => {
    try {
      const relative = decodeURIComponent(new URL(req.url, "http://localhost").pathname).replace(/^\/s\/1\//, "");
      const file = path.resolve(directory, relative || "index.html");
      if (!file.startsWith(path.resolve(directory) + path.sep)) throw new Error("outside viewer");
      res.writeHead(200, { "content-type": mime[path.extname(file)] ?? "application/octet-stream", "cache-control": "no-store" });
      res.end(fs.readFileSync(file));
    } catch { res.writeHead(404); res.end(); }
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  return { url: `http://127.0.0.1:${server.address().port}/s/1/`, close: () => new Promise((resolve) => server.close(resolve)) };
}

/** The same headless compatibility journey checks local artifacts and deployed
 * files. On the deployed path every file used is checked against viewer.json. */
export async function checkViewer(target, { envelope, link, expectedText } = {}) {
  const remote = /^https:\/\//.test(target);
  const hosted = remote ? null : await serveViewer(path.resolve(target));
  const base = remote ? target : hosted.url;
  let manifest, browser, activePage;
  let activeCase = "startup", activePrefix = "", activeKey = "";
  try {
  if (remote) {
    const response = await fetch(new URL("viewer.json", base), { cache: "no-store", redirect: "error", signal: AbortSignal.timeout(15_000) });
    if (!response.ok) throw new Error("deployed viewer manifest is unavailable");
    manifest = await response.json();
    for (const field of ["generation", "link", "envelope", "document", "snapshot_schema"]) assert.equal(manifest[field], 1, `deployed ${field}`);
  } else manifest = validateViewer(target);
  assert.equal(manifest.fixture_sha256, fixtureFingerprint(root), "viewer was not built for this writer's fixture corpus");
  browser = await chromium.launch({ headless: true });
  const cases = envelope ? [{ name: "native-created", envelope, link, expectedText }] : fs.readdirSync(path.join(root, "tests/fixtures/quick-share/g1")).map((name) => {
    const dir = path.join(root, "tests/fixtures/quick-share/g1", name);
    return { name, envelope: fs.readFileSync(path.join(dir, "tine-share.bin"), "utf8"), link: fs.readFileSync(path.join(dir, "link.txt"), "utf8").trim(), doc: JSON.parse(fs.readFileSync(path.join(dir, "document.json"), "utf8")) };
  });
  if (!envelope) {
    // Golden writer documents are checked against a fresh Rust writer by the
    // generation-1 native test; seal and render those through the browser too.
    for (const fixture of [...cases]) {
      const doc = JSON.parse(fs.readFileSync(path.join(root, "tests/fixtures/quick-share/g1", fixture.name, "writer.json"), "utf8"));
      cases.push({ ...fixture, name: `${fixture.name}-writer`, doc, envelope: await sealDocument(doc, fixture.link) });
    }
    const fixture = cases.find((entry) => entry.name === "pages");
    const doc = structuredClone(fixture.doc);
    doc.assets = { "assets/local.png": { file: "a0000000000000000.bin" } }; // reserved extension is ignored by g1
    doc.snapshot.pages.find((entry) => entry.name === "Project").blocks.push({ id: "media-policy-test", raw: 'Remote media sentinel\n![remote](https://remote.example/pixel.png) ![local](../assets/local.png)\n{{youtube dQw4w9WgXcQ}}\n<iframe src="https://remote.example/frame"></iframe>', children: [] });
    doc.snapshot.pages.find((entry) => entry.name === "Project").blocks.push({ id: "math-policy-test", raw: "$$x^2$$", children: [] });
    cases.push({ ...fixture, name: "media-policy", doc, expectedText: "Remote media sentinel", envelope: await sealDocument(doc, fixture.link) });
  }
    for (const fixture of cases) {
      const context = await browser.newContext();
      const page = await context.newPage();
      activePage = page; activeCase = fixture.name;
      const prefix = new URL(fixture.link).hash.split("/").slice(0, 3).join("/");
      const key = prefix.split("/")[2];
      activePrefix = prefix; activeKey = key;
      let secretRequests = 0, unexpectedRequests = 0;
      page.on("request", (request) => { if (request.url().includes(key) || (request.postData() ?? "").includes(key) || JSON.stringify(request.headers()).includes(key)) secretRequests++; });
      await context.addInitScript(() => {
        window.__shareInitialCspViolations = [];
        document.addEventListener("securitypolicyviolation", (event) => window.__shareInitialCspViolations.push({ directive: event.effectiveDirective, uri: event.blockedURI }));
        const original = window.fetch;
        window.__shareFetchLeak = false;
        window.fetch = function(input, init) {
          const key = location.hash.split("/")[2];
          const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
          if (key && (url.includes(key) || String(init?.body ?? "").includes(key) || JSON.stringify(init?.headers ?? {}).includes(key))) window.__shareFetchLeak = true;
          return original.call(this, input, init);
        };
      });
      await context.route("**/*", async (route) => {
        const url = route.request().url();
        if (url.startsWith("https://gist.githubusercontent.com/")) return route.fulfill({ status: 200, contentType: "text/plain", body: fixture.envelope, headers: { "access-control-allow-origin": "*" } });
        if (url.startsWith(base)) {
          if (!remote) return route.continue();
          const name = decodeURIComponent(url.slice(base.length).split("?")[0]) || "index.html";
          const response = await fetch(url, { redirect: "error", cache: "no-store", signal: AbortSignal.timeout(15_000) });
          const bytes = Buffer.from(await response.arrayBuffer());
          assert.equal(response.ok, true, `deployed file unavailable: ${name}`);
          assert.equal(sha256(bytes), manifest.files?.[name], `deployed checksum mismatch: ${name}`);
          return route.fulfill({ status: 200, contentType: mime[path.extname(name)] ?? "application/octet-stream", body: bytes });
        }
        unexpectedRequests++; return route.abort();
      });
      await page.goto(`${base}${prefix}${fixture.expectedText ? "/page/Project" : ""}`, { waitUntil: "domcontentloaded" });
      await page.locator(".page-title").first().waitFor({ timeout: 30_000 });
      await assert.doesNotReject(() => page.getByLabel("User content · Shared by a Tine user").waitFor());
      if (fixture.expectedText) await page.getByText(fixture.expectedText, { exact: false }).first().waitFor();
      if (fixture.name === "media-policy") {
        assert.equal(await page.locator("iframe,video,audio,img[src*='remote.example']").count(), 0);
        await page.getByText("local.png — not included", { exact: true }).waitFor();
        await page.getByRole("link", { name: "Media from remote.example (open externally)" }).first().waitFor();
      }
      if (fixture.doc) {
        for (const entry of fixture.doc.snapshot.pages) {
          await page.goto(`${base}${prefix}/page/${encodeURIComponent(entry.name)}`, { waitUntil: "domcontentloaded" });
          await page.locator(".page-title").filter({ hasText: entry.title || entry.name }).first().waitFor({ timeout: 30_000 });
          // The actual reader, query renderer, aliases and parser are all running.
          assert.equal(new URL(page.url()).hash.startsWith(prefix), true);
          if (entry.blocks.length && entry.name === "Org page") await page.getByText("org task", { exact: false }).first().waitFor();
        }
      }
      // Follow an actual reader page link, then reload and copy the address.
      await page.goto(`${base}${prefix}/page/Project`, { waitUntil: "domcontentloaded" });
      const next = page.locator('.page-ref').filter({ hasText: "Notes ü" }).first();
      if (!fixture.doc || fixture.doc.snapshot.pages.some((entry) => entry.name === "Notes ü")) {
        await next.waitFor({ timeout: 15_000 });
        await next.click();
        await page.locator(".page-title").filter({ hasText: "Notes ü" }).first().waitFor();
        assert.equal(new URL(page.url()).hash.startsWith(prefix), true);
        await page.reload();
        await page.locator(".page-title").filter({ hasText: "Notes ü" }).first().waitFor();
      }
      if (fixture.doc?.snapshot.pages.some((entry) => entry.name === "Notes ü")) {
        await page.goto(`${base}${prefix}/block/6512a000-0000-4000-8000-000000000002`);
        await page.getByText("block with an id", { exact: false }).first().waitFor();
        assert.equal(new URL(page.url()).hash.startsWith(prefix), true);
      }
      assert.equal(await page.evaluate(() => window.__shareFetchLeak), false, "fragment entered fetch");
      assert.equal(secretRequests, 0, "fragment entered a request");
      assert.equal(unexpectedRequests, 0, "viewer requested an unexpected host");
      assert.deepEqual(await page.evaluate(() => window.__shareInitialCspViolations), [], "the shipped CSP blocked a reader resource");
      // Deliberately exercise CSP behind renderer suppression. A missed image,
      // iframe or media path must fail closed, without reaching its origin.
      await page.evaluate(async () => {
        window.__shareViolations = [];
        document.addEventListener("securitypolicyviolation", (event) => window.__shareViolations.push(event.effectiveDirective));
        for (const tag of ["img", "video", "iframe"]) { const el = document.createElement(tag); el.src = "https://remote.example/probe"; document.body.append(el); }
        const workerUrl = URL.createObjectURL(new Blob(['postMessage("probe")'], { type: "text/javascript" }));
        try { const worker = new Worker(workerUrl); worker.onerror = (event) => event.preventDefault(); } catch { /* CSP may refuse synchronously. */ }
        await new Promise((resolve) => setTimeout(resolve, 200));
      });
      const violations = await page.evaluate(() => window.__shareViolations);
      for (const directive of ["img-src", "media-src", "frame-src", "worker-src"]) assert.ok(violations.includes(directive), `CSP did not block ${directive}`);
      assert.equal(unexpectedRequests, 0, "CSP allowed a remote media request");
      await context.close();
      console.log(`Viewer compatibility OK: ${fixture.name}`);
    }
    if (!remote && !envelope) {
      // Test the actual production renderer with the stricter alternative.
      // KaTeX inserts style attributes and the reader inserts presentation CSS;
      // removing unsafe-inline must produce a style-src violation.
      const fixture = cases.find((entry) => entry.name === "media-policy");
      const context = await browser.newContext();
      const page = await context.newPage();
      activePage = page; activeCase = "style-policy-probe";
      await page.addInitScript(() => { window.__styleProbe = []; document.addEventListener("securitypolicyviolation", (event) => window.__styleProbe.push(event.effectiveDirective)); });
      await context.route("https://gist.githubusercontent.com/**", (route) => route.fulfill({ status: 200, contentType: "text/plain", body: fixture.envelope, headers: { "access-control-allow-origin": "*" } }));
      await context.route(base, (route) => route.fulfill({ status: 200, contentType: "text/html", body: fs.readFileSync(path.join(target, "index.html"), "utf8").replace("style-src 'self' 'unsafe-inline'", "style-src 'self'") }));
      activePrefix = new URL(fixture.link).hash.split("/").slice(0, 3).join("/"); activeKey = activePrefix.split("/")[2];
      await page.goto(`${base}${activePrefix}/page/Project`);
      await page.locator(".katex").first().waitFor();
      const styles = await page.evaluate(() => window.__styleProbe);
      assert.ok(styles.some((directive) => directive.startsWith("style-src")), "the stricter style policy unexpectedly needs no inline styles; revisit shipped CSP");
      console.log(`CSP bundle probe: unsafe-inline needed (${[...new Set(styles)].join(", ")}); generation 1 needs no workers`);
      await context.close();
    }
    // A feature refusal must be visible before any gist is requested.
    const page = await browser.newPage();
    await page.addInitScript(() => { window.DecompressionStream = undefined; });
    await page.goto(`${base}#invalid`);
    await page.getByRole("alert").filter({ hasText: "This browser cannot open Tine shares" }).waitFor();
    await page.close();
    return fixtureFingerprint(root);
  } catch (error) {
    const out = path.join(root, "artifacts/quick-share-viewer"); fs.mkdirSync(out, { recursive: true });
    let observed = String(error);
    if (activePrefix) observed = observed.replaceAll(activePrefix, "<share>");
    if (activeKey) observed = observed.replaceAll(activeKey, "<key>");
    fs.writeFileSync(path.join(out, "check-failure.json"), JSON.stringify({ built_from: manifest?.built_from, journey: activeCase, expected: "the viewer renders a compatible share under CSP without leaking its fragment", observed, classification: "ambiguous: viewer or browser/deployment infrastructure" }, null, 2) + "\n");
    await activePage?.screenshot({ path: path.join(out, "check-failure.png") }).catch(() => {});
    throw new Error(`Viewer compatibility failed (${activeCase}); see artifacts/quick-share-viewer/check-failure.json`);
  } finally { await browser?.close(); await hosted?.close(); }
}
if (process.argv[1] === fileURLToPath(import.meta.url)) await checkViewer(process.argv[2] ?? path.join(root, "artifacts/quick-share-viewer/build"));
