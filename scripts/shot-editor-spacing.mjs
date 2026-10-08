// Visual regression harness for raw editor punctuation/number spacing.
// Runs the real frontend against the mock backend and writes four block crops to
// screenshots/ for human inspection. This specifically covers the WebKitGTK
// screenshots that exposed Noto Emoji keycap-base metrics in ordinary source.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5199;
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });

async function waitForServer(url) {
  for (let i = 0; i < 60; i++) {
    try { if ((await fetch(url)).ok) return; } catch {}
    await sleep(250);
  }
  throw new Error("server did not start");
}

async function setEditor(page, value) {
  if (!(await page.locator("textarea.block-editor").count())) {
    await page.locator(".ls-block .block-content").first().click();
  }
  await page.waitForSelector("textarea.block-editor", { timeout: 3000 });
  await page.evaluate((next) => {
    const editor = document.querySelector("textarea.block-editor");
    editor.value = next;
    editor.dispatchEvent(new Event("input", { bubbles: true }));
  }, value);
  await sleep(100);
}

try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1000, height: 700 } });
  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".ls-block", { timeout: 5000 });

  const cases = [
    ["heading", "## Current plan"],
    ["version", "v0.5.0"],
    ["bold", "**Some kind of AI integration?**"],
    ["priority", "DONE [#A] AA"],
  ];
  for (const [name, value] of cases) {
    await setEditor(page, value);
    await page.locator(".ls-block").first().screenshot({ path: `screenshots/editor-spacing-${name}.png` });
  }

  console.log(await page.locator("textarea.block-editor").evaluate((editor) => ({
    fontFamily: getComputedStyle(editor).fontFamily,
    value: editor.value,
  })));
  await browser.close();
  server.kill("SIGKILL");
} catch (error) {
  console.error(String(error));
  server.kill("SIGKILL");
  process.exit(1);
}
