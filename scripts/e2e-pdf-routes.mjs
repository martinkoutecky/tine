// Native Direct Files proof for first-class PDF pane routes. The assertions are
// user-visible route/session outcomes; fixtures live only in a fresh temp graph.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { ensureDisplay } from "./lib/e2e-display.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

await ensureDisplay();

if (process.platform !== "linux") throw new Error("PDF route native proof is Linux-only");

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME
  ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver")
  : "tauri-driver");
const WD = process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver";
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4592);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4593);
const TMP = path.join(os.tmpdir(), `tine-pdf-routes-e2e-${process.pid}`);
const GRAPH = path.join(TMP, "graph");
const APP_DATA = path.join(TMP, "xdg", "data", "page.tine.Tine");
const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || TMP;

function makePdf(label, pages = 3) {
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    `<< /Type /Pages /Kids [${Array.from({ length: pages }, (_, index) => `${3 + index * 2} 0 R`).join(" ")}] /Count ${pages} >>`,
  ];
  for (let index = 0; index < pages; index += 1) {
    const content = `BT /F1 22 Tf 72 720 Td (${label} page ${index + 1}) Tj ET`;
    const pageObject = 3 + index * 2;
    const streamObject = pageObject + 1;
    objects.push(`<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 ${3 + pages * 2} 0 R >> >> /Contents ${streamObject} 0 R >>`);
    objects.push(`<< /Length ${Buffer.byteLength(content)} >>\nstream\n${content}\nendstream`);
  }
  objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
  let pdf = "%PDF-1.7\n";
  const offsets = [0];
  for (const [index, object] of objects.entries()) {
    offsets.push(Buffer.byteLength(pdf));
    pdf += `${index + 1} 0 obj\n${object}\nendobj\n`;
  }
  const xref = Buffer.byteLength(pdf);
  pdf += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  for (let index = 1; index < offsets.length; index += 1) {
    pdf += `${String(offsets[index]).padStart(10, "0")} 00000 n \n`;
  }
  pdf += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(pdf, "utf8");
}

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, "xdg", dir), { recursive: true });
fs.mkdirSync(APP_DATA, { recursive: true });
fs.mkdirSync(ARTIFACTS, { recursive: true });
fs.writeFileSync(path.join(GRAPH, "logseq", "config.edn"), "{}\n");
fs.writeFileSync(path.join(GRAPH, "assets", "first.pdf"), makePdf("First route fixture"));
fs.writeFileSync(path.join(GRAPH, "assets", "second.pdf"), makePdf("Second route fixture", 1));
fs.writeFileSync(path.join(GRAPH, "pages", "hls__first.md"), "- Notes for the first PDF\n");
fs.writeFileSync(path.join(GRAPH, "pages", "Source.md"), "- Legacy source pane\n");
fs.writeFileSync(path.join(GRAPH, "pages", "Other.md"), "- Legacy second pane\n");
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}.md`;
fs.writeFileSync(path.join(GRAPH, "journals", journal), [
  "- ![First PDF](../assets/first.pdf)",
  "- ![Second PDF](../assets/second.pdf)",
  "",
].join("\n"));

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "xdg", "data"),
  XDG_CONFIG_HOME: path.join(TMP, "xdg", "config"),
  XDG_CACHE_HOME: path.join(TMP, "xdg", "cache"),
  XDG_CONFIG_DIRS: process.env.XDG_CONFIG_DIRS || "/etc/xdg",
  XDG_DATA_DIRS: process.env.XDG_DATA_DIRS || "/usr/local/share:/usr/share",
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  GDK_BACKEND: "x11",
};
const driverLog = fs.openSync(path.join(ARTIFACTS, "tauri-driver.log"), "w");
const receipt = {
  storageMode: "direct-files",
  companionRoute: {},
  persistedRoute: {},
  legacyMigration: {},
};
let driver;
let browser;

async function start(label) {
  driver = spawn(TD, webdriverServerArgs(DRIVER_PORT, NATIVE_PORT, WD), {
    env,
    stdio: ["ignore", driverLog, driverLog],
    detached: true,
  });
  await sleep(2500);
  browser = await remote({
    hostname: "127.0.0.1",
    port: DRIVER_PORT,
    path: "/",
    logLevel: "error",
    connectionRetryCount: 1,
    connectionRetryTimeout: 60_000,
    capabilities: tauriCapabilities(APP, `pdf-routes-${label}`),
  });
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 30_000 });
}

async function stop() {
  try { await browser?.deleteSession(); } catch {}
  browser = undefined;
  try { if (driver?.pid) process.kill(-driver.pid, "SIGKILL"); } catch {}
  driver = undefined;
  await sleep(750);
}

const paneState = () => browser.execute(() =>
  [...document.querySelectorAll(".pane-leaf[data-pane-id]")].map((pane) => ({
    paneId: pane.getAttribute("data-pane-id"),
    pdf: pane.querySelector(".pdf-viewer")?.getAttribute("data-pdf-filename") ?? null,
    page: pane.querySelector(".page-title")?.textContent?.trim() ?? null,
    tabs: [...pane.querySelectorAll(".tab")].map((tab) => ({
      title: tab.querySelector(".tab-title")?.textContent?.trim() ?? "",
      active: tab.classList.contains("active"),
    })),
    pdfLinks: pane.querySelectorAll(".pdf-link").length,
  })),
);

async function waitForPdf(filename) {
  await browser.waitUntil(() => browser.execute((wanted) => {
    const viewer = document.querySelector(`.pdf-viewer[data-pdf-filename="${CSS.escape(wanted)}"]`);
    return viewer?.getAttribute("data-pdf-ready") === "true";
  }, filename), { timeout: 30_000, timeoutMsg: `${filename} did not become a ready PDF route` });
}

async function clickPdfLink(label) {
  const link = await browser.$(`.pdf-link*=${label}`);
  await link.waitForClickable({ timeout: 10_000 });
  await link.click();
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function fnvSessionName(root) {
  let hash = 0xcbf29ce484222325n;
  for (const byte of Buffer.from(root)) {
    hash ^= BigInt(byte);
    hash = BigInt.asUintN(64, hash * 0x100000001b3n);
  }
  const name = path.basename(root).replace(/[^A-Za-z0-9]/g, "_") || "graph";
  return `${name}-${hash.toString(16).padStart(16, "0")}.json`;
}

const sessionPath = path.join(APP_DATA, "sessions", fnvSessionName(fs.realpathSync(GRAPH)));

try {
  await start("route-actions");
  await clickPdfLink("First PDF");
  await waitForPdf("first.pdf");
  let panes = await paneState();
  assert(panes.length === 2, `first PDF did not create exactly one structural companion: ${JSON.stringify(panes)}`);
  const sourcePane = panes.find((pane) => pane.pdfLinks === 2);
  let pdfPane = panes.find((pane) => pane.pdf === "first.pdf");
  assert(sourcePane && pdfPane, `source/PDF pane roles were not both visible: ${JSON.stringify(panes)}`);
  assert(pdfPane.tabs.length === 1 && pdfPane.tabs[0].active && pdfPane.tabs[0].title === "First PDF",
    `PDF was not represented by the active ordinary pane tab: ${JSON.stringify(pdfPane)}`);

  // Pointer focus makes this the same user path as selecting the still-visible
  // source pane before opening its second asset.
  await browser.execute((paneId) => {
    document.querySelector(`.pane-leaf[data-pane-id="${CSS.escape(paneId)}"]`)
      ?.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, button: 0 }));
  }, sourcePane.paneId);
  await clickPdfLink("Second PDF");
  await waitForPdf("second.pdf");
  panes = await paneState();
  pdfPane = panes.find((pane) => pane.pdf === "second.pdf");
  assert(panes.length === 2 && pdfPane?.tabs.length === 2,
    `second PDF did not reuse the companion as a second tab: ${JSON.stringify(panes)}`);
  assert(pdfPane.tabs.some((tab) => tab.title === "First PDF") && pdfPane.tabs.some((tab) => tab.title === "Second PDF" && tab.active),
    `companion PDF tab titles/activation were wrong: ${JSON.stringify(pdfPane)}`);

  await browser.$('button[aria-label="Close PDF"]').click();
  await waitForPdf("first.pdf");
  panes = await paneState();
  pdfPane = panes.find((pane) => pane.pdf === "first.pdf");
  assert(panes.length === 2 && pdfPane?.tabs.length === 1 && pdfPane.tabs[0].title === "First PDF",
    `toolbar Close did not terminally close only the active second PDF tab: ${JSON.stringify(panes)}`);

  const pageInput = await browser.$(".pdf-page-input");
  await pageInput.setValue("2");
  await browser.keys("Enter");
  await browser.waitUntil(async () => (await pageInput.getValue()) === "2", {
    timeout: 10_000,
    timeoutMsg: "PDF page control did not settle on page 2",
  });
  await browser.$('button[title="Zoom in"]').click();
  const savedZoom = await browser.$(".pdf-zoom-level").getText();

  await browser.$(".pdf-notes-btn").click();
  await browser.waitUntil(() => browser.execute(() =>
    [...document.querySelectorAll(".pane-leaf")].some((pane) =>
      pane.querySelector(".page-title")?.textContent?.trim() === "hls__first")), {
    timeout: 15_000,
    timeoutMsg: "Notes did not open in the structural companion pane",
  });
  panes = await paneState();
  const notesPane = panes.find((pane) => pane.page === "hls__first");
  pdfPane = panes.find((pane) => pane.pdf === "first.pdf");
  assert(panes.length === 2 && notesPane?.paneId === sourcePane.paneId && pdfPane?.paneId !== notesPane.paneId,
    `Notes did not reuse the PDF's structural companion: ${JSON.stringify(panes)}`);
  receipt.companionRoute = {
    sourcePane: sourcePane.paneId,
    pdfPane: pdfPane.paneId,
    ordinaryPdfTab: true,
    secondPdfReusedCompanion: true,
    toolbarCloseSelectedCorrectTab: true,
    notesOpenedInStructuralCompanion: true,
  };

  // View position is stored by the reader sidecar; pane/tab layout is stored by
  // the explicit route session. Wait for both debounces before a clean relaunch.
  await sleep(4500);
  assert(fs.existsSync(sessionPath), `route session was not persisted at ${sessionPath}`);
  await stop();

  await start("route-restore");
  await waitForPdf("first.pdf");
  panes = await paneState();
  const restoredPdfPane = panes.find((pane) => pane.pdf === "first.pdf");
  const restoredNotesPane = panes.find((pane) => pane.page === "hls__first");
  assert(panes.length === 2 && restoredPdfPane && restoredNotesPane,
    `saved PDF/Notes pane routes were not restored: ${JSON.stringify(panes)}`);
  const restoredPage = await browser.$(".pdf-page-input").getValue();
  const restoredZoom = await browser.$(".pdf-zoom-level").getText();
  assert(restoredPage === "2", `restored PDF page was ${restoredPage}, expected 2`);
  assert(restoredZoom === savedZoom, `restored PDF zoom was ${restoredZoom}, expected ${savedZoom}`);
  receipt.persistedRoute = {
    layoutRestored: true,
    page: Number(restoredPage),
    zoom: restoredZoom,
  };
  await stop();

  const pageSnapshot = (name) => ({
    tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }],
    activeIndex: 0,
    scrolls: [null],
  });
  const legacy = {
    ...pageSnapshot("Source"),
    layout: {
      kind: "split",
      dir: "row",
      ratio: 0.42,
      children: [
        { kind: "pane", paneId: "main", ...pageSnapshot("Source") },
        { kind: "pane", paneId: "kept-pane", ...pageSnapshot("Other") },
      ],
    },
    focusedPaneId: "main",
    pdfTarget: { filename: "first.pdf", label: "First PDF" },
  };
  fs.mkdirSync(path.dirname(sessionPath), { recursive: true });
  fs.writeFileSync(sessionPath, `${JSON.stringify(legacy, null, 2)}\n`);

  await start("legacy-migration");
  await waitForPdf("first.pdf");
  panes = await paneState();
  const pages = panes.map((pane) => pane.page).filter(Boolean).sort();
  assert(panes.length === 3 && pages.includes("Source") && pages.includes("Other") && panes.some((pane) => pane.pdf === "first.pdf"),
    `legacy PDF migration lost an existing pane or failed to add a route pane: ${JSON.stringify(panes)}`);
  receipt.legacyMigration = {
    preexistingPaneCount: 2,
    migratedPaneCount: panes.length,
    preservedPages: pages,
    pdfRouteAdded: true,
  };

  fs.writeFileSync(path.join(ARTIFACTS, "pdf-routes-native-receipt.json"), `${JSON.stringify(receipt, null, 2)}\n`);
  console.log(`PASS: Direct Files PDF pane routes, restore, and legacy migration are stable: ${JSON.stringify(receipt)}`);
} finally {
  await stop();
  try { fs.closeSync(driverLog); } catch {}
}
