// Layout check for D-20261018-UI1: DOM order cannot prove that Cancel stays
// immediately left of the primary action once the row has to wrap, so mount the
// real <DialogActions> with the production stylesheet in Chromium and measure.
// At every width Cancel and the primary action must share one row, Cancel
// directly left of the primary action, and the primary action must be the
// trailing item; the auxiliary group is above or leading, never between them.
// PROBE_BREAK=1 injects the pre-fix behaviour (independent wrapping) and the
// script must then FAIL: that is the check's own self-test.
// Usage: node scripts/shot-action-placement-layout.mjs   (source scripts/env.sh for Chromium)
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "action-layout-"));
const { createServer } = await import("vite");
const { default: solid } = await import("vite-plugin-solid");
const { chromium } = await import("playwright");

const shapes = {
  "css-editor": `<DialogActions aux={<><button class="settings-btn">Reload</button><button class="settings-btn">Save</button></>}
    cancel={<button class="settings-btn">Cancel</button>} primary={<button class="settings-btn settings-btn-primary">Save &amp; close</button>} />`,
  "export-dialog": `<DialogActions cancel={<button class="export-btn-secondary">Cancel</button>} primary={<button class="export-btn-primary">Export to PDF</button>} />`,
  "long-aux": `<DialogActions aux={<><button class="settings-btn">Keep anyway</button><button class="settings-btn">Reload from file</button><button class="settings-btn">Copy details</button></>}
    cancel={<button class="settings-btn">Cancel</button>} primary={<button class="settings-btn settings-btn-primary">Remove them</button>} />`,
};
fs.writeFileSync(path.join(scratch, "entry.tsx"), `
import { render } from "solid-js/web";
import { DialogActions } from ${JSON.stringify(path.join(repo, "src/components/DialogActions"))};
import ${JSON.stringify(path.join(repo, "src/styles/app.css"))};
const shapes = [${Object.entries(shapes).map(([name, jsx]) => `[${JSON.stringify(name)}, () => ${jsx}]`).join(",")}];
render(() => <div class="modal-overlay"><div class="settings-modal" role="dialog" style={{ "flex-direction": "column", padding: "20px", gap: "12px" }}>
  {shapes.map(([name, view]) => <div data-shape={name}>{view()}</div>)}
</div></div>, document.body);
`);
const server = await createServer({
  configFile: false, root: repo, cacheDir: path.join(scratch, "cache"), logLevel: "error", plugins: [solid()],
  server: { port: 0, fs: { allow: [repo, scratch] } },
});
await server.listen();
const origin = `http://localhost:${server.httpServer.address().port}`;
const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu"] });
const failures = [];
try {
  const page = await browser.newPage();
  page.on("pageerror", (e) => failures.push(`pageerror ${e}`));
  await page.route(`${origin}/probe`, (route) => route.fulfill({
    contentType: "text/html",
    body: `<html><head><style>:root{--overlay-inset-top:0px;--overlay-inset-bottom:0px;--overlay-inset-left:0px;--overlay-inset-right:0px;--ls-font-family:Arial;--bg-primary:white;--text-primary:black;--border-color:#ddd;--link-color:blue;--on-accent:white}
      ${process.env.PROBE_BREAK === "1" ? ".dialog-actions-main{display:contents!important}" : ""}</style></head>
      <body><script type="module" src="/@fs${path.join(scratch, "entry.tsx")}"></script></body></html>`,
  }));
  await page.goto(`${origin}/probe`);
  await page.waitForSelector(".dialog-actions button");
  for (const width of [1200, 390, 360, 320]) {
    await page.setViewportSize({ width, height: 780 });
    const rows = await page.evaluate(() => [...document.querySelectorAll("[data-shape]")].map((shape) => ({
      shape: shape.dataset.shape,
      buttons: [...shape.querySelectorAll("button")].map((b) => {
        const r = b.getBoundingClientRect();
        return { label: b.textContent.trim(), top: Math.round(r.top), left: r.left, right: r.right };
      }),
    })));
    for (const { shape, buttons } of rows) {
      const primary = buttons.at(-1);
      const cancel = buttons.at(-2);
      const where = `${shape} @${width}px`;
      if (cancel.top !== primary.top) failures.push(`${where}: Cancel (row ${cancel.top}) and ${primary.label} (row ${primary.top}) are on different rows`);
      if (!(cancel.right <= primary.left && primary.left - cancel.right <= 16)) failures.push(`${where}: Cancel is not immediately left of ${primary.label}`);
      if (buttons.some((b) => b.right > primary.right + 0.5)) failures.push(`${where}: ${primary.label} is not the trailing item`);
      for (const aux of buttons.slice(0, -2)) {
        if (aux.top === primary.top && aux.left >= cancel.left) failures.push(`${where}: auxiliary ${aux.label} sits between or after Cancel/${primary.label}`);
      }
      if (process.env.SHOT_DIR && shape === "css-editor") await page.screenshot({ path: path.join(process.env.SHOT_DIR, `layout-css-editor-${width}.png`) });
      if (process.env.SHOT_DEBUG) console.log(where, JSON.stringify(buttons.map((b) => [b.label, b.top, Math.round(b.left)])));
    }
  }
} finally {
  await browser.close();
  await server.close();
  fs.rmSync(scratch, { recursive: true, force: true });
}
if (failures.length) {
  console.error(`FAIL action-placement layout:\n  ${failures.join("\n  ")}`);
  process.exit(1);
}
console.log("PASS action-placement layout: Cancel stays immediately left of the primary action at 1200/390/360/320px");
