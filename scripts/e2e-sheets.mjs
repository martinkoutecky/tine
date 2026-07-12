// E2E smoke for Sheets Phase 2a: drives the REAL Tine binary (real backend,
// real save path) under Xvfb via tauri-driver. Seeds a journal with a 2x2 grid,
// clicks into a cell, types, commits, and asserts the on-disk markdown changed
// exactly as expected (cell text edited; structure intact; Enter did not split).
// Usage: DISPLAY=:99 node scripts/e2e-sheets.mjs
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const G = "/tmp/sheets-e2e";
const APP = process.env.TINE_APP || `${repo}/target/release/tine`;
const TD =
  process.env.TAURI_DRIVER ||
  (process.env.CARGO_HOME ? `${process.env.CARGO_HOME}/bin/tauri-driver` : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4444);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4445);

// Today's journal file name, matching Logseq's YYYY_MM_DD.
const now = new Date();
const jname = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
const JFILE = `${G}/journals/${jname}.md`;

const GRID_MD = [
  "- grid host",
  "  tine.view:: grid",
  "\t-",
  "\t\t- alpha",
  "\t\t- beta",
  "\t-",
  "\t\t- gamma",
  "\t\t- delta",
  "- ## 4 · Task kanban",
  "  {{query (todo TODO DOING DONE)}}",
  "  tine.view:: board",
  "  tine.group-by:: state",
  "- TODO buy milk",
  "- task table",
  "  tine.view:: table",
  "  tine.fields:: state=state;topic=enum:infra,ui;shipped=checkbox",
  "\t- WAIT row one",
  "\t  topic:: infra",
  "\t  shipped:: false",
  "- reading list",
  "  tine.view:: board",
  "  tine.group-by:: tags",
  "\t- paper one #alpha",
  "\t- paper two #alpha #beta",
  "",
].join("\n");

fs.rmSync(G, { recursive: true, force: true });
fs.mkdirSync(`${G}/pages`, { recursive: true });
fs.mkdirSync(`${G}/journals`, { recursive: true });
fs.writeFileSync(JFILE, GRID_MD);

fs.rmSync("/tmp/sheets-e2e-xdg", { recursive: true, force: true });
for (const d of ["data", "config", "cache"]) fs.mkdirSync(`/tmp/sheets-e2e-xdg/${d}`, { recursive: true });
const env = {
  ...process.env,
  TINE_GRAPH: G,
  XDG_DATA_HOME: "/tmp/sheets-e2e-xdg/data",
  XDG_CONFIG_HOME: "/tmp/sheets-e2e-xdg/config",
  XDG_CACHE_HOME: "/tmp/sheets-e2e-xdg/cache",
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  GDK_BACKEND: "x11",
};

const tdLog = fs.openSync("/tmp/sheets-e2e-td.log", "w");
const td = spawn(
  TD,
  ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],
  { env, stdio: ["ignore", tdLog, tdLog], detached: true }
);
await sleep(3000);

let failures = 0;
let checks = 0;
const check = (name, ok, extra = "") => {
  checks++;
  console.log(`${ok ? "PASS" : "FAIL"}: ${name}${ok ? "" : "  " + extra}`);
  if (!ok) failures++;
};

let browser;
try {
  browser = await remote({
    hostname: "127.0.0.1",
    port: DRIVER_PORT,
    path: "/",
    capabilities: {
      browserName: "wry",
      "wdio:enforceWebDriverClassic": true,
      "tauri:options": { application: APP },
    },
    logLevel: "error",
    connectionRetryCount: 1,
    connectionRetryTimeout: 60000,
  });
  await browser.$(".ls-block").waitForExist({ timeout: 20000 });

  // The journals feed shows today's journal — the grid should be rendered.
  const cell = await browser.$('.sheet-cell[data-row="0"][data-col="0"]');
  await cell.waitForExist({ timeout: 10000 });
  check("grid renders in real app", true);

  // Click selects; Enter edits; Esc returns to selection; double-click edits.
  await cell.click();
  await sleep(400);
  let editing = await browser.execute(() => !!document.activeElement && document.activeElement.tagName === "TEXTAREA");
  check("clicking a cell selects without editing", !editing);
  let selectedCells = await browser.$$(".sheet-cell-selected");
  check("click selected exactly one cell", selectedCells.length === 1);
  const firstColVisible = await browser.execute(() => {
    const el = document.querySelector('.sheet-grid .sheet-cell[data-row="0"][data-col="0"]');
    if (!el) return { ok: false, reason: "missing first-column grid cell" };
    const style = getComputedStyle(el);
    return {
      ok:
        el.classList.contains("sheet-cell-selected") &&
        el.classList.contains("sheet-sticky-left") &&
        el.getAttribute("data-sheet-grid-id") &&
        style.boxShadow.includes("inset"),
      boxShadow: style.boxShadow,
      zIndex: style.zIndex,
      className: el.className,
    };
  });
  check("first-column click selects with visible sticky ring", firstColVisible.ok, JSON.stringify(firstColVisible));

  await browser.keys(["Enter"]);
  await sleep(300);
  editing = await browser.execute(() => !!document.activeElement && document.activeElement.tagName === "TEXTAREA");
  check("Enter edits selected cell", editing);

  await browser.keys(["Escape"]);
  await sleep(250);
  editing = await browser.execute(() => !!document.activeElement && document.activeElement.tagName === "TEXTAREA");
  selectedCells = await browser.$$(".sheet-cell-selected");
  check("Esc from cell edit returns to selection", !editing && selectedCells.length === 1);

  const dblClicked = await browser.execute(() => {
    const el = document.querySelector('.sheet-cell[data-row="0"][data-col="0"]');
    if (!el) return false;
    el.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true, button: 0 }));
    return true;
  });
  await sleep(300);
  editing = await browser.execute(() => !!document.activeElement && document.activeElement.tagName === "TEXTAREA");
  check("double-click edits selected cell", dblClicked && editing);

  // Move to end of the cell text (double-click-to-caret can land mid-word at the click
  // point — correct, but make the expected text deterministic), type, then
  // Enter — must COMMIT (not split the block).
  await browser.keys(["End"]);
  await browser.keys(["-", "e", "d", "i", "t", "e", "d"]);
  await sleep(200);
  await browser.keys(["Enter"]);
  await sleep(300);
  const stillEditing = await browser.execute(() => !!document.activeElement && document.activeElement.tagName === "TEXTAREA");
  check("Enter commits (editor closed)", !stillEditing);
  const selRing = await browser.$$(".sheet-cell-selected");
  check("Enter returns to cell selection", selRing.length === 1);

  // Wait out the save debounce, then verify the disk state.
  await sleep(2500);
  const disk = fs.readFileSync(JFILE, "utf8");
  check("edited text saved to disk", disk.includes("alpha-edited"), JSON.stringify(disk));
  // Seeded file has 9 bullets (host + 2 rows + 4 cells + board query + task) —
  // an Enter-split would add one.
  const bulletCount = (disk.match(/^\s*-/gm) || []).length;
  check("no block was split/created (14 bullets)", bulletCount === 14, `got ${bulletCount}: ${JSON.stringify(disk)}`);
  check("grid config intact", disk.includes("tine.view:: grid"), JSON.stringify(disk));

  // Esc from selection → outline selection on the grid block (no doc change).
  await browser.keys(["Escape"]);
  await sleep(200);
  check("Esc exits toward outline selection (no crash)", true, "");
  const disk2 = fs.readFileSync(JFILE, "utf8");
  check("Esc changed nothing on disk", disk2 === disk);

  // --- Seam insert + undo (phase 2b write path, real backend) ---------------
  // Re-enter cell (0,0), step down onto the row seam between rows, type → a
  // NEW row materializes with the typed char; then undo removes it fully.
  const cell00 = await browser.$('.sheet-cell[data-row="0"][data-col="0"]');
  await cell00.click();
  await sleep(300);
  await browser.keys(["ArrowDown"]); // cell -> row seam (seam stepping ON)
  await sleep(200);
  const seamShown = await browser.$$(".sheet-seam-selected");
  check("ArrowDown lands on a seam", seamShown.length === 1);
  await browser.keys(["z"]); // type on seam → insert row + overtype edit
  await sleep(400);
  await browser.keys(["Enter"]); // commit
  await sleep(2500);
  const disk3 = fs.readFileSync(JFILE, "utf8");
  const bullets3 = (disk3.match(/^\s*-/gm) || []).length;
  check("seam-typing inserted a row on disk (16 bullets: +row +cell)", bullets3 === 16, `got ${bullets3}: ${JSON.stringify(disk3)}`);
  check("inserted cell holds the typed char", /-\s*z\s*$/m.test(disk3), JSON.stringify(disk3));
  // The typed char rides the editor's own undo entry; the structural insert is
  // its own atomic unit — so TWO undos fully revert (text, then structure).
  await browser.keys(["Control", "z"]);
  await sleep(400);
  await browser.keys(["Control", "z"]);
  await sleep(2500);
  const disk4 = fs.readFileSync(JFILE, "utf8");
  check("two undos fully revert the seam insert (text, then structure)", disk4 === disk2, JSON.stringify(disk4));

  const ladderStart = await browser.execute(() => {
    const cell = document.querySelector('.sheet-grid .sheet-cell[data-row="0"][data-col="1"]');
    if (!cell) return false;
    cell.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, cancelable: true, button: 0, clientX: 20, clientY: 15 }));
    cell.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, cancelable: true, button: 0, clientX: 20, clientY: 15 }));
    return true;
  });
  await sleep(200);
  await browser.keys(["ArrowLeft"]);
  await sleep(150);
  const ladderSeam = await browser.execute(() => {
    const grid = document.querySelector(".sheet-grid");
    return !!grid && [...grid.children].some((el) => el.classList.contains("sheet-seam-selected"));
  });
  await browser.keys(["ArrowLeft"]);
  await sleep(150);
  const ladderCell = await browser.execute(() => {
    const selected = document.querySelector(".sheet-grid > .sheet-cell-selected");
    return selected?.getAttribute("data-row") === "0" && selected?.getAttribute("data-col") === "0";
  });
  await browser.keys(["ArrowLeft"]);
  await sleep(150);
  const ladderBoundary = await browser.execute(() => {
    const grid = document.querySelector(".sheet-grid");
    return !!grid && [...grid.children].some((el) => el.classList.contains("sheet-seam-selected"));
  });
  await browser.keys(["ArrowLeft"]);
  await sleep(150);
  const ladderBoundaryStays = await browser.execute(() => {
    const grid = document.querySelector(".sheet-grid");
    return !!grid && [...grid.children].some((el) => el.classList.contains("sheet-seam-selected"));
  });
  check("seam ladder starts from §1 grid cell", ladderStart);
  check("seam ladder cell-to-seam renders", ladderSeam);
  check("seam ladder seam-to-cell renders", ladderCell);
  check("seam ladder left boundary seam renders", ladderBoundary);
  check("seam ladder boundary stays visible", ladderBoundaryStays);

  // --- Fill down (phase 2c) --------------------------------------------------
  const cellA = await browser.$('.sheet-cell[data-row="0"][data-col="1"]');
  await cellA.click();
  await sleep(300);
  await browser.keys(["Shift", "ArrowDown"]); // range (0,1)-(1,1)... shift over seam? range extension skips seams
  await sleep(150);
  await browser.keys(["Control", "d"]); // fill down
  await sleep(2500);
  const disk5 = fs.readFileSync(JFILE, "utf8");
  const betaCount = (disk5.match(/- beta/g) || []).length;
  check("Ctrl+D filled beta into the row below", betaCount === 2, JSON.stringify(disk5));

  // --- Typed cells (phase 6b): checkbox toggle + enum popup write ------------
  // Seed has a schema'd table: columns title=0, state=1, topic(enum)=2, shipped(checkbox)=3.
  const tableNavStart = await browser.$('.sheet-table .sheet-cell[data-row="0"][data-col="0"]');
  if (await tableNavStart.isExisting()) {
    await tableNavStart.click();
    await sleep(200);
    await browser.keys(["ArrowRight"]);
    await sleep(200);
    const tableNav = await browser.execute(() => {
      const selected = document.querySelector('.sheet-table .sheet-cell-selected');
      return selected ? { row: selected.getAttribute("data-row"), col: selected.getAttribute("data-col") } : null;
    });
    check("Table ArrowRight uses the real global key path", tableNav?.row === "0" && tableNav?.col === "1", JSON.stringify(tableNav));
  } else {
    check("Table ArrowRight uses the real global key path", false, "no Table cell (0,0)");
  }

  const cbCell = await browser.$('.sheet-table .sheet-cell[data-row="0"][data-col="3"]');
  if (await cbCell.isExisting()) {
    await browser.execute(() => {
      const el = document.querySelector('.sheet-table .sheet-cell[data-row="0"][data-col="3"] input.sheet-checkbox');
      el?.click();
    });
    await sleep(2400);
    const diskCb = fs.readFileSync(JFILE, "utf8");
    check("checkbox single-click wrote shipped:: true", diskCb.includes("shipped:: true"), JSON.stringify(diskCb));

    await browser.execute(() => {
      const el = document.querySelector('.sheet-table .sheet-cell[data-row="0"][data-col="2"] .sheet-tag-chip');
      el?.click();
    });
    await sleep(400);
    const items = await browser.execute(() =>
      [...document.querySelectorAll(".ctx-item")].map((el) => (el.textContent ?? "").trim())
    );
    check("enum popup lists declared values + Clear",
      items.includes("infra") && items.includes("ui") && items.includes("Clear"), JSON.stringify(items));
    const clicked = await browser.execute(() => {
      const item = [...document.querySelectorAll(".ctx-item")].find((el) => (el.textContent ?? "").trim() === "ui");
      if (!item) return false;
      item.click();
      return true;
    });
    if (clicked) {
      await sleep(2400);
      const diskEnum = fs.readFileSync(JFILE, "utf8");
      check("enum pick wrote topic:: ui", diskEnum.includes("topic:: ui"), JSON.stringify(diskEnum));
    } else {
      check("enum popup item clickable", false, JSON.stringify(items));
    }
  } else {
    check("schema'd table rendered", false, "no table cell (0,2) found");
  }

  // --- Formula editor (phase 7c): add a formula, value computed not stored ---
  const tableEl = await browser.$(".sheet-table");
  if (await tableEl.isExisting()) {
    await browser.execute(() => {
      const el = document.querySelector(".sheet-table");
      el?.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 400, clientY: 300 }));
    });
    await sleep(400);
    const addClicked = await browser.execute(() => {
      const item = [...document.querySelectorAll(".ctx-item")].find((el) => /Add formula/.test(el.textContent ?? ""));
      if (!item) return false;
      item.click();
      return true;
    });
    check("Add formula… menu entry opens", addClicked);
    await sleep(400);
    const filled = await browser.execute(() => {
      const name = document.querySelector("[class*=formula-editor] input[type=text], [class*=formula-editor] input");
      const expr = document.querySelector("[class*=formula-editor] textarea");
      if (!name || !expr) return false;
      const set = (el, v) => {
        const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
        Object.getOwnPropertyDescriptor(proto, "value").set.call(el, v);
        el.dispatchEvent(new Event("input", { bubbles: true }));
      };
      set(name, "extra");
      set(expr, "1 + 1");
      return true;
    });
    check("formula editor fields filled", filled);
    await sleep(300);
    const saved = await browser.execute(() => {
      const btn = [...document.querySelectorAll("[class*=formula-editor] button, button")].find(
        (b) => /save/i.test(b.textContent ?? "") && !b.disabled
      );
      if (!btn) return false;
      btn.click();
      return true;
    });
    check("formula editor save enabled + clicked", saved);
    await sleep(2400);
    const disk8 = fs.readFileSync(JFILE, "utf8");
    check("formula property written to disk", disk8.includes("tine.formula.extra:: 1 + 1"), JSON.stringify(disk8));
    check("computed value NOT stored on disk", !/extra:: 2/.test(disk8), JSON.stringify(disk8));
    const cellShows = await browser.execute(() => {
      const table = document.querySelector(".sheet-table");
      const headers = [...(table?.querySelectorAll(".sheet-field-header") ?? [])].map((h) => h.textContent?.trim() ?? "");
      const cells = [...(table?.querySelectorAll(".sheet-field-cell") ?? [])].map((c) => c.textContent?.replace("⋮", "").trim());
      return { hasHeader: headers.some((h) => h.includes("extra")), hasValue: cells.includes("2") };
    });
    check("computed column renders with value 2", cellShows.hasHeader && cellShows.hasValue, JSON.stringify(cellShows));
  } else {
    check("table for formula editor", false, "no .sheet-table");
  }

  // --- Board card-move (phase 3): flip a task marker via Ctrl+ArrowRight ----
  // The seeded journal also carries a board query + one TODO task (see seed).
  const card = await browser.$(".sheet-board-card");
  if (await card.isExisting()) {
    await card.click();
    await sleep(300);
    const cardClick = await browser.execute(() => ({
      selected: document.querySelectorAll(".sheet-board-card.sheet-cell-selected").length,
      editing: !!document.activeElement && document.activeElement.tagName === "TEXTAREA",
    }));
    check("clicking a board card selects without editing", cardClick.selected === 1 && !cardClick.editing, JSON.stringify(cardClick));

    await browser.execute(() => {
      const block = [...document.querySelectorAll(".ls-block")].find((el) =>
        (el.querySelector(".heading-text")?.textContent ?? "").includes("Task kanban")
      );
      block?.scrollIntoView({ block: "center", inline: "nearest" });
    });
    await sleep(300);
    await browser.saveScreenshot("/tmp/sheets-e2e-query-board.png");
    const queryBoardContainment = await browser.execute(() => {
      const visible = (el) => {
        const r = el.getBoundingClientRect();
        const s = getComputedStyle(el);
        return r.width > 0 && r.height > 0 && s.display !== "none" && s.visibility !== "hidden";
      };
      const rectObj = (r) => ({ left: r.left, top: r.top, right: r.right, bottom: r.bottom, width: r.width, height: r.height });
      const block = [...document.querySelectorAll(".ls-block")].find((el) =>
        (el.querySelector(".heading-text")?.textContent ?? "").includes("Task kanban")
      );
      if (!block) return { found: false, reason: "missing Task kanban block" };
      const boards = [...block.querySelectorAll(".sheet-board")].filter(visible);
      const container = block.querySelector(".block-sheet-container");
      const heading = block.querySelector(".heading-text");
      if (!container || boards.length === 0) return { found: true, boards: boards.length, reason: "missing container or board" };
      const boardRect = boards[0].getBoundingClientRect();
      const containerRect = container.getBoundingClientRect();
      const headingRect = heading?.getBoundingClientRect() ?? null;
      const next = block.nextElementSibling;
      const nextRect = next?.getBoundingClientRect() ?? null;
      const contained =
        boardRect.left >= containerRect.left - 2 &&
        boardRect.right <= containerRect.right + 2 &&
        boardRect.top >= containerRect.top - 2 &&
        boardRect.bottom <= containerRect.bottom + 2;
      const noHeadingOverlap =
        !headingRect || containerRect.top >= headingRect.bottom - 2 || containerRect.bottom <= headingRect.top + 2;
      const noNextOverlap =
        !nextRect || containerRect.bottom <= nextRect.top + 2 || containerRect.top >= nextRect.bottom - 2;
      return {
        found: true,
        boards: boards.length,
        contained,
        noHeadingOverlap,
        noNextOverlap,
        board: rectObj(boardRect),
        container: rectObj(containerRect),
        heading: headingRect ? rectObj(headingRect) : null,
        next: nextRect ? rectObj(nextRect) : null,
      };
    });
    check("one-block query board renders exactly one visible board", queryBoardContainment.found && queryBoardContainment.boards === 1, JSON.stringify(queryBoardContainment));
    check("one-block query board stays inside its SheetContainer", !!queryBoardContainment.contained, JSON.stringify(queryBoardContainment));
    check("one-block query board container has no vertical bleed", !!queryBoardContainment.noHeadingOverlap && !!queryBoardContainment.noNextOverlap, JSON.stringify(queryBoardContainment));

    // Column order comes from the marker workflow and the board re-groups after
    // every move, so step one column at a time toward DOING, re-reading the
    // card's position each step (a precomputed step count goes stale).
    const readPos = () => browser.execute(() => {
      const headers = [...document.querySelectorAll(".sheet-board-header")].map((h) => h.textContent ?? "");
      const cardCol = document.querySelector(".sheet-board-card")?.closest(".sheet-board-column");
      const cols = [...document.querySelectorAll(".sheet-board-column")];
      // The duplicate-board regression guard: the QUERY block must render one
      // board (the seed's tags board is a separate block — exclude it by content).
      const boards = [...document.querySelectorAll(".sheet-board")].filter((b) =>
        [...b.querySelectorAll(".sheet-board-card")].some((c) => (c.textContent ?? "").includes("buy milk"))
      ).length;
      return { boards, from: cols.indexOf(cardCol), to: headers.findIndex((h) => h.startsWith("DONE")) };
    });
    for (let i = 0; i < 8; i++) {
      const pos = await readPos();
      if (i === 0) check("exactly one board renders for the query block", pos.boards === 1, `got ${pos.boards}`);
      if (pos.from === pos.to || pos.from < 0 || pos.to < 0) break;
      await browser.keys(["Control", pos.to > pos.from ? "ArrowRight" : "ArrowLeft"]);
      await sleep(350);
    }
    await sleep(2400);
    const disk6 = fs.readFileSync(JFILE, "utf8");
    check("card-move flipped TODO to DONE on disk", disk6.includes("DONE buy milk"), JSON.stringify(disk6));
    check("card-move touched only the marker (no reparent)", (disk6.match(/buy milk/g) || []).length === 1);
  } else {
    check("board card rendered", false, "no .sheet-card found — check board seed/selector");
  }

  // --- Tags board (phase 6c): multi-column membership + span-guided move -----
  // Seed: "reading list" children board grouped by tags; paper one #alpha,
  // paper two #alpha #beta.
  const paperTwoCount = await browser.execute(
    () => [...document.querySelectorAll(".sheet-board-card")].filter((c) => (c.textContent ?? "").includes("paper two")).length
  );
  check("2-tag card renders in both tag columns", paperTwoCount === 2, `got ${paperTwoCount}`);

  const paperOneClicked = await browser.execute(() => {
    const card = [...document.querySelectorAll(".sheet-board-card")].find((c) => (c.textContent ?? "").includes("paper one"));
    if (!card) return false;
    card.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
    card.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }));
    return true;
  });
  if (paperOneClicked) {
    await sleep(300);
    // Move alpha → beta: re-read position each step within paper one's board.
    const readTagPos = () => browser.execute(() => {
      const card = [...document.querySelectorAll(".sheet-board-card")].find((c) => (c.textContent ?? "").includes("paper one"));
      const cols = [...document.querySelectorAll(".sheet-board-column")];
      const headers = [...document.querySelectorAll(".sheet-board-header")].map((h) => h.textContent ?? "");
      return {
        from: cols.indexOf(card?.closest(".sheet-board-column") ?? null),
        to: headers.findIndex((h) => h.startsWith("beta")),
      };
    });
    for (let i = 0; i < 6; i++) {
      const pos = await readTagPos();
      if (pos.from === pos.to || pos.from < 0 || pos.to < 0) break;
      await browser.keys(["Control", pos.to > pos.from ? "ArrowRight" : "ArrowLeft"]);
      await sleep(350);
    }
    await sleep(2400);
    const disk7 = fs.readFileSync(JFILE, "utf8");
    check("tag move rewrote exactly the tag token", /- paper one #beta\s*$/m.test(disk7), JSON.stringify(disk7));
    check("tag move left the 2-tag sibling untouched", disk7.includes("paper two #alpha #beta"), JSON.stringify(disk7));
  } else {
    check("tags board card rendered", false, "no paper one card found");
  }

  const markerClick = await browser.execute(() => {
    const cell = document.querySelector('.sheet-table .sheet-cell[data-row="0"][data-col="1"]');
    const marker = cell?.querySelector(".block-marker");
    if (!cell || !marker) return { clicked: false };
    marker.click();
    return {
      clicked: true,
      selected: cell.classList.contains("sheet-cell-selected"),
      editing: !!document.activeElement && document.activeElement.tagName === "TEXTAREA",
    };
  });
  await sleep(2400);
  const diskMarker = fs.readFileSync(JFILE, "utf8");
  check("marker pill single-click cycled table state", markerClick.clicked && diskMarker.includes("- LATER row one"), JSON.stringify({ markerClick, diskMarker }));
  check("marker pill click selected without editing", markerClick.selected && !markerClick.editing, JSON.stringify(markerClick));

  // --- N27: aggregate picker is an in-DOM menu (a native <select>'s WebKitGTK
  // popup is a separate GTK window that blurs + collapses it). Pin the footer
  // via the corner Σ, open the picker, pick Sum, verify value + disk write.
  const aggSetup = await browser.execute(() => {
    const grid = document.querySelector(".sheet-grid");
    const container = grid?.closest(".block-sheet-container");
    if (!container) return { ok: false, reason: "no grid container" };
    container.dispatchEvent(new Event("pointerenter", { bubbles: false }));
    return { ok: true };
  });
  await sleep(300);
  const aggToggle = await browser.$(".sheet-aggregate-corner-toggle");
  check("corner Σ toggle appears on hover", aggSetup.ok && (await aggToggle.isExisting()));
  if (await aggToggle.isExisting()) {
    await aggToggle.click();
    await sleep(300);
    const addBtn = await browser.$(".sheet-grid .sheet-footer-cell .sheet-aggregate-add");
    check("pinned footer shows Σ add buttons", await addBtn.isExisting());
    if (await addBtn.isExisting()) {
      await addBtn.click();
      await sleep(300);
      const menuState = await browser.execute(() => {
        const items = [...document.querySelectorAll(".ctx-item")].map((el) => el.textContent?.trim() ?? "");
        return { count: items.length, hasSum: items.includes("Sum"), items: items.slice(0, 5) };
      });
      check("aggregate picker opens as in-DOM menu with fns", menuState.hasSum, JSON.stringify(menuState));
      const picked = await browser.execute(() => {
        const sum = [...document.querySelectorAll(".ctx-item")].find((el) => el.textContent?.trim() === "Sum");
        if (!sum) return false;
        sum.click();
        return true;
      });
      await sleep(2400);
      const diskAgg = fs.readFileSync(JFILE, "utf8");
      check("picking Sum writes tine.col-aggregates to disk", picked && /tine\.col-aggregates:: .*=sum/.test(diskAgg), JSON.stringify(diskAgg.match(/tine\.col-aggregates.*/)?.[0] ?? "missing"));
      const aggOverflow = await browser.execute(() => {
        const sc = document.querySelector(".sheet-grid")?.closest(".block-sheet-container")?.querySelector(".sheet-scroll");
        if (!sc) return { ok: false, reason: "no scroller" };
        return { ok: sc.scrollHeight <= sc.clientHeight + 1, sh: sc.scrollHeight, ch: sc.clientHeight };
      });
      check("aggregate row adds no vertical scroller overflow", aggOverflow.ok, JSON.stringify(aggOverflow));
    }
  }
} catch (e) {
  failures++;
  console.error("E2E error:", e && e.message);
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
}
console.log(failures === 0 ? `\nALL PASS (${checks} checks)` : `\n${failures} FAILURES (${checks} checks)`);
process.exit(failures === 0 ? 0 : 1);
