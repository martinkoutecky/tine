// Screenshot sections of website/compare.html (static file) to check the plugin
// pills + query wording.
import { chromium } from "playwright";
import { pathToFileURL } from "node:url";

const src = process.argv[2] || new URL("../website/compare.html", import.meta.url).pathname;
const out = "/tmp/claude-3042/-aux-koutecky-logseq/5a8edc2b-663a-4ee8-b183-325ec619c4d9/scratchpad";
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1000, height: 1000 } });
await page.goto(pathToFileURL(src).href, { waitUntil: "networkidle" });
await page.waitForTimeout(300);
// The quick-answers cards (queries) near the top.
await page.locator(".qa").screenshot({ path: `${out}/compare-qa.png` });
// The "Search, assets & export" table (PDF export row) + "goes further" (tabs).
const tables = page.locator("table.cmp");
const n = await tables.count();
await tables.nth(n - 2).screenshot({ path: `${out}/compare-export.png` });
await tables.nth(n - 1).screenshot({ path: `${out}/compare-further.png` });
await browser.close();
console.log("wrote compare-qa/export/further.png");
