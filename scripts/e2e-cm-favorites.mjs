// CM step 2 batch 4 (census Q12): favorites follow a rename, with one answerer
// (the backend's rename config step), on the user's paths.
// UI-CM-FAVORITES-CLI-RENAME-STALE: a CLI `page rename` with no window open
// (`tine serve`) left the sidebar favorite on the old name, so opening it
// opened a blank page of that name. UI-CM-FAVORITES-NAMESPACE-CHILD-STALE: an
// app rename of a namespace parent left a favorite namespace child on its old
// name. Also: a title:: change while the page is open moves its favorite
// (FAV-RETITLE, through the backend's `follow_page_identity`).
// Every check reads both config.edn (what persists) and the sidebar (what
// the user sees), and opens the favorite with a native click.
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { remote } from "webdriverio";
import { currentPageTitle, openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { installAccountHome } from "./lib/e2e-account-home.mjs";
import { ensurePrivateSessionBus } from "./lib/e2e-session-bus.mjs";

ensurePrivateSessionBus();

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = path.resolve(process.env.TINE_APP || path.join(ROOT, "target/debug/tine"));
const DRIVER = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin/tauri-driver") : "tauri-driver");
const TMP = fs.mkdtempSync("/tmp/tine-cm-favorites-");
const GRAPH = path.join(TMP, "graph");
const ARTIFACT = process.env.E2E_ARTIFACT_DIR || path.join(TMP, "artifacts");
const port = Number(process.env.E2E_DRIVER_PORT || 4736);
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache", "state"]) fs.mkdirSync(path.join(TMP, dir));
fs.mkdirSync(ARTIFACT, { recursive: true });
const CONFIG = path.join(GRAPH, "logseq", "config.edn");
fs.writeFileSync(CONFIG,
  "{:file/name-format :triple-lowbar\n :favorites [\"Alpha\" \"proj/child\" \"proj\" \"Titled\"]\n :tine/guide-announced? true}\n");
const files = {
  "pages/Alpha.md": "- alpha body\n",
  "pages/proj.md": "- parent body\n",
  "pages/proj___child.md": "- child body\n",
  "pages/Titled.md": "title:: Titled\n\n- titled body\n",
};
for (const [rel, text] of Object.entries(files)) fs.writeFileSync(path.join(GRAPH, rel), text);
installAccountHome(TMP, ARTIFACT);
const env = {
  ...process.env, TAURI_WEBVIEW_AUTOMATION: "true", TINE_GRAPH: GRAPH, TINE_RUN: "",
  XDG_DATA_HOME: path.join(TMP, "data"), XDG_CONFIG_HOME: path.join(TMP, "config"), XDG_CACHE_HOME: path.join(TMP, "cache"), XDG_STATE_HOME: path.join(TMP, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
};
let browser, driver, serve, log;
const proof = { app: APP, graph: GRAPH, steps: [], failures: [] };
/** Record a failed outcome and go on, so one run shows every journey's
 *  result (fail-before evidence); any failure fails the run at the end. */
async function check(what, run) {
  try { await run(); } catch (error) { proof.failures.push(`${what}: ${error.message ?? error}`); console.log(`FAILURE ${what}: ${error.message ?? error}`); }
}

/** `tine <args> --graph G --json`; resolves with the parsed reply. */
function cli(args) {
  return new Promise((resolve, reject) => {
    const child = spawn(APP, [...args, "--graph", GRAPH, "--json"], { env, timeout: 60_000 });
    let out = "", err = "";
    child.stdout.on("data", (d) => (out += d));
    child.stderr.on("data", (d) => (err += d));
    child.on("error", reject);
    child.on("exit", (code) => {
      if (code !== 0) reject(new Error(`tine ${args.join(" ")} exited ${code}: ${err}`));
      else { try { resolve(JSON.parse(out)); } catch (e) { reject(e); } }
    });
  });
}

/** `:favorites` as config.edn now stores it (the fixture's plain shape). */
function storedFavorites() {
  const text = fs.readFileSync(CONFIG, "utf8");
  const vector = /:favorites\s*\[([^\]]*)\]/.exec(text)?.[1] ?? "";
  return [...vector.matchAll(/"((?:[^"\\]|\\.)*)"/g)].map((m) => JSON.parse(`"${m[1]}"`));
}

/** The Favorites section's page rows, in sidebar order. */
const sidebarFavorites = () => browser.execute(() => {
  const section = [...document.querySelectorAll(".nav-section")].find((node) => node.querySelector(".nav-fav-add-group"));
  return [...(section?.querySelectorAll(".nav-page:not(.nav-fav-group)") ?? [])].map((row) => row.textContent.replace("⭐", "").trim());
});

async function expectFavorites(expected, what) {
  try {
    await browser.waitUntil(async () => JSON.stringify(storedFavorites()) === JSON.stringify(expected)
      && JSON.stringify(await sidebarFavorites()) === JSON.stringify(expected), { timeout: 15_000, interval: 150 });
  } catch {
    throw new Error(`${what}: expected favorites ${JSON.stringify(expected)}; config.edn has ${JSON.stringify(storedFavorites())}, the sidebar shows ${JSON.stringify(await sidebarFavorites())}`);
  }
  proof.steps.push(`${what}: config.edn and the sidebar list ${JSON.stringify(expected)}`);
}

/** Native click on the sidebar favorite `name`; it must open that page with `body`. */
async function openFavorite(name, body) {
  const point = await browser.waitUntil(() => browser.execute((wanted) => {
    const section = [...document.querySelectorAll(".nav-section")].find((node) => node.querySelector(".nav-fav-add-group"));
    const row = [...(section?.querySelectorAll(".nav-page:not(.nav-fav-group)") ?? [])].find((node) => node.textContent.replace("⭐", "").trim() === wanted);
    if (!row) return false;
    row.scrollIntoView({ block: "center" });
    const rect = row.getBoundingClientRect();
    const x = Math.round(rect.x + rect.width / 2), y = Math.round(rect.y + rect.height / 2);
    return row.contains(document.elementFromPoint(x, y)) ? { x, y } : false;
  }, name), { timeout: 15_000, interval: 150, timeoutMsg: `no clickable sidebar favorite ${JSON.stringify(name)}` });
  await browser.performActions([
    { type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [{ type: "pointerMove", origin: "viewport", ...point }, { type: "pointerDown", button: 0 }, { type: "pointerUp", button: 0 }] },
  ]);
  await browser.releaseActions();
  try {
    await browser.waitUntil(async () => (await currentPageTitle(browser)) === name
      && browser.execute((text) => (document.querySelector(".main-content .page-blocks")?.textContent ?? "").includes(text), body), { timeout: 15_000, interval: 150 });
  } catch {
    const shown = await browser.execute(() => document.querySelector(".main-content .page-blocks")?.textContent ?? "");
    throw new Error(`favorite ${JSON.stringify(name)} opened ${JSON.stringify(await currentPageTitle(browser))} showing ${JSON.stringify(shown.slice(0, 200))}; expected ${JSON.stringify(body)}`);
  }
  proof.steps.push(`the favorite ${name} opens its page with its body`);
}

/** Rename the routed page through its title (double-click, type, Enter). */
async function renameByTitle(to) {
  await browser.execute(() => {
    document.querySelector("h1.page-title")?.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true, view: window }));
  });
  await browser.$(".page-title-input").waitForExist({ timeout: 4000, timeoutMsg: "rename input never appeared" });
  await browser.execute((value) => {
    const input = document.querySelector(".page-title-input");
    input.focus();
    input.value = value;
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  }, to);
}

try {
  // 1. CLI rename with no window: `tine serve` owns the graph, nothing in
  // TypeScript runs. The backend's config step must move the favorite.
  log = fs.openSync(path.join(ARTIFACT, "serve.log"), "w");
  serve = spawn(APP, ["serve", GRAPH], { env, stdio: ["ignore", log, log] });
  for (let i = 0; ; i++) {
    try { if ((await cli(["status"])).owner === "serve") break; } catch {}
    assert(i < 50, "tine serve never owned the graph");
    await sleep(200);
  }
  await cli(["page", "rename", "Alpha", "Omega"]);
  await waitForFileText(path.join(GRAPH, "pages/Omega.md"), (text) => text === "- alpha body\n");
  const exited = new Promise((resolve) => serve.once("exit", resolve));
  serve.kill("SIGTERM");
  await exited;
  serve = null;
  await check("CLI rename", async () => {
    assert.deepEqual(storedFavorites(), ["Omega", "proj/child", "proj", "Titled"],
      `a CLI rename with the app closed left config.edn's favorites at ${JSON.stringify(storedFavorites())}`);
    proof.steps.push("CLI rename with no window: config.edn's favorite follows");
  });

  const driverLog = fs.openSync(path.join(ARTIFACT, "driver.log"), "w");
  driver = spawn(DRIVER, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", driverLog, driverLog], detached: true });
  fs.closeSync(driverLog);
  await sleep(1500);
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });
  await check("next open", async () => {
    await expectFavorites(["Omega", "proj/child", "proj", "Titled"], "next open after the CLI rename");
    await openFavorite("Omega", "alpha body");
  });

  // 2. App rename of a namespace parent: the child's favorite follows too.
  await openPageByName(browser, "proj");
  await renameByTitle("work");
  await waitForFileText(path.join(GRAPH, "pages/work___child.md"), (text) => text === "- child body\n");
  await check("namespace rename", async () => {
    await expectFavorites(["Omega", "work/child", "work", "Titled"], "app rename of a namespace parent");
    await openFavorite("work/child", "child body");
  });

  // 3. A title:: change of an open page (an outside editor here) renames it:
  // the backend moves its favorite (FAV-RETITLE).
  await openPageByName(browser, "Titled");
  fs.writeFileSync(path.join(GRAPH, "pages/Titled.md"), "title:: Retitled\n\n- titled body\n");
  await check("retitle", async () => {
    await expectFavorites(["Omega", "work/child", "work", "Retitled"], "a title:: change of the open page");
    await openFavorite("Retitled", "titled body");
  });

  const listed = fs.readdirSync(path.join(GRAPH, "pages")).sort();
  assert.deepEqual(listed, ["Omega.md", "Titled.md", "work.md", "work___child.md"], `opening favorites created or lost pages: ${listed}`);
  proof.steps.push("opening favorites created no page");
  if (proof.failures.length) throw new Error(`favorites journeys failed: ${proof.failures.join(" | ")}`);
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
  serve?.kill("SIGTERM");
  if (log !== undefined) try { fs.closeSync(log); } catch {}
  // Only processes launched against this journey's disposable graph.
  for (const pid of fs.readdirSync("/proc").filter((p) => /^\d+$/.test(p))) {
    try {
      if (fs.readFileSync(`/proc/${pid}/environ`, "utf8").split("\0").includes(`TINE_GRAPH=${GRAPH}`)
        && path.basename(fs.readlinkSync(`/proc/${pid}/exe`)) === path.basename(APP)) process.kill(Number(pid), "SIGKILL");
    } catch {}
  }
}
