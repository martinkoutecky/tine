// Render the published "Feature showcase" demo page and screenshot it (full page).
// Grounds how Tine's HTML EXPORT renders every page-level feature. NOTE: the export
// path differs from the in-app render (it drops task markers/queries/embeds) — this
// screenshots the export, not the app. Run after `build-demo-site`.
import { chromium } from "playwright";
const b = await chromium.launch();
const page = await b.newPage({ viewport: { width: 820, height: 1400 }, deviceScaleFactor: 2 });
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
await page.goto("file://" + process.cwd() + "/website/demo/feature-showcase.html", { waitUntil: "networkidle" });
await new Promise((r) => setTimeout(r, 800));
await page.screenshot({ path: "screenshots/showcase-published.png", fullPage: true });
console.log(errors.length ? "PAGEERRORS:\n" + errors.join("\n") : "no page errors");
await b.close();
