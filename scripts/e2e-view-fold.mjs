// Built G1 viewer and live export: routed read-only folds, navigation, reload,
// and zero mutation requests. Uses disposable, synthetic snapshots only.
// Build `npx vite build --mode quick-share` first. For the static check, run
// TINE_VIEW_FOLD_STATIC_DIR=$PWD/artifacts/view-fold/static cargo test -p
// tine-graph-features --lib static_child_folds_keep_authored_state_and_all_content.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { webcrypto } from "node:crypto";
import { gzipSync } from "node:zlib";
import { chromium } from "playwright";
import { serveViewer } from "./check-quick-share-viewer.mjs";

const out = path.resolve("artifacts/view-fold"); fs.mkdirSync(out, { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(out, dir), { recursive: true });
const fixture = "tests/fixtures/quick-share/g1/pages";
const doc = JSON.parse(fs.readFileSync(`${fixture}/document.json`, "utf8"));
const link = fs.readFileSync(`${fixture}/link.txt`, "utf8").trim();
const prefix = new URL(link).hash.split("/").slice(0, 3).join("/");
doc.snapshot.pages.find(p => p.name === "Project").blocks.unshift({
  id: "fold-parent", raw: "A collapsed bullet\ncollapsed:: true", collapsed: true,
  children: [{ id: "fold-child", raw: "Previously hidden child", collapsed: false, children: [] }],
});
const header = Buffer.concat([Buffer.from("TSH1"), Buffer.from([1]), Buffer.alloc(16, 7)]);
const nonce = webcrypto.getRandomValues(new Uint8Array(12));
const baseKey = await webcrypto.subtle.importKey("raw", Buffer.from(prefix.split("/")[2], "base64url"), "HKDF", false, ["deriveKey"]);
const key = await webcrypto.subtle.deriveKey({ name: "HKDF", hash: "SHA-256", salt: header.subarray(5), info: Buffer.from("tine-share/1/enc") }, baseKey, { name: "AES-GCM", length: 256 }, false, ["encrypt"]);
const cipher = await webcrypto.subtle.encrypt({ name: "AES-GCM", iv: nonce, additionalData: header }, key, gzipSync(JSON.stringify(doc)));
const envelope = Buffer.concat([header, nonce, Buffer.from(cipher)]).toString("base64url");
const hosted = await serveViewer(path.resolve("artifacts/quick-share-viewer/build"));
const browser = await chromium.launch({ args: ["--no-sandbox"], env: { ...process.env,
  HOME: path.join(out, "home"), XDG_DATA_HOME: path.join(out, "data"),
  XDG_CONFIG_HOME: path.join(out, "config"), XDG_CACHE_HOME: path.join(out, "cache"), XDG_STATE_HOME: path.join(out, "state") } });
const proof = { result: "running", cases: [] };
let page;
let staticHost;
try {
  for (const mode of ["quick-share", "published"]) {
    const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
    page = await context.newPage();
    const mutations = [], errors = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("request", req => { if (!["GET", "HEAD"].includes(req.method())) mutations.push(req.method()); });
    await context.route("https://gist.githubusercontent.com/**", route => route.fulfill({ body: envelope, contentType: "text/plain", headers: { "access-control-allow-origin": "*" } }));
    // Same production entry, with the published export's actual meta/backend.
    if (mode === "published") {
      await context.route("**/quick-share.html", route => route.fulfill({ contentType: "text/html", body:
        fs.readFileSync("artifacts/quick-share-viewer/build/quick-share.html", "utf8").replace('content="quick-share:1"', 'content="snapshot.json"')
          .replace("connect-src https://gist.githubusercontent.com https://api.github.com", "connect-src 'self'") }));
      await context.route("**/snapshot.json", route => route.fulfill({ json: doc.snapshot }));
    }
    const hash = mode === "quick-share" ? prefix : "#";
    const url = `${hosted.url}quick-share.html${hash}/page/Project`;
    await page.goto(url);
    const parent = page.locator('[data-block-id="fold-parent"]').first();
    const child = page.getByText("Previously hidden child", { exact: true });
    await parent.waitFor();
    assert.equal(await child.count(), 0);
    await page.screenshot({ path: path.join(out, `${mode}-collapsed.png`) });
    await parent.locator(":scope > .block-main .collapse-toggle").first().click();
    await child.waitFor({ state: "visible", timeout: 3000 });
    await page.screenshot({ path: path.join(out, `${mode}-expanded.png`) });
    await page.locator(".page-ref").filter({ hasText: "Notes ü" }).first().click();
    await page.locator(".page-title").filter({ hasText: "Notes ü" }).first().waitFor();
    await page.evaluate(hash => { location.hash = `${hash}/page/Project`; }, hash);
    await child.waitFor({ state: "visible" });
    await parent.locator(":scope > .block-main .collapse-toggle").first().click();
    await child.waitFor({ state: "detached" });
    await parent.locator(":scope > .block-main .collapse-toggle").first().click();
    await child.waitFor({ state: "visible" });
    await page.reload(); await parent.waitFor();
    assert.equal(await child.count(), 0, "reload must restore collapsed::");
    assert.deepEqual(mutations, []); assert.deepEqual(errors, []);
    proof.cases.push({ mode, expanded: true, collapsed: true, navigationRetained: true, reloadReset: true, mutations });
    await context.close();
  }
  staticHost = await serveViewer(path.join(out, "static"));
  const staticContext = await browser.newContext({ javaScriptEnabled: false, viewport: { width: 1280, height: 900 } });
  page = await staticContext.newPage();
  await page.goto(`${staticHost.url}folds.html`);
  const hidden = page.getByText("Hidden child", { exact: true });
  await hidden.waitFor({ state: "hidden" });
  await page.getByText("Visible child", { exact: true }).waitFor({ state: "visible" });
  const folded = page.locator("details.outline-fold").first();
  const row = folded.locator(":scope > summary");
  assert.match(await row.innerText(), /Folded/);
  assert.equal(await page.getByText("Children", { exact: true }).count(), 0);
  assert.equal(await page.locator("details.outline-fold").count(), 2);
  // The arrow is a CSS triangle in the row's gutter, outside its text box.
  const box = await row.boundingBox();
  await page.mouse.click(box.x - 24, box.y + 12);
  await hidden.waitFor({ state: "visible" });
  await page.screenshot({ path: path.join(out, "static-expanded.png") });
  await row.getByRole("link", { name: "Jump", exact: true }).click();
  assert.equal(new URL(page.url()).hash, "#b2", "summary link must navigate");
  assert.notEqual(await folded.getAttribute("open"), null, "link must not toggle an open row");
  // Blank space on the parent row toggles without targeting its links.
  await row.click({ position: { x: box.width - 10, y: 12 } });
  await hidden.waitFor({ state: "hidden" });
  await page.screenshot({ path: path.join(out, "static-collapsed.png") });
  await row.getByRole("link", { name: "Jump", exact: true }).click();
  assert.equal(await folded.getAttribute("open"), null, "link must not toggle a closed row");
  await row.getByRole("link", { name: "Folds", exact: true }).click();
  assert.equal(page.url(), `${staticHost.url}folds.html`, "page link must navigate");
  await hidden.waitFor({ state: "hidden" });
  proof.cases.push({ mode: "static", browser: "Chromium", scriptFree: true, expanded: true, collapsed: true,
    arrowToggles: true, rowBackgroundToggles: true, summaryLinksNavigateWithoutToggling: true });
  await staticContext.close();
  proof.result = "passed";
} catch (error) {
  proof.result = "failed"; proof.observed = String(error).split("\n")[0];
  proof.expected = "fold arrow reveals the child without mutations"; proof.classification = "product or harness; inspect screenshot";
  await page?.screenshot({ path: path.join(out, "failure.png") }).catch(() => {});
  throw error;
} finally {
  fs.writeFileSync(path.join(out, "browser-proof.json"), JSON.stringify(proof, null, 2) + "\n");
  await browser.close(); await hosted.close(); await staticHost?.close();
}
