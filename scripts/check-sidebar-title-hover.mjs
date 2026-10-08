// GH #563: run the actual Sidebar and hover handler with real Chromium layout.
import assert from "node:assert/strict";
import { mkdir, readFile } from "node:fs/promises";
import { basename } from "node:path";
import { createServer } from "vite";
import solid from "vite-plugin-solid";
import { chromium } from "playwright";

const name = "A long page title with 📚 that must remain identifiable";
const fixture = `
import { render } from "/node_modules/solid-js/web/dist/web.js";
import { Sidebar } from "/src/components/Sidebar.tsx";
import { seedFavorites, setFavorites, setRecentPages } from "/src/ui.ts";
import { backend } from "/src/backend.ts";
import { refreshPageIndex } from "/src/pageIndex.ts";
import { initParser } from "/src/render/parse.ts";
import "/src/styles/theme.css";
import "/src/styles/app.css";
import "/src/styles/favorites.css";
const name = ${JSON.stringify(name)};
await initParser();
seedFavorites([name, "Short"]);
setFavorites([{name, kind:"page"}, {name:"Short", kind:"page"}]);
setRecentPages([{name, kind:"page"}, {name:"Short", kind:"page"}]);
backend().pageInventory = async () => ({rev:"1", entries:[name, "Short", "Project/" + name].map((name, i) => ({key:name.toLowerCase(), name, is_journal:false, day:null, target:{kind:"existing", id:"pages/fixture-"+i+".md", others:[]}}))});
await refreshPageIndex();
render(() => Sidebar(), document.getElementById("fixture"));
window.fixtureReady = true;
`;
const server = await createServer({
  configFile: false,
  define: { __BUILD_TIME__: JSON.stringify("fixture"), __GIT_COMMIT__: JSON.stringify(""), __TINE_COMMUNITY_REGISTRY__: "false" },
  server: { host: "127.0.0.1", port: 0, watch: null },
  plugins: [solid(), {
    name: "sidebar-hover-fixture",
    resolveId(id) { if (id === "/qbc-fixture.js") return "\0qbc-fixture"; },
    load(id) { if (id === "\0qbc-fixture") return fixture; },
    configureServer(server) {
      server.middlewares.use(async (req, res, next) => {
        if (req.url?.startsWith("/twemoji/")) {
          res.setHeader("Content-Type", "image/svg+xml");
          return res.end(await readFile(new URL(`../node_modules/@twemoji/svg/${basename(req.url)}`, import.meta.url)));
        }
        if (req.url !== "/qbc-hover") return next();
        res.setHeader("Content-Type", "text/html");
        res.end(await server.transformIndexHtml(req.url, '<div id="fixture" style="width:180px"></div><script type="module" src="/qbc-fixture.js"></script>'));
      });
    },
  }],
});
let browser;
try {
  await server.listen();
  browser = await chromium.launch({ headless: true, args: ["--no-sandbox"] });
  const page = await browser.newPage({ viewport: { width: 1000, height: 1000 } });
  page.on("pageerror", error => console.error(error.message));
  page.on("requestfailed", request => console.error(request.url(), request.failure()?.errorText));
  await page.goto(`http://127.0.0.1:${server.httpServer.address().port}/qbc-hover`);
  await page.waitForFunction(() => window.fixtureReady);
  await page.locator(".nav-section-header").filter({ hasText: "ALL PAGES" }).click();
  await page.locator(".nav-section-header").filter({ hasText: "NAMESPACES" }).click();
  for (const list of ["#sidebar-favorites-list", "#sidebar-recent-list", '.nav-section:has-text("ALL PAGES")', ".ns-tree"]) {
    const label = page.locator(`${list} .nav-page-label`).filter({ hasText: "A long page title with" }).first();
    await label.hover();
    assert.equal(await label.getAttribute("title"), list === ".ns-tree" ? `Project/${name}` : name);
    assert.equal(await label.evaluate(el => el.scrollWidth > el.clientWidth), true);
    await page.mouse.move(900, 900);
    assert.equal(await label.getAttribute("title"), null);
  }
  const long = page.locator("#sidebar-recent-list .nav-page-label").filter({ hasText: "A long page title with" });
  await long.hover();
  await page.locator("#fixture").evaluate(el => { el.style.width = "800px"; });
  await page.waitForFunction(() => document.querySelector("#sidebar-recent-list .nav-page-label").title === "");
  const short = page.locator("#sidebar-recent-list .nav-page-label").filter({ hasText: "Short" });
  await short.hover();
  assert.equal(await short.getAttribute("title"), "");
  await page.locator("#fixture").evaluate(el => { el.style.width = "180px"; });
  await mkdir("artifacts/qbc", { recursive: true });
  await page.screenshot({ path: "artifacts/qbc/sidebar-titles.png" });
  console.log("PASS: favorites, recents, all-pages and namespace titles reveal full names only when truncated; resize clears the hovered tooltip.");
} finally {
  await browser?.close();
  await server.close();
}
