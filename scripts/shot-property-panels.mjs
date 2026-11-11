// GH #920: real routed page + live reference, synthetic data only.
// SHOT_STAGE=before node scripts/shot-property-panels.mjs (captures, then fails pre-fix).
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import fs from "node:fs";
import assert from "node:assert/strict";
const out = process.env.E2E_ARTIFACT_DIR || "target/artifacts/og-920", stage = process.env.SHOT_STAGE || "after";
const html = ".property-panels-shot.html", entry = ".property-panels-shot.tsx";
const port = 20000 + Math.floor(Math.random() * 20000);
const panelsSelector = ".page-properties, .block-properties, .page-property-reference > .block-main > .block-content-wrapper > .block-content";
fs.mkdirSync(out, { recursive: true });
fs.writeFileSync(html, `<html data-theme="light"><body><div id="root"></div><script type="module" src="/${entry}"></script></body></html>`);
fs.writeFileSync(entry, `
import { render } from "solid-js/web";
import "./src/styles/theme.css";
import "./src/styles/app.css";
import "./src/styles/ls-shim.css";
import { initParser } from "./src/render/parse";
import { loadSingle } from "./src/document/workingSet";
import { PageView } from "./src/components/Page";
import { EmbedMacro } from "./src/components/EmbedMacro";
import { mainPaneRouter } from "./src/router";
import { backend } from "./src/backend";
await initParser();
const page = { name: "Property panels", title: "Property panels", kind: "page", format: "md",
 pre_block: "owner:: Ada\\nstatus:: In progress", blocks: [{ id: "body", raw: "A block with metadata\\nowner:: Ada\\nstatus:: Ready", collapsed: false, children: [] }] };
backend().getPage = async () => ({ ...page, id: "pages/Property panels.md" });
backend().getBacklinks = async () => [{ page: page.name, kind: "page", blocks: [{ id: "props", raw: page.pre_block, page_property: true, collapsed: false, children: [] }] }];
loadSingle(page);
mainPaneRouter.openPage(page.name, "page", { inPlace: true });
render(() => <main style="max-width:760px;margin:40px auto"><PageView /><h2>Page embed</h2>
<EmbedMacro body="embed [[Property panels]]" />
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
  await tab.goto(`http://localhost:${port}/${html}`);
  await tab.waitForSelector(".page-property-reference-row");
  await tab.waitForSelector(".embed-block .block-properties");
  const metrics = [];
  for (const theme of ["", "nord", "solarized", "gruvbox", "soft"]) {
    const sheet = await tab.addStyleTag({ content: theme ? fs.readFileSync(`src/styles/themes/${theme}.css`, "utf8") : "/* default */" });
    for (const mode of ["light", "dark"]) {
      await tab.evaluate(({ theme, mode }) => {
        document.documentElement.dataset.theme = mode;
        document.documentElement.dataset.colorScheme = theme;
      }, { theme, mode });
      const panels = await tab.locator(panelsSelector).evaluateAll(elements => elements.map(el => {
        const css = getComputedStyle(el);
        let ancestor = el.parentElement;
        while (ancestor && getComputedStyle(ancestor).backgroundColor === "rgba(0, 0, 0, 0)") ancestor = ancestor.parentElement;
        return { background: css.backgroundColor, ancestorBackground: ancestor ? getComputedStyle(ancestor).backgroundColor : null,
          container: ancestor?.className, color: css.color, padding: css.padding, radius: css.borderRadius };
      }));
      metrics.push({ theme: theme || "default", mode, panels });
      await tab.screenshot({ path: `${out}/${stage}-${theme || "default"}-${mode}.png`, fullPage: true });
    }
    await sheet.evaluate(el => el.remove());
  }
  await tab.addStyleTag({ content: "html { --ls-block-properties-background-color: rgb(90, 80, 70); }" });
  const override = await tab.locator(panelsSelector).evaluateAll(elements => elements.map(el => getComputedStyle(el).backgroundColor));
  await tab.locator(".page-properties .prop-row").first().click();
  await tab.locator(".block-editor").first().waitFor();
  await tab.screenshot({ path: `${out}/${stage}-editing.png`, fullPage: true });
  fs.writeFileSync(`${out}/${stage}-metrics.json`, JSON.stringify({ metrics, override }, null, 2));
  for (const { panels } of metrics) {
    assert.equal(panels.length, 4, "page, block, referenced page and embedded block properties rendered");
    for (const panel of panels) {
      assert.notEqual(panel.background, "rgba(0, 0, 0, 0)", "properties need a tinted panel");
      assert.ok(panel.ancestorBackground, "properties have a painted ancestor");
      assert.notEqual(panel.background, panel.ancestorBackground, `properties stand out from ${panel.container}`);
    }
  }
  assert.ok(override.every(color => color === "rgb(90, 80, 70)"), "Logseq custom.css hook reaches every panel");
  console.log("Property panels: all bundled palettes in both modes, custom.css and editing passed");
} finally {
  await browser?.close(); server.kill(); fs.unlinkSync(html); fs.unlinkSync(entry);
}
