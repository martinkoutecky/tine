// GH #623 comment 17: the image hover "Trash" must drop the reference durably
// and then move the file to the recoverable trash. The page file is read from
// disk and the native confirmation is answered the way a user does (xdotool).
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { remote } from "webdriverio";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { answerNativeDialog } from "./lib/e2e-native-dialog.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
const DRIVER = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin/tauri-driver") : "tauri-driver");
const TMP = fs.mkdtempSync("/tmp/tine-gh623-trash-");
const GRAPH = path.join(TMP, "graph");
const ARTIFACT = process.env.E2E_ARTIFACT_DIR || path.join(TMP, "artifacts");
const port = Number(process.env.E2E_DRIVER_PORT || 4694);
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, dir));
fs.mkdirSync(ARTIFACT, { recursive: true });
// 1x1 PNG
const PNG = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==", "base64");
fs.writeFileSync(path.join(GRAPH, "assets/x.png"), PNG);
fs.writeFileSync(path.join(GRAPH, "assets/y.png"), PNG);
fs.writeFileSync(path.join(GRAPH, "pages/Photos.md"), "- solo ![solo](../assets/x.png)\n- shared ![shared](../assets/y.png)\n");
fs.writeFileSync(path.join(GRAPH, "pages/Gallery.md"), "- also ![again](../assets/y.png)\n");
const env = {
  ...process.env, TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "data"), XDG_CONFIG_HOME: path.join(TMP, "config"), XDG_CACHE_HOME: path.join(TMP, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
};
const log = fs.openSync(path.join(ARTIFACT, "driver.log"), "w");
const driver = spawn(DRIVER, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
const proof = { app: APP, graph: GRAPH, steps: [] };
const read = (rel) => fs.readFileSync(path.join(GRAPH, rel), "utf8");

async function trashImage(alt) {
  await browser.waitUntil(() => browser.execute((label) => !!document.querySelector(`img.inline-image[alt="${label}"]`), alt), { timeout: 15_000, timeoutMsg: `image ${alt} never rendered` });
  await browser.execute((label) => {
    document.querySelector(`img.inline-image[alt="${label}"]`).closest(".inline-image-wrap").querySelector(".asset-action-trash").click();
  }, alt);
  await answerNativeDialog("yes", { env });
}
const toastTexts = () => browser.execute(() => [...document.querySelectorAll(".toast-msg")].map((node) => node.textContent ?? ""));

try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });
  await openPageByName(browser, "Photos");
  await trashImage("solo");
  await browser.waitUntil(async () => (await toastTexts()).some((text) => text.includes("moved to trash")), { timeout: 15_000, timeoutMsg: `no success toast; saw ${JSON.stringify(await toastTexts())}` });
  if ((await toastTexts()).some((text) => text.includes("Couldn't") || text.includes("referenced"))) throw new Error(`failure toast: ${JSON.stringify(await toastTexts())}`);
  if (fs.existsSync(path.join(GRAPH, "assets/x.png"))) throw new Error("x.png was not moved out of assets/");
  const trashDir = path.join(GRAPH, "logseq/.tine-trash/assets");
  if (!(fs.existsSync(trashDir) && fs.readdirSync(trashDir).some((entry) => entry.endsWith("x.png")))) throw new Error("x.png is not in the recoverable trash");
  if (read("pages/Photos.md").includes("x.png")) throw new Error("Photos.md still references x.png");
  proof.steps.push("a file nobody else uses is saved away from the block, then trashed (no 'asset is referenced' failure)");

  await trashImage("shared");
  await browser.waitUntil(async () => (await toastTexts()).some((text) => text.includes("still used elsewhere")), { timeout: 15_000, timeoutMsg: `no kept-file notice; saw ${JSON.stringify(await toastTexts())}` });
  if (!fs.existsSync(path.join(GRAPH, "assets/y.png"))) throw new Error("y.png is still used by Gallery but was trashed");
  if (read("pages/Photos.md").includes("y.png")) throw new Error("Photos.md lost the edit: still references y.png");
  if (read("pages/Gallery.md") !== "- also ![again](../assets/y.png)\n") throw new Error("Gallery.md changed");
  proof.steps.push("a file another page still uses is kept; the block edit stands; the user is told");
  await browser.saveScreenshot(path.join(ARTIFACT, "trash.png"));
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
