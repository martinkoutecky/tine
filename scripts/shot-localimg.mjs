// Verifies the local-file <img> opt-in (ADR 0019) end-to-end in the REAL app:
// creates a real PNG OUTSIDE the graph, seeds a page with a self-closed
// <img src="file://…"/> (raw_html), enables the opt-in via tine-settings.json, and
// asserts the sanitized <img> gets a blob: src (i.e. the gated read_local_image
// read the file). Also asserts it does NOT load when the opt-in is off.
// Usage: node scripts/shot-localimg.mjs [out.png]
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";

const G = "/tmp/tgraph-localimg";
const XDG = "/tmp/txdg-localimg";
const IMG = "/tmp/tine-localimg-src/cat.png";
const APP = process.env.TINE_APP || `${process.env.HOME}/research/tine`;
const TD =
  process.env.TAURI_DRIVER ||
  (process.env.CARGO_HOME ? `${process.env.CARGO_HOME}/bin/tauri-driver` : "tauri-driver");
const OUT = process.argv[2] || "/tmp/tine-localimg.png";
const OPT_IN = process.env.OPT_IN !== "0"; // default: opt-in ON

// A real 1x1 red PNG (base64).
const PNG_B64 =
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

function seed() {
  fs.rmSync(G, { recursive: true, force: true });
  fs.mkdirSync(`${G}/pages`, { recursive: true });
  fs.mkdirSync(`${G}/journals`, { recursive: true });
  fs.mkdirSync(`${G}/logseq`, { recursive: true });
  fs.mkdirSync("/tmp/tine-localimg-src", { recursive: true });
  fs.writeFileSync(IMG, Buffer.from(PNG_B64, "base64"));
  fs.writeFileSync(
    `${G}/pages/LocalImg.md`,
    [
      "- # Local-file <img> opt-in",
      `- A local image (self-closed, raw HTML): <img src="file://${IMG}" alt="cat"/>`,
      "",
    ].join("\n"),
  );
  fs.writeFileSync(`${G}/journals/2026_07_05.md`, "- open [[LocalImg]]\n");

  fs.rmSync(XDG, { recursive: true, force: true });
  for (const d of ["data", "config", "cache"]) fs.mkdirSync(`${XDG}/${d}`, { recursive: true });
  // Pre-enable (or not) the opt-in the same way the Settings toggle persists it.
  const appData = `${XDG}/data/dev.tine.app`;
  fs.mkdirSync(appData, { recursive: true });
  fs.writeFileSync(`${appData}/tine-settings.json`, JSON.stringify({ allow_local_file_images: OPT_IN }));
}

seed();

const xvfb = spawn("Xvfb", [":97", "-screen", "0", "1400x1000x24"], { stdio: "ignore" });
await sleep(1500);

const env = {
  ...process.env,
  DISPLAY: ":97",
  TINE_GRAPH: G,
  XDG_DATA_HOME: `${XDG}/data`,
  XDG_CONFIG_HOME: `${XDG}/config`,
  XDG_CACHE_HOME: `${XDG}/cache`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  GDK_BACKEND: "x11",
};

const tdLog = fs.openSync("/tmp/td-localimg.log", "w");
const td = spawn(TD, ["--port", "4450", "--native-port", "4451", "--native-driver", "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", tdLog, tdLog] });
await sleep(3000);

let browser;
try {
  browser = await remote({
    hostname: "127.0.0.1",
    port: 4450,
    path: "/",
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
    logLevel: "error",
    connectionRetryCount: 1,
    connectionRetryTimeout: 60000,
  });

  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20000 });
  await sleep(1200);
  for (const sel of [`a.page-ref=LocalImg`, `span.page-ref=LocalImg`, `.page-ref=LocalImg`, `*=LocalImg`]) {
    const el = await browser.$(sel);
    if (await el.isExisting()) { await el.click(); break; }
  }
  await sleep(4000); // parse + async blob load
  const res = await browser.execute(() => {
    const img = document.querySelector(".page span.raw-html img, .ls-block span.raw-html img");
    return { found: !!img, src: img ? img.getAttribute("src") || "" : "" };
  });
  const isBlob = res.src.startsWith("blob:");
  console.log("OPT_IN:", OPT_IN, "| img found:", res.found, "| src is blob:", isBlob, "| src:", res.src.slice(0, 24));
  console.log(OPT_IN ? (isBlob ? "PASS: local image loaded" : "FAIL: expected blob src") : (isBlob ? "FAIL: loaded while OFF" : "PASS: correctly not loaded"));
  await browser.saveScreenshot(OUT);
  console.log("screenshot →", OUT);
} catch (e) {
  console.error("ERROR:", e.message);
} finally {
  try { await browser?.deleteSession(); } catch {}
  td.kill("SIGKILL");
  xvfb.kill("SIGKILL");
}
