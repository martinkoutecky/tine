// Real-app regression for GH #61: current Logseq highlight sidecars use #uuid
// reader tags, list-shaped :rects, and x1/y1/x2/y2 coordinates. Opening the PDF
// used to make the native process allocate without bound before the pane mounted.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER ||
  (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4520);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4521);
const TMP = "/tmp/tine-pdf-logseq-e2e";
const GRAPH = `${TMP}/graph`;
const PDF = "JVBERi0xLjQKMSAwIG9iago8PCAvVHlwZSAvQ2F0YWxvZyAvUGFnZXMgMiAwIFIgPj4KZW5kb2JqCjIgMCBvYmoKPDwgL1R5cGUgL1BhZ2VzIC9LaWRzIFszIDAgUl0gL0NvdW50IDEgPj4KZW5kb2JqCjMgMCBvYmoKPDwgL1R5cGUgL1BhZ2UgL1BhcmVudCAyIDAgUiAvTWVkaWFCb3ggWzAgMCA2MTIgNzkyXSAvUmVzb3VyY2VzIDw8IC9Gb250IDw8IC9GMSA1IDAgUiA+PiA+PiAvQ29udGVudHMgNCAwIFIgPj4KZW5kb2JqCjQgMCBvYmoKPDwgL0xlbmd0aCAyMDUgPj4Kc3RyZWFtCkJUIC9GMSAyMCBUZiA3MiA3MjAgVGQgKFRpbmUgUERGIHZpZXdlcikgVGogRVQKQlQgL0YxIDEzIFRmIDcyIDY5MCBUZCAoU2VsZWN0IHRoaXMgdGV4dCB0byBjcmVhdGUgYSBoaWdobGlnaHQuKSBUaiBFVApCVCAvRjEgMTMgVGYgNzIgNjY4IFRkIChIaWdobGlnaHRzIHBlcnNpc3QgdG8gYXNzZXRzLzxrZXk+LmVkbiArIGFuIGhsc19fIHBhZ2UuKSBUaiBFVAoKZW5kc3RyZWFtCmVuZG9iago1IDAgb2JqCjw8IC9UeXBlIC9Gb250IC9TdWJ0eXBlIC9UeXBlMSAvQmFzZUZvbnQgL0hlbHZldGljYSA+PgplbmRvYmoKeHJlZgowIDYKMDAwMDAwMDAwMCA2NTUzNSBmIAowMDAwMDAwMDA5IDAwMDAwIG4gCjAwMDAwMDAwNTggMDAwMDAgbiAKMDAwMDAwMDExNSAwMDAwMCBuIAowMDAwMDAwMjQxIDAwMDAwIG4gCjAwMDAwMDA0OTcgMDAwMDAgbiAKdHJhaWxlcgo8PCAvU2l6ZSA2IC9Sb290IDEgMCBSID4+CnN0YXJ0eHJlZgo1NjcKJSVFT0Y=";
const EDN = `{:highlights [{:id #uuid "6a5604f8-a337-4336-a711-2ba6bc14fbfd"
  :page 1
  :position {:bounding {:x1 72 :y1 698 :x2 248 :y2 724 :width 612 :height 792}
             :rects ({:x1 72 :y1 698 :x2 248 :y2 724 :width 612 :height 792})
             :page 1}
  :content {:text "Tine PDF viewer"}
  :properties {:color "yellow"}}]
  :extra {:page 1}}
`;

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
fs.writeFileSync(`${GRAPH}/assets/logseq-sample.pdf`, Buffer.from(PDF, "base64"));
fs.writeFileSync(`${GRAPH}/assets/logseq-sample.edn`, EDN);
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- ![Logseq sample](../assets/logseq-sample.pdf)\n");

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: `${TMP}/xdg/data`,
  XDG_CONFIG_HOME: `${TMP}/xdg/config`,
  XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  GDK_BACKEND: "x11",
};
const log = fs.openSync(`${TMP}/tauri-driver.log`, "w");
const td = spawn(TD, [
  "--port", String(DRIVER_PORT),
  "--native-port", String(NATIVE_PORT),
  "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver",
], { env, stdio: ["ignore", log, log], detached: true });
await sleep(2500);

function processTreeRssKiB() {
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
  await browser.$(".pdf-link").waitForExist({ timeout: 20_000 });
  const before = processTreeRssKiB();
  await browser.$(".pdf-link").click();
  let peak = before;
  for (let i = 0; i < 15; i++) {
    await sleep(200);
    peak = Math.max(peak, processTreeRssKiB());
    if (peak - before > 768 * 1024) throw new Error("opening the Logseq PDF grew resident memory by more than 768 MiB");
  }
  await browser.$(".pdf-page canvas").waitForExist({ timeout: 20_000 });
  await browser.$(".pdf-hl").waitForExist({ timeout: 10_000 });
  const geometry = await browser.execute(() => {
    const highlight = document.querySelector(".pdf-hl");
    return highlight ? {
      left: highlight.style.left,
      top: highlight.style.top,
      width: highlight.style.width,
      height: highlight.style.height,
    } : null;
  });
  if (!geometry || Object.values(geometry).some((value) => !value || value.includes("NaN"))) {
    throw new Error(`Logseq highlight geometry was not converted safely: ${JSON.stringify(geometry)}`);
  }
  console.log(`PASS: Logseq PDF sidecar opened with bounded memory and highlight geometry ${JSON.stringify(geometry)}`);
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
