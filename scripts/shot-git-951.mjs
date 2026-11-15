// Synthetic routed-page proof of the Git section, before and after #951/#952.
import { chromium } from "playwright";
import { createServer } from "vite";
import solid from "vite-plugin-solid";
import { mkdirSync, realpathSync } from "node:fs";
import assert from "node:assert/strict";

const out = "target/artifacts/git-951";
mkdirSync(out, { recursive: true });
const before = process.argv.includes("--before");
const server = await createServer({ configFile: false, plugins: [solid()],
  define: { __BUILD_TIME__: JSON.stringify("1970-01-01T00:00:00Z"), __GIT_COMMIT__: JSON.stringify("git-951"), __TINE_COMMUNITY_REGISTRY__: "true" },
  server: { port: 0, fs: { allow: [process.cwd(), realpathSync("node_modules"), realpathSync("node_modules/inter-ui"), realpathSync("node_modules/katex")] } } });
await server.listen();
let browser;
try {
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  const page = await browser.newPage({ viewport: { width: 1280, height: 1100 } });
  await page.goto(server.resolvedUrls.local[0], { waitUntil: "commit" });
  await page.locator("[data-search-trigger]").waitFor({ timeout: 60000 });
  await page.evaluate(async () => {
    const { backend } = await import("/src/backend.ts");
    backend().gitAutocommitStatus = async () => ({ available: true, repository: true, branch: "main", policy: "commit", interval_minutes: 10,
      sync_unusable: ["no-upstream", "configure one tracked upstream before syncing"],
      last: { commit: "d31e850f01234567", committed_at: 1700000000, gitignore_added: "/example/graph/.gitignore", gitignore_lines: [".syncthing.*", "logseq/.tine-*"] } });
    (await import("/src/router.ts")).openPage("Tine");
  });
  await page.locator(".page-title").filter({ hasText: "Tine" }).first().waitFor();
  await page.getByTitle(/^Settings/).first().click();
  await page.locator(".settings-nav-item").filter({ hasText: /^Graph$/ }).click();
  const section = page.locator('[data-setting-id="git-settings-label"]');
  await section.locator("[data-git-policy]").waitFor();
  await section.scrollIntoViewIfNeeded();
  await section.screenshot({ path: `${out}/${before ? "before" : "after"}.png` });
  if (!before) {
    assert.equal(await section.getByRole("heading").count(), 3);
    assert.match(await section.innerText(), /even while you keep editing/);
    const status = section.getByRole("status").filter({ hasText: "Branch: main" });
    assert.match(await status.innerText(), /Branch: main/);
    const statusColor = await status.evaluate(element => getComputedStyle(element).color);
    const ruleColor = await section.getByText(/even while you keep editing/).evaluate(element => getComputedStyle(element).color);
    assert.notEqual(statusColor, ruleColor, "device status has a visibly different text style");
  }
} finally { await browser?.close(); await server.close(); }
