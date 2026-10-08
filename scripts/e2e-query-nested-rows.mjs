// Linux real-WebKit journey for GH #651: Search / List / Table / Board are presentations of ONE result
// membership, so a task nested under another task must be a row of a Table query (and a card of a Board)
// exactly when the List shows it as a result. The reporter's shape: four parent tasks, three with a nested
// child task; `(task TODO DOING)` in Table view; then narrowed to `(task TODO)` so parent B stops matching.
// A real engine and renderer are the point (query_run -> SheetTable rows), which jsdom can only mock.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { openPageByName } from "./lib/e2e-navigation.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_BASE = Number(process.env.E2E_DRIVER_PORT || 4516);
const NATIVE_BASE = Number(process.env.E2E_NATIVE_PORT || 4517);
const TMP_ROOT = path.resolve(process.env.E2E_TMP_ROOT || process.env.TMPDIR || "/tmp");
fs.mkdirSync(TMP_ROOT, { recursive: true });
const TMP = fs.mkdtempSync(path.join(TMP_ROOT, "tine-og-nested-rows-e2e-"));
const GRAPH = `${TMP}/graph`;
const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || `${TMP}/artifacts`;

for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.mkdirSync(ARTIFACTS, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, '{:preferred-format "Markdown"\n :preferred-workflow :todo}\n');
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- Open [[Tasks]]\n");

// The reporter's outline, with every task name unique so a title cell identifies exactly one block.
const OUTLINE = [
  "- # Outline",
  "\t- TODO Parent task A",
  "\t  SCHEDULED: <2027-06-01 Tue>",
  "\t\t- TODO Child task A1",
  "\t\t  SCHEDULED: <2026-10-09 Fri>",
  "\t- DOING Parent task B",
  "\t\t- TODO Child task B1",
  "\t- TODO Parent task C",
  "\t\t- DOING Child task C1",
  "\t- TODO Sibling task D",
  "\t  SCHEDULED: <2026-10-08 Thu>",
  // A task nested under a NON-task parent, and a task two levels down.
  "\t- Note parent",
  "\t\t- TODO Child of a note",
  "\t- DONE Done parent",
  "\t\t- TODO Child of a done task",
  "\t\t\t- TODO Grandchild of a done task",
].join("\n");
const page = (query, view) => [
  "- # Open tasks",
  `\t- {{query ${query}}}`,
  ...(view ? [`\t  tine.view:: ${view}`] : []),
  "\t  tine.columns:: state;scheduled;deadline;page",
  "\t  tine.sort:: scheduled asc",
  "",
].join("\n");
// The query is graph-wide, so the tasks live on their own page and each query page holds only its query.
fs.writeFileSync(`${GRAPH}/pages/Outline.md`, `${OUTLINE}\n`);
fs.writeFileSync(`${GRAPH}/pages/Tasks.md`, page("(task TODO DOING)", "table"));
fs.writeFileSync(`${GRAPH}/pages/Narrow.md`, page("(task TODO)", "table"));
fs.writeFileSync(`${GRAPH}/pages/Listed.md`, page("(task TODO DOING)", null));
fs.writeFileSync(`${GRAPH}/pages/Boarded.md`, page("(task TODO DOING)", "board"));

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: `${TMP}/xdg/data`,
  XDG_CONFIG_HOME: `${TMP}/xdg/config`,
  XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  GDK_BACKEND: "x11",
};

async function withApp(index, fn) {
  const driverPort = DRIVER_BASE + index * 2;
  const nativePort = NATIVE_BASE + index * 2;
  const log = fs.openSync(`${TMP}/tauri-driver-${index}.log`, "w");
  const td = spawn(TD, ["--port", String(driverPort), "--native-port", String(nativePort), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], {
    env, stdio: ["ignore", log, log], detached: true,
  });
  await sleep(2500);
  let browser;
  try {
    browser = await remote({
      hostname: "127.0.0.1", port: driverPort, path: "/", logLevel: "error",
      connectionRetryCount: 1, connectionRetryTimeout: 60_000,
      capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
    });
    await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });
    try {
      await fn(browser);
    } catch (error) {
      const state = await browser.execute(() => ({
        text: document.body.innerText,
        queryBlocks: [...document.querySelectorAll(".page-blocks .query-block")].map((b) => b.outerHTML.slice(0, 6000)),
      })).catch(() => ({}));
      fs.writeFileSync(`${ARTIFACTS}/failure-state-${index}.json`, `${JSON.stringify(state, null, 2)}\n`);
      try { await browser.saveScreenshot(`${ARTIFACTS}/failure-${index}.png`); } catch {}
      throw error;
    }
    await sleep(500);
  } finally {
    try { await browser?.deleteSession(); } catch {}
    try { process.kill(-td.pid, "SIGKILL"); } catch {}
    fs.closeSync(log);
  }
}

/** What the first query block shows: its count badge and the names of its task rows (title cells for Table,
 *  card titles for Board, the nested block text for List), in display order. */
const observe = (browser) => browser.execute(() => {
  const block = document.querySelector(".page-blocks .query-block");
  if (!block) return null;
  block.scrollIntoView({ block: "center" });
  const text = (node) => (node.textContent ?? "").replace(/\s+/g, " ").trim();
  const names = [...block.querySelectorAll(".sheet-title-cell, .sheet-board-card-title, .query-group .ls-block")]
    .map(text).flatMap((t) => [...t.matchAll(/(?:Parent|Child|Sibling|Grandchild) (?:task [A-D]\d?|of a (?:note|done task))/g)].map((m) => m[0]));
  return { count: text(block.querySelector(".query-count") ?? block).slice(0, 12), names, text: text(block) };
});

async function observeWhen(browser, ready, what) {
  let last = null;
  await browser.waitUntil(async () => {
    last = await observe(browser);
    return !!last && ready(last);
  }, { timeout: 25_000, interval: 200 }).catch(() => {
    throw new Error(`${what}; the query block showed ${JSON.stringify(last)}`);
  });
  return last;
}

const same = (a, b) => JSON.stringify([...new Set(a)].sort()) === JSON.stringify([...new Set(b)].sort());

// Every membership below is OG's: a matching block is dropped from the top level only when its IMMEDIATE parent
// also matches (it then renders inside that parent's subtree, where the List shows it); every other match is a row.
const BOTH = {
  // (task TODO DOING): all four parents match, so A1/B1/C1 sit under matching parents. The note's child, and the
  // done task's child, have a non-matching parent: they are their own rows.
  "Tasks": ["Parent task A", "Parent task B", "Parent task C", "Sibling task D", "Child of a note", "Child of a done task"],
  // (task TODO): B (DOING) and the done parent do not match, so B1 and the done task's child are own rows; the
  // grandchild's parent matches, so it stays inside it.
  "Narrow": ["Parent task A", "Child task B1", "Parent task C", "Sibling task D", "Child of a note", "Child of a done task"],
};

await withApp(0, async (browser) => {
  // Table: the four parents (and the two own rows below non-matching parents). A1/B1/C1 sit under a MATCHING
  // parent, so they are not top-level results -- exactly OG's table (`tree/filter-top-level-blocks`).
  await openPageByName(browser, "Tasks");
  const table = await observeWhen(browser, (o) => o.names.length >= 4, "the Table never listed task rows");
  fs.writeFileSync(`${ARTIFACTS}/tasks-table.json`, `${JSON.stringify(table, null, 2)}\n`);
  if (!same(table.names, BOTH.Tasks)) {
    throw new Error(`Table (task TODO DOING) rows ${JSON.stringify(table.names)}; expected ${JSON.stringify(BOTH.Tasks)}`);
  }
  if (table.count !== String(BOTH.Tasks.length)) throw new Error(`Table count badge ${table.count}; expected ${BOTH.Tasks.length}`);

  // Narrowed to (task TODO): parent B no longer matches, so B1 is its own row.
  await openPageByName(browser, "Narrow");
  const narrow = await observeWhen(browser, (o) => o.names.length >= 3, "the narrowed Table never listed task rows");
  fs.writeFileSync(`${ARTIFACTS}/narrow-table.json`, `${JSON.stringify(narrow, null, 2)}\n`);
  if (!same(narrow.names, BOTH.Narrow)) {
    throw new Error(`Table (task TODO) rows ${JSON.stringify(narrow.names)}; expected ${JSON.stringify(BOTH.Narrow)}`);
  }
  if (narrow.count !== String(BOTH.Narrow.length)) throw new Error(`narrowed count badge ${narrow.count}; expected ${BOTH.Narrow.length}`);

  // Board: the same membership as the Table.
  await openPageByName(browser, "Boarded");
  const board = await observeWhen(browser, (o) => o.names.length >= 4, "the Board never listed task cards");
  fs.writeFileSync(`${ARTIFACTS}/tasks-board.json`, `${JSON.stringify(board, null, 2)}\n`);
  if (!same(board.names, BOTH.Tasks)) {
    throw new Error(`Board (task TODO DOING) cards ${JSON.stringify(board.names)}; expected ${JSON.stringify(BOTH.Tasks)}`);
  }
  if (board.count !== table.count) throw new Error(`Board count ${board.count} differs from the Table's ${table.count}`);

  // List: the same results, and the nested children show under their matching parents (what the reporter saw).
  await openPageByName(browser, "Listed");
  const list = await observeWhen(browser, (o) => BOTH.Tasks.every((n) => o.names.includes(n)), "the List never listed the task results");
  fs.writeFileSync(`${ARTIFACTS}/tasks-list.json`, `${JSON.stringify(list, null, 2)}\n`);
  const nested = ["Child task A1", "Child task B1", "Child task C1"].filter((n) => !list.names.includes(n));
  if (nested.length) throw new Error(`the List did not show the children under their matching parents: missing ${JSON.stringify(nested)}`);
  if (list.count !== table.count) throw new Error(`List count ${list.count} differs from the Table's ${table.count}`);
});
console.log(`e2e-query-nested-rows: ok (artifacts in ${ARTIFACTS})`);
