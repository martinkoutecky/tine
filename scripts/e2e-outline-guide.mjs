// Linux real-WebKit regression for GH #128: the narrow outline guide is a real
// pointer/keyboard control, follows OG's any-collapsed => expand-all semantics,
// persists one source edit, and leaves the guide parent expanded.
import { spawn, spawnSync } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4492);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4493);
const TMP = "/tmp/tine-outline-guide-e2e";
const GRAPH = `${TMP}/graph`;
const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || TMP;
const PAGE = `${GRAPH}/pages/Outline guide.md`;

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
fs.mkdirSync(ARTIFACTS, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
fs.writeFileSync(PAGE, [
  "- Root",
  "  - Child",
  "    collapsed:: true",
  "    - Grandchild",
  "      - Great grandchild",
  "  - Leaf",
  "",
].join("\n"));
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- open [[Outline guide]]\n");
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: `${TMP}/xdg/data`, XDG_CONFIG_HOME: `${TMP}/xdg/config`, XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
};
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
  for (const selector of ["a.page-ref=Outline guide", "span.page-ref=Outline guide", "*=Outline guide"]) {
    const link = await browser.$(selector);
    if (await link.isExisting()) { await link.click(); break; }
  }
  await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === "Outline guide", {
    timeout: 10_000, timeoutMsg: "outline guide page did not open",
  });
  const root = await browser.$(".page-view > .ls-block, .page-blocks > .ls-block, .ls-block");
  const guide = await root.$(":scope > .block-children-container > button.block-children-left-border");
  await guide.waitForExist({ timeout: 10_000 });
  if ((await guide.getAttribute("aria-label")) !== "Expand all descendants") {
    throw new Error("guide did not expose its initial expand action accessibly");
  }

  // The child starts folded, so the root guide must expand every model descendant.
  await guide.click();
  await browser.waitUntil(async () => (await root.getText()).includes("Great grandchild"), {
    timeout: 10_000, timeoutMsg: "partially folded subtree did not expand completely",
  });
  if ((await guide.getAttribute("aria-label")) !== "Collapse all descendants") {
    throw new Error("guide did not expose its new collapse action accessibly");
  }

  // Keyboard activation collapses all descendants while Root/Child/Leaf remain visible.
  await guide.click(); // collapse by pointer once, to exercise the real narrow hit target
  await browser.waitUntil(async () => !(await root.getText()).includes("Grandchild"), { timeout: 10_000 });
  const collapsedText = await root.getText();
  if (!collapsedText.includes("Root") || !collapsedText.includes("Child") || !collapsedText.includes("Leaf")) {
    throw new Error(`guide parent/direct children disappeared: ${collapsedText}`);
  }
  await browser.execute((element) => element.focus(), guide);
  await browser.keys(["Enter"]);
  await browser.waitUntil(async () => (await root.getText()).includes("Great grandchild"), {
    timeout: 10_000, timeoutMsg: "keyboard activation did not expand descendants",
  });

  // Collapse once more and wait for the normal debounced page save. Both
  // collapsible descendants persist; the root never gets collapsed::.
  await guide.click();
  await browser.waitUntil(() => {
    const disk = fs.readFileSync(PAGE, "utf8");
    const collapsedCount = (disk.match(/collapsed:: true/g) || []).length;
    return collapsedCount === 2 && !/^- Root\n\s+collapsed:: true/m.test(disk);
  }, { timeout: 10_000, interval: 100, timeoutMsg: "collapse transaction did not persist the expected descendants" });
  // WebKitWebDriver's screenshot endpoint can hang after reactive DOM removal;
  // capture the actual Xvfb root instead so visual evidence never changes the
  // behavioral result being tested.
  try {
    const shot = spawnSync("import", ["-window", "root", path.join(ARTIFACTS, "outline-guide.png")], { env });
    if (shot.status !== 0) console.warn(`WARN: Xvfb outline-guide screenshot failed: ${shot.stderr?.toString()}`);
  } catch (error) {
    console.warn(`WARN: Xvfb outline-guide screenshot failed: ${error}`);
  }
  console.log("PASS: outline guide pointer, keyboard, partial-tree, and persistence behavior");
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
