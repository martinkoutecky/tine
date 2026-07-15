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

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, process.platform === "win32" ? "target/release/tine.exe" : "target/release/tine");
const TD = process.env.TAURI_DRIVER ||
  (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4520);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4521);
const TMP = path.join(os.tmpdir(), `tine-pdf-logseq-e2e-${process.pid}`);
const GRAPH = path.join(TMP, "graph");
const PDF = "JVBERi0xLjQKMSAwIG9iago8PCAvVHlwZSAvQ2F0YWxvZyAvUGFnZXMgMiAwIFIgPj4KZW5kb2JqCjIgMCBvYmoKPDwgL1R5cGUgL1BhZ2VzIC9LaWRzIFszIDAgUl0gL0NvdW50IDEgPj4KZW5kb2JqCjMgMCBvYmoKPDwgL1R5cGUgL1BhZ2UgL1BhcmVudCAyIDAgUiAvTWVkaWFCb3ggWzAgMCA2MTIgNzkyXSAvUmVzb3VyY2VzIDw8IC9Gb250IDw8IC9GMSA1IDAgUiA+PiA+PiAvQ29udGVudHMgNCAwIFIgPj4KZW5kb2JqCjQgMCBvYmoKPDwgL0xlbmd0aCAyMDUgPj4Kc3RyZWFtCkJUIC9GMSAyMCBUZiA3MiA3MjAgVGQgKFRpbmUgUERGIHZpZXdlcikgVGogRVQKQlQgL0YxIDEzIFRmIDcyIDY5MCBUZCAoU2VsZWN0IHRoaXMgdGV4dCB0byBjcmVhdGUgYSBoaWdobGlnaHQuKSBUaiBFVApCVCAvRjEgMTMgVGYgNzIgNjY4IFRkIChIaWdobGlnaHRzIHBlcnNpc3QgdG8gYXNzZXRzLzxrZXk+LmVkbiArIGFuIGhsc19fIHBhZ2UuKSBUaiBFVAoKZW5kc3RyZWFtCmVuZG9iago1IDAgb2JqCjw8IC9UeXBlIC9Gb250IC9TdWJ0eXBlIC9UeXBlMSAvQmFzZUZvbnQgL0hlbHZldGljYSA+PgplbmRvYmoKeHJlZgowIDYKMDAwMDAwMDAwMCA2NTUzNSBmIAowMDAwMDAwMDA5IDAwMDAwIG4gCjAwMDAwMDAwNTggMDAwMDAgbiAKMDAwMDAwMDExNSAwMDAwMCBuIAowMDAwMDAwMjQxIDAwMDAwIG4gCjAwMDAwMDA0OTcgMDAwMDAgbiAKdHJhaWxlcgo8PCAvU2l6ZSA2IC9Sb290IDEgMCBSID4+CnN0YXJ0eHJlZgo1NjcKJSVFT0Y=";
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
const EDN_SECOND = `{:highlights []
  :extra {:page 1 :scale 1.25 :plugin "second-keep"}
  :future-root 84}
`;

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, "xdg", dir), { recursive: true });
fs.writeFileSync(path.join(GRAPH, "logseq", "config.edn"), "{:preferred-format \"Org\"}\n");
fs.writeFileSync(path.join(GRAPH, "assets", "logseq-sample.pdf"), Buffer.from(PDF, "base64"));
fs.writeFileSync(path.join(GRAPH, "assets", "logseq-second.pdf"), Buffer.from(PDF, "base64"));
const sidecar = path.join(GRAPH, "assets", "logseq-sample.edn");
fs.writeFileSync(sidecar, EDN);
const originalSidecar = fs.readFileSync(sidecar, "utf8");
const secondSidecar = path.join(GRAPH, "assets", "logseq-second.edn");
fs.writeFileSync(secondSidecar, EDN_SECOND);
const originalSecondSidecar = fs.readFileSync(secondSidecar, "utf8");
const hlsPage = path.join(GRAPH, "pages", "hls__logseq-sample.org");
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(path.join(GRAPH, "journals", `${journal}.md`), [
  "- ![Logseq sample](../assets/logseq-sample.pdf)",
  "- ![Logseq second](../assets/logseq-second.pdf)",
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
const driverArgs = process.platform === "win32"
  ? ["--port", String(DRIVER_PORT)]
  : [
      "--port", String(DRIVER_PORT),
      "--native-port", String(NATIVE_PORT),
      "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver",
    ];
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
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
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
    "* Tine PDF viewer",
    ":PROPERTIES:",
    ":hl-page: 1",
    ":ls-type: annotation",
    ":id: 6a5604f8-a337-4336-a711-2ba6bc14fbfd",
  ]) {
    if (!hls.includes(expected)) throw new Error(`OG-compatible Org hls page is missing ${expected}: ${hls}`);
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

  // OG persists last-view page/scale in :extra after a debounce. Tine must
  // update only those fields, retaining both unknown :extra and root data. A
  // target switch is a complete viewer identity boundary: cleanup flushes A to
  // A, then B opens with only B's state and sidecar baseline.
  await browser.$('button[title="Zoom in"]').click();
  await browser.execute(() => {
    const links = [...document.querySelectorAll(".pdf-link")];
    links[1]?.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
  });
  await browser.waitUntil(() => browser.execute(() =>
    document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-filename") === "logseq-second.pdf"), {
    timeout: 10_000,
    timeoutMsg: "switching PDFs did not replace the viewer identity",
  });
  const secondZoom = await browser.$(".pdf-zoom-level").getText();
  if (secondZoom !== "125%") throw new Error(`second PDF inherited the first PDF scale: ${secondZoom}`);
  await browser.waitUntil(() => {
    const written = fs.readFileSync(sidecar, "utf8");
    return written.includes(":scale 1.93") && written.includes(':plugin "keep"') && written.includes(":future-root 42");
  }, { timeout: 10_000, timeoutMsg: "PDF A cleanup did not persist A's pending view state" });
  if (fs.readFileSync(secondSidecar, "utf8") !== originalSecondSidecar) {
    throw new Error("switching from PDF A wrote A's pending state or baseline into PDF B");
  }
  await browser.execute(() => {
    const links = [...document.querySelectorAll(".pdf-link")];
    links[0]?.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
  });
  await browser.waitUntil(() => browser.execute(() =>
    document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-filename") === "logseq-sample.pdf"), {
    timeout: 10_000,
    timeoutMsg: "switching back to PDF A did not remount its identity",
  });
  const returnedZoom = await browser.$(".pdf-zoom-level").getText();
  if (returnedZoom !== "193%") throw new Error(`PDF A did not restore its own state after B: ${returnedZoom}`);

  // Recoloring is a real mutation. It must preserve the foreign root metadata
  // while writing the UUID/list/corner shape that Logseq itself consumes.
  // Center the highlight explicitly before asking WebDriver to click it. Its
  // implicit scroll can otherwise place a zoomed highlight underneath the
  // fixed PDF toolbar and report an intercepted click even though the element
  // itself is a valid pointer target in the app.
  await browser.execute(() => document.querySelector(".pdf-hl")?.scrollIntoView({ block: "center", inline: "center" }));
  await browser.$(".pdf-hl").click();
  await browser.$(".pdf-color-menu").waitForExist({ timeout: 5000 });
  await browser.execute(() => {
    document.querySelector(".pdf-color-swatch")?.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
  });
  await browser.waitUntil(() => {
    const written = fs.readFileSync(sidecar, "utf8");
    return written.includes('#uuid "6a5604f8-a337-4336-a711-2ba6bc14fbfd"') &&
      written.includes(":rects (") && written.includes(":x1 ") &&
      written.includes(":extra {:page 1") && written.includes(':plugin "keep"') && written.includes(":future-root 42");
  }, { timeout: 10_000, timeoutMsg: "Tine did not persist a Logseq-compatible sidecar" });
  console.log(`PASS: Logseq PDFs switch as isolated identities with bounded resources, restored view state, OG-compatible notes, correct geometry, and compatible write-back on ${process.platform}`);
} finally {
  try { await browser?.deleteSession(); } catch {}
  if (process.platform === "win32") spawnSync("taskkill", ["/PID", String(td.pid), "/T", "/F"], { stdio: "ignore" });
  else try { process.kill(-td.pid, "SIGKILL"); } catch {}
  stopCiWindowsApp();
  fs.closeSync(log);
}
