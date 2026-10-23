// Internal card 3x4: session navigation wins over :default-home at startup.
import { spawn, execFileSync } from "node:child_process";
import { remote } from "webdriverio";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as sleep } from "node:timers/promises";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName, openPageByLink, currentPageTitle } from "./lib/e2e-navigation.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const RUN = path.join(process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "artifacts/tab-restore"), `run-${Date.now()}`);
const GRAPH = path.join(RUN, "graph");
for (const dir of ["graph/pages", "graph/journals", "graph/logseq", "home", "data", "config", "cache"]) fs.mkdirSync(path.join(RUN, dir), { recursive: true });
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), '{:default-home {:page "Index"} :favorites ["Sidebar 2026"]}\n');
const pages = {
  Index: "- Index body\n",
  Tasks: "- [[Paralg 2026]] [[Control 2026]] [[Command 2026]] [[Context 2026]]\n",
  "Paralg 2026": "- Original course body [[Courses/Paralg 2026]]\n",
  "Courses___Paralg 2026": "title:: Courses/Paralg 2026\n\n- Continued course body\n",
  "Control 2026": "- Control body\n", "Command 2026": "- Command body\n",
  "Context 2026": "- Context body\n", "Sidebar 2026": "- Sidebar body\n",
};
for (const [name, text] of Object.entries(pages)) fs.writeFileSync(path.join(GRAPH, `pages/${name}.md`), text);
const env = { ...process.env, HOME: path.join(RUN, "home"), TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(RUN, "data"), XDG_CONFIG_HOME: path.join(RUN, "config"), XDG_CACHE_HOME: path.join(RUN, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const tested = execFileSync("git", ["rev-parse", "HEAD"], { cwd: ROOT, encoding: "utf8" }).trim();
let stage = "initial launch";
function mark(next) {
  stage = next;
  fs.writeFileSync(path.join(RUN, "stage.txt"), `${stage}\n`);
  console.log(stage);
}
function shot(name) {
  // WebKitWebDriver's screenshot endpoint can hang even when the visible app
  // and script commands work. Capture the isolated native display instead.
  execFileSync("import", ["-window", "root", path.join(RUN, name)], { env });
}
const proof = { tested, app: APP, run: RUN, steps: [] };
function savedSessions(dir = path.join(RUN, "data")) {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(dir, entry.name);
    if (entry.isDirectory()) return savedSessions(file);
    if (!entry.name.endsWith(".json") || entry.name.endsWith("-workspaces.json")) return [];
    try {
      const session = JSON.parse(fs.readFileSync(file, "utf8"));
      return session.tabs?.some((tab) => tab.history?.some((route) => route.name === "Tasks")) ? [{ file, session }] : [];
    } catch { return []; }
  });
}
const state = (browser) => browser.execute(() => [...document.querySelectorAll(".tab-strip-scroll .tab[data-tab-id]")].map((tab) => ({
  title: tab.querySelector(".tab-title")?.textContent.trim(), pinned: tab.classList.contains("pinned"), active: tab.classList.contains("active"),
})));
async function title(browser, name) {
  await browser.waitUntil(async () => await currentPageTitle(browser) === name, { timeout: 20_000, timeoutMsg: `route never became ${name}` });
}
async function activate(browser, name) {
  await browser.execute((name) => [...document.querySelectorAll(".tab-strip-scroll .tab")].find((tab) => tab.querySelector(".tab-title")?.textContent.trim() === name)?.click(), name);
  await title(browser, name);
}
async function pinActive(browser) {
  await (await browser.$(".tab-strip-scroll .tab.active")).doubleClick();
  await browser.waitUntil(async () => (await state(browser)).find((tab) => tab.active)?.pinned, { timeout: 5000 });
}
async function link(browser, name) {
  const links = await browser.$$(".main-content .page-ref");
  for (const candidate of links) if ((await candidate.getText()).replaceAll("[", "").replaceAll("]", "").trim() === name) return candidate;
  throw new Error(`missing page link ${name}`);
}
async function nativeClick(browser, element, button) {
  const point = await browser.execute((element) => {
    const rect = element.getBoundingClientRect();
    return { x: Math.round(screenX + rect.left + rect.width / 2), y: Math.round(screenY + rect.top + rect.height / 2) };
  }, element);
  // WebKitWebDriver delivered button=1 down followed by button=0 up/click.
  // Native X11 input preserves the actual completed middle-button gesture.
  execFileSync(process.env.E2E_XDOTOOL || "xdotool", ["mousemove", "--sync", String(point.x), String(point.y), "click", String(button)], { env });
}
async function background(browser, name, method) {
  mark(`open ${name} via ${method}`);
  const before = (await state(browser)).length;
  const target = await link(browser, name);
  if (method === "middle") {
    await browser.execute(() => {
      globalThis.__tabRestoreGesture = [];
      for (const type of ["mousedown", "mouseup", "click", "auxclick"]) document.addEventListener(type, (event) => {
        globalThis.__tabRestoreGesture.push({ type, button: event.button, text: event.target.textContent });
      }, { capture: true, once: true });
    });
    await nativeClick(browser, target, 2);
    proof.middleGesture = await browser.execute(() => globalThis.__tabRestoreGesture);
    if (!proof.middleGesture.some((event) => event.type === "mousedown" && event.button === 1)) throw new Error(`driver did not deliver middle button: ${JSON.stringify(proof.middleGesture)}`);
  }
  else if (method === "context") {
    await nativeClick(browser, target, 3);
    await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll('[role="menuitem"]')].some((item) => item.textContent.includes("Open in new tab"))), { timeout: 5000 });
    await browser.execute(() => [...document.querySelectorAll('[role="menuitem"]')].find((item) => item.textContent.includes("Open in new tab"))?.click());
  } else {
    // Exercise both platform modifier interpretations in the native WebView.
    await browser.execute((name, modifier) => {
      const target = [...document.querySelectorAll(".main-content .page-ref")].find((link) => link.textContent.includes(name));
      target.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, [modifier]: true }));
    }, name, method === "control" ? "ctrlKey" : "metaKey");
  }
  await browser.waitUntil(async () => (await state(browser)).length === before + 1, { timeout: 5000, timeoutMsg: `${method} did not open a tab` });
  if (method !== "middle") await title(browser, "Tasks");
}

async function withApp(index, action) {
  const port = Number(process.env.E2E_DRIVER_PORT || 4712) + index * 2;
  const log = fs.openSync(path.join(RUN, `driver-${index}.log`), "w");
  const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"), { env, stdio: ["ignore", log, log], detached: true });
  let browser;
  try {
    await sleep(1500); // driver startup only; domain readiness uses predicates.
    browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 1,
      connectionRetryTimeout: 30_000,
      capabilities: tauriCapabilities(APP, `tab-restore-${index}`) });
    mark(`native process ${index} ready`);
    await ensureMainWindow(browser);
    await browser.setWindowSize(1280, 900);
    await browser.$(".ls-block, .page-title").waitForExist({ timeout: 30_000 });
    await action(browser);
    // The app's actual close request flushes the session, including a pending
    // debounce. WebDriver deletion alone would kill the app without safe-close.
    mark(`clean close ${index}`);
    await browser.execute(() => document.querySelector(".win-close")?.click());
    await browser.waitUntil(async () => {
      try { await browser.execute(() => document.title); return false; } catch { return true; }
    }, { timeout: 20_000, timeoutMsg: "app did not exit after close" });
  } catch (error) {
    try { shot(`failure-${index}.png`); } catch {}
    const observed = browser ? await state(browser).catch(() => null) : null;
    fs.writeFileSync(path.join(RUN, "failure.json"), JSON.stringify({ tested, journey: "tab-restore", stage,
      expected: "Every saved tab retains its route, order, pins and active state after restart", observed, error: String(error), classification: "ambiguous; inspect stage and driver log" }, null, 2));
    throw error;
  } finally {
    try { await browser?.deleteSession(); } catch {}
    try { process.kill(-driver.pid, "SIGKILL"); } catch {}
    fs.closeSync(log);
  }
}

let expected;
await withApp(0, async (browser) => {
  await title(browser, "Index");
  await pinActive(browser);
  await openPageByName(browser, "Tasks");
  await pinActive(browser);
  await background(browser, "Paralg 2026", "middle");
  await activate(browser, "Paralg 2026");
  expected = await state(browser);
  proof.steps.push({ stage: "literal middle-click case before quit", tabs: expected });
});
proof.savedAfterFirstClose = savedSessions();
fs.writeFileSync(path.join(RUN, "before-relaunch.json"), JSON.stringify(proof, null, 2));
fs.writeFileSync(path.join(GRAPH, "pages/Paralg 2026.md"), "- Externally changed between runs [[Courses/Paralg 2026]]\n");
await withApp(1, async (browser) => {
  mark("literal middle-click restore");
  await title(browser, "Paralg 2026");
  await browser.waitUntil(() => browser.execute(() => document.querySelector(".main-content")?.textContent.includes("Externally changed between runs")), {
    timeout: 15_000, timeoutMsg: "restarted course did not display its externally changed body",
  });
  if (JSON.stringify(await state(browser)) !== JSON.stringify(expected)) throw new Error("literal restart changed tabs");
  shot("restored-course.png");
  await openPageByLink(browser, "Courses/Paralg 2026");
  await activate(browser, "Tasks");
  await background(browser, "Control 2026", "control");
  await background(browser, "Command 2026", "command");
  await background(browser, "Context 2026", "context");
  mark("sidebar new tab");
  await browser.execute(() => {
    const favorite = [...document.querySelectorAll(".left-sidebar .nav-page")].find((item) => item.textContent.includes("Sidebar 2026"));
    if (!favorite) throw new Error("sidebar favorite missing");
    favorite.dispatchEvent(new MouseEvent("auxclick", { bubbles: true, cancelable: true, button: 1 }));
  });
  await browser.waitUntil(async () => (await state(browser)).some((tab) => tab.title === "Sidebar 2026"), { timeout: 5000 });
  // Quit with the middle-clicked tab inactive; restore must keep Tasks active.
  expected = await state(browser);
  proof.steps.push({ stage: "sibling entry points and continued navigation before quit", tabs: expected });
});
await withApp(2, async (browser) => {
  mark("restore sibling entry points");
  await title(browser, "Tasks");
  if (JSON.stringify(await state(browser)) !== JSON.stringify(expected)) throw new Error(`restart changed tabs: ${JSON.stringify(await state(browser))}`);
  shot("restored-siblings.png");
  for (const name of expected.map((tab) => tab.title)) await activate(browser, name);
  proof.steps.push({ stage: "all restored tabs render their saved pages", tabs: await state(browser) });
});
// Browsing and app state never write tab metadata into any source page.
for (const [name, text] of Object.entries(pages)) {
  const expectedText = name === "Paralg 2026" ? "- Externally changed between runs [[Courses/Paralg 2026]]\n" : text;
  if (fs.readFileSync(path.join(GRAPH, `pages/${name}.md`), "utf8") !== expectedText) throw new Error(`navigation changed graph page ${name}`);
}
fs.writeFileSync(path.join(RUN, "proof.json"), JSON.stringify(proof, null, 2));
console.log(`PASS: tab routes, history navigation, pins, order and focus survive native restart (${RUN})`);
