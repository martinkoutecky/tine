// Capture real mock-app screenshots while exercising all launch plugins.
// Usage: npm run build, build the plugin WASMs, then run this script.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5197;
const ROOT = new URL("../..", import.meta.url).pathname.replace(/\/$/, "");
const QUERY_ROOT = process.env.TINE_QUERY_PLUGIN_ROOT ?? `${ROOT}/tine-plugin-query-filter`;
const BULLET_ROOT = process.env.TINE_BULLET_PLUGIN_ROOT ?? `${ROOT}/tine-plugin-bullet-threading`;
const HEADING_ROOT = process.env.TINE_HEADING_PLUGIN_ROOT ?? `${ROOT}/tine-plugin-heading-level-shortcuts`;
const SCREENSHOT_ROOT = process.env.TINE_PLUGIN_SCREENSHOT_ROOT;
const PLATFORM = process.env.TINE_PLUGIN_PLATFORM ?? "desktop";

function screenshotDir(pluginRoot, slug) {
  return SCREENSHOT_ROOT ? `${SCREENSHOT_ROOT}/${slug}` : `${pluginRoot}/docs`;
}

const BULLET_SHOTS = screenshotDir(BULLET_ROOT, "bullet-threading");
const QUERY_SHOTS = screenshotDir(QUERY_ROOT, "query-filter");
const HEADING_SHOTS = screenshotDir(HEADING_ROOT, "heading-level-shortcuts");

const server = spawn(`${ROOT}/tine-plugins/node_modules/.bin/vite`, ["preview", "--host", "127.0.0.1", "--port", String(PORT), "--strictPort"], {
  stdio: "ignore",
});

async function waitForServer(url) {
  for (let attempt = 0; attempt < 40; attempt += 1) {
    try {
      if ((await fetch(url)).ok) return;
    } catch {
      // Preview is still starting.
    }
    await sleep(250);
  }
  throw new Error("preview server did not start");
}

async function openPlugins(page) {
  await page.getByTitle("Settings (t s)").click();
  await page.getByRole("button", { name: "Plugins", exact: true }).click();
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
  const url = `http://127.0.0.1:${PORT}/${PLATFORM === "desktop" ? "" : `?platform=${PLATFORM}`}`;
  await waitForServer(url);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
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
  const errors = [];
  page.on("pageerror", (error) => errors.push(String(error)));
  page.on("console", (message) => message.type() === "error" && errors.push(`console: ${message.text()}`));

  await page.goto(url);
  await page.waitForSelector(".page-title");
  for (const root of [BULLET_SHOTS, QUERY_SHOTS, HEADING_SHOTS]) mkdirSync(root, { recursive: true });
  await openPlugins(page);
  await installLocal(page, BULLET_ROOT, "tine_plugin_bullet_threading.wasm");
  await page.locator(".settings-pane-head .icon-btn").click();
  await navigate(page, "Jun 14th, 2026");
  const threaded = page.locator(".ls-block.plugin-thread-lines:has(> .block-children-container)").first();
  await threaded.waitFor();
  await threaded.screenshot({ path: `${BULLET_SHOTS}/bullet-threading.png` });

  await openPlugins(page);
  await installLocal(page, QUERY_ROOT, "tine_plugin_query_filter.wasm");
  await page.locator(".settings-pane-head .icon-btn").click();
  await navigate(page, "Jun 14th, 2026");

  const queryBlock = page.locator(".ls-block", { hasText: "All todos + Prio A" }).first();
  await queryBlock.waitFor();
  const queryId = await queryBlock.getAttribute("data-block-id");
  if (!queryId) throw new Error("inline query block has no stable id");
  const tasksBlock = page.locator(`.ls-block[data-block-id="${queryId}"]`);
  await queryBlock.locator(".block-content").first().click({ position: { x: 24, y: 10 } });
  const editor = page.locator("textarea.block-editor");
  await editor.waitFor();
  await editor.fill(
    PLATFORM === "desktop"
      ? "Tasks\n{{query (todo TODO DOING DONE)}}"
      : "Tasks\n{{query (todo TODO DOING DONE)}}\ntine.view:: table",
  );
  await page.keyboard.press("Escape");
  await editor.waitFor({ state: "detached" });
  if (PLATFORM === "desktop") {
    await tasksBlock.getByRole("button", { name: "Table", exact: true }).click();
  }
  await page.locator(".sheet-table").waitFor();
  await tasksBlock.locator(".block-content").first().click({ position: { x: 24, y: 10 } });
  const tableEditor = page.locator("textarea.block-editor");
  await tableEditor.waitFor();
  const tableRaw = await tableEditor.inputValue();
  if (!tableRaw.includes("tine.view:: table")) {
    throw new Error(`query editor did not retain the selected table view: ${JSON.stringify(tableRaw)}`);
  }
  await page.locator(".toast-close").evaluateAll((buttons) => buttons.forEach((button) => button.click()));
  await page.keyboard.press("Control+k");
  await page.locator(".switcher-input").fill("hide completed");
  await page.getByText("Query view: hide completed rows", { exact: true }).waitFor();
  await page.screenshot({ path: `${QUERY_SHOTS}/query-filter-command.png` });
  await page.getByText("Query view: hide completed rows", { exact: true }).click();
  await sleep(300);
  if (await page.locator(".toast-error").count()) {
    throw new Error(`query-filter command failed: ${await page.locator(".toast-error").innerText()}`);
  }
  const closedCells = tasksBlock.locator(".sheet-table .sheet-cell").filter({ hasText: /^(?:DONE|CANCELED|CANCELLED)$/ });
  if (await closedCells.count()) throw new Error("query-filter result still contains a closed task row");
  await tasksBlock.screenshot({ path: `${QUERY_SHOTS}/query-filter-result.png` });

  await openPlugins(page);
  await installLocal(page, HEADING_ROOT, "tine_plugin_heading_level_shortcuts.wasm");
  await page.locator(".settings-pane-head .icon-btn").click();
  await navigate(page, "Jun 14th, 2026");
  const headingBlock = page.locator(".ls-block", { hasText: "Started the" }).first();
  await headingBlock.locator(".block-content").first().click({ position: { x: 80, y: 12 } });
  const headingEditor = page.locator("textarea.block-editor");
  await headingEditor.waitFor();
  const originalHeading = await headingEditor.inputValue();
  await page.keyboard.press("Control+k");
  await page.locator(".switcher-input").fill("heading level 1");
  await page.getByText("Heading: level 1", { exact: true }).waitFor();
  await page.screenshot({ path: `${HEADING_SHOTS}/heading-level-command.png` });
  await page.getByText("Heading: level 1", { exact: true }).click();
  await headingBlock.locator(".heading-text.h1").waitFor();
  await headingBlock.locator(".block-content").first().click({ position: { x: 80, y: 12 } });
  await page.waitForFunction(
    (before) => document.querySelector("textarea.block-editor")?.value === `# ${before}`,
    originalHeading,
  );
  await page.keyboard.press("Escape");
  await headingBlock.locator(".heading-text.h1").waitFor();
  await headingBlock.screenshot({ path: `${HEADING_SHOTS}/heading-level-result.png` });

  if (errors.length) throw new Error(`browser errors: ${errors.join("; ")}`);
  await browser.close();
  console.log(`${PLATFORM} plugin behavior and documentation screenshots captured`);
} finally {
  server.kill("SIGTERM");
}
