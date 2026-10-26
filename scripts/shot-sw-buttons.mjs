// Source-derived button specimens, with shipped CSS, in both themes.
// Usage: node scripts/shot-sw-buttons.mjs before|after
import { chromium } from "playwright";
import { spawn, execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { setTimeout as delay } from "node:timers/promises";
import ts from "typescript";
import assert from "node:assert/strict";

const phase = process.argv[2] ?? "after";
const out = `artifacts/sw-buttons/${phase}`;
mkdirSync(out, { recursive: true });
const files = ["ContextMenu", "Welcome", "UnsavedRecovery", "DiagnosticsTab", "GraphVerification", "ErrorToastHistory", "FailureBoundary", "PropertyType", "WorkspaceSwitcher", "QueryDisplay", "QueryPlanningDatePick"];
const specimens = [];
const sourceText = file => phase === "before" ? execFileSync("git", ["show", `HEAD:${file}`], { encoding: "utf8" }) : readFileSync(file, "utf8");
for (const file of [...files.map(name => `src/components/${name}.tsx`), "src/App.tsx"]) {
  const source = ts.createSourceFile(file, sourceText(file), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const rows = [];
  function visit(node) {
    if (ts.isJsxElement(node) && node.openingElement.tagName.getText(source) === "button") {
      const opening = node.openingElement;
      const attr = name => opening.attributes.properties.find(a => a.name?.getText(source) === name)?.initializer;
      const cls = attr("class");
      const className = cls && ts.isStringLiteral(cls) ? cls.text : "";
      const text = node.getText(source);
      if (file.endsWith("ContextMenu.tsx") && !text.includes('t("pages-save-icon")') && !text.includes("write(value().trim()")) return;
      if (file.endsWith("App.tsx") && !text.includes('t("app-close-tab")')) return;
      const key = text.match(/t\("([^"]+)"/);
      rows.push({ className, label: key?.[1] ?? "Date / type / view", type: attr("type")?.text ?? "button" });
    }
    ts.forEachChild(node, visit);
  }
  visit(source);
  specimens.push({ file, rows });
}
const port = 21000 + Math.floor(Math.random() * 10000);
const server = spawn("npx", ["vite", "--port", String(port), "--strictPort"], { stdio: "ignore", detached: true });
let browser;
try {
  for (let i = 0; ; i++) {
    try { if ((await fetch(`http://localhost:${port}`)).ok) break; } catch {}
    if (i > 100) throw new Error("Vite did not start");
    await delay(200);
  }
  browser = await chromium.launch({ args: ["--no-sandbox"] });
  const page = await browser.newPage({ viewport: { width: 1500, height: 1300 } });
  await page.goto(`http://localhost:${port}`);
  await page.evaluate(async () => { await import("/src/styles/theme.css"); await import("/src/styles/app.css"); });
  if (phase === "before") await page.evaluate(css => {
    document.querySelector('style[data-vite-dev-id$="/src/styles/app.css"]').textContent = css;
  }, sourceText("src/styles/app.css"));
  const results = [];
  for (const theme of ["light", "dark"]) {
    await page.evaluate(({ specimens, theme }) => {
      document.documentElement.dataset.theme = theme;
      document.body.replaceChildren();
      const main = document.createElement("main");
      main.style.cssText = "padding:24px;background:var(--bg-primary);color:var(--text-primary);min-height:100vh";
      document.body.append(main);
      const heading = document.createElement("h1"); heading.textContent = `Button style specimens — ${theme}`; main.append(heading);
      for (const { file, rows } of specimens) {
        const section = document.createElement("section");
        section.style.cssText = "padding:10px;border-bottom:1px solid var(--border-color)";
        const label = document.createElement("div"); label.textContent = file; label.style.cssText = "font-size:12px;margin-bottom:8px"; section.append(label);
        const actions = document.createElement("div"); actions.style.cssText = "display:flex;gap:8px;flex-wrap:wrap";
        for (const row of rows) {
          const button = document.createElement("button"); button.className = row.className; button.type = row.type;
          button.textContent = row.label.replace(/^(context-menu-ui|[a-z]+(?:-[a-z]+)?)-/, "");
          actions.append(button);
        }
        section.append(actions); main.append(section);
      }
      const future = document.createElement("button"); future.textContent = "Future classless action"; future.dataset.future = "true"; main.append(future);
    }, { specimens, theme });
    await page.screenshot({ path: `${out}/${theme}.png`, fullPage: true });
    results.push(...await page.locator("button").evaluateAll((buttons, theme) => buttons.map(button => {
      const css = getComputedStyle(button);
      return { theme, label: button.textContent, className: button.className, appearance: css.appearance, border: css.borderTopStyle, color: css.color, background: css.backgroundColor };
    }), theme));
  }
  writeFileSync(`${out}/observations.json`, JSON.stringify(results, null, 2) + "\n");
  for (const row of results) {
    assert.equal(row.appearance, "none", `${row.theme}: ${row.label} uses native appearance`);
    assert.ok(!["outset", "inset"].includes(row.border), `${row.theme}: ${row.label} uses native bevel`);
  }
  console.log(`${results.length} button specimens passed across light and dark themes.`);
} finally {
  await browser?.close();
  try { process.kill(-server.pid, "SIGTERM"); } catch {}
}
