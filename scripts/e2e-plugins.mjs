// Render and exercise the signed community catalogue against the browser mock.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5194;
const server = spawn(
  "./node_modules/.bin/vite",
  ["preview", "--host", "127.0.0.1", "--port", String(PORT), "--strictPort"],
  { stdio: "inherit" }
);

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

try {
  const url = `http://127.0.0.1:${PORT}/`;
  await waitForServer(url);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  const page = await browser.newPage({ viewport: { width: 1280, height: 860 }, deviceScaleFactor: 2 });
  const pageErrors = [];
  page.on("pageerror", (error) => pageErrors.push(String(error)));
  page.on("console", (message) => {
    if (message.type() === "error") pageErrors.push(`console: ${message.text()}`);
  });
  page.on("requestfailed", (request) => pageErrors.push(`request: ${request.url()} (${request.failure()?.errorText})`));
  await page.goto(url);
  await page.waitForSelector(".page-title");
  await page.getByTitle("Settings (t s)").click();
  await page.getByRole("button", { name: "Plugins", exact: true }).click();
  try {
    await page.getByText("Bullet threading v0.2.0", { exact: true }).waitFor({ timeout: 15_000 });
  } catch (error) {
    console.error(await page.locator(".settings-pane-body").innerText());
    console.error(pageErrors.join("\n"));
    throw error;
  }
  await page.getByText("Query filter shortcuts v0.2.0", { exact: true }).waitFor();
  await page.getByText("Heading level shortcuts v0.1.0", { exact: true }).waitFor();
  await page.getByText(/Signed registry.*automated deterministic.*AI audits/).waitFor();

  const bullet = page.locator(".settings-field", { hasText: "Bullet threading" });
  const queryFilter = page.locator(".settings-field", { hasText: "Query filter shortcuts" });
  await bullet.getByText("Low-risk automated pass", { exact: true }).waitFor();
  await queryFilter.getByText("Human-reviewed before publication", { exact: true }).waitFor();
  await queryFilter.getByRole("button", { name: "Details & screenshots", exact: true }).waitFor();
  await queryFilter.getByRole("button", { name: "Safety report", exact: true }).click();
  await queryFilter.getByText(/why human review was required/i).waitFor({ timeout: 15_000 });
  await queryFilter.getByText(/holds every graph-writing plugin for human review/i).waitFor();
  await queryFilter.getByText(/earlier draft could act on the wrong focused block/i).waitFor();
  await queryFilter.getByText(/broad textual recognition of query-view blocks/i).waitFor();
  await queryFilter.getByText("Low-risk finding", { exact: true }).first().waitFor();
  await queryFilter.getByText(/severity describes possible impact, not reviewer confidence/i).waitFor();
  const evidenceIds = await queryFilter.locator(".plugin-safety-report code[title]").evaluateAll((elements) =>
    elements.map((element) => element.getAttribute("title"))
  );
  if (evidenceIds.length !== 3 || !evidenceIds.slice(1).every((value) => /^[0-9a-f]{64}$/.test(value ?? ""))) {
    throw new Error(`safety report did not expose verified source/package/report identities: ${evidenceIds.join(", ")}`);
  }

  mkdirSync("screenshots", { recursive: true });
  await page.screenshot({ path: "screenshots/plugins-safety-report.png" });
  await queryFilter.getByRole("button", { name: "Hide safety report", exact: true }).click();
  await page.screenshot({ path: "screenshots/plugins-light.png" });

  await bullet.getByRole("button", { name: "Install", exact: true }).click();
  await page.getByText(/installed disabled/i).waitFor({ timeout: 15_000 });
  const installed = page.locator(".plugin-detail-page", { hasText: "page.tine.bullet-threading" });
  await installed.waitFor();
  const toggle = installed.locator(".settings-toggle");
  await toggle.click();
  try {
    await page.waitForFunction(
      () => [...document.querySelectorAll(".plugin-detail-page")]
        .find((element) => element.textContent?.includes("page.tine.bullet-threading"))
        ?.querySelector(".settings-toggle")
        ?.getAttribute("aria-checked") === "true",
      undefined,
      { timeout: 10_000 }
    );
  } catch (error) {
    console.error(await installed.innerText());
    console.error(pageErrors.join("\n"));
    throw error;
  }

  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: "screenshots/plugins-mobile.png" });
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  if (overflow > 1) throw new Error(`plugin settings overflow the mobile viewport by ${overflow}px`);
  for (let attempt = 0; attempt < 10 && await page.locator(".toast-close").count(); attempt += 1) {
    await page.locator(".toast-close").first().click();
  }
  page.once("dialog", (dialog) => void dialog.accept());
  await installed.getByRole("button", { name: "Uninstall…", exact: true }).click();
  await page.getByText(/was uninstalled/i).waitFor();
  await page.getByRole("tab", { name: "Browse", exact: true }).click();
  await bullet.getByRole("button", { name: "Install", exact: true }).waitFor();
  if (await page.locator(".settings-field", { hasText: "page.tine.bullet-threading" }).count()) {
    throw new Error("uninstalled plugin still appears in the Installed section");
  }

  // The registry canonicalizes theme modes independently of manifest key order.
  // Exercise the real signed Dev Theme Colors package that previously returned
  // to Install and then failed its metadata comparison.
  await page.setViewportSize({ width: 1280, height: 860 });
  await page.getByRole("button", { name: "Appearance", exact: true }).click();
  const devTheme = page.locator(".settings-field", { hasText: "Dev Theme Colors" }).first();
  await devTheme.getByRole("button", { name: "Install", exact: true }).waitFor({ timeout: 15_000 });
  await devTheme.getByRole("button", { name: "Install", exact: true }).click();
  await page.getByText(/Dev Theme Colors .* installed\./).waitFor({ timeout: 15_000 });
  const installedTheme = page.locator(".installed-theme-row", { hasText: "Dev Theme Colors" });
  await installedTheme.getByRole("button", { name: "Use theme", exact: true }).waitFor();
  page.once("dialog", (dialog) => void dialog.accept());
  await installedTheme.getByRole("button", { name: "Uninstall…", exact: true }).click();
  await page.getByText(/Dev Theme Colors was uninstalled\./).waitFor();
  await devTheme.getByRole("button", { name: "Install", exact: true }).waitFor();
  if (pageErrors.length) throw new Error(`browser errors: ${pageErrors.join("; ")}`);
  await browser.close();
  console.log("plugin catalogue E2E passed");
} finally {
  server.kill("SIGTERM");
}
