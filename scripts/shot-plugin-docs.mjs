// Capture real mock-app screenshots while exercising all launch plugins.
// Usage: build the plugin WASMs, then run this script (it builds the mock app itself).
import { chromium } from "playwright";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { build } from "vite";
import { setTimeout as sleep } from "node:timers/promises";
import { editorSelector, editorState, editorText } from "./lib/e2e-block-editor.mjs";
import {
  resolveLaunchPluginRoots,
  resolveRepositoryRoot,
  resolveViteExecutable,
} from "./docs-preview-paths.mjs";

const PORT = 5197;
const ROOT = resolveRepositoryRoot(import.meta.url);
const VITE = resolveViteExecutable(ROOT);
const {
  query: QUERY_ROOT,
  bullet: BULLET_ROOT,
  heading: HEADING_ROOT,
} = resolveLaunchPluginRoots(ROOT);
const SCREENSHOT_ROOT = process.env.TINE_PLUGIN_SCREENSHOT_ROOT;
const PLATFORM = process.env.TINE_PLUGIN_PLATFORM ?? "desktop";

function screenshotDir(pluginRoot, slug) {
  return SCREENSHOT_ROOT ? `${SCREENSHOT_ROOT}/${slug}` : `${pluginRoot}/docs`;
}

const BULLET_SHOTS = screenshotDir(BULLET_ROOT, "bullet-threading");
const QUERY_SHOTS = screenshotDir(QUERY_ROOT, "query-filter");
const HEADING_SHOTS = screenshotDir(HEADING_ROOT, "heading-level-shortcuts");

// The browser core answers the tasks query from the `?plugintasks` fixture page
// (src/mock/flags/plugintasks/), while the real view toggle, sheet filter and
// plugin WASM run unchanged. Build the mock app into this harness's temporary
// directory, never into the shipped app.
const previewDir = mkdtempSync(join(tmpdir(), "tine-plugin-preview-"));
try {
  await build({ root: ROOT, mode: "mock", build: { outDir: previewDir, emptyOutDir: true } });
} catch (error) {
  rmSync(previewDir, { recursive: true, force: true });
  throw error;
}

let browser;
let serverSpawnError;
const server = spawn(VITE, ["preview", "--outDir", previewDir, "--host", "127.0.0.1", "--port", String(PORT), "--strictPort"], {
  stdio: "ignore",
});
server.once("error", (error) => { serverSpawnError = error; });

async function waitForServer(url) {
  for (let attempt = 0; attempt < 40; attempt += 1) {
    if (serverSpawnError) throw serverSpawnError;
    try {
      if ((await fetch(url)).ok) return;
    } catch {
      // Preview is still starting.
    }
    await sleep(250);
  }
  if (serverSpawnError) throw serverSpawnError;
  throw new Error("preview server did not start");
}

async function openPlugins(page) {
  await page.getByTitle("Settings (t s)").click();
  // Phone Settings lists categories with a CSS chevron in the accessible name.
  await page.locator('.settings-nav-item[data-settings-tab="plugins"]').click();
}

async function dismissInitialMobileDrawer(page) {
  if (PLATFORM === "desktop") return;

  const app = page.locator(".app-container");
  await app.waitFor();
  await page.waitForFunction(() =>
    document.querySelector(".app-container")?.getAttribute("data-mobile-drawer-mode") === "true"
  );

  const activeDrawer = await app.getAttribute("data-active-drawer");
  if (!activeDrawer) return;
  if (activeDrawer !== "left") {
    throw new Error(`unexpected initial ${activeDrawer} mobile drawer`);
  }

  await page.locator('[data-mobile-drawer-panel="left"]').waitFor({ state: "visible" });
  await page.getByRole("button", { name: "Close navigation sidebar", exact: true }).click();
  await page.waitForFunction(() => {
    const container = document.querySelector(".app-container");
    return container?.getAttribute("data-active-drawer") === ""
      && !document.querySelector("[data-mobile-drawer-scrim]");
  });
}

async function installLocal(page, root, wasmName) {
  const manifest = JSON.parse(await readFile(`${root}/manifest.json`, "utf8"));
  await page.locator('input[type="file"]').setInputFiles([
    `${root}/manifest.json`,
    `${root}/target/wasm32-unknown-unknown/release/${wasmName}`,
  ]);
  await page.locator(".toast-msg", { hasText: `${manifest.name} ${manifest.version} installed disabled` }).waitFor({ timeout: 10_000 });
  const installed = page.locator(".plugin-detail-page", { hasText: manifest.id });
  await installed.waitFor();
  await installed.locator(".settings-toggle").click();
  await page.waitForFunction(
    (id) => [...document.querySelectorAll(".plugin-detail-page")]
      .find((element) => element.textContent?.includes(id))
      ?.querySelector(".settings-toggle")
      ?.getAttribute("aria-checked") === "true",
    manifest.id
  );
  return manifest;
}

async function navigate(page, name) {
  await page.keyboard.press("Control+k");
  await page.locator(".switcher-input").fill(name);
  await sleep(350);
  await page.locator(".switcher-row").first().click();
  await page.getByText(name, { exact: true }).first().waitFor();
  await sleep(300);
}

try {
  if (!["desktop", "android", "ios"].includes(PLATFORM)) throw new Error(`unsupported test platform ${PLATFORM}`);
  const url = `http://127.0.0.1:${PORT}/?plugintasks${PLATFORM === "desktop" ? "" : `&platform=${PLATFORM}`}`;
  await waitForServer(url);
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  const page = await browser.newPage({
    viewport: PLATFORM === "desktop" ? { width: 1180, height: 820 } : { width: 390, height: 844 },
    deviceScaleFactor: 2,
    ...(PLATFORM === "desktop" ? {} : {
      userAgent: "Mozilla/5.0 (Linux; Android 14; Tine smoke) AppleWebKit/537.36 Chrome/126 Mobile Safari/537.36",
      isMobile: true,
      hasTouch: true,
    }),
  });
  page.setDefaultTimeout(8_000);
  // Adapt the shared editor inspection helper's variadic WebDriver call to Playwright.
  const editorBrowser = {
    execute: (fn, ...args) => page.evaluate(`(${fn.toString()})(...${JSON.stringify(args)})`),
  };
  const errors = [];
  page.on("pageerror", (error) => errors.push(String(error)));
  page.on("console", (message) => message.type() === "error" && errors.push(`console: ${message.text()}`));

  await page.goto(url);
  await page.waitForSelector(".page-title");
  await dismissInitialMobileDrawer(page);
  for (const root of [BULLET_SHOTS, QUERY_SHOTS, HEADING_SHOTS]) mkdirSync(root, { recursive: true });
  await openPlugins(page);
  await installLocal(page, BULLET_ROOT, "tine_plugin_bullet_threading.wasm");
  await page.locator(".settings-pane-head .settings-close").click();
  await navigate(page, "Jun 14th, 2026");
  const threaded = page.locator(".ls-block.plugin-thread-lines:has(> .block-children-container)").first();
  await threaded.waitFor();
  await threaded.screenshot({ path: `${BULLET_SHOTS}/bullet-threading.png` });

  await openPlugins(page);
  await installLocal(page, QUERY_ROOT, "tine_plugin_query_filter.wasm");
  await page.locator(".settings-pane-head .settings-close").click();
  await navigate(page, "Jun 14th, 2026");

  const queryBlock = page.locator(".ls-block", { hasText: "All todos + Prio A" }).first();
  await queryBlock.waitFor();
  const queryId = await queryBlock.getAttribute("data-block-id");
  if (!queryId) throw new Error("inline query block has no stable id");
  const tasksBlock = page.locator(`.ls-block[data-block-id="${queryId}"]`);
  await queryBlock.locator(".block-content").first().click({ position: { x: 24, y: 10 } });
  const editor = page.locator(editorSelector());
  await editor.waitFor();
  assert.equal((await editorState(editorBrowser)).kind, "cm6", "launch plugins exercise the default editor");
  // Replace the block's text as a person does: select all, then type. Playwright's fill sets a page selection and
  // inserts in the same instant. Where the keyboard writes through an EditContext (Android), the editor has not read
  // that selection yet, so the new text would land after the old text.
  await editor.press("Control+a");
  const tasksText = PLATFORM === "desktop"
    ? "Tasks\n{{query (todo TODO DOING DONE)}}"
    : "Tasks\n{{query (todo TODO DOING DONE)}}\ntine.view:: table";
  await page.keyboard.insertText(tasksText);
  assert.equal(await editorText(editorBrowser), tasksText, "the typed text replaces the block's text");
  await page.keyboard.press("Escape");
  await editor.waitFor({ state: "detached" });
  if (PLATFORM === "desktop") {
    // The calm query block keeps its view choice in one place: ⋯ → View.
    await tasksBlock.locator(".qc-actions > summary").first().click();
    await page.getByRole("button", { name: "View", exact: true }).click();
    await tasksBlock.locator('.qc-panel [aria-label="Query view"]').getByRole("button", { name: "table", exact: true }).click();
    await tasksBlock.locator(".qc-panel .qc-done").click();
    await tasksBlock.locator(".qc-panel").waitFor({ state: "detached" });
  }
  await page.locator(".sheet-table").waitFor();
  // The query now runs over the whole mock graph, so several rows are closed. A row on screen also carries its cell
  // menu button, so closed rows are read from the State column, not from a cell's whole text.
  const closedCells = tasksBlock
    .locator('.sheet-table .sheet-cell[data-sheet-column-id="state"]')
    .filter({ hasText: /(?:DONE|CANCELED|CANCELLED)$/ });
  await closedCells.first().waitFor();
  await tasksBlock.getByText("Closed task", { exact: true }).waitFor();
  await tasksBlock.getByText("Open task", { exact: true }).waitFor();
  await tasksBlock.locator(".block-content").first().click({ position: { x: 24, y: 10 } });
  const tableEditor = page.locator(editorSelector());
  await tableEditor.waitFor();
  const tableRaw = await editorText(editorBrowser);
  if (!tableRaw.includes("tine.view:: table")) {
    throw new Error(`query editor did not retain the selected table view: ${JSON.stringify(tableRaw)}`);
  }
  await page.locator(".toast-close").evaluateAll((buttons) => buttons.forEach((button) => button.click()));
  await page.keyboard.press("Control+k");
  const switcher = page.locator(".switcher-input");
  await switcher.waitFor();
  await page.waitForFunction(() => document.activeElement?.classList.contains("switcher-input"));
  await switcher.fill("hide completed");
  await page.waitForFunction(() => document.querySelector(".switcher-input")?.value === "hide completed");
  await page.getByText("Query view: hide completed rows", { exact: true }).waitFor();
  await page.screenshot({ path: `${QUERY_SHOTS}/query-filter-command.png` });
  await page.getByText("Query view: hide completed rows", { exact: true }).click();
  await closedCells.first().waitFor({ state: "detached" });
  await tasksBlock.getByText("Closed task", { exact: true }).waitFor({ state: "detached" });
  await tasksBlock.getByText("Open task", { exact: true }).waitFor();
  if (await page.locator(".toast-error").count()) {
    throw new Error(`query-filter command failed: ${await page.locator(".toast-error").innerText()}`);
  }
  if (await closedCells.count()) throw new Error("query-filter result still contains a closed task row");
  await tasksBlock.screenshot({ path: `${QUERY_SHOTS}/query-filter-result.png` });

  await openPlugins(page);
  await installLocal(page, HEADING_ROOT, "tine_plugin_heading_level_shortcuts.wasm");
  await page.locator(".settings-pane-head .settings-close").click();
  await navigate(page, "Jun 14th, 2026");
  const headingBlock = page.locator(".ls-block", { hasText: "Started the" }).first();
  await headingBlock.locator(".block-content").first().click({ position: { x: 80, y: 12 } });
  const headingEditor = page.locator(editorSelector());
  await headingEditor.waitFor();
  const originalHeading = await editorText(editorBrowser);
  await page.keyboard.press("Control+k");
  await page.locator(".switcher-input").fill("heading level 1");
  await page.getByText("Heading: level 1", { exact: true }).waitFor();
  await page.screenshot({ path: `${HEADING_SHOTS}/heading-level-command.png` });
  await page.getByText("Heading: level 1", { exact: true }).click();
  await headingBlock.locator(".heading-text.h1").waitFor();
  await headingBlock.locator(".block-content").first().click({ position: { x: 80, y: 12 } });
  await headingEditor.waitFor();
  assert.equal(await editorText(editorBrowser), `# ${originalHeading}`);
  await page.keyboard.press("Escape");
  await headingBlock.locator(".heading-text.h1").waitFor();
  await headingBlock.screenshot({ path: `${HEADING_SHOTS}/heading-level-result.png` });

  if (errors.length) throw new Error(`browser errors: ${errors.join("; ")}`);
  console.log(`${PLATFORM} plugin behavior and documentation screenshots captured`);
} finally {
  await browser?.close();
  server.kill("SIGTERM");
  rmSync(previewDir, { recursive: true, force: true });
}
