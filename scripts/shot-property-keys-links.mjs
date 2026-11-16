// Real page and reference components, before/after visual evidence for GH #753.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import fs from "node:fs";
const out = "artifacts/og-property-keys-links";
const stage = process.env.SHOT_STAGE || "after";
const html = ".property-keys-shot.html", entry = ".property-keys-shot.tsx", port = 5237;
fs.mkdirSync(out, { recursive: true });
fs.writeFileSync(html, `<html data-theme="light"><body><div id="root"></div><script type="module" src="/${entry}"></script></body></html>`);
fs.writeFileSync(entry, `
import { render } from "solid-js/web";
import "./src/styles/theme.css";
import "./src/styles/app.css";
import { initParser } from "./src/render/parse";
import { loadSingle } from "./src/document/workingSet";
import { PageView } from "./src/components/Page";
import { LiveRefGroup } from "./src/components/LiveRefGroup";
import { mainPaneRouter } from "./src/router";
import { backend } from "./src/backend";
await initParser();
const page = { name: "Source", title: "Source", kind: "page", format: "md",
 pre_block: "related:: Target\\naad:: val12", blocks: [{ id: "body", raw: "Body\\nowner:: Ada", collapsed: false, children: [] }] };
backend().getPage = async () => ({ ...page, id: "pages/Source.md" });
loadSingle(page);
mainPaneRouter.openPage("Source", "page", { inPlace: true });
render(() => <main style="max-width:760px;margin:40px auto"><PageView /><h2>Linked References</h2>
<LiveRefGroup page="Source" kind="page" blocks={[{ id: "props", raw: page.pre_block, page_property: true, collapsed: false, children: [] }]} surface="ref" eager />
</main>, document.getElementById("root"));
`);
const server = spawn("./node_modules/.bin/vite", ["--port", String(port), "--strictPort"], { stdio: "ignore" });
let browser;
try {
  for (let i = 0; i < 100; i++) {
    try { if ((await fetch(`http://localhost:${port}/${html}`)).ok) break; } catch {}
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  const tab = await browser.newPage({ viewport: { width: 1100, height: 850 } });
  tab.on("pageerror", error => console.error(error));
  tab.on("console", message => { if (message.type() === "error") console.error(message.text()); });
  await tab.goto(`http://localhost:${port}/${html}`);
  await tab.waitForSelector(".page-property-reference-row");
  await tab.screenshot({ path: `${out}/${stage}-main-and-references.png`, fullPage: true });
  fs.writeFileSync(`${out}/${stage}-visual.json`, JSON.stringify(await tab.evaluate(() => ({
    pageKeys: [...document.querySelectorAll(".page-properties .prop-key")].map(el => ({ text: el.textContent, linked: !!el.querySelector("a.page-ref") })),
    referenceKeys: [...document.querySelectorAll(".page-property-reference-row .prop-key")].map(el => ({ text: el.textContent, linked: !!el.querySelector("a.page-ref") })),
  })), null, 2));
} finally {
  await browser?.close(); server.kill(); fs.unlinkSync(html); fs.unlinkSync(entry);
}
