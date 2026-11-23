// Repro: an external change delivered while a block on the page is open in the
// editor (the page host pushes new text; `upsertPage` carries the editor).
// Observes, in headless Chromium on the browser mock app (dev server): is the
// external text kept in the window, is the editor kept, and what the window
// submits to the page host afterwards. The external change is the mock host's
// `externalWrite` (the preview's stand-in for the desktop's file watcher, which
// writes through the browser core); submits are captured by wrapping the app's
// backend().pageSubmit in the page. No source is patched.
// Usage: node scripts/repro-carry-blur.mjs   (exit 1 when any case loses data)
// CB_ENGINE=webkit runs Playwright's WebKit (which fires no blur on removal).
import { chromium, webkit } from "playwright";
import { createServer } from "vite";
import { setTimeout as sleep } from "node:timers/promises";
import { resolveRepositoryRoot } from "./docs-preview-paths.mjs";
import { idOf, mockBlockIds } from "./lib/mock-block-ids.mjs";

const ROOT = resolveRepositoryRoot(import.meta.url);
const PAGE = "Formula1/2026";
const PATH = "pages/Formula1___2026.md";

const server = await createServer({ root: ROOT, logLevel: "error",
  server: { host: "127.0.0.1", port: 5231, strictPort: false, watch: null, hmr: false } });
await server.listen();
const url = server.resolvedUrls.local[0];

// The page file's text: its icon, then one root block per raw (continuation lines under the bullet).
const pageText = (raws) => `icon:: 🏁\n\n${raws.map((raw) => `- ${raw.split("\n").join("\n  ")}\n`).join("")}`;

// Each case: initial text, the block to edit, the external text.
const CASES = [
  { name: "B without id::, external edit of B",
    before: ["Alpha", "Bravo"], edit: 1, after: ["Alpha", "Bravo EXTERNAL"] },
  { name: "B with id::, external edit of B",
    before: ["Alpha", "Bravo\nid:: 6705d2f1-0000-4000-8000-00000000000b"], edit: 1,
    after: ["Alpha", "Bravo EXTERNAL\nid:: 6705d2f1-0000-4000-8000-00000000000b"] },
  { name: "B without id::, external edit of another block",
    before: ["Alpha", "Bravo"], edit: 1, after: ["Alpha EXTERNAL", "Bravo"] },
  { name: "B with id::, external edit of another block",
    before: ["Alpha", "Bravo\nid:: 6705d2f1-0000-4000-8000-00000000000b"], edit: 1,
    after: ["Alpha EXTERNAL", "Bravo\nid:: 6705d2f1-0000-4000-8000-00000000000b"] },
  { name: "B a code block, external edit of B",
    before: ["Alpha", "```js\nlet x = 1\n```"], edit: 1, after: ["Alpha", "```js\nlet x = 2\n```"],
    show: ["Alpha", "let x = 2"] },
  { name: "B with a non-canonical planning line, external edit of B",
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
    // Capture what the window submits, and write the case's starting text as
    // the page file's content; the first edit opens the page and the host's
    // Open answer installs it.
    await page.evaluate(async ([k, text]) => {
      // The classic editor this repro observes (CodeMirror is the default).
      (await import("/src/editor/cm6Setting.ts")).setCm6EditorEnabled(false);
      const { backend } = await import("/src/backend.ts");
      const api = backend();
      const submit = api.pageSubmit.bind(api);
      globalThis.__cbSubmits = [];
      api.pageSubmit = (session, id, key, dto, ...rest) => {
        globalThis.__cbSubmits.push({ key, dto: JSON.parse(JSON.stringify(dto)) });
        return submit(session, id, key, dto, ...rest);
      };
      globalThis.__cbPush = (key, text) => api.externalWrite(key, text);
      await globalThis.__cbPush(k, text);
    }, [PATH, pageText(c.before)]);
    // The core names the two root blocks by position; their ids stay across the external edit.
    const ids = (await mockBlockIds(page, [PATH]))[PATH];
    c.ids = c.before.map((raw) => idOf(ids, raw));
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
    await page.evaluate(([k, text]) => globalThis.__cbPush(k, text), [PATH, pageText(c.after)]);
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
  await server.close();
}
console.log(JSON.stringify(results, null, 2));
console.log(failures ? `FAIL: ${failures}/${CASES.length} cases` : `PASS: ${CASES.length}/${CASES.length} cases`);
process.exit(failures ? 1 : 0);
