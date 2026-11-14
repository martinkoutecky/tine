// GH #453: global code wrapping preserves every mounted reading viewport.
import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { spawn, execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { chooseEditor } from "./lib/e2e-editor-choice.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { installAccountHome, assertAppLease } from "./lib/e2e-account-home.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const root = path.resolve(import.meta.dirname, "..");
const app = process.env.TINE_APP || path.join(root, "target/debug/tine");
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, "artifacts/code-wrap-toggle"));
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache", "state", "home"]) fs.mkdirSync(path.join(out, dir));
const account = installAccountHome(out);
fs.writeFileSync(path.join(graph, "logseq/config.edn"), '{:shortcuts {:ui/toggle-code-block-wrap "mod+alt+w"}}\n');
const file = path.join(graph, "pages/Wrap Demo.md");
// All code and reading markers are descendants of one enclosing outline
// parent: its top stays fixed while wrapping moves the visible descendant.
const source = '- Long outline parent\n' + Array.from({ length: 16 }, (_, n) =>
  `  - \`\`\`js\n    ${Array.from({ length: 3 }, () => '// ' + 'long code line '.repeat(28)).join("\n    ")}\n    \`\`\`\n  - Reading marker ${n}\n`).join("");
fs.writeFileSync(file, source);
const env = { ...process.env, HOME: path.join(out, "home"), TINE_GRAPH: graph, XDG_DATA_HOME: path.join(out, "data"),
  XDG_CONFIG_HOME: path.join(out, "config"), XDG_CACHE_HOME: path.join(out, "cache"),
  XDG_STATE_HOME: path.join(out, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4876);
const paletteAction = process.env.E2E_WRAP_PALETTE_ACTION || "click";
const modes = process.env.E2E_EDITOR ? [process.env.E2E_EDITOR] : ["textarea", "cm6"];
if (!modes.every(mode => ["textarea", "cm6"].includes(mode))) throw Error("Unknown E2E_EDITOR");
if (!["click", "enter"].includes(paletteAction)) throw Error("Unknown E2E_WRAP_PALETTE_ACTION");
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env, stdio: ["ignore", log, log], detached: true });
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(), workingDiff: true, app,
  binarySha256: createHash("sha256").update(fs.readFileSync(app)).digest("hex"), paletteAction,
  journey: "Palette, binding and Settings preserve visible reading text in main, split and sidebar with both editors", offsets: [] };
let browser;
let stage = "startup";
const screenshot = async (file) => {
  // WebKitGTK can stall on repeated WebDriver screenshots; capture Xvfb.
  if (process.platform === "linux") execFileSync("import", ["-window", "root", file], { timeout: 10000 });
  else await browser.saveScreenshot(file);
};
try {
  await sleep(1500);
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0,
    connectionRetryTimeout: 30000, capabilities: tauriCapabilities(app, "code-wrap-toggle") });
  await ensureMainWindow(browser);
  await browser.setWindowSize(1800, 1000);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await openPageByName(browser, "Wrap Demo");
  await assertAppLease(account);
  const setting = async (label, search, value) => {
    await browser.$('button[title^="Settings"]').click();
    await browser.$(".settings-search-input").setValue(search);
    const result = await browser.$(".settings-search-result");
    await result.waitForDisplayed(); await result.click();
    const toggle = await browser.$(`[data-setting-label="${label}"] [role="switch"]`);
    await toggle.waitForDisplayed();
    if (await toggle.getAttribute("aria-checked") !== String(value)) await toggle.click();
    await browser.waitUntil(async () => await toggle.getAttribute("aria-checked") === String(value));
    await browser.keys(["Escape"]);
  };
  // The marker must be readable, not behind the default pinned parent. Keep
  // the nested outline, but disable its overlay through the normal preference.
  await setting("Sticky parent bullets", "Sticky parent bullets", false);
  stage = "mount split and sidebar";
  await browser.keys(["Control", "Alt", "\\"]);
  await browser.waitUntil(() => browser.execute(() => document.querySelectorAll(".main-content").length === 2), { timeout: 10000 });
  await browser.execute(() => document.querySelector("h1.page-title").dispatchEvent(new MouseEvent("click", { bubbles: true, button: 0, shiftKey: true })));
  await browser.waitUntil(() => browser.execute(() => !!document.querySelector(".right-sidebar [data-block-id]")), { timeout: 10000 });
  const offsets = () => browser.execute(() => [...document.querySelectorAll(".main-content, .right-sidebar")].map(scroller => {
    const marker = [...scroller.querySelectorAll("[data-block-id]")].find(el => el.textContent.trim() === "Reading marker 6");
    return marker.getBoundingClientRect().top - scroller.getBoundingClientRect().top;
  }));
  const readable = () => browser.execute(() => [...document.querySelectorAll(".main-content, .right-sidebar")].every(scroller => {
    const marker = [...scroller.querySelectorAll("[data-block-id]")].find(el => el.textContent.trim() === "Reading marker 6");
    if (!marker) return false;
    const rect = marker.getBoundingClientRect(), pane = scroller.getBoundingClientRect();
    const hit = document.elementFromPoint(rect.left + rect.width / 2, rect.top + rect.height / 2);
    return rect.top >= pane.top && rect.bottom < pane.bottom && hit?.closest("[data-block-id]") === marker;
  }));
  for (const mode of modes) {
    stage = `choose ${mode}`;
    await chooseEditor(browser, mode);
    // Verify the chosen engine mounts, then return to its rendered code card.
    await browser.$(".main-content pre.code-block").click();
    await browser.$(mode === "cm6" ? '.main-content [data-block-editor="cm6"]' : ".main-content textarea.block-editor").waitForDisplayed();
    await browser.keys(["Escape"]); await browser.keys(["Escape"]);
    await browser.waitUntil(() => browser.execute(() => !document.querySelector('.main-content textarea.block-editor, .main-content [data-block-editor="cm6"]')));
    stage = `${mode}: language label opens source`;
    await browser.$('.main-content button.code-language').click();
    await browser.waitUntil(() => browser.execute(() => {
      const editor = document.querySelector('.main-content textarea.block-editor, .main-content .cm-content');
      return (editor?.value ?? editor?.textContent ?? '').startsWith('```js');
    }));
    await browser.keys(["Delete"]); await browser.keys(["Delete"]);
    await browser.keys(["Escape"]); await browser.keys(["Escape"]);
    await browser.waitUntil(() => browser.execute(() => !document.querySelector('.main-content textarea.block-editor, .main-content [data-block-editor="cm6"]')));
    const withoutLanguage = source.replace('```js', '```');
    await waitForFileText(file, text => text === withoutLanguage);
    const plainLabel = await browser.$('.main-content button.code-language');
    if (await plainLabel.getText() !== "plain") throw Error("Unlabelled code lost its source control");
    await plainLabel.click();
    await browser.$(mode === "cm6" ? '.main-content [data-block-editor="cm6"]' : ".main-content textarea.block-editor").waitForDisplayed();
    await browser.keys("js"); await browser.keys(["Escape"]); await browser.keys(["Escape"]);
    await waitForFileText(file, text => text === source);
    await browser.waitUntil(() => browser.execute(() => !document.querySelector('.main-content textarea.block-editor, .main-content [data-block-editor="cm6"]')));
    // Opening three copies can still be admitting lazy bodies and restoring
    // scroll. Establish settled marker offsets before measuring.
    await browser.waitUntil(async () => {
      await browser.execute(() => {
        for (const scroller of document.querySelectorAll(".main-content, .right-sidebar")) {
          const marker = [...scroller.querySelectorAll("[data-block-id]")].find(el => el.textContent.trim() === "Reading marker 6");
          if (!marker) throw Error("Reading marker absent");
          scroller.scrollTop += marker.getBoundingClientRect().top - scroller.getBoundingClientRect().top - 4;
        }
        return new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      });
      const positions = await offsets();
      return positions.length === 3 && positions.every(position => Math.abs(position - 4) < 1);
    }, { timeout: 10000, timeoutMsg: "Reading markers did not settle in view before the toggle" });
    if (!await readable()) throw Error("Reading marker is obscured before the toggle");
    const before = await offsets(); proof.offsets.push({ mode, action: "before", positions: before });
    const geometry = await browser.execute(() => [...document.querySelectorAll(".main-content, .right-sidebar")].map(scroller => {
      const top = scroller.getBoundingClientRect().top;
      return { scroll: scroller.scrollTop,
        nearby: [...scroller.querySelectorAll("[data-block-id]")].filter(el => {
          const rect = el.getBoundingClientRect(); return rect.bottom > top - 50 && rect.top < top + 100;
        }).map(el => ({ text: el.textContent.slice(0, 100), top: el.getBoundingClientRect().top - top, bottom: el.getBoundingClientRect().bottom - top })) };
    }));
    proof.offsets.at(-1).geometry = geometry;
    await screenshot(path.join(out, `${mode}-before.png`));
    for (const [action, wrapped] of [["palette", true], ["binding", false], ["settings", true], ["settings", false]]) {
      stage = `${mode}: toggle ${wrapped ? "on" : "off"} via ${action}`;
      if (action === "palette") {
        await browser.keys(["Control", "Shift", "p"]);
        const input = await browser.$(".switcher-input"); await input.waitForDisplayed(); await input.setValue("Toggle code block wrap");
        const row = await browser.$(".switcher-row*=Toggle code block wrap"); await row.waitForDisplayed();
        if (paletteAction === "enter") await browser.keys(["Enter"]);
        else await row.click();
      } else if (action === "binding") await browser.keys(["Control", "Alt", "w"]);
      else await setting("Wrap code lines", "Wrap code lines", wrapped);
      await browser.waitUntil(() => browser.execute(expected => {
        const cards = [...document.querySelectorAll("pre.code-block")];
        return cards.length > 0 && cards.every(el => el.classList.contains("code-wrapping") === expected);
      }, wrapped), { timeout: 10000 });
      await sleep(250);
      const after = await offsets(); proof.offsets.push({ mode, action, wrapped, positions: after });
      if (after.length !== 3 || after.some((offset, i) => Math.abs(offset - before[i]) > 3)) throw Error(`Viewport moved: ${JSON.stringify({ before, after })}`);
      if (!await readable()) throw Error("Reading marker is obscured after the toggle");
      await screenshot(path.join(out, `${mode}-${action}-${wrapped ? "wrapped" : "unwrapped"}.png`));
    }
  }
  if (fs.readFileSync(file, "utf8") !== source) throw Error("Toggling changed note source");
  proof.outcome = "PASS";
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2));
  console.log(`PASS GH #453: ${out}`);
} catch (error) {
  try { await screenshot(path.join(out, "failure.png")); } catch {}
  try { fs.writeFileSync(path.join(out, "failure-dom.html"), await browser.getPageSource()); } catch {}
  fs.writeFileSync(path.join(out, "failure.json"), JSON.stringify({ ...proof, stage,
    expected: "Three mounted reading markers retain offsets within 3px in both directions; source unchanged",
    observed: String(error.stack), classification: String(error.message).startsWith("Viewport moved:") ? "product" : "ambiguous; inspect log and DOM" }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
