// Linux real-app proof for GH #208: in-page Find's Replace all on a routed named
// page edits only the visible text, persists through the normal save path, and
// is one undo step. The search word also sits inside a hidden `id::` property
// value and inside a `[label](url)` link address; both must survive byte-exact,
// because Find matches rendered text and Replace edits only what Find matched.
import { spawn } from "node:child_process";
import net from "node:net";
import { remote } from "webdriverio";
import { setTimeout as delay } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { APP_ID } from "./lib/app-identity.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4624);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4625);
const TMP = "/tmp/tine-find-replace-e2e";
const GRAPH = `${TMP}/graph`;
const PAGE_NAME = "Find Replace Test";
const PAGE = `${GRAPH}/pages/${PAGE_NAME}.md`;
const MOD = process.platform === "darwin" ? "Meta" : "Control";

// "cafe" is all hex digits, so it can sit inside the block uuid.
const ORIGINAL = [
  "- Meet at the cafe at noon",
  "  id:: cafe1111-1111-4111-8111-111111111111",
  "- Read the [menu](https://example.com/cafe/menu) before the cafe opens",
  "",
].join("\n");
const REPLACED = [
  "- Meet at the bistro at noon",
  "  id:: cafe1111-1111-4111-8111-111111111111",
  "- Read the [menu](https://example.com/cafe/menu) before the bistro opens",
  "",
].join("\n");

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
fs.writeFileSync(PAGE, ORIGINAL);
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, `- find and replace probe [[${PAGE_NAME}]]\n`);

for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
const appData = `${TMP}/xdg/data/${APP_ID}`;
fs.mkdirSync(appData, { recursive: true });
fs.writeFileSync(`${appData}/tine-settings.json`, JSON.stringify({ last_graph_path: fs.realpathSync(GRAPH) }, null, 2));

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
const log = fs.openSync(`${TMP}/tauri-driver.log`, "w");
const td = spawn(TD, ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], {
  env, stdio: ["ignore", log, log], detached: true,
});

/** Poll until tauri-driver accepts connections (a readiness predicate, not a fixed wait). */
async function waitForPort(port, timeoutMs = 15_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const open = await new Promise((resolve) => {
      const socket = net.connect({ host: "127.0.0.1", port }, () => { socket.destroy(); resolve(true); });
      socket.on("error", () => resolve(false));
    });
    if (open) return;
    await delay(100);
  }
  throw new Error(`tauri-driver did not listen on ${port}; see ${TMP}/tauri-driver.log`);
}

let browser;
const pageText = () => browser.execute(() =>
  [...document.querySelectorAll(".page-blocks .ls-block .block-content, .ls-block .block-content")]
    .map((node) => node.textContent ?? "").join("\n"));
const findState = () => browser.execute(() => ({
  open: Boolean(document.querySelector(".inpage-find-bar")),
  count: document.querySelector(".inpage-find-count")?.textContent?.trim() ?? "",
  replaceAllEnabled: [...document.querySelectorAll(".inpage-replace-btn")]
    .some((button) => button.textContent?.trim() === "Replace all" && !button.disabled),
}));

try {
  await waitForPort(DRIVER_PORT);
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER_PORT, path: "/", logLevel: "error",
    connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
  });
  await browser.$(".ls-block").waitForExist({ timeout: 20_000 });
  await openPageByName(browser, PAGE_NAME);
  await browser.waitUntil(async () => (await pageText()).includes("before the cafe opens"), {
    timeout: 10_000, interval: 100, timeoutMsg: "the named page's blocks did not render",
  });
  // Nothing is in edit mode: Mod+F belongs to the page, not to a block editor.
  await browser.execute(() => { if (document.activeElement instanceof HTMLElement) document.activeElement.blur(); });

  await browser.keys([MOD, "f"]);
  const findInput = await browser.$(".inpage-find-input");
  await findInput.waitForExist({ timeout: 5_000 });
  await findInput.setValue("cafe");
  // Two visible matches; the uuid and the link address are not rendered text.
  await browser.waitUntil(async () => (await findState()).count === "1 / 2", {
    timeout: 5_000, interval: 100, timeoutMsg: async () => `find count never became "1 / 2": ${JSON.stringify(await findState())}`,
  });

  await browser.$(".inpage-replace-toggle").click();
  const replaceInput = await browser.$(".inpage-replace-input");
  await replaceInput.waitForExist({ timeout: 5_000 });
  await replaceInput.setValue("bistro");
  await browser.waitUntil(async () => (await findState()).replaceAllEnabled, {
    timeout: 5_000, interval: 100, timeoutMsg: "Replace all never became enabled",
  });
  const clicked = await browser.execute(() => {
    const button = [...document.querySelectorAll(".inpage-replace-btn")].find((node) => node.textContent?.trim() === "Replace all");
    if (!(button instanceof HTMLButtonElement) || button.disabled) return false;
    button.click();
    return true;
  });
  if (!clicked) throw new Error("Replace all button was not clickable");

  const saved = await waitForFileText(PAGE, (text) => text === REPLACED, { timeoutMs: 15_000 });
  if (!saved.includes("id:: cafe1111-1111-4111-8111-111111111111")) throw new Error("the id:: property line changed");
  if (!saved.includes("[menu](https://example.com/cafe/menu)")) throw new Error("the link address changed");
  await browser.waitUntil(async () => {
    const text = await pageText();
    return text.includes("before the bistro opens") && !text.includes("cafe");
  }, { timeout: 5_000, interval: 100, timeoutMsg: "the page did not show the replaced text" });

  // Close Find (focus leaves its input) and press undo once.
  await browser.keys(["Escape"]);
  await browser.waitUntil(async () => !(await findState()).open, {
    timeout: 5_000, interval: 100, timeoutMsg: "Escape did not close Find",
  });
  await browser.keys([MOD, "z"]);
  await waitForFileText(PAGE, (text) => text === ORIGINAL, { timeoutMs: 15_000 });
  await browser.waitUntil(async () => (await pageText()).includes("before the cafe opens"), {
    timeout: 5_000, interval: 100, timeoutMsg: "the page did not show the undone text",
  });
  console.log("PASS: Replace all on a routed named page saved only the visible matches (id:: and link address intact), and one undo restored the original file");
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
