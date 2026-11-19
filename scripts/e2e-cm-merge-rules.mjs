// Native journey: rename requests follow the confirmed conceptual model
// (D-20261116-CM536, r6 O0 to O4), through the page title and the context
// menu, on one disposable graph:
//  1. O4: renaming old moves old/sub, which a journal declares as its alias;
//     the journal keeps its file and day, and its alias and every link follow.
//  2. O3: a name that is only another page's alias opens the alias dialog
//     (D-20261115-HELP): it shows the declaring page, its alias line, what the
//     name opens now and each allowed request with its effect. Cancel writes
//     nothing; "use the name for this page" removes the alias, then renames;
//     for a source without a file, the dialog's merge repoints its references.
//  2b. D-20261116-TAGH: merging two tagged pages (native confirm, answered on
//     the dialog window) leaves one united tags line.
//  3. O3: an alias-only source renamed through the context menu rewrites its
//     declaration, links and stored queries, creates no file, and stays
//     attached after a reload.
//  4. O4: renaming Foo onto its own claimed child Foo/bar is refused with the
//     reason; no file changes.
// The native confirm's text cannot be read by the driver; its tags line is
// covered by src/renameHelpers.test.ts.
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { remote } from "webdriverio";
import { APP_ID } from "./lib/app-identity.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { currentPageTitle, openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";
import { answerNativeDialog } from "./lib/e2e-native-dialog.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = path.join(root, "artifacts/cm/cm-merge-rules");
fs.mkdirSync(artifacts, { recursive: true });
const run = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:tine/guide-announced? true}\n");
const seeded = {
  "pages/Machine Learning.md": "alias:: AI, ML\n\n- ml body\n",
  "pages/Tag Kept.md": "tags:: b, c\n\n- tag kept\n",
  "pages/Tag Old.md": "tags:: a, b\n\n- tag old\n",
  "pages/ML notes.md": "- notes body\n",
  "pages/Owner Two.md": "alias:: Short Two\n\n- owner two\n",
  "pages/ghost referrer.md": "- [[Ghost]]\n",
  "pages/Alias Source Owner.md": "alias:: Alias Source\n\n- alias owner body\n",
  "pages/Alias journey.md": '- [[Alias Source]]\n- {{query (page "Alias Source")}}\n',
  "pages/q.md": "- see [[ML notes]]\n- also [[ML]] here\n",
  "pages/Foo.md": "- foo body\n",
  "pages/Foo%2Fbar.md": "- bar body\n",
  "pages/p.md": "- see [[Foo]] and [[Foo/bar]]\n",
  "pages/old.md": "- source\n",
  "pages/journal referrer.md": "- see [[old/sub]]\n",
  "journals/2026_10_26.md": "alias:: old/sub\n\n- day\n",
};
for (const [rel, text] of Object.entries(seeded)) fs.writeFileSync(path.join(graph, rel), text);
fs.writeFileSync(path.join(graph, "journals", "2026_11_04.md"), "- journal\n");
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, dir));
fs.mkdirSync(path.join(run, "data", APP_ID), { recursive: true });
fs.writeFileSync(path.join(run, "data", APP_ID, "tine-settings.json"), JSON.stringify({ known_graphs: [{ name: "graph", path: graph }], last_graph_path: graph }));
const ownerFixture = accountHomeFixture(run, run);
const env = { ...ownerFixture.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(run, "data"),
  XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4912);
const log = fs.openSync(path.join(run, "driver.log"), "w");
let driver;
const commit = execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
let browser;
let stage = "startup";

/** Every page and journal file, graph-relative path → bytes. */
function snapshot() {
  const files = {};
  for (const dir of ["pages", "journals"]) {
    for (const name of fs.readdirSync(path.join(graph, dir)).sort()) files[`${dir}/${name}`] = fs.readFileSync(path.join(graph, dir, name), "utf8");
  }
  return files;
}

/** Rename the open page through its title, as the user does. */
async function renameTitle(next) {
  await browser.execute(() => {
    document.querySelector("h1.page-title")?.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true, view: window }));
  });
  await browser.$(".page-title-input").waitForExist({ timeout: 4000, timeoutMsg: "the rename input never appeared" });
  await browser.execute((value) => {
    const input = document.querySelector(".page-title-input");
    input.focus();
    input.value = value;
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  }, next);
}

try {
  driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"), { env, stdio: ["ignore", log, log], detached: true });
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 5,
    connectionRetryTimeout: 30_000, capabilities: tauriCapabilities(process.env.TINE_APP || path.join(root, "target/debug/tine"), "cm-merge-rules") });
  await ensureMainWindow(browser);
  await browser.$(".graph-switch-btn").waitForExist({ timeout: 30_000 });
  await assertAppLease(ownerFixture);
  const before = snapshot();

  stage = "O4: renaming old moves old/sub, which a journal declares; the journal keeps its file and day";
  await openPageByName(browser, "old");
  await renameTitle("new");
  await waitForFileText(path.join(graph, "journals/2026_10_26.md"), (text) => text === "alias:: new/sub\n\n- day\n", { timeoutMs: 15_000 });
  await waitForFileText(path.join(graph, "pages/journal referrer.md"), (text) => text === "- see [[new/sub]]\n", { timeoutMs: 15_000 });
  await waitForFileText(path.join(graph, "pages/new.md"), (text) => text === "- source\n", { timeoutMs: 15_000 });
  // The transaction removes the source after writing its new files: wait for it (old.md moved).
  await waitForFileText(path.join(graph, "pages/old.md"), (text) => text === "<absent>", { timeoutMs: 15_000 });
  await browser.waitUntil(async () => (await currentPageTitle(browser)) === "new", { timeout: 15_000, timeoutMsg: "the renamed page is not open as new" });

  stage = "O3: a name that is only another page's alias opens the alias dialog, which shows the component";
  const beforeAliasHeld = snapshot();
  await openPageByName(browser, "ML notes");
  await renameTitle("ML");
  const aliasDialog = browser.$(".tine-rename-alias-dialog");
  await aliasDialog.waitForExist({ timeout: 15_000, timeoutMsg: "the alias dialog never opened" });
  const shown = await browser.execute(() => {
    const dialog = document.querySelector(".tine-rename-alias-dialog");
    return {
      heading: dialog.querySelector("h2")?.textContent ?? "",
      component: [...dialog.querySelectorAll("[data-component-page]")].map((li) => li.textContent),
      merge: dialog.querySelector('[data-choice="merge"]')?.textContent ?? null,
      own: dialog.querySelector('[data-choice="own"]')?.textContent ?? null,
      edit: dialog.querySelector("[data-alias-edit]")?.textContent ?? "",
      retarget: dialog.querySelector('[data-choice-block="own"] [data-retarget="ML"]')?.textContent ?? "",
    };
  });
  assert.match(shown.heading, /“ML” already opens “Machine Learning”/);
  assert.ok(shown.component.some((line) => /Machine Learning/.test(line) && /alias:: AI, ML/.test(line) && /opens “ML” now/.test(line)), JSON.stringify(shown));
  assert.ok(shown.component.some((line) => /ML notes/.test(line) && /the page you are renaming/.test(line)), JSON.stringify(shown));
  assert.match(shown.merge ?? "", /Merge “ML notes” into “Machine Learning”/);
  assert.match(shown.own ?? "", /Use the name “ML” for this page/);
  assert.match(shown.edit, /alias:: AI, ML becomes alias:: AI/);
  assert.match(shown.retarget, /Links to “ML” open “ML” instead of “Machine Learning”, their text unchanged, in 1 page: q/);
  assert.deepEqual(snapshot(), beforeAliasHeld, "showing the dialog changed a file");

  stage = "O3: cancelling the alias dialog writes nothing";
  await browser.$("[data-choice-cancel]").click();
  await aliasDialog.waitForExist({ reverse: true, timeout: 4000, timeoutMsg: "the dialog stayed after Cancel" });
  assert.equal(await currentPageTitle(browser), "ML notes");
  assert.deepEqual(snapshot(), beforeAliasHeld, "cancelling changed a file");

  stage = "O3 helper: use the name for this page removes the alias, then renames";
  await renameTitle("ML");
  await aliasDialog.waitForExist({ timeout: 15_000, timeoutMsg: "the alias dialog never reopened" });
  await browser.$('[data-choice="own"]').click();
  await waitForFileText(path.join(graph, "pages/ML.md"), (text) => text === "- notes body\n", { timeoutMs: 15_000 });
  await waitForFileText(path.join(graph, "pages/Machine Learning.md"), (text) => text === "alias:: AI\n\n- ml body\n", { timeoutMs: 15_000 });
  await waitForFileText(path.join(graph, "pages/q.md"), (text) => text === "- see [[ML]]\n- also [[ML]] here\n", { timeoutMs: 15_000 });
  // The transaction removes the source after writing its new files: wait for it (ML notes.md moved).
  await waitForFileText(path.join(graph, "pages/ML notes.md"), (text) => text === "<absent>", { timeoutMs: 15_000 });

  stage = "O3 helper: for a source without a file, the dialog's merge repoints its references";
  await openPageByName(browser, "Ghost");
  await renameTitle("Short Two");
  await aliasDialog.waitForExist({ timeout: 15_000, timeoutMsg: "no alias dialog for the file-less source" });
  const ghostMerge = await browser.$('[data-choice="merge"]');
  assert.match(await ghostMerge.getText(), /Merge “Ghost” into “Owner Two”/);
  await ghostMerge.click();
  await waitForFileText(path.join(graph, "pages/ghost referrer.md"), (text) => text === "- [[Owner Two]]\n", { timeoutMs: 15_000 });
  assert.equal(fs.readFileSync(path.join(graph, "pages/Owner Two.md"), "utf8"), "alias:: Short Two\n\n- owner two\n");

  stage = "TAGH: merging two tagged pages leaves one united tags line";
  await openPageByName(browser, "Tag Old");
  await renameTitle("Tag Kept");
  await answerNativeDialog("yes", { env });
  await waitForFileText(path.join(graph, "pages/Tag Kept.md"), (text) => text.startsWith("tags:: b, c, a\n") && text.includes("- tag kept") && text.includes("- tag old"), { timeoutMs: 15_000 });
  const kept = fs.readFileSync(path.join(graph, "pages/Tag Kept.md"), "utf8");
  assert.equal(kept.match(/tags::/g)?.length, 1, kept);
  // The transaction removes the source after writing its new files: wait for it (the merged source went to the trash).
  await waitForFileText(path.join(graph, "pages/Tag Old.md"), (text) => text === "<absent>", { timeoutMs: 15_000 });

  stage = "O3: rename an alias-only link through the context menu";
  await openPageByName(browser, "Alias journey");
  await browser.$('a.page-ref[data-ref="alias source"]').waitForExist({ timeout: 15_000 });
  await browser.execute(() => {
    const link = document.querySelector('a.page-ref[data-ref="alias source"]');
    link.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2 }));
  });
  await browser.$('[data-page-action-id="rename-page"]').waitForExist({ timeout: 4000 });
  await browser.$('[data-page-action-id="rename-page"]').click();
  await browser.$('.ctx-rename-name').setValue("Alias Renamed");
  await browser.keys("Enter");
  const declaration = path.join(graph, "pages/Alias Source Owner.md");
  await waitForFileText(declaration, text => text === "alias:: Alias Renamed\n\n- alias owner body\n", { timeoutMs: 15_000 });
  await waitForFileText(path.join(graph, "pages/Alias journey.md"), text => text === '- [[Alias Renamed]]\n- {{query (page "Alias Renamed")}}\n', { timeoutMs: 15_000 });
  assert.ok(!fs.existsSync(path.join(graph, "pages/Alias Renamed.md")), "no file is created for an alias");
  assert.ok(!fs.existsSync(path.join(graph, "pages/Alias Source.md")));

  stage = "O3: reload keeps the renamed alias attached";
  await browser.refresh();
  await browser.$(".graph-switch-btn").waitForExist({ timeout: 30_000 });
  await openPageByName(browser, "Alias Source Owner");
  await browser.waitUntil(() => browser.execute(() =>
    [...document.querySelectorAll('[data-alias-names] a.page-ref, [data-alias-group-pages] a.page-ref')].some(link => link.textContent.trim() === "Alias Renamed")),
  { timeout: 15_000, timeoutMsg: "renamed alias detached after reload" });
  assert.equal(fs.readFileSync(declaration, "utf8"), "alias:: Alias Renamed\n\n- alias owner body\n");

  stage = "O4: renaming Foo onto its own claimed child Foo/bar is refused with the reason";
  const beforeRefusal = snapshot();
  await openPageByName(browser, "Foo");
  await renameTitle("Foo/bar");
  await browser.waitUntil(() => browser.execute(() =>
    [...document.querySelectorAll(".toast-error .toast-msg")].some((toast) => /already a page or an alias inside “Foo”/.test(toast.textContent ?? ""))),
  { timeout: 15_000, timeoutMsg: "no error toast saying why the rename was refused" });
  assert.deepEqual(snapshot(), beforeRefusal, "the refused rename changed a file");

  ownerFixture.verify();
  fs.writeFileSync(path.join(run, "receipt.json"), JSON.stringify({ commit, workingDiff: true, outcome: "PASS", journey: "O4 rename moves a journal-declared alias under the source and keeps the journal; O3 alias dialog shows the component and effects, Cancel writes nothing, use-the-name removes the alias then renames, the file-less merge repoints references; TAGH merge leaves one united tags line; O3 alias-only context-menu rename rewrites declaration, links and queries, creates no file and survives reload; O4 own-child refusal writes nothing", graph }, null, 2));
  console.log(`PASS CM merge rules. ${run}`);
} catch (error) {
  try { await browser?.saveScreenshot(path.join(run, "failure.png")); } catch {}
  const files = Object.entries(snapshot()).map(([rel, text]) => `${rel}: ${JSON.stringify(text)}`).join("\n");
  fs.writeFileSync(path.join(run, "failure.txt"), `Tested: ${commit} + worktree diff\nJourney: ${stage}\nObserved: ${error.stack}\n${files}\n`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
  ownerFixture.close?.();
}
