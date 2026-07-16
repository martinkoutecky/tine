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
const APP = process.env.TINE_APP || path.join(ROOT, "target/release", process.platform === "win32" ? "tine.exe" : "tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4660);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4661);
const TMP = process.env.E2E_TMP_DIR || `/tmp/tine-og-parity-references-${process.pid}`;
const GRAPH = `${TMP}/graph`;
const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || `${TMP}/artifacts`;
const TARGET = "11111111-1111-4111-8111-111111111111";
const EDITOR = "22222222-2222-4222-8222-222222222222";
const SLASH_EDITOR = "33333333-3333-4333-8333-333333333333";
const LINK_EDITOR = "44444444-4444-4444-8444-444444444444";
const TEST_PAGE = `${GRAPH}/pages/OG Parity References.md`;
const APP_DATA = `${TMP}/xdg/data/page.tine.Tine`;
const SETTINGS = `${APP_DATA}/tine-settings.json`;

/** WebDriver's `addValue` is a convenience mutation, not the literal key path
 * the reported Linux behavior uses. Keep all user-facing autocomplete probes on
 * real key events so WebKit dispatch, Solid's input handler, and the debounce
 * are exercised together. */
const typeKeys = async (browser, text) => {
  // Separate WebDriver commands preserve key-up/key-down boundaries for repeated
  // delimiters such as `[[` and `((` on WebKitGTK.
  for (const key of text) await browser.keys([key]);
};
const activeAutocomplete = async (browser) => browser.execute(() => {
  const active = document.querySelector(".autocomplete .ac-item.active");
  return {
    active: active?.querySelector(".ac-label")?.textContent?.trim() ?? "",
    labels: [...document.querySelectorAll(".autocomplete .ac-label")].map((node) => node.textContent?.trim() ?? ""),
    editor: document.activeElement instanceof HTMLTextAreaElement ? document.activeElement.value : null,
  };
});
const waitForActiveAutocomplete = async (browser, expected, timeoutMsg) => {
  let last = null;
  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    last = await activeAutocomplete(browser);
    if (last.active === expected) return;
    await sleep(50);
  }
  throw new Error(`${timeoutMsg}; last=${JSON.stringify(last)}`);
};
const editorState = (browser) => browser.execute(() => {
  const active = document.activeElement;
  return active instanceof HTMLTextAreaElement
    ? { value: active.value, start: active.selectionStart, end: active.selectionEnd }
    : { value: null, start: null, end: null };
});
const clearActiveEditor = async (browser) => {
  await browser.keys(["Control", "a"]);
  await browser.keys(["Backspace"]);
  await browser.waitUntil(async () => (await editorState(browser)).value === "", {
    timeout: 5000,
    timeoutMsg: "literal Ctrl-A/Backspace did not clear the active editor",
  });
};
const expectEditor = async (browser, expected, caret, message) => {
  let last = null;
  const deadline = Date.now() + 5000;
  while (Date.now() < deadline) {
    last = await editorState(browser);
    if (last.value === expected && last.start === caret && last.end === caret) return last;
    await sleep(50);
  }
  throw new Error(`${message}; last=${JSON.stringify(last)}`);
};
const popupSnapshot = (browser) => browser.execute(() => ({
  active: document.querySelector(".autocomplete .ac-item.active .ac-label")?.textContent?.trim() ?? "",
  labels: [...document.querySelectorAll(".autocomplete .ac-label")].map((node) => node.textContent?.trim() ?? ""),
  rows: document.querySelectorAll(".autocomplete .ac-item").length,
}));
const selectSetting = async (browser, value) => {
  await browser.$('button[title^="Settings"]').click();
  await browser.$(".settings-modal").waitForExist({ timeout: 5000 });
  await browser.$("//button[contains(concat(' ', normalize-space(@class), ' '), ' settings-nav-item ') and normalize-space(.)='Editor']").click();
  const advanced = await browser.$(".settings-advanced-toggle");
  if ((await advanced.getAttribute("aria-expanded")) !== "true") await advanced.click();
  const select = await browser.$('select[aria-label="Link autocomplete default"]');
  await select.waitForExist({ timeout: 5000 });
  await select.selectByAttribute("value", value);
  await browser.waitUntil(() => fs.existsSync(SETTINGS) && JSON.parse(fs.readFileSync(SETTINGS, "utf8")).link_autocomplete_policy === value, {
    timeout: 5000,
    timeoutMsg: `Settings UI did not persist ${value} policy`,
  });
  await browser.keys(["Escape"]);
  await browser.$(".settings-modal").waitForExist({ reverse: true, timeout: 5000 });
};
const toggleReferenceSpacing = async (browser) => {
  await browser.$('button[title^="Settings"]').click();
  await browser.$(".settings-modal").waitForExist({ timeout: 5000 });
  const row = await browser.$('[data-setting-label="Space after inserting a reference"]');
  await row.waitForExist({ timeout: 5000 });
  await row.$('.settings-toggle[aria-checked="false"]').click();
  await browser.waitUntil(() => fs.existsSync(SETTINGS) && JSON.parse(fs.readFileSync(SETTINGS, "utf8")).space_after_ref_completion === true, {
    timeout: 5000,
    timeoutMsg: "Settings UI did not enable reference continuation spacing",
  });
  await browser.keys(["Escape"]);
};
const ensureParityPage = async (browser) => {
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });
  const current = await browser.$("h1.page-title").getText().catch(() => "");
  if (current.trim() !== "OG Parity References") {
    await browser.keys(["Control", "k"]);
    const input = await browser.$(".switcher-input");
    await input.waitForExist({ timeout: 5000 });
    await input.setValue("OG Parity References");
    await browser.waitUntil(() => browser.execute((wanted) => [...document.querySelectorAll(".switcher-row:not(.block-result) .switcher-name")]
      .some((node) => node.textContent?.trim() === wanted), "OG Parity References"), {
      timeout: 10_000,
      timeoutMsg: "named parity page was absent from Ctrl-K after restart",
    });
    const clicked = await browser.execute((wanted) => {
      const name = [...document.querySelectorAll(".switcher-row:not(.block-result) .switcher-name")]
        .find((node) => node.textContent?.trim() === wanted);
      const row = name?.closest(".switcher-row");
      if (!row) return false;
      row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
      return true;
    }, "OG Parity References");
    if (!clicked) throw new Error("named parity result disappeared during restart navigation");
  }
  await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === "OG Parity References", {
    timeout: 10_000,
    timeoutMsg: "routed named parity page did not become active",
  });
};

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.mkdirSync(ARTIFACTS, { recursive: true });
fs.mkdirSync(APP_DATA, { recursive: true });
fs.writeFileSync(SETTINGS, '{"link_autocomplete_policy":"adaptive","space_after_ref_completion":false}\n');
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
fs.writeFileSync(TEST_PAGE, [
  "- Testing block",
  `  id:: ${TARGET}`,
  "- ",
  `  id:: ${EDITOR}`,
  "- ",
  `  id:: ${SLASH_EDITOR}`,
  "- Parity label",
  `  id:: ${LINK_EDITOR}`,
  "",
].join("\n"));
fs.writeFileSync(`${GRAPH}/pages/Parity Target.md`, "- target\n");
fs.writeFileSync(`${GRAPH}/pages/Parity Target___Child.md`, "- child\n");
fs.writeFileSync(`${GRAPH}/pages/Fuzzy Existing.md`, "- fuzzy target\n");
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
const driverArgs = process.platform === "linux"
  ? ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"]
  : ["--port", String(DRIVER_PORT)];
let td = spawn(TD, driverArgs, {
  env,
  stdio: ["ignore", log, log],
  detached: process.platform !== "win32",
});
const killDriverTree = () => {
  try {
    if (process.platform === "win32") td.kill("SIGKILL");
    else process.kill(-td.pid, "SIGKILL");
  } catch {}
};
await sleep(2500);

let browser;
const receipt = {
  schemaVersion: 1,
  scenario: "references.authoring",
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
  await typeKeys(browser, "((test");

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
  // GH #155: bare slash must select Page reference, chain into the active but
  // row-free blank page lifecycle, then use ordinary adaptive page completion.
  const slashContent = await browser.$(`[data-block-id="${SLASH_EDITOR}"] .block-content`);
  await slashContent.click();
  const slashEditor = await browser.$(`[data-block-id="${SLASH_EDITOR}"] textarea.block-editor`);
  await slashEditor.waitForExist({ timeout: 5000 });
  await typeKeys(browser, "/");
  await waitForActiveAutocomplete(browser, "Page reference", "bare slash did not activate Page reference");
  await browser.keys(["Enter"]);
  await browser.waitUntil(async () => (await slashEditor.getValue()) === "[[]]", {
    timeout: 5000,
    timeoutMsg: "Page reference did not leave the caret in an empty pair",
  });
  const blankPopupRows = await browser.execute(() => document.querySelectorAll(".autocomplete .ac-item").length);
  if (blankPopupRows !== 0) throw new Error(`blank page lifecycle rendered ${blankPopupRows} rows`);
  receipt.observations.bareSlash = await browser.execute(() => {
    const active = document.activeElement;
    return active instanceof HTMLTextAreaElement ? { value: active.value, start: active.selectionStart, end: active.selectionEnd } : null;
  });
  if (receipt.observations.bareSlash?.start !== 2 || receipt.observations.bareSlash?.end !== 2) {
    throw new Error(`Page reference caret was not inside [[]]: ${JSON.stringify(receipt.observations.bareSlash)}`);
  }
  await typeKeys(browser, "P");
  await browser.waitUntil(async () => (await popupSnapshot(browser)).rows > 0, {
    timeout: 10_000,
    timeoutMsg: "first character did not populate the active blank page picker",
  });
  receipt.observations.bareSlashFirstCharacter = await popupSnapshot(browser);
  await typeKeys(browser, "arity Tar");
  await waitForActiveAutocomplete(browser, "Parity Target", "adaptive page completion did not activate the shortest prefix match");
  receipt.observations.adaptivePrefixOgSpacing = await popupSnapshot(browser);
  await browser.keys(["Enter"]);
  await expectEditor(browser, "[[Parity Target]]", 17, "adaptive page completion did not preserve OG byte spacing");
  await browser.keys(["Escape"]);

  // The GH #35 continuation space remains independently configurable. Flip it
  // through the visible Settings control, then repeat the same literal flow.
  await toggleReferenceSpacing(browser);
  await slashContent.click();
  await slashEditor.waitForExist({ timeout: 5000 });
  await clearActiveEditor(browser);
  await typeKeys(browser, "[[Parity Tar");
  await waitForActiveAutocomplete(browser, "Parity Target", "Tine-spacing prefix did not select the existing page");
  await browser.keys(["Enter"]);
  receipt.observations.adaptivePrefixTineSpacing = await expectEditor(
    browser,
    "[[Parity Target]] ",
    18,
    "adaptive page completion did not add the configured continuation space",
  );

  const pageProbe = async (query, expectedActive, expectedAccepted, check) => {
    await clearActiveEditor(browser);
    await typeKeys(browser, `[[${query}`);
    await waitForActiveAutocomplete(browser, expectedActive, `page probe ${query} had the wrong active row`);
    const snapshot = await popupSnapshot(browser);
    check(snapshot);
    await browser.keys(["Enter"]);
    const accepted = await expectEditor(
      browser,
      `${expectedAccepted} `,
      expectedAccepted.length + 1,
      `page probe ${query} did not apply its active outcome`,
    );
    return { ...snapshot, accepted };
  };
  receipt.observations.adaptiveOutcomes = {
    exact: await pageProbe("Parity Target", "Parity Target", "[[Parity Target]]", (snapshot) => {
      if (snapshot.labels[0] !== "Parity Target" || snapshot.labels.some((label) => label.startsWith('Create "'))) {
        throw new Error(`exact page completion was not deduplicated: ${JSON.stringify(snapshot)}`);
      }
    }),
    fuzzyOnly: await pageProbe("Fz", 'Create "Fz"', "[[Fz]]", (snapshot) => {
      if (snapshot.labels[1] !== "Fuzzy Existing") throw new Error(`adaptive fuzzy ordering drifted: ${JSON.stringify(snapshot)}`);
    }),
    nonexistent: await pageProbe("NoSuchC1", 'Create "NoSuchC1"', "[[NoSuchC1]]", (snapshot) => {
      if (snapshot.labels.length !== 1) throw new Error(`nonexistent completion exposed unexpected rows: ${JSON.stringify(snapshot)}`);
    }),
  };

  const slashSentinel = async (query, expectedActive, expectedPrefix) => {
    await clearActiveEditor(browser);
    await typeKeys(browser, `/${query}`);
    await waitForActiveAutocomplete(browser, expectedActive, `slash sentinel /${query} drifted`);
    const snapshot = await popupSnapshot(browser);
    if (expectedPrefix.some((label, index) => snapshot.labels[index] !== label)) {
      throw new Error(`slash sentinel /${query} order drifted: ${JSON.stringify(snapshot)}`);
    }
    await browser.keys(["Escape"]);
    return snapshot;
  };
  receipt.observations.slashRank = {
    A: await slashSentinel("A", "Priority A", ["Priority A"]),
    priority: await slashSentinel("priority", "Priority A", ["Priority A", "Priority B", "Priority C"]),
    kanban: await slashSentinel("kanban", "Board", ["Board"]),
    query: await slashSentinel("query", "Query", ["Query", "Query (visual builder)"]),
  };
  await clearActiveEditor(browser);
  await browser.keys(["Escape"]);

  // The literal Linux Mod-L chord reaches the editor dispatcher, wraps selected
  // text and leaves the collapsed caret in the Markdown URL field.
  const linkContent = await browser.$(`[data-block-id="${LINK_EDITOR}"] .block-content`);
  await linkContent.click();
  const linkEditor = await browser.$(`[data-block-id="${LINK_EDITOR}"] textarea.block-editor`);
  await linkEditor.waitForExist({ timeout: 5000 });
  await browser.execute(() => {
    window.__ogParityModL = [];
    window.addEventListener("keydown", (event) => {
      if (event.ctrlKey && event.key.toLowerCase() === "l") window.__ogParityModL.push(event.defaultPrevented);
    });
  });
  await browser.keys(["Control", "a"]);
  await browser.keys(["Control", "l"]);
  await browser.waitUntil(async () => (await linkEditor.getValue()) === "[Parity label]()", {
    timeout: 5000,
    timeoutMsg: "Mod-L did not insert the selected Markdown link",
  });
  receipt.observations.modL = await browser.execute(() => {
    const active = document.activeElement;
    return active instanceof HTMLTextAreaElement ? { value: active.value, start: active.selectionStart, end: active.selectionEnd } : null;
  });
  if (receipt.observations.modL?.start !== 15 || receipt.observations.modL?.end !== 15) {
    throw new Error(`Mod-L caret was not inside (): ${JSON.stringify(receipt.observations.modL)}`);
  }
  const consumed = await browser.execute(() => window.__ogParityModL?.at(-1) ?? false);
  if (!consumed) throw new Error("literal Linux Mod-L was not preventDefault-consumed by the editor");
  receipt.observations.modL.consumed = consumed;

  await clearActiveEditor(browser);
  await browser.keys(["Control", "l"]);
  receipt.observations.linkEmpty = await expectEditor(browser, "[]()", 1, "Mod-L empty selection branch drifted");

  await clearActiveEditor(browser);
  await typeKeys(browser, "https://example.com");
  await browser.keys(["Control", "a"]);
  await browser.keys(["Control", "l"]);
  receipt.observations.linkRecognizedUrl = await expectEditor(
    browser,
    "[](https://example.com)",
    1,
    "Mod-L recognized URL branch drifted",
  );

  await clearActiveEditor(browser);
  await typeKeys(browser, "Toolbar label");
  await browser.keys(["Control", "a"]);
  const toolbarLink = await browser.$('.sel-toolbar [data-selection-action="link"]');
  await toolbarLink.waitForExist({ timeout: 5000 });
  await toolbarLink.click();
  receipt.observations.toolbarLink = await expectEditor(
    browser,
    "[Toolbar label]()",
    16,
    "selection-toolbar Link did not use the shared format-aware insertion boundary",
  );

  await clearActiveEditor(browser);
  await typeKeys(browser, "/link");
  await waitForActiveAutocomplete(browser, "Link", "simple slash Link action was not active");
  await browser.keys(["Enter"]);
  receipt.observations.slashLink = await expectEditor(
    browser,
    "[]()",
    1,
    "simple slash Link did not use the shared empty-selection boundary",
  );

  // Remap the command through the visible Keyboard shortcuts pane, then prove
  // the live editor dispatch follows the override rather than the built-in.
  await browser.keys(["Escape"]);
  await browser.$('button[title^="Settings"]').click();
  await browser.$(".settings-modal").waitForExist({ timeout: 5000 });
  await browser.$("//button[contains(concat(' ', normalize-space(@class), ' '), ' settings-nav-item ') and normalize-space(.)='Keyboard shortcuts']").click();
  const insertLinkKeycap = await browser.$("//span[contains(concat(' ', normalize-space(@class), ' '), ' help-shortcut-id ') and normalize-space(.)='editor/insert-link']/ancestor::div[contains(concat(' ', normalize-space(@class), ' '), ' help-shortcut-row ')]//button[contains(concat(' ', normalize-space(@class), ' '), ' help-keycap-button ')]");
  await insertLinkKeycap.waitForExist({ timeout: 5000 });
  await insertLinkKeycap.click();
  await browser.keys(["Control", "Shift", "l"]);
  await browser.waitUntil(async () => (await insertLinkKeycap.getAttribute("class")).includes("overridden"), {
    timeout: 5000,
    timeoutMsg: "Insert link shortcut override did not become active",
  });
  receipt.observations.configuredOverride = { displayed: (await insertLinkKeycap.getText()).trim() };
  await browser.keys(["Escape"]);
  await linkContent.click();
  await linkEditor.waitForExist({ timeout: 5000 });
  await clearActiveEditor(browser);
  await typeKeys(browser, "Override label");
  await browser.keys(["Control", "a"]);
  await browser.keys(["Control", "Shift", "l"]);
  receipt.observations.configuredOverride.edit = await expectEditor(
    browser,
    "[Override label]()",
    17,
    "configured Insert link shortcut did not override the built-in",
  );

  // Change policy through Advanced Settings and prove the already-mounted
  // editor consumes the new mode without a reload.
  await browser.keys(["Escape"]);
  await selectSetting(browser, "typed");
  await slashContent.click();
  await slashEditor.waitForExist({ timeout: 5000 });
  await clearActiveEditor(browser);
  await typeKeys(browser, "[[FEx");
  await waitForActiveAutocomplete(browser, 'Create "FEx"', "typed policy was not live in the mounted main WebView");
  receipt.observations.typedLive = await popupSnapshot(browser);
  await browser.keys(["Escape"]);
  await clearActiveEditor(browser);
  await browser.keys(["Escape"]);

  // Restart the actual app with the same XDG directory. This proves startup
  // initialization from device settings, not merely the live Solid signal.
  await browser.deleteSession();
  browser = undefined;
  killDriverTree();
  await sleep(800);
  td = spawn(TD, driverArgs, { env, stdio: ["ignore", log, log], detached: process.platform !== "win32" });
  await sleep(2500);
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
  await ensureParityPage(browser);
  const restartedSlashContent = await browser.$(`[data-block-id="${SLASH_EDITOR}"] .block-content`);
  await restartedSlashContent.click();
  const restartedSlashEditor = await browser.$(`[data-block-id="${SLASH_EDITOR}"] textarea.block-editor`);
  await restartedSlashEditor.waitForExist({ timeout: 5000 });
  await typeKeys(browser, "[[FEx");
  await waitForActiveAutocomplete(browser, 'Create "FEx"', "typed policy did not survive a same-XDG restart");
  receipt.observations.typedRestart = await popupSnapshot(browser);
  await browser.keys(["Escape"]);
  await clearActiveEditor(browser);
  await browser.keys(["Escape"]);

  await selectSetting(browser, "existing");
  await restartedSlashContent.click();
  await restartedSlashEditor.waitForExist({ timeout: 5000 });
  await typeKeys(browser, "[[Fzy");
  await waitForActiveAutocomplete(browser, "Fuzzy Existing", "existing-first policy did not lead with a fuzzy match");
  receipt.observations.existingLive = await popupSnapshot(browser);
  await browser.keys(["Escape"]);
  await clearActiveEditor(browser);
  await browser.keys(["Escape"]);

  // A duplicate split is a second editor surface in the same main WebView.
  // Accept there under existing-first so shared policy initialization is an
  // observation rather than an assumption.
  await browser.keys(["Control", "Alt", "\\"]);
  await browser.waitUntil(() => browser.execute(() => document.querySelectorAll("[data-pane-id]").length === 2), {
    timeout: 8000,
    timeoutMsg: "native split did not create the second editor surface",
  });
  const paneIds = await browser.execute(() => [...document.querySelectorAll("[data-pane-id]")].map((pane) => pane.getAttribute("data-pane-id")));
  const splitPane = paneIds.find((id) => id && id !== "main");
  if (!splitPane) throw new Error(`could not identify duplicate split pane: ${JSON.stringify(paneIds)}`);
  const splitContent = await browser.$(`[data-pane-id="${splitPane}"] [data-block-id="${SLASH_EDITOR}"] .block-content`);
  await splitContent.click();
  const splitEditor = await browser.$(`[data-pane-id="${splitPane}"] [data-block-id="${SLASH_EDITOR}"] textarea.block-editor`);
  await splitEditor.waitForExist({ timeout: 5000 });
  await typeKeys(browser, "[[Fzy");
  await waitForActiveAutocomplete(browser, "Fuzzy Existing", "split editor did not consume main-WebView existing-first policy");
  await browser.keys(["Enter"]);
  receipt.observations.splitExisting = await expectEditor(
    browser,
    "[[Fuzzy Existing]] ",
    19,
    "split editor did not accept the existing page with configured spacing",
  );
  await browser.keys(["Escape"]);
  const persistedSplitValue = () => {
    const lines = fs.readFileSync(TEST_PAGE, "utf8").split("\n");
    const propertyIndex = lines.findIndex((line) => line.trim() === `id:: ${SLASH_EDITOR}`);
    if (propertyIndex < 1) return null;
    return lines[propertyIndex - 1].replace(/^- /, "");
  };
  await browser.waitUntil(() => persistedSplitValue() === "[[Fuzzy Existing]]", {
    timeout: 10_000,
    timeoutMsg: "split acceptance did not commit through the guarded save path",
  });
  receipt.observations.commitBeforeReload = { disk: persistedSplitValue() };

  // One final process reload proves the ordinary committed Markdown renders.
  await browser.deleteSession();
  browser = undefined;
  killDriverTree();
  await sleep(800);
  td = spawn(TD, driverArgs, { env, stdio: ["ignore", log, log], detached: process.platform !== "win32" });
  await sleep(2500);
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER_PORT, path: "/", logLevel: "error",
    connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
  });
  await ensureParityPage(browser);
  const reloadedRef = await browser.$(`[data-block-id="${SLASH_EDITOR}"] .page-ref`);
  await reloadedRef.waitForExist({ timeout: 10_000 });
  receipt.observations.reload = {
    rendered: (await reloadedRef.getText()).trim(),
    disk: persistedSplitValue(),
    policy: JSON.parse(fs.readFileSync(SETTINGS, "utf8")).link_autocomplete_policy,
  };
  if (receipt.observations.reload.rendered !== "Fuzzy Existing" || receipt.observations.reload.disk !== "[[Fuzzy Existing]]") {
    throw new Error(`committed page reference did not survive reload: ${JSON.stringify(receipt.observations.reload)}`);
  }
  await sleep(300);
  await browser.saveScreenshot(`${ARTIFACTS}/rendered.png`);
  fs.writeFileSync(`${ARTIFACTS}/receipt.json`, `${JSON.stringify(receipt, null, 2)}\n`);
} finally {
  try { await browser?.deleteSession(); } catch {}
  killDriverTree();
  fs.closeSync(log);
}

// The established native capture scenario is the byte-identical observation
// required by section 6.2(8): cold non-default policy, main-Settings live
// change, hidden-WebView reopen, same-XDG process restart, page/tag Enter, and
// saved literal input. Execute it as part of this matrix; do not replace it with
// a `coveredBy` claim. It is Linux/X11-specific because native focus is proven
// independently with xdotool before WebDriver sends keys.
if (process.platform === "linux") {
  const captureArtifacts = `${ARTIFACTS}/quick-capture`;
  fs.mkdirSync(captureArtifacts, { recursive: true });
  await sleep(700);
  const captureResult = await new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [path.join(ROOT, "scripts/e2e-capture.mjs")], {
      cwd: ROOT,
      env: {
        ...process.env,
        TINE_APP: APP,
        E2E_ARTIFACT_DIR: captureArtifacts,
        // The routed-page driver tree has already been reaped; reuse its two
        // runner-allocated free ports rather than doing unsafe arithmetic near
        // the top of the TCP range.
        E2E_DRIVER_PORT: String(DRIVER_PORT),
        E2E_NATIVE_PORT: String(NATIVE_PORT),
        E2E_WINDOW_MANAGER: process.env.E2E_CAPTURE_WINDOW_MANAGER || "openbox",
      },
      stdio: ["ignore", "pipe", "pipe"],
      detached: true,
    });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (chunk) => { stdout += chunk; });
    child.stderr.on("data", (chunk) => { stderr += chunk; });
    const timer = setTimeout(() => {
      try { process.kill(-child.pid, "SIGKILL"); } catch {}
      reject(new Error(`native quick-capture subscenario timed out; stdout=${stdout}; stderr=${stderr}`));
    }, 120_000);
    child.once("error", (error) => {
      clearTimeout(timer);
      reject(error);
    });
    child.once("exit", (code, signal) => {
      clearTimeout(timer);
      if (code === 0) resolve({ stdout: stdout.trim(), stderr: stderr.trim() });
      else reject(new Error(`native quick-capture subscenario failed code=${code} signal=${signal}; stdout=${stdout}; stderr=${stderr}`));
    });
  });
  receipt.observations.quickCapture = captureResult;
} else {
  receipt.observations.quickCapture = {
    status: "not-run",
    reason: "native X11 focus observation is Linux-only; no cross-platform proxy was claimed",
  };
}
fs.writeFileSync(`${ARTIFACTS}/receipt.json`, `${JSON.stringify(receipt, null, 2)}\n`);
console.log("PASS: reference-authoring native matrix covered routed page, split editor, restart, guarded disk/render, and Linux quick capture");
