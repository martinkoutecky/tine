// GH #671: reporter template, fresh/existing journal, routed page and feed.
// GH671_TEMPLATE_FILE may supply a real templates.md; a synthetic one is the default.
import { execFileSync, spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { remote } from "webdriverio";
import { openJournals, openPageByName } from "./lib/e2e-navigation.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const testedBase = execFileSync("rtk", ["proxy", "git", "rev-parse", "HEAD"], { cwd: ROOT, encoding: "utf8" }).trim();
const ARTIFACTS = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "artifacts/gh671"));
const APP = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
// Default: a synthetic template with the report's shape (page alias, a
// non-parent template with a renderer macro, tab-indented empty children,
// a second template). GH671_TEMPLATE_FILE overrides it with a real file.
const SYNTHETIC_TEMPLATE = [
  "alias:: [[template]]", "",
  "- Daily template", "  template:: Daily Template", "  template-including-parent:: false",
  "\t- {{renderer :todomaster}}", "\t- Prepare", "\t\t- Energy check", "\t\t- Goals", "\t\t\t-",
  "\t- Notes", "\t- Reflect", "\t\t- **Highlights**", "\t\t\t-",
  "- [[Project/Part 2]]", "  template:: Part 2 pipeline", "\t- Step one", "",
].join("\n");
const TEMPLATE = process.env.GH671_TEMPLATE_FILE ? fs.readFileSync(process.env.GH671_TEMPLATE_FILE, "utf8") : SYNTHETIC_TEMPLATE;
const PORT = Number(process.env.E2E_DRIVER_PORT || 4761);
fs.mkdirSync(ARTIFACTS, { recursive: true });
const run = fs.mkdtempSync(path.join(ARTIFACTS, "native-"));
const now = new Date();
const day = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
const outcomes = [];
for (const mode of ["fresh", "existing", "no-template"]) {
  const tmp = path.join(run, mode);
  const graph = path.join(tmp, "graph");
  for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
  for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(tmp, dir), { recursive: true });
  fs.writeFileSync(path.join(graph, "pages/templates.md"), TEMPLATE);
  fs.writeFileSync(path.join(graph, "pages/Other.md"), "- Another page\n");
  fs.writeFileSync(path.join(graph, "logseq/config.edn"), mode === "no-template" ? "{}\n" : '{:default-templates {:journals "Daily Template"}}\n');
  const journal = path.join(graph, "journals", `${day}.md`);
  if (mode === "existing") fs.writeFileSync(journal, "- EXISTING-JOURNAL-SENTINEL\n");
  const env = { ...process.env, TINE_GRAPH: graph,
    XDG_DATA_HOME: path.join(tmp, "data"), XDG_CONFIG_HOME: path.join(tmp, "config"), XDG_CACHE_HOME: path.join(tmp, "cache"),
    WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
  const log = fs.openSync(path.join(tmp, "driver.log"), "w");
  const driver = spawn(process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME || "", "bin/tauri-driver"),
    ["--port", String(PORT), "--native-port", String(PORT + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],
    { env, stdio: ["ignore", log, log], detached: true });
  let browser;
  let stage = "cold startup";
  const connect = () => remote({ hostname: "127.0.0.1", port: PORT, path: "/", logLevel: "error", connectionRetryCount: 2,
    connectionRetryTimeout: 30_000, capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  async function observe() {
    await browser.$("h1.page-title").waitForExist({ timeout: 30_000 });
    if (mode !== "no-template") await browser.waitUntil(() => browser.execute((needle) => document.body.textContent.includes(needle),
      mode === "fresh" ? "Prepare" : "EXISTING-JOURNAL-SENTINEL"), { timeout: 30_000 });
    const error = await browser.$(".page-load-error").isExisting();
    if (error) throw new Error("Journals route displayed a load error");
  }
  try {
    browser = await connect();
    await observe();
    const title = await browser.$("h1.page-title").getText();
    stage = "routed journal page";
    // An unsaved empty day has no search row; saved days use the shared
    // canonical navigation helper and the backend's physical page inventory.
    if (mode === "no-template") await browser.$("h1.page-title").click();
    else await openPageByName(browser, title);
    await browser.$(".page-title, .journal-title").waitForExist({ timeout: 10_000 });
    if (await browser.$(".page-load-error").isExisting()) throw new Error("Routed journal displayed a load error");
    stage = "navigate away and return to Journals";
    await openPageByName(browser, "Other");
    await openJournals(browser);
    await observe();
    stage = "cold reopen";
    await browser.deleteSession();
    browser = await connect();
    await observe();
    if (fs.readFileSync(path.join(graph, "pages/templates.md"), "utf8") !== TEMPLATE) throw new Error("Template source changed");
    if (mode === "existing" && fs.readFileSync(journal, "utf8") !== "- EXISTING-JOURNAL-SENTINEL\n") throw new Error("Existing journal changed");
    if (mode === "fresh" && !fs.readFileSync(journal, "utf8").includes("Prepare")) throw new Error("Template was not saved");
    if (mode === "no-template" && fs.existsSync(journal)) throw new Error("Opening an empty journal wrote a file");
    outcomes.push({ mode, outcome: "passed" });
    console.log(`PASS ${mode}: startup, routed day, return, cold reopen`);
  } catch (error) {
    try { await browser?.saveScreenshot(path.join(tmp, "failure.png")); } catch {}
    try { fs.writeFileSync(path.join(tmp, "failure-dom.html"), await browser.getPageSource()); } catch {}
    fs.writeFileSync(path.join(tmp, "failure.txt"), `Tested: ${testedBase} + working diff\nJourney: ${mode}, ${stage}\nExpected: journal visible, source preserved.\nObserved: ${error.stack}\nClassification: ambiguous; inspect driver and DOM.\n`);
    throw error;
  } finally {
    try { await browser?.deleteSession(); } catch {}
    try { process.kill(-driver.pid, "SIGKILL"); } catch {}
    fs.closeSync(log);
  }
}
fs.writeFileSync(path.join(run, "proof.json"), JSON.stringify({ testedBase, workingDiff: true, app: APP, outcomes }, null, 2));
