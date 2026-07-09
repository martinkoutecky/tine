// e2e-blockselect.mjs — headless repro for the Esc→block-select regression.
//
// Scenario: open a page with a few plain blocks, click into a block to edit,
// press Escape (should exit editing INTO block-select mode), then ArrowDown /
// Shift+ArrowDown to walk/extend the selection. Also tests Shift+ArrowDown
// directly from an editing block (Martin: "shift-down does not start selecting").
//
// Observes real WebKitGTK focus/DOM state at each step. Does NOT modify src/.

import { spawn } from "node:child_process";
import { remote, Key } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const G = "/tmp/txdg-bsel-g";
const LOCAL_APP = path.join(ROOT, "target/release/tine");
const LOCAL_TAURI_DRIVER = path.resolve(ROOT, "..", ".toolchain", "cargo", "bin", "tauri-driver");
const CARGO_TAURI_DRIVER = process.env.CARGO_HOME
  ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver")
  : null;
const APP =
  process.env.TINE_APP ||
  (fs.existsSync(LOCAL_APP) ? LOCAL_APP : `${process.env.HOME}/research/tine`);
const TD =
  process.env.TAURI_DRIVER ||
  (CARGO_TAURI_DRIVER && fs.existsSync(CARGO_TAURI_DRIVER)
    ? CARGO_TAURI_DRIVER
    : fs.existsSync(LOCAL_TAURI_DRIVER)
      ? LOCAL_TAURI_DRIVER
      : "tauri-driver");

// Plain single-line blocks placed DIRECTLY in the journal, which the app opens
// by default → these are unambiguously main-feed blocks (in mainPages()/visibleOrder).
const JOURNAL_FIXTURE = `- alpha first block
- beta second block
- gamma third block
- delta fourth block
- epsilon fifth block
- see [[SelTest]] linked here
`;
// Also a real page fixture, to compare main-page (routed) vs journal-feed behavior.
const PAGE_FIXTURE = `- alpha first block
- beta second block
- gamma third block
- delta fourth block
- epsilon fifth block
`;

function seed() {
  fs.rmSync(G, { recursive: true, force: true });
  fs.mkdirSync(`${G}/pages`, { recursive: true });
  fs.mkdirSync(`${G}/journals`, { recursive: true });
  fs.mkdirSync(`${G}/logseq`, { recursive: true });
  fs.writeFileSync(`${G}/pages/SelTest.md`, PAGE_FIXTURE);
  // The default-open journal for "today". Use the date the app will open.
  const d = new Date();
  const stem = `${d.getFullYear()}_${String(d.getMonth() + 1).padStart(2, "0")}_${String(d.getDate()).padStart(2, "0")}`;
  fs.writeFileSync(`${G}/journals/${stem}.md`, JOURNAL_FIXTURE);
}

let xvfb;
async function ensureDisplay() {
  if (process.env.DISPLAY) return;
  const displays = process.env.XVFB_DISPLAY ? [process.env.XVFB_DISPLAY] : [":98", ":99", ":100", ":101"];
  let lastLog = "/tmp/xvfb-bsel.log";
  let lastError = "";
  for (const display of displays) {
    const suffix = display.replace(/[^0-9]/g, "") || "x";
    lastLog = `/tmp/xvfb-bsel-${suffix}.log`;
    const xvfbLog = fs.openSync(lastLog, "w");
    let spawnError = "";
    const child = spawn("Xvfb", [display, "-screen", "0", "1400x1000x24"], {
      stdio: ["ignore", xvfbLog, xvfbLog],
    });
    child.on("error", (e) => { spawnError = e.message; });
    await sleep(900);
    if (spawnError) { lastError = spawnError; continue; }
    if (child.exitCode == null) { xvfb = child; process.env.DISPLAY = display; return; }
    lastError = `display ${display} exited with code ${child.exitCode}`;
  }
  throw new Error(`Xvfb failed to start (${lastError}); see ${lastLog}`);
}

seed();
await ensureDisplay();

fs.rmSync("/tmp/txdg-bsel", { recursive: true, force: true });
for (const d of ["data", "config", "cache"]) fs.mkdirSync(`/tmp/txdg-bsel/${d}`, { recursive: true });

const env = {
  ...process.env,
  TINE_GRAPH: G,
  XDG_DATA_HOME: "/tmp/txdg-bsel/data",
  XDG_CONFIG_HOME: "/tmp/txdg-bsel/config",
  XDG_CACHE_HOME: "/tmp/txdg-bsel/cache",
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  GDK_BACKEND: "x11",
};
console.log("DISPLAY=", process.env.DISPLAY, "APP=", APP);

const tdLog = fs.openSync("/tmp/td-bsel.log", "w");
const td = spawn(TD, ["--port", "4444", "--native-port", "4445", "--native-driver", "/usr/bin/WebKitWebDriver"], {
  env, stdio: ["ignore", tdLog, tdLog],
});
await sleep(3000);

const lines = [];
const log = (line) => { console.log(line); lines.push(line); };

const probe = async (tag) => {
  const s = await browser.execute(() => {
    const ae = document.activeElement;
    const blocks = [...document.querySelectorAll(".ls-block")];
    const selectedMains = [...document.querySelectorAll(".block-main.selected")];
    const editingMains = [...document.querySelectorAll(".block-main.editing")];
    const editors = [...document.querySelectorAll("textarea.block-editor")];
    const idxOfMain = (m) => { const b = m.closest(".ls-block"); return b ? blocks.indexOf(b) : -1; };
    const textOf = (b) => (b.textContent || "").replace(/\s+/g, " ").trim().slice(0, 24);
    return {
      aeTag: ae ? ae.tagName : "null",
      aeCls: ae ? String(ae.className).slice(0, 30) : "",
      aeConnected: ae ? ae.isConnected : null,
      aeIsTextarea: ae instanceof HTMLTextAreaElement,
      aeContentEditable: ae ? ae.isContentEditable : null,
      nSelected: selectedMains.length,
      selectedIdx: selectedMains.map(idxOfMain),
      selectedText: selectedMains.map((m) => textOf(m.closest(".ls-block"))),
      nEditing: editingMains.length,
      editingIdx: editingMains.map(idxOfMain),
      nEditors: editors.length,
      activeEditorConnected: editors.map((e) => e === ae),
      nblocks: blocks.length,
    };
  });
  log(`  ${tag.padEnd(22)} ae=${s.aeTag}.${s.aeCls} conn=${s.aeConnected} ceditable=${s.aeContentEditable}` +
      ` | sel=${s.nSelected}${JSON.stringify(s.selectedIdx)}${JSON.stringify(s.selectedText)}` +
      ` editing=${s.nEditing}${JSON.stringify(s.editingIdx)} editors=${s.nEditors}`);
  return s;
};

const clickBlock = async (blockIdx, selStart) => {
  await browser.execute((idx) => {
    const blocks = [...document.querySelectorAll(".ls-block")];
    const block = blocks[idx];
    if (!block) return;
    const wrapper = block.querySelector(":scope > .block-main .block-content-wrapper");
    const el = wrapper || block;
    el.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
    el.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, cancelable: true, button: 0 }));
  }, blockIdx);
  await sleep(500);
  await browser.execute((s) => {
    const ae = document.activeElement;
    if (!(ae instanceof HTMLTextAreaElement)) return;
    const pos = s === "end" ? ae.value.length : Number(s);
    ae.setSelectionRange(pos, pos);
  }, selStart === "end" ? "end" : String(selStart));
  await sleep(150);
};

let browser;
try {
  browser = await remote({
    hostname: "127.0.0.1", port: 4444, path: "/",
    capabilities: {
      browserName: "wry",
      "wdio:enforceWebDriverClassic": true,
      "tauri:options": { application: APP },
    },
    logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60000,
  });

  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20000 });
  await sleep(1500);

  // Capture uncaught errors + console.error from the webview.
  await browser.execute(() => {
    // @ts-ignore
    window.__errs = [];
    window.addEventListener("error", (e) => window.__errs.push("onerror: " + (e.message || String(e.error))));
    window.addEventListener("unhandledrejection", (e) => window.__errs.push("unhandled: " + String(e.reason && e.reason.message || e.reason)));
    const oe = console.error;
    console.error = (...a) => { window.__errs.push("console.error: " + a.map((x) => (x && x.message) || String(x)).join(" ")); oe(...a); };
  });
  const readErrs = async (tag) => {
    const errs = await browser.execute(() => { const e = window.__errs.slice(); window.__errs.length = 0; return e; });
    if (errs.length) { log(`  [ERRORS @ ${tag}] (${errs.length}):`); for (const x of errs) log(`     ${x.slice(0, 200)}`); }
    else log(`  [no errors @ ${tag}]`);
    return errs;
  };

  // Stay on the DEFAULT journal feed — its blocks are main-feed (in visibleOrder).
  log("=== default journal feed (no navigation) ===");
  await sleep(1000);
  const blockCount = await browser.execute(() => document.querySelectorAll(".ls-block").length);
  log(`Blocks: ${blockCount}`);
  // Show which blocks are in the main feed vs a satellite surface.
  const feedMap = await browser.execute(() => [...document.querySelectorAll(".ls-block")].map((b, i) => {
    const inSat = !!b.closest(".agenda-block, .query-block, .refs-block, .right-sidebar, .block-embed, .linked-refs");
    return { i, sat: inSat, text: (b.textContent || "").replace(/\s+/g, " ").trim().slice(0, 22) };
  }));
  log(`feed map: ${JSON.stringify(feedMap)}`);

  const blockIds = async () => browser.execute(() =>
    [...document.querySelectorAll(".ls-block")].map((b) => (b.getAttribute("data-block-id") || "").slice(0, 8)));

  // Poll the selected-class count rapidly for `ms` to catch a flash-then-clear.
  const pollSelected = async (ms) => {
    const t0 = Date.now();
    const seen = [];
    while (Date.now() - t0 < ms) {
      const n = await browser.execute(() => document.querySelectorAll(".block-main.selected").length);
      seen.push(n);
      if (seen.length > 40) break;
    }
    return seen;
  };

  // ------ SCENARIO A: Esc -> block-select, then ArrowDown / Shift+ArrowDown ------
  log(`\n${"=".repeat(64)}`);
  log("SCENARIO A: click block 1, Esc, ArrowDown, Shift+ArrowDown");
  log("=".repeat(64));
  log(`ids before: ${JSON.stringify(await blockIds())}`);
  await clickBlock(1, "end");
  await probe("after click(1)");
  await readErrs("before Esc");
  await browser.keys([Key.Escape]);
  const flash = await pollSelected(1200);
  log(`  selected-count poll over ~1.2s after Esc: ${JSON.stringify(flash)}`);
  await readErrs("after Esc");
  await probe("after Escape");
  log(`ids after Esc: ${JSON.stringify(await blockIds())}`);
  await browser.keys([Key.ArrowDown]);
  await sleep(400);
  await probe("after ArrowDown");
  await browser.keys([Key.ArrowUp]);
  await sleep(400);
  await probe("after ArrowUp");
  await browser.keys([Key.Shift, Key.ArrowDown]);
  await sleep(400);
  await probe("after Shift+ArrowDown");
  await browser.keys([Key.Shift, Key.ArrowDown]);
  await sleep(400);
  await probe("after Shift+ArrowDown 2");

  // ------ SCENARIO B: click, then Shift+ArrowDown directly (no Esc) ------
  log(`\n${"=".repeat(64)}`);
  log("SCENARIO B: click block 1 (caret at end), Shift+ArrowDown directly");
  log("=".repeat(64));
  await clickBlock(1, "end");
  await probe("after click(1)");
  await browser.keys([Key.Shift, Key.ArrowDown]);
  await sleep(400);
  await probe("after Shift+ArrowDown");
  await browser.keys([Key.Shift, Key.ArrowDown]);
  await sleep(400);
  await probe("after Shift+ArrowDown 2");

  // ------ SCENARIO C: force-focus body via WebDriver keys after Esc ------
  // Probe whether the removed textarea remains activeElement or focus fell to body.
  log(`\n${"=".repeat(64)}`);
  log("SCENARIO C: Esc then inspect raw focus + dispatch synthetic ArrowDown at activeElement");
  log("=".repeat(64));
  await clickBlock(2, "end");
  await probe("after click(2)");
  await browser.keys([Key.Escape]);
  await sleep(500);
  const post = await probe("after Escape");
  // What does the app's own store think? read editingId / hasSelection via a hook if exposed.
  const introspect = await browser.execute(() => {
    const ae = document.activeElement;
    return {
      aeOuter: ae ? ae.outerHTML.slice(0, 80) : null,
      aeParentConnected: ae && ae.parentElement ? ae.parentElement.isConnected : null,
      bodyIsActive: document.activeElement === document.body,
      htmlIsActive: document.activeElement === document.documentElement,
    };
  });
  log(`  introspect: ${JSON.stringify(introspect)}`);
  // Now dispatch a synthetic ArrowDown keydown on window (capture) to see routing.
  const synthRouted = await browser.execute(() => {
    const ae = document.activeElement;
    const ev = new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true, cancelable: true });
    (ae || document.body).dispatchEvent(ev);
    return { dispatchedOn: ae ? ae.tagName : "body" };
  });
  await sleep(300);
  log(`  synthetic ArrowDown dispatched on: ${JSON.stringify(synthRouted)}`);
  await probe("after synthetic ArrowDown");

  // ------ SCENARIO D: routed PAGE (openPage) — click the page-ref, edit, Esc ------
  log(`\n${"=".repeat(64)}`);
  log("SCENARIO D: route to SelTest PAGE via page-ref, then click block 1, Esc");
  log("=".repeat(64));
  const routed = await browser.execute(() => {
    const ref = [...document.querySelectorAll("a.page-ref, span.page-ref, .page-ref")]
      .find((el) => (el.textContent || "").includes("SelTest"));
    if (!ref) return false;
    ref.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
    ref.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, cancelable: true, button: 0 }));
    ref.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }));
    return true;
  });
  await sleep(1500);
  const SAT = ".linked-references, .unlinked-references, .reference-blocks, .block-references, .embed-block, .right-sidebar, .agenda-block, .query-block";
  const routeInfo = await browser.execute((satSel) => {
    const title = (document.querySelector(".page-title, .journal-title")?.textContent || "").trim().slice(0, 30);
    const map = [...document.querySelectorAll(".ls-block")].map((b, i) => ({
      i,
      sat: !!b.closest(satSel),
      refBlock: b.classList.contains("ref-block"),
      text: (b.textContent || "").replace(/\s+/g, " ").trim().slice(0, 22),
    }));
    return { title, map };
  }, SAT);
  log(`  clicked page-ref: ${routed}  page title: ${JSON.stringify(routeInfo.title)}`);
  log(`  routed map: ${JSON.stringify(routeInfo.map)}`);
  const dmap = routeInfo.map;
  const mainIdx = dmap.findIndex((b) => !b.sat && !b.refBlock && /first block|second block/.test(b.text));
  log(`  editing MAIN-page block idx=${mainIdx} (sat=false, ref-block=false)`);
  if (mainIdx >= 0) {
    await clickBlock(mainIdx, "end");
    // Confirm the surface of the block we're actually editing.
    const surf = await browser.execute((satSel) => {
      const m = document.querySelector(".block-main.editing");
      const b = m?.closest(".ls-block");
      return {
        found: !!b,
        refBlock: b?.classList.contains("ref-block") ?? null,
        sat: b ? !!b.closest(satSel) : null,
        parentSurface: b ? (b.closest(satSel)?.className || "MAIN-CONTENT") : null,
      };
    }, SAT);
    log(`  editing-block surface: ${JSON.stringify(surf)}`);
    await probe(`after click(${mainIdx})`);
    // CORROBORATION: edit-mode caret ArrowDown should still move between blocks
    // (nextVisible has a pageVisibleOrder fallback) even though block-select can't.
    await browser.keys([Key.ArrowDown]);
    await sleep(400);
    await probe("routed-page edit ArrowDown (caret nav)");
    await clickBlock(mainIdx, "end");
    await browser.keys([Key.Escape]);
    await sleep(600);
    await probe("routed-page after Escape");
    await browser.keys([Key.ArrowDown]);
    await sleep(400);
    await probe("routed-page after ArrowDown");
    // Also test Shift+ArrowDown directly from editing on the routed page.
    await clickBlock(mainIdx, "end");
    await browser.keys([Key.Shift, Key.ArrowDown]);
    await sleep(400);
    await probe("routed-page Shift+ArrowDown from edit");
  }

  // ------ SCENARIO F: LINKED-REFERENCES satellite — edit a ref block, Esc ------
  log(`\n${"=".repeat(64)}`);
  log("SCENARIO F: edit a block inside SelTest's linked references, Esc");
  log("=".repeat(64));
  const satIdx = dmap.findIndex((b) => b.sat && b.text);
  log(`  satellite (linked-ref) block idx=${satIdx} (from routed map)`);
  // Re-read live (the routed page shows linked refs at the bottom).
  const liveSat = await browser.execute(() => {
    const blocks = [...document.querySelectorAll(".ls-block")];
    for (let i = 0; i < blocks.length; i++) {
      const b = blocks[i];
      if (b.closest(".refs-block, .linked-refs, .references, .block-embed") && (b.textContent || "").trim())
        return { i, text: (b.textContent || "").replace(/\s+/g, " ").trim().slice(0, 30) };
    }
    return null;
  });
  log(`  live satellite block: ${JSON.stringify(liveSat)}`);
  if (liveSat) {
    await clickBlock(liveSat.i, "end");
    await probe(`after click(sat ${liveSat.i})`);
    await browser.keys([Key.Escape]);
    await sleep(600);
    await probe("satellite after Escape");
  }

  log(`\nVERDICT-INPUT: see selected-block presence + activeElement across steps above.`);
} catch (e) {
  log(`\nE2E ERROR: ${String(e).split("\n").slice(0, 8).join(" | ")}`);
  process.exitCode = 1;
} finally {
  const notesDir = path.join(ROOT, "subagent-tasks/notes");
  fs.mkdirSync(notesDir, { recursive: true });
  fs.appendFileSync(
    path.join(notesDir, "issue2-block-select-raw.md"),
    `\n\n---\n# block-select e2e run (${new Date().toISOString()})\n\n\`\`\`\n${lines.join("\n")}\n\`\`\`\n`
  );
  console.log(`\nAppended raw log to subagent-tasks/notes/issue2-block-select-raw.md`);
  try { await browser?.deleteSession(); } catch {}
  td.kill("SIGKILL");
  xvfb?.kill("SIGKILL");
}
