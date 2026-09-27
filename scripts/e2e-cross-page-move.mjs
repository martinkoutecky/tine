// A real keyboard move across journal days, followed by undo, redo, quit and relaunch.
import { spawn } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { remote } from "webdriverio";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { openJournals } from "./lib/e2e-navigation.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const app = process.env.TINE_APP || path.join(root, "target/release/tine");
fs.mkdirSync(path.join(root, "test-results"), { recursive: true });
const artifacts = fs.mkdtempSync(path.join(root, "test-results", "e2e-cross-page-move-"));
const graph = path.join(artifacts, "graph");
const xdg = path.join(artifacts, "xdg");
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(xdg, dir), { recursive: true });
fs.writeFileSync(path.join(graph, "logseq", "config.edn"), '{:journal/page-title-format "yyyy-MM-dd"}\n');

const pad = (n) => String(n).padStart(2, "0");
const day = (offset) => { const now = new Date(); return new Date(now.getFullYear(), now.getMonth(), now.getDate() - offset, 12); };
const stem = (date) => `${date.getFullYear()}_${pad(date.getMonth() + 1)}_${pad(date.getDate())}`;
const title = (date) => `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
const newer = day(0), older = day(1);
const newerFile = path.join(graph, "journals", `${stem(newer)}.md`);
const olderFile = path.join(graph, "journals", `${stem(older)}.md`);
const movedText = "UNIQUE-CROSS-DAY-MOVE-TASK";
fs.writeFileSync(newerFile, "- newer keeper\n");
fs.writeFileSync(olderFile, `- ${movedText}\n- older keeper\n`);

let xvfb;
if (!process.env.DISPLAY) {
  const display = `:${200 + (process.pid % 500)}`;
  const log = fs.openSync(path.join(artifacts, "xvfb.log"), "w");
  xvfb = spawn("Xvfb", [display, "-screen", "0", "1400x1000x24", "-nolisten", "tcp"], { stdio: ["ignore", log, log] });
  process.env.DISPLAY = display;
  const socket = `/tmp/.X11-unix/X${display.slice(1)}`;
  await waitUntil(() => fs.existsSync(socket) && xvfb.exitCode === null, "Xvfb display", 10_000);
}

const driverPort = Number(process.env.E2E_DRIVER_PORT || 4644);
const nativePort = Number(process.env.E2E_NATIVE_PORT || 4645);
const env = {
  ...process.env, TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(xdg, "data"), XDG_CONFIG_HOME: path.join(xdg, "config"), XDG_CACHE_HOME: path.join(xdg, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
};
const driverLog = fs.openSync(path.join(artifacts, "tauri-driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(driverPort, nativePort, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"), {
  env, stdio: ["ignore", driverLog, driverLog], detached: true,
});
let browser;

async function waitUntil(check, label, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await check()) return;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`${label} did not become ready`);
}
async function portReady(port) {
  return new Promise((resolve) => {
    const socket = net.createConnection({ host: "127.0.0.1", port });
    socket.once("connect", () => { socket.destroy(); resolve(true); });
    socket.once("error", () => resolve(false));
  });
}
async function launch() {
  await waitUntil(() => portReady(driverPort), "tauri-driver", 15_000);
  const client = await remote({
    hostname: "127.0.0.1", port: driverPort, path: "/", logLevel: "error",
    connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: tauriCapabilities(app, "default", process.platform),
  });
  await client.$(".page-section").waitForExist({ timeout: 20_000 });
  await openJournals(client);
  return client;
}
async function renderedCopies(client, expectedDay) {
  return client.execute(({ expectedDay, movedText }) => {
    const pages = [...document.querySelectorAll(".page-section")].map((section) => ({
      title: section.querySelector("h1.page-title")?.textContent?.trim() ?? "",
      count: (section.querySelector(".page-blocks")?.textContent?.match(new RegExp(movedText, "g")) ?? []).length,
    }));
    return { pages, expected: pages.find((page) => page.title === expectedDay)?.count ?? 0, total: pages.reduce((sum, page) => sum + page.count, 0) };
  }, { expectedDay, movedText });
}
async function assertState(label, expectedDay, expectedFile) {
  await waitForFileText(expectedFile, (text) => text.includes(movedText), { timeoutMs: 20_000 });
  await waitForFileText(expectedFile === newerFile ? olderFile : newerFile, (text) => !text.includes(movedText), { timeoutMs: 20_000 });
  const allDisk = fs.readFileSync(newerFile, "utf8") + fs.readFileSync(olderFile, "utf8");
  if ((allDisk.match(new RegExp(movedText, "g")) ?? []).length !== 1) throw new Error(`${label}: disk has duplicate or missing block`);
  await browser.waitUntil(async () => { const state = await renderedCopies(browser, expectedDay); return state.expected === 1 && state.total === 1; }, {
    timeout: 15_000, interval: 100, timeoutMsg: `${label}: rendered journal ownership did not match disk`,
  });
  console.log(`PASS ${label}: ${expectedDay}, exactly once on disk and rendered`);
}

try {
  browser = await launch();
  await assertState("initial", title(older), olderFile);
  const blockId = await browser.execute((text) => {
    const block = [...document.querySelectorAll(".ls-block")].find((item) => item.querySelector(":scope > .block-main .block-content")?.textContent?.includes(text));
    return block?.getAttribute("data-block-id") ?? null;
  }, movedText);
  if (!blockId) throw new Error("source block is not rendered");
  await browser.$(`[data-block-id="${blockId}"] .block-content-wrapper`).click();
  await browser.keys("Escape");
  await browser.keys(["Alt", "Shift", "ArrowUp"]);
  await assertState("move", title(newer), newerFile);
  await browser.keys(["Control", "z"]);
  await assertState("undo", title(older), olderFile);
  await browser.keys(["Control", "Shift", "z"]);
  await assertState("redo", title(newer), newerFile);
  await browser.deleteSession();
  browser = await launch();
  await assertState("relaunch", title(newer), newerFile);
  console.log(`PASS cross-page move journey; artifacts ${artifacts}`);
} catch (error) {
  try { await browser?.saveScreenshot(path.join(artifacts, "failure.png")); } catch {}
  fs.writeFileSync(path.join(artifacts, "failure.txt"), `${String(error)}\n${error?.stack ?? ""}\n`);
  console.error(`FAIL cross-page move journey: ${error?.stack ?? error}; artifacts ${artifacts}`);
  process.exitCode = 1;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  xvfb?.kill("SIGKILL");
  fs.closeSync(driverLog);
}
