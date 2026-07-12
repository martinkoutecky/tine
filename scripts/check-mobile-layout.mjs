import { chromium } from "playwright";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const css = fs.readFileSync(path.join(root, "src/styles/app.css"), "utf8");
const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 390, height: 844 } });
  await page.setContent(`<!doctype html><style>${css}</style><main class="main-content"><div class="main-content-inner"><h1>Mobile outline</h1><div class="ls-block">• Text should use the narrow screen.</div></div></main>`);
  const geometry = await page.locator(".main-content-inner").evaluate((element) => {
    const style = getComputedStyle(element);
    const rect = element.getBoundingClientRect();
    return {
      width: rect.width,
      paddingLeft: Number.parseFloat(style.paddingLeft),
      paddingRight: Number.parseFloat(style.paddingRight),
      usable: rect.width - Number.parseFloat(style.paddingLeft) - Number.parseFloat(style.paddingRight),
    };
  });
  if (geometry.paddingLeft > 16 || geometry.paddingRight > 16 || geometry.usable < 350) {
    throw new Error(`mobile content remains too narrow: ${JSON.stringify(geometry)}`);
  }
  fs.mkdirSync(path.join(root, "test-results"), { recursive: true });
  await page.screenshot({ path: path.join(root, "test-results/mobile-content-width.png"), fullPage: true });
  console.log(`PASS: mobile content uses ${geometry.usable}px of a 390px viewport`);
} finally {
  await browser.close();
}
