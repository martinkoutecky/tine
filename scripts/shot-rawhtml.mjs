// Real-app render verification for the raw-HTML sanitizer (ADR 0019): seeds a page
// with allowlisted raw HTML + a hostile injection, opens it in the REAL Tauri app
// (real backend → real lsdoc ASTs → renderRawHtml/DOMPurify) under Xvfb, and
// screenshots so the LIVE render can be eyeballed in actual WebKitGTK.
// Usage: node scripts/shot-rawhtml.mjs [out.png]
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";

const G = "/tmp/tgraph-rawhtml";
const APP = process.env.TINE_APP || `${process.env.HOME}/research/tine`;
const TD =
  process.env.TAURI_DRIVER ||
  (process.env.CARGO_HOME ? `${process.env.CARGO_HOME}/bin/tauri-driver` : "tauri-driver");
const OUT = process.argv[2] || "/tmp/tine-rawhtml.png";

const PAGE = [
  "- # Raw HTML (sanitized) — ADR 0019",
  "- Formatting tags at a word boundary render: <ins>inserted</ins>, <del>deleted</del>, <kbd>Ctrl</kbd> + <kbd>C</kbd>, <mark>marked</mark>, and <sup>superscript</sup> / <sub>subscript</sub>.",
  "- A safe block container: <div class=\"note\">a raw &lt;div&gt; renders</div>",
  "- Parity: a glued tag like H<sub>2</sub>O stays literal (mldoc = Plain), same as Logseq.",
  "- Hostile input is neutralized: <b onmouseover=\"steal()\">bold survives, handler stripped</b> and a <script>alert(1)</script> vanishes.",
  "",
].join("\n");

function seed() {
  fs.rmSync(G, { recursive: true, force: true });
  fs.mkdirSync(`${G}/pages`, { recursive: true });
  fs.mkdirSync(`${G}/journals`, { recursive: true });
  fs.mkdirSync(`${G}/logseq`, { recursive: true });
  fs.writeFileSync(`${G}/pages/RawHtml.md`, PAGE);
  fs.writeFileSync(`${G}/journals/2026_07_05.md`, "- open [[RawHtml]]\n");
}

seed();
fs.rmSync("/tmp/txdg-rawhtml", { recursive: true, force: true });
for (const d of ["data", "config", "cache"]) fs.mkdirSync(`/tmp/txdg-rawhtml/${d}`, { recursive: true });

const xvfb = spawn("Xvfb", [":98", "-screen", "0", "1400x1200x24"], { stdio: "ignore" });
await sleep(1500);

const env = {
  ...process.env,
  DISPLAY: ":98",
  TINE_GRAPH: G,
  XDG_DATA_HOME: "/tmp/txdg-rawhtml/data",
  XDG_CONFIG_HOME: "/tmp/txdg-rawhtml/config",
  XDG_CACHE_HOME: "/tmp/txdg-rawhtml/cache",
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  GDK_BACKEND: "x11",
};

const tdLog = fs.openSync("/tmp/td-rawhtml.log", "w");
const td = spawn(TD, ["--port", "4448", "--native-port", "4449", "--native-driver", "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", tdLog, tdLog] });
await sleep(3000);

let browser;
try {
  browser = await remote({
    hostname: "127.0.0.1",
    port: 4448,
    path: "/",
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
    logLevel: "error",
    connectionRetryCount: 1,
    connectionRetryTimeout: 60000,
  });

  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20000 });
  await sleep(1200);
  for (const sel of [`a.page-ref=RawHtml`, `span.page-ref=RawHtml`, `.page-ref=RawHtml`, `*=RawHtml`]) {
    const el = await browser.$(sel);
    if (await el.isExisting()) { await el.click(); break; }
  }
  await sleep(4000);
  // Assert in the live DOM: allowlisted tags exist, handlers/scripts do NOT.
  const checks = await browser.execute(() => {
    const root = document.querySelector(".page") || document.body;
    const html = root.innerHTML;
    return {
      ins: !!root.querySelector("ins"),
      kbd: !!root.querySelector("kbd"),
      sup: !!root.querySelector("sup"),
      sub: !!root.querySelector("sub"),
      div: !!root.querySelector(".raw-html div.note, div.note"),
      scriptTag: !!root.querySelector("span.raw-html script, .ls-block script"),
      hasOnmouseover: /onmouseover/i.test(html),
      hasAlert1: /alert\(1\)/.test(html),
    };
  });
  console.log("LIVE DOM CHECKS:", JSON.stringify(checks, null, 0));
  await browser.saveScreenshot(OUT);
  console.log("screenshot →", OUT);
} catch (e) {
  console.error("ERROR:", e.message);
} finally {
  try { await browser?.deleteSession(); } catch {}
  td.kill("SIGKILL");
  xvfb.kill("SIGKILL");
}
