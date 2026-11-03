// CM step 2 batch 3 (conceptual model Q2, "what is a file's page name"), by
// the user's literal paths in a graph with the default (legacy) file-name
// format.
// UI-CM-ORG-FILE-LINK-LEGACY-FORMAT: Ctrl/Cmd+O on an Org link
// [[file:./a.b.org][ab]] opened "a.b" (a blank page) instead of the file's
// page "a/b"; [[file:./Ten___Child.org][ten]] opened "Ten/Child" instead of
// "Ten___Child". UI-CM-CONFLICT-ROW-PAGE-TITLE: the conflict overview listed
// the sync copy of a page with `title:: Bar` as "foo".
// Navigation must create no file and change no bytes.
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { remote } from "webdriverio";
import { currentPageTitle, openPageByName } from "./lib/e2e-navigation.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
const DRIVER = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin/tauri-driver") : "tauri-driver");
const TMP = fs.mkdtempSync("/tmp/tine-cm-file-names-");
const GRAPH = path.join(TMP, "graph");
const ARTIFACT = process.env.E2E_ARTIFACT_DIR || path.join(TMP, "artifacts");
const port = Number(process.env.E2E_DRIVER_PORT || 4736);
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, dir));
fs.mkdirSync(ARTIFACT, { recursive: true });
// No :file/name-format: the graph uses the default, legacy.
fs.writeFileSync(path.join(GRAPH, "logseq", "config.edn"), "{:tine/guide-announced? true}\n");
const files = {
  "pages/a.b.org": "* Body of the a slash b page\n",
  "pages/Ten___Child.org": "* Body of the ten child page\n",
  "pages/Org Links.org": "* first link here [[file:./a.b.org][ab]]\n* second link here [[file:./Ten___Child.org][ten]]\n",
  "pages/foo.md": "title:: Bar\n\n- winner body\n",
  "pages/foo.sync-conflict-20261101-120000-ABCDEFG.md": "title:: Bar\n\n- copy body\n",
};
for (const [rel, text] of Object.entries(files)) fs.writeFileSync(path.join(GRAPH, rel), text);
const env = {
  ...process.env, TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "data"), XDG_CONFIG_HOME: path.join(TMP, "config"), XDG_CACHE_HOME: path.join(TMP, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
};
const log = fs.openSync(path.join(ARTIFACT, "driver.log"), "w");
const driver = spawn(DRIVER, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
const proof = { app: APP, graph: GRAPH, steps: [], failures: [] };
/** Run one user check; a failure is recorded and the journey goes on, so one
 *  run reports every broken path. The journey fails at the end if any did. */
async function check(what, run) {
  try { await run(); } catch (error) { proof.failures.push(`${what}: ${error.message ?? error}`); }
}

const mainText = () => browser.execute(() => document.querySelector(".main-content .page-blocks")?.textContent ?? "");

/** Native click on the start of the block whose text starts with `prefix`
 *  (its plain words, not its link), then Ctrl+O. */
async function followFromBlock(prefix) {
  let why = "";
  const point = await browser.waitUntil(async () => {
    const found = await browser.execute((wanted) => {
    const content = [...document.querySelectorAll(".main-content .ls-block .block-content-wrapper")]
      .find((node) => (node.textContent ?? "").trim().startsWith(wanted));
    if (!content) return "no block";
    content.scrollIntoView({ block: "center" });
    // The block's leading words: its first rendered span, which is not the link.
    const words = content.querySelector(".block-content > span");
    const rect = words?.getBoundingClientRect();
    if (!rect || !rect.width) return "no leading words";
    const x = Math.round(rect.x + Math.min(12, rect.width / 2)), y = Math.round(rect.y + rect.height / 2);
    const hit = document.elementFromPoint(x, y);
    if (!content.contains(hit) || hit?.closest("a")) return `covered by ${hit?.outerHTML?.slice(0, 120)}`;
    return { x, y };
    }, prefix);
    if (typeof found === "object" && found) return found;
    why = String(found);
    return false;
  }, { timeout: 15_000, interval: 150 }).catch(() => { throw new Error(`no clickable block starting ${JSON.stringify(prefix)}: ${why}`); });
  await browser.performActions([
    { type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [{ type: "pointerMove", origin: "viewport", ...point }, { type: "pointerDown", button: 0 }, { type: "pointerUp", button: 0 }] },
  ]);
  await browser.releaseActions();
  await browser.$(".main-content textarea.block-editor").waitForExist({ timeout: 10_000 });
  await browser.keys(["Control", "o"]);
}

async function expectFollow(prefix, link, title, body) {
  await openPageByName(browser, "Org Links");
  await followFromBlock(prefix);
  try {
    await browser.waitUntil(async () => (await currentPageTitle(browser)) === title && (await mainText()).includes(body),
      { timeout: 15_000, interval: 150 });
  } catch {
    throw new Error(`Ctrl+O on ${link} opened ${JSON.stringify(await currentPageTitle(browser))} showing ${JSON.stringify((await mainText()).slice(0, 200))}; expected the page ${JSON.stringify(title)} with ${JSON.stringify(body)}`);
  }
  await browser.keys("Escape").catch(() => {});
  proof.steps.push(`Ctrl+O on ${link} opens ${title} with its body`);
}

try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });

  await check("legacy dot link", () => expectFollow("first link here", "[[file:./a.b.org][ab]]", "a/b", "Body of the a slash b page"));
  await check("legacy triple-lowbar stem link", () => expectFollow("second link here", "[[file:./Ten___Child.org][ten]]", "Ten___Child", "Body of the ten child page"));

  // The conflict overview, reached as the user does: the sidebar's badge.
  await check("conflict overview row", async () => {
  await browser.waitUntil(() => browser.execute(() => {
    const badge = document.querySelector(".conflict-queue-badge");
    if (!(badge instanceof HTMLElement)) return false;
    badge.click();
    return true;
  }), { timeout: 20_000, interval: 200, timeoutMsg: "the conflict badge never appeared" });
  let rows = [];
  try {
    await browser.waitUntil(async () => {
      rows = await browser.execute(() => [...document.querySelectorAll(".conflict-overview .conflict-overview-open")]
        .map((node) => (node.textContent ?? "").trim()));
      return rows.includes("Bar");
    }, { timeout: 15_000, interval: 150 });
  } catch {
    throw new Error(`the conflict overview lists ${JSON.stringify(rows)}; expected the page's name "Bar" (pages/foo.md has title:: Bar)`);
  }
  if (rows.includes("foo")) throw new Error(`the conflict overview still lists the file stem: ${JSON.stringify(rows)}`);
  proof.steps.push("the conflict overview lists the copy of pages/foo.md (title:: Bar) as Bar");
  });

  const listed = (dir) => fs.readdirSync(path.join(GRAPH, dir)).sort();
  const want = Object.keys(files).map((rel) => rel.slice("pages/".length)).sort();
  if (JSON.stringify(listed("pages")) !== JSON.stringify(want)) throw new Error(`navigation changed pages: ${listed("pages")}`);
  for (const [rel, text] of Object.entries(files)) {
    if (fs.readFileSync(path.join(GRAPH, rel), "utf8") !== text) throw new Error(`navigation changed ${rel}`);
  }
  proof.steps.push("navigation creates no file and leaves fixture bytes unchanged");
  if (proof.failures.length) throw new Error(proof.failures.join("\n"));
  console.log(JSON.stringify(proof));
} catch (error) {
  proof.error = String(error);
  if (browser) {
    await browser.saveScreenshot(path.join(ARTIFACT, "failure.png")).catch(() => {});
    fs.writeFileSync(path.join(ARTIFACT, "failure-dom.html"), await browser.execute(() => document.body.innerHTML).catch(() => ""));
  }
  throw error;
} finally {
  fs.writeFileSync(path.join(ARTIFACT, "proof.json"), JSON.stringify(proof, null, 2));
  await browser?.deleteSession().catch(() => {});
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
