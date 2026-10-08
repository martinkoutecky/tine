// Repro/verify: clicking a {{query}} collapse arrow must TOGGLE collapse, not
// enter edit mode of the owning block. Headless Chromium over the mock backend.
//
// The mock's "Jun 14th, 2026" journal exercises BOTH query render paths:
//   - pure query block:  {{query (todo TODO DOING)}}                 → macro-host
//   - inline in a block: All todos + Prio A {{query (and …)}} (id 1002fa7a) → block-content
// Both put a .query-collapse arrow inside a .ls-block; both were regressed by the
// click→caret rewrite (edit entry moved to mousedown, past the arrow's click-phase
// stopPropagation). This drives a real click and checks the arrow toggles.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5209;
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });

async function waitForServer(url, tries = 40) {
  for (let i = 0; i < tries; i++) {
    try { const r = await fetch(url); if (r.ok) return; } catch {}
    await sleep(250);
  }
  throw new Error("server did not start");
}

try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  const page = await browser.newPage({ viewport: { width: 1200, height: 900 }, deviceScaleFactor: 1 });
  page.on("pageerror", (e) => console.log("pageerror:", String(e).split("\n")[0]));

  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".page-title", { timeout: 8000 });
  await sleep(500);
  // Scroll the inline-query block into view so its (lazily deferred) body renders.
  await page.locator('.ls-block[data-block-id="1002fa7a-7164-456c-9e53-3032f783711c"]').scrollIntoViewIfNeeded().catch(() => {});
  await sleep(500);

  // Every query arrow that sits inside a real block is a bug surface.
  const arrows = page.locator(".query-collapse");
  const total = await arrows.count();
  const cases = [];
  for (let i = 0; i < total; i++) {
    const meta = await arrows.nth(i).evaluate((el) => {
      const block = el.closest(".ls-block");
      return {
        inBlock: !!block,
        blockId: block?.getAttribute("data-block-id") ?? null,
        host: el.closest(".macro-host") ? "macro-host" : el.closest(".block-content") ? "block-content" : "page-agenda",
      };
    });
    if (meta.inBlock) cases.push({ i, ...meta });
  }
  console.log(`arrows: ${total} total, ${cases.length} inside a block`);

  let allPass = cases.length > 0;
  for (const c of cases) {
    const arrow = arrows.nth(c.i);
    await arrow.scrollIntoViewIfNeeded();
    const was = await arrow.evaluate((el) => el.classList.contains("collapsed"));
    await arrow.click();
    await sleep(250);
    const res = await page.evaluate((id) => {
      const block = document.querySelector(`.ls-block[data-block-id="${id}"]`);
      if (!block) return { editing: "block-gone", collapsed: null };
      const ta = block.querySelector("textarea, .cm-editor, [contenteditable='true']");
      const arr = block.querySelector(".query-collapse");
      return { editing: ta ? "EDITING" : "not-editing", collapsed: arr ? arr.classList.contains("collapsed") : null };
    }, c.blockId);
    const pass = res.editing === "not-editing" && res.collapsed === !was;
    allPass &&= pass;
    console.log(`  [${c.host}] block ${c.blockId}: collapsed ${was}→${res.collapsed}, editing=${res.editing}  ${pass ? "PASS ✅" : "FAIL ❌"}`);
    // toggle back so a second run starts clean-ish
    await arrow.click().catch(() => {});
    await sleep(150);
  }

  console.log(allPass ? "\nALL PASS ✅ — query arrows toggle collapse, never enter edit" : "\nFAIL ❌");
  await browser.close();
  process.exitCode = allPass ? 0 : 1;
} catch (e) {
  console.log("ERROR:", String(e).split("\n")[0]);
  process.exitCode = 2;
} finally {
  server.kill("SIGTERM");
}
