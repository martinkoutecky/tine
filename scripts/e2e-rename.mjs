// Drives the REAL Tauri app (real backend + frontend) under Xvfb via tauri-driver
// to reproduce the rename bug. Seeds a fresh graph, opens Pokus2 (its Linked
// References auto-load Tine via LiveRefGroup), renames the title Pokus2->Pokus,
// then dumps the on-disk files. Usage: node scripts/e2e-rename.mjs
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";

const G = "/tmp/tgraph";
const APP = process.env.TINE_APP || `${process.env.HOME}/research/tine`;
const TD =
  process.env.TAURI_DRIVER ||
  (process.env.CARGO_HOME ? `${process.env.CARGO_HOME}/bin/tauri-driver` : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4444);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4445);

function seed() {
  fs.rmSync(G, { recursive: true, force: true });
  fs.mkdirSync(`${G}/pages`, { recursive: true });
  fs.mkdirSync(`${G}/journals`, { recursive: true });
  fs.mkdirSync(`${G}/logseq`, { recursive: true });
  fs.writeFileSync(`${G}/pages/Pokus2.md`, "- Tohle je pokus\n");
  // Mirrors the real Tine.md: a ```calc fenced block on a bullet line, then the
  // ref in a LATER bullet (the case that used to be mis-read as "inside code").
  fs.writeFileSync(
    `${G}/pages/Tine.md`,
    "- ## Tests\n\t- ```calc\n\t  1 + 2\n\t  var = 2+4\n\t  ```\n\t- #+BEGIN_TIP\n\t  a tip\n\t  #+END_TIP\n\t- [[Pokus2]]\n"
  );
  fs.writeFileSync(`${G}/pages/Testtest2.md`, "- This is a test test page\n- [[Pokus2]]\n");
  fs.writeFileSync(`${G}/journals/2026_06_24.md`, "- journal ref [[Pokus2]]\n");
}
const dump = (tag) => {
  console.log(`\n===== ${tag} =====`);
  for (const f of ["pages/Pokus.md", "pages/Pokus2.md", "pages/Tine.md", "pages/Testtest2.md", "journals/2026_06_24.md"]) {
    const p = `${G}/${f}`;
    console.log(`-- ${f}:`, fs.existsSync(p) ? JSON.stringify(fs.readFileSync(p, "utf8")) : "(absent)");
  }
};

seed();
dump("BEFORE");

// Isolate the app's session/config (it persists the active tab globally, which
// would leak across runs) into a fresh dir each run.
fs.rmSync("/tmp/txdg", { recursive: true, force: true });
for (const d of ["data", "config", "cache"]) fs.mkdirSync(`/tmp/txdg/${d}`, { recursive: true });
const env = {
  ...process.env, // DISPLAY inherited
  TINE_GRAPH: G,
  XDG_DATA_HOME: "/tmp/txdg/data",
  XDG_CONFIG_HOME: "/tmp/txdg/config",
  XDG_CACHE_HOME: "/tmp/txdg/cache",
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  GDK_BACKEND: "x11",
};
console.log("DISPLAY=", process.env.DISPLAY);

const tdLog = fs.openSync("/tmp/td.log", "w");
const td = spawn(
  TD,
  ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],
  { env, stdio: ["ignore", tdLog, tdLog], detached: true }
);
await sleep(3000);

let browser;
try {
  browser = await remote({
    hostname: "127.0.0.1",
    port: DRIVER_PORT,
    path: "/",
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
    logLevel: "error",
    connectionRetryCount: 1,
    connectionRetryTimeout: 60000,
  });

  // Wait for first paint (journals feed).
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20000 });
  await sleep(1200);

  // Open Pokus2 by clicking its [[Pokus2]] link in the journal feed.
  let opened = false;
  for (const sel of ["span.page-ref=Pokus2", "a.page-ref=Pokus2", ".page-ref=Pokus2", "a=Pokus2", "span=Pokus2", "*=Pokus2"]) {
    const el = await browser.$(sel);
    if (await el.isExisting()) {
      await el.click();
      opened = true;
      console.log("clicked link via:", sel);
      break;
    }
  }
  if (!opened) throw new Error("no [[Pokus2]] link found in feed");
  await sleep(1800);

  const title = await browser.$(".page-title");
  const initialTitle = (await title.isExisting()) ? await title.getText() : "(none)";
  console.log("PAGE TITLE:", initialTitle);
  if (initialTitle !== "Pokus2") throw new Error(`expected Pokus2 before rename, got ${initialTitle}`);
  // Let Linked References (LiveRefGroup -> loads Tine, Testtest2, journal) hydrate.
  await sleep(2500);
  const refPages = await browser.$$(".reference-page");
  const refNames = [];
  for (const r of refPages) refNames.push(await r.getText());
  console.log("LINKED-REF pages before rename:", JSON.stringify(refNames));

  // Replicate "Tine is an open/pinned tab": click through to Tine (active view),
  // then back to Pokus2, before renaming.
  const tineRef = await browser.$(".reference-page=Tine");
  if (await tineRef.isExisting()) {
    await tineRef.click();
    await sleep(1500);
    console.log("opened page:", await (await browser.$(".page-title")).getText());
    const back = await browser.$('button[title="Go back"]');
    if (!(await back.isExisting()) || !(await back.isEnabled())) {
      throw new Error("Tine's Go back button was not available after opening the linked reference");
    }
    await back.click();
    await sleep(1500);
    const backTitle = await (await browser.$(".page-title")).getText();
    console.log("back on:", backTitle);
    if (backTitle !== "Pokus2") throw new Error(`history did not return to Pokus2; got ${backTitle}`);
  } else throw new Error("no Tine linked-reference header to exercise open-page rename state");

  const fillAndEnter = (sel, val) =>
    browser.execute((s, v) => {
      const inp = document.querySelector(s);
      inp.focus();
      inp.value = v;
      inp.dispatchEvent(new Event("input", { bubbles: true }));
      inp.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    }, sel, val);

  if (process.env.TINE_E2E_RENAME === "ctx") {
    // Context-menu rename: right-click the title, click "Rename page…", fill, Enter.
    await browser.execute(() => {
      const t = document.querySelector(".page-title");
      const r = t.getBoundingClientRect();
      t.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: r.left + 6, clientY: r.top + 6, view: window }));
    });
    await sleep(700);
    const clicked = await browser.execute(() => {
      const it = [...document.querySelectorAll(".ctx-item")].find((e) => e.textContent.trim().startsWith("Rename page"));
      if (it) { it.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, view: window })); return true; }
      return false;
    });
    console.log("clicked 'Rename page…' item:", clicked);
    await sleep(500);
    const ri = await browser.$(".ctx-rename-name");
    try { await ri.waitForExist({ timeout: 4000 }); } catch {}
    if (await ri.isExisting()) {
      console.log("ctx rename input appeared");
      await fillAndEnter(".ctx-rename-name", "Pokus");
      console.log("ctx rename: set 'Pokus' + Enter");
    } else {
      throw new Error("context-menu rename input never appeared");
    }
  } else {
    // Double-click title rename (SolidJS delegated dblclick).
    await browser.execute(() => {
      const t = document.querySelector(".page-title");
      if (t) t.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true, view: window }));
    });
    await sleep(500);
    const input = await browser.$(".page-title-input");
    try { await input.waitForExist({ timeout: 4000 }); } catch {}
    if (await input.isExisting()) {
      console.log("rename input appeared");
      await fillAndEnter(".page-title-input", "Pokus");
      console.log("set name 'Pokus' + Enter via execute");
    } else {
      throw new Error("rename input never appeared (dblclick did not reach startRename)");
    }
  }
  await sleep(3000);
  const title2 = await browser.$(".page-title");
  const afterTitle = (await title2.isExisting()) ? await title2.getText() : "(none)";
  console.log("PAGE TITLE after:", afterTitle);
  if (afterTitle !== "Pokus") throw new Error(`expected Pokus after rename, got ${afterTitle}`);
  const pokus = `${G}/pages/Pokus.md`;
  const old = `${G}/pages/Pokus2.md`;
  if (!fs.existsSync(pokus) || fs.existsSync(old)) throw new Error("rename did not move Pokus2.md to Pokus.md");
  if (fs.readFileSync(pokus, "utf8") !== "- Tohle je pokus\n") throw new Error("renamed page content changed");
  for (const file of [`${G}/pages/Tine.md`, `${G}/pages/Testtest2.md`, `${G}/journals/2026_06_24.md`]) {
    const body = fs.readFileSync(file, "utf8");
    if (!body.includes("[[Pokus]]") || body.includes("[[Pokus2]]")) throw new Error(`references not rewritten in ${file}`);
  }
  console.log("PASS: renamed intended page and rewrote every reference");
} catch (e) {
  console.log("E2E ERROR:", String(e).split("\n").slice(0, 4).join(" | "));
  process.exitCode = 1;
} finally {
  dump("AFTER");
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
}
