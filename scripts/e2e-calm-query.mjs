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
fs.writeFileSync(`${graph}/pages/Links.md`, "- [[Tasks]] [[Tasks]]\n  unrelated-column:: only outside the task results\n");
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
// Result actions refuse ("results changed since the query ran") until a graph
// save has re-run the answer; the query block is aria-busy until then.
async function resultsCurrent(browser) {
  await browser.waitUntil(() => browser.execute(selector => {
    const block = document.querySelector(selector);
    return !!block && block.getAttribute("aria-busy") !== "true";
  }, active), { timeout: 15_000, interval: 100, timeoutMsg: "query results never became current" });
}
async function action(browser, name) {
  if (["Copy results", "Collapse all", "Expand all"].includes(name)) await resultsCurrent(browser);
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
  // Saving can change `query` to `tine-query`. Locate the saved query by its
  // domain title before observing the editor and keyboard return path.
  await markQuery(browser, title);
  assert.ok(await browser.execute(selector => document.querySelector(`${selector} .qc-panel`)?.contains(document.activeElement), active),
    "Save query text keeps keyboard focus inside the editor");
  await browser.keys("Escape");
  assert.equal(await browser.$(`${active} .qc-panel`).isExisting(), false, "Save then Escape closes the editor");
  assert.ok(await browser.execute(selector => document.activeElement === document.querySelector(`${selector} .qc-actions > summary`), active),
    "Escape returns focus to the query actions");
}
async function closeEditor(browser) {
  if (await browser.$(`${active} .qc-done`).isExisting()) {
    await browser.$(`${active} .qc-done`).click();
    assert.ok(await browser.execute(selector => document.activeElement === document.querySelector(`${selector} .qc-actions > summary`), active),
      "Done returns focus to the query actions");
  }
}
await session(1, async browser => {
  phase = "rest, title rename, inline live results";
  await openPageByName(browser, "Queries"); await markQuery(browser, "Open tasks");
  await browser.waitUntil(() => browser.execute(selector => document.querySelector(`${selector} .query-count`)?.textContent === "2", active), { timeout: 20_000 });
  assert.equal(await browser.execute(selector => !!document.querySelector(`${selector} .qs-sentence`), active), false);
  phase = "folding the query bullet folds its results (GH #619 c6)";
  const hostToggle = selector => document.querySelector(selector).closest(".ls-block").querySelector(":scope > .block-main .collapse-toggle")?.click();
  await browser.execute(hostToggle, active);
  await browser.waitUntil(() => browser.execute(selector => document.querySelector(`${selector} .query-group`) === null, active),
    { timeout: 10_000, timeoutMsg: "folding the query's bullet left its results shown" });
  await browser.waitUntil(() => /Open tasks"\}\}\}\n\s+collapsed:: true/.test(fs.readFileSync(queries, "utf8")), { timeout: 15_000, timeoutMsg: "the bullet fold was not saved as collapsed:: true" });
  await browser.execute(hostToggle, active);
  await browser.waitUntil(() => browser.execute(selector => !!document.querySelector(`${selector} .query-group`), active),
    { timeout: 15_000, timeoutMsg: "unfolding the query's bullet did not bring its results back" });
  await browser.waitUntil(() => !/Open tasks"\}\}\}\n\s+collapsed:: true/.test(fs.readFileSync(queries, "utf8")), { timeout: 15_000, timeoutMsg: "the bullet unfold was not saved" });
  phase = "rest, title rename, inline live results";
  // One click on the title starts the rename with the caret in the field
  // (GH #619 c6: WebView2 honoured `autofocus` once per document).
  await browser.$(`${active} .query-title`).click();
  await browser.$(`${active} .query-title-input`).waitForExist({ timeout: 10_000 });
  await browser.waitUntil(() => browser.execute(selector => document.activeElement === document.querySelector(`${selector} .query-title-input`), active),
    { timeout: 5_000, timeoutMsg: "one click on the title did not put the caret in the rename field" });
  await browser.execute(selector => {
    const owner = document.querySelector(selector).closest(".ls-block");
    const samples = [];
    const sample = () => {
      const query = owner.querySelector(".query-block");
      samples.push({ count: query?.querySelector(".query-count")?.textContent ?? null,
        results: query?.querySelectorAll(".query-group").length ?? 0 });
    };
    const observer = new MutationObserver(sample);
    observer.observe(owner, { childList: true, subtree: true, characterData: true });
    let frame;
    const paint = () => { sample(); frame = requestAnimationFrame(paint); };
    paint();
    window.__calmRename = { stop: () => { observer.disconnect(); cancelAnimationFrame(frame); sample(); return samples; } };
  }, active);
  await browser.keys(["Control", "a"]); await browser.keys("Calm tasks"); await browser.keys("Enter");
  await browser.waitUntil(() => fs.readFileSync(queries, "utf8").includes('"Calm tasks"'), { timeout: 15_000 });
  await markQuery(browser, "Calm tasks"); await resultsCurrent(browser);
  const renameSamples = await browser.execute(() => window.__calmRename.stop());
  fs.writeFileSync(`${out}/rename-samples.json`, JSON.stringify(renameSamples));
  assert.ok(renameSamples.length > 0 && renameSamples.every(sample => sample.count === "2" && sample.results > 0),
    `title-only rename never empties the shown answer: ${JSON.stringify(renameSamples)}`);
  await action(browser, "Edit");
  await browser.waitUntil(() => browser.execute(selector => !!document.querySelector(`${selector} .qc-panel .qs-sheet`) && !!document.querySelector(`${selector} .query-group`), active), { timeout: 15_000, timeoutMsg: "editor panel or results missing after rename" });
  await closeEditor(browser);
  await textEdit(browser, "@block and task = 'TODO' and priority = 'A'");
  await browser.waitUntil(() => browser.execute(() => {
    const query = [...document.querySelectorAll(".page-blocks .query-block")].find(block => block.querySelector(".query-title")?.textContent?.trim() === "Calm tasks");
    return query?.querySelector(".query-count")?.textContent === "1";
  }), { timeout: 20_000 });
  await markQuery(browser, "Calm tasks");
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
  await resultsCurrent(browser);
  await browser.$(`${active} [role="checkbox"]`).click();
  await browser.waitUntil(() => fs.readFileSync(`${graph}/pages/Tasks.md`, "utf8").includes("DONE [#A] First journey task"), { timeout: 15_000 });
  phase = "visible default columns and select none/all persist through the query save";
  await action(browser, "View");
  const ownerColumn = await browser.$(`${active} [aria-label="Columns"] input[aria-label="owner"]`);
  assert.ok(await ownerColumn.isSelected(), "visible owner column starts selected");
  phase = "column vocabulary belongs to the current results (GH #680)";
  await browser.execute(selector => [...document.querySelectorAll(`${selector} [aria-label="Columns"] button`)]
    .find(button => button.textContent?.includes("column"))?.click(), active);
  const offeredColumns = await browser.execute(selector => [...document.querySelectorAll(`${selector} [aria-label="Columns"] [role="option"]`)]
    .map(option => option.textContent?.trim()), active);
  assert.ok(offeredColumns.includes("owner"), "result property is offered");
  assert.ok(!offeredColumns.includes("unrelated-column"), "unrelated graph property is absent");
  assert.ok(!offeredColumns.includes("scheduled"), "built-in absent from the task results is absent");
  await browser.execute(selector => [...document.querySelectorAll(`${selector} [aria-label="Columns"] button`)]
    .find(button => button.textContent?.includes("column"))?.click(), active);
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
  phase = "frozen Search selection skips a changed positional source";
  await browser.execute(() => [...document.querySelectorAll(".query-results-selection button")].find(button => button.textContent === "Select results")?.click());
  const selectedLocator = await browser.$(".query-workspace .query-result-row").getAttribute("data-inpage-find-surface");
  const planningPath = `${graph}/pages/Planning.md`;
  fs.writeFileSync(planningPath, "- Unselected insertion\n" + fs.readFileSync(planningPath, "utf8"));
  await browser.waitUntil(() => browser.execute(previous => {
    const row = document.querySelector(".query-workspace .query-result-row");
    return row?.textContent?.includes("Upcoming journey task") && row.getAttribute("data-inpage-find-surface") !== previous;
  }, selectedLocator), { timeout: 20_000 });
  await browser.execute(() => [...document.querySelectorAll(".query-results-selection button")].find(button => button.textContent === "Copy results")?.click());
  await browser.waitUntil(() => browser.execute(() => document.body.innerText.includes("1 results changed since the query ran; run it again")), { timeout: 20_000 });
  assert.equal(await browser.$(".export-preview").isExisting(), false);
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
  phase = "saved planning comparison reopens without changing its answer";
  await action(browser, "Edit");
  await browser.execute(selector => document.querySelector(`${selector} .qs-value`)?.click(), active);
  await browser.$('[aria-label="Date comparison"]').waitForExist({ timeout: 15_000 });
  assert.equal(await browser.$('[aria-label="Date comparison"]').getValue(), "gt");
  assert.equal(await browser.$('[aria-label="Date"]').getValue(), "today");
  await browser.execute(() => document.querySelector(".qs-value-editor .qs-commit")?.click());
  await closeEditor(browser);
  await browser.waitUntil(() => browser.execute(selector => document.querySelector(selector)?.textContent?.includes("Upcoming journey task") && !document.querySelector(selector)?.textContent?.includes("Overdue journey task"), active), { timeout: 20_000 });
  await markQuery(browser, "Task board");
  await browser.$(`${active} .sheet-board-card`).waitForExist({ timeout: 20_000 });
  phase = "board last column access, drag save and genuine click navigation";
  await resultsCurrent(browser);
  // Start the independent pointer journey from neutral focus. The board's
  // existing captured blur listener cancels a drag if an old control blurs
  // during pointerdown; that focus-transition defect is outside this lane.
  await browser.execute(() => document.activeElement?.blur());
  await browser.execute(selector => {
    const card = [...document.querySelectorAll(`${selector} .sheet-board-card`)].find(card => card.textContent.includes("Second journey task"));
    card?.scrollIntoView({ block: "center", inline: "nearest" });
  }, active);
  // Scrolling a cold card near can hydrate its source and refresh the answer.
  // Wait after that admission, then choose the current card by domain identity.
  await resultsCurrent(browser);
  const points = await browser.execute(selector => {
    const card = [...document.querySelectorAll(`${selector} .sheet-board-card`)].find(card => card.textContent.includes("Second journey task"));
    card?.setAttribute("data-calm-card", "active");
    const target = [...document.querySelectorAll(`${selector} .sheet-board-column`)].find(column => column.querySelector(".sheet-board-header > span")?.textContent === "DONE");
    const a = card.getBoundingClientRect(), b = target.getBoundingClientRect();
    const points = { from: { x: Math.round(a.x + a.width / 2), y: Math.round(a.y + a.height / 2) }, to: { x: Math.round(b.x + b.width / 2), y: Math.round(b.y + 90) } };
    const events = [];
    const trace = event => events.push({ type: event.type, target: event.target?.className,
      ghost: !!document.querySelector(".sheet-board-drag-ghost"), busy: document.querySelector(selector)?.getAttribute("aria-busy") });
    const types = ["pointerdown", "pointerup", "pointercancel", "lostpointercapture", "blur"];
    for (const type of types) window.addEventListener(type, trace, true);
    window.__calmBoardTrace = { stop: () => { for (const type of types) window.removeEventListener(type, trace, true); return events; } };
    return { ...points, hits: [points.from, points.to].map(point => {
      const hit = document.elementFromPoint(point.x, point.y);
      return { class: hit?.className, column: hit?.closest(".sheet-board-column")?.querySelector(".sheet-board-header > span")?.textContent };
    }) };
  }, active);
  fs.writeFileSync(`${out}/board-drag-points.json`, JSON.stringify(points));
  await browser.performActions([{ type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [
    { type: "pointerMove", origin: "viewport", duration: 0, ...points.from }, { type: "pointerDown", button: 0 },
    { type: "pointerMove", origin: "viewport", duration: 500, ...points.to }, { type: "pointerUp", button: 0 },
  ] }]);
  await browser.releaseActions();
  fs.writeFileSync(`${out}/board-drag-events.json`, JSON.stringify(await browser.execute(() => window.__calmBoardTrace.stop())));
  await browser.waitUntil(() => fs.readFileSync(`${graph}/pages/Tasks.md`, "utf8").includes("DONE Second journey task"), { timeout: 15_000 });
  assert.equal(await browser.execute(() => document.querySelector("h1.page-title")?.textContent), "Queries", "dragging keeps the query open");
  await markQuery(browser, "Task board");
  await browser.$(`${active} .sheet-board-card`).click();
  await browser.waitUntil(() => browser.execute(() => document.querySelector("h1.page-title")?.textContent === "Tasks"), { timeout: 20_000 });
});
console.log("PASS native calm queries: rename, inline edit, real filtering, hierarchical copy, folding, task save, TQL Search, backend backlinks and restart/navigation");
