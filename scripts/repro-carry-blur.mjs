// Repro: an external change delivered while a block on the page is open in the
// editor (the page host pushes new text; `upsertPage` carries the editor).
// Observes, in headless Chromium on the browser mock app: is the external text
// kept in the window, is the editor kept, and what the window submits to the
// page host afterwards. Harness-only hook: a temporary build transforms
// src/mockPageHost.ts to expose push/submit capture; the shipped app is unchanged.
// Usage: node scripts/repro-carry-blur.mjs   (exit 1 when any case loses data)
// CB_ENGINE=webkit runs Playwright's WebKit (which fires no blur on removal).
import { chromium, webkit } from "playwright";
import { spawn } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { build } from "vite";
import { setTimeout as sleep } from "node:timers/promises";
import { resolveRepositoryRoot, resolveViteExecutable } from "./docs-preview-paths.mjs";

const PORT = 5231;
const ROOT = resolveRepositoryRoot(import.meta.url);
const VITE = resolveViteExecutable(ROOT);
const PAGE = "Formula1/2026";
const PATH = "pages/Formula1___2026.md";

const outDir = mkdtempSync(join(tmpdir(), "tine-carry-blur-"));
let hooked = false;
await build({
  root: ROOT,
  logLevel: "error",
  build: { outDir, emptyOutDir: true, minify: false },
  plugins: [{
    name: "carry-blur-host-hook",
    enforce: "pre",
    transform(code, id) {
      if (id !== join(ROOT, "src/mockPageHost.ts")) return;
      const anchor = "  return {\n    async pageWindowReloaded()";
      const submit = "async pageSubmit(_session: number, id: number, key: string) {";
      if (!code.includes(anchor) || !code.includes(submit)) throw new Error("mockPageHost shape changed");
      hooked = true;
      return code
        .replace("admit(id); mail(path, id, view(path), false);", "admit(id); ((globalThis as any).__cbOpens ??= []).push(path); mail(path, id, view(path), false);")
        .replace("const page = read(key);", "const page = (globalThis as any).__cbOverride?.get(key) ?? read(key);")
        .replace(submit, "async pageSubmit(_session: number, id: number, key: string, dto?: PageDto) { (globalThis as any).__cbSubmits.push({ key, dto: JSON.parse(JSON.stringify(dto)) });")
        .replace(anchor, `  (globalThis as any).__cbSubmits = [];
  (globalThis as any).__cbOverride = new Map();
  (globalThis as any).__cbPush = (key: string, dto: PageDto) => {
    const version = (versions.get(key) ?? 1) + 1;
    versions.set(key, version);
    const rev = "ext-" + version;
    (globalThis as any).__cbOverride.set(key, dto);
    for (const listener of listeners) listener({ session, key, answer: null, notice,
      page: { version, conflict: false, risk: false, disk: { kind: "file", rev }, text: { kind: "page", dto: { ...dto, rev } } } });
  };
${anchor}`);
    },
  }],
});
if (!hooked) throw new Error("hook not installed");

const server = spawn(VITE, ["preview", "--outDir", outDir, "--host", "127.0.0.1", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
const url = `http://127.0.0.1:${PORT}/`;
for (let i = 0; i < 40; i++) { try { if ((await fetch(url)).ok) break; } catch { /* starting */ } await sleep(250); }

const blk = (id, raw) => ({ id, raw, collapsed: false, children: [] });
const pageDto = (blocks) => ({ name: PAGE, kind: "page", title: PAGE, pre_block: "icon:: 🏁", format: "md", blocks });

// Each case: initial text, the block to edit, the external text.
const CASES = [
  { name: "B without id::, external edit of B", ids: ["cb-a", "cb-b"],
    before: ["Alpha", "Bravo"], edit: 1, after: ["Alpha", "Bravo EXTERNAL"] },
  { name: "B with id::, external edit of B", ids: ["cb-a", "cb-b"],
    before: ["Alpha", "Bravo\nid:: 6705d2f1-0000-4000-8000-00000000000b"], edit: 1,
    after: ["Alpha", "Bravo EXTERNAL\nid:: 6705d2f1-0000-4000-8000-00000000000b"] },
  { name: "B without id::, external edit of another block", ids: ["cb-a", "cb-b"],
    before: ["Alpha", "Bravo"], edit: 1, after: ["Alpha EXTERNAL", "Bravo"] },
  { name: "B with id::, external edit of another block", ids: ["cb-a", "cb-b"],
    before: ["Alpha", "Bravo\nid:: 6705d2f1-0000-4000-8000-00000000000b"], edit: 1,
    after: ["Alpha EXTERNAL", "Bravo\nid:: 6705d2f1-0000-4000-8000-00000000000b"] },
  { name: "B a code block, external edit of B", ids: ["cb-a", "cb-b"],
    before: ["Alpha", "```js\nlet x = 1\n```"], edit: 1, after: ["Alpha", "```js\nlet x = 2\n```"],
    show: ["Alpha", "let x = 2"] },
  { name: "B with a non-canonical planning line, external edit of B", ids: ["cb-a", "cb-b"],
    before: ["Alpha", "Bravo\nmore\nSCHEDULED: <2026-10-12 Mon>\ntail"], edit: 1,
    after: ["Alpha", "Bravo EXTERNAL\nmore\nSCHEDULED: <2026-10-12 Mon>\ntail"] },
];

let browser;
let failures = 0;
const results = [];
try {
  browser = process.env.CB_ENGINE === "webkit" ? await webkit.launch()
    : await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  console.log(`engine ${browser.browserType().name()} ${browser.version()}`);
  for (const c of CASES) {
    const page = await browser.newPage({ viewport: { width: 1180, height: 820 } });
    page.setDefaultTimeout(8_000);
    const errors = [];
    let phase = "setup";
    page.on("pageerror", (e) => errors.push(`[${phase}] ${e.stack ?? e}`));
    await page.goto(url);
    await page.waitForSelector(".page-title");
    await page.keyboard.press("Control+k");
    await page.locator(".switcher-input").fill(PAGE);
    await sleep(350);
    await page.locator(".switcher-row").first().click();
    await page.waitForFunction((p) => [...document.querySelectorAll(".page-title")].some((e) => e.textContent?.includes(p)), PAGE);
    // Install the case's starting text as the host's disk content; the first
    // edit opens the page and the host's Open answer installs it.
    await page.evaluate(([k, d]) => globalThis.__cbOverride.set(k, d), [PATH, pageDto(c.ids.map((id, i) => blk(id, c.before[i])))]);
    await page.locator(".ls-block .block-content").first().click();
    await page.locator(`.ls-block[data-block-id="${c.ids[0]}"]`).first().waitFor();
    await page.keyboard.press("Escape");
    await page.keyboard.press("Escape");
    await sleep(500);
    const target = page.locator(`.ls-block[data-block-id="${c.ids[c.edit]}"] .block-content`).first();
    await target.waitFor();
    await target.click();
    const editor = page.locator("textarea.block-editor");
    await editor.waitFor();
    await page.evaluate(() => { globalThis.__cbSubmits.length = 0; globalThis.__cbBlurs = 0;
      document.addEventListener("blur", () => { globalThis.__cbBlurs++; }, true); });
    const opened = await editor.inputValue();
    phase = "external";
    // The external change arrives while the editor is open (no input typed).
    await page.evaluate(([k, d]) => globalThis.__cbPush(k, d), [PATH, pageDto(c.ids.map((id, i) => blk(id, c.after[i])))]);
    await sleep(2500); // past the save debounce, so any commit is submitted
    const state = await page.evaluate((ids) => ({
      texts: ids.map((id) => document.querySelector(`.ls-block[data-block-id="${id}"] .block-content`)?.textContent ?? null),
      editorOpen: !!document.querySelector("textarea.block-editor"),
      editorOn: document.querySelector("textarea.block-editor")?.closest(".ls-block")?.getAttribute("data-block-id") ?? null,
      editorValue: document.querySelector("textarea.block-editor")?.value ?? null,
      active: document.activeElement?.tagName ?? null,
      blurs: globalThis.__cbBlurs,
      submits: globalThis.__cbSubmits.map((s) => s.dto?.blocks?.map((b) => b.raw)),
    }), c.ids);
    const visible = (raw) => raw.split("\n").filter((l) => !/^id::/.test(l)).join("\n");
    // Texts as the window holds them: an open editor shows its value, others render.
    const keptExternal = c.after.every((raw, i) => {
      const shown = state.editorOn === c.ids[i] ? state.editorValue : state.texts[i];
      return shown !== null && shown.includes(c.show?.[i] ?? visible(raw));
    });
    const submittedRevert = state.submits.some((blocks) => blocks && blocks.some((raw, i) => raw !== c.after[i]));
    const editorKept = state.editorOpen && state.editorOn === c.ids[c.edit];
    const ok = keptExternal && !submittedRevert && editorKept && errors.length === 0;
    if (!ok) failures++;
    results.push({ case: c.name, opened, ...state, keptExternal, submittedRevert, editorKept, errors, ok });
    await page.close();
  }
} finally {
  await browser?.close();
  server.kill();
  rmSync(outDir, { recursive: true, force: true });
}
console.log(JSON.stringify(results, null, 2));
console.log(failures ? `FAIL: ${failures}/${CASES.length} cases` : `PASS: ${CASES.length}/${CASES.length} cases`);
process.exit(failures ? 1 : 0);
