// GH #898: routed production App over a synthetic in-memory graph. Browser
// contexts and HOME/XDG are disposable; no native graph or account is opened.
// STICKY_BASE=<commit> serves the pre-fix modules for the same fail-before check.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { chromium } from "playwright";
import { createServer } from "vite";

const out = path.resolve(process.env.E2E_ARTIFACT_DIR || "target/artifacts/og-embed-sticky-898");
const label = process.env.STICKY_LABEL || (process.env.STICKY_BASE ? "before" : "after");
mkdirSync(out, { recursive: true });
const env = { ...process.env };
for (const [key, dir] of [["HOME", "home"], ["XDG_DATA_HOME", "data"], ["XDG_CONFIG_HOME", "config"], ["XDG_CACHE_HOME", "cache"], ["XDG_STATE_HOME", "state"]]) {
  env[key] = path.join(out, `${label}-${dir}`); mkdirSync(env[key], { recursive: true });
}
const tested = process.env.STICKY_BASE || `${execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim()} + working diff`;
const b = (id, raw, children = []) => ({ id, raw, collapsed: false, children });
const dto = (name, blocks) => ({ name, title: name, kind: "page", pre_block: null, blocks });
const uuid = "89800000-0000-4000-8000-000000000898";
const source = b(uuid, `Source **root**\nid:: ${uuid}`, [b("branch", "Source branch", Array.from({ length: 70 }, (_, i) => b(`leaf-${i}`, `Embedded row ${i}`)))]);
const tail = () => Array.from({ length: 35 }, (_, i) => b(`tail-${i}`, `Following sibling ${i}`));
const fixtures = [
  dto("Sticky source", [source]),
  dto("Sticky page host", [b("intro", "[[Sticky page host]] · [[Sticky block host]]"), b("outer", "Host ancestor", [b("page-host", "{{embed [[Sticky source]]}}")]), ...tail()]),
  dto("Sticky block host", [b("intro-block", "[[Sticky block host]]"), b("block-outer", "Block host ancestor [[Sticky source]]", [b("block-host", `{{embed ((${uuid}))}}`)]), ...tail()]),
];
writeFileSync(path.join(out, "synthetic-graph.json"), JSON.stringify(fixtures, null, 2));
const beforeFiles = ["src/components/Block.tsx", "src/components/StickyAncestor.tsx", "src/editor/caretReveal.ts"];
const server = await createServer({ server: { port: Number(process.env.STICKY_PORT || 5298), strictPort: false }, plugins: [{
  name: "sticky-embed-synthetic-graph", enforce: "pre",
  load(id) {
    const old = beforeFiles.find(name => id.split("?")[0].endsWith(`/${name}`));
    if (old && process.env.STICKY_BASE) return execFileSync("git", ["show", `${process.env.STICKY_BASE}:${old}`], { encoding: "utf8" });
  },
  transform(code, id) {
    const file = id.split("?")[0];
    if (file.endsWith("/src/mock.ts")) return code.replace("export function mockBackend", `NAMED.push(...${JSON.stringify(fixtures)});\nexport function mockBackend`);
  },
}] });
let browser, page;
const results = [], failures = [], errors = [];
let stage = "startup";
try {
  await server.listen();
  browser = await chromium.launch({ env });
  async function fresh(mobile = false) {
    await page?.close();
    page = await browser.newPage({ viewport: mobile ? { width: 390, height: 760 } : { width: 1440, height: 900 }, hasTouch: mobile, isMobile: mobile });
    page.setDefaultTimeout(30_000);
    page.on("pageerror", error => errors.push(String(error)));
    page.on("console", message => { if (message.type() === "error") console.error(message.text()); });
    await page.goto(`http://localhost:${server.httpServer.address().port}/`, { waitUntil: "domcontentloaded", timeout: 60_000 });
    await page.locator(".page-title").first().waitFor({ timeout: 60_000 });
  }
  await fresh();
  async function open(name, scope = ".main-content") {
    await page.keyboard.press("Control+k");
    await page.locator(".switcher-input").fill(name);
    const row = page.locator(".switcher-row:not(.block-result)").filter({ has: page.locator(".switcher-name", { hasText: new RegExp(`^${name}$`) }) });
    await row.first().click();
    await page.locator(`${scope} h1.page-title`).filter({ hasText: name }).first().waitFor();
  }
  async function check(scope, hostId, type, name, { notice = 0, click = true } = {}) {
    stage = name;
    console.log(`Checking ${name}`);
    const host = page.locator(`${scope} [data-block-id="${hostId}"]`).first();
    await host.scrollIntoViewIfNeeded();
    await host.locator('[data-block-id="leaf-20"]').waitFor();
    await host.evaluate((el, reserved) => {
      const root = el.closest(".main-content, .right-sidebar");
      root.style.setProperty("--seen-header-height", `${reserved}px`);
      root.dispatchEvent(new Event("seen-notice-resize"));
      const row = el.querySelector('[data-block-id="leaf-20"] > .block-main');
      root.scrollTop += row.getBoundingClientRect().top - root.getBoundingClientRect().top - 170 - reserved;
    }, notice);
    await page.waitForTimeout(180); // IO paints asynchronously; geometry is the oracle below.
    const g = await host.evaluate((el, { notice, type, uuid }) => {
      const root = el.closest(".main-content, .right-sidebar"), view = root.getBoundingClientRect();
      const pins = [...root.querySelectorAll('.sticky-ancestor button:not([hidden])')].map(button => {
        const holder = button.parentElement, box = button.getBoundingClientRect(), own = holder.parentElement;
        return { id: holder.dataset.stickyBlock, text: button.textContent, top: box.top, bottom: box.bottom,
          subtreeBottom: own.getBoundingClientRect().bottom };
      }).filter(pin => pin.bottom > view.top && pin.top < view.bottom);
      const wanted = type === "page" ? el.dataset.blockId : uuid;
      const pin = pins.find(pin => pin.id === wanted);
      const sourcePin = pins.find(pin => pin.id === uuid);
      const branch = pins.find(pin => pin.id === "branch");
      let depth = 0;
      const own = type === "page" ? el : el.querySelector(`[data-block-id="${uuid}"]`);
      for (let p = own.parentElement; p && p !== root; p = p.parentElement) if (p.hasAttribute("data-sticky-parent")) depth++;
      return { pins, wanted, depth, expectedTop: view.top + notice + depth * 30, viewTop: view.top, viewBottom: view.bottom,
        hostMainBottom: el.querySelector(":scope > .block-main").getBoundingClientRect().bottom,
        pin, sourcePin, branch, embeds: el.querySelectorAll(".embed-block").length,
        childrenOnScreen: [...el.querySelectorAll('[data-block-id^="leaf-"] > .block-main')].some(row => {
          const box = row.getBoundingClientRect(); return box.top < view.bottom && box.bottom > view.top + notice + (depth + 1) * 30;
        }) };
    }, { notice, type, uuid });
    results.push({ name, ...g });
    await page.screenshot({ path: path.join(out, `${label}-${name}.png`) });
    const problems = [];
    if (!g.pin) problems.push("the embed's visible parent bullet scrolled away without a pin");
    if (!g.childrenOnScreen) problems.push("fixture did not keep embedded rows in view");
    if (g.pin && Math.abs(g.pin.top - g.expectedTop) > 1) problems.push("embed pin is not in its ancestor depth slot");
    if (type === "page" && g.pin && (!g.sourcePin || g.sourcePin.top < g.pin.bottom - 1)) problems.push("embedded root does not stack below its host");
    if (g.sourcePin && (!g.branch || g.branch.top < g.sourcePin.bottom - 1)) problems.push("embedded branch does not stack below its root");
    if (g.embeds !== 1) problems.push("pin mounted another live embed");
    if (g.pin && g.hostMainBottom < g.viewBottom) problems.push("fixture did not exercise a tall embedded row");
    if (problems.length) { failures.push({ name, problems }); return; }
    if (click) {
      await host.locator(`.sticky-ancestor[data-sticky-block="${g.wanted}"] > button`).click();
      await page.waitForFunction(({ scope, hostId }) => {
        const el = document.querySelector(`${scope} [data-block-id="${hostId}"]`), root = el.closest(".main-content, .right-sidebar");
        const top = el.getBoundingClientRect().top, view = root.getBoundingClientRect();
        return top >= view.top - 1 && top < view.bottom;
      }, { scope, hostId });
    }
    // The next sibling pushes every pin back inside its own subtree (GH #717).
    const end = await host.evaluate((el, notice) => {
      const root = el.closest(".main-content, .right-sidebar");
      const next = [...root.querySelectorAll(".ls-block")].find(other => !el.contains(other) && !other.contains(el)
        && (el.compareDocumentPosition(other) & Node.DOCUMENT_POSITION_FOLLOWING));
      const row = next?.querySelector(":scope > .block-main");
      if (!row) return false;
      root.scrollTop += row.getBoundingClientRect().top - root.getBoundingClientRect().top - notice - 5;
      return true;
    }, notice);
    if (end) {
      await page.waitForTimeout(100);
      const escaped = await host.evaluate(el => [...el.querySelectorAll('.sticky-ancestor button:not([hidden])')].filter(button =>
        button.getBoundingClientRect().bottom > button.parentElement.parentElement.getBoundingClientRect().bottom + 1).length);
      assert.equal(escaped, 0, `${name}: a pin escaped its subtree`);
      await page.screenshot({ path: path.join(out, `${label}-${name}-end.png`) });
    }
  }
  async function checkCaret() {
    stage = "page-embed-keyboard-caret";
    const row = page.locator('.main-content [data-block-id="page-host"] [data-block-id="leaf-20"] > .block-main .block-content');
    await row.click();
    await page.keyboard.press("Home");
    await page.evaluate(() => {
      const editor = document.activeElement, root = editor.closest(".main-content");
      root.scrollTop += editor.getBoundingClientRect().top - root.getBoundingClientRect().top - 2;
    });
    await page.keyboard.press("ArrowUp");
    await page.waitForFunction(async () => {
      const editor = document.activeElement, root = editor?.closest(".main-content");
      if (!root || editor.closest(".ls-block")?.getAttribute("data-block-id") !== "leaf-19") return false;
      const { editorCaretRow } = await import("/src/editor/caretReveal.ts");
      const g = editorCaretRow(editor, root);
      return g && g.top >= g.visibleTop - 1 && g.bottom <= g.visibleBottom + 1;
    });
    await page.screenshot({ path: path.join(out, `${label}-page-keyboard-caret.png`) });
    await page.keyboard.press("Escape");
    await page.keyboard.press("Escape");
  }
  await open("Sticky page host");
  await check(".main-content", "page-host", "page", "page-desktop");
  if (!process.env.STICKY_BASE) await checkCaret();
  // Literal link gestures create real sidebar and split-pane surfaces.
  await page.locator('.main-content .page-ref[data-ref="sticky page host"]').first().click({ modifiers: ["Shift"] });
  await check(".right-sidebar", "page-host", "page", "page-sidebar", { notice: 46 });
  await page.keyboard.press("Control+Alt+Backslash");
  await page.waitForFunction(() => document.querySelectorAll(".pane-leaf").length === 2);
  await page.evaluate(() => [...document.querySelectorAll(".pane-leaf")].at(-1).setAttribute("data-sticky-probe", "split"));
  const split = '[data-sticky-probe="split"]';
  await page.locator(`${split} .tab`).first().click();
  await check(`${split} .main-content`, "page-host", "page", "page-split");
  await fresh();
  await open("Sticky block host");
  await check(".main-content", "block-host", "block", "block-desktop");
  await page.locator('.main-content .page-ref[data-ref="sticky block host"]').first().click({ modifiers: ["Shift"] });
  await check(".right-sidebar", "block-host", "block", "block-sidebar");
  await page.keyboard.press("Control+Alt+Backslash");
  await page.waitForFunction(() => document.querySelectorAll(".pane-leaf").length === 2);
  await page.evaluate(() => [...document.querySelectorAll(".pane-leaf")].at(-1).setAttribute("data-sticky-probe", "split"));
  await check(`${split} .main-content`, "block-host", "block", "block-split");
  // Linked References renders the macro host through the same Block boundary.
  await fresh();
  await open("Sticky source");
  await page.locator(".main-content .linked-references").scrollIntoViewIfNeeded();
  const linked = page.locator('.main-content .linked-references [data-block-id="page-host"]').first();
  await linked.waitFor();
  await linked.scrollIntoViewIfNeeded();
  await check(".main-content .linked-references", "page-host", "page", "page-linked-references");
  await page.locator(".linked-references .reference-group").filter({ has: page.locator(".reference-page", { hasText: "Sticky block host" }) }).scrollIntoViewIfNeeded();
  await check(".main-content .linked-references", "block-host", "block", "block-linked-references");
  // A fresh context gives the mobile viewport one pane and the default setting.
  await fresh(true);
  await open("Sticky page host");
  await check(".main-content", "page-host", "page", "page-mobile");
  await open("Sticky block host");
  await check(".main-content", "block-host", "block", "block-mobile");
  assert.deepEqual(errors, [], "browser exceptions");
  assert.deepEqual(failures, [], "embed parent bullets must pin while their embedded rows remain visible");
  console.log(`PASS: ${results.length} routed App embed pin geometry cases`);
} catch (error) {
  await page?.screenshot({ path: path.join(out, `${label}-failure.png`) }).catch(() => {});
  writeFileSync(path.join(out, `${label}-failure.json`), JSON.stringify({ tested, journey: stage,
    expected: "Embed parent bullet pins while reading embedded rows; embedded ancestors stack below it",
    observed: String(error.stack), failures, errors, classification: failures.length ? "product" : "ambiguous; inspect harness failure" }, null, 2));
  throw error;
} finally {
  writeFileSync(path.join(out, `${label}-geometry.json`), JSON.stringify({ tested, results, failures, errors }, null, 2));
  await browser?.close();
  await server.close();
}
