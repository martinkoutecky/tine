// GH #1004: bound Collapse (one level) on a selected query block folds every result of the
// answer, also the result groups (one per page) that are not drawn yet when the key is
// pressed. Scrolling down afterwards shows no hit with its child open, and no file changes:
// result folds are local to the result view.
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { spawn, execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { editorSelector, scratchProfile } from "./lib/native-editor-bench.mjs";

const root = path.resolve(import.meta.dirname, "..");
const app = process.env.TINE_APP || path.join(root, "target/debug/tine");
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, "artifacts/gh1004-query-fold"));
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
fs.writeFileSync(path.join(graph, "logseq/config.edn"),
  '{:tine/guide-announced? true :shortcuts {:editor/collapse-children "mod+alt+k" :editor/expand-children "mod+alt+j"}}\n');
const N = 30;
const all = Array.from({ length: N }, (_, i) => i + 1);
// One hit per page, so the answer has 30 result groups.
for (const i of all) fs.writeFileSync(path.join(graph, "pages", `Source ${String(i).padStart(2, "0")}.md`), `- Ticket ${i} #Ticket\n  - Child ${i}\n`);
const QUERY_PAGE = "Ticket Query";
fs.writeFileSync(path.join(graph, "pages", `${QUERY_PAGE}.md`), "- Intro\n- {{query (and [[Ticket]])}}\n");
const files = () => Object.fromEntries(["pages", "journals"].flatMap(dir => fs.readdirSync(path.join(graph, dir))
  .map(name => [`${dir}/${name}`, crypto.createHash("sha256").update(fs.readFileSync(path.join(graph, dir, name))).digest("hex")])));

const env = { ...process.env, ...scratchProfile(out), TINE_GRAPH: graph,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4991);
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env, stdio: ["ignore", log, log], detached: true });
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(), workingDiff: true,
  app, sha256: crypto.createHash("sha256").update(fs.readFileSync(app)).digest("hex"),
  journey: "Bound Collapse (one level) on a selected query over 30 pages folds every result, drawn or not; files unchanged" };
const shot = file => { try { execFileSync("import", ["-window", "root", path.join(out, file)], { env, timeout: 15000 }); } catch (e) { console.log(`screenshot ${file} failed: ${e}`); } };
let browser;
let stage = "startup";

const installHelpers = () => browser.execute(() => {
  const own = el => (el.querySelector(":scope > .block-main .block-content-wrapper")?.textContent ?? "").trim();
  window.__q = {
    host: () => document.querySelector(".main-content .query-group")?.closest(".ls-block"),
    intro: () => [...document.querySelectorAll(".main-content .ls-block")].find(el => own(el) === "Intro")?.getAttribute("data-block-id") ?? null,
    scroller() {
      for (let el = window.__q.host()?.parentElement; el; el = el.parentElement) {
        const s = getComputedStyle(el);
        if (/(auto|scroll)/.test(s.overflowY) && el.scrollHeight > el.clientHeight) return el;
      }
      return document.scrollingElement;
    },
    // Each drawn hit: whether its child row is visible.
    rows() {
      const res = {};
      for (const el of document.querySelectorAll(".main-content .query-group .ls-block")) {
        const m = /^Ticket (\d+)\b/.exec(own(el));
        if (!m) continue;
        const child = [...el.querySelectorAll(".ls-block")].find(c => own(c) === `Child ${m[1]}`);
        res[m[1]] = { childVisible: !!child && !!child.offsetParent && child.getBoundingClientRect().height > 0 };
      }
      return res;
    },
    groups() {
      const gs = [...document.querySelectorAll(".main-content .query-group")];
      return { total: gs.length, drawn: gs.filter(g => g.getAttribute("data-find-source-mounted") === "true").length };
    },
    // Every group within the near margin of the viewport is drawn and its hit rows rendered.
    nearDrawn() {
      const view = window.__q.scroller().getBoundingClientRect();
      return [...document.querySelectorAll(".main-content .query-group")].every(g => {
        const r = g.getBoundingClientRect();
        if (r.bottom < view.top - 1200 || r.top > view.bottom + 1200) return true;
        return g.getAttribute("data-find-source-mounted") === "true" && !!g.querySelector(".ls-block");
      });
    },
    selected: () => !!window.__q.host()?.querySelector(":scope > .block-main.selected"),
    collapsed: () => !!window.__q.host()?.classList.contains("collapsed"),
    scroll(dy) { const s = window.__q.scroller(); s.scrollTop += dy; return { top: s.scrollTop, max: s.scrollHeight - s.clientHeight, h: s.clientHeight }; },
  };
});
const q = (name, ...args) => browser.execute((name, args) => window.__q[name](...args), name, args);
const waitNear = () => browser.waitUntil(() => q("nearDrawn"), { timeout: 20000, timeoutMsg: "groups near the viewport were not drawn" });

try {
  await sleep(1500);
  for (let attempt = 0; ; attempt++) {
    try {
      browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0,
        connectionRetryTimeout: 60000, capabilities: tauriCapabilities(app, "gh1004-query-fold") });
      break;
    } catch (error) {
      if (!String(error).includes("Maximum number of active sessions") || attempt >= 11) throw error;
      console.log("Another native journey owns WebKit; waiting 10 seconds before retry.");
      await sleep(10000);
    }
  }
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 60000 });
  // A small window keeps more of the 30 groups undrawn at the press.
  try { await browser.setWindowSize(1024, 600); } catch (e) { console.log(`setWindowSize: ${e}`); }
  stage = "open query page";
  await openPageByName(browser, QUERY_PAGE);
  await browser.waitUntil(() => browser.execute(() => !!document.querySelector(".main-content .query-group .ls-block")), { timeout: 30000, timeoutMsg: "no result rows rendered" });
  await installHelpers();
  await waitNear();
  const before = files();

  stage = "select the query block";
  const intro = await q("intro");
  if (!intro) throw Error("Intro block not found");
  await browser.execute(id => {
    const el = document.querySelector(`[data-block-id="${id}"] > .block-main .block-content-wrapper`);
    el.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
    document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
  }, intro);
  await browser.$(`[data-block-id="${intro}"] > .block-main ${editorSelector}`).waitForExist({ timeout: 10000 });
  await browser.keys(["Escape"]);
  await browser.waitUntil(() => browser.execute(id => !!document.querySelector(`[data-block-id="${id}"] > .block-main.selected`), intro), { timeout: 5000, timeoutMsg: "Intro not selected" });
  await browser.keys(["ArrowDown"]);
  await browser.waitUntil(() => q("selected"), { timeout: 5000, timeoutMsg: "query block not selected" });
  proof.atPress = await q("groups");
  if (proof.atPress.drawn >= N) throw Error(`every group was drawn before the press (${JSON.stringify(proof.atPress)}): the journey needs undrawn groups`);

  stage = "collapse";
  await browser.keys(["Control", "Alt", "k", "NULL"]);
  await browser.waitUntil(async () => Object.values(await q("rows")).every(v => !v.childVisible), { timeout: 10000, timeoutMsg: "drawn hits did not fold" });
  if (await q("collapsed")) throw Error("the query block folded itself");

  stage = "scroll through the results";
  const seen = {};
  const take = async () => { for (const [k, v] of Object.entries(await q("rows"))) seen[k] = v; };
  await take();
  for (let guard = 0; guard < 400; guard++) {
    const pos = await q("scroll", 0);
    if (pos.top >= pos.max - 1) break;
    await q("scroll", Math.floor(pos.h * 0.6));
    await waitNear();
    await take();
  }
  proof.afterScroll = { groups: await q("groups"), seen: Object.keys(seen).length, open: all.filter(i => seen[i]?.childVisible) };
  const missing = all.filter(i => !seen[i]);
  if (missing.length) throw Error(`hits never drawn while scrolling: ${missing.join(",")}`);
  if (proof.afterScroll.open.length) throw Error(`hits shown with their child open after Collapse: ${proof.afterScroll.open.join(",")}`);

  stage = "files unchanged";
  const after = files();
  const changed = Object.keys({ ...before, ...after }).filter(k => before[k] !== after[k]);
  if (changed.length) throw Error(`files changed by the fold: ${changed.join(", ")}`);
  shot("collapsed.png");
  proof.outcome = "PASS";
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2));
  console.log(`PASS gh1004-query-fold: ${out}`);
} catch (error) {
  shot("failure.png");
  try { fs.writeFileSync(path.join(out, "failure-dom.html"), await browser.getPageSource()); } catch {}
  fs.writeFileSync(path.join(out, "failure.json"), JSON.stringify({ ...proof, stage, observed: String(error.stack) }, null, 2));
  console.log(`FAIL gh1004-query-fold at ${stage}: ${error.message}`);
  process.exitCode = 1;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
