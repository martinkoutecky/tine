// GH #735: hover preview size, delay and content in a real browser layout (jsdom has none).
// Serves the app with the in-page mock backend, replaces getPage("Tine") with a long
// page (code fence, callout, quote, collapsed parent), hovers a [[Tine]] link and checks:
//   - nothing opens before Logseq's 1000 ms dwell;
//   - the popup is at most 600px tall (Logseq's cap) in a 1400px window and scrolls inside;
//   - code, callout, quote render as on the page, and a collapsed block hides its children.
// Screenshot: artifacts/hover-popup/peek-popup.png (look at it).
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5197;
const OUT = "artifacts/hover-popup";
mkdirSync(OUT, { recursive: true });
const server = spawn("npx", ["vite", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
async function waitForServer(url, tries = 120) {
  for (let i = 0; i < tries; i++) {
    try { if ((await fetch(url)).ok) return; } catch {}
    await sleep(500);
  }
  throw new Error("server did not start");
}
const fail = [];
const check = (ok, what) => { console.log(ok ? "ok  " : "FAIL", what); if (!ok) fail.push(what); };
let browser;
try {
  await waitForServer(`http://localhost:${PORT}/`);
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  const page = await browser.newPage({ viewport: { width: 1200, height: 1400 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".ls-block", { timeout: 20000 });
  await page.evaluate(async () => {
    const { backend } = await import("/src/backend.ts");
    const b = (id, raw, extra = {}) => ({ id, raw, collapsed: false, children: [], ...extra });
    const blocks = [
      b("p0", "A long page. It has more content than a hover preview should ever show at once."),
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
      name.toLowerCase() === "tine"
        ? { id: "pages/Tine.md", name: "Tine", title: "Tine", kind: "page", format: "md", pre_block: "", blocks }
        : orig(name, kind);
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
  check(box.height <= 601, `popup height ${Math.round(box.height)}px <= 600px in a 1400px window`);
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
  await page.screenshot({ path: `${OUT}/peek-popup-unfolded.png` });
  check(errors.length === 0, `no page errors ${errors.join("; ")}`);
} catch (e) {
  console.error(String(e));
  fail.push(String(e));
} finally {
  await browser?.close();
  server.kill("SIGKILL");
}
process.exit(fail.length ? 1 : 0);
