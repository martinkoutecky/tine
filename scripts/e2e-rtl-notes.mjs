// GH #823, Linux real WebKit: a Persian task reads right to left in the view and the editor, arrows keep their
// on-screen meaning at its edges, the note layout setting mirrors the outline, and the note's bytes never change.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4494);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4495);
const TMP = "/tmp/tine-rtl-notes-e2e";
const GRAPH = `${TMP}/graph`;
const PAGE = `${GRAPH}/pages/Rtl.md`;
const SOURCE = ["- before", "- TODO یادداشت فارسی", "  - فرزند", "- after", ""].join("\n");

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{:tine/guide-announced? true}\n");
fs.writeFileSync(PAGE, SOURCE);
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- open [[Rtl]]\n");
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
const env = {
  ...process.env, TINE_GRAPH: GRAPH, HOME: `${TMP}/xdg`,
  XDG_DATA_HOME: `${TMP}/xdg/data`, XDG_CONFIG_HOME: `${TMP}/xdg/config`, XDG_CACHE_HOME: `${TMP}/xdg/cache`,
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

  // Into the editor: same direction. Past the block's start is on the right.
  await browser.$(`[data-block-id="${view.id}"] > .block-main .block-content`).click();
  await browser.$(`[data-block-id="${view.id}"] > .block-main textarea, [data-block-id="${view.id}"] > .block-main .cm-content`).waitForExist({ timeout: 5_000 });
  const edit = await facts();
  if (edit.direction !== "rtl") throw new Error(`the editor is ${edit.direction}, the view was rtl`);
  await browser.keys(["Home"]);
  for (let i = 0; i < 40 && (await facts()).active === view.id; i++) await browser.keys(["ArrowRight"]);
  const before = view.ids[view.ids.indexOf(view.id) - 1];
  const after = await facts();
  if (after.active !== before) throw new Error(`ArrowRight toward the start left for ${after.active}, want the previous block`);
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
  console.log("PASS: right-to-left block direction, arrows, note layout, unchanged bytes");
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
