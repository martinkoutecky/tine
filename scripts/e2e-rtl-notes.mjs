// GH #823, Linux real WebKit: a Persian task reads right to left in the view and the editor, arrows keep their
// on-screen meaning at its edges, the note layout setting mirrors the outline, and the note's bytes never change.
import { spawn, execFileSync } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { installAccountHome, assertAppLease } from "./lib/e2e-account-home.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4494);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4495);
const TMP = "/tmp/tine-rtl-notes-e2e";
const GRAPH = `${TMP}/graph`;
const PAGE = `${GRAPH}/pages/Rtl.md`;
const SOURCE = ["- before", "- TODO [#A] یادداشت فارسی", "  - فرزند", "- after", ""].join("\n");

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{:tine/guide-announced? true}\n");
fs.writeFileSync(PAGE, SOURCE);
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- open [[Rtl]]\n");
for (const dir of ["data", "config", "cache", "state", "tmp"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
const owner = installAccountHome(`${TMP}/owner`, path.resolve("artifacts/og-rtl-823/fix-native/owner"));
const env = {
  ...process.env, TINE_GRAPH: GRAPH, HOME: `${TMP}/xdg`,
  XDG_DATA_HOME: `${TMP}/xdg/data`, XDG_CONFIG_HOME: `${TMP}/xdg/config`, XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  XDG_STATE_HOME: `${TMP}/xdg/state`, TMPDIR: `${TMP}/xdg/tmp`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
};
const log = fs.openSync(`${TMP}/tauri-driver.log`, "w");
const td = spawn(TD, ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
await sleep(2500);

let browser;
try {
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER_PORT, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
  });
  await browser.$(".page-ref").waitForExist({ timeout: 20_000 });
  await assertAppLease(owner);
  await (await browser.$("span.page-ref=Rtl, a.page-ref=Rtl").isExisting() ? browser.$("span.page-ref=Rtl, a.page-ref=Rtl") : browser.$("*=Rtl")).click();
  await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === "Rtl", { timeout: 10_000 });
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".ls-block")].some(r => r.textContent.includes("یاد"))), { timeout: 15_000 });
  let target = null; // an editor's text is its value, not textContent: once found, the row is known by id
  const facts = () => browser.execute((known) => {
    const rows = [...document.querySelectorAll(".ls-block")];
    const row = known ? rows.find(r => r.dataset.blockId === known) : rows.find(r => (r.querySelector(":scope > .block-main")?.textContent ?? "").includes("یاد"));
    const main = row.querySelector(":scope > .block-main");
    const body = main.querySelector(".block-content, textarea, .cm-content");
    const bullet = main.querySelector(".bullet-container").getBoundingClientRect(), box = body.getBoundingClientRect();
    return { tag: body.tagName, direction: getComputedStyle(body).direction, bulletRight: bullet.left >= box.right - 1,
      id: row.dataset.blockId, active: document.activeElement?.closest("[data-block-id]")?.dataset.blockId ?? null,
      ids: rows.map(r => r.dataset.blockId), rect: { x: box.left + box.width / 2, y: box.top + box.height / 2 } };
  }, target);
  const view = await facts();
  target = view.id;
  if (view.direction !== "rtl") throw new Error(`a Persian task rendered ${view.direction}`);
  if (view.bulletRight) throw new Error("the default layout put the bullet on the right");

  // Into the editor: same direction. The Latin marker at source offset zero
  // still has text on the right; the arrow must not cross blocks there.
  await browser.$(`[data-block-id="${view.id}"] > .block-main .block-content`).click();
  await browser.$(`[data-block-id="${view.id}"] > .block-main textarea, [data-block-id="${view.id}"] > .block-main .cm-content`).waitForExist({ timeout: 5_000 });
  const edit = await facts();
  if (edit.direction !== "rtl") throw new Error(`the editor is ${edit.direction}, the view was rtl`);
  await browser.execute(() => {
    const ta = document.querySelector("textarea");
    if (!ta) throw new Error("this journey exercises the default textarea editor");
    ta.focus(); ta.setSelectionRange(0, 0);
  });
  await browser.keys(["ArrowRight"]);
  if ((await facts()).active !== view.id) throw new Error("ArrowRight left a mixed task while text remained on that side");
  // Textareas expose no bidi affinity. Up remains a way out of a mixed marker
  // boundary where the conservative horizontal rule defers to native movement.
  await browser.execute(() => { const ta = document.querySelector("textarea"); ta.setSelectionRange(0, 0); });
  await browser.keys(["ArrowUp"]);
  const before = view.ids[view.ids.indexOf(view.id) - 1];
  const after = await facts();
  if (after.active !== before) throw new Error(`ArrowUp from the first visual row left for ${after.active}, want the previous block`);
  await browser.keys(["Escape"]);

  // The setting mirrors the outline.
  await browser.execute(() => document.activeElement?.blur?.());
  await browser.$('button.icon-btn[title^="Settings"]').click();
  await browser.$(".settings-nav-item").waitForExist({ timeout: 10_000 });
  await browser.execute(() => { const open = [...document.querySelectorAll("button, summary")].find(e => /^\W*Advanced/.test(e.textContent.trim())); open?.click(); });
  await browser.waitUntil(() => browser.execute(() => document.querySelector(".settings-advanced-body")?.textContent.includes("Right-to-left note layout") ?? false), { timeout: 10_000, timeoutMsg: "the note layout setting is not under Appearance, Advanced" });
  const toggled = await browser.execute(() => {
    const field = [...document.querySelectorAll("*")].filter(e => e.children.length === 0 && e.textContent.trim() === "Right-to-left note layout")[0];
    let el = field; while (el && !el.querySelector("button, [role=switch]")) el = el.parentElement;
    const toggle = el?.querySelector("button, [role=switch]"); toggle?.click(); return !!toggle;
  });
  if (!toggled) throw new Error("the note layout toggle was not found");
  await browser.keys(["Escape"]);
  await browser.waitUntil(async () => (await facts()).bulletRight, { timeout: 10_000, timeoutMsg: "the note layout setting did not put bullets on the right" });
  await sleep(1500);
  if (fs.readFileSync(PAGE, "utf8") !== SOURCE) throw new Error("reading and mirroring changed the note's bytes");
  // Switch the real interface to Persian, using its persistent language picker.
  // Then exercise a real submenu and measure the physical native grip classes.
  await browser.keys(["t"]); await browser.keys(["s"]);
  const language = browser.$('[data-setting-label="Language"] select');
  await language.waitForExist({ timeout: 10_000 });
  await language.selectByAttribute("value", "fa");
  await browser.waitUntil(() => browser.execute(() => document.documentElement.lang === "fa" && document.documentElement.dir === "rtl"), { timeout: 10_000 });
  await browser.keys(["Escape"]);
  await browser.deleteSession();
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER_PORT, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
  });
  await browser.$(".ls-block").waitForExist({ timeout: 20_000 });
  await browser.waitUntil(() => browser.execute(() => document.documentElement.lang === "fa" && document.documentElement.dir === "rtl"), { timeout: 10_000 });
  const grip = await browser.execute(() => {
    const e = document.querySelector(".grip-e"), w = document.querySelector(".grip-w");
    if (!e || !w) throw new Error("native resize grips are missing");
    return { east: e.getBoundingClientRect().right, west: w.getBoundingClientRect().left, width: innerWidth };
  });
  if (Math.abs(grip.east - grip.width) > 1 || Math.abs(grip.west) > 1) throw new Error(`Persian resize grips name the wrong screen edges: ${JSON.stringify(grip)}`);
  await browser.$("h1.page-title").click({ button: "right" });
  await browser.$(".ctx-submenu").waitForExist({ timeout: 5000 });
  await browser.$(".ctx-submenu").moveTo();
  const submenu = await browser.execute(() => {
    const r = document.querySelector(".ctx-submenu-menu").getBoundingClientRect();
    return { left: r.left, right: r.right, width: innerWidth };
  });
  if (submenu.left < 0 || submenu.right > submenu.width) throw new Error(`Persian submenu is off screen: ${JSON.stringify(submenu)}`);
  const evidence = path.resolve("artifacts/og-rtl-823/fix-native"); fs.mkdirSync(evidence, { recursive: true });
  await browser.saveScreenshot(`${evidence}/persian-submenu.png`);
  fs.writeFileSync(`${evidence}/geometry.json`, JSON.stringify({ grip, submenu }, null, 2));
  console.log("PASS: mixed RTL task arrows, vertical exit, note layout, unchanged bytes, Persian interface submenu and resize grips");
} catch (error) {
  const evidence = path.resolve("artifacts/og-rtl-823/fix-native"); fs.mkdirSync(evidence, { recursive: true });
  try { await browser?.saveScreenshot(`${evidence}/failure.png`); } catch {}
  fs.writeFileSync(`${evidence}/failure.json`, JSON.stringify({
    commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(), dirty: true,
    journey: "scripts/e2e-rtl-notes.mjs", expected: "Mixed-task movement, vertical exit, mirrored layout, unchanged bytes, Persian relaunch with usable submenu and physical grips",
    observed: String(error), classification: "ambiguous; inspect screenshot and driver log",
  }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
