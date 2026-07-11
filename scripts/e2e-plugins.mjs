// Render and exercise the signed community catalogue against the browser mock.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5194;
const server = spawn(
  "npx",
  ["vite", "preview", "--host", "127.0.0.1", "--port", String(PORT), "--strictPort"],
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
    await page.getByText("Bullet threading v0.1.0", { exact: true }).waitFor({ timeout: 15_000 });
  } catch (error) {
    console.error(await page.locator(".settings-pane-body").innerText());
    console.error(pageErrors.join("\n"));
    throw error;
  }
  await page.getByText("Query filter shortcuts v0.1.0", { exact: true }).waitFor();
  await page.getByText(/Signed registry.*automated deterministic.*AI audits/).waitFor();

  mkdirSync("screenshots", { recursive: true });
  await page.screenshot({ path: "screenshots/plugins-light.png" });

  const bullet = page.locator(".settings-field", { hasText: "Bullet threading" });
  await bullet.getByRole("button", { name: "Install", exact: true }).click();
  await page.getByText(/installed disabled/i).waitFor({ timeout: 15_000 });
  await bullet.getByRole("button", { name: "Installed", exact: true }).waitFor();
  const installed = page.locator(".settings-field", { hasText: "page.tine.bullet-threading" });
  const toggle = installed.locator(".settings-toggle");
  await toggle.click();
  try {
    await page.waitForFunction(
      () => [...document.querySelectorAll(".settings-field")]
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
  if (pageErrors.length) throw new Error(`browser errors: ${pageErrors.join("; ")}`);
  await browser.close();
  console.log("plugin catalogue E2E passed");
} finally {
  server.kill("SIGTERM");
}
