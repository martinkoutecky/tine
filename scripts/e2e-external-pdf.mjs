// GH #732: literal outside-file opens, reader tools and whole-graph byte proof.
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import crypto from "node:crypto";
import { pathToFileURL } from "node:url";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = process.env.E2E_ARTIFACT_DIR || path.join(root, "target/artifacts/og-external-pdf-732");
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(out, dir));

function syntheticPdf() {
  const stream = text => {
    const body = `BT /F1 20 Tf 72 720 Td (${text}) Tj ET\n`;
    return `<< /Length ${Buffer.byteLength(body)} >>\nstream\n${body}endstream`;
  };
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    "<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 7 0 R >> >> /Contents 4 0 R >>",
    stream("Outside OrchidQuartz source."),
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 7 0 R >> >> /Contents 6 0 R >>",
    stream("Second page outside the graph."),
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
  ];
  let pdf = "%PDF-1.7\n";
  const offsets = [0];
  for (const [i, object] of objects.entries()) {
    offsets.push(Buffer.byteLength(pdf)); pdf += `${i + 1} 0 obj\n${object}\nendobj\n`;
  }
  const xref = Buffer.byteLength(pdf);
  pdf += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  for (const offset of offsets.slice(1)) pdf += `${String(offset).padStart(10, "0")} 00000 n \n`;
  return Buffer.from(`${pdf}trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`);
}
const outside = path.join(out, "outside 文 paper.pdf");
const url = pathToFileURL(outside).href;
fs.writeFileSync(outside, syntheticPdf());
const wrong = path.join(out, "wrong.pdf"); fs.writeFileSync(wrong, "This is not a PDF.");
const missing = path.join(out, "missing.pdf");
// GH #732: the same outside PDF, named relative to the graph folder (the file-link resolver's root).
const relative = `../${encodeURIComponent(path.basename(outside))}`;
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:tine/guide-announced? true}\n");
fs.writeFileSync(path.join(graph, "pages/Outside PDFs.md"), [
  `- ![Outside paper](${url})`,
  `- [System viewer](${url})`,
  `- ![Wrong file](${pathToFileURL(wrong).href})`,
  `- ![Missing file](${pathToFileURL(missing).href})`,
  `- ![Relative paper](${relative})`,
  `- ![Relative file paper](file:${relative})`,
  "",
].join("\n"));
fs.writeFileSync(path.join(graph, "pages/Org outside.org"), `* [[${url}][Org outside paper]]\n* [[file:${relative}][Org relative paper]]\n`);

const fixture = accountHomeFixture(out, out, { ...process.env, HOME: path.join(out, "home"), TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(out, "data"), XDG_CONFIG_HOME: path.join(out, "config"),
  XDG_CACHE_HOME: path.join(out, "cache"), XDG_STATE_HOME: path.join(out, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" });
const app = path.resolve(process.env.TINE_APP || path.join(root, "target/release/tine"));
const port = Number(process.env.E2E_DRIVER_PORT || 4972);
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver",
  ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],
  { env: fixture.env, stdio: ["ignore", log, log], detached: true });
const base = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
const hash = bytes => crypto.createHash("sha256").update(bytes).digest("hex");
const appSha256 = hash(fs.readFileSync(app));
function snapshot(directory, prefix = "") {
  return Object.fromEntries(fs.readdirSync(directory, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name)).flatMap(entry => {
    const rel = prefix + entry.name, full = path.join(directory, entry.name);
    return entry.isDirectory() ? [[rel + "/", "directory"], ...Object.entries(snapshot(full, rel + "/"))]
      : [[rel, hash(fs.readFileSync(full))]];
  }));
}
let browser, stage = "startup";
const evidence = [];
const screenshot = name => execFileSync("import", ["-window", "root", path.join(out, `${name}.png`)], { timeout: 10000 });
async function clickLink(label) {
  let match;
  await browser.waitUntil(async () => {
    for (const link of await browser.$$(".main-content a")) {
      if ((await link.getText()).includes(label) && await link.isDisplayed()) { match = link; return true; }
    }
    return false;
  }, { timeout: 10000, timeoutMsg: `No host PDF link for ${label}` });
  await match.click();
}
async function waitReady() {
  await browser.$('.pdf-viewer[data-pdf-ready="true"] .pdf-page canvas').waitForExist({ timeout: 20000 });
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".pdf-page .textLayer span")]
    .some(span => span.textContent?.includes("OrchidQuartz"))), { timeout: 20000 });
  assert.ok((await browser.$(".pdf-read-only").getText()).includes("read-only"));
  assert.equal(await browser.$(".pdf-notes-btn").isExisting(), false);
  assert.equal(await browser.$('[title^="Area highlight"]').isExisting(), false);
}
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: app } } });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await assertAppLease(fixture);
  await browser.setWindowSize(1700, 1050);
  await openPageByName(browser, "Outside PDFs");
  const before = snapshot(graph);
  fs.writeFileSync(path.join(out, "graph-before.json"), JSON.stringify(before, null, 2));
  stage = "Markdown link opens outside PDF beside its source";
  await clickLink("Outside paper"); await waitReady();
  assert.ok(await browser.$("h1.page-title").isDisplayed(), "source notes stay visible beside the reader");
  screenshot("markdown-reader");

  stage = "search outside text";
  await browser.$('[title="Find in document (Ctrl+F)"]').click();
  await browser.$(".pdf-find-input").setValue("OrchidQuartz");
  await browser.waitUntil(async () => /1.*1/.test(await browser.$(".pdf-find-count").getText()), { timeout: 10000 });
  screenshot("search-match");
  evidence.push({ stage, matches: await browser.$(".pdf-find-count").getText() });
  await browser.$('[title="Close (Esc)"]').click();

  stage = "native text selection and clipboard copy without highlighting";
  const spans = await browser.$$('.pdf-page[data-page="1"] .textLayer span');
  let text;
  for (const span of spans) if ((await span.getText()).includes("OrchidQuartz")) { text = span; break; }
  assert.ok(text); await text.doubleClick();
  await browser.waitUntil(() => browser.execute(() => !!window.getSelection()?.toString().trim()), { timeout: 5000 });
  const selected = await browser.execute(() => window.getSelection().toString());
  await browser.keys(["Control", "c"]);
  assert.equal(await browser.$(".pdf-color-menu").isExisting(), false);
  await browser.$('[title="Find in document (Ctrl+F)"]').click();
  const find = await browser.$(".pdf-find-input"); await find.click();
  await browser.keys(["Control", "a"]); await browser.keys(["Backspace"]);
  await browser.keys(["Control", "v"]);
  await browser.waitUntil(async () => (await find.getValue()) === selected, { timeout: 5000 });
  evidence.push({ stage, selected, pasted: await find.getValue() });
  await browser.$('[title="Close (Esc)"]').click();

  stage = "zoom and page navigation";
  const scale = await browser.$(".pdf-zoom-level").getText();
  await browser.$('[title="Zoom in"]').click();
  assert.notEqual(await browser.$(".pdf-zoom-level").getText(), scale);
  await browser.$(".pdf-page-input").setValue("2"); await browser.keys(["Enter"]);
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll('.pdf-page[data-page="2"] .textLayer span')]
    .some(span => span.textContent?.includes("Second page"))), { timeout: 10000 });
  assert.equal(await browser.$(".pdf-page-input").getValue(), "2");
  screenshot("page-two");
  // Reader tools move into this menu only when the pane is narrow. At wide
  // widths their explanation is already visible beside the toolbar controls.
  await browser.setWindowSize(1100, 950);
  const notice = await browser.execute(() => {
    const text = document.querySelector(".pdf-toolbar .pdf-read-only");
    const toolbar = text.closest(".pdf-toolbar").getBoundingClientRect(), rect = text.getBoundingClientRect();
    return { hidden: !text.getClientRects().length, contained: rect.top >= toolbar.top && rect.bottom <= toolbar.bottom + 1 };
  });
  assert.ok(notice.hidden || notice.contained, "The narrow read-only explanation must fit its toolbar or move into the tools menu");
  await browser.$('[aria-label="More settings"]').click();
  const tools = await browser.$(".pdf-settings-menu").getText();
  assert.ok(tools.includes("read-only")); assert.ok(!tools.includes("Area highlight") && !tools.includes("Notes"));
  screenshot("read-only-tools");
  await browser.keys(["Escape"]);
  await browser.setWindowSize(1700, 1050);
  await browser.$(".pdf-close-btn").click();

  stage = "Org link opens the same outside PDF";
  await openPageByName(browser, "Org outside");
  await clickLink("Org outside paper"); await waitReady(); screenshot("org-reader");
  await browser.$(".pdf-close-btn").click();
  stage = "relative Org link opens the same outside PDF";
  await clickLink("Org relative paper"); await waitReady(); screenshot("org-relative-reader");
  await browser.$(".pdf-close-btn").click();
  await openPageByName(browser, "Outside PDFs");
  for (const label of ["Relative paper", "Relative file paper"]) {
    stage = `relative Markdown link opens the same outside PDF: ${label}`;
    await browser.waitUntil(() => browser.execute((name, file) => [...document.querySelectorAll("a.pdf-link")]
      .some(link => link.textContent.includes(name) && link.title === file), label, outside),
    { timeout: 10000, timeoutMsg: `${label} should be a reader link titled with the resolved file ${outside}` });
    await clickLink(label); await waitReady();
    if (label === "Relative paper") screenshot("markdown-relative-reader");
    evidence.push({ stage, resolved: outside });
    await browser.$(".pdf-close-btn").click();
  }
  for (const [label, expectedPath] of [["Wrong file", wrong], ["Missing file", missing]]) {
    stage = `reader error: ${label}`;
    await clickLink(label);
    await browser.$(".pdf-load-error").waitForExist({ timeout: 10000 });
    const message = await browser.$(".pdf-load-error").getText();
    assert.ok(message.includes(expectedPath), message);
    assert.ok(await browser.$(".pdf-close-btn").isDisplayed());
    evidence.push({ stage, message }); screenshot(label.replaceAll(" ", "-"));
    await browser.$(".pdf-close-btn").click();
  }
  stage = "entire graph directory remains byte-identical";
  const after = snapshot(graph);
  fs.writeFileSync(path.join(out, "graph-after.json"), JSON.stringify(after, null, 2));
  assert.deepEqual(after, before, "Opening/reading outside PDFs changed graph files or directories");
  assert.equal(hash(fs.readFileSync(outside)), hash(syntheticPdf()));
  evidence.push({ stage, entries: Object.keys(before).length });
  fs.writeFileSync(path.join(out, "receipt.json"), JSON.stringify({ outcome: "PASS", base, app, appSha256, evidence }, null, 2));
  console.log(`PASS ${out}`);
} catch (error) {
  try { screenshot("failure"); } catch {}
  const observed = { base, app, appSha256, stage, expected: "Outside PDF opens read-only, reader tools work, and all graph bytes are preserved",
    observed: error.stack, classification: error.message?.includes("Missing text parameter") ? "harness"
      : error.code === "ERR_ASSERTION" ? "product" : "ambiguous harness/infrastructure", evidence };
  fs.writeFileSync(path.join(out, "failure.json"), JSON.stringify(observed, null, 2));
  if (browser) fs.writeFileSync(path.join(out, "failure-dom.json"), JSON.stringify(await browser.execute(() => ({ text: document.body.innerText, html: document.querySelector(".pdf-viewer")?.outerHTML })), null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log); fixture.close?.(); fixture.verify();
}
