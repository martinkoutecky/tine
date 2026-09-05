// Real-app regression for long-PDF scroll geometry and bounded rendering.
// The fixture is generated at runtime so no user document enters the repository.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  startWebdriverApplication,
  stopWebdriverApplication,
  tauriCapabilities,
  webdriverServerArgs,
} from "./e2e-capabilities.mjs";
import { ensureDisplay, stopDisplay } from "./lib/e2e-display.mjs";

const PAGE_COUNT = 34;
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER ||
  (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const WD = process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver";
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4630);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4631);
const TMP = path.join(os.tmpdir(), `tine-pdf-scroll-resources-${process.pid}`);
const GRAPH = path.join(TMP, "graph");
const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || TMP;
const APP_REAL = fs.realpathSync(APP);

function makeLongPdf() {
  const pageIds = Array.from({ length: PAGE_COUNT }, (_, index) => 4 + index * 2);
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    `<< /Type /Pages /Kids [${pageIds.map((id) => `${id} 0 R`).join(" ")}] /Count ${PAGE_COUNT} >>`,
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
  ];
  for (let page = 1; page <= PAGE_COUNT; page += 1) {
    const pageId = 4 + (page - 1) * 2;
    const contentId = pageId + 1;
    const content = [
      "BT /F1 24 Tf 72 720 Td",
      `(Long PDF resource regression page ${page}) Tj`,
      "0 -32 Td /F1 13 Tf",
      "(The page wrapper must retain its geometry when its canvas is evicted.) Tj ET",
    ].join("\n");
    objects.push(
      `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents ${contentId} 0 R >>`,
      `<< /Length ${Buffer.byteLength(content)} >>\nstream\n${content}\nendstream`,
    );
  }

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
for (const dir of ["pages", "journals", "logseq", "assets"]) {
  fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
}
for (const dir of ["data", "config", "cache"]) {
  fs.mkdirSync(path.join(TMP, "xdg", dir), { recursive: true });
}
fs.mkdirSync(ARTIFACTS, { recursive: true });
fs.writeFileSync(path.join(GRAPH, "assets", "long.pdf"), makeLongPdf());
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}.md`;
fs.writeFileSync(path.join(GRAPH, "journals", journal), "- ![Long PDF](../assets/long.pdf)\n");

// Provision the display BEFORE snapshotting process.env. `env` is what the app
// is launched with, so calling ensureDisplay() after this object is built hands
// the app an environment with no DISPLAY: it dies inside the driver with
// "Failed to initialize gtk backend!" and the journey only ever sees
// UND_ERR_HEADERS_TIMEOUT from the WebDriver session POST -- which reads as a
// hang, and is the exact misdiagnosis scripts/lib/e2e-display.mjs was written
// to prevent. It was invisible under scripts/run-e2e.mjs, which wraps every
// native Linux scenario in `xvfb-run -a` so DISPLAY is already set; only a
// direct run of this file hit it. Every sibling journey calls ensureDisplay at
// the top for this reason.
await ensureDisplay({ geometry: "1600x1100x24" });

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

function assert(condition, message, details) {
  if (!condition) {
    throw new Error(`${message}${details === undefined ? "" : `: ${JSON.stringify(details)}`}`);
  }
}

async function click(browser, selector) {
  const target = await browser.$(selector);
  await target.waitForExist({ timeout: 15_000 });
  await target.click();
}

function appMemory() {
  const rows = [];
  for (const entry of fs.readdirSync("/proc")) {
    if (!/^\d+$/.test(entry)) continue;
    try {
      const status = fs.readFileSync(`/proc/${entry}/status`, "utf8");
      let exe = "";
      let smaps = "";
      try { exe = fs.readlinkSync(`/proc/${entry}/exe`); } catch {}
      try { smaps = fs.readFileSync(`/proc/${entry}/smaps_rollup`, "utf8"); } catch {}
      rows.push({
        pid: Number(entry),
        ppid: Number(status.match(/^PPid:\s+(\d+)$/m)?.[1] || 0),
        exe,
        rssKiB: Number(status.match(/^VmRSS:\s+(\d+)\s+kB$/m)?.[1] || 0),
        pssKiB: Number(smaps.match(/^Pss:\s+(\d+)\s+kB$/m)?.[1] || 0),
      });
    } catch {}
  }
  const owned = new Set(rows.filter((row) => row.exe === APP_REAL).map((row) => row.pid));
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
  const appRows = rows.filter((row) => owned.has(row.pid));
  return {
    rssKiB: appRows.reduce((total, row) => total + row.rssKiB, 0),
    pssKiB: appRows.reduce((total, row) => total + row.pssKiB, 0),
    processCount: appRows.length,
  };
}

async function sample(browser) {
  const dom = await browser.execute(() => {
    const scroll = document.querySelector(".pdf-scroll");
    if (!(scroll instanceof HTMLElement)) return null;
    const viewport = scroll.getBoundingClientRect();
    const pages = [...scroll.querySelectorAll(".pdf-page")].map((page) => {
      const rect = page.getBoundingClientRect();
      const canvas = page.querySelector("canvas");
      return {
        page: Number(page.dataset.page),
        height: rect.height,
        visible: rect.bottom > viewport.top && rect.top < viewport.bottom,
        canvasPixels: canvas instanceof HTMLCanvasElement ? canvas.width * canvas.height : 0,
      };
    });
    return {
      scrollTop: scroll.scrollTop,
      scrollHeight: scroll.scrollHeight,
      clientHeight: scroll.clientHeight,
      pages,
      visible: pages.filter((page) => page.visible).map((page) => page.page),
      canvasCount: pages.filter((page) => page.canvasPixels > 0).length,
      canvasPixels: pages.reduce((total, page) => total + page.canvasPixels, 0),
      activeRenders: Number(scroll.dataset.activeRenders || 0),
      queuedRenders: Number(scroll.dataset.queuedRenders || 0),
      maxQueuedRenders: Number(scroll.dataset.maxQueuedRenders || 0),
      cancelledRenders: Number(scroll.dataset.cancelledRenders || 0),
      fastScrolling: scroll.dataset.fastScrolling === "true",
      frameDiag: window.__tinePdfFrameDiag ?? null,
    };
  });
  return { ...dom, memory: appMemory() };
}

async function wheel(browser, deltaY, id) {
  const center = await browser.execute(() => {
    const rect = document.querySelector(".pdf-scroll")?.getBoundingClientRect();
    return rect ? { x: Math.round(rect.left + rect.width / 2), y: Math.round(rect.top + rect.height / 2) } : null;
  });
  assert(center, "PDF scroller has no geometry");
  try {
    await browser.performActions([{
      type: "wheel",
      id,
      actions: [{ type: "scroll", origin: "viewport", x: center.x, y: center.y, deltaX: 0, deltaY, duration: 80 }],
    }]);
  } finally {
    await browser.releaseActions().catch(() => {});
  }
}

const driverLog = fs.openSync(path.join(ARTIFACTS, "tauri-driver.log"), "w");
let webviewTarget;
let driver;
let browser;
const observations = [];
try {
  webviewTarget = await startWebdriverApplication(APP, env, NATIVE_PORT, "pdf-scroll-resources");
  driver = spawn(TD, webdriverServerArgs(DRIVER_PORT, NATIVE_PORT, WD), {
    env: webviewTarget.env,
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
    capabilities: tauriCapabilities(APP, "pdf-scroll-resources", process.platform, webviewTarget.debuggerAddress),
  });
  await browser.$(".pdf-link").waitForExist({ timeout: 30_000 });
  await click(browser, ".pdf-link");
  await browser.waitUntil(() => browser.execute(() =>
    document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-ready") === "true"), {
    timeout: 30_000,
    timeoutMsg: "long PDF did not become ready",
  });

  await browser.execute(() => {
    const state = window.__tinePdfFrameDiag = { frames: 0, maxGapMs: 0 };
    let previous = performance.now();
    const frame = (now) => {
      state.frames += 1;
      state.maxGapMs = Math.max(state.maxGapMs, now - previous);
      previous = now;
      requestAnimationFrame(frame);
    };
    requestAnimationFrame(frame);
  });

  // Allow intersection callbacks, rendering, and cache eviction to settle.
  await sleep(2500);
  const ready = await sample(browser);
  observations.push({ label: "ready", ...ready });
  assert(ready?.pages.length === PAGE_COUNT, "long PDF did not build every page wrapper", ready?.pages.length);
  const shortestPage = Math.min(...ready.pages.map((page) => page.height));
  assert(shortestPage >= 500, "offscreen PDF page wrappers collapsed after canvas eviction", { shortestPage, scrollHeight: ready.scrollHeight });
  assert(ready.canvasCount < PAGE_COUNT / 2, "opening a long PDF rendered distant offscreen pages", ready.canvasCount);
  assert(ready.canvasPixels <= 52_000_000, "PDF canvases exceeded the desktop backing-store budget", ready.canvasPixels);

  // The app deliberately gives Ctrl/Cmd +/- to whichever pane the user most
  // recently focused. Opening a companion PDF preserves focus in the source
  // pane, so select the reader before exercising its keyboard zoom contract.
  await click(browser, ".pdf-scroll");

  // A retained ordinary canvas is already width/height:100% of its page
  // wrapper. During optimistic zoom it must never receive a second scale
  // transform: the released failure made one Ctrl+ step overshoot, then visibly
  // shrink when the 120 ms settled render replaced it.
  await browser.execute(() => {
    const probe = window.__tinePdfZoomProbe = { startedAt: performance.now(), samples: [], done: false };
    const frame = (now) => {
      const scroll = document.querySelector(".pdf-scroll");
      const viewport = scroll?.getBoundingClientRect();
      const page = viewport && [...scroll.querySelectorAll(".pdf-page")]
        .find((candidate) => {
          const rect = candidate.getBoundingClientRect();
          return rect.bottom > viewport.top && rect.top < viewport.bottom;
        });
      const canvas = page?.querySelector(".canvasWrapper canvas:not([data-pdf-tile-key])");
      if (page instanceof HTMLElement && canvas instanceof HTMLCanvasElement) {
        const pageRect = page.getBoundingClientRect();
        const canvasRect = canvas.getBoundingClientRect();
        probe.samples.push({
          elapsedMs: now - probe.startedAt,
          zoom: document.querySelector(".pdf-zoom-level")?.textContent?.trim() ?? "",
          pageWidth: pageRect.width,
          canvasWidth: canvasRect.width,
          canvasToPage: pageRect.width > 0 ? canvasRect.width / pageRect.width : 0,
          transform: canvas.style.transform,
        });
      }
      if (now - probe.startedAt < 750) requestAnimationFrame(frame);
      else probe.done = true;
    };
    requestAnimationFrame(frame);
  });
  await browser.keys(["Control", "="]);
  await browser.waitUntil(() => browser.execute(() => window.__tinePdfZoomProbe?.done === true), {
    timeout: 5_000,
    timeoutMsg: "PDF zoom frame probe did not finish",
  });
  const zoomProbe = await browser.execute(() => window.__tinePdfZoomProbe);
  observations.push({ label: "single-step-zoom", zoomProbe });
  assert(zoomProbe?.samples?.length > 2, "PDF zoom probe collected too few frames", zoomProbe);
  const finalZoom = zoomProbe.samples.at(-1)?.zoom;
  assert(finalZoom && finalZoom !== zoomProbe.samples[0]?.zoom, "Ctrl+ did not change focused PDF zoom", zoomProbe);
  const maxCanvasToPage = Math.max(...zoomProbe.samples.map((entry) => entry.canvasToPage));
  assert(maxCanvasToPage <= 1.02, "ordinary PDF canvas overshot its resized wrapper during zoom", {
    maxCanvasToPage,
    samples: zoomProbe.samples.filter((entry) => entry.canvasToPage > 1.02),
  });

  let previousFirst = ready.visible[0] ?? 1;
  for (let index = 0; index < 12; index += 1) {
    await wheel(browser, 240, `gentle-${index}`);
    await sleep(180);
    const state = await sample(browser);
    observations.push({ label: `gentle-${index + 1}`, ...state });
    const first = state.visible[0] ?? previousFirst;
    assert(first >= previousFirst, "gentle downward scrolling moved backward", { previousFirst, first });
    assert(first - previousFirst <= 2, "gentle scrolling skipped across PDF pages", { previousFirst, first, visible: state.visible });
    assert(Math.min(...state.pages.map((page) => page.height)) >= 500, "PDF page geometry collapsed while scrolling", state.visible);
    assert(state.canvasPixels <= 52_000_000, "PDF canvases exceeded the desktop backing-store budget while scrolling", state.canvasPixels);
    previousFirst = first;
  }

  // Exercise the reported expensive shape: zoom first, then outrun page
  // rasterization with a rapid wheel burst. Memory and frame gaps are evidence,
  // not exact cross-machine gates; the semantic assertions are bounded work and
  // a nonblank destination page after the burst settles.
  for (let index = 0; index < 6; index += 1) {
    await click(browser, 'button[title="Zoom in"]');
  }
  await sleep(500);
  observations.push({ label: "zoom-settled", ...await sample(browser) });
  for (let index = 0; index < 20; index += 1) {
    await wheel(browser, 700, `rapid-${index}`);
    await sleep(35);
    observations.push({ label: `rapid-${index + 1}`, ...await sample(browser) });
  }
  await sleep(800);
  const stress = await sample(browser);
  observations.push({ label: "rapid-settled", ...stress });
  assert(stress.activeRenders <= 1, "PDF scheduler admitted concurrent full-page renders", stress);
  assert(stress.visible.length > 0, "rapid PDF scrolling lost the visible page", stress);
  assert(
    stress.pages.some((page) => page.visible && page.canvasPixels > 0),
    "rapid PDF scrolling settled on a blank viewport",
    stress,
  );
  assert(stress.canvasPixels <= 52_000_000, "PDF canvases exceeded the desktop budget after rapid zoom/scroll", stress.canvasPixels);

  fs.writeFileSync(path.join(ARTIFACTS, "pdf-scroll-resources.json"), `${JSON.stringify(observations, null, 2)}\n`);
  console.log(JSON.stringify({
    ok: true,
    pageCount: PAGE_COUNT,
    initialCanvasCount: ready.canvasCount,
    shortestPage,
    scrollHeight: ready.scrollHeight,
    finalVisible: observations.at(-1)?.visible,
    finalMemory: stress.memory,
    maxFrameGapMs: stress.frameDiag?.maxGapMs,
    maxQueuedRenders: stress.maxQueuedRenders,
    cancelledRenders: stress.cancelledRenders,
  }));
} catch (error) {
  fs.writeFileSync(path.join(ARTIFACTS, "pdf-scroll-resources.json"), `${JSON.stringify(observations, null, 2)}\n`);
  try { await browser?.saveScreenshot(path.join(ARTIFACTS, "pdf-scroll-resources-failure.png")); } catch {}
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { if (driver?.pid) process.kill(-driver.pid, "SIGKILL"); } catch {}
  stopWebdriverApplication(webviewTarget);
  stopDisplay();
  fs.closeSync(driverLog);
}
