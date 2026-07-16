// Real-app regression for GH #61: current Logseq highlight sidecars use #uuid
// reader tags, list-shaped :rects, and x1/y1/x2/y2 coordinates. Opening the PDF
// used to make the native process allocate without bound before the pane mounted.
import { spawn, spawnSync } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, process.platform === "win32" ? "target/release/tine.exe" : "target/release/tine");
const TD = process.env.TAURI_DRIVER ||
  (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4520);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4521);
const TMP = path.join(os.tmpdir(), `tine-pdf-logseq-e2e-${process.pid}`);
const GRAPH = path.join(TMP, "graph");
const SAMPLE_ID = "6a5604f8-a337-4336-a711-2ba6bc14fbfd";
const PDF = "JVBERi0xLjQKMSAwIG9iago8PCAvVHlwZSAvQ2F0YWxvZyAvUGFnZXMgMiAwIFIgPj4KZW5kb2JqCjIgMCBvYmoKPDwgL1R5cGUgL1BhZ2VzIC9LaWRzIFszIDAgUl0gL0NvdW50IDEgPj4KZW5kb2JqCjMgMCBvYmoKPDwgL1R5cGUgL1BhZ2UgL1BhcmVudCAyIDAgUiAvTWVkaWFCb3ggWzAgMCA2MTIgNzkyXSAvUmVzb3VyY2VzIDw8IC9Gb250IDw8IC9GMSA1IDAgUiA+PiA+PiAvQ29udGVudHMgNCAwIFIgPj4KZW5kb2JqCjQgMCBvYmoKPDwgL0xlbmd0aCAyMDUgPj4Kc3RyZWFtCkJUIC9GMSAyMCBUZiA3MiA3MjAgVGQgKFRpbmUgUERGIHZpZXdlcikgVGogRVQKQlQgL0YxIDEzIFRmIDcyIDY5MCBUZCAoU2VsZWN0IHRoaXMgdGV4dCB0byBjcmVhdGUgYSBoaWdobGlnaHQuKSBUaiBFVApCVCAvRjEgMTMgVGYgNzIgNjY4IFRkIChIaWdobGlnaHRzIHBlcnNpc3QgdG8gYXNzZXRzLzxrZXk+LmVkbiArIGFuIGhsc19fIHBhZ2UuKSBUaiBFVAoKZW5kc3RyZWFtCmVuZG9iago1IDAgb2JqCjw8IC9UeXBlIC9Gb250IC9TdWJ0eXBlIC9UeXBlMSAvQmFzZUZvbnQgL0hlbHZldGljYSA+PgplbmRvYmoKeHJlZgowIDYKMDAwMDAwMDAwMCA2NTUzNSBmIAowMDAwMDAwMDA5IDAwMDAwIG4gCjAwMDAwMDAwNTggMDAwMDAgbiAKMDAwMDAwMDExNSAwMDAwMCBuIAowMDAwMDAwMjQxIDAwMDAwIG4gCjAwMDAwMDA0OTcgMDAwMDAgbiAKdHJhaWxlcgo8PCAvU2l6ZSA2IC9Sb290IDEgMCBSID4+CnN0YXJ0eHJlZgo1NjcKJSVFT0Y=";
const SECOND_FIRST_ID = "7b6704f8-a337-4336-a711-2ba6bc14fbf1";
const SECOND_SECOND_ID = "7b6704f8-a337-4336-a711-2ba6bc14fbf2";
const EDN = `{:highlights [{:id #uuid "6a5604f8-a337-4336-a711-2ba6bc14fbfd"
  :page 1
  :position {:bounding {:x1 96.7058823529 :y1 937.452777778 :x2 333.098039216 :y2 972.372222222
                        :width 822 :height 1063.7}
             :rects ({:x1 96.7058823529 :y1 937.452777778 :x2 333.098039216 :y2 972.372222222
                      :width 822 :height 1063.7})
             :page 1}
  :content {:text "Tine PDF viewer"}
  :properties {:color "yellow"}}]
  :extra {:page 1 :scale 1.75 :plugin "keep"}
  :future-root 42}
`;
const EDN_SECOND = `{:highlights [{:id #uuid "${SECOND_FIRST_ID}"
  :page 1
  :position {:bounding {:x1 96.7058823529 :y1 937.452777778 :x2 333.098039216 :y2 972.372222222
                        :width 822 :height 1063.7}
             :rects ({:x1 96.7058823529 :y1 937.452777778 :x2 333.098039216 :y2 972.372222222
                      :width 822 :height 1063.7})
             :page 1}
  :content {:text "Second PDF first"}
  :properties {:color "yellow"}}
 {:id #uuid "${SECOND_SECOND_ID}"
  :page 1
  :position {:bounding {:x1 96.7058823529 :y1 850 :x2 333.098039216 :y2 884
                        :width 822 :height 1063.7}
             :rects ({:x1 96.7058823529 :y1 850 :x2 333.098039216 :y2 884
                      :width 822 :height 1063.7})
             :page 1}
  :content {:text "Second PDF second"}
  :properties {:color "green"}}]
  :extra {:page 1 :scale 1.25 :plugin "second-keep"}
  :future-root 84}
`;

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, "xdg", dir), { recursive: true });
fs.writeFileSync(path.join(GRAPH, "logseq", "config.edn"), "{:preferred-format \"Org\"}\n");
fs.writeFileSync(path.join(GRAPH, "assets", "logseq-sample.pdf"), Buffer.from(PDF, "base64"));
const secondPdf = Buffer.from(PDF, "base64");
const pdfLabelOffset = secondPdf.indexOf(Buffer.from("Tine PDF viewer"));
if (pdfLabelOffset < 0) throw new Error("PDF fixture label was not found");
Buffer.from("Second PDF test").copy(secondPdf, pdfLabelOffset); // same byte length: xref stays valid
fs.writeFileSync(path.join(GRAPH, "assets", "logseq-second.pdf"), secondPdf);
const sidecar = path.join(GRAPH, "assets", "logseq-sample.edn");
fs.writeFileSync(sidecar, EDN);
const originalSidecar = fs.readFileSync(sidecar, "utf8");
const secondSidecar = path.join(GRAPH, "assets", "logseq-second.edn");
fs.writeFileSync(secondSidecar, EDN_SECOND);
const originalSecondSidecar = fs.readFileSync(secondSidecar, "utf8");
const hlsPage = path.join(GRAPH, "pages", "hls__logseq-sample.org");
// Existing sidecar plus an existing annotation page whose block is missing:
// Copy ref / Linked references must repair the pair through the guarded writer
// before exposing SAMPLE_ID (GH #168).
fs.writeFileSync(hlsPage, [
  "#+FILE: [[../assets/logseq-sample.pdf][Logseq sample]]",
  "#+FILE-PATH: ../assets/logseq-sample.pdf",
  "",
].join("\n"));
fs.writeFileSync(path.join(GRAPH, "pages", "hls__logseq-second.org"), [
  "#+FILE: [[../assets/logseq-second.pdf][Logseq second]]",
  "#+FILE-PATH: ../assets/logseq-second.pdf",
  "* Second PDF first",
  ":PROPERTIES:",
  ":hl-page: 1",
  ":hl-color: yellow",
  ":ls-type: annotation",
  `:id: ${SECOND_FIRST_ID}`,
  ":END:",
  "* Second PDF second",
  ":PROPERTIES:",
  ":hl-page: 1",
  ":hl-color: green",
  ":ls-type: annotation",
  `:id: ${SECOND_SECOND_ID}`,
  ":END:",
  "",
].join("\n"));
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(path.join(GRAPH, "journals", `${journal}.md`), [
  "- ![Logseq sample](../assets/logseq-sample.pdf)",
  "- ![Logseq second](../assets/logseq-second.pdf)",
  `- First exact annotation ((${SECOND_FIRST_ID}))`,
  `- Second exact annotation ((${SECOND_SECOND_ID}))`,
  `- First sample annotation ((${SAMPLE_ID}))`,
  "- [[hls__logseq-second]]",
  "",
].join("\n"));

// Windows Tauri/WebView2 can outlive tauri-driver briefly after a preceding
// scenario. On an isolated CI runner, remove only this test binary before
// starting so the single-instance plugin cannot forward us into the old graph.
function stopCiWindowsApp() {
  if (process.platform === "win32" && process.env.CI === "true") {
    spawnSync("taskkill", ["/IM", path.basename(APP), "/T", "/F"], { stdio: "ignore" });
  }
}
stopCiWindowsApp();

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "xdg", "data"),
  XDG_CONFIG_HOME: path.join(TMP, "xdg", "config"),
  XDG_CACHE_HOME: path.join(TMP, "xdg", "cache"),
  APPDATA: path.join(TMP, "appdata"),
  LOCALAPPDATA: path.join(TMP, "localappdata"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  GDK_BACKEND: "x11",
};
const log = fs.openSync(path.join(process.env.E2E_ARTIFACT_DIR || TMP, "tauri-driver.log"), "w");
const driverArgs = webdriverServerArgs(
  DRIVER_PORT,
  NATIVE_PORT,
  process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver",
);
const td = spawn(TD, driverArgs, { env, stdio: ["ignore", log, log], detached: process.platform !== "win32" });
await sleep(2500);

function processTreeRssKiB() {
  if (process.platform !== "linux") return null;
  const rows = [];
  for (const entry of fs.readdirSync("/proc")) {
    if (!/^\d+$/.test(entry)) continue;
    try {
      const status = fs.readFileSync(`/proc/${entry}/status`, "utf8");
      const ppid = Number(status.match(/^PPid:\s+(\d+)$/m)?.[1] || 0);
      const rss = Number(status.match(/^VmRSS:\s+(\d+)\s+kB$/m)?.[1] || 0);
      let exe = "";
      try { exe = fs.readlinkSync(`/proc/${entry}/exe`); } catch {}
      rows.push({ pid: Number(entry), ppid, rss, exe });
    } catch {}
  }
  const owned = new Set(rows.filter((row) => row.exe === APP).map((row) => row.pid));
  let changed = true;
  while (changed) {
    changed = false;
    for (const row of rows) {
      if (owned.has(row.ppid) && !owned.has(row.pid)) {
        owned.add(row.pid);
        changed = true;
      }
    }
  }
  return rows.filter((row) => owned.has(row.pid)).reduce((sum, row) => sum + row.rss, 0);
}

let browser;
try {
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER_PORT, path: "/", logLevel: "error",
    connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: tauriCapabilities(APP),
  });
  await browser.$(".ls-block").waitForExist({ timeout: 30_000 });
  const pdfLink = browser.$(".pdf-link");
  try {
    await pdfLink.waitForExist({ timeout: 20_000 });
  } catch (error) {
    const diagnostic = await browser.execute(() => ({
      href: location.href,
      title: document.title,
      bodyText: document.body.innerText.slice(0, 4000),
      blocks: Array.from(document.querySelectorAll(".ls-block")).slice(0, 20).map((block) => ({
        text: block.textContent?.slice(0, 500),
        html: block.innerHTML.slice(0, 1500),
      })),
    }));
    const evidenceDir = process.env.E2E_ARTIFACT_DIR || TMP;
    fs.writeFileSync(path.join(evidenceDir, "startup-diagnostic.json"), JSON.stringify(diagnostic, null, 2));
    try { await browser.saveScreenshot(path.join(evidenceDir, "startup-diagnostic.png")); } catch {}
    throw new Error(`PDF link did not render: ${JSON.stringify(diagnostic)}`, { cause: error });
  }
  const before = processTreeRssKiB();
  await pdfLink.click();
  let peak = before;
  for (let i = 0; i < 15; i++) {
    await sleep(200);
    const rss = processTreeRssKiB();
    if (rss != null && before != null) {
      peak = Math.max(peak, rss);
      if (peak - before > 768 * 1024) throw new Error("opening the Logseq PDF grew resident memory by more than 768 MiB");
    }
  }
  await browser.$(".pdf-page canvas").waitForExist({ timeout: 20_000 });
  await browser.$(".pdf-hl").waitForExist({ timeout: 10_000 });
  const restoredZoom = await browser.$(".pdf-zoom-level").getText();
  if (restoredZoom !== "175%") throw new Error(`PDF scale was not restored from :extra: ${restoredZoom}`);
  if (fs.readFileSync(sidecar, "utf8") !== originalSidecar) {
    throw new Error("opening the PDF rewrote an existing Logseq sidecar before a user change");
  }
  const hls = fs.readFileSync(hlsPage, "utf8");
  for (const expected of [
    "#+FILE: [[../assets/logseq-sample.pdf][Logseq sample]]",
    "#+FILE-PATH: ../assets/logseq-sample.pdf",
  ]) {
    if (!hls.includes(expected)) throw new Error(`OG-compatible Org hls page is missing ${expected}: ${hls}`);
  }
  if (hls.includes(`:id: ${SAMPLE_ID}`)) {
    throw new Error("opening an existing hls page unexpectedly repaired its missing annotation before a reference action");
  }
  const geometry = await browser.execute(() => {
    const highlight = document.querySelector(".pdf-hl");
    const page = document.querySelector(".pdf-page");
    return highlight ? {
      left: highlight.style.left,
      top: highlight.style.top,
      width: highlight.style.width,
      height: highlight.style.height,
      leftRatio: parseFloat(highlight.style.left) / parseFloat(page.style.width),
      widthRatio: parseFloat(highlight.style.width) / parseFloat(page.style.width),
    } : null;
  });
  if (!geometry || Object.values(geometry).some((value) => !value || String(value).includes("NaN"))) {
    throw new Error(`Logseq highlight geometry was not converted safely: ${JSON.stringify(geometry)}`);
  }
  if (Math.abs(geometry.leftRatio - 72 / 612) > 0.005 || Math.abs(geometry.widthRatio - 176 / 612) > 0.005) {
    throw new Error(`Logseq zoom-space highlight was misplaced: ${JSON.stringify(geometry)}`);
  }

  // GH #168: both click and right-click share the existing-highlight menu. The
  // reference actions must safely ensure a missing annotation block before
  // copying or routing, and Linked references must reveal ordinary referrers.
  await browser.execute(() => {
    const highlight = document.querySelector(".pdf-hl");
    const rect = highlight?.getBoundingClientRect();
    highlight?.dispatchEvent(new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      clientX: rect?.left ?? 120,
      clientY: rect?.top ?? 120,
      view: window,
    }));
  });
  await browser.$(".pdf-color-menu").waitForExist({ timeout: 5000 });
  // WebKitDriver can invalidate the session when several element-text commands
  // are issued together. Snapshot both labels in one document command instead.
  const existingActions = await browser.execute(() =>
    [...document.querySelectorAll(".pdf-hl-action")].map((element) => element.textContent?.trim()));
  if (!existingActions.includes("Copy ref") || !existingActions.includes("Linked references")) {
    throw new Error(`existing PDF highlight menu omitted reference actions: ${JSON.stringify(existingActions)}`);
  }
  await browser.execute(() => {
    const action = [...document.querySelectorAll(".pdf-hl-action")]
      .find((element) => element.textContent?.trim() === "Copy ref");
    action?.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, view: window }));
  });
  await browser.waitUntil(() => fs.readFileSync(hlsPage, "utf8").includes(`:id: ${SAMPLE_ID}`), {
    timeout: 10_000,
    timeoutMsg: "Copy ref did not safely upsert the missing annotation block",
  });
  await browser.waitUntil(() => browser.execute(() =>
    [...document.querySelectorAll(".toast")].some((toast) => toast.textContent?.includes("Copied highlight ref"))), {
    timeout: 5000,
    timeoutMsg: "Copy ref did not reach the native clipboard success boundary",
  });
  const repairedHls = fs.readFileSync(hlsPage, "utf8");
  for (const expected of ["* Tine PDF viewer", ":hl-page: 1", ":ls-type: annotation", `:id: ${SAMPLE_ID}`]) {
    if (!repairedHls.includes(expected)) throw new Error(`repaired annotation page is missing ${expected}: ${repairedHls}`);
  }

  await browser.execute(() => {
    document.querySelector(".pdf-hl")
      ?.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, view: window }));
  });
  await browser.execute(() => {
    const action = [...document.querySelectorAll(".pdf-hl-action")]
      .find((element) => element.textContent?.trim() === "Linked references");
    action?.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, view: window }));
  });
  await browser.waitUntil(() => browser.execute((highlightId) => {
    const block = document.querySelector(`.ls-block[data-block-id="${highlightId}"]`);
    return block?.querySelector(".block-references")?.textContent?.includes("First sample annotation") ?? false;
  }, SAMPLE_ID), {
    timeout: 10_000,
    timeoutMsg: "Linked references did not open the annotation block with its ordinary referrers visible",
  });
  await browser.$('button[title="Go back"]').click();
  await browser.waitUntil(() => browser.execute(() =>
    document.querySelectorAll(".pdf-link").length >= 2 &&
    document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-filename") === "logseq-sample.pdf"), {
    timeout: 10_000,
    timeoutMsg: "returning from PDF highlight references did not restore the journal surface",
  });

  // OG carries an annotation entity through block-ref navigation, then scrolls
  // to that exact highlight. A filename is the resource identity; a second
  // target in the same file must not remount it. A different filename still is
  // a complete boundary: cleanup flushes A to A, then B opens only B's state.
  await browser.$('button[title="Zoom in"]').click();

  // First prove ordinary direct-link A -> B -> A with real pointer clicks and
  // visibly distinct document bytes, before exercising highlight entry paths.
  let pdfLinks = await browser.$$(".pdf-link");
  await pdfLinks[1].click();
  await browser.waitUntil(() => browser.execute(() =>
    document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-filename") === "logseq-second.pdf" &&
    document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-ready") === "true"), {
    timeout: 10_000,
    timeoutMsg: "a direct PDF link did not switch from A to B",
  });
  await browser.waitUntil(() => browser.execute(() =>
    document.querySelector(".textLayer")?.textContent?.includes("Second PDF test")), {
    timeout: 10_000,
    timeoutMsg: "PDF B's distinct text never appeared",
  });
  await browser.waitUntil(() => {
    const written = fs.readFileSync(sidecar, "utf8");
    return written.includes(":scale 1.93") && written.includes(':plugin "keep"') && written.includes(":future-root 42");
  }, { timeout: 10_000, timeoutMsg: "PDF A cleanup did not persist A's pending view state" });
  if (fs.readFileSync(secondSidecar, "utf8") !== originalSecondSidecar) {
    throw new Error("switching from PDF A wrote A's pending state or baseline into PDF B");
  }
  pdfLinks = await browser.$$(".pdf-link");
  await pdfLinks[0].click();
  await browser.waitUntil(() => browser.execute(() =>
    document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-filename") === "logseq-sample.pdf" &&
    document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-ready") === "true"), {
    timeout: 10_000,
    timeoutMsg: "a direct PDF link did not switch from B back to A",
  });
  const directReturnedZoom = await browser.$(".pdf-zoom-level").getText();
  if (directReturnedZoom !== "193%") throw new Error(`PDF A did not restore its own state after direct B: ${directReturnedZoom}`);

  const firstSecondRef = browser.$(`[data-block-ref="${SECOND_FIRST_ID}"]`);
  await firstSecondRef.waitForExist({ timeout: 10_000 });
  await browser.waitUntil(() => browser.execute((highlightId) =>
    !document.querySelector(`[data-block-ref="${highlightId}"]`)?.classList.contains("block-ref-missing"), SECOND_FIRST_ID), {
    timeout: 10_000,
    timeoutMsg: "the first PDF annotation block reference did not resolve",
  });
  await firstSecondRef.click();
  try {
    await browser.waitUntil(() => browser.execute((highlightId) =>
      document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-filename") === "logseq-second.pdf" &&
      document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-highlight-target") === highlightId &&
      document.querySelector(".pdf-hl-target")?.getAttribute("data-highlight-id") === highlightId &&
      document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-ready") === "true", SECOND_FIRST_ID), {
      timeout: 10_000,
      timeoutMsg: "a block reference did not open PDF B at its exact first highlight",
    });
  } catch (error) {
    const diagnostic = await browser.execute(() => ({
      filename: document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-filename"),
      target: document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-highlight-target"),
      ready: document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-ready"),
      highlights: [...document.querySelectorAll(".pdf-hl")].map((element) => ({
        id: element.getAttribute("data-highlight-id"),
        target: element.classList.contains("pdf-hl-target"),
      })),
      toast: document.querySelector(".toast")?.textContent,
      references: [...document.querySelectorAll("[data-block-ref]")].map((element) => ({
        id: element.getAttribute("data-block-ref"),
        className: element.className,
        title: element.getAttribute("title"),
      })),
      location: location.href,
      headings: [...document.querySelectorAll("h1,h2,.page-title")].map((element) => element.textContent),
    }));
    throw new Error(`${error.message}: ${JSON.stringify(diagnostic)}`, { cause: error });
  }
  const secondViewerElementId = (await browser.$(".pdf-viewer")).elementId;
  const secondZoom = await browser.$(".pdf-zoom-level").getText();
  if (secondZoom !== "125%") throw new Error(`second PDF inherited the first PDF scale: ${secondZoom}`);
  if (fs.readFileSync(secondSidecar, "utf8") !== originalSecondSidecar) {
    throw new Error("switching from PDF A wrote A's pending state or baseline into PDF B");
  }

  await browser.$(`[data-block-ref="${SECOND_SECOND_ID}"]`).click();
  await browser.waitUntil(() => browser.execute((highlightId) =>
    document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-highlight-target") === highlightId &&
    document.querySelector(".pdf-hl-target")?.getAttribute("data-highlight-id") === highlightId, SECOND_SECOND_ID), {
    timeout: 10_000,
    timeoutMsg: "a same-PDF block reference did not scroll to its exact second highlight",
  });
  if ((await browser.$(".pdf-viewer")).elementId !== secondViewerElementId) {
    throw new Error("same-PDF highlight navigation remounted the PDF resource");
  }
  if (fs.readFileSync(secondSidecar, "utf8") !== originalSecondSidecar) {
    throw new Error("same-PDF highlight navigation rewrote the sidecar");
  }

  // OG direct-link parity: reopening the already-current resource without a
  // page/highlight intent is a no-op, not an implicit jump to page 1. The exact
  // current highlight and the mounted reader identity must survive.
  pdfLinks = await browser.$$(".pdf-link");
  await pdfLinks[1].click();
  await sleep(250);
  const sameResourceDirectState = await browser.execute(() => ({
    target: document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-highlight-target"),
    overlay: document.querySelector(".pdf-hl-target")?.getAttribute("data-highlight-id"),
  }));
  if (sameResourceDirectState.target !== SECOND_SECOND_ID || sameResourceDirectState.overlay !== SECOND_SECOND_ID) {
    throw new Error(`same-resource direct link discarded the current location: ${JSON.stringify(sameResourceDirectState)}`);
  }
  if ((await browser.$(".pdf-viewer")).elementId !== secondViewerElementId) {
    throw new Error("same-resource direct link remounted the PDF resource");
  }

  // Return to A while the journal's real links are still present. Recoloring is
  // a real mutation and must preserve foreign root metadata while writing the
  // UUID/list/corner shape Logseq consumes.
  pdfLinks = await browser.$$(".pdf-link");
  await pdfLinks[0].click();
  await browser.waitUntil(() => browser.execute(() =>
    document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-filename") === "logseq-sample.pdf" &&
    document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-ready") === "true"), {
    timeout: 10_000,
    timeoutMsg: "switching back to PDF A did not remount and finish loading its identity",
  });
  const returnedZoom = await browser.$(".pdf-zoom-level").getText();
  if (returnedZoom !== "193%") throw new Error(`PDF A did not restore its own state after B: ${returnedZoom}`);
  await browser.$(".pdf-hl").waitForExist({
    timeout: 10_000,
    timeoutMsg: "PDF A restored its view state before its highlight overlay was ready",
  });
  await browser.execute(() => document.querySelector(".pdf-hl")?.scrollIntoView({ block: "center", inline: "center" }));
  await browser.execute(() => {
    document.querySelector(".pdf-hl")
      ?.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, view: window }));
  });
  await browser.$(".pdf-color-menu").waitForExist({ timeout: 5000 });
  await browser.execute(() => {
    document.querySelector(".pdf-color-swatch")
      ?.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, view: window }));
  });
  await browser.waitUntil(() => {
    const written = fs.readFileSync(sidecar, "utf8");
    return written.includes('#uuid "6a5604f8-a337-4336-a711-2ba6bc14fbfd"') &&
      written.includes(":rects (") && written.includes(":x1 ") &&
      written.includes(":extra {:page 1") && written.includes(':plugin "keep"') && written.includes(":future-root 42");
  }, { timeout: 10_000, timeoutMsg: "Tine did not persist a Logseq-compatible sidecar" });

  // Exercise the sibling entry surface too: navigate to the hls page and click
  // a different rendered annotation badge, not merely an inline ((block ref)).
  await browser.$(`[data-block-ref="${SECOND_FIRST_ID}"]`).click();
  await browser.waitUntil(() => browser.execute((highlightId) =>
    document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-filename") === "logseq-second.pdf" &&
    document.querySelector(".pdf-hl-target")?.getAttribute("data-highlight-id") === highlightId, SECOND_FIRST_ID), {
    timeout: 10_000,
    timeoutMsg: "the second B open did not restore its exact first highlight",
  });
  const annotationViewerElementId = (await browser.$(".pdf-viewer")).elementId;
  const hlsLink = browser.$('//a[contains(@class,"ref") and contains(normalize-space(.),"logseq-second")]');
  await hlsLink.waitForExist({ timeout: 10_000 });
  await hlsLink.click();
  await browser.$(`.pdf-annotation-line .hl-prefix[data-highlight-id="${SECOND_SECOND_ID}"]`)
    .waitForExist({ timeout: 10_000 });
  await browser.$(`.pdf-annotation-line .hl-prefix[data-highlight-id="${SECOND_SECOND_ID}"]`).click();
  await browser.waitUntil(() => browser.execute((highlightId) =>
    document.querySelector(".pdf-hl-target")?.getAttribute("data-highlight-id") === highlightId, SECOND_SECOND_ID), {
    timeout: 10_000,
    timeoutMsg: "a rendered AnnotationBody did not navigate to its exact highlight",
  });
  if ((await browser.$(".pdf-viewer")).elementId !== annotationViewerElementId) {
    throw new Error("AnnotationBody navigation remounted the same PDF resource");
  }
  console.log(`PASS: Logseq PDFs keep asset identity separate from exact highlight navigation, with bounded resources, restored view state, OG-compatible notes, correct geometry, and compatible write-back on ${process.platform}`);
} finally {
  try { await browser?.deleteSession(); } catch {}
  if (process.platform === "win32") spawnSync("taskkill", ["/PID", String(td.pid), "/T", "/F"], { stdio: "ignore" });
  else try { process.kill(-td.pid, "SIGKILL"); } catch {}
  stopCiWindowsApp();
  fs.closeSync(log);
}
