import { chromium } from "playwright";
import { readFileSync } from "node:fs";
const svg = readFileSync("docs/app-icon.svg", "utf8");
const b = await chromium.launch();
const p = await b.newPage({ viewport: { width: 1024, height: 1024 }, deviceScaleFactor: 1 });
await p.setContent(`<!doctype html><html><body style="margin:0;padding:0">${svg}</body></html>`);
await p.locator("svg").screenshot({ path: "scripts/app-icon-1024.png", omitBackground: true });
await b.close();
console.log("rendered scripts/app-icon-1024.png");
