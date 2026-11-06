// GH #863 / #809 diagnostic journeys for Linux WebKitGTK and Windows WebView2.
// ONLY=863 or ONLY=809 runs one report, so a native freeze cannot hide the other.
// #863 lifecycle cases: 863-created, 863-created-blur,
// 863-created-unreferenced, 863-external, 863-external-alternate,
// 863-external-alternate-blur, 863-custom, 863-crlf, 863-directory-case,
// 863-upper-extension, and 863-edit (typing before any title open).
// Each starts a fresh fixture. Existing-file cases
// also save successive edits from Journals while the editor remains open.
// Fixture data only. A passing Linux run does not establish a Windows fix.
import { execFileSync, spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { x11Tools } from "./lib/e2e-x11.mjs";
import { startWebdriverApplication, stopWebdriverApplication, tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

if (!["linux", "win32"].includes(process.platform)) throw new Error("This probe supports Linux and Windows");
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const out = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, "artifacts/fix-beta-win-863-809", `native-${process.pid}`));
const run = path.join(out, `fixture-${process.pid}`);
const graph = path.join(run, "Graph with spaces é");
const app = path.resolve(process.env.TINE_APP || path.join(root, "target/debug", process.platform === "win32" ? "tine.exe" : "tine"));
const port = Number(process.env.E2E_DRIVER_PORT || 4672);
const nativePort = Number(process.env.E2E_NATIVE_PORT || 4673);
const issues = process.env.ONLY ? process.env.ONLY.split(",") : ["863", "809"];
// Run each journal lifecycle independently with ONLY=863-<case>.
const journalCase = issues.find(issue => issue.startsWith("863")) ?? "863";
const creates = journalCase.startsWith("863-created");
const external = journalCase.startsWith("863-external");
const custom = journalCase === "863-custom";
const directoryCase = journalCase === "863-directory-case";
if (directoryCase && process.platform !== "win32") throw new Error("863-directory-case requires a Windows case-insensitive volume");
const virtual = creates || external;
const referenced = virtual && journalCase !== "863-created-unreferenced";
const journalsDirectory = directoryCase ? "Journals" : "journals";
for (const dir of ["pages", journalsDirectory, "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state", "appdata", "localappdata", "webview"]) fs.mkdirSync(path.join(run, dir), { recursive: true });
const now = new Date();
const day = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(now.getDate()).padStart(2, "0")}`;
const journalRel = `${journalsDirectory}/${custom ? day : day.replaceAll("-", "_")}.${journalCase === "863-upper-extension" ? "MD" : "md"}`;
const journalFile = path.join(graph, journalRel);
const journalBytes = `- GH863 existing journal body${journalCase === "863-crlf" ? "\r\n" : "\n"}`;
const pageName = "Probe Alpha";
const pageFile = path.join(graph, `pages/${pageName}.md`);
fs.writeFileSync(path.join(graph, "logseq/config.edn"), `{:journal/page-title-format "yyyy-MM-dd"${custom ? ' :journal/file-name-format "yyyy-MM-dd"' : ""}}\n`);
if (!virtual) fs.writeFileSync(journalFile, journalBytes);
fs.writeFileSync(pageFile, "- GH809 original page body\n");
if (referenced) fs.writeFileSync(path.join(graph, "pages/Journal mentions.md"), `- [[${day}]]\n`);
const fixture = accountHomeFixture(run, out);
// The shared Windows launcher takes its profile root from the harness process.
process.env.E2E_WEBVIEW_USER_DATA_ROOT = path.join(run, "webview");
const env = { ...fixture.env, TINE_GRAPH: graph, HOME: path.join(run, "home"),
  XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"),
  XDG_CACHE_HOME: path.join(run, "cache"), XDG_STATE_HOME: path.join(run, "state"),
  APPDATA: path.join(run, "appdata"), LOCALAPPDATA: path.join(run, "localappdata"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const report = { testedCommit: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
  journey: "scripts/e2e-beta-win-863-809.mjs", platform: process.platform, app, issues, stage: "launch", steps: [], status: "running",
  expected: "Existing journal opens; a page search result opens in a usable window and main stays responsive" };
const write = () => fs.writeFileSync(path.join(out, "result.json"), JSON.stringify(report, null, 2) + "\n");
const step = (stage, detail) => { report.stage = stage; report.steps.push({ stage, detail }); write(); console.log(`ok: ${stage}`); };
write();
let target, driver, browser, log, wm;
const { xdo } = x11Tools(env);
async function shot(name) {
  // WebKitWebDriver cannot screenshot a related popup (its automation flag
  // is ignored). Capture the actual X11 display instead.
  try {
    if (process.platform === "linux") execFileSync("import", ["-window", "root", path.join(out, name)], { env, timeout: 5000 });
    else await browser.saveScreenshot(path.join(out, name));
  } catch (error) { console.log(`Screenshot unavailable: ${String(error)}`); }
}
async function diagnostics() {
  const result = await browser.executeAsync(done => {
    window.__TAURI_INTERNALS__.invoke("diagnostic_report", { buildCommit: "", buildTime: "", renderer: null })
      .then(value => done({ text: value.text }), error => done({ error: String(error) }));
  });
  if (result.error) throw new Error(`Diagnostic capture failed: ${result.error}`);
  return JSON.parse(result.text);
}
async function captureJournalProbe(name) {
  const evidence = await browser.execute(() => window.__gh863Probe);
  fs.writeFileSync(path.join(out, name), JSON.stringify(evidence, null, 2) + "\n");
  return evidence;
}
async function assertNoJournalConflict() {
  const evidence = await captureJournalProbe("journal-ipc.json");
  if (evidence.mail.some(mail => mail.conflict || mail.conflictReported) || evidence.warnings.length) {
    report.classification = "product";
    throw new Error("GH863 unchanged journal edit reported a conflict; see journal-ipc.json");
  }
}
function stop() {
  if (driver?.pid) {
    if (process.platform === "win32") spawnSync("taskkill", ["/PID", String(driver.pid), "/T", "/F"], { stdio: "ignore" });
    else { try { process.kill(-driver.pid, "SIGKILL"); } catch { /* already exited */ } }
  }
  stopWebdriverApplication(target);
  wm?.kill();
}
// A frozen UI/driver must still leave a bounded failure capsule on disk.
const watchdog = setTimeout(() => {
  report.status = "failed";
  report.classification = "ambiguous";
  report.observed = `No completion within 150 seconds; last stage: ${report.stage}`;
  write(); stop(); fixture.close?.(); process.exit(1);
}, 150_000);
try {
  if (process.platform === "linux") {
    wm = spawn(process.env.E2E_WINDOW_MANAGER || "openbox", ["--sm-disable"], { env, stdio: "ignore" });
  }
  target = await startWebdriverApplication(app, env, nativePort, "gh863-809");
  log = fs.openSync(path.join(out, "driver.log"), "w");
  driver = spawn(process.env.TAURI_DRIVER || "tauri-driver",
    webdriverServerArgs(port, nativePort, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
    { env: target.env, stdio: ["ignore", log, log], detached: process.platform !== "win32" });
  await sleep(2000);
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error",
    connectionRetryCount: 0, connectionRetryTimeout: 15_000,
    capabilities: tauriCapabilities(app, "gh863-809", process.platform, target.debuggerAddress)
  });
  await ensureMainWindow(browser);
  await browser.$(".ls-block").waitForExist({ timeout: 30_000 });
  await assertAppLease(fixture);
  const main = await browser.getWindowHandle();
  step("app-ready", await browser.execute(() => ({ title: document.title, ua: navigator.userAgent })));

  if (issues.some(issue => issue.startsWith("863"))) {
    // Observe the real UI's calls and host mail; issue no synthetic opens or
    // submits, which would change session/request ordering and hide the bug.
    // All identities are from this disposable fixture, never the reporter's graph.
    await browser.executeAsync(done => {
      const internals = window.__TAURI_INTERNALS__;
      const evidence = window.__gh863Probe = { ipc: [], mail: [], warnings: [] };
      const keep = (list, value) => { list.push({ at: performance.now(), ...value }); if (list.length > 2000) list.shift(); };
      const page = dto => dto && ({ id: dto.id, name: dto.name, kind: dto.kind, rev: dto.rev, format: dto.format });
      const invoke = internals.invoke.bind(internals);
      const watched = new Set(["get_page", "get_page_by_path", "journal_feed_page", "resolve_page", "page_open", "page_submit", "page_close"]);
      internals.invoke = async (command, args, options) => {
        if (!watched.has(command)) return invoke(command, args, options);
        const request = { command, session: args?.session, id: args?.id, path: args?.path, name: args?.name, kind: args?.kind,
          pages: args?.pages, key: args?.key, version: args?.version, dto: page(args?.dto) };
        try {
          const result = await invoke(command, args, options);
          const reply = command === "journal_feed_page" ? { pages: result.pages.map(page), unreadable: result.unreadable }
            : command === "get_page" || command === "get_page_by_path" ? page(result) : result;
          keep(evidence.ipc, { ...request, reply });
          return result;
        } catch (error) { keep(evidence.ipc, { ...request, error: String(error) }); throw error; }
      };
      const observer = new MutationObserver(() => {
        const text = [...document.querySelectorAll(".conflict-banner")].map(el => el.textContent).join("\n");
        if (text && evidence.warnings.at(-1)?.text !== text) keep(evidence.warnings, { text });
      });
      observer.observe(document.body, { subtree: true, childList: true, characterData: true });
      invoke("plugin:event|listen", { event: "page-mail", target: { kind: "Any" },
        handler: internals.transformCallback(event => {
          const mail = event.payload;
          keep(evidence.mail, { key: mail.key, session: mail.session, version: mail.page?.version,
            conflict: mail.page?.conflict, risk: mail.page?.risk, disk: mail.page?.disk,
            text: mail.page?.text?.kind, dto: page(mail.page?.text?.dto), answer: mail.answer,
            conflictReported: mail.notice?.conflictReported, notice: mail.notice });
        }) }).then(() => done(null), error => done(String(error)));
    }).then(error => { if (error) throw new Error(`Could not observe page mail: ${error}`); });
    await openPageByName(browser, pageName); // The route starts on an ordinary page.
    const clicked = await browser.execute(() => {
      const button = document.querySelector('.topbar-optional-action[title="Journals"]');
      button?.click(); return !!button;
    });
    if (!clicked) throw new Error("Journals toolbar button was absent");
    await browser.waitUntil(() => browser.execute(() => !!document.querySelector("h1.journal-title")), { timeout: 15_000 });
    let openedFile = journalFile;
    let expectedBytes = journalBytes;
    let expectedBody = "GH863 existing journal body";
    if (virtual) {
      // References finishing proves the native completion that admits virtual
      // referenced days has arrived, rather than just frontend withToday().
      if (referenced) await browser.waitUntil(() => browser.execute(() =>
        [...document.querySelectorAll(".references-count")].some(el => el.textContent.trim() === "1")), { timeout: 15_000 });
      if (fs.existsSync(journalFile)) throw new Error("Virtual fixture unexpectedly already has a journal file");
      await browser.$(".main-content .page-section .block-content-wrapper").click();
      const editor = await browser.$(".main-content textarea.block-editor");
      await editor.waitForExist({ timeout: 10_000 });
      if (creates) {
        expectedBody = "GH863 created in feed";
        await editor.addValue(expectedBody);
        expectedBytes = await waitForFileText(journalFile, text => text.includes(expectedBody));
      } else {
        const before = (await diagnostics()).sessions.current.filter(event => event.event === "watcher.batch").length;
        if (journalCase.startsWith("863-external-alternate")) openedFile = path.join(graph, `journals/${day}.md`);
        expectedBody = "GH863 externally created journal";
        expectedBytes = `- ${expectedBody}\n`;
        fs.writeFileSync(openedFile, expectedBytes);
        await browser.waitUntil(async () =>
          (await diagnostics()).sessions.current.filter(event => event.event === "watcher.batch").length > before,
        { timeout: 15_000, interval: 500, timeoutMsg: "External journal creation was not observed by the watcher" });
      }
      if (journalCase.endsWith("-blur")) await browser.keys(["Escape"]);
      step("863-file-created", { journalCase, openedFile, editorActive: await editor.isExisting(), expectedBytes });
      fs.writeFileSync(path.join(out, "before-click-diagnostics.json"), JSON.stringify(await diagnostics(), null, 2));
    }
    if (journalCase !== "863-edit") {
      step("863-before-title-click", await browser.execute(() => ({
        titles: [...document.querySelectorAll("h1.journal-title")].map(el => el.textContent?.trim()),
        body: document.querySelector(".main-content")?.textContent,
      })));
      await browser.execute((title) => {
        const heading = [...document.querySelectorAll("h1.journal-title")].find(el => el.textContent?.trim() === title);
        if (!heading) throw new Error(`Journal title ${title} absent`);
        heading.click();
      }, day);
      let outcome;
      await browser.waitUntil(async () => {
        outcome = await browser.execute((body) => {
          if (document.body.textContent.includes("no longer available at that path")) return "refused";
          return !document.querySelector(".main-content .journal-feed-day") && !!document.querySelector("h1.page-title")
            && document.querySelector(".main-content")?.textContent.includes(body) ? "opened" : null;
        }, expectedBody);
        return outcome !== null;
      }, { timeout: 15_000, timeoutMsg: "Journal title did not open the existing body on its own route" });
      if (outcome === "refused") { report.classification = "product"; throw new Error("GH863 physical-page refusal"); }
      if (fs.readFileSync(openedFile, "utf8") !== expectedBytes) throw new Error("Opening the journal changed its bytes");
      if (virtual && fs.readdirSync(path.join(graph, "journals")).join() !== path.basename(openedFile)) throw new Error("Virtual journal opening created an extra file");
      await shot("863-routed-journal.png");
      step("863-pass", "Toolbar Journals > today's title opened its existing file without changing bytes");
    }
    if (!virtual) {
      if (journalCase !== "863-edit") await browser.$('.topbar-optional-action[title="Journals"]').click();
      const body = await browser.$(`[data-feed-day="${day}"] .block-content-wrapper`);
      await body.waitForExist({ timeout: 15_000 });
      await body.click();
      const editor = await browser.$(".main-content textarea.block-editor");
      await editor.waitForExist({ timeout: 10_000 });
      await browser.keys(["Control", "End"]);
      let typed = await editor.getValue();
      for (const character of ["a", "b", "c"]) {
        typed += character;
        await editor.addValue(character);
        // Keep the editor active and prove each edit reaches the same file.
        // A rapid burst alone can coalesce into one submit and miss this report.
        await waitForFileText(openedFile, text => text.includes(typed));
        await assertNoJournalConflict();
        step("863-feed-edit-saved", { character, bytes: fs.statSync(openedFile).size });
      }
      expectedBytes = fs.readFileSync(openedFile, "utf8");
      await browser.execute(title => {
        [...document.querySelectorAll("h1.journal-title")].find(el => el.textContent?.trim() === title)?.click();
      }, day);
      await browser.waitUntil(() => browser.execute(text =>
        !document.querySelector(".main-content .journal-feed-day") && document.querySelector(".main-content")?.textContent.includes(text), typed),
      { timeout: 15_000, timeoutMsg: "Edited journal title did not reopen its saved text" });
      await assertNoJournalConflict();
      if (fs.readFileSync(openedFile, "utf8") !== expectedBytes) throw new Error("Reopening the edited journal changed its bytes");
      if (fs.readdirSync(path.join(graph, journalsDirectory)).join() !== path.basename(openedFile)) throw new Error("Editing the journal created an extra file");
      step("863-feed-edits-pass", "Three successive feed edits saved to the existing file without conflict, then its title reopened the saved text");
    }
    await captureJournalProbe("journal-ipc.json");
  }

  if (issues.includes("809")) {
    await openPageByName(browser, pageName);
    const before = new Set(await browser.getWindowHandles());
    await browser.keys(["Control", "k"]);
    await browser.$(".switcher-input").waitForExist({ timeout: 5000 });
    await browser.$(".switcher-input").setValue(pageName);
    await browser.waitUntil(() => browser.execute((name) => {
      if (document.querySelector('#switcher-results [role="status"]')) return false;
      const row = [...document.querySelectorAll(".switcher-row:not(.block-result)")]
        .find(el => el.querySelector(".switcher-name")?.textContent?.trim() === name);
      if (!row) return false;
      row.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2, clientX: 400, clientY: 250 }));
      return true;
    }, pageName), { timeout: 10_000, timeoutMsg: "No matching Ctrl+K page result" });
    step("809-before-new-window", "Ctrl+K > search page > right-click result");
    await browser.execute(() => {
      const item = [...document.querySelectorAll(".ctx-page-item")].find(el => el.textContent?.trim() === "Open in new window");
      if (!item) throw new Error("Open in new window context-menu item absent");
      item.click();
    });
    let popup;
    await browser.waitUntil(async () => {
      popup = (await browser.getWindowHandles()).find(handle => !before.has(handle));
      return !!popup;
    }, { timeout: 15_000, timeoutMsg: "No separate workspace window appeared" });
    await browser.switchToWindow(popup);
    await browser.waitUntil(() => browser.execute(() => document.body.textContent.includes("GH809 original page body")),
      { timeout: 15_000, timeoutMsg: "Popup stayed blank instead of showing the selected page" });
    step("809-popup-rendered", await browser.getTitle());
    await shot("809-popup.png");
    // Use the ordinary editor/save journey to demonstrate usable popup input.
    // Related WebKit popups cannot serialize WebDriver element handles either.
    const point = await browser.execute(() => {
      const rect = document.querySelector(".block-content").getBoundingClientRect();
      return { x: Math.round(rect.left + 30), y: Math.round(rect.top + rect.height / 2) };
    });
    await browser.performActions([{ type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [
      { type: "pointerMove", duration: 0, x: point.x, y: point.y, origin: "viewport" },
      { type: "pointerDown", button: 0 }, { type: "pointerUp", button: 0 },
    ] }]);
    await browser.releaseActions();
    await browser.waitUntil(() => browser.execute(() => !!document.querySelector("textarea.block-editor")), { timeout: 5000 });
    if (process.platform === "linux") {
      const title = (await browser.getTitle()).replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
      const id = xdo("search", "--onlyvisible", "--name", `^${title}$`).split(/\s+/)[0];
      xdo("windowactivate", "--sync", id);
      xdo("key", "--clearmodifiers", "ctrl+a");
      xdo("type", "--clearmodifiers", "--delay", "40", "GH809 edited in popup");
      xdo("key", "Escape");
    } else {
      await browser.keys(["Control", "a"]);
      await browser.keys("GH809 edited in popup");
      await browser.keys(["Escape"]);
    }
    await browser.waitUntil(() => fs.readFileSync(pageFile, "utf8").includes("GH809 edited in popup"),
      { timeout: 10_000, timeoutMsg: "Popup edit did not save" });
    await browser.switchToWindow(main);
    await browser.waitUntil(() => browser.execute(() => document.body.textContent.includes("GH809 edited in popup")),
      { timeout: 10_000, timeoutMsg: "Main did not remain responsive and show the popup edit" });
    await shot("809-main.png");
    step("809-pass", "Popup rendered the page, saved its edit, and main showed the edit");
  }
  fixture.verify(); report.status = "passed"; write();
} catch (error) {
  report.status = "failed"; report.classification ??= "ambiguous"; report.observed = String(error);
  write();
  try {
    report.dom = await browser?.execute(() => document.body.textContent?.slice(0, 6000));
    if (issues.some(issue => issue.startsWith("863"))) {
      const evidence = await captureJournalProbe("journal-ipc.json");
      if (evidence?.mail.some(mail => mail.conflict || mail.conflictReported) || evidence?.warnings.length) {
        report.classification = "product";
        report.observed += "; host conflict mail or conflict banner captured in journal-ipc.json";
      }
    }
    if (issues.some(issue => issue.startsWith("863"))) fs.writeFileSync(path.join(out, "failure-diagnostics.json"), JSON.stringify(await diagnostics(), null, 2));
    await shot("failure.png");
  } catch { /* A frozen renderer cannot provide a screenshot or DOM. */ }
  write(); process.exitCode = 1; console.error(error);
} finally {
  // Bound cleanup independently of a frozen browser session.
  if (browser) await Promise.race([browser.deleteSession().catch(() => {}), sleep(1000)]);
  stop(); clearTimeout(watchdog); fixture.close?.();
  if (log !== undefined) fs.closeSync(log);
}
