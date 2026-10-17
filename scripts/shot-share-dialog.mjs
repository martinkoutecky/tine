// Screenshot the share dialog in light and dark: "This page" with a left-out page and the host-page note, and
// "A query starting at this page" with the builder and live preview. Headless Chromium over the mock backend on the
// vite dev server (the page imports the app's own modules to give the mock the desktop engine's publication plan,
// which the browser mock refuses by design).
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5287;
const OUT = process.env.SHOT_DIR || "screenshots";
mkdirSync(OUT, { recursive: true });
const server = spawn("npx", ["vite", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });

async function waitForServer(url, tries = 120) {
  for (let i = 0; i < tries; i++) {
    try { if ((await fetch(url)).ok) return; } catch {}
    await sleep(250);
  }
  throw new Error("server did not start");
}

try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  const page = await browser.newPage({ viewport: { width: 1100, height: 1000 }, deviceScaleFactor: 2 });
  const errors = [];
  const step = (name) => console.log(`${new Date().toISOString().slice(11, 19)} ${name}`);
  page.on("console", (m) => m.type() === "error" && console.log("console error:", m.text().slice(0, 200)));
  page.on("pageerror", (e) => errors.push(String(e).split("\n")[0]));
  step("goto");
  await page.goto(`http://localhost:${PORT}/`, { timeout: 240_000 });
  step("wait title");
  await page.waitForSelector(".page-title", { timeout: 120_000 });
  await sleep(500);
  await page.evaluate(async () => {
    const { backend } = await import("/src/backend.ts");
    const api = backend();
    // The browser mock cannot print or parse a query IR (it refuses by design); give it the desktop engine's answers for the selector.
    const selector = (name) => ({ anchor: "page", diagnostics: [], source: { kind: "builder" },
      filter: { kind: "leaf", leaf: { kind: "attr", attr: "name", op: "eq", value: { kind: "text", text: name } } } });
    api.printQuery = async (query) => `@page and name = '${query.filter?.leaf?.value?.text ?? "Project"}'`;
    api.parseQuery = async (text) => ({ query: selector(/name = '([^']*)'/.exec(text)?.[1] ?? "Project"), view: {} });
    api.publishQueryPlan = async () => ({
      anchor: "page", rowCount: 2, folder: "project", fingerprint: "shot", path: "/home/me/graph/published-queries/project", exists: false, suggestedFolder: null,
      pages: [{ name: "Project", path: "pages/Project.md", journal: false }, { name: "Project plan", path: "pages/Project plan.md", journal: false }],
      leftOut: [{ name: "Diary", path: "pages/Diary.md", reason: "private" }, { name: "Exports", path: "pages/Exports.md", reason: "exportsPage" }],
      selectsHostPage: true,
    });
  });
  step("patched plan");
  for (const theme of ["light", "dark"]) {
    step(theme);
    await page.evaluate((value) => document.documentElement.setAttribute("data-theme", value), theme);
    await page.evaluate(async () => { (await import("/src/shareState.ts")).openShareDialog({ page: "Project" }); });
    try {
      await page.waitForSelector(".share-dialog .share-export:not([disabled])", { timeout: 15_000 });
    } catch (error) {
      await page.screenshot({ path: `${OUT}/share-dialog-failure-${theme}.png` });
      console.log("dialog text:", (await page.locator(".share-dialog").innerText().catch(() => "(no dialog)")).replace(/\n+/g, " | ").slice(0, 600));
      throw error;
    }
    await sleep(400);
    await page.locator(".share-dialog").screenshot({ path: `${OUT}/share-dialog-page-${theme}.png` });
    await page.locator('.share-dialog input[name="share-what"]').nth(2).check();
    await sleep(900);
    await page.locator(".share-dialog").screenshot({ path: `${OUT}/share-dialog-query-${theme}.png` });
    await page.evaluate(async () => { (await import("/src/shareState.ts")).closeShareDialog(); });
    await sleep(200);
  }
  console.log(errors.length ? "ERRORS:\n" + errors.join("\n") : "no page errors");
  await browser.close();
  console.log("DONE");
} catch (e) {
  console.log("ERROR:", String(e).split("\n").slice(0, 3).join(" | "));
  process.exitCode = 2;
} finally {
  server.kill("SIGTERM");
}
