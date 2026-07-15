// OG-parity interaction corpus: references and embeds.
//
// The pilot deliberately uses WebDriver typing and Enter selection rather than
// setting component state.  It records the popup, caret, rendered result and
// persisted Markdown so the same semantic contract can be replayed in Logseq OG.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4660);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4661);
const TMP = process.env.E2E_TMP_DIR || `/tmp/tine-og-parity-references-${process.pid}`;
const GRAPH = `${TMP}/graph`;
const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || `${TMP}/artifacts`;
const TARGET = "11111111-1111-4111-8111-111111111111";
const EDITOR = "22222222-2222-4222-8222-222222222222";
const TEST_PAGE = `${GRAPH}/pages/OG Parity References.md`;

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.mkdirSync(ARTIFACTS, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
fs.writeFileSync(TEST_PAGE, [
  "- Testing block",
  `  id:: ${TARGET}`,
  "- ",
  `  id:: ${EDITOR}`,
  "",
].join("\n"));
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- Open [[OG Parity References]]\n");

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: `${TMP}/xdg/data`,
  XDG_CONFIG_HOME: `${TMP}/xdg/config`,
  XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  LIBGL_ALWAYS_SOFTWARE: "1",
  GDK_BACKEND: "x11",
};
if (process.env.OG_PARITY_NORMAL_COMPOSITING !== "1") {
  env.WEBKIT_DISABLE_DMABUF_RENDERER = "1";
  env.WEBKIT_DISABLE_COMPOSITING_MODE = "1";
}
const log = fs.openSync(`${ARTIFACTS}/tauri-driver.log`, "w");
const td = spawn(TD, ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], {
  env,
  stdio: ["ignore", log, log],
});
await sleep(2500);

let browser;
const receipt = {
  schemaVersion: 1,
  scenario: "references.block-autocomplete",
  app: "tine",
  appIdentity: {
    path: path.resolve(APP),
    version: null,
    declaredVersion: JSON.parse(fs.readFileSync(path.join(ROOT, "src-tauri/tauri.conf.json"), "utf8")).version,
    artifactSha256: crypto.createHash("sha256").update(fs.readFileSync(APP)).digest("hex"),
    sourceRevision: process.env.TINE_SOURCE_REVISION || null,
    platform: `${process.platform}-${process.arch}`,
  },
  literalInput: "((test then Enter",
  observations: {},
};

try {
  browser = await remote({
    hostname: "127.0.0.1",
    port: DRIVER_PORT,
    path: "/",
    logLevel: "error",
    connectionRetryCount: 1,
    connectionRetryTimeout: 60_000,
    capabilities: {
      browserName: "wry",
      "wdio:enforceWebDriverClassic": true,
      "tauri:options": { application: APP },
    },
  });
  receipt.appIdentity.version = await browser.execute(() => window.__TAURI_INTERNALS__.invoke("plugin:app|version"));
  if (receipt.appIdentity.version !== receipt.appIdentity.declaredVersion) {
    throw new Error(`running app version ${receipt.appIdentity.version} disagrees with checkout declaration ${receipt.appIdentity.declaredVersion}`);
  }
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });
  let opened = false;
  for (const selector of ["a.page-ref=OG Parity References", "span.page-ref=OG Parity References", "*=OG Parity References"]) {
    const pageLink = await browser.$(selector);
    if (await pageLink.isExisting()) {
      await pageLink.click();
      opened = true;
      break;
    }
  }
  if (!opened) {
    await browser.keys(["Control", "k"]);
    const switcher = await browser.$(".switcher-input");
    await switcher.waitForExist({ timeout: 5000 });
    await switcher.setValue("OG Parity References");
    // The Create row appears synchronously, before the debounced graph search.
    // Wait for the exact graph-backed Pages result; selecting the first row can
    // silently create a page or choose a block hit instead of opening fixture.
    try {
      await browser.waitUntil(() => browser.execute((wanted) => [...document.querySelectorAll(".switcher-section")].some((section) =>
        section.querySelector(".switcher-group-header > span:first-child")?.textContent?.trim() === "Pages"
        && [...section.querySelectorAll(".switcher-row:not(.block-result) .switcher-name")]
          .some((element) => element.textContent?.trim() === wanted)
      ), "OG Parity References"), { timeout: 10_000, interval: 100 });
    } catch (error) {
      const dump = await browser.$(".switcher").getText().catch(() => "<switcher absent>");
      throw new Error(`exact Pages result did not resolve; switcher was ${dump.slice(0, 2400)}`, { cause: error });
    }
    const selected = await browser.execute((wanted) => {
      const section = [...document.querySelectorAll(".switcher-section")].find((candidate) =>
        candidate.querySelector(".switcher-group-header > span:first-child")?.textContent?.trim() === "Pages"
      );
      const name = [...(section?.querySelectorAll(".switcher-row:not(.block-result) .switcher-name") ?? [])]
        .find((candidate) => candidate.textContent?.trim() === wanted);
      const row = name?.closest(".switcher-row");
      if (!row) return false;
      row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
      return true;
    }, "OG Parity References");
    if (!selected) throw new Error("exact Pages result disappeared before selection");
  }
  await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === "OG Parity References", {
    timeout: 10_000,
    timeoutMsg: "OG parity reference page did not open",
  });

  const content = await browser.$(`[data-block-id="${EDITOR}"] .block-content`);
  await content.click();
  const editor = await browser.$(`[data-block-id="${EDITOR}"] textarea.block-editor`);
  await editor.waitForExist({ timeout: 5000 });
  await editor.addValue("((test");

  const activeItem = await browser.$(".autocomplete .ac-item.active");
  await activeItem.waitForExist({ timeout: 10_000 });
  receipt.observations.popup = await browser.execute(() => {
    const item = document.querySelector(".autocomplete .ac-item.active");
    return {
      label: item?.querySelector(".ac-label")?.textContent?.trim() ?? "",
      secondary: item?.querySelector(".ac-sub")?.textContent?.trim() ?? "",
      text: item?.textContent?.trim() ?? "",
    };
  });
  await browser.saveScreenshot(`${ARTIFACTS}/autocomplete.png`);
  if (receipt.observations.popup.label !== "Testing block") {
    throw new Error(`block autocomplete selected ${JSON.stringify(receipt.observations.popup)}`);
  }

  await browser.keys(["Enter"]);
  await browser.waitUntil(async () => !(await browser.$(".autocomplete").isExisting()), {
    timeout: 5000,
    timeoutMsg: "block autocomplete did not close after Enter",
  });
  receipt.observations.edit = await browser.execute(() => {
    const active = document.activeElement;
    return active instanceof HTMLTextAreaElement
      ? { value: active.value, start: active.selectionStart, end: active.selectionEnd }
      : { value: null, start: null, end: null };
  });
  const acceptedPrefix = `((${TARGET}))`;
  const acceptedValue = receipt.observations.edit.value;
  if (![acceptedPrefix, `${acceptedPrefix} `].includes(acceptedValue)
      || receipt.observations.edit.start !== receipt.observations.edit.end
      || receipt.observations.edit.start !== acceptedValue.length) {
    throw new Error(`unexpected accepted block reference: ${JSON.stringify(receipt.observations.edit)}`);
  }
  await browser.keys(["Escape"]);
  const persistedEditorValue = () => {
    const lines = fs.readFileSync(TEST_PAGE, "utf8").split("\n");
    const propertyIndex = lines.findIndex((line) => line.trim() === `id:: ${EDITOR}`);
    if (propertyIndex < 1) return null;
    const blockLine = lines[propertyIndex - 1];
    return blockLine.startsWith("- ") ? blockLine.slice(2) : null;
  };
  await browser.waitUntil(() => persistedEditorValue() === acceptedValue, {
    timeout: 10_000,
    timeoutMsg: "accepted block reference did not persist exactly in the editor block",
  });
  const rendered = await browser.$(`[data-block-id="${EDITOR}"] .block-ref`);
  await rendered.waitForExist({ timeout: 5000 });
  receipt.observations.renderedText = (await rendered.getText()).trim();
  receipt.observations.persisted = fs.readFileSync(TEST_PAGE, "utf8");
  receipt.observations.persistedEditorValue = persistedEditorValue();
  if (receipt.observations.renderedText !== "Testing block") {
    throw new Error(`rendered block reference was ${JSON.stringify(receipt.observations.renderedText)}`);
  }
  await sleep(300);
  await browser.saveScreenshot(`${ARTIFACTS}/rendered.png`);
  fs.writeFileSync(`${ARTIFACTS}/receipt.json`, `${JSON.stringify(receipt, null, 2)}\n`);
  console.log("PASS: literal block-reference autocomplete selected, rendered, and persisted the target");
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { td.kill("SIGKILL"); } catch {}
  fs.closeSync(log);
}
