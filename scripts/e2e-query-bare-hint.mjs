// Linux real-WebKit journey for GH #422 (D-18): a stored `{{query (task)}}` answers what Logseq answers -- the
// bare clause adds no condition, so on its own the query shows nothing -- and a note ON the query says so and
// offers rewrites. Clicking **Open tasks** rewrites the block on disk to explicit open markers through the
// ordinary query save, and the query then lists the open tasks (WAITING included, DONE / CANCELED not).
// A real engine, renderer and save path are the point (parse_query -> og_hint -> print -> page file), which the
// jsdom test (src/components/QueryBareHint.test.tsx) can only mock.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4516);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4517);
const TMP_ROOT = path.resolve(process.env.E2E_TMP_ROOT || process.env.TMPDIR || "/tmp");
fs.mkdirSync(TMP_ROOT, { recursive: true });
const TMP = fs.mkdtempSync(path.join(TMP_ROOT, "tine-og-bare-query-e2e-"));
const GRAPH = `${TMP}/graph`;
const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || `${TMP}/artifacts`;

for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.mkdirSync(ARTIFACTS, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, '{:preferred-format "Markdown"\n :preferred-workflow :todo}\n');
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- Open [[Bare]]\n");

const OPEN = ["Write the report", "Reply from Ann"];
const FINISHED = ["Ship the release", "Old plan"];
fs.writeFileSync(`${GRAPH}/pages/Work.md`, [
  "- TODO Write the report",
  "- WAITING Reply from Ann",
  "- DONE Ship the release",
  "- CANCELED Old plan",
  "- Plain note about work",
  "",
].join("\n"));
const BARE = `${GRAPH}/pages/Bare.md`;
fs.writeFileSync(BARE, "- {{query (task)}}\n");
const REWRITTEN = "{{query (task TODO DOING NOW LATER WAITING WAIT STARTED IN-PROGRESS)}}";

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

/** What the page's query block shows: whether the note is up, its buttons, and the task names in the results. */
const observe = (browser) => browser.execute((names) => {
  const block = document.querySelector(".page-blocks .query-block");
  if (!block) return null;
  block.scrollIntoView({ block: "center" });
  const text = (node) => (node?.textContent ?? "").replace(/\s+/g, " ").trim();
  const hint = block.querySelector(".query-og-hint");
  const results = [...block.querySelectorAll(".query-group .ls-block")].map(text).join(" | ");
  return {
    hint: hint ? text(hint) : null,
    buttons: hint ? [...hint.querySelectorAll(".query-og-hint-rewrite")].map(text) : [],
    empty: text(block.querySelector(".query-empty")),
    found: names.filter((name) => results.includes(name)),
  };
}, [...OPEN, ...FINISHED, "Plain note about work"]);

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

const log = fs.openSync(`${TMP}/tauri-driver.log`, "w");
const td = spawn(TD, ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], {
  env, stdio: ["ignore", log, log], detached: true,
});
await sleep(2500);
let browser;
try {
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER_PORT, path: "/", logLevel: "error",
    connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
  });
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });
  try {
    await openPageByName(browser, "Bare");
    // As in Logseq, the bare clause adds no condition and the query has no other: it shows no task at all.
    const before = await observeWhen(browser, (o) => !!o.hint && /No results/.test(o.empty),
      "the bare (task) query never showed its note beside an empty answer");
    fs.writeFileSync(`${ARTIFACTS}/before.json`, `${JSON.stringify(before, null, 2)}\n`);
    if (before.found.length) throw new Error(`a bare (task) query listed ${JSON.stringify(before.found)}; Logseq lists nothing`);
    if (!before.hint.includes("adds no condition")) throw new Error(`the note did not explain the bare clause: ${before.hint}`);
    for (const label of ["Open tasks", "Any task"]) {
      if (!before.buttons.includes(label)) throw new Error(`the note offered ${JSON.stringify(before.buttons)}, not ${label}`);
    }

    const click = await browser.execute(() => {
      const button = [...document.querySelectorAll(".page-blocks .query-og-hint-rewrite")]
        .find((b) => (b.textContent ?? "").trim() === "Open tasks");
      if (!button) return false;
      button.click();
      return true;
    });
    if (!click) throw new Error("the Open tasks button vanished before it could be clicked");

    // The rewrite is saved to the page file as an ordinary Logseq query with explicit open markers.
    const saved = await waitForFileText(BARE, (text) => text.includes(REWRITTEN));
    fs.writeFileSync(`${ARTIFACTS}/Bare.md`, saved);
    if (saved.includes("{{query (task)}}")) throw new Error(`the bare query survived beside the rewrite: ${JSON.stringify(saved)}`);

    // The query now lists the open tasks and not the finished ones, and the note is gone.
    const after = await observeWhen(browser, (o) => OPEN.every((name) => o.found.includes(name)),
      "the rewritten query never listed the open tasks");
    fs.writeFileSync(`${ARTIFACTS}/after.json`, `${JSON.stringify(after, null, 2)}\n`);
    const wrong = after.found.filter((name) => !OPEN.includes(name));
    if (wrong.length) throw new Error(`the Open tasks query also listed ${JSON.stringify(wrong)}`);
    if (after.hint) throw new Error(`the note stayed on an explicit query: ${after.hint}`);
  } catch (error) {
    const state = await browser.execute(() => ({
      text: document.body.innerText,
      queryBlocks: [...document.querySelectorAll(".page-blocks .query-block")].map((b) => b.outerHTML.slice(0, 6000)),
    })).catch(() => ({}));
    fs.writeFileSync(`${ARTIFACTS}/failure-state.json`, `${JSON.stringify(state, null, 2)}\n`);
    try { await browser.saveScreenshot(`${ARTIFACTS}/failure.png`); } catch {}
    throw error;
  }
  await sleep(500);
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
console.log(`e2e-query-bare-hint: ok (artifacts in ${ARTIFACTS})`);
