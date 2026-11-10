import { execFileSync } from "node:child_process";

// WebKitWebDriver can stall on screenshots after a resize. Capture the native
// X11 display on Linux; other platforms use their WebDriver screenshot command.
export async function saveJourneyScreenshot(browser, file) {
  if (process.platform === "linux") execFileSync("import", ["-window", "root", file], { timeout: 10_000 });
  else await browser.saveScreenshot(file);
}
