// GH #869: Copy ref on an area highlight, native paste into another routed page.
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { deflateSync } from "node:zlib";
import assert from "node:assert/strict";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const OUT = path.join(ROOT, "artifacts/og-pdf-area-ref-869");
fs.mkdirSync(OUT, { recursive: true });
const RUN = fs.mkdtempSync(path.join(OUT, "native-"));
const GRAPH = path.join(RUN, "graph");
for (const dir of ["pages", "journals", "logseq", "assets/book"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(RUN, dir));
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), "{:tine/guide-announced? true}\n");
const ID = "86900000-0000-4000-8000-000000000001";
const STAMP = 1730000000000;
const sourceFormat = process.env.E2E_AREA_FORMAT === "org" ? "org" : "md";
// A generated one-page fixture with a blue rectangle. No external attachment.
const content = "q 0 0.25 1 rg 72 592 240 100 re f Q\n";
const objects = ["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
  "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>",
  `<< /Length ${content.length} >>\nstream\n${content}endstream`];
let pdf = "%PDF-1.4\n";
const offsets = [0];
for (const [i, object] of objects.entries()) { offsets.push(Buffer.byteLength(pdf)); pdf += `${i + 1} 0 obj\n${object}\nendobj\n`; }
const xref = Buffer.byteLength(pdf);
pdf += `xref\n0 5\n0000000000 65535 f \n${offsets.slice(1).map(n => `${String(n).padStart(10, "0")} 00000 n \n`).join("")}trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
fs.writeFileSync(path.join(GRAPH, "assets/book.pdf"), pdf);
// Seed the corresponding 240x100 crop using a standard PNG encoder.
function chunk(type, data) {
  const payload = Buffer.concat([Buffer.from(type), data]);
  let crc = 0xffffffff;
  for (const byte of payload) { crc ^= byte; for (let bit = 0; bit < 8; bit++) crc = (crc >>> 1) ^ ((crc & 1) ? 0xedb88320 : 0); }
  const prefix = Buffer.alloc(4), suffix = Buffer.alloc(4);
  prefix.writeUInt32BE(data.length); suffix.writeUInt32BE((crc ^ 0xffffffff) >>> 0);
  return Buffer.concat([prefix, payload, suffix]);
}
const header = Buffer.alloc(13); header.writeUInt32BE(240); header.writeUInt32BE(100, 4); header[8] = 8; header[9] = 2;
const pixels = Buffer.alloc(100 * 721);
for (let y = 0; y < 100; y++) for (let x = 0; x < 240; x++) { pixels[y * 721 + 1 + x * 3 + 1] = 64; pixels[y * 721 + 1 + x * 3 + 2] = 255; }
fs.writeFileSync(path.join(GRAPH, `assets/book/1_${ID}_${STAMP}.png`), Buffer.concat([
  Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", header), chunk("IDAT", deflateSync(pixels)), chunk("IEND", Buffer.alloc(0))]));
fs.writeFileSync(path.join(GRAPH, "assets/book.edn"), `{:highlights [{:id #uuid "${ID}" :page 1 :position {:bounding {:x1 72 :y1 100 :x2 312 :y2 200 :width 612 :height 792} :rects () :page 1} :content {:text "" :image ${STAMP}} :properties {:color "blue"}}]}\n`);
const properties = [["hl-page", "1"], ["hl-color", "blue"], ["ls-type", "annotation"], ["id", ID], ["hl-type", "area"], ["hl-stamp", String(STAMP)]];
fs.writeFileSync(path.join(GRAPH, `pages/hls__book.${sourceFormat}`), sourceFormat === "org"
  ? `#+FILE-PATH: ../assets/book.pdf\n\n*\n:PROPERTIES:\n${properties.map(([k, v]) => `:${k}: ${v}\n`).join("")}:END:\n`
  : `file-path:: ../assets/book.pdf\n\n-\n${properties.map(([k, v]) => `  ${k}:: ${v}\n`).join("")}`);
fs.writeFileSync(path.join(GRAPH, "pages/Reader.md"), "- [Book](../assets/book.pdf)\n");
fs.writeFileSync(path.join(GRAPH, "pages/Citation.md"), `- Cite figure here: \n- {{embed ((${ID}))}}\n`);
fs.writeFileSync(path.join(GRAPH, "pages/Citation Org.org"), `* Cite figure here: \n* {{embed ((${ID}))}}\n`);

const APP = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
const TD = process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME || "", "bin/tauri-driver");
const port = Number(process.env.E2E_DRIVER_PORT || 4898);
const fixture = accountHomeFixture(RUN, RUN);
const env = { ...fixture.env, HOME: path.join(RUN, "home"), TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(RUN, "data"), XDG_CONFIG_HOME: path.join(RUN, "config"),
  XDG_CACHE_HOME: path.join(RUN, "cache"), XDG_STATE_HOME: path.join(RUN, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const log = fs.openSync(path.join(RUN, "driver.log"), "w");
const wm = spawn("openbox", [], { env, stdio: ["ignore", log, log], detached: true });
const driver = spawn(TD, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser, stage = "startup";
const evidence = { testedCommit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(), app: APP, sourceFormat, run: RUN, rows: [] };
const screenshot = name => execFileSync("import", ["-window", "root", path.join(RUN, name)], { timeout: 10000 });
async function loadedImages(selector, count) {
  await browser.waitUntil(() => browser.execute((selector, count) => {
    const images = [...document.querySelectorAll(selector)];
    return images.length >= count && images.every(image => image.complete && image.naturalWidth > 0);
  }, selector, count), { timeout: 20000, timeoutMsg: `${stage}: area image did not load` });
}
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await assertAppLease(fixture);
  await openPageByName(browser, "Reader");
  await browser.$(".pdf-link").click();
  await browser.$(`.pdf-hl-area[data-highlight-id="${ID}"]`).waitForExist({ timeout: 20000 });
  for (const [name, file] of [["Citation", "Citation.md"], ["Citation Org", "Citation Org.org"]]) {
    stage = `${name}: copy highlight reference`;
    await browser.execute(id => {
      const element = document.querySelector(`.pdf-hl-area[data-highlight-id="${id}"]`);
      const rect = element.getBoundingClientRect();
      element.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: rect.left + rect.width / 2, clientY: rect.top + rect.height / 2 }));
    }, ID);
    await browser.$(".pdf-color-menu .pdf-hl-action").click();
    await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".toast")].some(el => el.textContent.includes("Copied highlight ref"))), { timeout: 10000 });
    await browser.$(".pdf-close-btn").click();
    stage = `${name}: native paste`;
    await openPageByName(browser, name);
    await browser.$(".page-blocks .ls-block .block-content-wrapper").click();
    await browser.$("textarea.block-editor").waitForExist({ timeout: 10000 });
    await browser.keys(["End"]);
    await browser.performActions([{ type: "key", id: "paste-reference", actions: [
      { type: "keyDown", value: "\uE009" }, { type: "keyDown", value: "v" }, { type: "keyUp", value: "v" }, { type: "keyUp", value: "\uE009" }] }]);
    await browser.releaseActions();
    await browser.keys(["Escape"]);
    await waitForFileText(path.join(GRAPH, "pages", file), text => text.includes(`Cite figure here: ((${ID}))`));
    stage = `${name}: loaded reference and embed`;
    await loadedImages(`.page-blocks .block-ref[data-block-ref="${ID}"] img`, 1);
    await loadedImages(".page-blocks .embed-block img", 1);
    screenshot(`${file}.png`);
    evidence.rows.push({ name, imageWidths: await browser.execute(() => [...document.querySelectorAll(".page-blocks .pdf-area-image img")].map(image => image.naturalWidth)) });
    stage = `${name}: click crop to navigate`;
    await browser.$(`.block-ref[data-block-ref="${ID}"] img`).click();
    await browser.waitUntil(() => browser.execute(id => document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-highlight-target") === id, ID), { timeout: 15000 });
    assert.equal(await browser.$(".pdf-viewer").getAttribute("data-pdf-filename"), "book.pdf");
  }
  stage = "highlights source page";
  await browser.$(".pdf-close-btn").click();
  await openPageByName(browser, "hls__book");
  await loadedImages(".page-blocks .pdf-area-image img", 1);
  assert.doesNotMatch(await browser.$(".page-blocks").getText(), /:PROPERTIES:|:END:|hl-stamp|hl-type/);
  screenshot("source.png");
  fixture.verify();
  fs.writeFileSync(path.join(RUN, "pass.json"), JSON.stringify(evidence, null, 2) + "\n");
  console.log(`PASS GH #869: Markdown and Org Copy ref → native paste → loaded crop; embed, source and exact PDF navigation. ${RUN}`);
} catch (error) {
  try { screenshot("failure.png"); evidence.observed = await browser.execute(() => document.body.innerText.slice(0, 4000)); } catch {}
  fs.writeFileSync(path.join(RUN, "failure.json"), JSON.stringify({ ...evidence, stage, expected: "a pasted PDF area reference loads its image and opens its highlight", observedError: String(error), classification: "ambiguous" }, null, 2) + "\n");
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  for (const child of [driver, wm]) if (child?.pid) { try { process.kill(-child.pid, "SIGKILL"); } catch {} }
  fixture.close();
  fs.closeSync(log);
}
