// Linux native proof: real Rust answers, ordinary source saves and restart.
import { spawn, execFileSync } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";

const out = path.resolve(process.env.OUT ?? "artifacts/queries/native");
const testedCommit = execFileSync("rtk", ["proxy", "git", "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
const graph = `${out}/graph`;
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${graph}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${out}/xdg/${dir}`, { recursive: true });
const date = (offset) => {
  const day = new Date(); day.setDate(day.getDate() + offset);
  return { iso: `${day.getFullYear()}-${String(day.getMonth() + 1).padStart(2, "0")}-${String(day.getDate()).padStart(2, "0")}`, weekday: day.toLocaleDateString("en-US", { weekday: "short" }) };
};
const yesterday = date(-1), tomorrow = date(1), today = date(0);
fs.writeFileSync(`${graph}/logseq/config.edn`, "{}\n");
fs.writeFileSync(`${graph}/journals/${today.iso.replaceAll("-", "_")}.md`, "- Open [[Queries]] and [[Tasks]]\n");
fs.writeFileSync(`${graph}/pages/Tasks.md`, [
  "- TODO [#A] First journey task", "  owner:: Martin", "  - Child detail kept in copy",
  "- TODO Second journey task", "  owner:: Ada",
  "- DONE Completed journey task", "",
].join("\n"));
fs.writeFileSync(`${graph}/pages/Planning.md`, [
  "- Overdue journey task", `  SCHEDULED: <${yesterday.iso} ${yesterday.weekday}>`,
  "- Upcoming journey task", `  SCHEDULED: <${tomorrow.iso} ${tomorrow.weekday}>`, "",
].join("\n"));
fs.writeFileSync(`${graph}/pages/Links.md`, "- [[Tasks]] [[Tasks]]\n");
const queries = `${graph}/pages/Queries.md`;
fs.writeFileSync(queries, [
  '- {{query (task TODO) {:title "Open tasks"}}}',
  '- {{query (task TODO DONE) {:title "Task table"}}}', "  tine.view:: table",
  '- {{query (task TODO DONE) {:title "Task board"}}}', "  tine.view:: board", "  tine.group-field:: state",
  '- {{tine-query @block and scheduled < today {:title "Planning query"}}}', "",
].join("\n"));
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: `${out}/xdg/data`, XDG_CONFIG_HOME: `${out}/xdg/config`, XDG_CACHE_HOME: `${out}/xdg/cache`, WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const driver = process.env.TAURI_DRIVER ?? `${process.env.CARGO_HOME}/bin/tauri-driver`;
const app = path.resolve(process.env.TINE_APP ?? "target/release/tine");
const driverPort = Number(process.env.E2E_DRIVER_PORT ?? 4556);
const nativePort = Number(process.env.E2E_NATIVE_PORT ?? 4557);
let phase = "startup";
async function session(run, journey) {
  const log = fs.openSync(`${out}/driver-${run}.log`, "w");
  const processDriver = spawn(driver, ["--port", String(driverPort), "--native-port", String(nativePort), "--native-driver", process.env.WEBKIT_DRIVER ?? "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
  let browser;
  try {
    await sleep(1500);
    browser = await remote({ hostname: "127.0.0.1", port: driverPort, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
      capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: app } } });
    await ensureMainWindow(browser);
    await browser.$(".ls-block, .page-title").waitForExist({ timeout: 30_000 });
    await journey(browser);
    await browser.saveScreenshot(`${out}/accepted-${run}.png`);
    await sleep(1500);
  } catch (error) {
    const state = await browser?.execute(() => ({ text: document.body.innerText,
      inputs: [...document.querySelectorAll(".qc-panel input")].map(input => {
        const rect = input.getBoundingClientRect();
        const hit = document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2);
        return { label: input.getAttribute("aria-label"), rect: { x: rect.x, y: rect.y, width: rect.width, height: rect.height }, hit: hit ? { tag: hit.tagName, class: hit.className, text: hit.textContent?.slice(0, 80) } : null };
      }), queries: [...document.querySelectorAll(".query-block")].map(block => block.outerHTML.slice(0, 14000)) })).catch(() => null);
    const screenshot = `${out}/failure-${run}.png`;
    await browser?.saveScreenshot(screenshot).catch(() => {});
    fs.writeFileSync(`${out}/failure-${run}.json`, JSON.stringify({ testedCommit, diff: "uncommitted queries lane", journey: phase,
      classification: "ambiguous; classify from semantic state and driver log in the receipt", phase,
      expected: "native semantic journey completes", observed: String(error), screenshot, state }, null, 2));
    throw error;
  } finally {
    try { await browser?.deleteSession(); } catch {}
    try { process.kill(-processDriver.pid, "SIGTERM"); } catch {}
    fs.closeSync(log);
  }
}
async function markQuery(browser, title) {
  await browser.waitUntil(() => browser.execute((wanted) => {
    document.querySelector('[data-calm-query="active"]')?.removeAttribute("data-calm-query");
    const block = [...document.querySelectorAll(".page-blocks .query-block")].find(block => block.querySelector(".query-title")?.textContent?.trim() === wanted);
    if (!block) return false;
    block.setAttribute("data-calm-query", "active"); block.scrollIntoView({ block: "center" }); return true;
  }, title), { timeout: 20_000, interval: 200, timeoutMsg: `query ${title} absent` });
}
const active = '[data-calm-query="active"]';
async function action(browser, name) {
  await browser.$(`${active} .qc-actions > summary`).click();
  await browser.waitUntil(() => browser.execute((selector, wanted) => {
    const id = document.querySelector(`${selector} summary`)?.getAttribute("aria-controls");
    const menu = id ? document.getElementById(id) : null;
    const button = [...(menu?.querySelectorAll("button") ?? [])].find(button => button.textContent === wanted);
    if (!button) return false; button.setAttribute("data-calm-action", "active"); return true;
  }, active, name), { timeout: 10_000, timeoutMsg: `query action ${name} absent` });
  await browser.$('[data-calm-action="active"]').click();
}
async function textEdit(browser, text) {
  const title = await browser.execute(selector => document.querySelector(`${selector} .query-title`)?.textContent?.trim(), active);
  await action(browser, "Edit as text");
  const input = await browser.$(`${active} .query-text-pane-input`);
  await input.waitForExist({ timeout: 15_000 }); await input.setValue(text);
  await browser.waitUntil(() => browser.execute((selector) => {
    const button = [...document.querySelectorAll(`${selector} button`)].find(button => button.textContent?.trim() === "Save query text");
    if (!button || button.disabled) return false; button.setAttribute("data-calm-save", "active"); return true;
  }, active), { timeout: 15_000, timeoutMsg: "query text never became saveable" });
  await browser.$('[data-calm-save="active"]').click();
  // Saving can change `query` to `tine-query` and remount the macro. Locate
  // the domain title again instead of retaining the old DOM identity.
  await markQuery(browser, title);
}
async function closeEditor(browser) {
  if (await browser.$(`${active} .qc-done`).isExisting()) await browser.$(`${active} .qc-done`).click();
}
await session(1, async browser => {
  phase = "rest, title rename, inline live results";
  await openPageByName(browser, "Queries"); await markQuery(browser, "Open tasks");
  await browser.waitUntil(() => browser.execute(selector => document.querySelector(`${selector} .query-count`)?.textContent === "2", active), { timeout: 20_000 });
  assert.equal(await browser.execute(selector => !!document.querySelector(`${selector} .qs-sentence`), active), false);
  await browser.$(`${active} .query-title`).click();
  await browser.$(`${active} .query-title-input`).waitForExist({ timeout: 10_000 });
  await browser.$(`${active} .query-title-input`).click();
  await browser.keys(["Control", "a"]); await browser.keys("Calm tasks"); await browser.keys("Enter");
  await browser.waitUntil(() => fs.readFileSync(queries, "utf8").includes('"Calm tasks"'), { timeout: 15_000 });
  await markQuery(browser, "Calm tasks"); await action(browser, "Edit");
  assert.ok(await browser.execute(selector => !!document.querySelector(`${selector} .qc-panel .qs-sheet`) && !!document.querySelector(`${selector} .query-group`), active));
  await closeEditor(browser);
  await textEdit(browser, "@block and task = 'TODO' and priority = 'A'");
  await browser.waitUntil(() => browser.execute(selector => document.querySelector(`${selector} .query-count`)?.textContent === "1", active), { timeout: 20_000 });
  await closeEditor(browser);
  await browser.waitUntil(() => fs.readFileSync(queries, "utf8").includes("priority"), { timeout: 15_000 });
  phase = "copy result hierarchy and ordinary collapse save";
  await action(browser, "Copy results");
  await browser.$(".export-preview").waitForExist({ timeout: 15_000 });
  const copy = await browser.$(".export-preview").getValue();
  assert.ok(copy.includes("Tasks") && copy.includes("Child detail kept in copy"), copy);
  assert.ok(!copy.includes("Second journey task"), copy);
  fs.writeFileSync(`${out}/copy-preview.txt`, copy);
  await browser.execute(() => [...document.querySelectorAll(".export-foot button")].find(button => button.textContent === "Close")?.click());
  await action(browser, "Collapse all");
  await browser.waitUntil(() => fs.readFileSync(`${graph}/pages/Tasks.md`, "utf8").includes("collapsed:: true"), { timeout: 15_000 });
  await action(browser, "Expand all");
  await browser.waitUntil(() => !fs.readFileSync(`${graph}/pages/Tasks.md`, "utf8").includes("collapsed:: true"), { timeout: 15_000 });
  phase = "table task save and source navigation";
  await markQuery(browser, "Task table");
  await browser.$(`${active} [role="checkbox"]`).waitForExist({ timeout: 20_000 });
  await browser.$(`${active} [role="checkbox"]`).click();
  await browser.waitUntil(() => fs.readFileSync(`${graph}/pages/Tasks.md`, "utf8").includes("DONE [#A] First journey task"), { timeout: 15_000 });
  phase = "visible default columns and select none/all persist through the query save";
  await action(browser, "View");
  const ownerColumn = await browser.$(`${active} [aria-label="Columns"] input[aria-label="owner"]`);
  assert.ok(await ownerColumn.isSelected(), "visible owner column starts selected");
  // Use an interior point of the content viewport, away from native window
  // resize grips. WebKit's automatic nearest-edge scroll can land on one.
  await browser.execute(selector => document.querySelector(`${selector} input[aria-label="owner"]`)?.scrollIntoView({ block: "center" }), active);
  await browser.waitUntil(() => browser.execute(selector => {
    const input = document.querySelector(`${selector} input[aria-label="owner"]`);
    if (!input) return false;
    const rect = input.getBoundingClientRect();
    return document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2) === input;
  }, active), { timeout: 15_000, timeoutMsg: "owner column control is covered" });
  await ownerColumn.click();
  await browser.waitUntil(() => fs.readFileSync(queries, "utf8").includes("tine.columns::"), { timeout: 15_000 });
  await markQuery(browser, "Task table"); await closeEditor(browser);
  await action(browser, "View");
  await browser.execute(selector => [...document.querySelectorAll(`${selector} [aria-label="Columns"] button`)].find(button => button.textContent === "Select none")?.click(), active);
  await markQuery(browser, "Task table"); await closeEditor(browser);
  await browser.waitUntil(() => browser.execute(() => {
    // A property save reparses the host block and can remount its macro. The
    // selected title, not a temporary DOM marker, owns this observation.
    const query = [...document.querySelectorAll(".query-block")].find(block => block.querySelector(".query-title")?.textContent?.trim() === "Task table");
    return !!query?.querySelector(".sheet-title-cell") && !query.querySelector("[data-sheet-field-header]");
  }), { timeout: 15_000 });
  await markQuery(browser, "Task table");
  await action(browser, "View");
  await browser.execute(selector => [...document.querySelectorAll(`${selector} [aria-label="Columns"] button`)].find(button => button.textContent === "Select all")?.click(), active);
  await browser.waitUntil(() => browser.execute(() => {
    const query = [...document.querySelectorAll(".query-block")].find(block => block.querySelector(".query-title")?.textContent?.trim() === "Task table");
    return !!query?.querySelector('[data-sheet-field="prop:owner"]');
  }), { timeout: 15_000 });
  await markQuery(browser, "Task table"); await closeEditor(browser);
  phase = "planning comparisons through Rust";
  await markQuery(browser, "Planning query");
  await browser.waitUntil(() => browser.execute(selector => document.querySelector(selector)?.textContent?.includes("Overdue journey task"), active), { timeout: 20_000 });
  assert.ok(!await browser.execute(selector => document.querySelector(selector)?.textContent?.includes("Upcoming journey task"), active));
  await textEdit(browser, "@block and scheduled > today");
  await browser.waitUntil(() => browser.execute(selector => document.querySelector(selector)?.textContent?.includes("Upcoming journey task") && !document.querySelector(selector)?.textContent?.includes("Overdue journey task"), active), { timeout: 20_000 });
  await closeEditor(browser);
  phase = "TQL Open in Search and selected copy";
  await action(browser, "Open in Search");
  await browser.$(".query-workspace").waitForExist({ timeout: 15_000 });
  await browser.waitUntil(() => browser.execute(() => document.querySelector(".query-workspace")?.textContent?.includes("Upcoming journey task")), { timeout: 20_000 });
  await browser.execute(() => [...document.querySelectorAll(".query-results-selection button")].find(button => button.textContent === "Select results")?.click());
  await browser.execute(() => [...document.querySelectorAll(".query-results-selection button")].find(button => button.textContent === "Copy results")?.click());
  await browser.$(".export-preview").waitForExist({ timeout: 15_000 });
  assert.ok((await browser.$(".export-preview").getValue()).includes("Upcoming journey task"));
  await browser.execute(() => [...document.querySelectorAll(".export-foot button")].find(button => button.textContent === "Close")?.click());
  phase = "All pages backend counts and sort";
  if (!await browser.$(".nav-all-pages-open").isExisting()) {
    await browser.execute(() => [...document.querySelectorAll(".nav-section-header")].find(header => header.textContent?.includes("ALL PAGES"))?.click());
  }
  await browser.$(".nav-all-pages-open").waitForExist({ timeout: 15_000 });
  await browser.$(".nav-all-pages-open").click();
  await browser.$(".all-pages-backlinks").waitForExist({ timeout: 15_000 });
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".all-pages-row")].find(row => row.querySelector(".all-pages-open")?.textContent === "Tasks")?.querySelector(".all-pages-backlinks")?.textContent === "2"), { timeout: 20_000 });
  await browser.execute(() => [...document.querySelectorAll(".all-pages-col")].find(button => button.textContent.startsWith("Backlinks"))?.click());
  assert.equal(await browser.execute(() => document.querySelector(".all-pages-row .all-pages-open")?.textContent), "Tasks");
});
await session(2, async browser => {
  phase = "persisted query and task after restart, board navigation";
  await openPageByName(browser, "Queries"); await markQuery(browser, "Planning query");
  await browser.waitUntil(() => browser.execute(selector => document.querySelector(selector)?.textContent?.includes("Upcoming journey task") && !document.querySelector(selector)?.textContent?.includes("Overdue journey task"), active), { timeout: 20_000 });
  assert.ok(fs.readFileSync(`${graph}/pages/Tasks.md`, "utf8").includes("DONE [#A] First journey task"));
  await markQuery(browser, "Task board");
  await browser.$(`${active} .sheet-board-card`).waitForExist({ timeout: 20_000 });
  phase = "board last column access, drag save and genuine click navigation";
  await browser.execute(selector => {
    const card = [...document.querySelectorAll(`${selector} .sheet-board-card`)].find(card => card.textContent.includes("Second journey task"));
    card?.setAttribute("data-calm-card", "active"); card?.scrollIntoView({ block: "center", inline: "nearest" });
  }, active);
  await browser.$('[data-calm-card="active"] [role="checkbox"]').waitForExist({ timeout: 15_000 });
  const points = await browser.execute(selector => {
    const card = document.querySelector('[data-calm-card="active"]');
    const target = [...document.querySelectorAll(`${selector} .sheet-board-column`)].find(column => column.querySelector(".sheet-board-header > span")?.textContent === "DONE");
    const a = card.getBoundingClientRect(), b = target.getBoundingClientRect();
    return { from: { x: Math.round(a.x + a.width / 2), y: Math.round(a.y + a.height / 2) }, to: { x: Math.round(b.x + b.width / 2), y: Math.round(b.y + 90) } };
  }, active);
  await browser.performActions([{ type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [
    { type: "pointerMove", origin: "viewport", duration: 0, ...points.from }, { type: "pointerDown", button: 0 },
    { type: "pointerMove", origin: "viewport", duration: 500, ...points.to }, { type: "pointerUp", button: 0 },
  ] }]);
  await browser.releaseActions();
  await browser.waitUntil(() => fs.readFileSync(`${graph}/pages/Tasks.md`, "utf8").includes("DONE Second journey task"), { timeout: 15_000 });
  assert.equal(await browser.execute(() => document.querySelector("h1.page-title")?.textContent), "Queries", "dragging keeps the query open");
  await markQuery(browser, "Task board");
  await browser.$(`${active} .sheet-board-card`).click();
  await browser.waitUntil(() => browser.execute(() => document.querySelector("h1.page-title")?.textContent === "Tasks"), { timeout: 20_000 });
});
console.log("PASS native calm queries: rename, inline edit, real filtering, hierarchical copy, folding, task save, TQL Search, backend backlinks and restart/navigation");
