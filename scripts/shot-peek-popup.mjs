// GH #735/#750/#752: hover preview size, content, placement and sticky source actions.
// Serves the app with the in-page mock backend, replaces getPage("Tine") with a long
// page (code fence, callout, quote, collapsed parent), hovers a [[Tine]] link and checks:
//   - nothing opens before Logseq's 1000 ms dwell;
//   - the popup respects max(600px, half the window), available space, and scrolls inside;
//   - code, callout, quote render as on the page, and a collapsed block hides its children.
// Also covers page/block refs in main/sidebar, top/middle/bottom, light/dark,
// 1280×720 and 1920×1080. Screenshots: artifacts/peek-popup/ (look at them).
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = Number(process.env.PEEK_SHOT_PORT || 52754);
const OUT = process.env.PEEK_SHOT_OUT || "artifacts/peek-popup";
mkdirSync(OUT, { recursive: true });
const server = spawn(process.execPath, ["node_modules/vite/bin/vite.js", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
async function waitForServer(url, tries = 120) {
  for (let i = 0; i < tries; i++) {
    try { if ((await fetch(url, { signal: AbortSignal.timeout(2000) })).ok) return; } catch {}
    await sleep(500);
  }
  throw new Error("server did not start");
}
const fail = [];
const check = (ok, what) => { console.log(ok ? "ok  " : "FAIL", what); if (!ok) fail.push(what); };
let browser;
let page;
try {
  await waitForServer(`http://localhost:${PORT}/`);
  browser = await chromium.launch({ executablePath: process.env.PEEK_BROWSER_EXECUTABLE, args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  page = await browser.newPage({ viewport: { width: 1200, height: 1400 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".ls-block", { timeout: 20000 });
  for (const close of await page.locator('.toast-close').all()) await close.click();
  await page.evaluate(async () => {
    const { backend } = await import("/src/backend.ts");
    const b = (id, raw, extra = {}) => ({ id, raw, collapsed: false, children: [], ...extra });
    const blocks = [
      b("p0", "A long page. [[Tine]] ((58900000-0000-4000-8000-0000000000b1))"),
      b("code", "A snippet\n```rust\nfn main() {\n    println!(\"hello, tine\");\n}\n```"),
      b("note", "#+BEGIN_NOTE\nA callout that renders as one.\n#+END_NOTE"),
      b("quote", "#+BEGIN_QUOTE\nTo be, or not to be.\n#+END_QUOTE"),
      b("mdquote", "> a markdown quote"),
      b("fold", "Folded parent\ncollapsed:: true", { collapsed: true, children: [b("hid", "child hidden by the fold")] }),
      ...Array.from({ length: 60 }, (_, i) => b(`l${i}`, `Filler line ${i} with enough words to look like ordinary prose in a page.`)),
    ];
    const real = backend();
    const orig = real.getPage.bind(real);
    real.getPage = async (name, kind) =>
      name.toLowerCase() === "tine" || name === "Peek fixture"
        ? { id: `pages/${name}.md`, name, title: name, kind: "page", format: "md", pre_block: "", blocks }
        : orig(name, kind);
    real.previewBlock = async () => ({ truncated: 0, group: { page: "Tine", kind: "page", blocks } });
  });
  const link = page.locator("a.page-ref:visible", { hasText: /^\[\[tine\]\]$/i }).first();
  await link.waitFor({ timeout: 10000 });
  await link.scrollIntoViewIfNeeded();
  await link.hover();
  await sleep(700);
  check((await page.locator(".peek-popup").count()) === 0, "no popup 700 ms after hover (Logseq waits 1000 ms)");
  await page.waitForSelector(".peek-popup .ls-block", { timeout: 4000 });
  await sleep(300);
  const popup = page.locator(".peek-popup");
  const box = await popup.boundingBox();
  const scrolls = await popup.evaluate((el) => el.scrollHeight > el.clientHeight + 1);
  check(box.height <= 701, `popup height ${Math.round(box.height)}px <= 700px in a 1400px window`);
  check(scrolls, "popup scrolls inside instead of growing");
  check((await popup.locator("pre.code-block").count()) === 1, "fenced code renders as a code block");
  const pre = await popup.locator("pre.code-block").evaluate((el) => getComputedStyle(el).whiteSpace);
  check(pre.startsWith("pre"), `code block keeps line breaks (white-space: ${pre})`);
  check((await popup.locator(".callout.callout-note").count()) === 1, "admonition renders as a callout");
  check((await popup.locator("blockquote.md-quote").count()) >= 2, "quotes render as blockquotes");
  check(!(await popup.innerText()).includes("child hidden by the fold"), "collapsed block hides its children");
  await page.screenshot({ path: `${OUT}/peek-popup.png` });
  await popup.locator(".ls-block", { hasText: "Folded parent" }).locator(".collapse-toggle").first().click();
  await sleep(150);
  check((await popup.innerText()).includes("child hidden by the fold"), "the fold arrow opens it locally");
  check((await popup.boundingBox()).y === box.y, "opening content keeps the popup's placement");
  await page.screenshot({ path: `${OUT}/peek-popup-unfolded.png` });
  await page.mouse.move(0, 0);
  await page.waitForSelector(".peek-popup", { state: "detached" });
  await page.evaluate(async () => {
    const { openInNewTab } = await import("/src/router.ts");
    openInNewTab({ kind: "page", name: "Peek fixture", pageKind: "page" }, true);
  });
  await page.waitForSelector('.main-content .block-ref');
  await page.evaluate(async () => {
    const { openPageInSidebar } = await import("/src/ui.ts");
    openPageInSidebar("Peek fixture");
  });
  await page.waitForSelector('.right-sidebar .block-ref');
  for (const viewport of [{ width: 1280, height: 720 }, { width: 1920, height: 1080 }]) {
    await page.setViewportSize(viewport);
    for (const theme of ["light", "dark"]) {
      await page.evaluate(theme => { document.documentElement.dataset.theme = theme; }, theme);
      for (const pane of ["main-content", "right-sidebar"]) {
        for (const kind of ["page-ref", "block-ref"]) {
          for (const [position, y] of [["top", 60], ["middle", viewport.height / 2], ["bottom", viewport.height - 60]]) {
            const anchor = page.locator(`.${pane} .${kind}`).first();
            await anchor.evaluate((el, y) => {
              const pane = el.closest('.main-content, .right-sidebar').getBoundingClientRect();
              el.style.cssText = `position:fixed;top:${y}px;left:${pane.left + 40}px;z-index:40`;
            }, y);
            await anchor.hover();
            await page.waitForSelector('.peek-popup .ls-block');
            const card = page.locator('.peek-popup');
            const initial = await card.boundingBox();
            const trigger = await anchor.boundingBox();
            const label = `${viewport.height}-${theme}-${pane}-${kind}-${position}`;
            check(initial.y + initial.height <= trigger.y || initial.y >= trigger.y + trigger.height, `${label}: anchor clear`);
            check(initial.x >= 0 && initial.x + initial.width <= viewport.width && initial.y >= 0 && initial.y + initial.height <= viewport.height, `${label}: within viewport`);
            await card.hover();
            const title = card.locator('.peek-popup-title');
            await card.evaluate(el => { el.scrollTop = el.scrollHeight; });
            const titleBox = await title.boundingBox();
            check(titleBox.y >= initial.y && titleBox.y + titleBox.height <= initial.y + initial.height, `${label}: scrolled title visible`);
            check(titleBox.y <= initial.y + 2, `${label}: no scrolling gap above title`);
            const opaque = await title.evaluate(el => getComputedStyle(el).backgroundColor);
            check(opaque !== 'rgba(0, 0, 0, 0)' && opaque !== 'transparent', `${label}: opaque title`);
            if (kind === "page-ref" && position === "middle") {
              await page.screenshot({ path: `${OUT}/${label}-scrolled.png` });
            }
            await page.mouse.move(0, 0);
            await page.waitForSelector('.peek-popup', { state: 'detached' });
            await anchor.evaluate(el => el.style.cssText = '');
          }
        }
      }
    }
  }
  check(errors.length === 0, `no page errors ${errors.join("; ")}`);
} catch (e) {
  console.error(String(e));
  fail.push(String(e));
  await page?.screenshot({ path: `${OUT}/failure.png` }).catch(() => {});
} finally {
  await browser?.close();
  server.kill("SIGKILL");
}
process.exit(fail.length ? 1 : 0);
