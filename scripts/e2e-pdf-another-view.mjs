// Native proof for UI-PDF-ANOTHER-VIEW-SHARED-HIGHLIGHTS. Real reader actions,
// rendered overlays and paired sidecar/notes bytes are the oracles.
import { spawn } from "node:child_process";
import net from "node:net";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";
import { remote } from "webdriverio";
import { APP_ID } from "./lib/app-identity.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "artifacts/og-pdf-multiview-e2e", String(process.pid));
const GRAPH = path.join(ARTIFACTS, "graph-a");
const OTHER = path.join(ARTIFACTS, "graph-b");
const SIDECAR = path.join(GRAPH, "assets/shared.edn");
const NOTES = path.join(GRAPH, "pages/hls__shared.md");
const ID = "6a5604f8-a337-4336-a711-2ba6bc14fbfd";
const TEXT = "Shared text highlight";
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4890);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4891);
const APP = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin/tauri-driver") : "tauri-driver");

// Same self-contained PDF object/xref construction as e2e-pdf-logseq's outline
// fixture, with two short lines that fit inside an ordinary split pane.
function fixturePdf() {
  const stream = (text) => {
    const content = `BT /F1 13 Tf 72 690 Td (${text}) Tj ET`;
    return `<< /Length ${Buffer.byteLength(content)} >>\nstream\n${content}\nendstream`;
  };
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    "<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 7 0 R >> >> /Contents 4 0 R >>",
    stream("Existing shared highlight"),
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 7 0 R >> >> /Contents 6 0 R >>",
    stream(TEXT),
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
  ];
  let pdf = "%PDF-1.7\n";
  const offsets = [0];
  for (const [i, object] of objects.entries()) {
    offsets.push(Buffer.byteLength(pdf));
    pdf += `${i + 1} 0 obj\n${object}\nendobj\n`;
  }
  const xref = Buffer.byteLength(pdf);
  pdf += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  for (const offset of offsets.slice(1)) pdf += `${String(offset).padStart(10, "0")} 00000 n \n`;
  return `${pdf}trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
}

fs.mkdirSync(ARTIFACTS, { recursive: true });
for (const graph of [GRAPH, OTHER]) {
  for (const dir of ["pages", "journals", "assets", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
  fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
  fs.writeFileSync(path.join(graph, "pages/Multiview.md"), "- ![Shared PDF](../assets/shared.pdf)\n");
}
fs.writeFileSync(path.join(GRAPH, "assets/shared.pdf"), fixturePdf());
const rect = "{:x1 72 :y1 88 :x2 225 :y2 104 :width 612 :height 792}";
fs.writeFileSync(SIDECAR, `{:highlights [{:id #uuid "${ID}" :page 1 :position {:bounding ${rect} :rects (${rect}) :page 1} :content {:text "Existing shared highlight"} :properties {:color "yellow"}}] :extra {:page 1 :scale 1.0}}\n`);
fs.writeFileSync(NOTES, `file:: [Shared PDF](../assets/shared.pdf)\nfile-path:: ../assets/shared.pdf\n\n- Existing shared highlight\n  hl-page:: 1\n  hl-color:: yellow\n  ls-type:: annotation\n  id:: ${ID}\n`);
const xdg = path.join(ARTIFACTS, "xdg");
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(xdg, dir), { recursive: true });
const appData = path.join(xdg, "data", APP_ID);
fs.mkdirSync(appData, { recursive: true });
fs.writeFileSync(path.join(appData, "tine-settings.json"), JSON.stringify({
  last_graph_path: GRAPH,
  known_graphs: [GRAPH, OTHER].map((graph) => ({ name: path.basename(graph), path: graph })),
}));
const env = {
  ...process.env, TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(xdg, "data"), XDG_CONFIG_HOME: path.join(xdg, "config"), XDG_CACHE_HOME: path.join(xdg, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
};
const log = fs.openSync(path.join(ARTIFACTS, "tauri-driver.log"), "w");
const driver = spawn(TD, ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], {
  env, stdio: ["ignore", log, log], detached: true,
});
let browser;
let journey = "launch";
const proof = { commit: process.env.E2E_TESTED_COMMIT || "9b0dbcc3670fb8a7852d589e37bb9f5d3a90f298", steps: [] };
const view = (id) => `[data-pdf-view-id="${id}"] .pdf-viewer`;
const count = (text, literal) => text.split(literal).length - 1;
const check = (ok, message) => { if (!ok) throw new Error(message); };

async function readyPort() {
  const until = Date.now() + 15_000;
  while (Date.now() < until) {
    const ready = await new Promise((resolve) => {
      const socket = net.connect({ host: "127.0.0.1", port: DRIVER_PORT }, () => { socket.destroy(); resolve(true); });
      socket.on("error", () => resolve(false));
    });
    if (ready) return;
    await delay(100);
  }
  throw new Error("tauri-driver did not listen");
}

async function snapshot() {
  return browser.execute(() => [...document.querySelectorAll(".pdf-viewer")].map((v) => ({
    id: v.closest("[data-pdf-view-id]")?.getAttribute("data-pdf-view-id"),
    pane: v.closest("[data-pane-id]")?.getAttribute("data-pane-id"),
    file: v.getAttribute("data-pdf-filename"), ready: v.getAttribute("data-pdf-ready"),
    page: v.querySelector(".pdf-page-input")?.value,
    zoom: v.querySelector(".pdf-zoom-level")?.textContent,
    scroll: v.querySelector(".pdf-scroll")?.scrollTop,
    highlights: [...v.querySelectorAll(".pdf-hl")].map((hl) => ({ id: hl.dataset.highlightId, color: hl.style.backgroundColor })),
  })));
}

async function assertViews(expected, label) {
  let actual;
  await browser.waitUntil(async () => {
    actual = await snapshot();
    return actual.length === expected.length && expected.every((e) => actual.some((a) =>
      a.id === e.id && a.page === e.page && a.zoom === e.zoom && Math.abs(a.scroll - e.scroll) <= 1));
  }, { timeout: 10_000, timeoutMsg: `${label}: independent viewport changed` });
  proof.steps.push({ label, views: actual });
}

async function overlay(id, color, present = true) {
  await browser.waitUntil(async () => (await snapshot()).every((v) => {
    const matches = v.highlights.filter((hl) => hl.id === id);
    return present ? matches.length === 1 && (!color || matches[0].color === color) : matches.length === 0;
  }), { timeout: 10_000, timeoutMsg: `shared overlay ${id} did not become ${present ? color || "present" : "absent"}` });
}

async function chooseColor(id, index) {
  const swatch = await browser.$(`${view(id)} .pdf-color-menu .pdf-color-swatch:nth-child(${index + 1})`);
  await swatch.waitForDisplayed({ timeout: 5000 });
  const color = await browser.execute((el) => el.style.backgroundColor, swatch);
  await swatch.click();
  await browser.$(`${view(id)} .pdf-color-menu`).waitForExist({ reverse: true, timeout: 5000 });
  return color;
}

async function editExisting(id, highlightId, colorIndex) {
  await browser.$(`${view(id)} .pdf-hl[data-highlight-id="${highlightId}"]`).click();
  return chooseColor(id, colorIndex);
}

async function selectText(id) {
  let target, previous, stable = 0;
  await browser.waitUntil(async () => {
    target = await browser.execute((selector, text) => {
      const span = [...document.querySelectorAll(`${selector} .textLayer span`)].find((s) => s.textContent === text);
      if (!span?.firstChild) return null;
      const range = document.createRange();
      range.selectNodeContents(span);
      const rect = range.getBoundingClientRect();
      const y = Math.round(rect.top + rect.height / 2);
      const endpoint = (edge, offset) => {
        for (let d = -3; d <= 3; d++) {
          const x = Math.round(edge) + d;
          const caret = document.caretRangeFromPoint(x, y);
          if (caret?.startContainer === span.firstChild && caret.startOffset === offset) return { x, y };
        }
        return null;
      };
      const start = endpoint(rect.left, 0), end = endpoint(rect.right, text.length);
      return start && end ? { start, end } : null;
    }, view(id), TEXT);
    const current = JSON.stringify(target);
    stable = target && current === previous ? stable + 1 : 0;
    previous = current;
    return stable >= 2;
  }, { timeout: 10_000, interval: 100, timeoutMsg: "PDF text endpoints did not settle" });
  try {
    await browser.performActions([{ type: "pointer", id: "shared-text", parameters: { pointerType: "mouse" }, actions: [
      { type: "pointerMove", duration: 0, origin: "viewport", ...target.start },
      { type: "pointerDown", button: 0 },
      { type: "pointerMove", duration: 350, origin: "viewport", ...target.end },
      { type: "pointerUp", button: 0 },
    ] }]);
  } finally { await browser.releaseActions(); }
  check(await browser.execute(() => window.getSelection()?.toString()) === TEXT, "native drag did not select fixture text");
}

try {
  await readyPort();
  browser = await remote({ hostname: "127.0.0.1", port: DRIVER_PORT, path: "/", logLevel: "error", connectionRetryCount: 1,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await browser.setWindowSize(1880, 1000);
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 30_000 });
  // Keep transient errors as evidence through the final graph drain.
  await browser.execute(() => {
    window.__pdfJourneyErrors = [];
    const capture = () => document.querySelectorAll(".toast.toast-error").forEach((n) => {
      if (!window.__pdfJourneyErrors.includes(n.textContent)) window.__pdfJourneyErrors.push(n.textContent);
    });
    new MutationObserver(capture).observe(document.body, { childList: true, subtree: true });
    capture();
  });
  await openPageByName(browser, "Multiview");
  await browser.$(".pdf-link").click();
  await browser.$('.pdf-viewer[data-pdf-ready="true"]').waitForExist({ timeout: 20_000 });
  journey = "open another view";
  await browser.$('button[aria-label="More settings"]').click();
  const another = await browser.$(".pdf-settings-menu .pdf-another-view");
  check(await another.getText() === "Open another view", "literal menu action missing");
  await another.click();
  await browser.waitUntil(async () => { const views = await snapshot(); return views.length === 2 && views.every((v) => v.ready === "true"); }, { timeout: 20_000 });
  const initial = await snapshot();
  const [a, b] = initial.map((v) => v.id);
  check(a && b && a !== b && initial.every((v) => v.file === "shared.pdf"), "two distinct views of the same PDF were not mounted");
  check(initial[0].pane && initial[1].pane && initial[0].pane !== initial[1].pane, "PDF views did not open in distinct panes");
  check(initial[0].page === initial[1].page && initial[0].zoom === initial[1].zoom, "another view did not inherit page/zoom");
  await overlay(ID);
  journey = "independent page and zoom";
  const page = await browser.$(`${view(b)} .pdf-page-input`);
  await page.setValue("2");
  await page.addValue("\uE007");
  await browser.waitUntil(async () => (await snapshot()).find((v) => v.id === b)?.page === "2", { timeout: 10_000 });
  const afterPage = await snapshot();
  check(afterPage.find((v) => v.id === a).page === initial[0].page && afterPage.every((v) => v.zoom === initial[0].zoom), "moving B changed A or zoom");
  await browser.$(`${view(b)} button[title="Zoom in"]`).click();
  await browser.waitUntil(async () => (await snapshot()).find((v) => v.id === b)?.zoom !== initial[1].zoom, { timeout: 10_000 });
  // Rendering/scroll anchoring settles before the frozen viewport assertions.
  let prior, settled = 0;
  await browser.waitUntil(async () => { const s = JSON.stringify(await snapshot()); settled = s === prior ? settled + 1 : 0; prior = s; return settled >= 3; }, { interval: 100, timeout: 10_000 });
  const expected = await snapshot();
  check(expected.find((v) => v.id === a).zoom === initial[0].zoom && expected.find((v) => v.id === b).page === "2", "zoom changed the other view or page");
  await assertViews(expected, "separate page/zoom");

  journey = "recolor A propagates to B and disk";
  const green = await editExisting(a, ID, 1);
  await overlay(ID, green);
  await waitForFileText(SIDECAR, (t) => t.includes(':color "green"'));
  await assertViews(expected, "recolor");

  journey = "create text in B propagates once to A and paired files";
  await selectText(b);
  const purple = await chooseColor(b, 4);
  const written = await waitForFileText(SIDECAR, (t) => t.includes(`:text "${TEXT}"`));
  const ids = [...written.matchAll(/:id #uuid "([^"]+)"/g)].map((m) => m[1]);
  const newId = ids.find((id) => id !== ID);
  check(newId && ids.length === 2 && count(written, newId) === 1, "sidecar did not contain each highlight once");
  await waitForFileText(NOTES, (t) => count(t, newId) === 1 && count(t, TEXT) === 1);
  await overlay(newId, purple);
  await assertViews(expected, "create");

  journey = "delete in A propagates to B and paired files";
  await browser.$(`${view(a)} .pdf-hl[data-highlight-id="${ID}"]`).click();
  await browser.$(`${view(a)} .pdf-hl-remove`).click();
  await overlay(ID, undefined, false);
  await waitForFileText(SIDECAR, (t) => !t.includes(ID) && count(t, newId) === 1);
  await waitForFileText(NOTES, (t) => !t.includes(ID) && count(t, newId) === 1);
  await assertViews(expected, "delete");

  journey = "close B, edit A, drain graph";
  await browser.$(`${view(b)} .pdf-close-btn`).click();
  await assertViews(expected.filter((v) => v.id === a), "close sibling");
  // The new page-2 overlay remains mounted in A; a deliberate page jump makes
  // it visible for the final real edit after the sibling lease was released.
  const remainingPage = await browser.$(`${view(a)} .pdf-page-input`);
  await remainingPage.setValue("2");
  await remainingPage.addValue("\uE007");
  await browser.waitUntil(async () => (await snapshot())[0].page === "2", { timeout: 10_000 });
  await editExisting(a, newId, 3);
  await waitForFileText(SIDECAR, (t) => count(t, newId) === 1 && t.includes(':color "red"'));
  await waitForFileText(NOTES, (t) => count(t, newId) === 1 && t.includes("hl-color:: red"));
  await browser.$(".graph-switch-btn").click();
  const clicked = await browser.execute((target) => {
    const row = [...document.querySelectorAll(".graph-switch-row")].find((n) => n.getAttribute("title") === target);
    row?.click(); return Boolean(row);
  }, OTHER);
  check(clicked, "known graph row missing");
  await browser.waitUntil(() => browser.execute(() => document.querySelector(".graph-switch-name")?.textContent?.trim() === "graph-b" && !document.querySelector(".pdf-viewer")), { timeout: 40_000 });
  const errors = await browser.execute(() => window.__pdfJourneyErrors);
  check(errors.length === 0, `error toasts during journey/drain: ${JSON.stringify(errors)}`);
  proof.steps.push({ label: "final edit durable and graph drained", newId, errors });
  fs.writeFileSync(path.join(ARTIFACTS, "receipt.json"), `${JSON.stringify(proof, null, 2)}\n`);
  console.log(`PASS: another PDF view shares highlights and preserves independent viewports; ${ARTIFACTS}`);
} catch (error) {
  try { await browser?.saveScreenshot(path.join(ARTIFACTS, "failure.png")); } catch {}
  let state; try { state = await snapshot(); } catch {}
  fs.writeFileSync(path.join(ARTIFACTS, "failure.json"), JSON.stringify({ ...proof, journey,
    expected: "shared overlays and paired bytes with independent viewports, then a clean graph drain",
    observed: String(error), state, classification: "ambiguous", screenshot: "failure.png", log: "tauri-driver.log",
  }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
