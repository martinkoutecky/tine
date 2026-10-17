// Share step 1: topbar share -> This page -> Folder -> the export folder holds that page, and the Exports page
// (created on first use) lists the definition. Markdown and Org graphs.
// Run against a custom-protocol debug build under xvfb-run. Fixtures stay local.
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { remote } from "webdriverio";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { openPageByName, currentPageTitle } from "./lib/e2e-navigation.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";

const root = path.resolve(import.meta.dirname, "..");
const app = process.env.TINE_APP || path.join(root, "target/debug/tine");
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, "artifacts/share-dialog/native"));
fs.mkdirSync(artifacts, { recursive: true });
const commit = process.env.E2E_TESTED_COMMIT || execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
const binarySha256 = createHash("sha256").update(fs.readFileSync(app)).digest("hex");
const port = Number(process.env.E2E_DRIVER_PORT || 4836);
const driverApp = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin/tauri-driver") : "tauri-driver");

for (const [caseIndex, format] of ["md", "org"].entries()) {
  // Each driver owns a fresh pair, so a terminating WebKit server from the preceding case cannot race the next bind.
  const casePort = port + caseIndex * 2;
  const out = fs.mkdtempSync(path.join(artifacts, `${format}-`));
  const graph = path.join(out, "graph");
  for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
  for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(out, dir));
  const bullet = format === "org" ? "*" : "-";
  const config = `{:tine/guide-announced? true :preferred-format :${format === "org" ? "org" : "markdown"}}\n`;
  fs.writeFileSync(path.join(graph, "logseq/config.edn"), config);
  fs.writeFileSync(path.join(graph, `pages/Project.${format}`), `${bullet} Shared sentinel text\n`);
  fs.writeFileSync(path.join(graph, `pages/Other.${format}`), `${bullet} Unshared sentinel text\n`);
  const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(out, "data"),
    XDG_CONFIG_HOME: path.join(out, "config"), XDG_CACHE_HOME: path.join(out, "cache"),
    WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
  const log = fs.openSync(path.join(out, "driver.log"), "w");
  const driver = spawn(driverApp, ["--port", String(casePort), "--native-port", String(casePort + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
  let browser;
  let journey = "startup";
  const proof = { commit, workingTree: process.env.E2E_TESTED_TREE || "share-dialog diff", app, binarySha256, format, graph, steps: [] };
  const check = (value, message) => { if (!value) throw new Error(message); };
  const mark = (name) => { journey = name; console.log(`${format}: ${name}`); };
  try {
    await browserReady(casePort);
    browser = await remote({ hostname: "127.0.0.1", port: casePort, path: "/", logLevel: "error", connectionRetryCount: 0, connectionRetryTimeout: 30_000,
      capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: app } } });
    await ensureMainWindow(browser);
    await browser.setWindowSize(1400, 1000);
    await browser.$(".page-title").waitForExist({ timeout: 30_000 });

    mark("the topbar share button is disabled on the journals feed and enabled on a page");
    check(await browser.$(".topbar-share").isExisting(), "no topbar share button");
    check(await browser.execute(() => document.querySelector(".topbar-share").disabled), "share is enabled with no page open");
    await openPageByName(browser, "Project");
    await browser.waitUntil(() => browser.execute(() => !document.querySelector(".topbar-share").disabled), { timeout: 10_000, timeoutMsg: "share stayed disabled on a page" });
    proof.steps.push(journey);

    mark("share this page to a folder");
    await browser.execute(() => document.querySelector(".topbar-share").click());
    await browser.$(".share-dialog").waitForExist({ timeout: 10_000 });
    check(await browser.execute(() => document.querySelector('.share-dialog input[name="share-what"]').checked), "This page is not the default choice");
    // The review resolves the page set through the engine; Export enables once it holds one page.
    await browser.waitUntil(() => browser.execute(() => {
      const button = document.querySelector(".share-dialog .share-export");
      return !!button && !button.disabled && document.querySelector(".share-dialog .query-export-pages")?.textContent?.includes("Project");
    }), { timeout: 20_000, interval: 150, timeoutMsg: "the share review never listed Project" });
    check(!await browser.execute(() => document.querySelector(".share-dialog")?.textContent?.includes("Other")), "the review lists a page that was not chosen");
    await browser.execute(() => document.querySelector(".share-dialog .share-export").click());
    await browser.$(".query-export-modal").waitForExist({ timeout: 10_000 });
    check(!await browser.$(".share-dialog").isExisting(), "the share dialog stayed open over the folder step");
    await browser.waitUntil(() => browser.execute(() => {
      const button = document.querySelector(".query-export-modal .export-btn-primary");
      return !!button && !button.disabled;
    }), { timeout: 20_000, interval: 150, timeoutMsg: "the folder step never enabled Export" });
    await browser.execute(() => document.querySelector(".query-export-modal .export-btn-primary").click());
    const exported = path.join(graph, "published-queries/project");
    const html = await waitForFileText(path.join(exported, "project.html"), text => text.includes("Shared sentinel text"), { timeoutMs: 30_000 });
    check(!html.includes("Unshared sentinel text"), "the export holds a page that was not chosen");
    check(!fs.existsSync(path.join(exported, `other.html`)), "the export holds the other page");
    proof.steps.push(journey);

    mark("the Exports page was created with its marker and lists the definition");
    const file = path.join(graph, "pages", `Exports.${format}`);
    const marker = format === "org" ? "#+tine/exports: true" : "tine/exports:: true";
    const privateLine = format === "org" ? "#+public: false" : "public:: false";
    const text = await waitForFileText(file, value => value.includes("{{tine-query @page and name = 'Project'}}") && value.includes("project"), { timeoutMs: 30_000 });
    check(text.includes(marker) && text.includes(privateLine), `the Exports page lost its marker or public:: false:\n${text}`);
    check(text.includes("(or (property export) (property tine.share))"), "the Exports page has no overview block");
    check(format === "org" ? /:export:\s+project/.test(text) : text.includes("export:: project"), `the definition lacks its export folder:\n${text}`);
    check(fs.readFileSync(path.join(graph, "logseq/config.edn"), "utf8") === config, "sharing changed config.edn");
    proof.steps.push(journey);

    mark("the Exports page opens, shows the definition, and hides its marker");
    await openPageByName(browser, "Exports");
    await browser.waitUntil(async () => (await currentPageTitle(browser)) === "Exports", { timeout: 15_000 });
    await browser.waitUntil(() => browser.execute(() => document.querySelector(".main-content")?.textContent?.includes("Project")), { timeout: 15_000, timeoutMsg: "the Exports page does not list the shared page" });
    check(!await browser.execute(() => document.querySelector(".main-content .page-properties, .main-content")?.textContent?.includes("tine/exports")), "the exports marker shows in the page");
    proof.steps.push(journey);
    console.log(`PASS ${format}`);
  } catch (error) {
    proof.failure = { journey, expected: "share to a folder exports the page and saves a definition on the Exports page", observed: String(error), classification: browser ? "ambiguous; inspect DOM and driver log" : "infrastructure: driver/session startup" };
    if (browser) fs.writeFileSync(path.join(out, "failure-dom.html"), await browser.execute(() => document.body.innerHTML).catch(() => ""));
    throw error;
  } finally {
    fs.writeFileSync(path.join(out, "proof.json"), `${JSON.stringify(proof, null, 2)}\n`);
    await browser?.deleteSession().catch(() => {});
    try { process.kill(-driver.pid, "SIGTERM"); } catch {}
    fs.closeSync(log);
    console.log(`Artifacts: ${out}`);
  }
}

async function browserReady(driverPort) {
  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    try { if ((await fetch(`http://127.0.0.1:${driverPort}/status`)).ok) return; } catch {}
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  throw new Error("tauri-driver did not become ready");
}
