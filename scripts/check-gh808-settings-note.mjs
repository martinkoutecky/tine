// GH #808: with a Journal sections page, Settings -> Journals tells where today's sections live and offers
// "Open page". The note sat inside the setting's non-shrinking control column, so it ran off the pane and
// the link was clipped. jsdom has no layout, so this drives the real frontend in Chromium (mock backend; a
// Vite-only transform makes the sections-page lookup find one page) and asserts the link is fully inside
// the visible pane, at desktop and phone widths and for a long unbroken page name.
//   node scripts/check-gh808-settings-note.mjs     (env: GH808_OUT for screenshots)
import { createServer } from "vite";
import { chromium } from "playwright";
import fs from "node:fs";
import path from "node:path";

const out = process.env.GH808_OUT || "target/artifacts/smallui/gh808";
fs.mkdirSync(out, { recursive: true });
const NAMES = { normal: "Journal sections", long: "Journal-sections-" + "customised-for-my-daily-review-".repeat(4) + "page" };
const server = await createServer({
  cacheDir: path.resolve(out, ".vite"),
  server: { port: 31808, strictPort: false, watch: null, fs: { allow: [process.cwd(), path.dirname(fs.realpathSync("node_modules/vite"))] } },
  plugins: [{
    name: "gh808-fixture", enforce: "pre",
    transform(code, id) {
      if (!id.endsWith("/src/journalSections.ts")) return;
      return code.replace("export async function locateSectionsPages(): Promise<string[]> {",
        "export async function locateSectionsPages(): Promise<string[]> {\n  { const n = (globalThis as any).__gh808Name; if (n) return [n]; }");
    },
  }],
});
await server.listen();
const browser = await chromium.launch({ headless: true, args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
const problems = [];
try {
  for (const [vname, viewport] of [["desktop", { width: 1280, height: 800 }], ["narrow", { width: 760, height: 800 }], ["phone", { width: 390, height: 844 }]]) {
    for (const [nname, pageName] of Object.entries(NAMES)) {
      const page = await browser.newPage({ viewport });
      await page.addInitScript((n) => { globalThis.__gh808Name = n; }, pageName);
      page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));
      await page.goto(server.resolvedUrls.local[0]);
      await page.waitForSelector(".page-title");
      await page.locator('button.icon-btn[title^="Settings"]').first().click();
      await page.waitForSelector(".settings-modal");
      await page.locator(".settings-nav-item", { hasText: "Journals" }).first().click();
      const link = page.locator(".settings-link", { hasText: "Open page" });
      await link.waitFor({ timeout: 5000 });
      // Scroll down to the field, never sideways: a user does not scroll a settings pane sideways to find a link.
      await page.evaluate(() => {
        const l = [...document.querySelectorAll(".settings-link")].find((e) => e.textContent.includes("Open page"));
        const b = document.querySelector(".settings-pane-body");
        b.scrollTop = l.getBoundingClientRect().top - b.getBoundingClientRect().top + b.scrollTop - 40;
        b.scrollLeft = 0;
      });
      await page.waitForTimeout(200);
      const m = await page.evaluate(() => {
        const l = [...document.querySelectorAll(".settings-link")].find((e) => e.textContent.includes("Open page"));
        const b = document.querySelector(".settings-pane-body");
        const r = l.getBoundingClientRect(), p = b.getBoundingClientRect();
        const note = l.closest(".settings-hint").getBoundingClientRect();
        return { left: r.left, right: r.right, width: r.width, paneLeft: p.left, paneRight: p.right, noteRight: note.right,
          scrollW: b.scrollWidth, clientW: b.clientWidth };
      });
      const tag = `${vname}/${nname}`;
      await page.locator(".settings-modal").screenshot({ path: `${out}/${tag.replace("/", "-")}.png` });
      const inside = m.left >= m.paneLeft - 0.5 && m.right <= m.paneRight + 0.5;
      const noHScroll = m.scrollW <= m.clientW + 1;
      console.log(tag, inside && noHScroll ? "OK  " : "FAIL", JSON.stringify(m));
      if (!inside) problems.push(`${tag}: Open page is clipped (link ${m.left.toFixed(0)}..${m.right.toFixed(0)}, pane ${m.paneLeft.toFixed(0)}..${m.paneRight.toFixed(0)})`);
      if (!noHScroll) problems.push(`${tag}: the Settings pane scrolls sideways (${m.scrollW} > ${m.clientW})`);
      await page.close();
    }
  }
} finally {
  await browser.close();
  await server.close();
}
if (problems.length) { console.error("FAIL\n" + problems.join("\n")); process.exit(1); }
console.log("PASS");
