// GH #658 / GH #596: capture real header/calendar components in stock and Soft themes.
// Usage: node scripts/shot-reference-controls.mjs <artifact-directory> [--check]
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";

const out = resolve(process.argv[2]);
mkdirSync(out, { recursive: true });
const check = process.argv.includes("--check");
const port = 20000 + Math.floor(Math.random() * 20000);
const server = spawn("npx", ["vite", "--port", String(port), "--strictPort"], { stdio: "ignore", detached: true });
let browser;
const observations = [];
try {
  for (let attempt = 0; ; attempt++) {
    try { if ((await fetch(`http://localhost:${port}/`)).ok) break; } catch {}
    if (attempt === 80) throw new Error("Vite did not start");
    await delay(250);
  }
  browser = await chromium.launch({ args: ["--no-sandbox"] });
  for (const palette of ["stock", "soft"]) for (const mode of ["light", "dark"]) {
    const page = await browser.newPage({ viewport: { width: 900, height: 750 } });
    await page.goto(`http://localhost:${port}/scripts/fixtures/reference-controls.html`);
    await page.locator(".linked-references").waitFor();
    if (palette === "soft") await page.addStyleTag({ content: readFileSync("src/styles/themes/soft.css", "utf8") });
    await page.evaluate((value) => document.documentElement.setAttribute("data-theme", value), mode);
    await page.locator("#open-calendar").click();
    await page.locator(".dp-cell:focus").waitFor();
    const inspect = () => page.evaluate(() => {
      const cell = document.querySelector(".dp-cell:focus");
      const style = getComputedStyle(cell);
      const probe = document.createElement("span");
      probe.style.color = "var(--accent)";
      document.body.append(probe);
      const accent = getComputedStyle(probe).color;
      probe.remove();
      return { day: cell.getAttribute("data-day"), outline: style.outlineStyle, color: style.outlineColor,
        width: style.outlineWidth, opacity: style.opacity, accent, background: style.backgroundColor, textColor: style.color };
    });
    const first = await inspect();
    await page.keyboard.press("ArrowRight");
    const moved = await inspect();
    if (first.day === moved.day) throw new Error("ArrowRight did not move calendar focus");
    await page.screenshot({ path: `${out}/${palette}-${mode}.png` });
    observations.push({ palette, mode, first, moved });
    await page.keyboard.press("Escape");
    if (await page.locator(".date-picker").count()) throw new Error("Escape did not close calendar");
    await page.close();
  }
  writeFileSync(`${out}/observations.json`, JSON.stringify(observations, null, 2) + "\n");
  if (check) for (const { palette, mode, moved } of observations) {
    const accentOutline = moved.outline !== "none" && parseFloat(moved.width) > 0 && moved.color === moved.accent;
    const accentFill = moved.background === moved.accent && moved.textColor !== moved.background;
    if (!accentOutline && !accentFill) {
      throw new Error(`${palette}/${mode}: keyboard focus must have a visible theme accent treatment: ${JSON.stringify(moved)}`);
    }
  }
  console.log(`Captured ${observations.length} palette/mode pairs; arrow movement and Escape passed${check ? "; accent focus passed" : ""}.`);
} finally {
  await browser?.close();
  try { process.kill(-server.pid, "SIGTERM"); } catch {}
}
