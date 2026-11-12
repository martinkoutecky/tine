// Native journey for CM step 2 batch 12 (rename and merge follow model round
// 9). Two user paths through the page title, on one disposable graph:
//  1. M-Q5 (D-20261102-MC5, divergence R4): renaming "ML notes" to "ML", a
//     name that is only an alias of Machine Learning, opens Tine's own dialog
//     with three choices. Cancel writes nothing. "Make ML its own page" moves
//     the page to pages/ML.md as a real page, leaves Machine Learning's
//     alias:: ML in place, and [[ML]] still shows in Machine Learning's
//     Linked References.
//  2. M-Q6 / CM-OWN-CHILD (divergence R12): renaming Foo to Foo/bar, where
//     Foo/bar is already a page, is refused; the user sees why in an error
//     toast, and no file changes.
// Written by the cm-merge-rules lane.
// Sources and alias targets without files, including alias-only context-menu
// rename, owner selection and reload. R13-C4 real targets have separate coverage.
// Batch 18: CM3-1 journal-reference-loss refusal explains itself and writes nothing.
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

const root = path.resolve(import.meta.dirname, "..");
const artifacts = path.join(root, "artifacts/cm/cm-merge-rules");
fs.mkdirSync(artifacts, { recursive: true });
const run = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:tine/guide-announced? true}\n");
const seeded = {
  "pages/Machine Learning.md": "alias:: ML\n\n- ml body\n",
  "pages/ML notes.md": "- notes body\n",
  "pages/Owner Two.md": "alias:: Short Two\n\n- owner two\n",
  "pages/Source Two.md": "- source two\n",
  "pages/queries.md": '- {{query (page "Source Two")}}\n- [[Source Two]] [[Ghost]]\n',
  "pages/Alias Source Owner.md": "alias:: Alias Source\n\n- alias owner body\n",
  "pages/Alias Target Owner.md": "alias:: Alias Target\n\n- target owner body\n",
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

const dialogChoices = () => browser.execute(() =>
  [...document.querySelectorAll(".tine-rename-alias-choice button")].map((button) => [button.dataset.choice, button.textContent.trim()]));

try {
  driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"), { env, stdio: ["ignore", log, log], detached: true });
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 5,
    connectionRetryTimeout: 30_000, capabilities: tauriCapabilities(process.env.TINE_APP || path.join(root, "target/debug/tine"), "cm-merge-rules") });
  await ensureMainWindow(browser);
  await browser.$(".graph-switch-btn").waitForExist({ timeout: 30_000 });
  await assertAppLease(ownerFixture);
  const before = snapshot();

  stage = "CM3-1: the post's namespace rename names the journal at risk and writes nothing";
  await openPageByName(browser, "old");
  await renameTitle("new");
  await browser.waitUntil(() => browser.execute(() =>
    [...document.querySelectorAll(".toast-error .toast-msg")].some((toast) =>
      /links reach the journal/.test(toast.textContent ?? "") && /Oct 26th, 2026/.test(toast.textContent ?? ""))),
  { timeout: 15_000, timeoutMsg: "no refusal explaining the lost references to Oct 26th, 2026" });
  assert.equal(await browser.$(".tine-rename-alias-choice").isExisting(), false);
  assert.equal(await currentPageTitle(browser), "old");
  assert.deepEqual(snapshot(), before, "the journal-risk refusal changed a file");

  stage = "M-Q5: the alias-only name asks three ways";
  await openPageByName(browser, "ML notes");
  await renameTitle("ML");
  await browser.$(".tine-rename-alias-choice").waitForExist({ timeout: 15_000, timeoutMsg: "the M-Q5 dialog never appeared" });
  const choices = await dialogChoices();
  const labels = Object.fromEntries(choices);
  assert.deepEqual(Object.keys(labels).sort(), ["cancel", "merge", "own-page"], JSON.stringify(choices));
  assert.match(labels.merge, /Merge into “Machine Learning”/);
  assert.match(labels["own-page"], /Make “ML” its own page \(still an alias of “Machine Learning”\)/);
  assert.equal(labels.cancel, "Cancel");

  stage = "M-Q5: Cancel writes nothing";
  await browser.$('.tine-rename-alias-choice button[data-choice="cancel"]').click();
  await browser.$(".tine-rename-alias-choice").waitForExist({ reverse: true, timeout: 4000, timeoutMsg: "the dialog stayed open after Cancel" });
  assert.deepEqual(snapshot(), before, "Cancel changed a file");

  stage = "M-Q5: make ML its own page";
  await openPageByName(browser, "ML notes");
  await renameTitle("ML");
  await browser.$('.tine-rename-alias-choice button[data-choice="own-page"]').waitForExist({ timeout: 15_000, timeoutMsg: "the M-Q5 dialog never reappeared" });
  await browser.$('.tine-rename-alias-choice button[data-choice="own-page"]').click();
  // A fileless alias retains the legacy owner/own-page choice. R13-C4 real
  // targets use the ordinary merge flow (covered in cm_merge_rules.rs).
  await waitForFileText(path.join(graph, "pages/ML.md"), (text) => text === "- notes body\n", { timeoutMs: 15_000 });
  await waitForFileText(path.join(graph, "pages/q.md"), (text) => text === "- see [[ML]]\n- also [[ML]] here\n", { timeoutMs: 15_000 });
  assert.ok(!fs.existsSync(path.join(graph, "pages/ML notes.md")), "ML notes.md moved");
  assert.equal(fs.readFileSync(path.join(graph, "pages/Machine Learning.md"), "utf8"), seeded["pages/Machine Learning.md"], "the owner keeps alias:: ML");
  await browser.waitUntil(async () => (await currentPageTitle(browser)) === "ML", { timeout: 15_000, timeoutMsg: "the renamed page is not open as ML" });

  stage = "M-Q5: [[ML]] still in Machine Learning's Linked References";
  await openPageByName(browser, "Machine Learning");
  await browser.waitUntil(() => browser.execute(() =>
    [...document.querySelectorAll(".linked-references")].some((section) => /also/.test(section.textContent ?? "") && /ML/.test(section.textContent ?? ""))),
  { timeout: 15_000, timeoutMsg: "q's [[ML]] block is missing from Machine Learning's Linked References" });

  stage = "r11: merging into a fileless alias owner rewrites queries";
  await openPageByName(browser, "Source Two");
  await renameTitle("Short Two");
  await browser.$('.tine-rename-alias-choice button[data-choice="merge"]').waitForExist({ timeout: 15_000 });
  await browser.$('.tine-rename-alias-choice button[data-choice="merge"]').click();
  await waitForFileText(path.join(graph, "pages/Owner Two.md"), (text) => text.includes("source two"), { timeoutMs: 15_000 });
  await waitForFileText(path.join(graph, "pages/queries.md"), (text) => text.includes('(page "Owner Two")') && text.includes("[[Owner Two]]") && !text.includes("Source Two"), { timeoutMs: 15_000 });
  // CM SPEC §4.1: the chosen member spelling (here the owner, Owner Two) drives the whole substitution, links and
  // stored queries alike; the old split (links kept the typed alias Short Two) predates B4's single substitution.
  assert.ok(!fs.existsSync(path.join(graph, "pages/Short Two.md")));
  assert.ok(!fs.existsSync(path.join(graph, "pages/Source Two.md")));

  stage = "r11: a file-less source offers merge into the owner and Cancel";
  await openPageByName(browser, "Ghost");
  await renameTitle("Short Two");
  await browser.$(".tine-rename-alias-choice").waitForExist({ timeout: 15_000 });
  assert.deepEqual(Object.keys(Object.fromEntries(await dialogChoices())).sort(), ["cancel", "merge"]);
  const beforeFilelessCancel = snapshot();
  await browser.$('.tine-rename-alias-choice button[data-choice="cancel"]').click();
  assert.deepEqual(snapshot(), beforeFilelessCancel);

  stage = "r12: rename an alias-only link through the real context menu and owner dialog";
  await openPageByName(browser, "Alias journey");
  await browser.$('a.page-ref[data-ref="alias source"]').waitForExist({ timeout: 15_000 });
  await browser.execute(() => {
    const link = document.querySelector('a.page-ref[data-ref="alias source"]');
    link.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2 }));
  });
  await browser.$('[data-page-action-id="rename-page"]').waitForExist({ timeout: 4000 });
  await browser.$('[data-page-action-id="rename-page"]').click();
  await browser.$('.ctx-rename-name').setValue("Alias Target");
  await browser.keys("Enter");
  await browser.$('.tine-rename-alias-choice').waitForExist({ timeout: 15_000 });
  const aliasChoices = Object.fromEntries(await dialogChoices());
  assert.deepEqual(Object.keys(aliasChoices).sort(), ["cancel", "merge"]);
  assert.match(aliasChoices.merge, /Alias Target Owner/);
  await browser.$('.tine-rename-alias-choice button[data-choice="merge"]').click();
  const declaration = path.join(graph, "pages/Alias Source Owner.md");
  // CM SPEC §4.1: the chosen member (Alias Target Owner) is the one spelling for the alias line, links and queries.
  await waitForFileText(declaration, text => text === "alias:: Alias Target Owner\n\n- alias owner body\n", { timeoutMs: 15_000 });
  await waitForFileText(path.join(graph, "pages/Alias journey.md"), text => text.includes("[[Alias Target Owner]]") && text.includes('(page "Alias Target Owner")') && !text.includes("Alias Source"), { timeoutMs: 15_000 });
  assert.ok(!fs.existsSync(path.join(graph, "pages/Alias Target.md")));
  assert.ok(!fs.existsSync(path.join(graph, "pages/Alias Source.md")));

  stage = "r12: reload preserves the rewritten declaration and alias group";
  await browser.refresh();
  await browser.$(".graph-switch-btn").waitForExist({ timeout: 30_000 });
  await openPageByName(browser, "Alias Source Owner");
  await browser.waitUntil(() => browser.execute(() =>
    [...document.querySelectorAll('[data-alias-names] a.page-ref, [data-alias-group-pages] a.page-ref')].some(link => link.textContent.trim() === "Alias Target Owner")),
  { timeout: 15_000, timeoutMsg: "renamed alias detached after reload" });
  assert.equal(fs.readFileSync(declaration, "utf8"), "alias:: Alias Target Owner\n\n- alias owner body\n");

  stage = "M-Q6: renaming Foo to its own used child Foo/bar is refused with the reason";
  const beforeRefusal = snapshot();
  await openPageByName(browser, "Foo");
  await renameTitle("Foo/bar");
  await browser.waitUntil(() => browser.execute(() =>
    [...document.querySelectorAll(".toast-error .toast-msg")].some((toast) => /already a page or an alias inside “Foo”/.test(toast.textContent ?? ""))),
  { timeout: 15_000, timeoutMsg: "no error toast saying why the rename was refused" });
  assert.equal(await browser.$(".tine-rename-alias-choice").isExisting(), false, "a refusal offers no choice");
  assert.deepEqual(snapshot(), beforeRefusal, "the refused rename changed a file");

  ownerFixture.verify();
  fs.writeFileSync(path.join(run, "receipt.json"), JSON.stringify({ commit, workingDiff: true, outcome: "PASS", journey: "CM3-1 journal-risk refusal names the journal and writes nothing; fileless M-Q5 dialog and own-page outcome; fileless alias owner merge and Cancel; alias-only context-menu owner merge creates no placeholder, rewrites declarations and queries and survives reload; M-Q6 own-child refusal writes nothing", graph }, null, 2));
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
