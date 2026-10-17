// GH #605: literal Customize → ordinary query edit → live journal → delete.
// Run against a custom-protocol debug build under xvfb-run. Fixtures stay local.
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { remote } from "webdriverio";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { openJournals, openPageByName, currentPageTitle } from "./lib/e2e-navigation.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { answerNativeDialog } from "./lib/e2e-native-dialog.mjs";

const root = path.resolve(import.meta.dirname, "..");
const app = process.env.TINE_APP || path.join(root, "target/debug/tine");
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, "artifacts/sections/native"));
fs.mkdirSync(artifacts, { recursive: true });
const commit = process.env.E2E_TESTED_COMMIT || execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
const binarySha256 = createHash("sha256").update(fs.readFileSync(app)).digest("hex");
const port = Number(process.env.E2E_DRIVER_PORT || 4816);
const driverApp = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin/tauri-driver") : "tauri-driver");

for (const [caseIndex, [format, collision]] of [["md", false], ["md", true], ["org", false]].entries()) {
  // Each driver owns a fresh pair, so a terminating WebKit server from the
  // preceding case cannot race the next native-port bind.
  const casePort = port + caseIndex * 2;
  const out = fs.mkdtempSync(path.join(artifacts, `${format}-${collision ? "occupied" : "fresh"}-`));
  const graph = path.join(out, "graph");
  for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
  for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(out, dir));
  const bullet = format === "org" ? "*" : "-";
  const now = new Date();
  const year = now.getFullYear();
  const month = String(now.getMonth() + 1).padStart(2, "0");
  const day = String(now.getDate()).padStart(2, "0");
  const config = `{:tine/guide-announced? true :preferred-format :${format === "org" ? "org" : "markdown"} :default-queries {:journals [{:title "Sections config fixture" :query (task TODO)}]}}\n`;
  const configFile = path.join(graph, "logseq/config.edn");
  fs.writeFileSync(configFile, config);
  fs.writeFileSync(path.join(graph, "journals", `${year}_${month}_${day}.${format}`), `${bullet} Today's fixture journal\n`);
  fs.writeFileSync(path.join(graph, `pages/Tasks.${format}`), `${bullet} TODO Config answer sentinel\n${bullet} DOING Edited answer sentinel\n${bullet} LATER Agenda answer sentinel\n  SCHEDULED: <${year}-${month}-${day}>\n`);
  const occupied = path.join(graph, `pages/Journal sections.${format}`);
  const occupiedBytes = `${bullet} My existing notes must survive\n`;
  if (collision) fs.writeFileSync(occupied, occupiedBytes);
  const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(out, "data"),
    XDG_CONFIG_HOME: path.join(out, "config"), XDG_CACHE_HOME: path.join(out, "cache"),
    WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
  const log = fs.openSync(path.join(out, "driver.log"), "w");
  const driver = spawn(driverApp, ["--port", String(casePort), "--native-port", String(casePort + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
  let browser;
  let journey = "startup";
  const proof = { commit, workingTree: process.env.E2E_TESTED_TREE || "sections diff", app, binarySha256, format, collision, graph, steps: [] };
  const check = (value, message) => { if (!value) throw new Error(message); };
  const mark = (name) => { journey = name; console.log(`${format}/${collision ? "occupied" : "fresh"}: ${name}`); };
  const waitSections = async (selector, include, exclude = []) => browser.waitUntil(() => browser.execute((region, yes, no) => {
    const text = [...document.querySelectorAll(region)].map(node => node.textContent ?? "").join("\n");
    return yes.every(word => text.includes(word)) && no.every(word => !text.includes(word));
  }, selector, include, exclude), { timeout: 20_000, interval: 150, timeoutMsg: `${journey}: sections did not show ${include.join(", ")}` });
  try {
    await browserReady(casePort);
    browser = await remote({ hostname: "127.0.0.1", port: casePort, path: "/", logLevel: "error", connectionRetryCount: 0, connectionRetryTimeout: 30_000,
      capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: app } } });
    await ensureMainWindow(browser);
    await browser.setWindowSize(1200, 900);
    await browser.$(".page-title").waitForExist({ timeout: 30_000 });
    await openJournals(browser);
    // A per-document witness proves all transitions happen without a reload.
    const witness = `${format}-${Date.now()}`;
    await browser.execute(value => { window.__sectionsJourneyWitness = value; }, witness);
    mark("config sections and agenda show before customization");
    await waitSections(".main-content .today-queries", ["Sections config fixture", "Config answer sentinel"]);
    await waitSections(".main-content .agenda-block", ["Agenda answer sentinel"]);
    proof.steps.push(journey);

    mark("Customize creates a seeded page and opens it");
    const customize = await browser.$(".today-sections-customize");
    await customize.scrollIntoView({ block: "center" });
    await customize.click();
    const name = collision ? "Journal sections 2" : "Journal sections";
    await browser.waitUntil(async () => (await currentPageTitle(browser)).toLowerCase() === name.toLowerCase(), { timeout: 20_000, timeoutMsg: "Customize did not open the new page" });
    const fileName = fs.readdirSync(path.join(graph, "pages")).find(file => file.toLowerCase() === `${name}.${format}`.toLowerCase());
    check(fileName, "Customize did not create the page file");
    const file = path.join(graph, "pages", fileName);
    const marker = format === "org" ? "#+tine/journal-sections: true" : "tine/journal-sections:: true";
    const seeded = await waitForFileText(file, text => text.includes(marker) && text.includes("#+BEGIN_QUERY") && text.includes("{{tine-query "));
    check(seeded.includes(':title "Sections config fixture"') && seeded.includes("(task TODO)"), "seed lost the configured query title or expression");
    check(seeded.indexOf("#+BEGIN_QUERY") < seeded.indexOf("{{tine-query "), "seeded agenda precedes config sections");
    if (collision) check(fs.readFileSync(occupied, "utf8") === occupiedBytes, "Customize overwrote the existing default-name page");
    proof.steps.push(journey);

    mark("Settings identifies and opens the active sections page without resetting the dialog");
    await openJournals(browser);
    await waitSections(".main-content .today-sections", ["Config answer sentinel", "Agenda answer sentinel"]);
    await browser.$('button[title^="Settings"]').click();
    await browser.$(".settings-modal").waitForExist({ timeout: 10_000 });
    await browser.execute(() => {
      const journals = [...document.querySelectorAll(".settings-nav-item")].find(node => node.textContent?.trim() === "Journals");
      if (!journals) throw new Error("Journals settings navigation is absent");
      journals.click();
    });
    await browser.waitUntil(() => browser.execute(wanted => {
      const active = document.querySelector(".settings-nav-item.active")?.textContent?.trim();
      const hint = [...document.querySelectorAll(".settings-hint")].find(node => node.textContent?.includes("Today's sections are defined"));
      return active === "Journals" && hint?.textContent?.toLowerCase().includes(wanted.toLowerCase());
    }, name), { timeout: 15_000, interval: 150, timeoutMsg: "Journals settings reset or failed to name the sections page" });
    // Find and click in one call: Settings may re-render the hint between calls.
    await browser.waitUntil(() => browser.execute(() => {
      const hint = [...document.querySelectorAll(".settings-hint")].find(node => node.textContent?.includes("Today's sections are defined"));
      const link = hint?.querySelector("button.settings-link");
      link?.click();
      return !!link;
    }), { timeout: 15_000, interval: 150, timeoutMsg: "Journals settings lost the sections page link" });
    await browser.waitUntil(async () => (await currentPageTitle(browser)).toLowerCase() === name.toLowerCase(), { timeout: 15_000, timeoutMsg: "Settings did not open the active sections page" });
    check(!await browser.$(".settings-modal").isExisting(), "Open page left Settings open");
    proof.steps.push(journey);

    mark("edit the seeded query as an ordinary block");
    // Press the query host itself: result rows and query controls have their own actions.
    await browser.execute(() => {
      const host = [...document.querySelectorAll(".main-content .page-blocks .block-content")]
        .find(node => node.textContent?.includes("Sections config fixture"));
      if (!host) throw new Error("seeded query block is absent");
      host.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
      host.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
    });
    const editor = await browser.$(".main-content textarea.block-editor");
    await editor.waitForExist({ timeout: 10_000 });
    const original = await editor.getValue();
    check(original.includes("(task TODO)"), "entered the wrong block editor");
    await editor.click();
    // Select just the task atom; sending a multiline string through WebKit's
    // keys endpoint drops line breaks and would corrupt the fixture query.
    await browser.execute(() => {
      const editor = document.querySelector(".main-content textarea.block-editor");
      const start = editor.value.indexOf("(task TODO)") + "(task ".length;
      editor.setSelectionRange(start, start + "TODO".length);
    });
    await browser.keys("DOING");
    await browser.$("h1.page-title").click();
    await waitForFileText(file, text => text.includes("(task DOING)") && !text.includes("(task TODO)"));
    await openJournals(browser);
    await waitSections(".main-content .today-sections", ["Edited answer sentinel", "Agenda answer sentinel"], ["Config answer sentinel"]);
    check(await browser.execute(value => window.__sectionsJourneyWitness === value, witness), "journal changed by reloading the app");
    proof.steps.push(journey);

    mark("delete the sections page and restore config sections live");
    // Shared navigation owns switcher activation and title readiness.
    await openPageByName(browser, name);
    await browser.execute(() => document.querySelector("h1.page-title").dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 300, clientY: 200 })));
    await browser.$('[data-page-action-id="delete-page"]').waitForExist({ timeout: 10_000 });
    await browser.execute(() => document.querySelector('[data-page-action-id="delete-page"]').click());
    await answerNativeDialog("yes", { env });
    await browser.waitUntil(() => !fs.existsSync(file), { timeout: 20_000, timeoutMsg: "delete did not remove the sections page file" });
    await openJournals(browser);
    await waitSections(".main-content .today-queries", ["Sections config fixture", "Config answer sentinel"], ["Edited answer sentinel"]);
    await waitSections(".main-content .agenda-block", ["Agenda answer sentinel"]);
    check(await browser.execute(value => window.__sectionsJourneyWitness === value, witness), "fallback needed an app reload");
    check(fs.readFileSync(configFile, "utf8") === config, "customizing changed config.edn bytes");
    if (collision) check(fs.readFileSync(occupied, "utf8") === occupiedBytes, "existing page changed during the journey");
    proof.steps.push(journey);
    console.log(`PASS ${format} collision=${collision}`);
  } catch (error) {
    proof.failure = { journey, expected: "seed, live edit and fallback preserve the user's graph", observed: String(error), classification: browser ? "ambiguous; inspect DOM and driver log" : "infrastructure: driver/session startup" };
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
