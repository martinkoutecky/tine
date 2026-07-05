// Repro for the "inline page icon / emoji not rendered" report (#16 follow-up):
// seeds a page whose `icon::` is an emoji, references it inline (table cell, bullet,
// alias, #tag) plus a literal-emoji cell, opens it in the REAL app, and screenshots
// so we can see which of {literal emoji, inline page icon} Tine renders vs drops.
// Usage: node scripts/shot-pageicon.mjs [out.png]
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";

const G = "/tmp/tgraph-pageicon";
const XDG = "/tmp/txdg-pageicon";
const APP = process.env.TINE_APP || `${process.env.HOME}/research/tine`;
const TD =
  process.env.TAURI_DRIVER ||
  (process.env.CARGO_HOME ? `${process.env.CARGO_HOME}/bin/tauri-driver` : "tauri-driver");
const OUT = process.argv[2] || "/tmp/tine-pageicon.png";

function seed() {
  fs.rmSync(G, { recursive: true, force: true });
  fs.mkdirSync(`${G}/pages`, { recursive: true });
  fs.mkdirSync(`${G}/journals`, { recursive: true });
  fs.mkdirSync(`${G}/logseq`, { recursive: true });
  fs.writeFileSync(`${G}/pages/Verstappen.md`, "icon:: 🏁\n- Driver page\n");
  fs.writeFileSync(`${G}/pages/Russell.md`, "icon:: 🏁\n- Driver page\n");
  fs.writeFileSync(
    `${G}/pages/Laps.md`,
    [
      "- # Inline page-icon repro",
      "- Bullet ref: [[Verstappen]] and [[Russell]]",
      "- Alias ref: [Max]([[Verstappen]])",
      "- Tag ref: #Verstappen",
      "- Literal emoji before a ref: 🏁 [[Verstappen]]",
      "- A table:",
      "- | Lap | Driver |",
      "  | --- | --- |",
      "  | 1:05 | [[Verstappen]] |",
      "  | 1:06 | [[Russell]] |",
      "",
    ].join("\n"),
  );
  fs.writeFileSync(`${G}/journals/2026_07_05.md`, "- open [[Laps]]\n");

  fs.rmSync(XDG, { recursive: true, force: true });
  for (const d of ["data", "config", "cache"]) fs.mkdirSync(`${XDG}/${d}`, { recursive: true });
}

seed();

const xvfb = spawn("Xvfb", [":96", "-screen", "0", "1200x1000x24"], { stdio: "ignore" });
await sleep(1500);

const env = {
  ...process.env,
  DISPLAY: ":96",
  TINE_GRAPH: G,
  XDG_DATA_HOME: `${XDG}/data`,
  XDG_CONFIG_HOME: `${XDG}/config`,
  XDG_CACHE_HOME: `${XDG}/cache`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  GDK_BACKEND: "x11",
};

const tdLog = fs.openSync("/tmp/td-pageicon.log", "w");
const td = spawn(TD, ["--port", "4452", "--native-port", "4453", "--native-driver", "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", tdLog, tdLog] });
await sleep(3000);

let browser;
try {
  browser = await remote({
    hostname: "127.0.0.1",
    port: 4452,
    path: "/",
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
    logLevel: "error",
    connectionRetryCount: 1,
    connectionRetryTimeout: 60000,
  });

  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20000 });
  await sleep(1200);
  for (const sel of [`a.page-ref=Laps`, `span.page-ref=Laps`, `.page-ref=Laps`, `*=Laps`]) {
    const el = await browser.$(sel);
    if (await el.isExisting()) { await el.click(); break; }
  }
  await sleep(3500);
  const info = await browser.execute(() => {
    const refs = Array.from(document.querySelectorAll("a.page-ref, a.tag")).slice(0, 8);
    return refs.map((a) => ({
      text: (a.textContent || "").trim().slice(0, 20),
      hasIconImg: !!a.querySelector("img,.page-icon"),
      html: a.innerHTML.slice(0, 80),
    }));
  });
  console.log("INLINE REFS:", JSON.stringify(info, null, 1));
  await browser.saveScreenshot(OUT);
  console.log("screenshot →", OUT);
} catch (e) {
  console.error("ERROR:", e.message);
} finally {
  try { await browser?.deleteSession(); } catch {}
  td.kill("SIGKILL");
  xvfb.kill("SIGKILL");
}
