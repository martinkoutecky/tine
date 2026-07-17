// Capture Tine itself using the two launch theme packages.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { setTimeout as sleep } from "node:timers/promises";
import {
  resolveLaunchThemeRoots,
  resolveRepositoryRoot,
  resolveViteExecutable,
} from "./docs-preview-paths.mjs";

const PORT = 5198;
const ROOT = resolveRepositoryRoot(import.meta.url);
const VITE = resolveViteExecutable(ROOT);
const { dev: DEV_ROOT, things: THINGS_ROOT } = resolveLaunchThemeRoots(ROOT);
const SCREENSHOT_ROOT = process.env.TINE_THEME_SCREENSHOT_ROOT;
const DEV_SHOTS = SCREENSHOT_ROOT ? `${SCREENSHOT_ROOT}/dev` : `${DEV_ROOT}/docs`;
const THINGS_SHOTS = SCREENSHOT_ROOT ? `${SCREENSHOT_ROOT}/things` : `${THINGS_ROOT}/docs`;

const server = spawn(
  VITE,
  ["preview", "--host", "127.0.0.1", "--port", String(PORT), "--strictPort"],
  { stdio: "ignore" },
);
let serverSpawnError;
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

async function openAppearance(page) {
  await page.getByTitle("Settings (t s)").click();
  await page.getByRole("button", { name: "Appearance", exact: true }).click();
}

async function installAndUse(page, root) {
  const manifest = JSON.parse(await readFile(`${root}/theme.json`, "utf8"));
  await page.locator('input[type="file"][accept="application/json,.json"]').setInputFiles(`${root}/theme.json`);
  await page.locator(".toast-msg", { hasText: `${manifest.name} ${manifest.version} installed` }).waitFor();
  const installed = page.locator(".installed-theme-row", { hasText: manifest.name });
  await installed.getByRole("button", { name: "Use theme", exact: true }).click();
  await installed.getByRole("button", { name: "Selected", exact: true }).waitFor();
}

async function navigate(page, name) {
  await page.keyboard.press("Control+k");
  await page.locator(".switcher-input").fill(name);
  await sleep(350);
  await page.locator(".switcher-row").first().click();
  await page.getByText(name, { exact: true }).first().waitFor();
}

async function closeToasts(page) {
  for (let attempt = 0; attempt < 20 && await page.locator(".toast-close").count(); attempt += 1) {
    await page.locator(".toast-close").first().click();
  }
}

try {
  const url = `http://127.0.0.1:${PORT}/`;
  await waitForServer(url);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  const page = await browser.newPage({ viewport: { width: 1280, height: 860 }, deviceScaleFactor: 2 });
  page.setDefaultTimeout(10_000);
  const errors = [];
  page.on("pageerror", (error) => errors.push(String(error)));
  page.on("console", (message) => message.type() === "error" && errors.push(`console: ${message.text()}`));
  mkdirSync(DEV_SHOTS, { recursive: true });
  mkdirSync(THINGS_SHOTS, { recursive: true });

  await page.goto(url);
  await page.waitForSelector(".page-title");
  await openAppearance(page);
  await installAndUse(page, DEV_ROOT);
  if (await page.locator('.theme-switch[aria-checked="false"]').count()) await page.locator(".theme-switch").click();
  await page.locator('.theme-switch[aria-checked="true"]').waitFor();
  await page.locator(".settings-pane-head .icon-btn").click();
  await navigate(page, "Jun 14th, 2026");
  await closeToasts(page);
  await page.screenshot({ path: `${DEV_SHOTS}/tine-dev-colors.png` });

  await openAppearance(page);
  await installAndUse(page, THINGS_ROOT);
  await page.locator(".settings-pane-head .icon-btn").click();
  await navigate(page, "Jun 14th, 2026");
  await closeToasts(page);
  await page.screenshot({ path: `${THINGS_SHOTS}/tine-things-colors.png` });

  if (errors.length) throw new Error(`browser errors: ${errors.join("; ")}`);
  await browser.close();
  console.log("theme documentation screenshots captured");
} finally {
  server.kill("SIGTERM");
}
