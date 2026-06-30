// Real-app verification for the post-M1/v0.2.0 audit fixes C1 (planning body-text
// loss) + C3 (priority header-only). Seeds a page with standalone/mid-text planning
// timestamps and priority edge cases, opens it in the REAL Tauri app under Xvfb, and
// screenshots the rendered page. Usage: node scripts/shot-planning-audit.mjs [out.png]
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";

const G = "/tmp/tgraph-planaudit";
const APP = process.env.TINE_APP || `${process.env.HOME}/research/tine`;
const TD =
  process.env.TAURI_DRIVER ||
  (process.env.CARGO_HOME ? `${process.env.CARGO_HOME}/bin/tauri-driver` : "tauri-driver");
const OUT = process.argv[2] || "/tmp/tine-planning-audit.png";

const PAGE = [
  "- # Planning + priority audit",
  "- TODO ship it", // C1: standalone SCHEDULED below → badge, body = "ship it"
  "  SCHEDULED: <2026-07-06 Mon>",
  "- DOING write the tests",
  "  DEADLINE: <2026-07-10 Fri>",
  "- a paragraph with SCHEDULED: <2026-07-06 Mon> inline", // C1: must NOT be blank
  "- just text", // C1: trailing line after SCHEDULED must survive
  "  SCHEDULED: <2026-07-06 Mon>",
  "  more text after",
  "- TODO [#A] real priority task", // C3: SHOWS an A priority chip
  "- Discuss [#A] tags inline", // C3: NO chip — [#A] is literal text mid-line
  "",
].join("\n");

fs.rmSync(G, { recursive: true, force: true });
fs.mkdirSync(`${G}/pages`, { recursive: true });
fs.mkdirSync(`${G}/journals`, { recursive: true });
fs.mkdirSync(`${G}/logseq`, { recursive: true });
fs.writeFileSync(`${G}/pages/Planaudit.md`, PAGE);
fs.writeFileSync(`${G}/journals/2026_06_30.md`, "- open [[Planaudit]]\n");

fs.rmSync("/tmp/txdg-planaudit", { recursive: true, force: true });
for (const d of ["data", "config", "cache"]) fs.mkdirSync(`/tmp/txdg-planaudit/${d}`, { recursive: true });

const xvfb = spawn("Xvfb", [":98", "-screen", "0", "1400x1400x24"], { stdio: "ignore" });
await sleep(1500);

const env = {
  ...process.env,
  DISPLAY: ":98",
  TINE_GRAPH: G,
  XDG_DATA_HOME: "/tmp/txdg-planaudit/data",
  XDG_CONFIG_HOME: "/tmp/txdg-planaudit/config",
  XDG_CACHE_HOME: "/tmp/txdg-planaudit/cache",
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  GDK_BACKEND: "x11",
};

const tdLog = fs.openSync("/tmp/td-planaudit.log", "w");
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
  for (const sel of [`a.page-ref=Planaudit`, `span.page-ref=Planaudit`, `.page-ref=Planaudit`, `*=Planaudit`]) {
    const el = await browser.$(sel);
    if (await el.isExisting()) { await el.click(); break; }
  }
  await sleep(4000);
  const title = await browser.$(".page-title");
  console.log("PAGE TITLE:", (await title.isExisting()) ? await title.getText() : "(none)");
  // Black-box assertions on the rendered DOM text — the body text that C1 used to eat.
  const body = await browser.execute(() => document.querySelector(".page, #app")?.innerText || "");
  const checks = {
    "C1 mid-text kept ('inline')": body.includes("a paragraph with") && body.includes("inline"),
    "C1 trailing kept ('more text after')": body.includes("more text after"),
    "standalone task body ('ship it')": body.includes("ship it"),
    "C3 literal [#A] text kept": body.includes("Discuss") && body.includes("tags inline"),
  };
  console.log("CHECKS:", JSON.stringify(checks, null, 2));
  await browser.saveScreenshot(OUT);
  console.log("screenshot →", OUT);
} catch (e) {
  console.error("ERROR:", e.message);
} finally {
  try { await browser?.deleteSession(); } catch {}
  td.kill("SIGKILL");
  xvfb.kill("SIGKILL");
}
