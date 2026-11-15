// GH #966: an Org page's labelled link `[[Emacs][my favorite editor]]` shows page-reference
// brackets like the plain `[[git]]` link, and follows Show brackets. Routed page, mock backend.
import { chromium } from "playwright";
import { createServer } from "vite";
import { mkdirSync, realpathSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";

const out = resolve(process.argv[2] ?? "artifacts/org-label-brackets-966/after");
mkdirSync(out, { recursive: true });
const server = await createServer({ configFile: "vite.config.ts", cacheDir: `${out}/vite-cache`,
  define: { __BUILD_TIME__: JSON.stringify("1970-01-01T00:00:00Z"), __GIT_COMMIT__: JSON.stringify("org-label-brackets-966"), __TINE_COMMUNITY_REGISTRY__: "true" },
  server: { port: 0, fs: { allow: [process.cwd(), realpathSync("node_modules"), dirname(realpathSync("node_modules/inter-ui"))] } } });
await server.listen();
let browser;
const observed = {}, failures = [];
try {
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  const page = await browser.newPage({ viewport: { width: 900, height: 300 } });
  page.on("pageerror", error => console.error(String(error)));
  await page.goto(`${server.resolvedUrls.local[0]}?regressions`);
  await page.locator("[data-search-trigger]").waitFor();
  await page.evaluate(async () => {
    document.documentElement.dataset.theme = "light";
    (await import("/src/router.ts")).openPage("Org label brackets 966");
  });
  const block = page.locator(".page-blocks .block-content-wrapper").first();
  await block.waitFor();
  await page.mouse.move(850, 250);
  const links = () => page.evaluate(() => [...document.querySelectorAll(".page-blocks .block-content-wrapper a")].map(a => ({ cls: a.className, text: a.textContent })));
  observed.bracketsOn = await links();
  await page.locator(".page-blocks").screenshot({ path: `${out}/brackets-on.png` });
  await page.evaluate(async () => (await import("/src/ui.ts")).changeShowBrackets(false));
  await page.locator(".page-blocks .block-content-wrapper a.page-ref", { hasText: "git" }).waitFor();
  await page.waitForFunction(() => ![...document.querySelectorAll(".page-blocks .bracket")].length);
  observed.bracketsOff = await links();
  await page.locator(".page-blocks").screenshot({ path: `${out}/brackets-off.png` });
  const texts = rows => rows.map(r => r.text);
  const sameKind = rows => rows.every(r => r.cls.includes("page-ref"));
  if (JSON.stringify(texts(observed.bracketsOn)) !== JSON.stringify(["[[git]]", "[[my favorite editor]]"]) || !sameKind(observed.bracketsOn))
    failures.push(`brackets on: ${JSON.stringify(observed.bracketsOn)}`);
  if (JSON.stringify(texts(observed.bracketsOff)) !== JSON.stringify(["git", "my favorite editor"]) || !sameKind(observed.bracketsOff))
    failures.push(`brackets off: ${JSON.stringify(observed.bracketsOff)}`);
  writeFileSync(`${out}/observations.json`, JSON.stringify({ observed, failures }, null, 2) + "\n");
  console.log(JSON.stringify({ observed, failures }, null, 2));
  if (failures.length) process.exitCode = 1;
} finally { await browser?.close(); await server.close(); }
