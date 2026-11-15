// GH #926: synthetic Favorites group and Recent rows in the real browser shell.
// Usage: node scripts/shot-leftpanel-926.mjs before|after
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { setTimeout as delay } from "node:timers/promises";
import assert from "node:assert/strict";

const phase = process.argv[2];
assert.ok(["before", "after"].includes(phase));
const out = `target/artifacts/leftpanel-926/${phase}`;
mkdirSync(out, { recursive: true });
const port = 23000 + Math.floor(Math.random() * 10000);
const server = spawn("npx", ["vite", "--port", String(port), "--strictPort"], { stdio: "ignore", detached: true });
let browser;
try {
  for (let attempt = 0; ; attempt++) {
    try { if ((await fetch(`http://localhost:${port}/`)).ok) break; } catch {}
    if (attempt === 80) throw new Error("Vite did not start");
    await delay(250);
  }
  browser = await chromium.launch({ args: ["--no-sandbox"] });
  const observations = [];
  for (const width of [390, 1280]) {
    const page = await browser.newPage({ viewport: { width, height: 844 } });
    await page.emulateMedia({ reducedMotion: "reduce" });
    await page.goto(`http://localhost:${port}/`);
    await page.locator(".page-title").first().waitFor();
    await page.evaluate(async () => {
      const { openPage } = await import("/src/router.ts");
      const { seedFavorites } = await import("/src/favorites.ts");
      const { backend } = await import("/src/backend.ts");
      const { setToasts } = await import("/src/toasts.ts");
      const { setRecentPages, setLeftSidebarOpen } = await import("/src/ui.ts");
      setLeftSidebarOpen(true);
      openPage("Tine", "page");
      const getPage = backend().getPage.bind(backend());
      backend().getPage = async (name, ...args) => name === "Favorites" ? {
        name, kind: "page", rev: "fixture", pre_block: "tine/favorites:: true",
        blocks: [{ id: "group", raw: "Projects", collapsed: false, children: [
          { id: "favorite", raw: "[[Tine]]", collapsed: false, children: [] },
        ] }],
      } : getPage(name, ...args);
      seedFavorites(["Tine"], "Favorites");
      setToasts([]);
      setRecentPages([{ name: "Tine", kind: "page" }, { name: "kitchen-sink", kind: "page" }]);
    });
    if (!await page.locator(".graph-switch-btn").isVisible()) {
      await page.locator('header.topbar button[title^="Toggle sidebar"]').click();
    }
    await page.locator(".nav-fav-group").waitFor();
    await page.evaluate(() => document.fonts.ready);
    const observed = await page.evaluate(() => {
      const rect = selector => {
        const el = document.querySelector(selector);
        const r = el?.getBoundingClientRect();
        return r ? { x: r.x, y: r.y, width: r.width, height: r.height, contentStart: r.x + parseFloat(getComputedStyle(el).paddingInlineStart) } : null;
      };
      return {
        headings: [...document.querySelectorAll(".nav-section-header, .nav-contents > .nav-item")].map(el => {
          const s = getComputedStyle(el);
          return { text: el.textContent, size: s.fontSize, weight: s.fontWeight, spacing: s.letterSpacing };
        }),
        picker: rect(".graph-switch-btn"), close: rect('[aria-label="Close navigation sidebar"]'),
        favorites: rect('[data-sidebar-section="favorites"]'),
        group: rect(".nav-fav-group"), child: rect('#sidebar-favorites-list .nav-page:not(.nav-fav-group)'),
        add: rect(".nav-fav-add-group"), recent: rect("#sidebar-recent-list .nav-page"),
      };
    });
    observations.push({ width, ...observed });
    await page.screenshot({ path: `${out}/${width}.png` });
    if (phase === "after") {
      for (const key of ["size", "weight", "spacing"]) assert.equal(new Set(observed.headings.map(h => h[key])).size, 1, key);
      assert.ok(observed.close, "close icon present");
      assert.ok(Math.abs(observed.close.y + observed.close.height / 2 - observed.picker.y - observed.picker.height / 2) < 1, "close centered on graph picker");
      assert.equal(observed.group.x, observed.add.x, "New group shares child indentation");
      assert.equal(observed.group.x, observed.recent.x, "Recent shares child indentation");
      const step = observed.group.x - observed.favorites.x;
      assert.ok(step > 0, "section children are indented");
      assert.equal(observed.child.contentStart - observed.group.contentStart, step, "nested favorite uses the same indentation step");
      await page.getByRole("button", { name: "Close navigation sidebar", exact: true }).click();
      await page.locator(".graph-switch-btn").waitFor({ state: "hidden" });
    }
    await page.close();
  }
  writeFileSync(`${out}/observations.json`, JSON.stringify(observations, null, 2) + "\n");
  console.log(`Captured ${phase}: phone drawer and desktop sidebar.`);
} finally {
  await browser?.close();
  try { process.kill(-server.pid, "SIGTERM"); } catch {}
}
