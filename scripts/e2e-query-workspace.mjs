// Linux real-WebKit regression for GH #98/#99/#69/#137/#144/#145/#173/#179:
// authoritative evidence in Ctrl+K and references, a graph-scoped virtual
// query tab that survives restart, friendly filters/explanation, and guarded
// materialization as one ordinary query page.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_BASE = Number(process.env.E2E_DRIVER_PORT || 4492);
const NATIVE_BASE = Number(process.env.E2E_NATIVE_PORT || 4493);
const TMP = "/tmp/tine-query-workspace-e2e";
const GRAPH = `${TMP}/graph`;
const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || `${TMP}/artifacts`;

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.mkdirSync(ARTIFACTS, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
fs.writeFileSync(`${GRAPH}/pages/Research.md`, [
  "- Evidence parent",
  "  - alpha starts here, then enough context separates the terms so the excerpt must preserve both useful windows without showing the entire block; beta finishes here",
  "- alpha beta draft must be excluded",
  "- alpha repeats alpha and beta remains visible",
  "",
].join("\n"));
const longPageToken = `GH140-${"unbroken-page-token-".repeat(8)}`;
for (let index = 0; index < 140; index += 1) {
  const suffix = index === 0 ? longPageToken : `result-${String(index).padStart(3, "0")}`;
  // The persistent workspace returns at most 40 page hits plus 100 block hits.
  // Matching both the title and body reproduces the reporter's literal 140-row
  // result composition instead of a smaller page-only approximation.
  fs.writeFileSync(`${GRAPH}/pages/Overflow ${suffix}.md`, "- Overflow geometry witness alpha\n");
}
fs.writeFileSync(`${GRAPH}/pages/Query parity.md`, [
  "- {{query (and (task TODO) (priority A) (not (page Templates)) (sort-by modified desc))}}",
  "- Query ancestor one",
  "  - Query ancestor two",
  "    - TODO [#A] Included result 1",
  ...Array.from({ length: 8 }, (_, index) => `- TODO [#A] Included result ${index + 2}`),
  "",
].join("\n"));
fs.writeFileSync(`${GRAPH}/pages/Templates.md`, "- TODO [#A] Excluded template result\n");
const unlinkedRaw = [
  "Unlinked visible phrase names Query parity near the start",
  ...Array.from({ length: 48 }, (_, index) => `context-${index + 1}`),
  "and closes with Query parity",
].join(" ");
fs.writeFileSync(`${GRAPH}/pages/Unlinked source.md`, `- ${unlinkedRaw}\n`);
fs.writeFileSync(`${GRAPH}/pages/Second unlinked.md`, "- A second source also names Query parity without brackets\n");
fs.writeFileSync(`${GRAPH}/pages/Linked source.md`, [
  "- Ancestor one",
  "  - Ancestor two",
  "    - Ancestor three",
  "      - Ancestor four",
  "        - [[Query parity]] appears explicitly and [[Query parity]] appears again",
  "          - A descendant-only filter witness carries #Evidence",
  "            - Grandchild body is hidden by the reference-local depth default",
  `              - Large unrelated descendant ${"context ".repeat(120)}`,
  "- [[Query parity]] source-collapsed witness",
  "  collapsed:: true",
  "  - Source-collapsed child carries #Evidence",
  "",
].join("\n"));
fs.writeFileSync(`${GRAPH}/pages/Tagged source.md`, "tags:: Query parity\n\n- This page is tagged through a bare page property.\n");
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- Open [[Research]] and [[qUeRy PaRiTy]]\n");

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: `${TMP}/xdg/data`,
  XDG_CONFIG_HOME: `${TMP}/xdg/config`,
  XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  GDK_BACKEND: "x11",
};

async function withApp(index, fn) {
  const driverPort = DRIVER_BASE + index * 2;
  const nativePort = NATIVE_BASE + index * 2;
  const log = fs.openSync(`${TMP}/tauri-driver-${index}.log`, "w");
  const td = spawn(TD, ["--port", String(driverPort), "--native-port", String(nativePort), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], {
    env, stdio: ["ignore", log, log], detached: true,
  });
  await sleep(2500);
  let browser;
  try {
    browser = await remote({
      hostname: "127.0.0.1", port: driverPort, path: "/", logLevel: "error",
      connectionRetryCount: 1, connectionRetryTimeout: 60_000,
      capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
    });
    await browser.$(".query-workspace, .ls-block, .page-title").waitForExist({ timeout: 20_000 });
    await fn(browser);
    await sleep(750);
  } finally {
    try { await browser?.deleteSession(); } catch {}
    try { process.kill(-td.pid, "SIGKILL"); } catch {}
    fs.closeSync(log);
  }
}

async function presentationButton(browser, label) {
  for (const button of await browser.$$(".query-presentations button")) {
    if ((await button.getText()).trim() === label) return button;
  }
  throw new Error(`missing ${label} presentation button`);
}

async function inlineQueryViewButton(browser, label) {
  for (const button of await browser.$$(".query-view-switcher button")) {
    if ((await button.getText()).trim() === label) return button;
  }
  throw new Error(`missing inline-query ${label} view button`);
}

async function assertInPageFind(browser, query, activeSelector) {
  // WebKitWebDriver itself reserves native Ctrl+F on some builds; dispatch the
  // identical cancelable DOM chord so this still exercises Tine's real global
  // keybinding and find UI without handing the session to the driver's browser UI.
  await browser.execute(() => window.dispatchEvent(new KeyboardEvent("keydown", {
    key: "f", code: "KeyF", ctrlKey: true, bubbles: true, cancelable: true,
  })));
  const input = await browser.$(".inpage-find-input");
  await input.waitForExist({ timeout: 5_000 });
  await input.setValue(query);
  try {
    await browser.waitUntil(async () => {
      const count = (await browser.$(".inpage-find-count").getText()).trim();
      return count !== "" && count !== "No results";
    }, { timeout: 10_000, timeoutMsg: `in-page find did not find ${query}` });
  } catch (error) {
    const proof = await browser.execute(() => ({
      value: document.querySelector(".inpage-find-input")?.value,
      count: document.querySelector(".inpage-find-count")?.textContent,
      paneIds: [...document.querySelectorAll("[data-pane-id]")].map((element) => element.getAttribute("data-pane-id")),
      surfaces: [...document.querySelectorAll("[data-inpage-find-surface]")].map((element) => ({
        id: element.getAttribute("data-inpage-find-surface"), text: element.textContent?.trim().slice(0, 200),
      })),
    }));
    throw new Error(`${String(error)}; proof=${JSON.stringify(proof)}`);
  }
  await browser.$(activeSelector).waitForExist({ timeout: 5_000 });
  await browser.execute(() => document.querySelector(".inpage-find-input")?.dispatchEvent(
    new KeyboardEvent("keydown", { key: "Escape", code: "Escape", bubbles: true, cancelable: true })
  ));
  await input.waitForExist({ reverse: true, timeout: 5_000 });
}

await withApp(0, async (browser) => {
  // The page outline is viewport-lazy; wait for the observed AST body rather
  // than treating its short-lived literal fallback as app readiness.
  await browser.$(".page-ref").waitForExist({ timeout: 10_000 });
  const routed = await browser.execute(() => {
    const refs = [...document.querySelectorAll(".page-ref")];
    const ref = refs.find((element) => element.textContent?.toLowerCase().includes("query parity"));
    if (!ref) return {
      ok: false,
      refs: refs.map((element) => element.textContent?.trim()),
      title: document.querySelector("h1.page-title")?.textContent?.trim(),
      text: document.querySelector(".page")?.textContent?.trim().slice(0, 500),
      parser: document.documentElement.dataset.lsdocParser,
      bodyHtml: document.querySelector(".block-content")?.innerHTML.slice(0, 1_000),
    };
    for (const type of ["mousedown", "mouseup", "click"]) {
      ref.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, button: 0 }));
    }
    return { ok: true, refs: [], title: "", text: "" };
  });
  if (!routed.ok) throw new Error(`query fixture page-ref is missing: ${JSON.stringify(routed)}`);
  await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === "Query parity", {
    timeout: 10_000, timeoutMsg: "could not route to the DSL query fixture",
  });
  await browser.waitUntil(async () => (await browser.$(".query-count").getText()).trim() === "9", {
    timeout: 10_000, timeoutMsg: "ordinary query did not produce its nine expected blocks",
  });
  await browser.waitUntil(() => browser.execute(() =>
    [...document.querySelectorAll(".query-block .ref-breadcrumb")]
      .some((breadcrumb) => (breadcrumb.textContent ?? "").includes("Query ancestor one")
        && (breadcrumb.textContent ?? "").includes("Query ancestor two"))), {
    timeout: 10_000, timeoutMsg: "list query did not render the nested hit's ancestor breadcrumb",
  });
  const referenceBlocks = await browser.$(".reference-blocks");
  await referenceBlocks.waitForExist({ timeout: 10_000 });
  await referenceBlocks.scrollIntoView();
  await browser.waitUntil(async () => (await browser.$(".reference-blocks").getText()).includes("Open"), {
    timeout: 10_000, timeoutMsg: "linked-reference content did not finish rendering",
  });
  await assertInPageFind(browser, "Open", ".reference-blocks.inpage-find-active-block");
  await browser.$(".unlinked-references .references-header").click();
  const unlinkedBlocks = await browser.$(".unlinked-references .reference-blocks");
  await unlinkedBlocks.waitForExist({ timeout: 10_000 });
  await unlinkedBlocks.scrollIntoView();
  await browser.waitUntil(() => browser.execute(() =>
    [...document.querySelectorAll(".unlinked-references .reference-blocks")]
      .some((group) => group.textContent?.includes("names Query parity near the start"))), {
    timeout: 10_000, timeoutMsg: "unlinked-reference content did not finish rendering",
  });
  await assertInPageFind(browser, "names Query parity near the start", ".unlinked-references .reference-blocks.inpage-find-active-block");
  const inlineSearchButton = await inlineQueryViewButton(browser, "Search");
  await inlineSearchButton.click();
  await browser.waitUntil(async () => (await browser.$$(".query-search-results .query-search-hit")).length === 9, {
    timeout: 10_000, timeoutMsg: "Search presentation dropped ordinary DSL query results",
  });
  const inlineProof = await browser.execute(() => ({
    count: document.querySelector(".query-count")?.textContent?.trim(),
    rows: [...document.querySelectorAll(".query-search-results .query-search-hit")].map((row) => ({
      page: row.querySelector(".search-result-context")?.textContent,
      text: row.querySelector(".search-result-excerpt")?.textContent,
      marks: row.querySelectorAll("mark").length,
    })),
  }));
  if (inlineProof.count !== "9" || inlineProof.rows.length !== 9
    || inlineProof.rows.some((row) => row.page !== "Query parity" || !row.text?.includes("Included result") || row.marks !== 0)) {
    throw new Error(`DSL Search presentation changed membership or invented evidence: ${JSON.stringify(inlineProof)}`);
  }
  // The inline query's left-hand Filters/Advanced builder is a sibling Search
  // renderer to the reporter's persistent workspace. Both result families must
  // obey the same narrow-pane geometry contract.
  await browser.setWindowSize(720, 700);
  const inlineWrapProof = await browser.execute(() => {
    const container = document.querySelector(".query-search-results")?.getBoundingClientRect();
    return [...document.querySelectorAll(".query-search-results .query-search-hit")].map((row) => {
      const rect = row.getBoundingClientRect();
      return {
        scrollWidth: row.scrollWidth,
        clientWidth: row.clientWidth,
        left: rect.left,
        right: rect.right,
        containerLeft: container?.left,
        containerRight: container?.right,
      };
    });
  });
  if (!inlineWrapProof.length || inlineWrapProof.some((row) => row.scrollWidth > row.clientWidth + 1
    || row.left < row.containerLeft - 1 || row.right > row.containerRight + 1)) {
    throw new Error(`inline Filters/Advanced Search rows overflow the narrow pane: ${JSON.stringify(inlineWrapProof)}`);
  }
  await browser.saveScreenshot(path.join(ARTIFACTS, "inline-query-search.png"));

  await browser.keys(["Control", "k"]);
  const input = await browser.$(".switcher-input");
  await input.waitForExist({ timeout: 5_000 });
  await input.setValue("alpha beta -draft");
  await browser.waitUntil(async () => (await browser.$$(".switcher-results .switcher-row.block-result")).length === 2, {
    timeout: 10_000, timeoutMsg: "friendly search did not return exactly the two included blocks",
  });

  const proof = await browser.execute(() => {
    const input = document.querySelector(".switcher-input");
    const rows = [...document.querySelectorAll(".switcher-results .switcher-row.block-result")];
    return {
      role: input?.getAttribute("role"),
      active: input?.getAttribute("aria-activedescendant"),
      rows: rows.map((row) => ({
        context: row.querySelector(".search-result-context")?.textContent,
        excerpt: row.querySelector(".search-result-excerpt")?.textContent,
        marks: [...row.querySelectorAll("mark")].map((mark) => mark.textContent?.toLowerCase()),
        height: row.getBoundingClientRect().height,
      })),
    };
  });
  if (proof.role !== "combobox" || !proof.active) throw new Error(`missing accessible combobox state: ${JSON.stringify(proof)}`);
  if (proof.rows.some((row) => !row.context?.includes("Research") || row.excerpt?.includes("draft") || row.height > 90)) {
    throw new Error(`result context/excerpt bounds are wrong: ${JSON.stringify(proof)}`);
  }
  if (proof.rows.some((row) => !row.marks.includes("alpha") || !row.marks.includes("beta"))) {
    throw new Error(`authoritative evidence did not highlight all positive terms: ${JSON.stringify(proof)}`);
  }
  const before = proof.active;
  await browser.keys(["ArrowDown"]);
  const after = await input.getAttribute("aria-activedescendant");
  if (!after || after === before) throw new Error("ArrowDown did not update aria-activedescendant");

  await browser.$("[data-open-search-tab]").click();
  await browser.$(".query-workspace").waitForExist({ timeout: 10_000 });
  const source = await browser.$(".query-workspace-source");
  if ((await source.getValue()) !== "alpha beta -draft") throw new Error("query workspace lost the Ctrl+K source");

  // GH #140: every persistent presentation keeps the authoritative evidence
  // highlights, and every visible result remains an in-page-find surface.
  await browser.setWindowSize(720, 700);
  const presentations = [
    ["Search", ".query-results-search"],
    ["List", ".query-results-list"],
    ["Table", ".query-results-table"],
    ["Board", ".query-results-board"],
  ];
  for (const [label, selector] of presentations) {
    await (await presentationButton(browser, label)).click();
    await browser.$(selector).waitForExist({ timeout: 5_000 });
    await browser.waitUntil(async () => (await browser.$$(".query-workspace mark")).length >= 4, {
      timeout: 5_000, timeoutMsg: `${label} presentation dropped search highlights`,
    });
    const presentationProof = await browser.execute(() => ({
      rows: [...document.querySelectorAll(".query-workspace [data-inpage-find-surface]")].map((row) => ({
        marks: [...row.querySelectorAll("mark")].map((mark) => mark.textContent?.toLowerCase()),
      })),
    }));
    if (presentationProof.rows.length !== 2
      || presentationProof.rows.some((row) => !row.marks.includes("alpha") || !row.marks.includes("beta"))) {
      throw new Error(`${label} evidence/surface mismatch: ${JSON.stringify(presentationProof)}`);
    }
  }
  await (await presentationButton(browser, "Search")).click();
  const wrapProof = await browser.execute(() => {
    const workspace = document.querySelector(".query-workspace")?.getBoundingClientRect();
    return [...document.querySelectorAll(".query-result-row")].map((row) => {
      const rect = row.getBoundingClientRect();
      return {
        scrollWidth: row.scrollWidth,
        clientWidth: row.clientWidth,
        left: rect.left,
        right: rect.right,
        workspaceLeft: workspace?.left,
        workspaceRight: workspace?.right,
      };
    });
  });
  if (!wrapProof.length || wrapProof.some((row) => row.scrollWidth > row.clientWidth + 1
    || row.left < row.workspaceLeft - 1 || row.right > row.workspaceRight + 1)) {
    throw new Error(`persistent search rows overflow the narrow pane: ${JSON.stringify(wrapProof)}`);
  }
  await assertInPageFind(browser, "alpha", ".query-result-row.inpage-find-active-block");

  // The reporter's v0.5.9 follow-up was not a two-block row case: it was the
  // persistent workspace with roughly 140 PAGE hits, including an intrinsically
  // wide title. Drive that literal family and assert the complete pane/grid
  // chain, because a row-only white-space assertion missed the grid item's
  // automatic min-content width and let the entire pane grow horizontally.
  const clearedSource = await browser.execute(() => {
    const input = document.querySelector(".query-workspace-source");
    if (!(input instanceof HTMLInputElement)) return false;
    input.value = "";
    input.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "deleteContent" }));
    return true;
  });
  if (!clearedSource) throw new Error("persistent query source was not available to clear");
  await browser.$(".query-advanced-toggle").click();
  await browser.$(".query-advanced-modal").waitForExist({ timeout: 5_000 });
  const friendlyInputs = await browser.$$(".query-friendly-fields input");
  await friendlyInputs[0].setValue("Overflow");
  await browser.$(".query-advanced-actions .primary").click();
  await browser.$(".query-advanced-modal").waitForExist({ reverse: true, timeout: 5_000 });
  await browser.waitUntil(async () => (await browser.$$(".query-results-search .query-result-row")).length === 140, {
    timeout: 15_000, timeoutMsg: "persistent workspace did not render the 140 page-result fixture",
  });
  const fullPaneWrapProof = await browser.execute(() => {
    const pane = document.querySelector(".query-workspace")?.closest(".main-content");
    const workspace = document.querySelector(".query-workspace");
    const grid = document.querySelector(".query-results-search");
    const items = [...document.querySelectorAll('.query-results-search > [role="listitem"]')];
    const rows = [...document.querySelectorAll(".query-results-search .query-result-row")];
    const measure = (element) => element ? {
      clientWidth: element.clientWidth,
      scrollWidth: element.scrollWidth,
      left: element.getBoundingClientRect().left,
      right: element.getBoundingClientRect().right,
    } : null;
    return {
      viewport: document.documentElement.clientWidth,
      bodyScrollWidth: document.body.scrollWidth,
      pane: measure(pane),
      workspace: measure(workspace),
      grid: measure(grid),
      items: items.map(measure),
      rows: rows.map(measure),
    };
  });
  const overflows = (entry) => !entry || entry.scrollWidth > entry.clientWidth + 1
    || entry.left < -1 || entry.right > fullPaneWrapProof.viewport + 1;
  if (fullPaneWrapProof.bodyScrollWidth > fullPaneWrapProof.viewport + 1
    || overflows(fullPaneWrapProof.pane)
    || overflows(fullPaneWrapProof.workspace)
    || overflows(fullPaneWrapProof.grid)
    || fullPaneWrapProof.items.length !== 140
    || fullPaneWrapProof.items.some(overflows)
    || fullPaneWrapProof.rows.length !== 140
    || fullPaneWrapProof.rows.some(overflows)) {
    throw new Error(`persistent 140-page Search workspace overflows its pane: ${JSON.stringify(fullPaneWrapProof)}`);
  }
  await browser.saveScreenshot(path.join(ARTIFACTS, "query-workspace-140-page-wrap.png"));

  await browser.$(".query-explain-toggle").click();
  await browser.$(".query-workspace-explanation").waitForExist({ timeout: 10_000 });
  await browser.$(".query-advanced-toggle").click();
  const dialog = await browser.$(".query-advanced-modal");
  await dialog.waitForExist({ timeout: 5_000 });
  if (!(await dialog.getText()).includes("All of these words")) throw new Error("friendly advanced fields are missing");
  // The visual builder popover is a semantic child of Advanced. Reactivating
  // the modal itself must not let its parent jump ahead of the still-visible
  // child; one Escape closes only the child and preserves the draft/modal.
  await browser.$(".query-switch-to-dsl").click();
  await browser.$(".qb-bar").waitForExist({ timeout: 5_000 });
  await browser.$(".qb-chip").click();
  await browser.$(".qb-menu").waitForExist({ timeout: 5_000 });
  await browser.execute(() => document.querySelector(".query-advanced-header")?.dispatchEvent(
    new MouseEvent("pointerdown", { bubbles: true, cancelable: true })
  ));
  await browser.keys(["Escape"]);
  await browser.$(".qb-menu").waitForExist({ reverse: true, timeout: 5_000 });
  if (!(await browser.$(".query-advanced-modal").isExisting())) {
    throw new Error("QueryBuilder child Escape also closed its Advanced parent");
  }
  await browser.keys(["Escape"]);
  await browser.$(".query-advanced-modal").waitForExist({ reverse: true, timeout: 5_000 });
  const table = await presentationButton(browser, "Table");
  await table.click();
  await browser.$(".query-results-table").waitForExist({ timeout: 5_000 });
  await browser.saveScreenshot(path.join(ARTIFACTS, "query-workspace.png"));
});

// A fresh native process restores the virtual route and its presentation from
// the graph-scoped device-local session; no temporary page has been written.
await withApp(1, async (browser) => {
  await browser.$(".query-workspace").waitForExist({ timeout: 20_000 });
  const source = await browser.$(".query-workspace-source");
  if ((await source.getValue()) !== "Overflow") throw new Error("restart lost the edited virtual query source");
  const restoredTable = await presentationButton(browser, "Table");
  if ((await restoredTable.getAttribute("aria-pressed")) !== "true") {
    throw new Error("restart lost the query presentation");
  }
  const graphFilesBefore = fs.readdirSync(`${GRAPH}/pages`).sort();
  const expectedBefore = [
    "Linked source.md",
    "Query parity.md",
    "Research.md",
    "Second unlinked.md",
    "Tagged source.md",
    "Templates.md",
    "Unlinked source.md",
    ...Array.from({ length: 140 }, (_, index) => {
      const suffix = index === 0 ? longPageToken : `result-${String(index).padStart(3, "0")}`;
      return `Overflow ${suffix}.md`;
    }),
  ].sort();
  if (JSON.stringify(graphFilesBefore) !== JSON.stringify(expectedBefore)) {
    throw new Error(`virtual query wrote a temporary page: ${graphFilesBefore.join(", ")}`);
  }

  const title = await browser.$(".query-workspace-save input");
  await title.setValue("Saved evidence search");
  await browser.$(".query-workspace-save button[type=submit]").click();
  await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === "Saved evidence search", {
    timeout: 10_000, timeoutMsg: "naming did not replace the virtual route with the saved page",
  });
  await sleep(1000);
  const saved = fs.readFileSync(`${GRAPH}/pages/Saved evidence search.md`, "utf8");
  if (!saved.includes('{{query (search "Overflow")}}') || !saved.includes("tine.view:: table")) {
    throw new Error(`materialized page is not the canonical one-block query:\n${saved}`);
  }
});

// A final fresh process proves the reference evidence controls in the actual
// native WebKit app without disturbing the virtual-workspace restore scenario.
await withApp(2, async (browser) => {
  await browser.keys(["Control", "k"]);
  const input = await browser.$(".switcher-input");
  await input.waitForExist({ timeout: 5_000 });
  await input.setValue("Query parity");
  await browser.waitUntil(() => browser.execute(() =>
    [...document.querySelectorAll(".switcher-row")].some((candidate) =>
      candidate.querySelector(".switcher-kind")?.textContent?.trim() === "page"
      && candidate.querySelector(".switcher-name")?.textContent?.trim() === "Query parity")), {
    timeout: 10_000, timeoutMsg: "exact Query parity page result did not render",
  });
  const opened = await browser.execute(() => {
    const row = [...document.querySelectorAll(".switcher-row")].find((candidate) =>
      candidate.querySelector(".switcher-kind")?.textContent?.trim() === "page"
      && candidate.querySelector(".switcher-name")?.textContent?.trim() === "Query parity");
    if (!row) return false;
    row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
    return true;
  });
  if (!opened) throw new Error("could not activate the exact Query parity result");
  await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === "Query parity", {
    timeout: 10_000, timeoutMsg: "could not route back to Query parity for reference evidence proof",
  });

  await browser.$(".linked-references .reference-bulk-controls").waitForExist({ timeout: 10_000 });
  await browser.execute(() => {
    const source = [...document.querySelectorAll(".linked-references .reference-group")]
      .find((group) => group.querySelector(".reference-page")?.textContent?.trim() === "Linked source");
    source?.scrollIntoView({ block: "center" });
  });
  await browser.waitUntil(() => browser.execute(() =>
    [...document.querySelectorAll(".linked-references .reference-group")]
      .some((group) => group.querySelector(".reference-page")?.textContent?.trim() === "Linked source"
        && group.querySelector(".reference-mention-count")?.textContent?.trim() === "2 mentions")), {
    timeout: 10_000, timeoutMsg: "linked reference evidence did not lazy-mount",
  });
  const linkedProof = await browser.execute(() => {
    const groups = [...document.querySelectorAll(".linked-references .reference-group")];
    const source = groups.find((group) => group.querySelector(".reference-page")?.textContent?.trim() === "Linked source");
    const tagged = groups.find((group) => group.querySelector(".reference-page")?.textContent?.trim() === "Tagged source");
    return {
      groupCount: groups.length,
      totalRoots: Number(document.querySelector(".linked-references .references-count")?.textContent?.trim()),
      mentions: source?.querySelector(".reference-mention-count")?.textContent?.trim(),
      jumps: source?.querySelectorAll(".reference-occurrence-jump").length,
      taggedPresent: !!tagged,
      breadcrumbs: [...(source?.querySelectorAll(".ref-breadcrumb") ?? [])]
        .map((item) => item.textContent?.replace(/\s+/g, "").trim()),
    };
  });
  if (linkedProof.groupCount !== 3 || linkedProof.totalRoots !== 4
    || linkedProof.mentions !== "2 mentions" || linkedProof.jumps !== 3
    || !linkedProof.taggedPresent
    || JSON.stringify(linkedProof.breadcrumbs) !== JSON.stringify(["…›Ancestortwo›Ancestorthree›Ancestorfour"])) {
    throw new Error(`linked reference evidence is incomplete: ${JSON.stringify(linkedProof)}`);
  }

  const linkedBytesBeforeDisclosure = fs.readFileSync(`${GRAPH}/pages/Linked source.md`, "utf8");
  const depthProof = await browser.execute(() => {
    const source = [...document.querySelectorAll(".linked-references .reference-group")]
      .find((group) => group.querySelector(".reference-page")?.textContent?.trim() === "Linked source");
    const roots = [...(source?.querySelectorAll(".live-ref-group > .ls-block") ?? [])];
    const ownText = (block) => block.querySelector(":scope > .block-main .block-content")?.textContent ?? "";
    const nested = roots.find((block) => ownText(block).includes("appears explicitly"));
    const stored = roots.find((block) => ownText(block).includes("source-collapsed witness"));
    const immediate = nested?.querySelector(":scope > .block-children-container > .block-children > .ls-block");
    return {
      nestedId: nested?.getAttribute("data-block-id"),
      immediateId: immediate?.getAttribute("data-block-id"),
      nestedText: nested?.textContent ?? "",
      immediateText: immediate?.textContent ?? "",
      immediateCollapsed: immediate?.classList.contains("collapsed"),
      storedId: stored?.getAttribute("data-block-id"),
      storedCollapsed: stored?.classList.contains("collapsed"),
      storedText: stored?.textContent ?? "",
    };
  });
  if (!depthProof.nestedId || !depthProof.immediateId || !depthProof.storedId
    || !depthProof.nestedText.includes("descendant-only filter witness")
    || depthProof.nestedText.includes("Grandchild body")
    || depthProof.immediateCollapsed !== true
    || depthProof.storedCollapsed !== true
    || depthProof.storedText.includes("Source-collapsed child")) {
    throw new Error(`reference-local initial depth is wrong: ${JSON.stringify(depthProof)}`);
  }

  await browser.execute((immediateId, storedId) => {
    for (const id of [immediateId, storedId]) {
      document.querySelector(`[data-block-id="${CSS.escape(id)}"] > .block-main .collapse-toggle`)?.click();
    }
  }, depthProof.immediateId, depthProof.storedId);
  await browser.waitUntil(() => browser.execute((nestedId, storedId) => {
    const nested = document.querySelector(`[data-block-id="${CSS.escape(nestedId)}"]`);
    const stored = document.querySelector(`[data-block-id="${CSS.escape(storedId)}"]`);
    return (nested?.textContent ?? "").includes("Grandchild body")
      && (stored?.textContent ?? "").includes("Source-collapsed child");
  }, depthProof.nestedId, depthProof.storedId), {
    timeout: 5_000, timeoutMsg: "local reference disclosures did not reveal the bounded descendants",
  });
  await sleep(500);
  if (fs.readFileSync(`${GRAPH}/pages/Linked source.md`, "utf8") !== linkedBytesBeforeDisclosure) {
    throw new Error("reference-local disclosure changed the source Markdown bytes");
  }

  for (const [id, clickText, marker] of [
    [depthProof.nestedId, "appears explicitly", "live-root-edit-witness"],
    [depthProof.immediateId, "descendant-only filter witness", "live-descendant-edit-witness"],
  ]) {
    const root = `[data-block-id="${id}"]`;
    const textSpanSelector = `${root} > .block-main > .block-content-wrapper > .block-content span[data-so]`;
    // The revealed descendant must still be present under its original identity
    // after saving its parent. This is deliberately an identity/disclosure check,
    // not a text-only approximation.
    await browser.$(textSpanSelector).waitForExist({
      timeout: 10_000,
      timeoutMsg: `linked-reference edit target ${marker} did not survive the reactive refresh`,
    });
    const textSpans = await browser.$$(textSpanSelector);
    let clickTarget;
    for (const span of textSpans) {
      if ((await span.getText()).includes(clickText)) {
        clickTarget = span;
        break;
      }
    }
    if (!clickTarget) throw new Error(`missing safe linked-reference edit target ${JSON.stringify(clickText)}`);
    // Drive the native pointer sequence on ordinary rendered text. Clicking the
    // block-content box itself lands on an embedded [[page link]] at its center,
    // which correctly navigates instead of entering edit mode.
    await clickTarget.click();
    const editor = await browser.$(`${root} textarea.block-editor`);
    await editor.waitForExist({ timeout: 5_000 });
    await browser.execute((selector) => {
      const input = document.querySelector(selector);
      if (!(input instanceof HTMLTextAreaElement)) throw new Error("missing linked-reference editor");
      input.focus();
      input.setSelectionRange(input.value.length, input.value.length);
    }, `${root} textarea.block-editor`);
    await editor.addValue(` ${marker}`);
    // Do not use the page title to commit the edit: it is a navigation control,
    // so clicking it remounts the page and legitimately resets an off-viewport
    // lazy reference group. The page-actions button is a real focus target that
    // commits the editor through the ordinary blur path without changing routes.
    await browser.$(".page-actions-trigger").click();
    await browser.keys(["Escape"]);
    await browser.waitUntil(() => fs.readFileSync(`${GRAPH}/pages/Linked source.md`, "utf8").includes(marker), {
      timeout: 10_000, timeoutMsg: `live linked-reference edit did not save ${marker}`,
    });
  }
  const linkedBytesAfterEditing = fs.readFileSync(`${GRAPH}/pages/Linked source.md`, "utf8");
  if (!linkedBytesAfterEditing.includes("\n  collapsed:: true\n")) {
    throw new Error("live reference edits changed the source-collapsed property's bytes");
  }

  const linkedFilterToggle = await browser.$('.linked-references button[aria-label="Filter linked references"]');
  await linkedFilterToggle.click();
  const linkedFilterSearch = await browser.$(".linked-references .reference-filter-search");
  await linkedFilterSearch.waitForExist({ timeout: 5_000 });
  await linkedFilterSearch.setValue('"descendant-only filter witness"');
  await browser.waitUntil(() => browser.execute((expectedGroups) => {
    const summary = document.querySelector(".linked-references .reference-filter-summary")?.textContent ?? "";
    const pages = [...document.querySelectorAll(".linked-references .reference-page")]
      .map((item) => item.textContent?.trim());
    return summary.includes(`1 of ${expectedGroups}`) && pages.length === 1 && pages[0] === "Linked source";
  }, linkedProof.totalRoots), { timeout: 10_000, timeoutMsg: "descendant-only linked-reference search did not retain its backlink root" });
  await browser.saveScreenshot(`${ARTIFACTS}/linked-reference-filter.png`);

  await browser.$(".linked-references .reference-filter-clear").click();
  await browser.waitUntil(() => browser.execute((expectedGroups) =>
    (document.querySelector(".linked-references .reference-filter-summary")?.textContent ?? "").includes(`${expectedGroups} of ${expectedGroups}`),
  linkedProof.totalRoots), {
    timeout: 5_000,
    timeoutMsg: "clearing linked-reference search did not restore every root",
  });
  let evidenceFacet;
  for (const button of await browser.$$(".linked-references .ref-filter-chip")) {
    if ((await button.getText()).includes("Evidence")) {
      evidenceFacet = button;
      break;
    }
  }
  if (!evidenceFacet) throw new Error("native descendant Evidence facet did not appear");
  await evidenceFacet.click();
  await browser.waitUntil(() => browser.execute(() => {
    const pages = [...document.querySelectorAll(".linked-references .reference-page")]
      .map((item) => item.textContent?.trim());
    return pages.length === 1 && pages[0] === "Linked source";
  }), { timeout: 5_000, timeoutMsg: "including the descendant Evidence facet did not retain Linked source" });
  await evidenceFacet.click();
  await browser.waitUntil(() => browser.execute((expectedGroups) => {
    const pages = [...document.querySelectorAll(".linked-references .reference-page")]
      .map((item) => item.textContent?.trim());
    return pages.length === expectedGroups - 1 && !pages.includes("Linked source");
  }, linkedProof.groupCount), { timeout: 5_000, timeoutMsg: "excluding the descendant Evidence facet did not hide Linked source" });
  await evidenceFacet.click();
  await browser.waitUntil(() => browser.execute(
    (expectedGroups) => document.querySelectorAll(".linked-references .reference-page").length === expectedGroups,
    linkedProof.groupCount,
  ), {
    timeout: 5_000,
    timeoutMsg: "third facet click did not clear the persisted include/exclude state",
  });

  const linkedBulk = await browser.$$(".linked-references .reference-bulk-controls button");
  await linkedBulk[0].click();
  await browser.waitUntil(async () => (await browser.$$(".linked-references .reference-blocks")).length === 0, {
    timeout: 5_000, timeoutMsg: "Collapse all did not unmount linked reference bodies",
  });
  await linkedBulk[1].click();
  await browser.waitUntil(async () => (await browser.$$(".linked-references .reference-blocks")).length === linkedProof.groupCount, {
    timeout: 5_000, timeoutMsg: "Expand all did not restore linked reference bodies",
  });

  const unlinkedHeader = await browser.$(".unlinked-references .references-header");
  // WebKitDriver does not reliably scroll an off-viewport non-button element
  // before its native click. Scroll, then drive the actual header interaction.
  await unlinkedHeader.scrollIntoView();
  await unlinkedHeader.click();
  await browser.$(".unlinked-references .reference-bulk-controls").waitForExist({ timeout: 10_000 });
  const unlinkedProof = await browser.execute((expectedRaw) => {
    const groups = [...document.querySelectorAll(".unlinked-references .reference-group")];
    const source = groups.find((group) => group.querySelector(".reference-page")?.textContent?.trim() === "Unlinked source");
    const excerpt = source?.querySelector(".reference-excerpt-text")?.textContent ?? "";
    return {
      groupCount: groups.length,
      mentions: source?.querySelector(".reference-mention-count")?.textContent?.trim(),
      jumps: source?.querySelectorAll(".reference-occurrence-jump").length,
      marks: source?.querySelectorAll("mark").length,
      bounded: excerpt.length < expectedRaw.length,
    };
  }, unlinkedRaw);
  if (unlinkedProof.groupCount < 2 || unlinkedProof.mentions !== "2 mentions"
    || unlinkedProof.jumps !== 2 || unlinkedProof.marks !== 2 || !unlinkedProof.bounded) {
    throw new Error(`unlinked reference evidence is incomplete: ${JSON.stringify(unlinkedProof)}`);
  }

  const showFull = await browser.$(".unlinked-references .reference-group .reference-show-full");
  await showFull.click();
  if ((await showFull.getAttribute("aria-expanded")) !== "true") {
    throw new Error("Show full block did not expose the complete source block");
  }
  await showFull.click();

  const unlinkedBulk = await browser.$$(".unlinked-references .reference-bulk-controls button");
  await unlinkedBulk[0].click();
  await browser.waitUntil(async () => (await browser.$$(".unlinked-references .reference-blocks")).length === 0, {
    timeout: 5_000, timeoutMsg: "Collapse all did not unmount unlinked reference bodies",
  });
  await unlinkedBulk[1].click();
  await browser.waitUntil(async () => (await browser.$$(".unlinked-references .reference-blocks")).length === unlinkedProof.groupCount, {
    timeout: 5_000, timeoutMsg: "Expand all did not restore unlinked reference bodies",
  });

  const jumped = await browser.execute(() => {
    const source = [...document.querySelectorAll(".unlinked-references .reference-group")]
      .find((group) => group.querySelector(".reference-page")?.textContent?.trim() === "Unlinked source");
    const jump = source?.querySelectorAll(".reference-occurrence-jump")[1];
    if (!(jump instanceof HTMLButtonElement)) return false;
    jump.click();
    return true;
  });
  if (!jumped) throw new Error("second unlinked occurrence control is missing");
  await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === "Unlinked source", {
    timeout: 10_000, timeoutMsg: "occurrence jump did not open its source page",
  });
  const editor = await browser.$("textarea.block-editor");
  await editor.waitForExist({ timeout: 10_000 });
  const caret = await browser.execute(() => {
    const textarea = document.querySelector("textarea.block-editor");
    return textarea instanceof HTMLTextAreaElement
      ? { value: textarea.value, start: textarea.selectionStart, end: textarea.selectionEnd }
      : null;
  });
  const expectedOffset = unlinkedRaw.lastIndexOf("Query parity");
  if (!caret || caret.value !== unlinkedRaw || caret.start !== expectedOffset || caret.end !== expectedOffset) {
    throw new Error(`exact occurrence jump landed at the wrong caret: ${JSON.stringify({ caret, expectedOffset })}`);
  }
});

console.log("PASS: typed search/reference evidence, persistent virtual query workspace, disclosure, exact jumps, explanation, and guarded save work in WebKit");
