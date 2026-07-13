// Linux real-WebKit regression for GH #98/#99/#69: authoritative evidence in
// Ctrl+K, a graph-scoped virtual query tab that survives restart, friendly
// filters/explanation, and guarded materialization as one ordinary query page.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_BASE = Number(process.env.E2E_DRIVER_PORT || 4492);
const NATIVE_BASE = Number(process.env.E2E_NATIVE_PORT || 4493);
const TMP = "/tmp/tine-query-workspace-e2e";
const GRAPH = `${TMP}/graph`;
const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || `${TMP}/artifacts`;

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.mkdirSync(ARTIFACTS, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
fs.writeFileSync(`${GRAPH}/pages/Research.md`, [
  "- Evidence parent",
  "  - alpha starts here, then enough context separates the terms so the excerpt must preserve both useful windows without showing the entire block; beta finishes here",
  "- alpha beta draft must be excluded",
  "- alpha repeats alpha and beta remains visible",
  "",
].join("\n"));
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- Open [[Research]]\n");

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
    await browser.$(".query-workspace, .ls-block, .page-title").waitForExist({ timeout: 20_000 });
    await fn(browser);
    await sleep(750);
  } finally {
    try { await browser?.deleteSession(); } catch {}
    try { process.kill(-td.pid, "SIGKILL"); } catch {}
    fs.closeSync(log);
  }
}

async function presentationButton(browser, label) {
  for (const button of await browser.$$(".query-presentations button")) {
    if ((await button.getText()).trim() === label) return button;
  }
  throw new Error(`missing ${label} presentation button`);
}

await withApp(0, async (browser) => {
  await browser.keys(["Control", "k"]);
  const input = await browser.$(".switcher-input");
  await input.waitForExist({ timeout: 5_000 });
  await input.setValue("alpha beta -draft");
  await browser.waitUntil(async () => (await browser.$$(".switcher-row.block-result")).length === 2, {
    timeout: 10_000, timeoutMsg: "friendly search did not return exactly the two included blocks",
  });

  const proof = await browser.execute(() => {
    const input = document.querySelector(".switcher-input");
    const rows = [...document.querySelectorAll(".switcher-row.block-result")];
    return {
      role: input?.getAttribute("role"),
      active: input?.getAttribute("aria-activedescendant"),
      rows: rows.map((row) => ({
        context: row.querySelector(".search-result-context")?.textContent,
        excerpt: row.querySelector(".search-result-excerpt")?.textContent,
        marks: [...row.querySelectorAll("mark")].map((mark) => mark.textContent?.toLowerCase()),
        height: row.getBoundingClientRect().height,
      })),
    };
  });
  if (proof.role !== "combobox" || !proof.active) throw new Error(`missing accessible combobox state: ${JSON.stringify(proof)}`);
  if (proof.rows.some((row) => !row.context?.includes("Research") || row.excerpt?.includes("draft") || row.height > 90)) {
    throw new Error(`result context/excerpt bounds are wrong: ${JSON.stringify(proof)}`);
  }
  if (proof.rows.some((row) => !row.marks.includes("alpha") || !row.marks.includes("beta"))) {
    throw new Error(`authoritative evidence did not highlight all positive terms: ${JSON.stringify(proof)}`);
  }
  const before = proof.active;
  await browser.keys(["ArrowDown"]);
  const after = await input.getAttribute("aria-activedescendant");
  if (!after || after === before) throw new Error("ArrowDown did not update aria-activedescendant");

  await browser.$("[data-open-search-tab]").click();
  await browser.$(".query-workspace").waitForExist({ timeout: 10_000 });
  const source = await browser.$(".query-workspace-source");
  if ((await source.getValue()) !== "alpha beta -draft") throw new Error("query workspace lost the Ctrl+K source");
  await browser.$(".query-explain-toggle").click();
  await browser.$(".query-workspace-explanation").waitForExist({ timeout: 10_000 });
  await browser.$(".query-advanced-toggle").click();
  const dialog = await browser.$(".query-advanced-modal");
  await dialog.waitForExist({ timeout: 5_000 });
  if (!(await dialog.getText()).includes("All of these words")) throw new Error("friendly advanced fields are missing");
  await browser.keys(["Escape"]);
  await browser.$(".query-advanced-modal").waitForExist({ reverse: true, timeout: 5_000 });
  const table = await presentationButton(browser, "Table");
  await table.click();
  await browser.$(".query-results-table").waitForExist({ timeout: 5_000 });
  await browser.saveScreenshot(path.join(ARTIFACTS, "query-workspace.png"));
});

// A fresh native process restores the virtual route and its presentation from
// the graph-scoped device-local session; no temporary page has been written.
await withApp(1, async (browser) => {
  await browser.$(".query-workspace").waitForExist({ timeout: 20_000 });
  const source = await browser.$(".query-workspace-source");
  if ((await source.getValue()) !== "alpha beta -draft") throw new Error("restart lost the virtual query source");
  const restoredTable = await presentationButton(browser, "Table");
  if ((await restoredTable.getAttribute("aria-pressed")) !== "true") {
    throw new Error("restart lost the query presentation");
  }
  const graphFilesBefore = fs.readdirSync(`${GRAPH}/pages`).sort();
  if (graphFilesBefore.length !== 1 || graphFilesBefore[0] !== "Research.md") {
    throw new Error(`virtual query wrote a temporary page: ${graphFilesBefore.join(", ")}`);
  }

  const title = await browser.$(".query-workspace-save input");
  await title.setValue("Saved evidence search");
  await browser.$(".query-workspace-save button[type=submit]").click();
  await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === "Saved evidence search", {
    timeout: 10_000, timeoutMsg: "naming did not replace the virtual route with the saved page",
  });
  await sleep(1000);
  const saved = fs.readFileSync(`${GRAPH}/pages/Saved evidence search.md`, "utf8");
  if (!saved.includes('{{query (search "alpha beta -draft")}}') || !saved.includes("tine.view:: table")) {
    throw new Error(`materialized page is not the canonical one-block query:\n${saved}`);
  }
});

console.log("PASS: typed search evidence, persistent virtual query workspace, explanation, and guarded save work in WebKit");
